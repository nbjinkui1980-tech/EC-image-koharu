use super::d0_held_input::HeldInput;
use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, RenameFlags, fstat, fsync, open, openat, renameat_with,
    statat, unlinkat,
};
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FaultPoint {
    Write,
    TempFsync,
    Rename,
    FinalFsync,
    DirectoryFsync,
    CleanupUnlink,
    CleanupDirectoryFsync,
}
type F = FaultPoint;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Metadata {
    dev: i128,
    ino: u64,
    owner: u64,
    mode: u64,
    file_type: FileType,
}
struct OutputDirectory {
    root: OwnedFd,
    descriptor: OwnedFd,
    components: Vec<OsString>,
    metadata: Metadata,
    ancestors: Vec<Metadata>,
}
pub(super) struct PublishedOutput {
    pub(super) descriptor: File,
    pub(super) bytes: Box<[u8]>,
    pub(super) sha256: [u8; 32],
}
pub(super) fn publish<T>(
    input: &HeldInput,
    output_directory: &Path,
    final_name: &OsStr,
    output: &[u8],
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
    success: impl FnOnce(&PublishedOutput) -> io::Result<T>,
) -> io::Result<T> {
    let directory = OutputDirectory::open(output_directory)?;
    validate_child(final_name)?;
    let output = output.to_vec().into_boxed_slice();
    let temp_name = OsString::from(format!(".output.{}.tmp", hex(&hash(&output))));
    let mut success = Some(success);
    input.with_revalidated_path(|validation| {
        validation.with_current_namespace(|| {
            let published = transact(&directory, final_name, &temp_name, output, fault)?;
            input.with_revalidated_path(|validation| {
                validation.with_current_namespace(|| {
                    directory.revalidate(final_name, &published)?;
                    success.take().expect("success callback called once")(&published)
                })
            })
        })
    })
}
impl OutputDirectory {
    fn open(path: &Path) -> io::Result<Self> {
        let components = canonical_components(path)?;
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crate is inside the repository");
        require(!path.starts_with(repository), "inside repository")?;
        let expected_owner = effective_owner()?;
        let root = fs(open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        let (descriptor, metadata, ancestors) = walk(root.as_fd(), &components)?;
        require(
            valid_directory(metadata, expected_owner),
            "invalid directory",
        )?;
        Ok(Self {
            root,
            descriptor,
            components,
            metadata,
            ancestors,
        })
    }
    fn revalidate(&self, final_name: &OsStr, published: &PublishedOutput) -> io::Result<()> {
        let (fresh, metadata, ancestors) = walk(self.root.as_fd(), &self.components)?;
        require(
            metadata == self.metadata && ancestors == self.ancestors,
            "output directory namespace changed",
        )?;
        let held = metadata_of(&fs(fstat(&published.descriptor))?);
        let named = metadata_of(&fs(statat(
            fresh.as_fd(),
            final_name,
            AtFlags::SYMLINK_NOFOLLOW,
        ))?);
        require(
            held == named
                && held.file_type.is_file()
                && held.owner == self.metadata.owner
                && held.mode & 0o7777 == 0o600
                && hash(&published.bytes) == published.sha256,
            "final output namespace or metadata changed",
        )
    }
}
fn transact(
    directory: &OutputDirectory,
    final_name: &OsStr,
    temp_name: &OsStr,
    output: Box<[u8]>,
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
) -> io::Result<PublishedOutput> {
    let temp = match state(directory.descriptor.as_fd(), final_name, temp_name)? {
        State::Empty => {
            let descriptor = fs(openat(
                directory.descriptor.as_fd(),
                temp_name,
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            ))?;
            require_file(&descriptor, directory.metadata.owner)?;
            let mut temp = File::from(descriptor);
            let split = output.len().min(1);
            let write = temp
                .write_all(&output[..split])
                .and_then(|()| fault(F::Write))
                .and_then(|()| temp.write_all(&output[split..]));
            if let Err(error) = write {
                drop(temp);
                return match cleanup_created(directory.descriptor.as_fd(), temp_name, fault) {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(io::Error::other(format!("{error}; {cleanup}"))),
                };
            }
            temp
        }
        State::Temp => open_exact(
            directory.descriptor.as_fd(),
            temp_name,
            directory.metadata.owner,
            &output,
        )?,
        State::Final => {
            let final_file = open_regular(
                directory.descriptor.as_fd(),
                final_name,
                directory.metadata.owner,
                OFlags::RDONLY,
            )?;
            let published = read_published(final_file, output)?;
            fault(F::FinalFsync)?;
            fs(fsync(&published.descriptor))?;
            fault(F::DirectoryFsync)?;
            fs(fsync(&directory.descriptor))?;
            return Ok(published);
        }
    };
    fault(F::TempFsync)?;
    fs(fsync(&temp))?;
    fault(F::Rename)?;
    fs(renameat_with(
        directory.descriptor.as_fd(),
        temp_name,
        directory.descriptor.as_fd(),
        final_name,
        RenameFlags::NOREPLACE,
    ))?;
    fault(F::FinalFsync)?;
    fs(fsync(&temp))?;
    fault(F::DirectoryFsync)?;
    fs(fsync(&directory.descriptor))?;
    read_published(temp, output)
}
enum State {
    Empty,
    Temp,
    Final,
}
fn state(directory: BorrowedFd<'_>, final_name: &OsStr, temp_name: &OsStr) -> io::Result<State> {
    let mut names = Dir::read_from(directory)?
        .map(|entry| entry.map(|entry| OsStr::from_bytes(entry.file_name().to_bytes()).to_owned()))
        .collect::<rustix::io::Result<Vec<_>>>()
        .map_err(io::Error::from)?;
    names.retain(|name| name != "." && name != "..");
    match names.as_slice() {
        [] => Ok(State::Empty),
        [name] if name == temp_name => Ok(State::Temp),
        [name] if name == final_name => Ok(State::Final),
        _ => Err(io::Error::other(
            "output directory has unknown or conflicting entries",
        )),
    }
}
fn read_published(mut descriptor: File, expected: Box<[u8]>) -> io::Result<PublishedOutput> {
    let bytes = read_exact(&mut descriptor, &expected)?;
    let sha256 = hash(&bytes);
    Ok(PublishedOutput {
        descriptor,
        bytes,
        sha256,
    })
}
fn open_exact(
    directory: BorrowedFd<'_>,
    name: &OsStr,
    owner: u64,
    expected: &[u8],
) -> io::Result<File> {
    let mut descriptor = open_regular(directory, name, owner, OFlags::RDWR)?;
    drop(read_exact(&mut descriptor, expected)?);
    Ok(descriptor)
}
fn read_exact(descriptor: &mut File, expected: &[u8]) -> io::Result<Box<[u8]>> {
    descriptor.rewind()?;
    let mut bytes = Vec::new();
    descriptor.read_to_end(&mut bytes)?;
    require(bytes == expected, "output bytes do not match")?;
    Ok(bytes.into_boxed_slice())
}
fn open_regular(
    directory: BorrowedFd<'_>,
    name: &OsStr,
    owner: u64,
    access: OFlags,
) -> io::Result<File> {
    let descriptor = fs(openat(
        directory,
        name,
        access | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ))?;
    require_file(&descriptor, owner)?;
    Ok(File::from(descriptor))
}
fn require_file(descriptor: impl AsFd, owner: u64) -> io::Result<()> {
    let metadata = metadata_of(&fs(fstat(descriptor))?);
    let valid =
        metadata.file_type.is_file() && metadata.owner == owner && metadata.mode & 0o7777 == 0o600;
    require(valid, "invalid output file")
}
fn effective_owner() -> io::Result<u64> {
    let descriptor = tempfile::tempfile()?;
    let owner = metadata_of(&fs(fstat(&descriptor))?).owner;
    Ok(owner)
}
fn cleanup_created(
    directory: BorrowedFd<'_>,
    name: &OsStr,
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
) -> io::Result<()> {
    let dir = directory;
    let unlink = fault(F::CleanupUnlink).and_then(|()| fs(unlinkat(dir, name, AtFlags::empty())));
    let sync = fault(F::CleanupDirectoryFsync).and_then(|()| fs(fsync(directory)));
    match (unlink, sync) {
        (Ok(()), Ok(())) => Ok(()),
        (unlink, fsync) => Err(io::Error::other(format!(
            "cleanup failed: {unlink:?}; {fsync:?}"
        ))),
    }
}
fn canonical_components(path: &Path) -> io::Result<Vec<OsString>> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() < 2 || bytes[0] != b'/' || bytes[1] == b'/' || bytes.ends_with(b"/") {
        return Err(invalid("output directory path is not canonical"));
    }
    bytes[1..]
        .split(|byte| *byte == b'/')
        .map(|component| {
            let component = OsStr::from_bytes(component);
            validate_child(component)?;
            Ok(component.to_owned())
        })
        .collect()
}
fn validate_child(name: &OsStr) -> io::Result<()> {
    let bytes = name.as_bytes();
    let valid = !bytes.is_empty()
        && bytes != b"."
        && bytes != b".."
        && !bytes.contains(&b'/')
        && !bytes.contains(&0);
    valid
        .then_some(())
        .ok_or_else(|| invalid("invalid child name"))
}
fn walk(
    root: BorrowedFd<'_>,
    components: &[OsString],
) -> io::Result<(OwnedFd, Metadata, Vec<Metadata>)> {
    let mut current = None;
    let mut ancestors = Vec::with_capacity(components.len().saturating_sub(1));
    for (index, component) in components.iter().enumerate() {
        let parent = current.as_ref().map_or_else(|| root.as_fd(), AsFd::as_fd);
        let next = fs(openat(
            parent,
            component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        let metadata = metadata_of(&fs(fstat(&next))?);
        require(
            metadata.file_type.is_dir(),
            "output path component is not a directory",
        )?;
        if index + 1 != components.len() {
            ancestors.push(metadata);
        }
        current = Some(next);
    }
    let descriptor =
        current.ok_or_else(|| invalid("output directory cannot be filesystem root"))?;
    let metadata = metadata_of(&fs(fstat(&descriptor))?);
    Ok((descriptor, metadata, ancestors))
}
fn metadata_of(stat: &rustix::fs::Stat) -> Metadata {
    Metadata {
        dev: i128::from(stat.st_dev),
        ino: stat.st_ino,
        owner: stat.st_uid.into(),
        mode: stat.st_mode.into(),
        file_type: FileType::from_raw_mode(stat.st_mode),
    }
}
fn valid_directory(metadata: Metadata, owner: u64) -> bool {
    metadata.file_type.is_dir() && metadata.owner == owner && metadata.mode & 0o7777 == 0o700
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn require(condition: bool, message: &'static str) -> io::Result<()> {
    condition
        .then_some(())
        .ok_or_else(|| io::Error::other(message))
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn fs<T>(result: rustix::io::Result<T>) -> io::Result<T> {
    result.map_err(Into::into)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs as s;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use tempfile::TempDir;
    struct Fixture {
        _temp: TempDir,
        input: std::path::PathBuf,
        output: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let root = s::canonicalize(temp.path()).unwrap();
            let input = root.join("input");
            let output = root.join("output");
            s::create_dir(&input).unwrap();
            s::create_dir(&output).unwrap();
            mode(&output, 0o700);
            let input = input.join("source.bin");
            s::write(&input, b"input").unwrap();
            Self {
                _temp: temp,
                input,
                output,
            }
        }
    }
    fn no_fault(_: FaultPoint) -> io::Result<()> {
        Ok(())
    }
    fn mode(path: &Path, mode: u32) {
        s::set_permissions(path, s::Permissions::from_mode(mode)).unwrap();
    }
    fn file(path: &Path, bytes: &[u8], permissions: u32) {
        s::write(path, bytes).unwrap();
        mode(path, permissions);
    }
    fn temp_name() -> String {
        format!(".output.{}.tmp", hex(&hash(b"durable")))
    }
    fn snapshot(fixture: &Fixture) -> Vec<String> {
        let mut entries = s::read_dir(&fixture.output)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                let metadata = s::symlink_metadata(&path).unwrap();
                let mode = metadata.permissions().mode();
                let bytes = s::read(path).unwrap();
                format!("{:?}:{mode:o}:{bytes:?}", entry.file_name())
            })
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }
    fn run(
        fixture: &Fixture,
        held: &HeldInput,
        fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
        calls: &Cell<usize>,
    ) -> io::Result<()> {
        publish(
            held,
            &fixture.output,
            OsStr::new("result.bin"),
            b"durable",
            fault,
            |published| {
                calls.set(calls.get() + 1);
                assert_eq!(
                    (&*published.bytes, published.sha256),
                    (&b"durable"[..], hash(b"durable"))
                );
                Ok(())
            },
        )
    }
    fn publish_err(held: &HeldInput, directory: &Path, child: &OsStr) -> bool {
        publish(held, directory, child, b"x", &mut no_fault, |_| Ok(())).is_err()
    }
    #[test]
    fn d0_output_transaction_success_idempotence_and_temp_recovery() {
        for initial in ["empty", "temp"] {
            let fixture = Fixture::new();
            if initial == "temp" {
                file(&fixture.output.join(temp_name()), b"durable", 0o600);
            }
            let held = HeldInput::open(&fixture.input).unwrap();
            let calls = Cell::new(0);
            run(&fixture, &held, &mut no_fault, &calls).unwrap();
            run(&fixture, &held, &mut no_fault, &calls).unwrap();
            assert_eq!(calls.get(), 2);
        }
    }
    #[test]
    fn d0_output_transaction_rejects_closed_states() {
        for state in "mismatch,extra,symlink,wrong-mode,both,temp-symlink,temp-mode,temp-name,temp-empty,temp-partial".split(',') {
            let fixture = Fixture::new();
            let final_path = fixture.output.join("result.bin");
            let temp = fixture.output.join(temp_name());
            match state {
                "mismatch" => file(&final_path, b"wrong", 0o600),
                "extra" => file(&fixture.output.join("extra"), b"x", 0o600),
                "symlink" => symlink(&fixture.input, &final_path).unwrap(),
                "wrong-mode" => file(&final_path, b"durable", 0o644),
                "both" => {
                    file(&final_path, b"durable", 0o600);
                    file(&temp, b"durable", 0o600);
                }
                "temp-symlink" => symlink(&fixture.input, &temp).unwrap(),
                "temp-mode" => file(&temp, b"durable", 0o644),
                "temp-name" => file(&fixture.output.join(".output.wrong.tmp"), b"durable", 0o600),
                "temp-empty" => file(&temp, b"", 0o600),
                "temp-partial" => file(&temp, b"dur", 0o600),
                _ => unreachable!(),
            }
            let before = snapshot(&fixture);
            let calls = Cell::new(0);
            let held = HeldInput::open(&fixture.input).unwrap();
            assert!(run(&fixture, &held, &mut no_fault, &calls).is_err(), "{state}");
            assert_eq!(calls.get(), 0, "{state}");
            assert_eq!(snapshot(&fixture), before, "{state}");
        }
    }
    #[test]
    fn d0_output_transaction_rejects_paths_before_mutation() {
        let fixture = Fixture::new();
        let held = HeldInput::open(&fixture.input).unwrap();
        mode(&fixture.output, 0o755);
        assert!(publish_err(&held, &fixture.output, OsStr::new("x")));
        mode(&fixture.output, 0o700);
        let link = fixture.output.with_file_name("link");
        symlink(&fixture.output, &link).unwrap();
        assert!(publish_err(&held, &link, OsStr::new("x")));
        assert!(publish_err(&held, Path::new("relative"), OsStr::new("x")));
        assert!(publish_err(&held, &fixture.output, OsStr::new("../x")));
        assert!(!fixture.output.join("x").exists());
        let directory = OutputDirectory::open(&fixture.output).unwrap();
        let file_owner = u64::from(s::metadata(&fixture.input).unwrap().uid());
        assert_eq!(effective_owner().unwrap(), file_owner);
        assert!(!valid_directory(directory.metadata, file_owner + 1));
        for suffix in ["/", "/.", "/..", "//nested"] {
            let path = std::path::PathBuf::from(format!("{}{suffix}", fixture.output.display()));
            assert!(canonical_components(&path).is_err());
        }
    }
    #[test]
    fn d0_output_transaction_namespace_replacement_blocks_success() {
        for replacement in ["input", "final", "ancestor"] {
            let fixture = Fixture::new();
            let held = HeldInput::open(&fixture.input).unwrap();
            let calls = Cell::new(0);
            let mut fault = |point| {
                if point != F::DirectoryFsync {
                    return Ok(());
                }
                match replacement {
                    "input" => {
                        s::rename(&fixture.input, fixture.input.with_file_name("old")).unwrap();
                        s::write(&fixture.input, b"input").unwrap();
                    }
                    "final" => {
                        let final_path = fixture.output.join("result.bin");
                        s::rename(&final_path, fixture.output.join("old")).unwrap();
                        s::write(final_path, b"durable").unwrap();
                    }
                    "ancestor" => {
                        let old = fixture.output.with_file_name("old-output");
                        s::rename(&fixture.output, &old).unwrap();
                        s::create_dir(&fixture.output).unwrap();
                        mode(&fixture.output, 0o700);
                        s::write(fixture.output.join("result.bin"), b"durable").unwrap();
                    }
                    _ => unreachable!(),
                }
                Ok(())
            };
            assert!(run(&fixture, &held, &mut fault, &calls).is_err());
            assert_eq!(calls.get(), 0, "{replacement}");
        }
    }
    #[test]
    fn d0_output_transaction_faults_do_not_publish_and_retry_converges() {
        for injected in [
            F::Write,
            F::TempFsync,
            F::Rename,
            F::FinalFsync,
            F::DirectoryFsync,
            F::CleanupUnlink,
            F::CleanupDirectoryFsync,
        ] {
            let fixture = Fixture::new();
            let held = HeldInput::open(&fixture.input).unwrap();
            let calls = Cell::new(0);
            let attempts = Cell::new(0);
            let cleanup_fails = matches!(injected, F::CleanupUnlink | F::CleanupDirectoryFsync);
            let write_fails = injected == F::Write || cleanup_fails;
            let collision = injected == F::Rename;
            let mut fault = |point| {
                let bit = u8::from(point == F::CleanupUnlink)
                    | u8::from(point == F::CleanupDirectoryFsync) << 1;
                attempts.set(attempts.get() | bit);
                if point == F::Write && write_fails {
                    assert_eq!(s::read(fixture.output.join(temp_name())).unwrap(), b"d");
                    Err(io::Error::other("write failed"))
                } else if point == injected && collision {
                    s::write(fixture.output.join("result.bin"), b"intruder")?;
                    Ok(())
                } else if point == injected {
                    Err(io::Error::other("cleanup or boundary injected"))
                } else {
                    Ok(())
                }
            };
            let error = run(&fixture, &held, &mut fault, &calls).unwrap_err();
            assert_eq!(calls.get(), 0, "{injected:?}");
            if cleanup_fails {
                assert!(error.to_string().contains("write failed; cleanup failed"));
                assert_eq!(attempts.get(), 3);
            }
            if injected == F::CleanupUnlink {
                let before = snapshot(&fixture);
                assert!(run(&fixture, &held, &mut no_fault, &calls).is_err());
                assert_eq!(snapshot(&fixture), before);
                continue;
            }
            if matches!(injected, F::Write | F::CleanupDirectoryFsync) {
                assert!(!fixture.output.join(temp_name()).exists());
            }
            if injected == F::Rename {
                let final_path = fixture.output.join("result.bin");
                assert_eq!(s::read(&final_path).unwrap(), b"intruder");
                assert!(fixture.output.join(temp_name()).exists());
                s::remove_file(final_path).unwrap();
            }
            run(&fixture, &held, &mut no_fault, &calls).unwrap();
            assert_eq!(calls.get(), 1);
            assert!(!fixture.output.join(temp_name()).exists());
        }
    }
}
