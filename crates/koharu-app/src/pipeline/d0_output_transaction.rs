use super::d0_held_input::HeldInput;
use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, RenameFlags, fstat, fsync, mkdirat, open, openat,
    renameat_with, statat, unlinkat,
};
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;

const MANIFEST_PREFLIGHT_DIRECTORY: &str = "d0-manifest-preflight";
const MANIFEST_PREFLIGHT_REPORT: &str = "report.json";
const D0_RUNTIME_DIRECTORY: &str = "d0-baseline-runtime";
const D0_RUNTIME_TEMP_DIRECTORY: &str = ".d0-baseline-runtime.tmp";
const D0_RUNTIME_REPORT: &str = "report.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FaultPoint {
    ChildDirectoryCreate,
    ParentDirectoryFsync,
    Write,
    TempFsync,
    Rename,
    FinalFsync,
    DirectoryFsync,
    CleanupUnlink,
    CleanupDirectoryFsync,
    BundleDirectoryCreate,
    BundleFileCreate,
    BundleWrite,
    BundleFileFsync,
    BundleTempDirectoryFsync,
    BundleRename,
    BundleParentFsync,
    BundleCleanupUnlink,
    BundleCleanupDirectoryFsync,
    BundleCleanupRmdir,
    BundleCleanupParentFsync,
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

struct ChildOutputDirectory {
    descriptor: OwnedFd,
    metadata: Metadata,
}

pub(super) struct PublishedOutput {
    pub(super) descriptor: File,
    pub(super) bytes: Box<[u8]>,
    pub(super) sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum D0ArtifactRole {
    Source,
    RawSegment,
    FinalEraseMask,
    Inpainted,
    Rendered,
}

impl D0ArtifactRole {
    const ALL: [Self; 5] = [
        Self::Source,
        Self::RawSegment,
        Self::FinalEraseMask,
        Self::Inpainted,
        Self::Rendered,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::RawSegment => "raw_segment",
            Self::FinalEraseMask => "final_erase_mask",
            Self::Inpainted => "inpainted",
            Self::Rendered => "rendered",
        }
    }
}

pub(super) struct D0BundleArtifact {
    pub(super) entry_index: u32,
    pub(super) role: D0ArtifactRole,
    pub(super) bytes: Box<[u8]>,
}

pub(super) struct PublishedBundleFile {
    pub(super) name: OsString,
    pub(super) output: PublishedOutput,
}

pub(super) struct PublishedBundle {
    _descriptor: OwnedFd,
    _metadata: Metadata,
    pub(super) files: Vec<PublishedBundleFile>,
}

struct ExpectedBundleFile {
    name: OsString,
    bytes: Box<[u8]>,
}

pub(super) fn d0_artifact_name(entry_index: u32, role: D0ArtifactRole) -> String {
    format!("entry-{entry_index:04}-{}.png", role.label())
}

pub(super) fn publish_d0_runtime_bundle<T>(
    evidence_root: &Path,
    artifacts: Vec<D0BundleArtifact>,
    report: Vec<u8>,
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
    pre_success_barrier: impl FnOnce(&PublishedBundle) -> io::Result<()>,
    success: impl FnOnce(&PublishedBundle) -> io::Result<T>,
) -> io::Result<T> {
    let parent = OutputDirectory::open(evidence_root)?;
    let expected = expected_bundle_files(artifacts, report)?;
    let published = transact_bundle(&parent, &expected, fault)?;
    let revalidated = revalidate_bundle(&parent, &published, &expected)?;
    pre_success_barrier(&revalidated)?;
    let fresh = revalidate_bundle(&parent, &revalidated, &expected)?;
    success(&fresh)
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
            let published = transact(
                directory.descriptor.as_fd(),
                directory.metadata.owner,
                final_name,
                &temp_name,
                output,
                fault,
            )?;
            input.with_revalidated_path(|validation| {
                validation.with_current_namespace(|| {
                    directory.revalidate(final_name, &published)?;
                    success.take().expect("success callback called once")(&published)
                })
            })
        })
    })
}

pub(super) fn publish_manifest_preflight_report<T>(
    evidence_root: &Path,
    output: &[u8],
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
    success: impl FnOnce(&PublishedOutput) -> io::Result<T>,
) -> io::Result<T> {
    let parent = OutputDirectory::open(evidence_root)?;
    let directory = ChildOutputDirectory::open_or_create(&parent, fault)?;
    let output = output.to_vec().into_boxed_slice();
    let temp_name = OsString::from(format!(".output.{}.tmp", hex(&hash(&output))));
    let mut published = transact(
        directory.descriptor.as_fd(),
        directory.metadata.owner,
        OsStr::new(MANIFEST_PREFLIGHT_REPORT),
        &temp_name,
        output,
        fault,
    )?;
    directory.revalidate(&parent, &mut published)?;
    success(&published)
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
        let fresh = self.reopen()?;
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

    fn reopen(&self) -> io::Result<OwnedFd> {
        let (fresh, metadata, ancestors) = walk(self.root.as_fd(), &self.components)?;
        require(
            metadata == self.metadata && ancestors == self.ancestors,
            "output directory namespace changed",
        )?;
        Ok(fresh)
    }
}

impl ChildOutputDirectory {
    fn open_or_create(
        parent: &OutputDirectory,
        fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
    ) -> io::Result<Self> {
        let name = OsStr::new(MANIFEST_PREFLIGHT_DIRECTORY);
        validate_child(name)?;
        let descriptor = match open_directory_at(parent.descriptor.as_fd(), name) {
            Ok(descriptor) => descriptor,
            Err(error) if error.raw_os_error() == Some(rustix::io::Errno::NOENT.raw_os_error()) => {
                fault(F::ChildDirectoryCreate)?;
                fs(mkdirat(
                    parent.descriptor.as_fd(),
                    name,
                    Mode::from_raw_mode(0o700),
                ))?;
                open_directory_at(parent.descriptor.as_fd(), name)?
            }
            Err(error) => return Err(error),
        };
        let metadata = metadata_of(&fs(fstat(&descriptor))?);
        require(
            valid_directory(metadata, parent.metadata.owner),
            "invalid child output directory",
        )?;
        fault(F::ParentDirectoryFsync)?;
        fs(fsync(&parent.descriptor))?;
        Ok(Self {
            descriptor,
            metadata,
        })
    }

    fn revalidate(
        &self,
        parent: &OutputDirectory,
        published: &mut PublishedOutput,
    ) -> io::Result<()> {
        let fresh_parent = parent.reopen()?;
        let fresh = open_directory_at(
            fresh_parent.as_fd(),
            OsStr::new(MANIFEST_PREFLIGHT_DIRECTORY),
        )?;
        let fresh_metadata = metadata_of(&fs(fstat(&fresh))?);
        require(
            fresh_metadata == self.metadata
                && valid_directory(fresh_metadata, parent.metadata.owner),
            "child output directory namespace changed",
        )?;
        let final_name = OsStr::new(MANIFEST_PREFLIGHT_REPORT);
        let temp_name = OsString::from(format!(".output.{}.tmp", hex(&hash(&published.bytes))));
        require(
            state(fresh.as_fd(), final_name, &temp_name)? == State::Final,
            "published output state changed",
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
            "published report namespace or metadata changed",
        )?;
        let reread = read_exact(&mut published.descriptor, &published.bytes)?;
        require(
            reread.as_ref() == published.bytes.as_ref(),
            "published report bytes changed",
        )
    }
}

fn open_directory_at(parent: BorrowedFd<'_>, name: &OsStr) -> io::Result<OwnedFd> {
    let descriptor = fs(openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ))?;
    let metadata = metadata_of(&fs(fstat(&descriptor))?);
    require(
        metadata.file_type.is_dir(),
        "output child is not a directory",
    )?;
    Ok(descriptor)
}

fn expected_bundle_files(
    mut artifacts: Vec<D0BundleArtifact>,
    report: Vec<u8>,
) -> io::Result<Vec<ExpectedBundleFile>> {
    require(!artifacts.is_empty(), "runtime bundle artifacts are empty")?;
    require(!report.is_empty(), "runtime bundle report is empty")?;
    artifacts.sort_by_key(|artifact| (artifact.entry_index, artifact.role));
    let entries = artifacts.len() / D0ArtifactRole::ALL.len();
    require(
        entries * D0ArtifactRole::ALL.len() == artifacts.len(),
        "runtime bundle artifact cardinality mismatch",
    )?;
    for (position, artifact) in artifacts.iter().enumerate() {
        let entry_index = u32::try_from(position / D0ArtifactRole::ALL.len())
            .map_err(|_| invalid("runtime bundle entry count overflow"))?;
        let role = D0ArtifactRole::ALL[position % D0ArtifactRole::ALL.len()];
        require(
            artifact.entry_index == entry_index
                && artifact.entry_index <= 9_999
                && artifact.role == role,
            "runtime bundle artifacts are not closed and contiguous",
        )?;
    }
    let mut expected = artifacts
        .into_iter()
        .map(|artifact| ExpectedBundleFile {
            name: d0_artifact_name(artifact.entry_index, artifact.role).into(),
            bytes: artifact.bytes,
        })
        .collect::<Vec<_>>();
    expected.push(ExpectedBundleFile {
        name: D0_RUNTIME_REPORT.into(),
        bytes: report.into_boxed_slice(),
    });
    expected.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
    Ok(expected)
}

fn transact_bundle(
    parent: &OutputDirectory,
    expected: &[ExpectedBundleFile],
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
) -> io::Result<PublishedBundle> {
    let parent_descriptor = parent.reopen()?;
    let temp = optional_bundle_directory(
        parent_descriptor.as_fd(),
        OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
        parent.metadata.owner,
    )?;
    let final_directory = optional_bundle_directory(
        parent_descriptor.as_fd(),
        OsStr::new(D0_RUNTIME_DIRECTORY),
        parent.metadata.owner,
    )?;
    match (temp, final_directory) {
        (Some(_), Some(_)) => Err(io::Error::other(
            "runtime bundle temp and final directories both exist",
        )),
        (None, Some(final_directory)) => {
            let published = read_bundle(final_directory, parent.metadata.owner, expected)?;
            sync_bundle_contents(&published, fault)?;
            fault(F::BundleParentFsync)?;
            fs(fsync(&parent_descriptor))?;
            Ok(published)
        }
        (Some(temp), None) => {
            let temp_metadata = metadata_of(&fs(fstat(&temp))?);
            let (names, exact) = inspect_recoverable_temp(&temp, parent.metadata.owner, expected)?;
            if !exact {
                cleanup_recoverable_temp(
                    parent_descriptor.as_fd(),
                    &temp,
                    temp_metadata,
                    &names,
                    fault,
                )?;
                return create_and_publish_bundle(parent, expected, fault);
            }
            let published = read_bundle(temp, parent.metadata.owner, expected)?;
            sync_bundle_contents(&published, fault)?;
            fault(F::BundleRename)?;
            let fresh_parent = parent.reopen()?;
            require_named_directory(
                fresh_parent.as_fd(),
                OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
                temp_metadata,
                parent.metadata.owner,
            )?;
            fs(renameat_with(
                fresh_parent.as_fd(),
                OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
                fresh_parent.as_fd(),
                OsStr::new(D0_RUNTIME_DIRECTORY),
                RenameFlags::NOREPLACE,
            ))?;
            fault(F::BundleParentFsync)?;
            fs(fsync(&fresh_parent))?;
            Ok(published)
        }
        (None, None) => create_and_publish_bundle(parent, expected, fault),
    }
}

fn create_and_publish_bundle(
    parent: &OutputDirectory,
    expected: &[ExpectedBundleFile],
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
) -> io::Result<PublishedBundle> {
    let parent_descriptor = parent.reopen()?;
    fault(F::BundleDirectoryCreate)?;
    fs(mkdirat(
        parent_descriptor.as_fd(),
        OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
        Mode::from_raw_mode(0o700),
    ))?;
    let temp = open_directory_at(
        parent_descriptor.as_fd(),
        OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
    )?;
    let temp_metadata = metadata_of(&fs(fstat(&temp))?);
    require(
        valid_directory(temp_metadata, parent.metadata.owner),
        "invalid runtime bundle temp directory",
    )?;

    for expected_file in expected {
        let result = (|| {
            fault(F::BundleFileCreate)?;
            let descriptor = fs(openat(
                temp.as_fd(),
                &expected_file.name,
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            ))?;
            require_file(&descriptor, parent.metadata.owner)?;
            let mut file = File::from(descriptor);
            let split = expected_file.bytes.len().min(1);
            file.write_all(&expected_file.bytes[..split])?;
            fault(F::BundleWrite)?;
            file.write_all(&expected_file.bytes[split..])?;
            fault(F::BundleFileFsync)?;
            fs(fsync(&file))
        })();
        if let Err(error) = result {
            return Err(cleanup_bundle_after_error(
                parent_descriptor.as_fd(),
                &temp,
                temp_metadata,
                expected,
                error,
                fault,
            ));
        }
    }
    if let Err(error) = fault(F::BundleTempDirectoryFsync).and_then(|()| fs(fsync(&temp))) {
        return Err(cleanup_bundle_after_error(
            parent_descriptor.as_fd(),
            &temp,
            temp_metadata,
            expected,
            error,
            fault,
        ));
    }
    let published = read_bundle(temp, parent.metadata.owner, expected)?;
    fault(F::BundleRename)?;
    let fresh_parent = parent.reopen()?;
    require_named_directory(
        fresh_parent.as_fd(),
        OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
        temp_metadata,
        parent.metadata.owner,
    )?;
    fs(renameat_with(
        fresh_parent.as_fd(),
        OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
        fresh_parent.as_fd(),
        OsStr::new(D0_RUNTIME_DIRECTORY),
        RenameFlags::NOREPLACE,
    ))?;
    fault(F::BundleParentFsync)?;
    fs(fsync(&fresh_parent))?;
    Ok(published)
}

fn optional_bundle_directory(
    parent: BorrowedFd<'_>,
    name: &OsStr,
    owner: u64,
) -> io::Result<Option<OwnedFd>> {
    match open_directory_at(parent, name) {
        Ok(descriptor) => {
            let metadata = metadata_of(&fs(fstat(&descriptor))?);
            require(
                valid_directory(metadata, owner),
                "invalid runtime bundle directory",
            )?;
            Ok(Some(descriptor))
        }
        Err(error) if error.raw_os_error() == Some(rustix::io::Errno::NOENT.raw_os_error()) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn require_named_directory(
    parent: BorrowedFd<'_>,
    name: &OsStr,
    expected: Metadata,
    owner: u64,
) -> io::Result<()> {
    let named = open_directory_at(parent, name)?;
    let metadata = metadata_of(&fs(fstat(&named))?);
    require(
        metadata == expected && valid_directory(metadata, owner),
        "runtime bundle directory namespace changed",
    )
}

fn read_bundle(
    descriptor: OwnedFd,
    owner: u64,
    expected: &[ExpectedBundleFile],
) -> io::Result<PublishedBundle> {
    let metadata = metadata_of(&fs(fstat(&descriptor))?);
    require(
        valid_directory(metadata, owner),
        "invalid runtime bundle directory",
    )?;
    let names = directory_names(descriptor.as_fd())?;
    require(
        names
            .iter()
            .map(OsString::as_os_str)
            .eq(expected.iter().map(|file| file.name.as_os_str())),
        "runtime bundle entries are unknown or partial",
    )?;
    let mut files = Vec::with_capacity(expected.len());
    for expected_file in expected {
        let file = open_regular(
            descriptor.as_fd(),
            &expected_file.name,
            owner,
            OFlags::RDONLY,
        )?;
        files.push(PublishedBundleFile {
            name: expected_file.name.clone(),
            output: read_published(file, expected_file.bytes.clone())?,
        });
    }
    Ok(PublishedBundle {
        _descriptor: descriptor,
        _metadata: metadata,
        files,
    })
}

fn directory_names(directory: BorrowedFd<'_>) -> io::Result<Vec<OsString>> {
    let mut names = Dir::read_from(directory)?
        .map(|entry| entry.map(|entry| OsStr::from_bytes(entry.file_name().to_bytes()).to_owned()))
        .collect::<rustix::io::Result<Vec<_>>>()
        .map_err(io::Error::from)?;
    names.retain(|name| name != "." && name != "..");
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    Ok(names)
}

fn inspect_recoverable_temp(
    temp: &OwnedFd,
    owner: u64,
    expected: &[ExpectedBundleFile],
) -> io::Result<(Vec<OsString>, bool)> {
    let names = directory_names(temp.as_fd())?;
    let mut exact = names.len() == expected.len();
    for name in &names {
        let expected_file = expected
            .iter()
            .find(|candidate| candidate.name == *name)
            .ok_or_else(|| io::Error::other("runtime temp contains an unknown entry"))?;
        let bytes = read_bounded_regular(temp.as_fd(), name, owner, expected_file.bytes.len())?;
        require(
            expected_file.bytes.starts_with(&bytes),
            "runtime temp file is not an expected byte prefix",
        )?;
        exact &= bytes.len() == expected_file.bytes.len();
    }
    Ok((names, exact))
}

fn read_bounded_regular(
    directory: BorrowedFd<'_>,
    name: &OsStr,
    owner: u64,
    maximum: usize,
) -> io::Result<Box<[u8]>> {
    let descriptor = open_regular(directory, name, owner, OFlags::RDONLY)?;
    let limit = u64::try_from(maximum)
        .map_err(|_| invalid("runtime bundle file size overflow"))?
        .checked_add(1)
        .ok_or_else(|| invalid("runtime bundle file size overflow"))?;
    let mut bytes = Vec::with_capacity(maximum.min(64 * 1024));
    descriptor.take(limit).read_to_end(&mut bytes)?;
    require(
        bytes.len() <= maximum,
        "runtime temp file exceeds expected bytes",
    )?;
    Ok(bytes.into_boxed_slice())
}

fn sync_bundle_contents(
    published: &PublishedBundle,
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
) -> io::Result<()> {
    for file in &published.files {
        fault(F::BundleFileFsync)?;
        fs(fsync(&file.output.descriptor))?;
    }
    fault(F::BundleTempDirectoryFsync)?;
    fs(fsync(&published._descriptor))
}

fn revalidate_bundle(
    parent: &OutputDirectory,
    published: &PublishedBundle,
    expected: &[ExpectedBundleFile],
) -> io::Result<PublishedBundle> {
    let fresh_parent = parent.reopen()?;
    let fresh = open_directory_at(fresh_parent.as_fd(), OsStr::new(D0_RUNTIME_DIRECTORY))?;
    let fresh_metadata = metadata_of(&fs(fstat(&fresh))?);
    require(
        fresh_metadata == published._metadata
            && valid_directory(fresh_metadata, parent.metadata.owner),
        "published runtime bundle namespace changed",
    )?;
    let revalidated = read_bundle(fresh, parent.metadata.owner, expected)?;
    for (held, named) in published.files.iter().zip(&revalidated.files) {
        let held_metadata = metadata_of(&fs(fstat(&held.output.descriptor))?);
        let named_metadata = metadata_of(&fs(fstat(&named.output.descriptor))?);
        require(
            held.name == named.name
                && held_metadata == named_metadata
                && held.output.bytes == named.output.bytes
                && held.output.sha256 == named.output.sha256,
            "published runtime bundle file changed",
        )?;
    }
    require_absent(fresh_parent.as_fd(), OsStr::new(D0_RUNTIME_TEMP_DIRECTORY))?;
    Ok(revalidated)
}

fn require_absent(parent: BorrowedFd<'_>, name: &OsStr) -> io::Result<()> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err(io::Error::other(
            "runtime bundle temp entry exists during success revalidation",
        )),
        Err(error) => Err(error.into()),
    }
}

fn cleanup_bundle_after_error(
    parent: BorrowedFd<'_>,
    temp: &OwnedFd,
    temp_metadata: Metadata,
    expected: &[ExpectedBundleFile],
    source: io::Error,
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
) -> io::Error {
    let cleanup =
        inspect_recoverable_temp(temp, temp_metadata.owner, expected).and_then(|(names, _)| {
            cleanup_recoverable_temp(parent, temp, temp_metadata, &names, fault)
        });
    match cleanup {
        Ok(()) => source,
        Err(cleanup) => io::Error::other(format!("{source}; cleanup failed: {cleanup}")),
    }
}

fn cleanup_recoverable_temp(
    parent: BorrowedFd<'_>,
    temp: &OwnedFd,
    temp_metadata: Metadata,
    names: &[OsString],
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
) -> io::Result<()> {
    for name in names {
        fault(F::BundleCleanupUnlink)?;
        fs(unlinkat(temp.as_fd(), name, AtFlags::empty()))?;
    }
    fault(F::BundleCleanupDirectoryFsync)?;
    fs(fsync(temp))?;
    fault(F::BundleCleanupRmdir)?;
    require(
        directory_names(temp.as_fd())?.is_empty(),
        "runtime temp changed during cleanup",
    )?;
    require_named_directory(
        parent,
        OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
        temp_metadata,
        temp_metadata.owner,
    )?;
    fs(unlinkat(
        parent,
        OsStr::new(D0_RUNTIME_TEMP_DIRECTORY),
        AtFlags::REMOVEDIR,
    ))?;
    fault(F::BundleCleanupParentFsync)?;
    fs(fsync(parent))
}

fn transact(
    directory: BorrowedFd<'_>,
    owner: u64,
    final_name: &OsStr,
    temp_name: &OsStr,
    output: Box<[u8]>,
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
) -> io::Result<PublishedOutput> {
    let temp = match state(directory, final_name, temp_name)? {
        State::Empty => {
            let descriptor = fs(openat(
                directory,
                temp_name,
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            ))?;
            require_file(&descriptor, owner)?;
            let mut temp = File::from(descriptor);
            let split = output.len().min(1);
            let write = temp
                .write_all(&output[..split])
                .and_then(|()| fault(F::Write))
                .and_then(|()| temp.write_all(&output[split..]));
            if let Err(error) = write {
                drop(temp);
                return match cleanup_created(directory, temp_name, fault) {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(io::Error::other(format!("{error}; {cleanup}"))),
                };
            }
            temp
        }
        State::Temp => open_exact(directory, temp_name, owner, &output)?,
        State::Final => {
            let final_file = open_regular(directory, final_name, owner, OFlags::RDONLY)?;
            let published = read_published(final_file, output)?;
            fault(F::FinalFsync)?;
            fs(fsync(&published.descriptor))?;
            fault(F::DirectoryFsync)?;
            fs(fsync(directory))?;
            return Ok(published);
        }
    };
    fault(F::TempFsync)?;
    fs(fsync(&temp))?;
    fault(F::Rename)?;
    fs(renameat_with(
        directory,
        temp_name,
        directory,
        final_name,
        RenameFlags::NOREPLACE,
    ))?;
    fault(F::FinalFsync)?;
    fs(fsync(&temp))?;
    fault(F::DirectoryFsync)?;
    fs(fsync(directory))?;
    read_published(temp, output)
}
#[derive(PartialEq, Eq)]
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
    let (socket, _peer) = UnixStream::pair()?;
    let owner = metadata_of(&fs(fstat(&socket))?).owner;
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
    fn report_child(fixture: &Fixture) -> std::path::PathBuf {
        fixture.output.join(MANIFEST_PREFLIGHT_DIRECTORY)
    }
    fn report_temp_name() -> String {
        format!(".output.{}.tmp", hex(&hash(b"{\"ok\":true}\n")))
    }
    fn prepare_report_child(fixture: &Fixture) -> std::path::PathBuf {
        let child = report_child(fixture);
        s::create_dir(&child).unwrap();
        mode(&child, 0o700);
        child
    }
    fn run_report(
        fixture: &Fixture,
        fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
        calls: &Cell<usize>,
    ) -> io::Result<()> {
        publish_manifest_preflight_report(&fixture.output, b"{\"ok\":true}\n", fault, |published| {
            calls.set(calls.get() + 1);
            assert_eq!(&*published.bytes, b"{\"ok\":true}\n");
            Ok(())
        })
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
    fn runtime_artifacts() -> Vec<D0BundleArtifact> {
        D0ArtifactRole::ALL
            .into_iter()
            .map(|role| D0BundleArtifact {
                entry_index: 0,
                role,
                bytes: format!("bytes-{}", role.label())
                    .into_bytes()
                    .into_boxed_slice(),
            })
            .collect()
    }
    fn run_bundle(
        fixture: &Fixture,
        fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
        calls: &Cell<usize>,
    ) -> io::Result<()> {
        run_bundle_with_barrier(fixture, fault, |_| Ok(()), calls)
    }
    fn run_bundle_with_barrier(
        fixture: &Fixture,
        fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
        barrier: impl FnOnce(&PublishedBundle) -> io::Result<()>,
        calls: &Cell<usize>,
    ) -> io::Result<()> {
        publish_d0_runtime_bundle(
            &fixture.output,
            runtime_artifacts(),
            b"{\"schema\":\"runtime\"}\n".to_vec(),
            fault,
            barrier,
            |published| {
                calls.set(calls.get() + 1);
                assert_eq!(published.files.len(), 6);
                assert_eq!(
                    published.files.last().unwrap().name,
                    OsStr::new(D0_RUNTIME_REPORT)
                );
                Ok(())
            },
        )
    }
    fn runtime_expected() -> Vec<ExpectedBundleFile> {
        expected_bundle_files(runtime_artifacts(), b"{\"schema\":\"runtime\"}\n".to_vec()).unwrap()
    }
    fn prepare_runtime_directory(fixture: &Fixture, name: &str) -> std::path::PathBuf {
        let directory = fixture.output.join(name);
        s::create_dir(&directory).unwrap();
        mode(&directory, 0o700);
        for expected in runtime_expected() {
            file(&directory.join(expected.name), &expected.bytes, 0o600);
        }
        directory
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
    fn d0_output_transaction_anonymous_socket_owner_matches_current_user_artifact() {
        let fixture = Fixture::new();
        assert_eq!(
            effective_owner().unwrap(),
            u64::from(s::metadata(&fixture.input).unwrap().uid())
        );
    }
    #[test]
    fn d0_output_transaction_report_recovers_and_is_idempotent() {
        for initial in ["absent", "empty", "temp", "final"] {
            let fixture = Fixture::new();
            if initial != "absent" {
                let child = prepare_report_child(&fixture);
                match initial {
                    "temp" => file(&child.join(report_temp_name()), b"{\"ok\":true}\n", 0o600),
                    "final" => file(
                        &child.join(MANIFEST_PREFLIGHT_REPORT),
                        b"{\"ok\":true}\n",
                        0o600,
                    ),
                    "empty" => {}
                    _ => unreachable!(),
                }
            }
            let calls = Cell::new(0);
            run_report(&fixture, &mut no_fault, &calls).unwrap();
            run_report(&fixture, &mut no_fault, &calls).unwrap();
            assert_eq!(calls.get(), 2, "{initial}");
            let child = report_child(&fixture);
            assert_eq!(
                s::metadata(&child).unwrap().permissions().mode() & 0o7777,
                0o700
            );
            assert_eq!(
                s::metadata(child.join(MANIFEST_PREFLIGHT_REPORT))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o600
            );
        }
    }
    #[test]
    fn d0_output_transaction_report_rejects_closed_states() {
        for state_name in [
            "child-symlink",
            "child-file",
            "child-mode",
            "unknown",
            "final-mismatch",
            "final-symlink",
            "final-mode",
            "temp-mismatch",
            "temp-symlink",
            "temp-mode",
            "both",
        ] {
            let fixture = Fixture::new();
            let child = report_child(&fixture);
            match state_name {
                "child-symlink" => symlink(&fixture.input, &child).unwrap(),
                "child-file" => file(&child, b"x", 0o600),
                _ => {
                    let child = prepare_report_child(&fixture);
                    match state_name {
                        "child-mode" => mode(&child, 0o755),
                        "unknown" => file(&child.join("unknown"), b"x", 0o600),
                        "final-mismatch" => {
                            file(&child.join(MANIFEST_PREFLIGHT_REPORT), b"wrong", 0o600)
                        }
                        "final-symlink" => {
                            symlink(&fixture.input, child.join(MANIFEST_PREFLIGHT_REPORT)).unwrap()
                        }
                        "final-mode" => file(
                            &child.join(MANIFEST_PREFLIGHT_REPORT),
                            b"{\"ok\":true}\n",
                            0o644,
                        ),
                        "temp-mismatch" => file(&child.join(report_temp_name()), b"wrong", 0o600),
                        "temp-symlink" => {
                            symlink(&fixture.input, child.join(report_temp_name())).unwrap()
                        }
                        "temp-mode" => {
                            file(&child.join(report_temp_name()), b"{\"ok\":true}\n", 0o644)
                        }
                        "both" => {
                            file(&child.join(report_temp_name()), b"{\"ok\":true}\n", 0o600);
                            file(
                                &child.join(MANIFEST_PREFLIGHT_REPORT),
                                b"{\"ok\":true}\n",
                                0o600,
                            );
                        }
                        _ => unreachable!(),
                    }
                }
            }
            let calls = Cell::new(0);
            assert!(
                run_report(&fixture, &mut no_fault, &calls).is_err(),
                "{state_name}"
            );
            assert_eq!(calls.get(), 0, "{state_name}");
        }
    }
    #[test]
    fn d0_output_transaction_report_directory_faults_retry_to_success() {
        for injected in [F::ChildDirectoryCreate, F::ParentDirectoryFsync] {
            let fixture = Fixture::new();
            let calls = Cell::new(0);
            let mut fault = |point| {
                if point == injected {
                    Err(io::Error::other("directory boundary injected"))
                } else {
                    Ok(())
                }
            };
            assert!(run_report(&fixture, &mut fault, &calls).is_err());
            assert_eq!(calls.get(), 0);
            assert_eq!(
                report_child(&fixture).exists(),
                injected == F::ParentDirectoryFsync
            );
            run_report(&fixture, &mut no_fault, &calls).unwrap();
            assert_eq!(calls.get(), 1);
        }
    }
    #[test]
    fn d0_output_transaction_report_namespace_replacement_blocks_success() {
        for replacement in ["root", "child", "final"] {
            let fixture = Fixture::new();
            let calls = Cell::new(0);
            let mut fault = |point| {
                if point != F::DirectoryFsync {
                    return Ok(());
                }
                let child = report_child(&fixture);
                match replacement {
                    "root" => {
                        s::rename(&fixture.output, fixture.output.with_file_name("old-root"))?;
                        s::create_dir(&fixture.output)?;
                        mode(&fixture.output, 0o700);
                        let child = prepare_report_child(&fixture);
                        file(
                            &child.join(MANIFEST_PREFLIGHT_REPORT),
                            b"{\"ok\":true}\n",
                            0o600,
                        );
                    }
                    "child" => {
                        s::rename(&child, fixture.output.join("old-child"))?;
                        let child = prepare_report_child(&fixture);
                        file(
                            &child.join(MANIFEST_PREFLIGHT_REPORT),
                            b"{\"ok\":true}\n",
                            0o600,
                        );
                    }
                    "final" => {
                        let final_path = child.join(MANIFEST_PREFLIGHT_REPORT);
                        s::rename(&final_path, child.join("old-report"))?;
                        file(&final_path, b"{\"ok\":true}\n", 0o600);
                    }
                    _ => unreachable!(),
                }
                Ok(())
            };
            assert!(run_report(&fixture, &mut fault, &calls).is_err());
            assert_eq!(calls.get(), 0, "{replacement}");
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

    #[test]
    fn d0_output_transaction_runtime_bundle_is_atomic_exact_and_idempotent() {
        let fixture = Fixture::new();
        let calls = Cell::new(0);
        run_bundle(&fixture, &mut no_fault, &calls).unwrap();
        run_bundle(&fixture, &mut no_fault, &calls).unwrap();
        assert_eq!(calls.get(), 2);
        assert!(!fixture.output.join(D0_RUNTIME_TEMP_DIRECTORY).exists());
        let final_directory = fixture.output.join(D0_RUNTIME_DIRECTORY);
        assert_eq!(
            s::metadata(&final_directory).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        let mut names = s::read_dir(&final_directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            runtime_expected()
                .into_iter()
                .map(|file| file.name)
                .collect::<Vec<_>>()
        );
        assert!(names.iter().all(|name| {
            s::metadata(final_directory.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
                == 0o600
        }));
    }

    #[test]
    fn d0_output_transaction_runtime_bundle_recovery_resyncs_before_rename() {
        let fixture = Fixture::new();
        prepare_runtime_directory(&fixture, D0_RUNTIME_TEMP_DIRECTORY);
        let calls = Cell::new(0);
        let mut ordering = Vec::new();
        run_bundle(
            &fixture,
            &mut |point| {
                if matches!(
                    point,
                    F::BundleFileFsync
                        | F::BundleTempDirectoryFsync
                        | F::BundleRename
                        | F::BundleParentFsync
                ) {
                    ordering.push(point);
                }
                Ok(())
            },
            &calls,
        )
        .unwrap();
        let mut expected = vec![F::BundleFileFsync; runtime_expected().len()];
        expected.extend([
            F::BundleTempDirectoryFsync,
            F::BundleRename,
            F::BundleParentFsync,
        ]);
        assert_eq!(ordering, expected);
        assert_eq!(calls.get(), 1);

        let fixture = Fixture::new();
        prepare_runtime_directory(&fixture, D0_RUNTIME_TEMP_DIRECTORY);
        let calls = Cell::new(0);
        let mut fault = |point| {
            if point == F::BundleTempDirectoryFsync {
                Err(io::Error::other("recovered temp directory fsync injected"))
            } else {
                Ok(())
            }
        };
        assert!(run_bundle(&fixture, &mut fault, &calls).is_err());
        assert_eq!(calls.get(), 0);
        assert!(!fixture.output.join(D0_RUNTIME_DIRECTORY).exists());
        assert!(fixture.output.join(D0_RUNTIME_TEMP_DIRECTORY).is_dir());
        run_bundle(&fixture, &mut no_fault, &calls).unwrap();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn d0_output_transaction_runtime_bundle_recovers_owned_partial_temp() {
        for state in ["empty", "prefix", "complete-subset", "mixed"] {
            let fixture = Fixture::new();
            let directory = fixture.output.join(D0_RUNTIME_TEMP_DIRECTORY);
            s::create_dir(&directory).unwrap();
            mode(&directory, 0o700);
            let expected = runtime_expected();
            match state {
                "empty" => {}
                "prefix" => file(
                    &directory.join(&expected[0].name),
                    &expected[0].bytes[..1],
                    0o600,
                ),
                "complete-subset" => file(
                    &directory.join(&expected[0].name),
                    &expected[0].bytes,
                    0o600,
                ),
                "mixed" => {
                    file(
                        &directory.join(&expected[0].name),
                        &expected[0].bytes,
                        0o600,
                    );
                    file(
                        &directory.join(&expected[1].name),
                        &expected[1].bytes[..1],
                        0o600,
                    );
                }
                _ => unreachable!(),
            }
            let calls = Cell::new(0);
            run_bundle(&fixture, &mut no_fault, &calls).unwrap();
            assert_eq!(calls.get(), 1, "{state}");
            assert!(!directory.exists(), "{state}");
            let parent = OutputDirectory::open(&fixture.output).unwrap();
            assert!(
                read_bundle(
                    open_directory_at(parent.descriptor.as_fd(), OsStr::new(D0_RUNTIME_DIRECTORY),)
                        .unwrap(),
                    parent.metadata.owner,
                    &expected,
                )
                .is_ok(),
                "{state}"
            );
        }
    }

    #[test]
    fn d0_output_transaction_runtime_bundle_cleanup_faults_retry_to_success() {
        for injected in [
            F::BundleCleanupUnlink,
            F::BundleCleanupDirectoryFsync,
            F::BundleCleanupRmdir,
            F::BundleCleanupParentFsync,
        ] {
            let fixture = Fixture::new();
            let calls = Cell::new(0);
            let mut write_failed = false;
            let mut cleanup_failed = false;
            let mut fault = |point| {
                if point == F::BundleWrite && !write_failed {
                    write_failed = true;
                    Err(io::Error::other("bundle write injected"))
                } else if point == injected && !cleanup_failed {
                    cleanup_failed = true;
                    Err(io::Error::other("bundle cleanup injected"))
                } else {
                    Ok(())
                }
            };
            assert!(run_bundle(&fixture, &mut fault, &calls).is_err());
            assert!(write_failed && cleanup_failed, "{injected:?}");
            assert_eq!(calls.get(), 0, "{injected:?}");
            assert!(!fixture.output.join(D0_RUNTIME_DIRECTORY).exists());

            run_bundle(&fixture, &mut no_fault, &calls).unwrap();
            assert_eq!(calls.get(), 1, "{injected:?}");
            assert!(!fixture.output.join(D0_RUNTIME_TEMP_DIRECTORY).exists());
        }
    }

    #[test]
    fn d0_output_transaction_runtime_bundle_barrier_precedes_fresh_output_revalidation() {
        for replacement in ["directory", "file"] {
            let fixture = Fixture::new();
            let calls = Cell::new(0);
            let barrier_calls = Cell::new(0);
            let result = run_bundle_with_barrier(
                &fixture,
                &mut no_fault,
                |_| {
                    barrier_calls.set(barrier_calls.get() + 1);
                    let final_directory = fixture.output.join(D0_RUNTIME_DIRECTORY);
                    match replacement {
                        "directory" => {
                            s::rename(&final_directory, fixture.output.join("old-final"))?;
                            prepare_runtime_directory(&fixture, D0_RUNTIME_DIRECTORY);
                        }
                        "file" => {
                            let expected = runtime_expected().remove(0);
                            let path = final_directory.join(&expected.name);
                            s::rename(&path, final_directory.join("old-file"))?;
                            file(&path, &expected.bytes, 0o600);
                        }
                        _ => unreachable!(),
                    }
                    Ok(())
                },
                &calls,
            );
            assert!(result.is_err(), "{replacement}");
            assert_eq!(barrier_calls.get(), 1, "{replacement}");
            assert_eq!(calls.get(), 0, "{replacement}");
        }

        let fixture = Fixture::new();
        let calls = Cell::new(0);
        assert!(
            run_bundle_with_barrier(
                &fixture,
                &mut no_fault,
                |_| Err(io::Error::other("future input revalidation failed")),
                &calls,
            )
            .is_err()
        );
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn d0_output_transaction_runtime_bundle_barrier_temp_entry_blocks_success() {
        for entry_type in ["directory", "file"] {
            let fixture = Fixture::new();
            let calls = Cell::new(0);
            let barrier_calls = Cell::new(0);
            let result = run_bundle_with_barrier(
                &fixture,
                &mut no_fault,
                |_| {
                    barrier_calls.set(barrier_calls.get() + 1);
                    let temp = fixture.output.join(D0_RUNTIME_TEMP_DIRECTORY);
                    match entry_type {
                        "directory" => {
                            s::create_dir(&temp)?;
                            mode(&temp, 0o700);
                        }
                        "file" => file(&temp, b"intruder", 0o600),
                        _ => unreachable!(),
                    }
                    Ok(())
                },
                &calls,
            );
            assert!(result.is_err(), "{entry_type}");
            assert_eq!(barrier_calls.get(), 1, "{entry_type}");
            assert_eq!(calls.get(), 0, "{entry_type}");
        }
    }

    #[test]
    fn d0_output_transaction_runtime_bundle_rejects_unknown_partial_and_insecure_states() {
        for state_name in [
            "final-symlink",
            "final-file",
            "final-mode",
            "unknown",
            "partial",
            "file-symlink",
            "file-mode",
            "file-mismatch",
            "temp-unknown",
            "temp-symlink",
            "temp-directory-mode",
            "temp-mode",
            "temp-mismatch",
            "both",
        ] {
            let fixture = Fixture::new();
            match state_name {
                "final-symlink" => {
                    symlink(&fixture.input, fixture.output.join(D0_RUNTIME_DIRECTORY)).unwrap()
                }
                "final-file" => file(&fixture.output.join(D0_RUNTIME_DIRECTORY), b"x", 0o600),
                "temp-unknown"
                | "temp-symlink"
                | "temp-directory-mode"
                | "temp-mode"
                | "temp-mismatch" => {
                    let directory = fixture.output.join(D0_RUNTIME_TEMP_DIRECTORY);
                    s::create_dir(&directory).unwrap();
                    mode(
                        &directory,
                        if state_name == "temp-directory-mode" {
                            0o755
                        } else {
                            0o700
                        },
                    );
                    let expected = runtime_expected().remove(0);
                    match state_name {
                        "temp-unknown" => file(&directory.join("unknown"), b"x", 0o600),
                        "temp-symlink" => {
                            symlink(&fixture.input, directory.join(expected.name)).unwrap()
                        }
                        "temp-mode" => {
                            file(&directory.join(expected.name), &expected.bytes[..1], 0o644)
                        }
                        "temp-mismatch" => {
                            file(&directory.join(expected.name), b"not-a-prefix", 0o600)
                        }
                        "temp-directory-mode" => {}
                        _ => unreachable!(),
                    }
                }
                _ => {
                    let directory = prepare_runtime_directory(&fixture, D0_RUNTIME_DIRECTORY);
                    match state_name {
                        "final-mode" => mode(&directory, 0o755),
                        "unknown" => file(&directory.join("unknown"), b"x", 0o600),
                        "partial" => s::remove_file(directory.join(D0_RUNTIME_REPORT)).unwrap(),
                        "file-symlink" => {
                            let name = runtime_expected().remove(0).name;
                            s::remove_file(directory.join(&name)).unwrap();
                            symlink(&fixture.input, directory.join(name)).unwrap();
                        }
                        "file-mode" => {
                            mode(&directory.join(runtime_expected().remove(0).name), 0o644)
                        }
                        "file-mismatch" => file(
                            &directory.join(runtime_expected().remove(0).name),
                            b"wrong",
                            0o600,
                        ),
                        "both" => {
                            prepare_runtime_directory(&fixture, D0_RUNTIME_TEMP_DIRECTORY);
                        }
                        _ => unreachable!(),
                    }
                }
            }
            let calls = Cell::new(0);
            assert!(
                run_bundle(&fixture, &mut no_fault, &calls).is_err(),
                "{state_name}"
            );
            assert_eq!(calls.get(), 0, "{state_name}");
        }

        let fixture = Fixture::new();
        let parent = OutputDirectory::open(&fixture.output).unwrap();
        prepare_runtime_directory(&fixture, D0_RUNTIME_DIRECTORY);
        assert!(
            optional_bundle_directory(
                parent.descriptor.as_fd(),
                OsStr::new(D0_RUNTIME_DIRECTORY),
                parent.metadata.owner.wrapping_add(1),
            )
            .is_err()
        );
    }

    #[test]
    fn d0_output_transaction_runtime_bundle_all_faults_preserve_all_or_nothing() {
        for injected in [
            F::BundleDirectoryCreate,
            F::BundleFileCreate,
            F::BundleWrite,
            F::BundleFileFsync,
            F::BundleTempDirectoryFsync,
            F::BundleRename,
            F::BundleParentFsync,
        ] {
            let fixture = Fixture::new();
            let calls = Cell::new(0);
            let mut fired = false;
            let mut fault = |point| {
                if point == injected && !fired {
                    fired = true;
                    Err(io::Error::other("runtime bundle injected"))
                } else {
                    Ok(())
                }
            };
            assert!(run_bundle(&fixture, &mut fault, &calls).is_err());
            assert!(fired, "{injected:?}");
            assert_eq!(calls.get(), 0, "{injected:?}");
            let final_directory = fixture.output.join(D0_RUNTIME_DIRECTORY);
            if final_directory.exists() {
                assert!(
                    read_bundle(
                        open_directory_at(
                            OutputDirectory::open(&fixture.output)
                                .unwrap()
                                .descriptor
                                .as_fd(),
                            OsStr::new(D0_RUNTIME_DIRECTORY),
                        )
                        .unwrap(),
                        effective_owner().unwrap(),
                        &runtime_expected(),
                    )
                    .is_ok()
                );
            }
            run_bundle(&fixture, &mut no_fault, &calls).unwrap();
            assert_eq!(calls.get(), 1, "{injected:?}");
        }
    }

    #[test]
    fn d0_output_transaction_runtime_bundle_rejects_directory_races() {
        for race in ["temp-replaced", "final-collision", "final-replaced"] {
            let fixture = Fixture::new();
            let calls = Cell::new(0);
            let mut fault = |point| {
                match (race, point) {
                    ("temp-replaced", F::BundleRename) => {
                        let temp = fixture.output.join(D0_RUNTIME_TEMP_DIRECTORY);
                        s::rename(&temp, fixture.output.join("old-temp"))?;
                        prepare_runtime_directory(&fixture, D0_RUNTIME_TEMP_DIRECTORY);
                    }
                    ("final-collision", F::BundleRename) => {
                        prepare_runtime_directory(&fixture, D0_RUNTIME_DIRECTORY);
                    }
                    ("final-replaced", F::BundleParentFsync) => {
                        let final_directory = fixture.output.join(D0_RUNTIME_DIRECTORY);
                        s::rename(&final_directory, fixture.output.join("old-final"))?;
                        prepare_runtime_directory(&fixture, D0_RUNTIME_DIRECTORY);
                    }
                    _ => {}
                }
                Ok(())
            };
            assert!(run_bundle(&fixture, &mut fault, &calls).is_err(), "{race}");
            assert_eq!(calls.get(), 0, "{race}");
        }
    }
}
