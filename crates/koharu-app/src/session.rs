//! A loaded project. One `ProjectSession` = one `.khrproj/` directory.
//!
//! Holds:
//!   - an exclusive `.lock` file (refuses a second opener)
//!   - the in-memory `Scene` behind a `parking_lot::RwLock` (never held across `.await`)
//!   - the `History` behind a `Mutex` (linear, all writes serialized)
//!   - the `BlobStore` (content-addressed images)
//!
//! On-disk layout:
//!   `.khrproj/project.toml`    — TOML-encoded `ProjectMeta`
//!   `.khrproj/scene.bin`       — postcard-encoded `Snapshot { epoch, scene }`
//!   `.khrproj/history.log`     — append-only `LogFrame { epoch, op }`
//!   `.khrproj/blobs/ab/cdef…`  — content-addressed blobs
//!   `.khrproj/.lock`           — exclusive file lock (session lifetime)

use std::fs::File;
use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, Result};
use atomicwrites::{AtomicFile, OverwriteBehavior};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use koharu_core::{Scene, op::Op};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::blobs::BlobStore;
use crate::history::{self, History};

const SCENE_FILE: &str = "scene.bin";
const LOG_FILE: &str = "history.log";
const LOCK_FILE: &str = ".lock";
const BLOBS_DIR: &str = "blobs";
const CACHE_DIR: &str = "cache";
const PROJECT_TOML: &str = "project.toml";
pub(crate) const SNAPSHOT_V2_PREFIX: &[u8] = b"KHARSCN\x02";

/// Snapshot written to `scene.bin`.
#[derive(Serialize, Deserialize)]
struct Snapshot {
    epoch: u64,
    scene: Scene,
}

#[cfg(test)]
#[derive(Default)]
struct CompactApplySync {
    state: Mutex<CompactApplyState>,
    ready: parking_lot::Condvar,
    release: parking_lot::Condvar,
}

#[cfg(test)]
#[derive(Default)]
struct CompactApplyState {
    compact_waiting: bool,
    apply_waiting: bool,
    released: bool,
}

#[cfg(test)]
impl CompactApplySync {
    fn wait_after_history_lock(&self) {
        let mut state = self.state.lock();
        state.compact_waiting = true;
        self.ready.notify_all();
        while !state.released {
            self.release.wait(&mut state);
        }
    }

    fn wait_before_apply_lock(&self) {
        let mut state = self.state.lock();
        state.apply_waiting = true;
        self.ready.notify_all();
        while !state.released {
            self.release.wait(&mut state);
        }
    }

    fn release_when_apply_contends(&self) {
        let mut state = self.state.lock();
        while !(state.compact_waiting && state.apply_waiting) {
            assert!(
                !self
                    .ready
                    .wait_for(&mut state, std::time::Duration::from_secs(10))
                    .timed_out(),
                "compact and apply must reach their controlled lock boundary (compact={}, apply={})",
                state.compact_waiting,
                state.apply_waiting,
            );
        }
        state.released = true;
        self.release.notify_all();
    }
}

/// A loaded project.
pub struct ProjectSession {
    pub dir: Utf8PathBuf,
    pub scene: RwLock<Scene>,
    pub history: Mutex<History>,
    pub blobs: Arc<BlobStore>,
    /// Held for the lifetime of the session.
    _lock: File,
    #[cfg(test)]
    compact_apply_sync: Mutex<Option<Arc<CompactApplySync>>>,
}

impl ProjectSession {
    /// Open an existing `.khrproj/` directory.
    pub fn open(dir: impl AsRef<Utf8Path>) -> Result<Arc<Self>> {
        let dir = dir.as_ref().to_path_buf();
        if !dir.is_dir() {
            anyhow::bail!("not a project directory: {dir}");
        }
        Self::open_inner(dir, false, false)
    }

    /// Open externally supplied project state, remove internal trust markers,
    /// and compact it before exposing the session.
    pub fn open_untrusted(dir: impl AsRef<Utf8Path>) -> Result<Arc<Self>> {
        let dir = dir.as_ref().to_path_buf();
        if !dir.is_dir() {
            anyhow::bail!("not a project directory: {dir}");
        }
        let session = Self::open_inner(dir, false, true)?;
        clear_typography_markers(&mut session.scene.write());
        session.compact()?;
        Ok(session)
    }

    /// Create a fresh `.khrproj/` at `dir`, failing if it already exists.
    pub fn create(dir: impl AsRef<Utf8Path>, name: impl Into<String>) -> Result<Arc<Self>> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(dir.as_std_path())
            .with_context(|| format!("create project dir {dir}"))?;
        // Project should be empty.
        let is_empty = std::fs::read_dir(dir.as_std_path())?.next().is_none();
        if !is_empty {
            anyhow::bail!("project directory not empty: {dir}");
        }
        // Seed the TOML with the name so open_inner can load it.
        let meta = ProjectTomlFile {
            name: name.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        std::fs::write(
            dir.join(PROJECT_TOML).as_std_path(),
            toml::to_string_pretty(&meta)?,
        )?;
        Self::open_inner(dir, true, false)
    }

    fn open_inner(dir: Utf8PathBuf, creating: bool, strict_history: bool) -> Result<Arc<Self>> {
        std::fs::create_dir_all(dir.join(BLOBS_DIR).as_std_path())?;
        std::fs::create_dir_all(dir.join(CACHE_DIR).as_std_path())?;

        // Exclusive lock — one opener at a time.
        let lock_path = dir.join(LOCK_FILE);
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path.as_std_path())
            .with_context(|| format!("open lock file {}", lock_path))?;
        lock.try_lock()
            .context("project is already open elsewhere")?;

        let blobs = Arc::new(BlobStore::open(dir.join(BLOBS_DIR).as_std_path())?);

        // Load or synthesize the scene + epoch.
        let (mut scene, mut epoch) = load_snapshot(&dir, creating)?;
        // Replay any log frames past the snapshot epoch.
        let log_path = dir.join(LOG_FILE);
        epoch =
            history::replay_with_policy(log_path.as_std_path(), epoch, &mut scene, strict_history)
                .with_context(|| format!("replay log {}", log_path))?;

        let history_obj = History::open(log_path.as_std_path(), epoch)?;

        Ok(Arc::new(Self {
            dir,
            scene: RwLock::new(scene),
            history: Mutex::new(history_obj),
            blobs,
            _lock: lock,
            #[cfg(test)]
            compact_apply_sync: Mutex::new(None),
        }))
    }

    // --- scene mutation ----------------------------------------------------

    /// Apply an Op. Returns the epoch after apply.
    pub fn apply(&self, op: Op) -> Result<u64> {
        #[cfg(test)]
        self.wait_before_apply_lock();
        let mut history = self.history.lock();
        let mut scene = self.scene.write();
        history.apply(&mut scene, op)
    }

    pub fn undo(&self) -> Result<Option<(u64, Op)>> {
        let mut history = self.history.lock();
        let mut scene = self.scene.write();
        history.undo(&mut scene)
    }

    pub fn redo(&self) -> Result<Option<(u64, Op)>> {
        let mut history = self.history.lock();
        let mut scene = self.scene.write();
        history.redo(&mut scene)
    }

    pub fn epoch(&self) -> u64 {
        self.history.lock().epoch()
    }

    /// Cheap clone of the scene for read-only consumers (pipeline engines).
    pub fn scene_snapshot(&self) -> Scene {
        self.scene.read().clone()
    }

    /// Return one coherent scene/epoch pair. Paths that need both locks use
    /// the history -> scene order so they cannot deadlock with compaction.
    pub fn scene_snapshot_with_epoch(&self) -> (u64, Scene) {
        let history = self.history.lock();
        let scene = self.scene.read();
        (history.epoch(), scene.clone())
    }

    /// Apply only if no concurrent mutation has advanced the history epoch.
    /// `None` is an expected optimistic-concurrency conflict.
    pub fn apply_if_epoch(&self, expected: u64, op: Op) -> Result<Option<u64>> {
        let mut history = self.history.lock();
        let mut scene = self.scene.write();
        if history.epoch() != expected {
            return Ok(None);
        }
        history.apply(&mut scene, op).map(Some)
    }

    // --- compaction --------------------------------------------------------

    /// Write a new snapshot (scene.bin) and truncate the log. Safe to call
    /// at any time; crash mid-compaction leaves the old snapshot + full log.
    pub fn compact(&self) -> Result<()> {
        // Retain this guard through the durable snapshot write and log
        // truncation so an apply cannot land between those two operations.
        let mut history = self.history.lock();
        #[cfg(test)]
        self.wait_after_history_lock();
        let snap = {
            let scene = self.scene.read();
            Snapshot {
                epoch: history.epoch(),
                scene: scene.clone(),
            }
        };
        let encoded = postcard::to_allocvec(&snap).context("encode snapshot")?;
        let mut bytes = Vec::with_capacity(SNAPSHOT_V2_PREFIX.len() + encoded.len());
        bytes.extend_from_slice(SNAPSHOT_V2_PREFIX);
        bytes.extend_from_slice(&encoded);
        AtomicFile::new(
            self.dir.join(SCENE_FILE).as_std_path(),
            OverwriteBehavior::AllowOverwrite,
        )
        .write(|f| f.write_all(&bytes))
        .context("write scene.bin atomically")?;
        // Log truncation only after snapshot is durably on disk.
        history.truncate_log()?;
        Ok(())
    }

    #[cfg(test)]
    fn wait_before_apply_lock(&self) {
        let sync = self.compact_apply_sync.lock().clone();
        if let Some(sync) = sync {
            sync.wait_before_apply_lock();
        }
    }

    #[cfg(test)]
    fn wait_after_history_lock(&self) {
        let sync = self.compact_apply_sync.lock().clone();
        if let Some(sync) = sync {
            sync.wait_after_history_lock();
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot loading / TOML metadata
// ---------------------------------------------------------------------------

fn load_snapshot(dir: &Utf8Path, creating: bool) -> Result<(Scene, u64)> {
    let scene_path = dir.join(SCENE_FILE);
    if scene_path.exists() {
        let bytes = std::fs::read(scene_path.as_std_path())
            .with_context(|| format!("read {}", scene_path))?;
        if let Some(encoded) = bytes.strip_prefix(SNAPSHOT_V2_PREFIX) {
            let snap: Snapshot = postcard::from_bytes(encoded)
                .with_context(|| format!("decode v2 {}", scene_path))?;
            return Ok((snap.scene, snap.epoch));
        }
        return crate::persistence_v1::decode_snapshot(&bytes)
            .with_context(|| format!("decode v1 {}", scene_path));
    }

    // No snapshot — build one from `project.toml` (or defaults for creation).
    let toml_path = dir.join(PROJECT_TOML);
    let meta = if toml_path.exists() {
        let text = std::fs::read_to_string(toml_path.as_std_path())?;
        toml::from_str::<ProjectTomlFile>(&text).with_context(|| format!("parse {}", toml_path))?
    } else if creating {
        ProjectTomlFile {
            name: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    } else {
        anyhow::bail!("missing project.toml at {}", toml_path);
    };

    let mut scene = Scene::default();
    scene.project.name = meta.name;
    scene.project.created_at = meta.created_at;
    scene.project.updated_at = meta.updated_at;
    Ok((scene, 0))
}

fn clear_typography_markers(scene: &mut Scene) {
    for node in scene
        .pages
        .values_mut()
        .flat_map(|page| page.nodes.values_mut())
    {
        if let koharu_core::NodeKind::Text(text) = &mut node.kind {
            text.typography_plan_verified = false;
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ProjectTomlFile {
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use koharu_core::{
        BlobRef, ImageData, ImageRole, MaskData, MaskRole, Node, NodeId, NodeKind, Op, Page,
        PageId, ProjectMetaPatch, TextAlign, TextData, TextShaderEffect, TextStrokeStyle,
        TextStyle, Transform,
    };
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;
    use tempfile::tempdir;

    fn tmp_dir() -> (tempfile::TempDir, Utf8PathBuf) {
        let dir = tempdir().unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        (dir, path.join("proj.khrproj"))
    }

    fn copy_v1_fixture(path: &Utf8Path) {
        std::fs::create_dir_all(path.as_std_path()).unwrap();
        std::fs::write(
            path.join(SCENE_FILE).as_std_path(),
            include_bytes!("../tests/fixtures/persistence-v1/scene.bin"),
        )
        .unwrap();
        std::fs::write(
            path.join(LOG_FILE).as_std_path(),
            include_bytes!("../tests/fixtures/persistence-v1/history.log"),
        )
        .unwrap();
    }

    fn only_text(scene: &Scene) -> &TextData {
        let node = scene
            .pages
            .values()
            .flat_map(|page| page.nodes.values())
            .find(|node| matches!(node.kind, NodeKind::Text(_)))
            .expect("fixture text node");
        let NodeKind::Text(text) = &node.kind else {
            unreachable!();
        };
        text
    }

    fn planner_owned_style() -> TextStyle {
        TextStyle {
            font_families: vec!["Planner Sans".into()],
            font_size: Some(24.0),
            color: [12, 34, 56, 255],
            stroke: Some(TextStrokeStyle {
                enabled: true,
                color: [78, 90, 123, 255],
                width_px: Some(2.0),
            }),
            effect: Some(TextShaderEffect {
                italic: true,
                bold: true,
            }),
            text_align: Some(TextAlign::Center),
        }
    }

    fn project_with_verified_text(path: &Utf8Path) -> (PageId, NodeId) {
        let session = ProjectSession::create(path, "verified").unwrap();
        let mut page = Page::new("p1", 320, 240);
        let page_id = page.id;
        let node_id = NodeId::new();
        page.nodes.insert(
            node_id,
            Node {
                id: node_id,
                transform: Transform::default(),
                visible: true,
                kind: NodeKind::Text(TextData {
                    text: Some("source".into()),
                    translation: Some("planned".into()),
                    style: Some(planner_owned_style()),
                    typography_plan_verified: true,
                    ..Default::default()
                }),
            },
        );
        session.apply(Op::AddPage { page, at: 0 }).unwrap();
        session.compact().unwrap();
        drop(session);
        (page_id, node_id)
    }

    fn stage_untrusted_history(path: &Utf8Path) -> (PageId, NodeId) {
        let (page, node) = project_with_verified_text(path);
        let session = ProjectSession::open(path).unwrap();
        session
            .apply(Op::UpdateNode {
                page,
                id: node,
                patch: koharu_core::NodePatch {
                    data: Some(koharu_core::NodeDataPatch::Text(
                        koharu_core::TextDataPatch {
                            translation: Some(Some("history value".into())),
                            style: Some(Some(planner_owned_style())),
                            typography_plan_verified: Some(true),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                },
                prev: koharu_core::NodePatch::default(),
            })
            .unwrap();
        drop(session);
        (page, node)
    }

    #[test]
    fn legacy_postcard_project_open_migrates_snapshot_and_replays_history() {
        let (_tmp, path) = tmp_dir();
        copy_v1_fixture(&path);

        let session = ProjectSession::open(&path).expect("open v1 fixture");
        assert_eq!(session.epoch(), 2);
        let scene = session.scene_snapshot();
        let text = only_text(&scene);
        assert_eq!(text.text.as_deref(), Some("legacy source"));
        assert_eq!(text.translation.as_deref(), Some("after history"));
        assert!(!text.typography_plan_verified);

        session.compact().expect("migrate to v2");
        drop(session);
        assert!(
            std::fs::read(path.join(SCENE_FILE))
                .unwrap()
                .starts_with(SNAPSHOT_V2_PREFIX)
        );
        assert_eq!(std::fs::metadata(path.join(LOG_FILE)).unwrap().len(), 0);

        let reopened = ProjectSession::open(&path).expect("reopen migrated project");
        assert_eq!(reopened.epoch(), 2);
        let scene = reopened.scene_snapshot();
        let text = only_text(&scene);
        assert_eq!(text.translation.as_deref(), Some("after history"));
        assert!(!text.typography_plan_verified);
    }

    #[test]
    fn current_snapshot_and_history_frames_use_v2_prefix() {
        let (_tmp, path) = tmp_dir();
        let (page, node) = project_with_verified_text(&path);
        let session = ProjectSession::open(&path).unwrap();
        session
            .apply(Op::UpdateNode {
                page,
                id: node,
                patch: koharu_core::NodePatch {
                    data: Some(koharu_core::NodeDataPatch::Text(
                        koharu_core::TextDataPatch {
                            translation: Some(Some("planned again".into())),
                            typography_plan_verified: Some(true),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                },
                prev: koharu_core::NodePatch::default(),
            })
            .unwrap();
        drop(session);

        assert!(
            std::fs::read(path.join(SCENE_FILE))
                .unwrap()
                .starts_with(SNAPSHOT_V2_PREFIX)
        );
        let log = std::fs::read(path.join(LOG_FILE)).unwrap();
        let frame_len = u32::from_le_bytes(log[..4].try_into().unwrap()) as usize;
        assert_eq!(frame_len, log.len() - 4);
        assert!(log[4..].starts_with(history::LOG_FRAME_V2_PREFIX));

        let reopened = ProjectSession::open(&path).unwrap();
        let scene = reopened.scene_snapshot();
        assert_eq!(
            only_text(&scene).translation.as_deref(),
            Some("planned again")
        );
        assert!(only_text(&scene).typography_plan_verified);
    }

    #[test]
    fn managed_project_reopen_preserves_verified_marker() {
        let (_tmp, path) = tmp_dir();
        project_with_verified_text(&path);

        let reopened = ProjectSession::open(&path).unwrap();
        assert!(only_text(&reopened.scene_snapshot()).typography_plan_verified);
    }

    #[test]
    fn untrusted_project_open_clears_marker_and_compacts_history() {
        let (_tmp, path) = tmp_dir();
        stage_untrusted_history(&path);

        let untrusted = ProjectSession::open_untrusted(&path).unwrap();
        let scene = untrusted.scene_snapshot();
        let text = only_text(&scene);
        assert_eq!(text.translation.as_deref(), Some("history value"));
        assert!(!text.typography_plan_verified);
        assert_eq!(untrusted.epoch(), 2);
        drop(untrusted);

        assert!(
            std::fs::read(path.join(SCENE_FILE))
                .unwrap()
                .starts_with(SNAPSHOT_V2_PREFIX)
        );
        assert_eq!(std::fs::metadata(path.join(LOG_FILE)).unwrap().len(), 0);
    }

    #[test]
    #[ignore = "hanonly-pre-greenc-red"]
    fn hanonly_pre_greenc_red_t3_untrusted_marker_lifecycle_contract() {
        let (_tmp, path) = tmp_dir();
        let (page, node) = stage_untrusted_history(&path);
        let untrusted = ProjectSession::open_untrusted(&path).unwrap();
        let scene = untrusted.scene_snapshot();
        let text = match &scene.node(page, node).expect("staged text ID").kind {
            NodeKind::Text(text) => text,
            _ => panic!("expected staged text"),
        };
        assert_eq!(text.translation.as_deref(), Some("history value"));
        assert!(!text.typography_plan_verified);
        assert!(text.style.is_none());
        assert_eq!(untrusted.epoch(), 2);
        assert!(
            std::fs::read(path.join(SCENE_FILE))
                .unwrap()
                .starts_with(SNAPSHOT_V2_PREFIX)
        );
        assert_eq!(std::fs::metadata(path.join(LOG_FILE)).unwrap().len(), 0);
        let epoch = untrusted.epoch();

        // This marker-free patch is accepted by the external route contract.
        let result = untrusted.apply(Op::UpdateNode {
            page,
            id: node,
            patch: koharu_core::NodePatch {
                data: Some(koharu_core::NodeDataPatch::Text(
                    koharu_core::TextDataPatch {
                        style: Some(Some(planner_owned_style())),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            prev: koharu_core::NodePatch::default(),
        });

        assert!(
            result.is_err(),
            "an untrusted session must reject Planner-owned style reintroduction"
        );
        assert_eq!(untrusted.epoch(), epoch);
        let scene = untrusted.scene_snapshot();
        let text = match &scene.node(page, node).expect("staged text ID").kind {
            NodeKind::Text(text) => text,
            _ => panic!("expected staged text"),
        };
        assert_eq!(text.translation.as_deref(), Some("history value"));
        assert!(!text.typography_plan_verified);
        assert!(text.style.is_none());
    }

    #[test]
    fn create_apply_close_reopen_preserves_scene() {
        let (_tmp, path) = tmp_dir();
        let page_id: PageId;
        {
            let session = ProjectSession::create(&path, "test").unwrap();
            let page = Page::new("p1", 800, 600);
            page_id = page.id;
            session
                .apply(Op::AddPage { page, at: 0 })
                .expect("apply AddPage");
            session.compact().unwrap();
            // Session drops, lock released.
        }
        let session = ProjectSession::open(&path).unwrap();
        assert_eq!(session.scene.read().pages.len(), 1);
        assert!(session.scene.read().pages.contains_key(&page_id));
    }

    #[test]
    fn legacy_optional_layers_round_trip_before_scope_reduction() {
        let (_tmp, path) = tmp_dir();
        let page_id: PageId;
        let expected = [
            BlobRef::new("source"),
            BlobRef::new("rendered"),
            BlobRef::new("custom"),
            BlobRef::new("segment"),
            BlobRef::new("bubble"),
            BlobRef::new("brush"),
            BlobRef::new("sprite"),
        ];
        {
            let session = ProjectSession::create(&path, "legacy-layers").unwrap();
            let mut page = Page::new("p1", 64, 64);
            page_id = page.id;
            for (role, blob) in [
                (ImageRole::Source, expected[0].clone()),
                (ImageRole::Rendered, expected[1].clone()),
                (ImageRole::Custom, expected[2].clone()),
            ] {
                let id = NodeId::new();
                page.nodes.insert(
                    id,
                    Node {
                        id,
                        transform: Transform::default(),
                        visible: true,
                        kind: NodeKind::Image(ImageData {
                            role,
                            blob,
                            opacity: 1.0,
                            natural_width: 64,
                            natural_height: 64,
                            name: None,
                        }),
                    },
                );
            }
            for (role, blob) in [
                (MaskRole::Segment, expected[3].clone()),
                (MaskRole::Bubble, expected[4].clone()),
                (MaskRole::BrushInpaint, expected[5].clone()),
            ] {
                let id = NodeId::new();
                page.nodes.insert(
                    id,
                    Node {
                        id,
                        transform: Transform::default(),
                        visible: true,
                        kind: NodeKind::Mask(MaskData { role, blob }),
                    },
                );
            }
            let text_id = NodeId::new();
            page.nodes.insert(
                text_id,
                Node {
                    id: text_id,
                    transform: Transform::default(),
                    visible: true,
                    kind: NodeKind::Text(TextData {
                        sprite: Some(expected[6].clone()),
                        ..Default::default()
                    }),
                },
            );
            session.apply(Op::AddPage { page, at: 0 }).unwrap();
            session.compact().unwrap();
        }

        let session = ProjectSession::open(&path).unwrap();
        let page = session.scene.read().pages.get(&page_id).unwrap().clone();
        let mut blobs = page
            .nodes
            .values()
            .filter_map(|node| match &node.kind {
                NodeKind::Image(data) => Some(data.blob.clone()),
                NodeKind::Mask(data) => Some(data.blob.clone()),
                NodeKind::Text(data) => data.sprite.clone(),
            })
            .collect::<Vec<_>>();
        blobs.sort_by(|left, right| left.0.cmp(&right.0));
        let mut expected_blobs = expected.to_vec();
        expected_blobs.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(blobs, expected_blobs);
        for (role, blob) in [
            (ImageRole::Source, &expected[0]),
            (ImageRole::Rendered, &expected[1]),
            (ImageRole::Custom, &expected[2]),
        ] {
            assert!(page.nodes.values().any(
                |node| matches!(&node.kind, NodeKind::Image(data) if data.role == role && &data.blob == blob)
            ));
        }
        for (role, blob) in [
            (MaskRole::Segment, &expected[3]),
            (MaskRole::Bubble, &expected[4]),
            (MaskRole::BrushInpaint, &expected[5]),
        ] {
            assert!(page.nodes.values().any(
                |node| matches!(&node.kind, NodeKind::Mask(data) if data.role == role && &data.blob == blob)
            ));
        }
        assert!(page.nodes.values().any(
            |node| matches!(&node.kind, NodeKind::Text(data) if data.sprite.as_ref() == Some(&expected[6]))
        ));
    }

    #[test]
    fn reopen_preserves_text_style_effects_in_scene_bin() {
        let (_tmp, path) = tmp_dir();
        let page_id: PageId;
        let node_id: NodeId;
        {
            let session = ProjectSession::create(&path, "styled").unwrap();
            let page = Page::new("p1", 800, 600);
            page_id = page.id;
            session
                .apply(Op::AddPage { page, at: 0 })
                .expect("apply AddPage");

            node_id = NodeId::new();
            let mut scene = session.scene.write();
            let page = scene.pages.get_mut(&page_id).expect("page");
            page.nodes.insert(
                node_id,
                Node {
                    id: node_id,
                    transform: Transform {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 40.0,
                        rotation_deg: 0.0,
                    },
                    visible: true,
                    kind: NodeKind::Text(TextData {
                        style: Some(TextStyle {
                            font_families: vec!["Arial".to_string()],
                            font_size: Some(20.0),
                            color: [0, 0, 0, 255],
                            effect: Some(TextShaderEffect {
                                italic: true,
                                bold: true,
                            }),
                            stroke: None,
                            text_align: None,
                        }),
                        ..Default::default()
                    }),
                },
            );
            drop(scene);
            session.compact().unwrap();
        }

        let session = ProjectSession::open(&path).unwrap();
        let scene = session.scene.read();
        let page = scene.pages.get(&page_id).expect("page");
        let node = page.nodes.get(&node_id).expect("node");
        let NodeKind::Text(text) = &node.kind else {
            panic!("expected text node");
        };
        let effect = text
            .style
            .as_ref()
            .and_then(|style| style.effect)
            .expect("effect");
        assert!(effect.italic);
        assert!(effect.bold);
    }

    #[test]
    fn exclusive_lock_prevents_second_open() {
        let (_tmp, path) = tmp_dir();
        let a = ProjectSession::create(&path, "test").unwrap();
        let err = ProjectSession::open(&path)
            .err()
            .expect("second open must fail");
        assert!(err.to_string().contains("already open"));
        drop(a);
    }

    #[test]
    fn compact_and_typography_epoch_paths_share_lock_order_without_deadlock() {
        let (_tmp, path) = tmp_dir();
        let session = ProjectSession::create(&path, "before").unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let (done_tx, done_rx) = mpsc::channel();

        let compact_session = session.clone();
        let compact_barrier = barrier.clone();
        let compact_done = done_tx.clone();
        let compact = std::thread::spawn(move || {
            compact_barrier.wait();
            compact_session.compact().unwrap();
            compact_done.send(()).unwrap();
        });

        let apply_session = session.clone();
        let apply_barrier = barrier.clone();
        let apply_done = done_tx;
        let apply = std::thread::spawn(move || {
            apply_barrier.wait();
            let (epoch, _) = apply_session.scene_snapshot_with_epoch();
            let _ = apply_session
                .apply_if_epoch(
                    epoch,
                    Op::UpdateProjectMeta {
                        patch: ProjectMetaPatch {
                            name: Some("after".into()),
                            ..Default::default()
                        },
                        prev: ProjectMetaPatch::default(),
                    },
                )
                .unwrap();
            apply_done.send(()).unwrap();
        });

        barrier.wait();
        for _ in 0..2 {
            done_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("concurrent paths must complete");
        }
        compact.join().unwrap();
        apply.join().unwrap();
    }

    #[test]
    fn compact_does_not_truncate_concurrent_apply_from_reopen() {
        let (_tmp, path) = tmp_dir();
        let session = ProjectSession::create(&path, "before").unwrap();
        let sync = Arc::new(CompactApplySync::default());
        *session.compact_apply_sync.lock() = Some(sync.clone());

        let compact_session = session.clone();
        let compact = std::thread::spawn(move || {
            compact_session.compact().unwrap();
        });
        let apply_session = session.clone();
        let apply = std::thread::spawn(move || {
            apply_session
                .apply(Op::UpdateProjectMeta {
                    patch: ProjectMetaPatch {
                        name: Some("after".into()),
                        ..Default::default()
                    },
                    prev: ProjectMetaPatch::default(),
                })
                .unwrap();
        });

        // The compact thread holds history before snapshot write and retains
        // it through log truncation; the apply thread is immediately before
        // that lock. Release only after both conditions hold, so the update
        // cannot enter the truncate window and must be replayable after reopen.
        sync.release_when_apply_contends();
        compact.join().unwrap();
        apply.join().unwrap();
        drop(session);

        let reopened = ProjectSession::open(&path).unwrap();
        assert_eq!(reopened.scene_snapshot().project.name, "after");
    }
}
