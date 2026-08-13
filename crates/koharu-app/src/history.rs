//! Linear undo/redo history + append-only durable op log.
//!
//! Two concerns, deliberately separated:
//!   1. **Durability log** — `history.log`: each applied op fsynced before ack
//!      so a crash loses at most the op currently being written.
//!   2. **Undo/redo stacks** — in-memory only; Cmd+Z within a session.
//!
//! Undo/redo are themselves logged ops: when the user undoes, we apply the
//! inverse and append it to the log as a normal op. Replay on open always
//! produces the post-undo state. No special entry type.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use koharu_core::{Op, Scene};
use serde::{Deserialize, Serialize};

/// Default cap for the in-memory undo stack. The log on disk is not capped —
/// it's compacted on snapshot.
const DEFAULT_UNDO_LIMIT: usize = 500;
pub(crate) const LOG_FRAME_V2_PREFIX: &[u8] = b"KHARLOG\x02";

// ---------------------------------------------------------------------------
// Log frames
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct LogFrame {
    epoch: u64,
    op: Op,
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

pub struct History {
    log_path: PathBuf,
    log: BufWriter<File>,
    epoch: u64,
    undo_stack: VecDeque<Op>,
    redo_stack: Vec<Op>,
    limit: usize,
    #[cfg(test)]
    fail_next_frame_write: bool,
}

impl History {
    /// Open the log at `path`, creating it if missing. Caller is expected to
    /// have already replayed any existing frames (see `Self::replay`).
    pub fn open(path: impl Into<PathBuf>, epoch: u64) -> Result<Self> {
        let log_path = path.into();
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&log_path)
            .with_context(|| format!("open history log {}", log_path.display()))?;
        Ok(Self {
            log_path,
            log: BufWriter::new(file),
            epoch,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            limit: DEFAULT_UNDO_LIMIT,
            #[cfg(test)]
            fail_next_frame_write: false,
        })
    }

    /// Override the in-memory undo-stack cap.
    pub fn with_undo_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Test-only fault injection: make the next `write_frame` fail before it
    /// touches the file, simulating write/flush/sync errors on the durable path.
    #[cfg(test)]
    pub(crate) fn fail_next_frame_write(&mut self) {
        self.fail_next_frame_write = true;
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Apply an op to the scene, fsync a frame to disk, push to the undo stack.
    ///
    /// Durability-first ordering: the op is applied to a scratch scene and the
    /// frame is made durable before any in-memory state is published, so a
    /// failed write leaves no trace in memory or on disk.
    pub fn apply(&mut self, scene: &mut Scene, mut op: Op) -> Result<u64> {
        let mut candidate = scene.clone();
        op.apply(&mut candidate).context("apply op to scene")?;
        self.write_frame(self.epoch + 1, &op)?;
        *scene = candidate;
        self.epoch += 1;
        self.push_undo(op);
        self.redo_stack.clear();
        Ok(self.epoch)
    }

    /// Undo the most recent op. Applies its inverse, records the inverse in
    /// the log, and moves the original onto the redo stack. Returns the new
    /// epoch + the inverse op that was just applied (so the RPC layer can
    /// broadcast it for clients to patch their mirrors without refetching).
    pub fn undo(&mut self, scene: &mut Scene) -> Result<Option<(u64, Op)>> {
        let Some(original) = self.undo_stack.back().cloned() else {
            return Ok(None);
        };
        let mut inverse = original.inverse();
        let mut candidate = scene.clone();
        inverse.apply(&mut candidate).context("apply inverse op")?;
        self.write_frame(self.epoch + 1, &inverse)?;
        *scene = candidate;
        self.epoch += 1;
        self.undo_stack.pop_back();
        let inverse_out = inverse.clone();
        self.redo_stack.push(original);
        Ok(Some((self.epoch, inverse_out)))
    }

    /// Re-apply the most recent undo. Symmetric with `undo`. Returns the new
    /// epoch + the op that was just re-applied.
    pub fn redo(&mut self, scene: &mut Scene) -> Result<Option<(u64, Op)>> {
        let Some(mut op) = self.redo_stack.last().cloned() else {
            return Ok(None);
        };
        let mut candidate = scene.clone();
        op.apply(&mut candidate).context("re-apply op")?;
        self.write_frame(self.epoch + 1, &op)?;
        *scene = candidate;
        self.epoch += 1;
        self.redo_stack.pop();
        let applied = op.clone();
        self.push_undo(op);
        Ok(Some((self.epoch, applied)))
    }

    /// Truncate the log after a snapshot has been committed.
    /// Caller must have already fsynced the snapshot file.
    pub fn truncate_log(&mut self) -> Result<()> {
        self.log.flush()?;
        self.log.get_ref().sync_all()?;
        // Reopen to truncate; BufWriter's underlying file handle is append-only.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(true)
            .open(&self.log_path)
            .with_context(|| format!("truncate history log {}", self.log_path.display()))?;
        file.sync_all()?;
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.log_path)?;
        self.log = BufWriter::new(file);
        Ok(())
    }

    // --- internals ---------------------------------------------------------

    fn write_frame(&mut self, epoch: u64, op: &Op) -> Result<()> {
        #[cfg(test)]
        if self.fail_next_frame_write {
            self.fail_next_frame_write = false;
            anyhow::bail!("injected frame write failure");
        }
        let frame = LogFrame {
            epoch,
            op: op.clone(),
        };
        let encoded = postcard::to_allocvec(&frame).context("encode log frame")?;
        let mut bytes = Vec::with_capacity(LOG_FRAME_V2_PREFIX.len() + encoded.len());
        bytes.extend_from_slice(LOG_FRAME_V2_PREFIX);
        bytes.extend_from_slice(&encoded);
        let len = u32::try_from(bytes.len()).context("log frame too large")?;
        self.log.write_all(&len.to_le_bytes())?;
        self.log.write_all(&bytes)?;
        self.log.flush()?;
        self.log.get_ref().sync_data()?;
        Ok(())
    }

    fn push_undo(&mut self, op: Op) {
        self.undo_stack.push_back(op);
        while self.undo_stack.len() > self.limit {
            self.undo_stack.pop_front();
        }
    }
}

// ---------------------------------------------------------------------------
// Replay — called once on project open, before a `History` is constructed.
// ---------------------------------------------------------------------------

/// Replay each frame in `log_path` with epoch greater than `start_epoch`
/// against `scene`. Returns the final epoch seen.
pub fn replay(log_path: &Path, start_epoch: u64, scene: &mut Scene) -> Result<u64> {
    replay_with_policy(log_path, start_epoch, scene, false)
}

pub(crate) fn replay_with_policy(
    log_path: &Path,
    start_epoch: u64,
    scene: &mut Scene,
    strict: bool,
) -> Result<u64> {
    if !log_path.exists() {
        return Ok(start_epoch);
    }
    let file =
        File::open(log_path).with_context(|| format!("open history log {}", log_path.display()))?;
    let mut reader = BufReader::new(file);
    let mut epoch = start_epoch;
    loop {
        if reader.fill_buf()?.is_empty() {
            break;
        }
        let mut len_buf = [0u8; 4];
        match reader.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !strict => {
                tracing::warn!(
                    path = %log_path.display(),
                    "truncated trailing frame length in history log; discarding"
                );
                break;
            }
            Err(e) => return Err(anyhow::Error::new(e).context("read log frame length")),
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        match reader.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !strict => {
                // Truncated frame (likely crash mid-write) — stop cleanly.
                tracing::warn!(
                    path = %log_path.display(),
                    expected_len = len,
                    "truncated trailing frame in history log; discarding"
                );
                break;
            }
            Err(e) => return Err(anyhow::Error::new(e).context("read log frame body")),
        }
        let decoded = if let Some(bytes) = buf.strip_prefix(LOG_FRAME_V2_PREFIX) {
            postcard::from_bytes::<LogFrame>(bytes)
                .map(|frame| (frame.epoch, frame.op))
                .map_err(anyhow::Error::new)
        } else {
            crate::persistence_v1::decode_log_frame(&buf)
        };
        let (frame_epoch, frame_op) = match decoded {
            Ok(frame) => frame,
            Err(err) if !strict => {
                tracing::warn!(
                    path = %log_path.display(),
                    error = %err,
                    "undecodable frame in history log; stopping replay"
                );
                break;
            }
            Err(err) => return Err(err.context("decode history log frame")),
        };
        if frame_epoch > epoch {
            let mut op = frame_op;
            op.apply(scene).context("replay op")?;
            epoch = frame_epoch;
        }
    }
    // Seek to end so subsequent appends go after the last valid frame.
    let _ = reader.seek(SeekFrom::End(0));
    Ok(epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_core::{ProjectMetaPatch, op::Op};
    use tempfile::tempdir;

    fn meta_op(name: &str) -> Op {
        Op::UpdateProjectMeta {
            patch: ProjectMetaPatch {
                name: Some(name.into()),
                ..Default::default()
            },
            prev: ProjectMetaPatch::default(),
        }
    }

    fn log_len(path: &std::path::Path) -> u64 {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    // AR04-T02 RED: a failed durable frame write must leave no trace — scene,
    // epoch, both stacks, and the log on disk all unchanged. Current code
    // mutates memory before persisting, so these fail until GREEN reorders.
    #[test]
    fn apply_frame_write_failure_leaves_no_trace() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.log");
        let mut scene = Scene::default();
        let mut history = History::open(&path, 0).unwrap();
        history.apply(&mut scene, meta_op("first")).unwrap();
        let log_before = log_len(&path);

        history.fail_next_frame_write();
        let result = history.apply(&mut scene, meta_op("second"));

        assert!(result.is_err(), "injected write failure must surface");
        assert_eq!(scene.project.name, "first", "scene must not change");
        assert_eq!(history.epoch(), 1, "epoch must not advance");
        assert_eq!(history.undo_stack.len(), 1, "undo stack must not change");
        assert!(history.redo_stack.is_empty(), "redo stack must not change");
        assert_eq!(log_len(&path), log_before, "log must not grow");
    }

    #[test]
    fn undo_frame_write_failure_leaves_no_trace() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.log");
        let mut scene = Scene::default();
        let mut history = History::open(&path, 0).unwrap();
        history.apply(&mut scene, meta_op("first")).unwrap();
        let log_before = log_len(&path);

        history.fail_next_frame_write();
        let result = history.undo(&mut scene);

        assert!(result.is_err(), "injected write failure must surface");
        assert_eq!(scene.project.name, "first", "scene must not change");
        assert_eq!(history.epoch(), 1, "epoch must not advance");
        assert_eq!(history.undo_stack.len(), 1, "op must remain undoable");
        assert!(history.redo_stack.is_empty(), "redo stack must not change");
        assert_eq!(log_len(&path), log_before, "log must not grow");
    }

    #[test]
    fn redo_frame_write_failure_leaves_no_trace() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.log");
        let mut scene = Scene::default();
        let mut history = History::open(&path, 0).unwrap();
        history.apply(&mut scene, meta_op("first")).unwrap();
        history.undo(&mut scene).unwrap();
        let log_before = log_len(&path);

        history.fail_next_frame_write();
        let result = history.redo(&mut scene);

        assert!(result.is_err(), "injected write failure must surface");
        assert_eq!(scene.project.name, "", "scene must not change");
        assert_eq!(history.epoch(), 2, "epoch must not advance");
        assert!(history.undo_stack.is_empty(), "undo stack must not change");
        assert_eq!(history.redo_stack.len(), 1, "op must remain redoable");
        assert_eq!(log_len(&path), log_before, "log must not grow");
    }

    // Lock: the success path keeps its observable semantics.
    #[test]
    fn apply_success_publishes_after_durable_frame() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.log");
        let mut scene = Scene::default();
        let mut history = History::open(&path, 0).unwrap();
        history.apply(&mut scene, meta_op("first")).unwrap();
        history.apply(&mut scene, meta_op("second")).unwrap();
        history.undo(&mut scene).unwrap();

        assert_eq!(history.epoch(), 3);
        assert_eq!(scene.project.name, "first");

        let mut replayed = Scene::default();
        assert_eq!(replay(&path, 0, &mut replayed).unwrap(), 3);
        assert_eq!(replayed.project.name, "first");
    }

    #[test]
    fn current_history_frame_uses_v2_prefix_and_replays() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.log");
        let mut scene = Scene::default();
        let mut history = History::open(&path, 0).unwrap();
        history
            .apply(
                &mut scene,
                Op::UpdateProjectMeta {
                    patch: ProjectMetaPatch {
                        name: Some("v2".into()),
                        ..Default::default()
                    },
                    prev: ProjectMetaPatch::default(),
                },
            )
            .unwrap();
        drop(history);

        let bytes = std::fs::read(&path).unwrap();
        let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(len, bytes.len() - 4);
        assert!(bytes[4..].starts_with(LOG_FRAME_V2_PREFIX));

        let mut replayed = Scene::default();
        assert_eq!(replay(&path, 0, &mut replayed).unwrap(), 1);
        assert_eq!(replayed.project.name, "v2");
    }

    #[test]
    fn trusted_replay_tolerates_undecodable_complete_v1_tail() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.log");
        let invalid_v1 = [0xff, 0xff, 0xff, 0xff];
        let mut bytes = (invalid_v1.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(&invalid_v1);
        std::fs::write(&path, bytes).unwrap();

        assert_eq!(replay(&path, 7, &mut Scene::default()).unwrap(), 7);
        assert!(replay_with_policy(&path, 7, &mut Scene::default(), true).is_err());
    }
}
