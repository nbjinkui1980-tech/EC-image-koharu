//! Unix-only, test-only immutable input primitive for the future D0 harness.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, open, openat, statat};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileMetadata {
    dev: i128,
    ino: u64,
    owner: u64,
    mode: u64,
    file_type: FileType,
}

struct OpenedInput {
    descriptor: File,
    bytes: Box<[u8]>,
    sha256: [u8; 32],
    metadata: FileMetadata,
    ancestors: Vec<FileMetadata>,
}

pub(super) struct HeldInput {
    root: OwnedFd,
    descriptor: File,
    bytes: Box<[u8]>,
    sha256: [u8; 32],
    components: Vec<OsString>,
    metadata: FileMetadata,
    ancestors: Vec<FileMetadata>,
}

pub(super) struct PathValidation<'a> {
    held: &'a HeldInput,
    fresh: OpenedInput,
}

impl PathValidation<'_> {
    pub(super) fn original_descriptor(&self) -> BorrowedFd<'_> {
        self.held.descriptor.as_fd()
    }
    pub(super) fn fresh_descriptor(&self) -> BorrowedFd<'_> {
        self.fresh.descriptor.as_fd()
    }
    pub(super) fn with_current_namespace<T>(
        &self,
        publish: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        let (final_name, directories) = self
            .held
            .components
            .split_last()
            .ok_or_else(|| invalid_input("path has no final component"))?;
        let (parent, ancestors) = walk_directories(self.held.root.as_fd(), directories)?;
        require(
            ancestors == self.held.ancestors && ancestors == self.fresh.ancestors,
            "input ancestor identity changed",
        )?;
        let parent_fd = parent
            .as_ref()
            .map_or_else(|| self.held.root.as_fd(), AsFd::as_fd);
        let named = metadata(&fs(statat(
            parent_fd,
            final_name,
            AtFlags::SYMLINK_NOFOLLOW,
        ))?);
        let original = metadata(&fs(fstat(&self.held.descriptor))?);
        let fresh = metadata(&fs(fstat(&self.fresh.descriptor))?);
        require(
            named == self.held.metadata
                && named == self.fresh.metadata
                && named == original
                && named == fresh
                && named.file_type.is_file()
                && hash(&self.held.bytes) == self.held.sha256
                && hash(&self.fresh.bytes) == self.fresh.sha256
                && self.fresh.sha256 == self.held.sha256,
            "current input namespace or metadata changed",
        )?;
        let result = publish();
        drop(parent);
        result
    }
}

impl HeldInput {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let components = canonical_components(path)?;
        let root = fs(open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        let opened = open_input(root.as_fd(), &components)?;
        Ok(Self {
            root,
            descriptor: opened.descriptor,
            bytes: opened.bytes,
            sha256: opened.sha256,
            components,
            metadata: opened.metadata,
            ancestors: opened.ancestors,
        })
    }
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(super) fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
    pub(super) fn with_revalidated_path<T>(
        &self,
        action: impl FnOnce(&PathValidation<'_>) -> io::Result<T>,
    ) -> io::Result<T> {
        let held_metadata = metadata(&fs(fstat(&self.descriptor))?);
        require(
            held_metadata == self.metadata && held_metadata.file_type.is_file(),
            "held descriptor metadata changed",
        )?;
        let held_hash = hash(&self.bytes);
        require(held_hash == self.sha256, "held bytes changed")?;
        let fresh = open_input(self.root.as_fd(), &self.components)?;
        require(
            fresh.metadata == held_metadata
                && fresh.metadata.file_type.is_file()
                && fresh.ancestors == self.ancestors
                && fresh.sha256 == held_hash,
            "input path identity or bytes changed",
        )?;
        action(&PathValidation { held: self, fresh })
    }
}

fn canonical_components(path: &Path) -> io::Result<Vec<OsString>> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() < 2 || bytes[0] != b'/' || bytes[1] == b'/' || bytes.ends_with(b"/") {
        return Err(invalid_input(
            "path must be an absolute canonical file path",
        ));
    }
    bytes[1..]
        .split(|byte| *byte == b'/')
        .map(|component| {
            if component.is_empty()
                || component == b"."
                || component == b".."
                || component.contains(&0)
            {
                Err(invalid_input("path contains a noncanonical component"))
            } else {
                Ok(OsStr::from_bytes(component).to_owned())
            }
        })
        .collect()
}

fn open_input(root: impl AsFd, components: &[OsString]) -> io::Result<OpenedInput> {
    let (final_name, directories) = components
        .split_last()
        .ok_or_else(|| invalid_input("path has no final component"))?;
    let (current, ancestors) = walk_directories(root.as_fd(), directories)?;
    let parent = current.as_ref().map_or_else(|| root.as_fd(), AsFd::as_fd);
    let descriptor = fs(openat(
        parent,
        final_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ))?;
    let metadata = metadata(&fs(fstat(&descriptor))?);
    require(metadata.file_type.is_file(), "input is not a regular file")?;
    let mut descriptor = File::from(descriptor);
    let mut bytes = Vec::new();
    descriptor.read_to_end(&mut bytes)?;
    let sha256 = hash(&bytes);
    Ok(OpenedInput {
        descriptor,
        bytes: bytes.into_boxed_slice(),
        sha256,
        metadata,
        ancestors,
    })
}

fn walk_directories(
    root: BorrowedFd<'_>,
    directories: &[OsString],
) -> io::Result<(Option<OwnedFd>, Vec<FileMetadata>)> {
    let mut current: Option<OwnedFd> = None;
    let mut ancestors = Vec::with_capacity(directories.len());
    for directory in directories {
        let parent = current.as_ref().map_or_else(|| root.as_fd(), AsFd::as_fd);
        let next = fs(openat(
            parent,
            directory,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        let next_metadata = metadata(&fs(fstat(&next))?);
        require(
            next_metadata.file_type.is_dir(),
            "input ancestor is not a directory",
        )?;
        ancestors.push(next_metadata);
        current = Some(next);
    }
    Ok((current, ancestors))
}

fn metadata(stat: &rustix::fs::Stat) -> FileMetadata {
    FileMetadata {
        dev: i128::from(stat.st_dev),
        ino: stat.st_ino,
        owner: stat.st_uid.into(),
        mode: stat.st_mode.into(),
        file_type: FileType::from_raw_mode(stat.st_mode),
    }
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn require(condition: bool, message: &'static str) -> io::Result<()> {
    condition
        .then_some(())
        .ok_or_else(|| io::Error::other(message))
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn fs<T>(result: rustix::io::Result<T>) -> io::Result<T> {
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::os::unix::fs::{MetadataExt, symlink};
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    const FIFO_CHILD_ENV: &str = "KOHARU_D0_FIFO_CHILD";
    const FIFO_RESULT_ENV: &str = "KOHARU_D0_FIFO_RESULT";
    const FIFO_CHILD_TEST: &str = "pipeline::d0_held_input::tests::d0_held_input_fifo_child_helper";

    fn fixture(bytes: &[u8]) -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().unwrap();
        let nested = std::fs::canonicalize(temp.path()).unwrap().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let path = nested.join("input.bin");
        std::fs::write(&path, bytes).unwrap();
        (temp, path)
    }

    #[test]
    fn d0_held_input_accepts_nested_regular_file_and_holds_descriptor() {
        let (_temp, path) = fixture(b"immutable");
        let held = HeldInput::open(&path).unwrap();
        assert_eq!(held.bytes(), b"immutable");
        assert_eq!(held.sha256(), hash(b"immutable"));
        assert_eq!(held.components.last().unwrap(), "input.bin");
        held.with_revalidated_path(|validation| validation.with_current_namespace(|| Ok(())))
            .unwrap();

        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            metadata(&fs(fstat(&held.descriptor)).unwrap()),
            held.metadata
        );
        assert_eq!(held.bytes(), b"immutable");
        drop(held);
    }

    #[test]
    fn d0_held_input_rejects_symlink_ancestor_and_final() {
        let (_temp, path) = fixture(b"input");
        let root = path.parent().unwrap().parent().unwrap();
        let linked_dir = root.join("linked-dir");
        symlink(path.parent().unwrap(), &linked_dir).unwrap();
        assert!(HeldInput::open(&linked_dir.join("input.bin")).is_err());

        let linked_file = root.join("linked-file");
        symlink(&path, &linked_file).unwrap();
        assert!(HeldInput::open(&linked_file).is_err());
    }

    #[test]
    fn d0_held_input_fifo_child_helper() {
        let Some(fifo) = std::env::var_os(FIFO_CHILD_ENV) else {
            return;
        };
        let result = std::env::var_os(FIFO_RESULT_ENV).unwrap();
        assert!(HeldInput::open(Path::new(&fifo)).is_err());
        std::fs::write(result, b"rejected").unwrap();
    }

    #[test]
    fn d0_held_input_rejects_fifo_without_blocking() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let fifo = root.join("input.fifo");
        let result = root.join("child-result");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );

        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg(FIFO_CHILD_TEST)
            .arg("--exact")
            .env(FIFO_CHILD_ENV, &fifo)
            .env(FIFO_RESULT_ENV, &result)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("FIFO open blocked");
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success());
        assert_eq!(std::fs::read(result).unwrap(), b"rejected");
    }

    fn assert_namespace_replacement_blocks(replace: impl FnOnce(&Path)) {
        let (_temp, path) = fixture(b"old");
        let held = HeldInput::open(&path).unwrap();
        let publish_calls = Cell::new(0);

        held.with_revalidated_path(|validation| {
            replace(&path);
            assert_eq!(
                metadata(&fs(fstat(validation.original_descriptor()))?),
                held.metadata
            );
            assert_eq!(
                metadata(&fs(fstat(validation.fresh_descriptor()))?),
                held.metadata
            );
            assert!(
                validation
                    .with_current_namespace(|| {
                        publish_calls.set(publish_calls.get() + 1);
                        Ok(())
                    })
                    .is_err()
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(publish_calls.get(), 0);
        assert_eq!(held.bytes(), b"old");
        assert!(held.with_revalidated_path(|_| Ok(())).is_err());
    }

    #[test]
    fn d0_held_input_final_replacement_blocks_publish() {
        assert_namespace_replacement_blocks(|path| {
            std::fs::rename(path, path.with_file_name("old-inode")).unwrap();
            std::fs::write(path, b"new").unwrap();
        });
    }

    #[test]
    fn d0_held_input_ancestor_replacement_blocks_publish() {
        assert_namespace_replacement_blocks(|path| {
            let ancestor = path.parent().unwrap();
            let old_ancestor = ancestor.with_file_name("old-nested");
            std::fs::rename(ancestor, &old_ancestor).unwrap();
            std::fs::create_dir(ancestor).unwrap();
            std::fs::hard_link(old_ancestor.join("input.bin"), path).unwrap();
        });
    }

    #[test]
    fn d0_held_input_unchanged_namespace_publishes_once_with_both_descriptors() {
        let (_temp, path) = fixture(b"old");
        let held = HeldInput::open(&path).unwrap();
        let publish_calls = Cell::new(0);

        let published = held
            .with_revalidated_path(|validation| {
                validation.with_current_namespace(|| {
                    assert_eq!(
                        metadata(&fs(fstat(validation.original_descriptor()))?),
                        held.metadata
                    );
                    assert_eq!(
                        metadata(&fs(fstat(validation.fresh_descriptor()))?),
                        held.metadata
                    );
                    publish_calls.set(publish_calls.get() + 1);
                    Ok("published")
                })
            })
            .unwrap();
        assert_eq!(published, "published");
        assert_eq!(publish_calls.get(), 1);
    }

    #[test]
    fn d0_held_input_rejects_changed_bytes_on_same_path() {
        let (_temp, path) = fixture(b"before");
        let held = HeldInput::open(&path).unwrap();
        std::fs::write(&path, b"after").unwrap();
        assert_eq!(
            fs(fstat(&held.descriptor)).unwrap().st_ino,
            std::fs::metadata(&path).unwrap().ino()
        );
        assert!(held.with_revalidated_path(|_| Ok(())).is_err());
    }

    #[test]
    fn d0_held_input_rejects_noncanonical_paths_and_non_files() {
        let (_temp, path) = fixture(b"input");
        let root = path.parent().unwrap().parent().unwrap();
        assert!(HeldInput::open(Path::new("relative")).is_err());
        assert!(HeldInput::open(&root.join(".").join("nested/input.bin")).is_err());
        assert!(HeldInput::open(&root.join("nested").join("..").join("nested/input.bin")).is_err());
        assert!(HeldInput::open(root).is_err());

        let doubled = path.to_string_lossy().replace("/nested/", "//nested/");
        assert!(HeldInput::open(Path::new(&doubled)).is_err());
    }
}
