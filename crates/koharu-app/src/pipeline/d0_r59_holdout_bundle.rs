//! Test-only Revision 59 plaintext holdout bundle validator.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};

use icu_properties::{
    CodePointMapData,
    props::{GeneralCategory, Script},
};
use image::{DynamicImage, ImageFormat, RgbaImage};
use rustix::fs::{Dir, FileType, Mode, OFlags, fstat, open, openat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::read::ZipReadOptions;

use super::d0_held_input::HeldInput;

const R59_ENTRY_IDS: [&str; 4] = ["r59-h01", "r59-h02", "r59-h03", "r59-h04"];
const R60_ENTRY_IDS: [&str; 4] = ["r60-h01", "r60-h02", "r60-h03", "r60-h04"];
const MANIFEST_NAME: &str = "manifest.json";
const ORACLE_NAME: &str = "oracle.json";
const HASHES_NAME: &str = "hashes.json";
const PYTHON_SHA256: &str = "3e7d30871a9740446f33a907b14d28f10ebe6d4e1c146a4c0788308f573a6609";
const SIPS_SHA256: &str = "e893abb712ee4799b10f4756943d9310229ddbaebea752ca9cd39a58240edcdf";

struct BundleContract {
    plan_revision: u32,
    entry_ids: [&'static str; 4],
    manifest_contract: &'static str,
    oracle_contract: &'static str,
    hashes_contract: &'static str,
    mask_identity_domain: &'static [u8],
    grayscale_masks_only: bool,
    protected_texts_required: bool,
}

const R59_BUNDLE_CONTRACT: BundleContract = BundleContract {
    plan_revision: 59,
    entry_ids: R59_ENTRY_IDS,
    manifest_contract: "hanonly-r59-holdout-manifest-v1",
    oracle_contract: "hanonly-r59-holdout-oracle-v1",
    hashes_contract: "hanonly-r59-holdout-hashes-v1",
    mask_identity_domain: b"hanonly-r59-binary-mask-v1\0",
    grayscale_masks_only: false,
    protected_texts_required: false,
};

const R60_BUNDLE_CONTRACT: BundleContract = BundleContract {
    plan_revision: 60,
    entry_ids: R60_ENTRY_IDS,
    manifest_contract: "hanonly-r60-holdout-manifest-v1",
    oracle_contract: "hanonly-r60-holdout-oracle-v1",
    hashes_contract: "hanonly-r60-holdout-hashes-v1",
    mask_identity_domain: b"hanonly-r60-binary-mask-v1\0",
    grayscale_masks_only: true,
    protected_texts_required: true,
};

#[derive(Clone, Copy)]
pub(super) struct R59FreezeCommitments<'a> {
    pub(super) plaintext_archive_sha256: &'a str,
    pub(super) manifest_sha256: &'a str,
    pub(super) oracle_sha256: &'a str,
    pub(super) hashes_sha256: &'a str,
}

pub(super) struct R59ValidatedReceiptData {
    pub(super) plaintext_archive_sha256: String,
    pub(super) manifest_sha256: String,
    pub(super) oracle_sha256: String,
    pub(super) hashes_sha256: String,
    pub(super) schema_validation_pass: bool,
    pub(super) asset_binding_pass: bool,
    pub(super) mask_source_clean_equality_pass: bool,
    pub(super) oracle_semantics_pass: bool,
}

pub(super) struct R59ValidatedBundle {
    pub(super) receipt: R59ValidatedReceiptData,
    pub(super) execution: R59ValidatedExecutionView,
}

pub(super) struct R59ValidatedExecutionView {
    pub(super) entries: Vec<R59ValidatedExecutionEntry>,
}

pub(super) struct R59ValidatedExecutionEntry {
    pub(super) id: String,
    pub(super) source_encoded_bytes: Box<[u8]>,
    pub(super) clean_reference_encoded_bytes: Box<[u8]>,
    pub(super) validated_source_rgba: RgbaImage,
    pub(super) validated_clean_reference_rgba: RgbaImage,
    pub(super) source_width: u32,
    pub(super) source_height: u32,
    pub(super) clean_width: u32,
    pub(super) clean_height: u32,
    pub(super) protected_rois: Vec<[u32; 4]>,
    pub(super) targets: Vec<R59ValidatedExecutionTarget>,
}

impl R59ValidatedExecutionEntry {
    pub(super) fn source_dynamic_image(&self) -> DynamicImage {
        DynamicImage::ImageRgba8(self.validated_source_rgba.clone())
    }
}

pub(super) struct R59ValidatedExecutionTarget {
    pub(super) id: String,
    pub(super) source_roi: [u32; 4],
    pub(super) clean_reference_edit_roi: [u32; 4],
    pub(super) erase_source_ink_mask_encoded_bytes: Box<[u8]>,
    pub(super) residual_source_ink_mask_encoded_bytes: Box<[u8]>,
    pub(super) validated_binary_mask: Box<[u8]>,
    pub(super) expected: String,
    pub(super) writing: String,
    pub(super) effect: String,
    pub(super) position: String,
    pub(super) translation_length: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    contract: String,
    entries: Vec<ManifestEntry>,
    plan_revision: u32,
    role: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestEntry {
    aspect: String,
    background: String,
    clean_reference_relpath: String,
    dimension_bin: String,
    id: String,
    multi_node: bool,
    protected_rois: Vec<Rect>,
    role: String,
    source_relpath: String,
    targets: Vec<ManifestTarget>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestTarget {
    clean_reference_edit_roi: Rect,
    effect: String,
    erase_source_ink_mask_relpath: String,
    expected: String,
    id: String,
    position: String,
    residual_source_ink_mask_relpath: String,
    source_roi: Rect,
    translation_length: String,
    writing: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct Rect([u32; 4]);

impl Rect {
    fn valid_in(self, width: u32, height: u32) -> bool {
        let [left, top, right, bottom] = self.0;
        left < right && top < bottom && right <= width && bottom <= height
    }

    fn contains_rect(self, other: Self) -> bool {
        let [left, top, right, bottom] = self.0;
        let [other_left, other_top, other_right, other_bottom] = other.0;
        left <= other_left && top <= other_top && other_right <= right && other_bottom <= bottom
    }

    fn contains_point(self, x: u32, y: u32) -> bool {
        let [left, top, right, bottom] = self.0;
        left <= x && x < right && top <= y && y < bottom
    }

    fn disjoint(self, other: Self) -> bool {
        let [left, top, right, bottom] = self.0;
        let [other_left, other_top, other_right, other_bottom] = other.0;
        right <= other_left || other_right <= left || bottom <= other_top || other_bottom <= top
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Oracle {
    contract: String,
    entries: Vec<OracleEntry>,
    manifest_sha256: String,
    plan_revision: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OracleEntry {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protected_texts: Option<Vec<OracleProtectedText>>,
    targets: Vec<OracleTarget>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OracleProtectedText {
    roi: Rect,
    source_script_class: String,
    source_text_sha256: String,
    source_text_utf8: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OracleTarget {
    expected_decision: String,
    expected_rejection_reason: Option<String>,
    id: String,
    source_han_scalar_count: u32,
    source_script_class: String,
    source_text_sha256: String,
    source_text_utf8: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Hashes {
    assets: BTreeMap<String, AssetHash>,
    contract: String,
    manifest_sha256: String,
    oracle_sha256: String,
    plan_revision: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AssetHash {
    byte_length: u64,
    decoded_kind: String,
    height: u32,
    normalized_identity_sha256: String,
    raw_sha256: String,
    width: u32,
}

struct HeldRoot {
    descriptor: OwnedFd,
    owner: u64,
}

struct HeldAsset {
    bytes: Box<[u8]>,
    metadata: Metadata,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Metadata {
    dev: i128,
    ino: u64,
    owner: u64,
    mode: u64,
    file_type: FileType,
}

struct DecodedAsset {
    rgba: RgbaImage,
    normalized_identity_sha256: String,
}

struct DecodedMask {
    bits: Vec<u8>,
    normalized_identity_sha256: String,
    width: u32,
    height: u32,
}

pub(super) fn validate_r59_plaintext_holdout_bundle(
    plaintext_root: &Path,
    canonical_plaintext_archive_path: &Path,
    canonical_plaintext_archive_bytes: &[u8],
    freeze: R59FreezeCommitments<'_>,
) -> io::Result<R59ValidatedBundle> {
    validate_plaintext_holdout_bundle(
        plaintext_root,
        canonical_plaintext_archive_path,
        canonical_plaintext_archive_bytes,
        freeze,
        &R59_BUNDLE_CONTRACT,
    )
}

pub(super) fn validate_r60_plaintext_holdout_bundle(
    plaintext_root: &Path,
    canonical_plaintext_archive_path: &Path,
    canonical_plaintext_archive_bytes: &[u8],
    freeze: R59FreezeCommitments<'_>,
) -> io::Result<R59ValidatedBundle> {
    validate_plaintext_holdout_bundle(
        plaintext_root,
        canonical_plaintext_archive_path,
        canonical_plaintext_archive_bytes,
        freeze,
        &R60_BUNDLE_CONTRACT,
    )
}

fn validate_plaintext_holdout_bundle(
    plaintext_root: &Path,
    canonical_plaintext_archive_path: &Path,
    canonical_plaintext_archive_bytes: &[u8],
    freeze: R59FreezeCommitments<'_>,
    contract: &BundleContract,
) -> io::Result<R59ValidatedBundle> {
    validate_pinned_tool("/usr/bin/python3", PYTHON_SHA256, b"Python 3.9.6\n")?;
    validate_pinned_tool("/usr/bin/sips", SIPS_SHA256, b"sips-316\n")?;
    for commitment in [
        freeze.plaintext_archive_sha256,
        freeze.manifest_sha256,
        freeze.oracle_sha256,
        freeze.hashes_sha256,
    ] {
        require(is_sha256(commitment), "invalid R59 freeze commitment")?;
    }

    let root = HeldRoot::open(plaintext_root)?;
    let archive = HeldInput::open(canonical_plaintext_archive_path)?;
    archive.require_file_and_parent_security(root.owner, 0o600, 0o700)?;
    require(
        archive.bytes() == canonical_plaintext_archive_bytes,
        "R59 archive path/bytes drift",
    )?;
    let archive_sha256 = sha256_hex(canonical_plaintext_archive_bytes);
    require(
        archive_sha256 == freeze.plaintext_archive_sha256,
        "R59 archive commitment drift",
    )?;

    let manifest_asset = root.read_file(MANIFEST_NAME)?;
    let oracle_asset = root.read_file(ORACLE_NAME)?;
    let hashes_asset = root.read_file(HASHES_NAME)?;
    let manifest_sha256 = sha256_hex(&manifest_asset.bytes);
    let oracle_sha256 = sha256_hex(&oracle_asset.bytes);
    let hashes_sha256 = sha256_hex(&hashes_asset.bytes);
    require(
        manifest_sha256 == freeze.manifest_sha256
            && oracle_sha256 == freeze.oracle_sha256
            && hashes_sha256 == freeze.hashes_sha256,
        "R59 plaintext commitment drift",
    )?;

    let manifest: Manifest = canonical_json(&manifest_asset.bytes)?;
    let oracle: Oracle = canonical_json(&oracle_asset.bytes)?;
    let hashes: Hashes = canonical_json(&hashes_asset.bytes)?;
    require(
        manifest.contract == contract.manifest_contract
            && manifest.plan_revision == contract.plan_revision
            && manifest.role == "holdout"
            && oracle.contract == contract.oracle_contract
            && oracle.plan_revision == contract.plan_revision
            && oracle.manifest_sha256 == manifest_sha256
            && hashes.contract == contract.hashes_contract
            && hashes.plan_revision == contract.plan_revision
            && hashes.manifest_sha256 == manifest_sha256
            && hashes.oracle_sha256 == oracle_sha256,
        "R59 manifest/oracle/hashes mutual binding drift",
    )?;
    require(
        manifest.entries.len() == contract.entry_ids.len()
            && oracle.entries.len() == contract.entry_ids.len(),
        "R59 entry cardinality drift",
    )?;

    let mut expected_paths = BTreeSet::new();
    let mut execution_entries = Vec::with_capacity(contract.entry_ids.len());
    let mut seen_inodes = HashSet::from([
        (manifest_asset.metadata.dev, manifest_asset.metadata.ino),
        (oracle_asset.metadata.dev, oracle_asset.metadata.ino),
        (hashes_asset.metadata.dev, hashes_asset.metadata.ino),
    ]);

    for (index, expected_id) in contract.entry_ids.iter().enumerate() {
        let entry = &manifest.entries[index];
        let oracle_entry = &oracle.entries[index];
        validate_entry_schema(entry, expected_id)?;
        validate_oracle_alignment(entry, oracle_entry, expected_id)?;
        validate_protected_texts(entry, oracle_entry, contract)?;

        let source_path = validate_source_path(&entry.source_relpath, expected_id)?;
        let clean_path = format!("assets/clean/{expected_id}.png");
        require(
            entry.clean_reference_relpath == clean_path,
            "R59 clean path mapping drift",
        )?;
        let source = read_bound_asset(
            &root,
            &hashes,
            source_path,
            "sips-normalized-scanlines-v1",
            &mut expected_paths,
            &mut seen_inodes,
        )?;
        let clean = read_bound_asset(
            &root,
            &hashes,
            &clean_path,
            "sips-normalized-scanlines-v1",
            &mut expected_paths,
            &mut seen_inodes,
        )?;
        let source_decoded = decode_image(&source.bytes)?;
        let clean_decoded = decode_image(&clean.bytes)?;
        require(
            source_decoded.rgba.dimensions() == clean_decoded.rgba.dimensions(),
            "R59 Source/Clean dimensions drift",
        )?;
        validate_decoded_record(&hashes.assets[source_path], &source_decoded)?;
        validate_decoded_record(&hashes.assets[&clean_path], &clean_decoded)?;

        let (width, height) = source_decoded.rgba.dimensions();
        validate_entry_geometry(entry, width, height)?;
        validate_oracle_entry(oracle_entry)?;

        let mut target_masks = Vec::with_capacity(entry.targets.len());
        for target in &entry.targets {
            let erase_path = format!("assets/masks/{expected_id}/{}-erase.png", target.id);
            let residual_path = format!("assets/masks/{expected_id}/{}-residual.png", target.id);
            require(
                target.erase_source_ink_mask_relpath == erase_path
                    && target.residual_source_ink_mask_relpath == residual_path,
                "R59 mask path mapping drift",
            )?;
            let erase = read_bound_asset(
                &root,
                &hashes,
                &erase_path,
                "binary-mask-v1",
                &mut expected_paths,
                &mut seen_inodes,
            )?;
            let residual = read_bound_asset(
                &root,
                &hashes,
                &residual_path,
                "binary-mask-v1",
                &mut expected_paths,
                &mut seen_inodes,
            )?;
            let erase_mask = decode_mask(&erase.bytes, contract)?;
            let residual_mask = decode_mask(&residual.bytes, contract)?;
            require(
                erase_mask.width == width
                    && erase_mask.height == height
                    && residual_mask.width == width
                    && residual_mask.height == height,
                "R59 mask dimensions drift",
            )?;
            validate_mask_record(&hashes.assets[&erase_path], &erase_mask)?;
            validate_mask_record(&hashes.assets[&residual_path], &residual_mask)?;
            require(
                erase_mask.bits == residual_mask.bits
                    && erase_mask.bits.contains(&1)
                    && erase_mask.normalized_identity_sha256
                        == residual_mask.normalized_identity_sha256,
                "R59 erase/residual mask drift",
            )?;
            target_masks.push((erase_mask.bits, erase.bytes, residual.bytes));
        }
        validate_pixels(
            entry,
            &source_decoded.rgba,
            &clean_decoded.rgba,
            &target_masks,
        )?;
        execution_entries.push(R59ValidatedExecutionEntry {
            id: entry.id.clone(),
            source_encoded_bytes: source.bytes,
            clean_reference_encoded_bytes: clean.bytes,
            validated_source_rgba: source_decoded.rgba.clone(),
            validated_clean_reference_rgba: clean_decoded.rgba.clone(),
            source_width: width,
            source_height: height,
            clean_width: clean_decoded.rgba.width(),
            clean_height: clean_decoded.rgba.height(),
            protected_rois: entry.protected_rois.iter().map(|rect| rect.0).collect(),
            targets: entry
                .targets
                .iter()
                .zip(target_masks)
                .map(
                    |(target, (mask, erase_bytes, residual_bytes))| R59ValidatedExecutionTarget {
                        id: target.id.clone(),
                        source_roi: target.source_roi.0,
                        clean_reference_edit_roi: target.clean_reference_edit_roi.0,
                        erase_source_ink_mask_encoded_bytes: erase_bytes,
                        residual_source_ink_mask_encoded_bytes: residual_bytes,
                        validated_binary_mask: mask.into_boxed_slice(),
                        expected: target.expected.clone(),
                        writing: target.writing.clone(),
                        effect: target.effect.clone(),
                        position: target.position.clone(),
                        translation_length: target.translation_length.clone(),
                    },
                )
                .collect(),
        });
    }

    require(
        expected_paths.iter().eq(hashes.assets.keys()),
        "R59 hashes asset set drift",
    )?;
    let mut all_paths = expected_paths;
    all_paths.extend([
        MANIFEST_NAME.to_owned(),
        ORACLE_NAME.to_owned(),
        HASHES_NAME.to_owned(),
    ]);
    let actual_paths = root.snapshot_paths()?;
    let expected_directories = expected_directories(&all_paths);
    require(
        actual_paths.files == all_paths && actual_paths.directories == expected_directories,
        "R59 plaintext root contains unknown or missing entries",
    )?;
    validate_archive(
        canonical_plaintext_archive_bytes,
        &root,
        &all_paths,
        &expected_directories,
        contract.plan_revision == 60,
    )?;

    Ok(R59ValidatedBundle {
        receipt: R59ValidatedReceiptData {
            plaintext_archive_sha256: archive_sha256,
            manifest_sha256,
            oracle_sha256,
            hashes_sha256,
            schema_validation_pass: true,
            asset_binding_pass: true,
            mask_source_clean_equality_pass: true,
            oracle_semantics_pass: true,
        },
        execution: R59ValidatedExecutionView {
            entries: execution_entries,
        },
    })
}

fn validate_entry_schema(entry: &ManifestEntry, expected_id: &str) -> io::Result<()> {
    require(
        entry.id == expected_id
            && entry.role == "holdout"
            && matches!(
                entry.background.as_str(),
                "pure" | "gradient" | "texture" | "product" | "person"
            )
            && !entry.targets.is_empty()
            && entry.multi_node == (entry.targets.len() > 1)
            && entry.targets.windows(2).all(|pair| pair[0].id < pair[1].id),
        "R59 manifest entry drift",
    )?;
    let mut ids = HashSet::new();
    for target in &entry.targets {
        require(
            valid_target_id(&target.id)
                && ids.insert(target.id.as_str())
                && target.expected == "automatic_strict"
                && matches!(target.writing.as_str(), "horizontal" | "vertical")
                && matches!(target.effect.as_str(), "plain" | "stroke")
                && matches!(target.position.as_str(), "interior" | "page_edge")
                && matches!(
                    target.translation_length.as_str(),
                    "short" | "equal" | "2x" | "3x"
                ),
            "R59 manifest target drift",
        )?;
    }
    Ok(())
}

fn validate_entry_geometry(entry: &ManifestEntry, width: u32, height: u32) -> io::Result<()> {
    require(
        entry.aspect == recompute_aspect(width, height)
            && entry.dimension_bin == recompute_dimension_bin(width, height)
            && entry
                .protected_rois
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            && entry
                .protected_rois
                .iter()
                .all(|rect| rect.valid_in(width, height)),
        "R59 entry geometry classification drift",
    )?;
    for (index, target) in entry.targets.iter().enumerate() {
        require(
            target.source_roi.valid_in(width, height)
                && target.clean_reference_edit_roi.valid_in(width, height)
                && target
                    .clean_reference_edit_roi
                    .contains_rect(target.source_roi)
                && entry
                    .protected_rois
                    .iter()
                    .all(|protected| protected.disjoint(target.clean_reference_edit_roi))
                && entry.targets[index + 1..].iter().all(|other| {
                    target
                        .clean_reference_edit_roi
                        .disjoint(other.clean_reference_edit_roi)
                }),
            "R59 ROI geometry drift",
        )?;
    }
    require(
        entry
            .protected_rois
            .iter()
            .enumerate()
            .all(|(index, rect)| {
                entry.protected_rois[index + 1..]
                    .iter()
                    .all(|other| rect.disjoint(*other))
            }),
        "R59 protected ROI overlap",
    )
}

fn validate_oracle_entry(entry: &OracleEntry) -> io::Result<()> {
    let categories = CodePointMapData::<GeneralCategory>::new();
    let scripts = CodePointMapData::<Script>::new();
    for target in &entry.targets {
        let text = &target.source_text_utf8;
        let han_count = text
            .chars()
            .filter(|character| scripts.get(*character) == Script::Han)
            .count();
        require(
            !text.is_empty()
                && text.chars().all(|character| {
                    !matches!(
                        categories.get(character),
                        GeneralCategory::Control
                            | GeneralCategory::PrivateUse
                            | GeneralCategory::Unassigned
                    )
                })
                && is_nfc(text)?
                && han_count > 0
                && target.source_han_scalar_count == han_count as u32
                && target.source_script_class == "han_or_mixed"
                && target.source_text_sha256 == sha256_hex(text.as_bytes())
                && target.expected_decision == "select"
                && target.expected_rejection_reason.is_none(),
            "R59 oracle semantic drift",
        )?;
    }
    Ok(())
}

fn validate_oracle_alignment(
    manifest: &ManifestEntry,
    oracle: &OracleEntry,
    expected_id: &str,
) -> io::Result<()> {
    require(
        oracle.id == expected_id
            && oracle.targets.len() == manifest.targets.len()
            && oracle
                .targets
                .iter()
                .zip(&manifest.targets)
                .all(|(left, right)| left.id == right.id),
        "R59 oracle order or identity drift",
    )
}

fn validate_protected_texts(
    manifest: &ManifestEntry,
    oracle: &OracleEntry,
    contract: &BundleContract,
) -> io::Result<()> {
    if !contract.protected_texts_required {
        return require(
            oracle.protected_texts.is_none(),
            "R59 protected-text schema drift",
        );
    }
    let protected = oracle
        .protected_texts
        .as_ref()
        .ok_or_else(|| invalid_data("R60 protected-text ground truth is missing"))?;
    require(
        protected.len() == manifest.protected_rois.len()
            && protected
                .iter()
                .zip(&manifest.protected_rois)
                .all(|(text, roi)| text.roi == *roi),
        "R60 protected-text ROI binding drift",
    )?;
    let scripts = CodePointMapData::<Script>::new();
    for text in protected {
        let source = text.source_text_utf8.as_str();
        require(
            !source.is_empty()
                && is_nfc(source)?
                && source
                    .chars()
                    .any(|character| scripts.get(character) == Script::Latin)
                && source.chars().all(|character| {
                    scripts.get(character) == Script::Latin
                        || character.is_ascii() && (' '..='~').contains(&character)
                })
                && text.source_script_class == "latin_or_ascii"
                && text.source_text_sha256 == sha256_hex(source.as_bytes()),
            "R60 protected-text semantic drift",
        )?;
    }
    Ok(())
}

fn is_nfc(text: &str) -> io::Result<bool> {
    let mut child = Command::new("/usr/bin/python3")
        .args([
            "-I",
            "-S",
            "-c",
            "import sys,unicodedata;s=sys.stdin.buffer.read().decode('utf-8');sys.stdout.buffer.write(unicodedata.normalize('NFC',s).encode('utf-8'))",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| invalid_data("R59 NFC validator stdin is unavailable"))?
        .write_all(text.as_bytes())?;
    let output = child.wait_with_output()?;
    require(
        output.status.success() && output.stderr.is_empty(),
        "R59 NFC validator failed",
    )?;
    Ok(output.stdout == text.as_bytes())
}

fn validate_pixels(
    entry: &ManifestEntry,
    source: &RgbaImage,
    clean: &RgbaImage,
    masks: &[(Vec<u8>, Box<[u8]>, Box<[u8]>)],
) -> io::Result<()> {
    let (width, height) = source.dimensions();
    let expected_len = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| invalid_data("R59 image dimensions overflow"))?;
    require(
        masks.iter().all(|mask| mask.0.len() == expected_len),
        "R59 mask byte length drift",
    )?;
    for y in 0..height {
        for x in 0..width {
            let offset = usize::try_from(u64::from(y) * u64::from(width) + u64::from(x))
                .map_err(|_| invalid_data("R59 pixel offset overflow"))?;
            let owners = masks
                .iter()
                .enumerate()
                .filter(|(_, mask)| mask.0[offset] == 1)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            require(owners.len() <= 1, "R59 target masks overlap")?;
            let protected = entry
                .protected_rois
                .iter()
                .any(|rect| rect.contains_point(x, y));
            let changed = source.get_pixel(x, y) != clean.get_pixel(x, y);
            match owners.first().copied() {
                Some(index) => require(
                    entry.targets[index]
                        .clean_reference_edit_roi
                        .contains_point(x, y)
                        && !protected
                        && changed,
                    "R59 mask foreground is outside exact Source/Clean delta",
                )?,
                None => require(
                    !changed,
                    "R59 Source/Clean delta exists outside target masks",
                )?,
            }
        }
    }
    Ok(())
}

fn read_bound_asset(
    root: &HeldRoot,
    hashes: &Hashes,
    relative: &str,
    decoded_kind: &str,
    expected_paths: &mut BTreeSet<String>,
    seen_inodes: &mut HashSet<(i128, u64)>,
) -> io::Result<HeldAsset> {
    validate_relative_path(relative)?;
    require(
        expected_paths.insert(relative.to_owned()),
        "R59 asset referenced more than once",
    )?;
    let record = hashes
        .assets
        .get(relative)
        .ok_or_else(|| invalid_data("R59 referenced asset is missing from hashes"))?;
    require(
        record.decoded_kind == decoded_kind
            && record.byte_length > 0
            && record.width > 0
            && record.height > 0
            && is_sha256(&record.raw_sha256)
            && is_sha256(&record.normalized_identity_sha256),
        "R59 asset hash record drift",
    )?;
    let held = root.read_file(relative)?;
    require(
        seen_inodes.insert((held.metadata.dev, held.metadata.ino)),
        "R59 asset inode is referenced more than once",
    )?;
    require(
        held.bytes.len() as u64 == record.byte_length
            && sha256_hex(&held.bytes) == record.raw_sha256,
        "R59 raw asset commitment drift",
    )?;
    Ok(held)
}

fn decode_image(bytes: &[u8]) -> io::Result<DecodedAsset> {
    let format = image::guess_format(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    require(
        matches!(
            format,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
        ),
        "R59 unsupported source image format",
    )?;
    let normalized_png = sips_png(bytes)?;
    let (width, height, color_type, scanlines) = normalized_png_scanlines(&normalized_png)?;
    let rgba = image::load_from_memory_with_format(&normalized_png, ImageFormat::Png)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .into_rgba8();
    require(
        rgba.dimensions() == (width, height),
        "R59 normalized image dimensions drift",
    )?;
    let mut identity = Sha256::new();
    identity.update(b"sips-normalized-scanlines-v1\0");
    identity.update(width.to_be_bytes());
    identity.update(height.to_be_bytes());
    identity.update([color_type]);
    identity.update(scanlines);
    Ok(DecodedAsset {
        normalized_identity_sha256: hex_digest(identity.finalize()),
        rgba,
    })
}

fn sips_png(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let temp = TempDir::new()?;
    let input = temp.path().join("input");
    let output = temp.path().join("output.png");
    std::fs::write(&input, bytes)?;
    let result = Command::new("/usr/bin/sips")
        .args(["-s", "format", "png"])
        .arg(&input)
        .arg("--out")
        .arg(&output)
        .output()?;
    require(
        result.status.success(),
        "R59 pinned sips normalization failed",
    )?;
    std::fs::read(output)
}

fn validate_pinned_tool(
    path: &str,
    expected_sha256: &str,
    expected_version: &[u8],
) -> io::Result<()> {
    let held = HeldInput::open(Path::new(path))?;
    held.require_file_and_parent_security(0, 0o755, 0o755)?;
    require(
        hex_digest(held.sha256()) == expected_sha256,
        "R59 pinned tool SHA drift",
    )?;
    let output = Command::new(path).arg("--version").output()?;
    require(
        output.status.success() && output.stdout == expected_version && output.stderr.is_empty(),
        "R59 pinned tool version drift",
    )
}

fn normalized_png_scanlines(bytes: &[u8]) -> io::Result<(u32, u32, u8, Vec<u8>)> {
    require(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "R59 normalized image is not PNG",
    )?;
    let mut offset = 8;
    let mut header = None;
    let mut idat = Vec::new();
    let mut saw_idat = false;
    let mut ended_idat = false;
    let mut saw_iend = false;
    while offset < bytes.len() {
        let length_end = offset
            .checked_add(4)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid_data("R59 normalized PNG is truncated"))?;
        let length = usize::try_from(u32::from_be_bytes(
            bytes[offset..length_end].try_into().unwrap(),
        ))
        .map_err(|_| invalid_data("R59 normalized PNG chunk length overflow"))?;
        let chunk_end = length_end
            .checked_add(4 + length + 4)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid_data("R59 normalized PNG chunk is truncated"))?;
        let kind = &bytes[length_end..length_end + 4];
        let data = &bytes[length_end + 4..chunk_end - 4];
        let expected_crc = u32::from_be_bytes(bytes[chunk_end - 4..chunk_end].try_into().unwrap());
        require(
            png_crc(kind, data) == expected_crc,
            "R59 normalized PNG CRC drift",
        )?;
        match kind {
            b"IHDR" => {
                require(
                    header.is_none() && offset == 8 && data.len() == 13,
                    "R59 normalized PNG IHDR drift",
                )?;
                let width = u32::from_be_bytes(data[..4].try_into().unwrap());
                let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
                require(
                    width > 0
                        && height > 0
                        && data[8] == 8
                        && matches!(data[9], 0 | 2 | 4 | 6)
                        && data[10] == 0
                        && data[11] == 0
                        && data[12] == 0,
                    "R59 normalized PNG IHDR values drift",
                )?;
                header = Some((width, height, data[9]));
            }
            b"IDAT" => {
                require(
                    header.is_some() && !ended_idat && !saw_iend,
                    "R59 normalized PNG IDAT order drift",
                )?;
                saw_idat = true;
                idat.extend_from_slice(data);
            }
            b"IEND" => {
                require(
                    saw_idat && !saw_iend && data.is_empty() && chunk_end == bytes.len(),
                    "R59 normalized PNG IEND drift",
                )?;
                saw_iend = true;
            }
            _ => {
                if saw_idat {
                    ended_idat = true;
                }
                require(
                    kind[0] & 0x20 != 0,
                    "R59 normalized PNG has unknown critical chunk",
                )?;
            }
        }
        offset = chunk_end;
    }
    let (width, height, color_type) =
        header.ok_or_else(|| invalid_data("R59 normalized PNG has no IHDR"))?;
    require(
        saw_iend && !idat.is_empty(),
        "R59 normalized PNG chunk set drift",
    )?;
    let channels = match color_type {
        0 => 1_u64,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => unreachable!(),
    };
    let scanline_length = u64::from(height)
        .checked_mul(
            u64::from(width)
                .checked_mul(channels)
                .and_then(|row| row.checked_add(1))
                .ok_or_else(|| invalid_data("R59 normalized scanline length overflow"))?,
        )
        .and_then(|length| usize::try_from(length).ok())
        .ok_or_else(|| invalid_data("R59 normalized scanline length overflow"))?;
    let scanlines = inflate_zlib(&idat, scanline_length)?;
    Ok((width, height, color_type, scanlines))
}

fn inflate_zlib(bytes: &[u8], expected_length: usize) -> io::Result<Vec<u8>> {
    require(
        bytes.len() >= 6
            && bytes[0] & 0x0f == 8
            && bytes[0] >> 4 <= 7
            && (u16::from(bytes[0]) * 256 + u16::from(bytes[1])).is_multiple_of(31)
            && bytes[1] & 0x20 == 0,
        "R59 normalized PNG zlib header drift",
    )?;
    let compressed = &bytes[2..bytes.len() - 4];
    let mut zip = Vec::new();
    zip.extend_from_slice(&0x04034b50_u32.to_le_bytes());
    zip.extend_from_slice(&20_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&8_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u32.to_le_bytes());
    zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(expected_length as u32).to_le_bytes());
    zip.extend_from_slice(&1_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.push(b'x');
    zip.extend_from_slice(compressed);
    let central_offset = zip.len() as u32;
    zip.extend_from_slice(&0x02014b50_u32.to_le_bytes());
    zip.extend_from_slice(&20_u16.to_le_bytes());
    zip.extend_from_slice(&20_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&8_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u32.to_le_bytes());
    zip.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    zip.extend_from_slice(&(expected_length as u32).to_le_bytes());
    zip.extend_from_slice(&1_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u32.to_le_bytes());
    zip.extend_from_slice(&0_u32.to_le_bytes());
    zip.push(b'x');
    let central_size = zip.len() as u32 - central_offset;
    zip.extend_from_slice(&0x06054b50_u32.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());
    zip.extend_from_slice(&1_u16.to_le_bytes());
    zip.extend_from_slice(&1_u16.to_le_bytes());
    zip.extend_from_slice(&central_size.to_le_bytes());
    zip.extend_from_slice(&central_offset.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes());

    let mut archive = zip::ZipArchive::new(Cursor::new(zip))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut file = archive
        .by_index_with_options(0, ZipReadOptions::new().ignore_crc32(true))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut output = Vec::new();
    file.read_to_end(&mut output)?;
    require(
        output.len() == expected_length
            && adler32(&output) == u32::from_be_bytes(bytes[bytes.len() - 4..].try_into().unwrap()),
        "R59 normalized PNG zlib payload drift",
    )?;
    Ok(output)
}

fn png_crc(kind: &[u8], data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320_u32 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (a, b) = bytes.iter().fold((1_u32, 0_u32), |(a, b), byte| {
        let a = (a + u32::from(*byte)) % 65_521;
        (a, (b + a) % 65_521)
    });
    b << 16 | a
}

fn decode_mask(bytes: &[u8], contract: &BundleContract) -> io::Result<DecodedMask> {
    let (width, height, color_type) = png_ihdr(bytes)?;
    require(
        color_type == 0 || !contract.grayscale_masks_only && color_type == 6,
        "R59 mask must be grayscale or RGBA PNG",
    )?;
    let dynamic = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let bits = match (color_type, dynamic) {
        (0, DynamicImage::ImageLuma8(image)) => image
            .into_raw()
            .into_iter()
            .map(binary_channel)
            .collect::<io::Result<Vec<_>>>()?,
        (6, DynamicImage::ImageRgba8(image)) => image
            .pixels()
            .map(|pixel| {
                let [red, green, blue, alpha] = pixel.0;
                require(
                    alpha == 255 && red == green && green == blue,
                    "R59 mask pixel is not opaque grayscale",
                )?;
                binary_channel(red)
            })
            .collect::<io::Result<Vec<_>>>()?,
        _ => return Err(invalid_data("R59 mask PNG decode kind drift")),
    };
    let mut identity = Sha256::new();
    identity.update(contract.mask_identity_domain);
    identity.update(width.to_be_bytes());
    identity.update(height.to_be_bytes());
    identity.update(&bits);
    Ok(DecodedMask {
        bits,
        normalized_identity_sha256: hex_digest(identity.finalize()),
        width,
        height,
    })
}

fn png_ihdr(bytes: &[u8]) -> io::Result<(u32, u32, u8)> {
    require(
        bytes.len() >= 33
            && &bytes[..8] == b"\x89PNG\r\n\x1a\n"
            && &bytes[8..12] == 13_u32.to_be_bytes().as_slice()
            && &bytes[12..16] == b"IHDR",
        "R59 mask is not a canonical PNG",
    )?;
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    require(
        width > 0
            && height > 0
            && bytes[24] == 8
            && matches!(bytes[25], 0 | 6)
            && bytes[26] == 0
            && bytes[27] == 0
            && bytes[28] == 0,
        "R59 mask PNG IHDR drift",
    )?;
    Ok((width, height, bytes[25]))
}

fn binary_channel(value: u8) -> io::Result<u8> {
    match value {
        0 => Ok(0),
        255 => Ok(1),
        _ => Err(invalid_data("R59 mask is not binary")),
    }
}

fn validate_decoded_record(record: &AssetHash, decoded: &DecodedAsset) -> io::Result<()> {
    let (width, height) = decoded.rgba.dimensions();
    require(
        record.width == width
            && record.height == height
            && record.normalized_identity_sha256 == decoded.normalized_identity_sha256,
        "R59 decoded image identity drift",
    )
}

fn validate_mask_record(record: &AssetHash, mask: &DecodedMask) -> io::Result<()> {
    require(
        record.width == mask.width
            && record.height == mask.height
            && record.normalized_identity_sha256 == mask.normalized_identity_sha256,
        "R59 decoded mask identity drift",
    )
}

impl HeldRoot {
    fn open(path: &Path) -> io::Result<Self> {
        let components = absolute_components(path)?;
        let slash = fs(open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        let mut current = slash;
        for component in components {
            current = fs(openat(
                current.as_fd(),
                component,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ))?;
        }
        let metadata = metadata(&fs(fstat(&current))?);
        require(
            metadata.file_type.is_dir()
                && metadata.owner == effective_owner()?
                && metadata.mode & 0o7777 == 0o700,
            "R59 plaintext root security metadata is invalid",
        )?;
        Ok(Self {
            descriptor: current,
            owner: metadata.owner,
        })
    }

    fn read_file(&self, relative: &str) -> io::Result<HeldAsset> {
        let components = relative_components(relative)?;
        let (name, directories) = components
            .split_last()
            .ok_or_else(|| invalid_data("R59 relative path is empty"))?;
        let mut current: Option<OwnedFd> = None;
        for directory in directories {
            let parent = current
                .as_ref()
                .map_or_else(|| self.descriptor.as_fd(), AsFd::as_fd);
            let next = fs(openat(
                parent,
                directory,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ))?;
            let metadata = metadata(&fs(fstat(&next))?);
            require(
                metadata.file_type.is_dir()
                    && metadata.owner == self.owner
                    && metadata.mode & 0o7777 == 0o700,
                "R59 plaintext directory security metadata is invalid",
            )?;
            current = Some(next);
        }
        let parent = current
            .as_ref()
            .map_or_else(|| self.descriptor.as_fd(), AsFd::as_fd);
        let descriptor = fs(openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        let current_metadata = metadata(&fs(fstat(&descriptor))?);
        require(
            current_metadata.file_type.is_file()
                && current_metadata.owner == self.owner
                && current_metadata.mode & 0o7777 == 0o600,
            "R59 plaintext file security metadata is invalid",
        )?;
        let mut file = File::from(descriptor);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        require(
            metadata(&fs(fstat(&file))?) == current_metadata,
            "R59 plaintext file metadata changed while reading",
        )?;
        Ok(HeldAsset {
            bytes: bytes.into_boxed_slice(),
            metadata: current_metadata,
        })
    }

    fn snapshot_paths(&self) -> io::Result<PathSnapshot> {
        let mut snapshot = PathSnapshot::default();
        snapshot_directory(
            self.descriptor.as_fd(),
            "",
            self.owner,
            &mut snapshot,
            &mut HashSet::new(),
        )?;
        Ok(snapshot)
    }
}

#[derive(Default)]
struct PathSnapshot {
    directories: BTreeSet<String>,
    files: BTreeSet<String>,
}

fn snapshot_directory(
    descriptor: BorrowedFd<'_>,
    prefix: &str,
    owner: u64,
    snapshot: &mut PathSnapshot,
    seen: &mut HashSet<(i128, u64)>,
) -> io::Result<()> {
    let mut names = Dir::read_from(descriptor)?
        .map(|entry| entry.map(|entry| OsStr::from_bytes(entry.file_name().to_bytes()).to_owned()))
        .collect::<rustix::io::Result<Vec<_>>>()
        .map_err(io::Error::from)?;
    names.retain(|name| name != "." && name != "..");
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let utf8 = name
            .to_str()
            .ok_or_else(|| invalid_data("R59 plaintext path is not UTF-8"))?;
        validate_component(utf8)?;
        let relative = if prefix.is_empty() {
            utf8.to_owned()
        } else {
            format!("{prefix}/{utf8}")
        };
        let child = fs(openat(
            descriptor,
            &name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ))?;
        let metadata = metadata(&fs(fstat(&child))?);
        require(
            metadata.owner == owner && seen.insert((metadata.dev, metadata.ino)),
            "R59 plaintext entry owner or inode drift",
        )?;
        if metadata.file_type.is_dir() {
            require(
                metadata.mode & 0o7777 == 0o700,
                "R59 plaintext directory mode drift",
            )?;
            snapshot.directories.insert(relative.clone());
            snapshot_directory(child.as_fd(), &relative, owner, snapshot, seen)?;
        } else if metadata.file_type.is_file() {
            require(
                metadata.mode & 0o7777 == 0o600,
                "R59 plaintext file mode drift",
            )?;
            snapshot.files.insert(relative);
        } else {
            return Err(invalid_data("R59 plaintext entry is not regular"));
        }
    }
    Ok(())
}

fn expected_directories(paths: &BTreeSet<String>) -> BTreeSet<String> {
    let mut directories = BTreeSet::new();
    for path in paths {
        let mut current = String::new();
        for component in path.split('/').take(path.split('/').count() - 1) {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(component);
            directories.insert(current.clone());
        }
    }
    directories
}

fn validate_archive(
    bytes: &[u8],
    root: &HeldRoot,
    files: &BTreeSet<String>,
    directories: &BTreeSet<String>,
    strict_raw_ustar: bool,
) -> io::Result<()> {
    require(
        bytes.len() >= 1024 && bytes.len().is_multiple_of(512),
        "R59 archive block length drift",
    )?;
    let mut offset = 0;
    let mut previous: Option<String> = None;
    let mut archive_files = BTreeSet::new();
    let mut archive_directories = BTreeSet::new();
    while offset + 1024 <= bytes.len() && bytes[offset..offset + 512].iter().any(|byte| *byte != 0)
    {
        let header = &bytes[offset..offset + 512];
        validate_tar_checksum(header)?;
        let uid = parse_tar_number(&header[108..116])?;
        let gid = parse_tar_number(&header[116..124])?;
        let size = parse_tar_number(&header[124..136])?;
        let mtime = parse_tar_number(&header[136..148])?;
        require(
            &header[257..263] == b"ustar\0"
                && &header[263..265] == b"00"
                && header[157..257].iter().all(|byte| *byte == 0)
                && header[265..512].iter().all(|byte| *byte == 0)
                && uid == 0
                && gid == 0
                && mtime == 0
                && canonical_tar_number(&header[108..116], uid)
                && canonical_tar_number(&header[116..124], gid)
                && canonical_tar_number(&header[124..136], size)
                && canonical_tar_number(&header[136..148], mtime),
            "R59 archive ustar metadata drift",
        )?;
        let raw_name = tar_string(&header[..100])?;
        let is_directory = header[156] == b'5';
        let name = if is_directory {
            raw_name
                .strip_suffix('/')
                .ok_or_else(|| invalid_data("R59 archive directory name drift"))?
        } else {
            raw_name
        };
        validate_relative_path(name)?;
        require(
            previous.as_ref().is_none_or(|value| value.as_str() < name),
            "R59 archive pathname order drift",
        )?;
        previous = Some(name.to_owned());
        let mode = parse_tar_number(&header[100..108])?;
        require(
            canonical_tar_number(&header[100..108], mode),
            "R59 archive mode encoding drift",
        )?;
        if is_directory {
            require(
                size == 0 && mode == 0o700 && archive_directories.insert(name.to_owned()),
                "R59 archive directory drift",
            )?;
        } else {
            require(
                header[156] == b'0' && mode == 0o600 && archive_files.insert(name.to_owned()),
                "R59 archive regular file drift",
            )?;
            let size = usize::try_from(size)
                .map_err(|_| invalid_data("R59 archive file length overflow"))?;
            let data_start = offset + 512;
            let data_end = data_start
                .checked_add(size)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| invalid_data("R59 archive is truncated"))?;
            let held = root.read_file(name)?;
            require(
                held.bytes.as_ref() == &bytes[data_start..data_end],
                "R59 archive/file byte drift",
            )?;
            if strict_raw_ustar {
                let padded = size
                    .checked_add(511)
                    .map(|size| size / 512 * 512)
                    .ok_or_else(|| invalid_data("R59 archive size overflow"))?;
                let padded_end = data_start
                    .checked_add(padded)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| invalid_data("R59 archive is truncated"))?;
                require(
                    bytes[data_end..padded_end].iter().all(|byte| *byte == 0),
                    "R59 archive file padding drift",
                )?;
            }
        }
        let padded = usize::try_from(size)
            .ok()
            .and_then(|size| size.checked_add(511))
            .map(|size| size / 512 * 512)
            .ok_or_else(|| invalid_data("R59 archive size overflow"))?;
        offset = offset
            .checked_add(512 + padded)
            .ok_or_else(|| invalid_data("R59 archive offset overflow"))?;
    }
    require(
        archive_files == *files
            && archive_directories == *directories
            && offset + 1024 <= bytes.len()
            && bytes[offset..].iter().all(|byte| *byte == 0)
            && (!strict_raw_ustar || offset + 1024 == bytes.len()),
        "R59 archive entry set or terminator drift",
    )
}

fn validate_tar_checksum(header: &[u8]) -> io::Result<()> {
    let expected = parse_tar_number(&header[148..156])?;
    let actual = header
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum::<u64>();
    let canonical = format!("{expected:06o}\0 ");
    require(
        expected == actual && &header[148..156] == canonical.as_bytes(),
        "R59 archive checksum drift",
    )
}

fn canonical_tar_number(field: &[u8], value: u64) -> bool {
    let canonical = format!("{value:0width$o}\0", width = field.len() - 1);
    field == canonical.as_bytes()
}

fn parse_tar_number(bytes: &[u8]) -> io::Result<u64> {
    require(
        !bytes.is_empty()
            && bytes
                .iter()
                .all(|byte| matches!(*byte, 0 | b' ' | b'0'..=b'7')),
        "R59 archive numeric field drift",
    )?;
    let text = bytes
        .split(|byte| *byte == 0 || *byte == b' ')
        .next()
        .unwrap_or_default();
    if text.is_empty() {
        Ok(0)
    } else {
        let text = std::str::from_utf8(text)
            .map_err(|_| invalid_data("R59 archive numeric field is not ASCII"))?;
        u64::from_str_radix(text, 8)
            .map_err(|_| invalid_data("R59 archive numeric field is invalid"))
    }
}

fn tar_string(bytes: &[u8]) -> io::Result<&str> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    require(
        bytes[end..].iter().all(|byte| *byte == 0),
        "R59 archive string padding drift",
    )?;
    std::str::from_utf8(&bytes[..end]).map_err(|_| invalid_data("R59 archive path is not UTF-8"))
}

fn canonical_json<T>(bytes: &[u8]) -> io::Result<T>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let value: T = serde_json::from_slice(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    require(canonical == bytes, "R59 JSON is not canonical")?;
    Ok(value)
}

fn validate_source_path<'a>(path: &'a str, entry_id: &str) -> io::Result<&'a str> {
    validate_relative_path(path)?;
    let prefix = format!("assets/source/{entry_id}.");
    require(
        path.strip_prefix(&prefix)
            .is_some_and(|extension| matches!(extension, "png" | "jpg" | "jpeg" | "webp")),
        "R59 source path mapping drift",
    )?;
    Ok(path)
}

fn validate_relative_path(path: &str) -> io::Result<()> {
    require(
        !path.is_empty()
            && !path.starts_with('/')
            && !path.ends_with('/')
            && !path.contains('\\')
            && !path.contains('\0')
            && path
                .split('/')
                .all(|component| !component.is_empty() && component != "." && component != ".."),
        "R59 relative path drift",
    )
}

fn validate_component(component: &str) -> io::Result<()> {
    require(
        !component.is_empty()
            && component != "."
            && component != ".."
            && !component.contains('/')
            && !component.contains('\\')
            && !component.contains('\0'),
        "R59 path component drift",
    )
}

fn valid_target_id(value: &str) -> bool {
    !value.is_empty()
        && value.is_ascii()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn recompute_aspect(width: u32, height: u32) -> &'static str {
    if u64::from(width) * 10 > u64::from(height) * 11 {
        "landscape"
    } else if u64::from(height) * 10 > u64::from(width) * 11 {
        "portrait"
    } else {
        "square_or_near"
    }
}

fn recompute_dimension_bin(width: u32, height: u32) -> &'static str {
    match width.max(height) {
        0..=719 => "lt720",
        720..=1439 => "720_1439",
        1440..=2159 => "1440_2159",
        _ => "gte2160",
    }
}

fn absolute_components(path: &Path) -> io::Result<Vec<OsString>> {
    let bytes = path.as_os_str().as_bytes();
    require(
        bytes.len() >= 2 && bytes[0] == b'/' && bytes[1] != b'/' && !bytes.ends_with(b"/"),
        "R59 root path is not canonical absolute",
    )?;
    bytes[1..]
        .split(|byte| *byte == b'/')
        .map(|component| {
            require(
                !component.is_empty() && component != b"." && component != b"..",
                "R59 root path component drift",
            )?;
            Ok(OsStr::from_bytes(component).to_owned())
        })
        .collect()
}

fn relative_components(path: &str) -> io::Result<Vec<OsString>> {
    validate_relative_path(path)?;
    Ok(path
        .split('/')
        .map(|component| OsString::from(component))
        .collect())
}

fn metadata(stat: &rustix::fs::Stat) -> Metadata {
    Metadata {
        dev: i128::from(stat.st_dev),
        ino: stat.st_ino,
        owner: stat.st_uid.into(),
        mode: stat.st_mode.into(),
        file_type: FileType::from_raw_mode(stat.st_mode),
    }
}

fn effective_owner() -> io::Result<u64> {
    let (socket, _peer) = UnixStream::pair()?;
    Ok(metadata(&fs(fstat(&socket))?).owner)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes))
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn require(condition: bool, message: &'static str) -> io::Result<()> {
    condition.then_some(()).ok_or_else(|| invalid_data(message))
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn fs<T>(result: rustix::io::Result<T>) -> io::Result<T> {
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use image::{GrayImage, ImageEncoder, Luma, Rgba};
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    struct Fixture {
        _temp: TempDir,
        root: std::path::PathBuf,
        archive: std::path::PathBuf,
        archive_bytes: Vec<u8>,
        freeze: [String; 4],
        contract: &'static BundleContract,
    }

    impl Fixture {
        fn build() -> Self {
            Self::build_for(&R59_BUNDLE_CONTRACT)
        }

        fn build_r60() -> Self {
            Self::build_for(&R60_BUNDLE_CONTRACT)
        }

        fn build_for(contract: &'static BundleContract) -> Self {
            let temp = TempDir::new().unwrap();
            let base = fs::canonicalize(temp.path()).unwrap();
            chmod(&base, 0o700);
            let root = base.join("plaintext");
            fs::create_dir(&root).unwrap();
            chmod(&root, 0o700);
            for directory in ["assets", "assets/source", "assets/clean", "assets/masks"] {
                fs::create_dir(root.join(directory)).unwrap();
                chmod(&root.join(directory), 0o700);
            }

            let mut manifest_entries = Vec::new();
            let mut oracle_entries = Vec::new();
            let mut assets = BTreeMap::new();
            for id in contract.entry_ids {
                let mask_directory = root.join(format!("assets/masks/{id}"));
                fs::create_dir(&mask_directory).unwrap();
                chmod(&mask_directory, 0o700);
                let source_path = format!("assets/source/{id}.png");
                let clean_path = format!("assets/clean/{id}.png");
                let erase_path = format!("assets/masks/{id}/text-erase.png");
                let residual_path = format!("assets/masks/{id}/text-residual.png");
                let mut source = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
                source.put_pixel(1, 1, Rgba([0, 0, 0, 255]));
                let clean = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
                let mut mask = GrayImage::from_pixel(4, 4, Luma([0]));
                mask.put_pixel(1, 1, Luma([255]));
                let source_bytes = encode_rgba(&source);
                let clean_bytes = encode_rgba(&clean);
                let mask_bytes = encode_gray(&mask);
                write_asset(&root, &source_path, &source_bytes);
                write_asset(&root, &clean_path, &clean_bytes);
                write_asset(&root, &erase_path, &mask_bytes);
                write_asset(&root, &residual_path, &mask_bytes);
                assets.insert(source_path.clone(), image_record(&source_bytes));
                assets.insert(clean_path.clone(), image_record(&clean_bytes));
                assets.insert(erase_path.clone(), mask_record(&mask_bytes, contract));
                assets.insert(residual_path.clone(), mask_record(&mask_bytes, contract));
                manifest_entries.push(json!({
                    "aspect": "square_or_near",
                    "background": "pure",
                    "clean_reference_relpath": clean_path,
                    "dimension_bin": "lt720",
                    "id": id,
                    "multi_node": false,
                    "protected_rois": [[3, 3, 4, 4]],
                    "role": "holdout",
                    "source_relpath": source_path,
                    "targets": [{
                        "clean_reference_edit_roi": [1, 1, 2, 2],
                        "effect": "plain",
                        "erase_source_ink_mask_relpath": erase_path,
                        "expected": "automatic_strict",
                        "id": "text",
                        "position": "interior",
                        "residual_source_ink_mask_relpath": residual_path,
                        "source_roi": [1, 1, 2, 2],
                        "translation_length": "equal",
                        "writing": "horizontal"
                    }]
                }));
                let text = "汉";
                let mut oracle_entry = json!({
                    "id": id,
                    "targets": [{
                        "expected_decision": "select",
                        "expected_rejection_reason": null,
                        "id": "text",
                        "source_han_scalar_count": 1,
                        "source_script_class": "han_or_mixed",
                        "source_text_sha256": sha256_hex(text.as_bytes()),
                        "source_text_utf8": text
                    }]
                });
                if contract.protected_texts_required {
                    let protected_text = "Product ID";
                    oracle_entry["protected_texts"] = json!([{
                        "roi": [3, 3, 4, 4],
                        "source_script_class": "latin_or_ascii",
                        "source_text_sha256": sha256_hex(protected_text.as_bytes()),
                        "source_text_utf8": protected_text
                    }]);
                }
                oracle_entries.push(oracle_entry);
            }
            let manifest = canonical_value(json!({
                "contract": contract.manifest_contract,
                "entries": manifest_entries,
                "plan_revision": contract.plan_revision,
                "role": "holdout"
            }));
            let oracle = canonical_value(json!({
                "contract": contract.oracle_contract,
                "entries": oracle_entries,
                "manifest_sha256": sha256_hex(&manifest),
                "plan_revision": contract.plan_revision
            }));
            let hashes = canonical_value(json!({
                "assets": assets,
                "contract": contract.hashes_contract,
                "manifest_sha256": sha256_hex(&manifest),
                "oracle_sha256": sha256_hex(&oracle),
                "plan_revision": contract.plan_revision
            }));
            write_asset(&root, MANIFEST_NAME, &manifest);
            write_asset(&root, ORACLE_NAME, &oracle);
            write_asset(&root, HASHES_NAME, &hashes);

            let paths = snapshot_fs(&root);
            let archive_bytes = tar(&root, &paths);
            let archive = base.join("holdout.tar");
            fs::write(&archive, &archive_bytes).unwrap();
            chmod(&archive, 0o600);
            Self {
                _temp: temp,
                root,
                archive,
                freeze: [
                    sha256_hex(&archive_bytes),
                    sha256_hex(&manifest),
                    sha256_hex(&oracle),
                    sha256_hex(&hashes),
                ],
                archive_bytes,
                contract,
            }
        }

        fn commitments(&self) -> R59FreezeCommitments<'_> {
            R59FreezeCommitments {
                plaintext_archive_sha256: &self.freeze[0],
                manifest_sha256: &self.freeze[1],
                oracle_sha256: &self.freeze[2],
                hashes_sha256: &self.freeze[3],
            }
        }

        fn validate(&self) -> io::Result<R59ValidatedBundle> {
            match self.contract.plan_revision {
                59 => validate_r59_plaintext_holdout_bundle(
                    &self.root,
                    &self.archive,
                    &self.archive_bytes,
                    self.commitments(),
                ),
                60 => validate_r60_plaintext_holdout_bundle(
                    &self.root,
                    &self.archive,
                    &self.archive_bytes,
                    self.commitments(),
                ),
                _ => unreachable!(),
            }
        }

        fn rewrite_oracle(&mut self, mutate: impl FnOnce(&mut serde_json::Value)) {
            self.rewrite_json(ORACLE_NAME, mutate);
            let oracle_sha256 = self.freeze[2].clone();
            self.rewrite_json(HASHES_NAME, |value| {
                value["oracle_sha256"] = serde_json::Value::String(oracle_sha256);
            });
            self.refresh_archive();
        }

        fn rewrite_manifest(&mut self, mutate: impl FnOnce(&mut serde_json::Value)) {
            self.rewrite_json(MANIFEST_NAME, mutate);
            let manifest_sha256 = self.freeze[1].clone();
            self.rewrite_json(ORACLE_NAME, |value| {
                value["manifest_sha256"] = serde_json::Value::String(manifest_sha256.clone());
            });
            let oracle_sha256 = self.freeze[2].clone();
            self.rewrite_json(HASHES_NAME, |value| {
                value["manifest_sha256"] = serde_json::Value::String(manifest_sha256);
                value["oracle_sha256"] = serde_json::Value::String(oracle_sha256);
            });
            self.refresh_archive();
        }

        fn rewrite_hashes(&mut self, mutate: impl FnOnce(&mut serde_json::Value)) {
            self.rewrite_json(HASHES_NAME, mutate);
            self.refresh_archive();
        }

        fn replace_image(&mut self, relative: &str, bytes: &[u8]) {
            fs::write(self.root.join(relative), bytes).unwrap();
            chmod(&self.root.join(relative), 0o600);
            let record = image_record(bytes);
            self.rewrite_json(HASHES_NAME, |value| value["assets"][relative] = record);
            self.refresh_archive();
        }

        fn replace_mask(&mut self, relative: &str, bytes: &[u8]) {
            fs::write(self.root.join(relative), bytes).unwrap();
            chmod(&self.root.join(relative), 0o600);
            let record = mask_record(bytes, self.contract);
            self.rewrite_json(HASHES_NAME, |value| value["assets"][relative] = record);
            self.refresh_archive();
        }

        fn refresh_archive(&mut self) {
            self.archive_bytes = tar(&self.root, &snapshot_fs(&self.root));
            fs::write(&self.archive, &self.archive_bytes).unwrap();
            chmod(&self.archive, 0o600);
            self.freeze[0] = sha256_hex(&self.archive_bytes);
        }

        fn rewrite_json(&mut self, name: &str, mutate: impl FnOnce(&mut serde_json::Value)) {
            let path = self.root.join(name);
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            mutate(&mut value);
            let bytes = canonical_value(value);
            fs::write(path, &bytes).unwrap();
            chmod(&self.root.join(name), 0o600);
            match name {
                MANIFEST_NAME => self.freeze[1] = sha256_hex(&bytes),
                ORACLE_NAME => self.freeze[2] = sha256_hex(&bytes),
                HASHES_NAME => self.freeze[3] = sha256_hex(&bytes),
                _ => unreachable!(),
            }
        }
    }

    fn rejection(fixture: &Fixture) -> String {
        match fixture.validate() {
            Ok(_) => panic!("fixture unexpectedly passed"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn d0_r59_holdout_bundle_accepts_closed_synthetic_bundle() {
        let fixture = Fixture::build();
        let validated = fixture.validate().unwrap();
        let receipt = &validated.receipt;
        assert!(
            receipt.schema_validation_pass
                && receipt.asset_binding_pass
                && receipt.mask_source_clean_equality_pass
                && receipt.oracle_semantics_pass
        );
        assert_eq!(receipt.plaintext_archive_sha256, fixture.freeze[0]);
        assert_eq!(receipt.manifest_sha256, fixture.freeze[1]);
        assert_eq!(receipt.oracle_sha256, fixture.freeze[2]);
        assert_eq!(receipt.hashes_sha256, fixture.freeze[3]);
        assert_eq!(
            validated
                .execution
                .entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            R59_ENTRY_IDS
        );
        for entry in &validated.execution.entries {
            assert_eq!(
                (
                    entry.source_width,
                    entry.source_height,
                    entry.clean_width,
                    entry.clean_height
                ),
                (4, 4, 4, 4)
            );
            assert_eq!(entry.protected_rois, [[3, 3, 4, 4]]);
            assert_eq!(entry.targets.len(), 1);
            let target = &entry.targets[0];
            assert_eq!(target.id, "text");
            assert_eq!(target.source_roi, [1, 1, 2, 2]);
            assert_eq!(target.clean_reference_edit_roi, [1, 1, 2, 2]);
            assert_eq!(target.validated_binary_mask.iter().sum::<u8>(), 1);
            assert_eq!(target.expected, "automatic_strict");
            assert_eq!(target.writing, "horizontal");
            assert_eq!(target.effect, "plain");
            assert_eq!(target.position, "interior");
            assert_eq!(target.translation_length, "equal");
            assert_eq!(entry.source_dynamic_image().width(), 4);
            assert_eq!(entry.validated_source_rgba.dimensions(), (4, 4));
            assert!(!entry.source_encoded_bytes.is_empty());
        }
    }

    #[test]
    fn d0_r60_holdout_bundle_accepts_closed_synthetic_bundle() {
        let fixture = Fixture::build_r60();
        let validated = fixture.validate().unwrap();
        assert_eq!(
            validated
                .execution
                .entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            R60_ENTRY_IDS
        );
        assert!(validated.receipt.schema_validation_pass);
    }

    #[test]
    fn d0_r60_archive_accepts_exact_ustar_terminator() {
        let fixture = Fixture::build_r60();
        validate_synthetic_archive(&fixture, &fixture.archive_bytes).unwrap();
    }

    #[test]
    fn d0_r60_archive_rejects_nonzero_file_padding() {
        let fixture = Fixture::build_r60();
        let mut archive = fixture.archive_bytes.clone();
        let padding_offset = first_file_padding_offset(&archive);
        archive[padding_offset] = 1;
        assert_eq!(
            validate_synthetic_archive(&fixture, &archive)
                .unwrap_err()
                .to_string(),
            "R59 archive file padding drift"
        );
    }

    #[test]
    fn d0_r60_archive_rejects_one_zero_block_terminator() {
        let fixture = Fixture::build_r60();
        let archive = &fixture.archive_bytes[..fixture.archive_bytes.len() - 512];
        assert!(validate_synthetic_archive(&fixture, archive).is_err());
    }

    #[test]
    fn d0_r60_archive_rejects_three_zero_block_terminator() {
        let fixture = Fixture::build_r60();
        let mut archive = fixture.archive_bytes.clone();
        archive.extend_from_slice(&[0; 512]);
        assert!(validate_synthetic_archive(&fixture, &archive).is_err());
    }

    #[test]
    fn d0_r60_archive_rejects_trailing_bytes() {
        let fixture = Fixture::build_r60();
        let mut archive = fixture.archive_bytes.clone();
        archive.push(0);
        assert!(validate_synthetic_archive(&fixture, &archive).is_err());
    }

    #[test]
    fn d0_r59_archive_keeps_legacy_padding_and_terminator_compatibility() {
        let fixture = Fixture::build();
        let mut archive = fixture.archive_bytes.clone();
        let padding_offset = first_file_padding_offset(&archive);
        archive[padding_offset] = 1;
        archive.extend_from_slice(&[0; 512]);
        validate_synthetic_archive(&fixture, &archive).unwrap();
    }

    #[test]
    fn d0_r60_holdout_bundle_keeps_r59_mask_contract_distinct() {
        let mut gray = GrayImage::from_pixel(2, 2, Luma([0]));
        gray.put_pixel(0, 0, Luma([255]));
        let gray = encode_gray(&gray);
        let r59 = decode_mask(&gray, &R59_BUNDLE_CONTRACT).unwrap();
        let r60 = decode_mask(&gray, &R60_BUNDLE_CONTRACT).unwrap();
        assert_ne!(
            r59.normalized_identity_sha256,
            r60.normalized_identity_sha256
        );

        let mut rgba = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]));
        rgba.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        let rgba = encode_rgba(&rgba);
        assert!(decode_mask(&rgba, &R59_BUNDLE_CONTRACT).is_ok());
        assert!(decode_mask(&rgba, &R60_BUNDLE_CONTRACT).is_err());
    }

    #[test]
    fn d0_r60_holdout_bundle_rejects_schema_geometry_and_latin_drift() {
        let mut fixture = Fixture::build_r60();
        fixture.rewrite_oracle(|value| {
            value["entries"][0]["targets"][0]["id"] = json!("different");
        });
        assert_eq!(rejection(&fixture), "R59 oracle order or identity drift");

        for invalid in ["A😀", "A\n", "汉A"] {
            let mut fixture = Fixture::build_r60();
            fixture.rewrite_oracle(|value| {
                value["entries"][0]["protected_texts"][0]["source_text_utf8"] = json!(invalid);
                value["entries"][0]["protected_texts"][0]["source_text_sha256"] =
                    json!(sha256_hex(invalid.as_bytes()));
            });
            assert_eq!(rejection(&fixture), "R60 protected-text semantic drift");
        }

        let mut fixture = Fixture::build_r60();
        let clean = encode_rgba(&RgbaImage::from_pixel(5, 4, Rgba([255, 255, 255, 255])));
        fixture.replace_image("assets/clean/r60-h01.png", &clean);
        assert_eq!(rejection(&fixture), "R59 Source/Clean dimensions drift");
    }

    #[test]
    fn d0_r60_holdout_bundle_rejects_pure_latin_target() {
        let mut fixture = Fixture::build_r60();
        fixture.rewrite_oracle(|value| {
            let text = "Product";
            let target = &mut value["entries"][0]["targets"][0];
            target["source_text_utf8"] = json!(text);
            target["source_text_sha256"] = json!(sha256_hex(text.as_bytes()));
            target["source_han_scalar_count"] = json!(0);
            target["source_script_class"] = json!("latin_or_ascii");
        });
        assert_eq!(rejection(&fixture), "R59 oracle semantic drift");
    }

    #[test]
    fn d0_r60_holdout_bundle_rejects_closed_schema_and_mask_drift() {
        let mut fixture = Fixture::build_r60();
        fixture.rewrite_manifest(|value| value["plan_revision"] = json!(59));
        assert_eq!(
            rejection(&fixture),
            "R59 manifest/oracle/hashes mutual binding drift"
        );

        let mut fixture = Fixture::build_r60();
        fixture.rewrite_manifest(|value| value["contract"] = json!("wrong"));
        assert_eq!(
            rejection(&fixture),
            "R59 manifest/oracle/hashes mutual binding drift"
        );

        let mut fixture = Fixture::build_r60();
        fixture.rewrite_oracle(|value| value["contract"] = json!("wrong"));
        assert_eq!(
            rejection(&fixture),
            "R59 manifest/oracle/hashes mutual binding drift"
        );
        let mut fixture = Fixture::build_r60();
        fixture.rewrite_oracle(|value| value["plan_revision"] = json!(59));
        assert_eq!(
            rejection(&fixture),
            "R59 manifest/oracle/hashes mutual binding drift"
        );
        let mut fixture = Fixture::build_r60();
        fixture.rewrite_hashes(|value| value["contract"] = json!("wrong"));
        assert_eq!(
            rejection(&fixture),
            "R59 manifest/oracle/hashes mutual binding drift"
        );
        let mut fixture = Fixture::build_r60();
        fixture.rewrite_hashes(|value| value["plan_revision"] = json!(59));
        assert_eq!(
            rejection(&fixture),
            "R59 manifest/oracle/hashes mutual binding drift"
        );

        let mut fixture = Fixture::build_r60();
        fixture.rewrite_manifest(|value| value["entries"][0]["id"] = json!("r60-h05"));
        assert_eq!(rejection(&fixture), "R59 manifest entry drift");

        let mut fixture = Fixture::build_r60();
        fixture.rewrite_hashes(|value| {
            value["assets"]["assets/source/r60-h01.png"]["normalized_identity_sha256"] =
                json!("0".repeat(64));
        });
        assert_eq!(rejection(&fixture), "R59 decoded image identity drift");

        let mut fixture = Fixture::build_r60();
        fixture.rewrite_hashes(|value| {
            value["assets"]["assets/source/r60-h01.png"]["decoded_kind"] = json!("wrong");
        });
        assert_eq!(rejection(&fixture), "R59 asset hash record drift");

        let mut non_binary = GrayImage::from_pixel(4, 4, Luma([0]));
        non_binary.put_pixel(1, 1, Luma([1]));
        let error = match decode_mask(&encode_gray(&non_binary), &R60_BUNDLE_CONTRACT) {
            Ok(_) => panic!("non-binary mask unexpectedly passed"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "R59 mask is not binary");

        let mut fixture = Fixture::build_r60();
        let mut residual = GrayImage::from_pixel(4, 4, Luma([0]));
        residual.put_pixel(2, 2, Luma([255]));
        fixture.replace_mask(
            "assets/masks/r60-h01/text-residual.png",
            &encode_gray(&residual),
        );
        assert_eq!(rejection(&fixture), "R59 erase/residual mask drift");
    }

    #[test]
    fn d0_r60_holdout_bundle_rejects_pixel_and_protected_text_drift() {
        let mut fixture = Fixture::build_r60();
        let mut clean = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
        clean.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
        fixture.replace_image("assets/clean/r60-h01.png", &encode_rgba(&clean));
        assert_eq!(
            rejection(&fixture),
            "R59 Source/Clean delta exists outside target masks"
        );

        let mut fixture = Fixture::build_r60();
        fixture.rewrite_oracle(|value| {
            value["entries"][0]
                .as_object_mut()
                .unwrap()
                .remove("protected_texts");
        });
        assert_eq!(
            rejection(&fixture),
            "R60 protected-text ground truth is missing"
        );

        let mut fixture = Fixture::build_r60();
        let text = "Δelta";
        fixture.rewrite_oracle(|value| {
            let protected = &mut value["entries"][0]["protected_texts"][0];
            protected["source_text_utf8"] = json!(text);
            protected["source_text_sha256"] = json!(sha256_hex(text.as_bytes()));
        });
        assert_eq!(rejection(&fixture), "R60 protected-text semantic drift");
    }

    #[test]
    fn d0_r60_holdout_bundle_checks_alignment_pixels_and_protected_order() {
        let fixture = Fixture::build_r60();
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(fixture.root.join(MANIFEST_NAME)).unwrap()).unwrap();
        let mut oracle: Oracle =
            serde_json::from_slice(&fs::read(fixture.root.join(ORACLE_NAME)).unwrap()).unwrap();
        let manifest_entry = &mut manifest.entries[0];
        let oracle_entry = &mut oracle.entries[0];

        let mut second_target: ManifestTarget =
            serde_json::from_value(serde_json::to_value(&manifest_entry.targets[0]).unwrap())
                .unwrap();
        second_target.id = "text2".into();
        manifest_entry.targets.push(second_target);
        let mut second_oracle: OracleTarget =
            serde_json::from_value(serde_json::to_value(&oracle_entry.targets[0]).unwrap())
                .unwrap();
        second_oracle.id = "text2".into();
        oracle_entry.targets.push(second_oracle);
        validate_oracle_alignment(manifest_entry, oracle_entry, "r60-h01").unwrap();
        oracle_entry.targets.swap(0, 1);
        assert_eq!(
            validate_oracle_alignment(manifest_entry, oracle_entry, "r60-h01")
                .unwrap_err()
                .to_string(),
            "R59 oracle order or identity drift"
        );
        oracle_entry.targets.pop();
        assert_eq!(
            validate_oracle_alignment(manifest_entry, oracle_entry, "r60-h01")
                .unwrap_err()
                .to_string(),
            "R59 oracle order or identity drift"
        );

        let fixture = Fixture::build_r60();
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(fixture.root.join(MANIFEST_NAME)).unwrap()).unwrap();
        let entry = &manifest.entries[0];
        let source =
            decode_image(&fs::read(fixture.root.join("assets/source/r60-h01.png")).unwrap())
                .unwrap()
                .rgba;
        let mask = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            validate_pixels(
                entry,
                &source,
                &source,
                &[(mask.clone(), Box::new([]), Box::new([]))],
            )
            .unwrap_err()
            .to_string(),
            "R59 mask foreground is outside exact Source/Clean delta"
        );

        let mut entry_value = serde_json::to_value(entry).unwrap();
        let mut second = entry_value["targets"][0].clone();
        second["id"] = json!("text2");
        entry_value["targets"].as_array_mut().unwrap().push(second);
        let entry: ManifestEntry = serde_json::from_value(entry_value).unwrap();
        assert_eq!(
            validate_pixels(
                &entry,
                &source,
                &RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255])),
                &[
                    (mask.clone(), Box::new([]), Box::new([])),
                    (mask, Box::new([]), Box::new([])),
                ],
            )
            .unwrap_err()
            .to_string(),
            "R59 target masks overlap"
        );

        let fixture = Fixture::build_r60();
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(fixture.root.join(MANIFEST_NAME)).unwrap()).unwrap();
        let mut oracle: Oracle =
            serde_json::from_slice(&fs::read(fixture.root.join(ORACLE_NAME)).unwrap()).unwrap();
        let manifest_entry = &mut manifest.entries[0];
        let oracle_entry = &mut oracle.entries[0];
        manifest_entry.protected_rois.insert(0, Rect([2, 3, 3, 4]));
        let mut second: OracleProtectedText = serde_json::from_value(
            serde_json::to_value(&oracle_entry.protected_texts.as_ref().unwrap()[0]).unwrap(),
        )
        .unwrap();
        second.roi = Rect([2, 3, 3, 4]);
        second.source_text_utf8 = "SKU-42!".into();
        second.source_text_sha256 = sha256_hex(second.source_text_utf8.as_bytes());
        oracle_entry
            .protected_texts
            .as_mut()
            .unwrap()
            .insert(0, second);
        validate_protected_texts(manifest_entry, oracle_entry, &R60_BUNDLE_CONTRACT).unwrap();
        oracle_entry.protected_texts.as_mut().unwrap().swap(0, 1);
        assert_eq!(
            validate_protected_texts(manifest_entry, oracle_entry, &R60_BUNDLE_CONTRACT)
                .unwrap_err()
                .to_string(),
            "R60 protected-text ROI binding drift"
        );

        let mut fixture = Fixture::build_r60();
        fixture.rewrite_oracle(|value| {
            let extra = value["entries"][0]["protected_texts"][0].clone();
            value["entries"][0]["protected_texts"]
                .as_array_mut()
                .unwrap()
                .push(extra);
        });
        assert_eq!(rejection(&fixture), "R60 protected-text ROI binding drift");
    }

    #[test]
    fn d0_r59_holdout_bundle_rejects_canonical_drift() {
        let fixture = Fixture::build();
        let path = fixture.root.join(MANIFEST_NAME);
        let mut bytes = fs::read(&path).unwrap();
        bytes.push(b'\n');
        fs::write(path, bytes).unwrap();
        assert!(fixture.validate().is_err());
    }

    #[test]
    fn d0_r59_holdout_bundle_rejects_mutual_hash_drift() {
        let mut fixture = Fixture::build();
        fixture.rewrite_json(HASHES_NAME, |value| {
            value["oracle_sha256"] = serde_json::Value::String("0".repeat(64));
        });
        assert!(fixture.validate().is_err());
    }

    #[test]
    fn d0_r59_holdout_bundle_rejects_mask_delta_mismatch() {
        let fixture = Fixture::build();
        let path = fixture.root.join("assets/clean/r59-h01.png");
        let mut clean = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
        clean.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
        fs::write(path, encode_rgba(&clean)).unwrap();
        assert!(fixture.validate().is_err());
    }

    #[test]
    fn d0_r59_holdout_bundle_rejects_oracle_semantic_drift() {
        let mut fixture = Fixture::build();
        fixture.rewrite_json(ORACLE_NAME, |value| {
            value["entries"][0]["targets"][0]["source_han_scalar_count"] =
                serde_json::Value::from(2);
        });
        assert!(fixture.validate().is_err());
    }

    #[test]
    fn d0_r59_holdout_bundle_nfc_accepts_uncomposed_mark_and_rejects_decomposed_form() {
        assert!(is_nfc("汉\u{301}").unwrap());
        assert!(!is_nfc("汉e\u{301}").unwrap());
        assert!(is_nfc("汉é").unwrap());
    }

    #[test]
    fn d0_r59_holdout_bundle_uses_frozen_aspect_boundary() {
        assert_eq!(recompute_aspect(110, 100), "square_or_near");
        assert_eq!(recompute_aspect(111, 100), "landscape");
        assert_eq!(recompute_aspect(100, 110), "square_or_near");
        assert_eq!(recompute_aspect(100, 111), "portrait");
    }

    #[test]
    fn d0_r59_holdout_bundle_rejects_path_and_symlink() {
        let mut fixture = Fixture::build();
        fixture.rewrite_json(MANIFEST_NAME, |value| {
            value["entries"][0]["source_relpath"] =
                serde_json::Value::String("assets/source/../r59-h01.png".into());
        });
        assert!(fixture.validate().is_err());

        let fixture = Fixture::build();
        let source = fixture.root.join("assets/source/r59-h01.png");
        fs::rename(&source, source.with_extension("real")).unwrap();
        symlink(source.with_extension("real"), &source).unwrap();
        assert!(fixture.validate().is_err());
    }

    fn image_record(bytes: &[u8]) -> serde_json::Value {
        let decoded = decode_image(bytes).unwrap();
        let (width, height) = decoded.rgba.dimensions();
        json!({
            "byte_length": bytes.len(),
            "decoded_kind": "sips-normalized-scanlines-v1",
            "height": height,
            "normalized_identity_sha256": decoded.normalized_identity_sha256,
            "raw_sha256": sha256_hex(bytes),
            "width": width
        })
    }

    fn mask_record(bytes: &[u8], contract: &BundleContract) -> serde_json::Value {
        let decoded = decode_mask(bytes, contract).unwrap();
        json!({
            "byte_length": bytes.len(),
            "decoded_kind": "binary-mask-v1",
            "height": decoded.height,
            "normalized_identity_sha256": decoded.normalized_identity_sha256,
            "raw_sha256": sha256_hex(bytes),
            "width": decoded.width
        })
    }

    fn encode_rgba(image: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    fn encode_gray(image: &GrayImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::L8,
            )
            .unwrap();
        bytes
    }

    fn canonical_value(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    fn write_asset(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        fs::write(&path, bytes).unwrap();
        chmod(&path, 0o600);
    }

    fn chmod(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn snapshot_fs(root: &Path) -> Vec<(String, bool)> {
        fn visit(root: &Path, relative: &Path, output: &mut Vec<(String, bool)>) {
            let mut entries = fs::read_dir(root.join(relative))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let child = relative.join(entry.file_name());
                let name = child.to_str().unwrap().to_owned();
                let directory = entry.file_type().unwrap().is_dir();
                output.push((name.clone(), directory));
                if directory {
                    visit(root, &child, output);
                }
            }
        }
        let mut output = Vec::new();
        visit(root, Path::new(""), &mut output);
        output.sort_by(|left, right| left.0.cmp(&right.0));
        output
    }

    fn validate_synthetic_archive(fixture: &Fixture, archive: &[u8]) -> io::Result<()> {
        let paths = snapshot_fs(&fixture.root);
        let files = paths
            .iter()
            .filter(|(_, directory)| !directory)
            .map(|(path, _)| path.clone())
            .collect();
        let directories = paths
            .iter()
            .filter(|(_, directory)| *directory)
            .map(|(path, _)| path.clone())
            .collect();
        validate_archive(
            archive,
            &HeldRoot::open(&fixture.root).unwrap(),
            &files,
            &directories,
            fixture.contract.plan_revision == 60,
        )
    }

    fn first_file_padding_offset(archive: &[u8]) -> usize {
        let mut offset = 0;
        loop {
            let header = &archive[offset..offset + 512];
            let size = usize::try_from(parse_tar_number(&header[124..136]).unwrap()).unwrap();
            if header[156] == b'0' && !size.is_multiple_of(512) {
                return offset + 512 + size;
            }
            offset += 512 + size.div_ceil(512) * 512;
        }
    }

    fn tar(root: &Path, paths: &[(String, bool)]) -> Vec<u8> {
        let mut output = Vec::new();
        for (path, directory) in paths {
            let mut header = [0_u8; 512];
            let name = if *directory {
                format!("{path}/")
            } else {
                path.clone()
            };
            header[..name.len()].copy_from_slice(name.as_bytes());
            tar_number(
                &mut header[100..108],
                if *directory { 0o700 } else { 0o600 },
            );
            tar_number(&mut header[108..116], 0);
            tar_number(&mut header[116..124], 0);
            let bytes = if *directory {
                Vec::new()
            } else {
                fs::read(root.join(path)).unwrap()
            };
            tar_number(&mut header[124..136], bytes.len() as u64);
            tar_number(&mut header[136..148], 0);
            header[148..156].fill(b' ');
            header[156] = if *directory { b'5' } else { b'0' };
            header[257..263].copy_from_slice(b"ustar\0");
            header[263..265].copy_from_slice(b"00");
            let checksum = header.iter().map(|byte| u64::from(*byte)).sum();
            tar_checksum(&mut header[148..156], checksum);
            output.extend_from_slice(&header);
            output.extend_from_slice(&bytes);
            output.resize(output.len().div_ceil(512) * 512, 0);
        }
        output.resize(output.len() + 1024, 0);
        output
    }

    fn tar_number(field: &mut [u8], value: u64) {
        let text = format!("{:0width$o}\0", value, width = field.len() - 1);
        field.copy_from_slice(text.as_bytes());
    }

    fn tar_checksum(field: &mut [u8], value: u64) {
        let text = format!("{:06o}\0 ", value);
        field.copy_from_slice(text.as_bytes());
    }
}
