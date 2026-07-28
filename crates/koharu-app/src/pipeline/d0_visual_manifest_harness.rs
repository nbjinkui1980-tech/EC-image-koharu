//! Ignored, manifest-only D0 visual evidence preflight.

use std::error::Error;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;

use rustix::fs::fstat;
use serde::{Deserialize, Serialize};

use super::d0_guarded_baseline::load_or_publish_target_correlation_map;
use super::d0_held_input::HeldInput;
use super::d0_output_transaction::{
    FaultPoint, PublishedOutput, publish_manifest_preflight_report,
};
use super::d0_revision_46_contract::BYTE_CEILING;
use super::d0_visual_manifest_oracles::validate_visual_oracles;
use super::d0_visual_manifest_pixels::{
    canonical_decoded_rgba_blake3, validate_dimensions_and_masks,
};
use super::d0_visual_manifest_schema::{HeldVisualManifestSchema, load_schema_and_hold_assets};

const VISUAL_INPUT_ENV: &str = "HANONLY_VISUAL_INPUT";
const VISUAL_INPUT_SHA256_ENV: &str = "HANONLY_VISUAL_INPUT_SHA256";
const VISUAL_MANIFEST_ENV: &str = "HANONLY_VISUAL_MANIFEST";
const VISUAL_MANIFEST_SHA256_ENV: &str = "HANONLY_VISUAL_MANIFEST_SHA256";
const VISUAL_EVIDENCE_ROOT_ENV: &str = "HANONLY_VISUAL_EVIDENCE_ROOT";
const SOURCE_GATE_FIXTURE_SHA256_ENV: &str = "HANONLY_SOURCE_GATE_FIXTURE_MANIFEST_SHA256";
const LEDGER_NAME: &str = "evidence-ledger.json";
const FIXTURE_RELATIVE_PATH: &str =
    "crates/koharu-app/tests/fixtures/source-gate-deterministic-recall/fixture-manifest.json";

type HarnessResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceLedger {
    version: u8,
    visual_input: String,
    visual_input_sha256: String,
    visual_manifest: String,
    visual_manifest_sha256: String,
    source_gate_fixture_manifest_sha256: String,
    evidence_root: String,
}

struct FrozenEnvironment {
    visual_input: String,
    visual_input_sha256: String,
    visual_manifest: String,
    visual_manifest_sha256: String,
    evidence_root: String,
    source_gate_fixture_manifest_sha256: String,
}

#[derive(Clone, Copy)]
struct HarnessSummary {
    entries: usize,
    targets: usize,
    masks: usize,
    protected_rois: usize,
    retained_bytes: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestPreflightReport {
    schema: String,
    image_input_contract: String,
    visual_input_sha256: String,
    visual_input_decoded_rgba_blake3: String,
    visual_manifest_sha256: String,
    source_gate_fixture_manifest_sha256: String,
    entries: usize,
    targets: usize,
    masks: usize,
    protected_rois: usize,
    retained_bytes: u64,
}

trait RevalidatedManifestAssets {
    fn with_revalidated_paths<T>(&self, action: impl FnOnce() -> io::Result<T>) -> io::Result<T>;
}

impl RevalidatedManifestAssets for HeldVisualManifestSchema {
    fn with_revalidated_paths<T>(&self, action: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        HeldVisualManifestSchema::with_revalidated_paths(self, action)
    }
}

impl FrozenEnvironment {
    fn from_process() -> HarnessResult<Self> {
        Ok(Self {
            visual_input: std::env::var(VISUAL_INPUT_ENV)?,
            visual_input_sha256: std::env::var(VISUAL_INPUT_SHA256_ENV)?,
            visual_manifest: std::env::var(VISUAL_MANIFEST_ENV)?,
            visual_manifest_sha256: std::env::var(VISUAL_MANIFEST_SHA256_ENV)?,
            evidence_root: std::env::var(VISUAL_EVIDENCE_ROOT_ENV)?,
            source_gate_fixture_manifest_sha256: std::env::var(SOURCE_GATE_FIXTURE_SHA256_ENV)?,
        })
    }

    fn validate_hashes(&self) -> io::Result<()> {
        for hash in [
            &self.visual_input_sha256,
            &self.visual_manifest_sha256,
            &self.source_gate_fixture_manifest_sha256,
        ] {
            decode_sha256(hash)?;
        }
        Ok(())
    }
}

impl EvidenceLedger {
    fn parse_and_validate(bytes: &[u8], environment: &FrozenEnvironment) -> io::Result<Self> {
        let ledger: Self = serde_json::from_slice(bytes)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        require(ledger.version == 1, "ledger version must be 1")?;
        for hash in [
            &ledger.visual_input_sha256,
            &ledger.visual_manifest_sha256,
            &ledger.source_gate_fixture_manifest_sha256,
        ] {
            decode_sha256(hash)?;
        }
        require(
            ledger.visual_input == environment.visual_input
                && ledger.visual_input_sha256 == environment.visual_input_sha256
                && ledger.visual_manifest == environment.visual_manifest
                && ledger.visual_manifest_sha256 == environment.visual_manifest_sha256
                && ledger.evidence_root == environment.evidence_root
                && ledger.source_gate_fixture_manifest_sha256
                    == environment.source_gate_fixture_manifest_sha256,
            "ledger does not exactly match the frozen environment",
        )?;
        Ok(ledger)
    }
}

fn run_manifest_only_preflight() -> HarnessResult<HarnessSummary> {
    let environment = FrozenEnvironment::from_process()?;
    environment.validate_hashes()?;

    let repository = repository_root()?;
    let input_path = PathBuf::from(&environment.visual_input);
    let manifest_path = PathBuf::from(&environment.visual_manifest);
    let evidence_root = PathBuf::from(&environment.evidence_root);
    for path in [&input_path, &manifest_path, &evidence_root] {
        require_absolute_canonical(path)?;
    }
    require(
        !evidence_root.starts_with(&repository),
        "evidence root must be outside the repository",
    )?;

    let ledger_path = evidence_root.join(LEDGER_NAME);
    require(
        ledger_path.parent() == Some(evidence_root.as_path()),
        "ledger parent must be the evidence root",
    )?;
    let ledger_input = HeldInput::open_bounded(&ledger_path, BYTE_CEILING)?;
    ledger_input.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    let ledger = EvidenceLedger::parse_and_validate(ledger_input.bytes(), &environment)?;

    let selected_input = HeldInput::open_bounded(&input_path, BYTE_CEILING)?;
    require(
        selected_input.sha256() == decode_sha256(&ledger.visual_input_sha256)?,
        "selected input sha256 mismatch",
    )?;
    let decoded_fingerprint = canonical_decoded_rgba_blake3(selected_input.bytes())?;

    let fixture_path = repository.join(FIXTURE_RELATIVE_PATH);
    require_absolute_canonical(&fixture_path)?;
    let fixture = HeldInput::open_bounded(&fixture_path, BYTE_CEILING)?;
    require(
        fixture.sha256() == decode_sha256(&ledger.source_gate_fixture_manifest_sha256)?,
        "source-gate fixture manifest sha256 mismatch",
    )?;
    validate_fixture_manifest(fixture.bytes())?;
    require_fixture_clean(&repository)?;

    let held_schema = load_schema_and_hold_assets(
        &manifest_path,
        &ledger.visual_manifest_sha256,
        &input_path,
        &decoded_fingerprint,
        &ledger.visual_input_sha256,
    )?;
    let validated = validate_visual_oracles(validate_dimensions_and_masks(held_schema)?)?;
    let targets = validated
        .upstream
        .held_schema
        .schema
        .entries
        .iter()
        .map(|entry| entry.targets.len())
        .sum();
    let summary = HarnessSummary {
        entries: validated.entries.len(),
        targets,
        masks: targets * 2,
        protected_rois: validated
            .upstream
            .held_schema
            .schema
            .entries
            .iter()
            .map(|entry| entry.protected_rois.len())
            .sum(),
        retained_bytes: validated.final_oracle_retained_bytes,
    };
    let report = canonical_report(&ledger, &decoded_fingerprint, summary)?;

    let revalidation = PreflightRevalidation {
        ledger: &ledger_input,
        selected_input: &selected_input,
        fixture: &fixture,
        assets: &validated.upstream.held_schema,
    };
    let mut correlation_fault = |_| Ok(());
    publish_revalidated_report(
        &revalidation,
        &evidence_root,
        &report,
        || {
            load_or_publish_target_correlation_map(
                &evidence_root,
                &validated.upstream.held_schema,
                &mut correlation_fault,
                |_, _| Ok(()),
            )
        },
        &mut |_| Ok(()),
        |_| Ok(summary),
    )
    .map_err(Into::into)
}

struct PreflightRevalidation<'a, A> {
    ledger: &'a HeldInput,
    selected_input: &'a HeldInput,
    fixture: &'a HeldInput,
    assets: &'a A,
}

fn publish_revalidated_report<T>(
    revalidation: &PreflightRevalidation<'_, impl RevalidatedManifestAssets>,
    evidence_root: &Path,
    report: &[u8],
    late_prepublication_validation: impl FnOnce() -> io::Result<()>,
    fault: &mut impl FnMut(FaultPoint) -> io::Result<()>,
    success: impl FnOnce(&PublishedOutput) -> io::Result<T>,
) -> io::Result<T> {
    with_complete_preflight_revalidation(revalidation, || {
        late_prepublication_validation()?;
        publish_manifest_preflight_report(evidence_root, report, fault, |published| {
            with_complete_preflight_revalidation(revalidation, || success(published))
        })
    })
}

fn with_complete_preflight_revalidation<T>(
    revalidation: &PreflightRevalidation<'_, impl RevalidatedManifestAssets>,
    action: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    revalidation
        .ledger
        .with_revalidated_path(|ledger_validation| {
            ledger_validation.with_current_namespace(|| {
                revalidation
                    .selected_input
                    .with_revalidated_path(|input_validation| {
                        input_validation.with_current_namespace(|| {
                            revalidation
                                .fixture
                                .with_revalidated_path(|fixture_validation| {
                                    fixture_validation.with_current_namespace(|| {
                                        revalidation.assets.with_revalidated_paths(action)
                                    })
                                })
                        })
                    })
            })
        })
}

fn canonical_report(
    ledger: &EvidenceLedger,
    decoded_fingerprint: &str,
    summary: HarnessSummary,
) -> io::Result<Vec<u8>> {
    let report = ManifestPreflightReport {
        schema: "hanonly-d0-manifest-preflight-v1".into(),
        image_input_contract: "image-input-contract-v1".into(),
        visual_input_sha256: ledger.visual_input_sha256.clone(),
        visual_input_decoded_rgba_blake3: decoded_fingerprint.into(),
        visual_manifest_sha256: ledger.visual_manifest_sha256.clone(),
        source_gate_fixture_manifest_sha256: ledger.source_gate_fixture_manifest_sha256.clone(),
        entries: summary.entries,
        targets: summary.targets,
        masks: summary.masks,
        protected_rois: summary.protected_rois,
        retained_bytes: summary.retained_bytes,
    };
    let mut bytes = serde_json::to_vec(&report)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn repository_root() -> io::Result<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or_else(|| io::Error::other("crate is not nested inside the repository"))?;
    fs::canonicalize(root)
}

fn require_absolute_canonical(path: &Path) -> io::Result<()> {
    let bytes = path.as_os_str().as_bytes();
    require(
        bytes.len() >= 2
            && bytes[0] == b'/'
            && bytes[1] != b'/'
            && !bytes.ends_with(b"/")
            && bytes[1..].split(|byte| *byte == b'/').all(|component| {
                !component.is_empty()
                    && component != b"."
                    && component != b".."
                    && !component.contains(&0)
            }),
        "path must be an absolute canonical path",
    )?;
    require(fs::canonicalize(path)? == path, "path must be canonical")
}

fn effective_owner() -> io::Result<u64> {
    let (socket, _peer) = UnixStream::pair()?;
    let stat = fstat(&socket).map_err(io::Error::from)?;
    Ok(stat.st_uid.into())
}

fn validate_fixture_manifest(bytes: &[u8]) -> io::Result<()> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid_data("fixture manifest must be a top-level object"))?;
    let fixtures = object
        .get("fixtures")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| invalid_data("fixture manifest fixtures must be an array"))?;
    require(
        !fixtures.is_empty(),
        "fixture manifest fixtures must be nonempty",
    )
}

fn require_fixture_clean(repository: &Path) -> io::Result<()> {
    let output = Command::new("git")
        .current_dir(repository)
        .args(["status", "--porcelain=v1", "--"])
        .arg(FIXTURE_RELATIVE_PATH)
        .output()?;
    require_clean_status(output.status.success(), &output.stdout)
}

fn require_clean_status(success: bool, stdout: &[u8]) -> io::Result<()> {
    require(success, "fixture git status command failed")?;
    require(stdout.is_empty(), "fixed fixture is dirty")
}

fn decode_sha256(value: &str) -> io::Result<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_data("sha256 must be 64 lowercase hex characters"));
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
    }
    Ok(decoded)
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("sha256 syntax checked before decoding"),
    }
}

fn require(condition: bool, message: &'static str) -> io::Result<()> {
    condition
        .then_some(())
        .ok_or_else(|| io::Error::other(message))
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[test]
#[ignore = "requires approved external 1+4+4 visual evidence manifest"]
fn han_only_visual_manifest_matrix() {
    let summary = run_manifest_only_preflight().expect("manifest-only visual preflight failed");
    eprintln!(
        "entries={} targets={} masks={} protected_rois={} retained_bytes={}",
        summary.entries,
        summary.targets,
        summary.masks,
        summary.protected_rois,
        summary.retained_bytes
    );
}

#[cfg(all(test, feature = "hanonly-test-evidence"))]
mod source_gate_selection {
    use super::super::d0_r51_holdout_bundle::{
        R51FreezeCommitments as R51BundleFreezeCommitments, R51ValidatedExecutionEntry,
        R51ValidatedExecutionTarget, R51ValidatedExecutionView, R51ValidatedReceiptData,
        validate_r51_plaintext_holdout_bundle,
    };
    use super::super::d0_visual_manifest_oracles::{
        OracleValidatedEntry, OracleValidatedTarget, ValidatedHalfOpenRect,
    };
    use super::super::d0_visual_manifest_schema::{
        Aspect, Background, DimensionBin, Effect, EntryRole, Expected, Position, TranslationLength,
        VisualManifestEntry, VisualManifestTarget, Writing,
    };
    use super::super::engines::source_language_gate::{
        PpCanonicalLineDiagnostic, PpCanonicalOccurrenceDiagnostic, PpDetectorDiagnostic,
        PpRecognitionDiagnostic, SourceGateCropPolicy, SourceGateCropPolicyGuard,
        SourceGateDecision, SourceGateDetectorAssignmentDiagnostic,
        SourceGateDetectorOwnershipDiagnostic, SourceGateDiagnosticCapture,
        SourceGateDiagnosticEvent, SourceGateRejectReason, SourceGateTargetGeometryDiagnostic,
        dispatch_source_gate, rgba_fingerprint,
    };
    use super::super::engines::support::{
        SOURCE_GATE_TARGET_DETECTOR, eligible_text_lines, line_support_mask,
    };
    use super::*;
    use chrono::{SecondsFormat, Utc};
    use image::{DynamicImage, RgbaImage};
    use koharu_core::{Node, NodeId, NodeKind, Page, Scene, TextData, Transform};
    use koharu_llm::NativeLogCaptureGuard;
    use koharu_llm::paddleocr_vl::{PaddleOcrVl, PaddleOcrVlTask};
    use koharu_llm::safe::{LlamaBackendDeviceType, list_llama_ggml_backend_devices};
    use koharu_ml::pp_ocr_v5::PpOcrV5;
    use koharu_runtime::{ComputePolicy, RuntimeManager, default_app_data_root};
    use rustix::fs::{
        AtFlags, Dir, FileType, Mode, OFlags, fstat, fsync, linkat, mkdirat, open, openat, statat,
        unlinkat,
    };
    use sha2::{Digest, Sha256};
    use std::cell::OnceCell;
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::ffi::{OsStr, OsString};
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    const PHASE_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_PHASE";
    const R51_FORMAL_CUSTODY_ENV: &str = "HANONLY_R51_FORMAL_CUSTODY";
    const R51_CUSTODY_DIRECTORY_ENV: &str = "HANONLY_R51_CUSTODY_DIRECTORY";
    const R51_PLAINTEXT_DIRECTORY_ENV: &str = "HANONLY_R51_PLAINTEXT_DIRECTORY";
    const R51_PLAINTEXT_ARCHIVE_ENV: &str = "HANONLY_R51_PLAINTEXT_ARCHIVE";
    const R51_CALIBRATION_MANIFEST_SHA256_ENV: &str = "HANONLY_R51_CALIBRATION_MANIFEST_SHA256";
    const R51_OPEN_MARKER_SHA256_ENV: &str = "HANONLY_R51_OPEN_MARKER_SHA256";
    const R51_COMPLETION_SUMMARY_STDOUT_PREFIX: &str = "HANONLY_R51_COMPLETION_SUMMARY=";
    const B0_SHA_ENV: &str = "HANONLY_B0_SHA";
    const ARTIFACT_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_ARTIFACT";
    const REPORT_DIR_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_REPORT_DIR";
    const REQUIRED_CHECK_ENV: &str = "HANONLY_SOURCE_GATE_REQUIRED_CHECK_ATTESTATION";
    const ARTIFACT_VERSION: u32 = 2;
    const PLAN_REVISION: u32 = 51;
    const B0_DEFAULT_GPU_LAYERS: u32 = 1000;
    const REQUIRED_CHECK_COMMAND: &str =
        "bun scripts/check-hanonly-production-policy.ts --b0-source-gate-anti-fixture";
    const CHECKER_ENDPOINT: &str = "scripts/check-hanonly-production-policy.ts";
    const PPOCR_PREPROCESSING_PREIMAGE: &str = "{\"contract\":\"hanonly-b0-ppocr-crop-local-preprocessing-v1\",\"operations\":[\"decode-crop-rgba\",\"isotropic-upscale-if-short-side-below-64\",\"detect-and-recognize-in-upscaled-crop-space\"]}";
    const INVERSE_MAPPING_PREIMAGE: &str = "{\"contract\":\"hanonly-b0-inverse-mapping-v1\",\"operations\":[\"divide-upscaled-word-box-coordinates-by-inference-scale\",\"preserve-half-open-crop-local-geometry\",\"translate-by-source-crop-origin\"]}";
    const COVERAGE_ACCEPTANCE_PREIMAGE: &str = "{\"contract\":\"hanonly-b0-coverage-acceptance-v2\",\"requirements\":[\"no-rejected-after-vl\",\"no-pp-vl-incomplete-coverage\",\"every-oracle-source-ink-pixel-covered-by-selected-downstream-support\"]}";
    const SOURCE_REMOVAL_PREFLIGHT_PREIMAGE: &str = "{\"contract\":\"hanonly-b0-source-removal-preflight-v1\",\"requirements\":[\"target-recall-equals-one\",\"protected-false-positive-count-equals-zero\",\"rotation-targets-excluded\",\"unmatched-selected-node-count-equals-zero\",\"coverage-acceptance-passes\"]}";
    const ANTI_FIXTURE_SCANNED_ROOTS: &[&str] = &[
        "crates/koharu-app/src/pipeline/engines/source_language_gate.rs",
        "crates/koharu-ml/src/pp_ocr_v5.rs",
        "crates/koharu-llm/src/paddleocr_vl.rs",
        "crates/koharu-app/src/pipeline/mod.rs",
        "scripts/check-hanonly-production-policy.ts",
        "scripts/check-hanonly-production-policy.test.ts",
        "scripts/hanonly_evidence_ledger.py",
        "scripts/hanonly_evidence_ledger_test.py",
    ];
    const ANTI_FIXTURE_ALLOWED_DESCRIPTOR_ROOTS: &[&str] = &[
        "crates/koharu-app/src/pipeline/mod.rs",
        "scripts/check-hanonly-production-policy.ts",
        "scripts/check-hanonly-production-policy.test.ts",
        "scripts/hanonly_evidence_ledger.py",
        "scripts/hanonly_evidence_ledger_test.py",
    ];
    const SOURCE_COLOR_CONTRACT_SHA256: &str =
        "13d2256fed7b8189e67db7222ce6ce7964f2745c977c42e7693679ffb2a341f8";
    const COLOR_CONSTANT_SET_SHA256: &str =
        "ea277ff2674aae711b62a39b6a0b930e7d9c863bd518521c59ff44be56c4c6e9";
    const PP_REPO: &str = "marsena/paddleocr-onnx-models";
    const PP_DETECTION_MODEL: &str = "PP-OCRv5_server_det_infer.onnx";
    const PP_RECOGNITION_MODEL: &str = "PP-OCRv5_server_rec_infer.onnx";
    const PP_RECOGNITION_CONFIG: &str = "PP-OCRv5_server_rec_infer.yml";
    const VL_MAX_NEW_TOKENS: usize = 256;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Phase {
        CalibrationFreeze,
        Holdout,
    }

    #[derive(Deserialize)]
    struct SelectionManifest {
        entries: Vec<SelectionManifestEntry>,
    }

    #[derive(Deserialize)]
    struct SelectionManifestEntry {
        id: String,
        role: EntryRole,
    }

    struct SelectionEnvironment {
        phase: Phase,
        r51_formal_custody: Option<R51FormalCustody>,
        b0_sha: String,
        visual_input: PathBuf,
        visual_input_sha256: String,
        visual_manifest: PathBuf,
        visual_manifest_sha256: String,
        calibration_manifest_sha256: String,
        evidence_root: PathBuf,
        report_dir: PathBuf,
        source_gate_fixture_manifest_sha256: String,
        artifact: PathBuf,
        calibration_entry_ids: Vec<String>,
        holdout_entry_ids: Vec<String>,
        required_check: RequiredCheck,
        required_check_attestation: HeldInput,
    }

    struct R51FormalCustody {
        contract_sha256: String,
        holdout: Option<R51HoldoutCustody>,
    }

    struct R51HoldoutCustody {
        directory: PathBuf,
        plaintext_directory: PathBuf,
        plaintext_archive: PathBuf,
        freeze: R51FreezeCommitments,
        expected_open_marker_sha256: String,
        open_marker: OnceCell<PublishedArtifact>,
    }

    struct R51FreezeCommitments {
        receipt_sha256: String,
        ciphertext_sha256: String,
        plaintext_archive_sha256: String,
        manifest_sha256: String,
        oracle_sha256: String,
        hashes_sha256: String,
    }

    impl SelectionEnvironment {
        fn parse(mut get: impl FnMut(&str) -> Option<String>) -> io::Result<Self> {
            let phase = match required(&mut get, PHASE_ENV)?.as_str() {
                "calibration-freeze" => Phase::CalibrationFreeze,
                "holdout" => Phase::Holdout,
                _ => return Err(invalid_data("invalid Source Gate selection phase")),
            };
            let r51_formal_custody = match get(R51_FORMAL_CUSTODY_ENV).as_deref() {
                None | Some("0") => None,
                Some("1") => {
                    let contract_path =
                        repository_root()?.join(".omx/plans/hanonly-r51-b0-custody-contract.json");
                    let contract_sha256 = sha256_file(&contract_path)?;
                    let holdout = if phase == Phase::Holdout {
                        let directory =
                            PathBuf::from(required(&mut get, R51_CUSTODY_DIRECTORY_ENV)?);
                        let plaintext_directory =
                            PathBuf::from(required(&mut get, R51_PLAINTEXT_DIRECTORY_ENV)?);
                        let plaintext_archive =
                            PathBuf::from(required(&mut get, R51_PLAINTEXT_ARCHIVE_ENV)?);
                        require_absolute_canonical(&directory)?;
                        require_absolute_canonical(&plaintext_directory)?;
                        require_absolute_canonical(&plaintext_archive)?;
                        Some(R51HoldoutCustody {
                            freeze: load_r51_freeze_commitments(&directory)?,
                            directory,
                            plaintext_directory,
                            plaintext_archive,
                            expected_open_marker_sha256: required_hash(
                                &mut get,
                                R51_OPEN_MARKER_SHA256_ENV,
                            )?,
                            open_marker: OnceCell::new(),
                        })
                    } else {
                        None
                    };
                    Some(R51FormalCustody {
                        contract_sha256,
                        holdout,
                    })
                }
                Some(_) => return Err(invalid_data("invalid R51 formal custody mode")),
            };
            let b0_sha = required(&mut get, B0_SHA_ENV)?;
            require(
                b0_sha.len() == 40
                    && b0_sha
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "B0 sha must be 40 lowercase hex characters",
            )?;
            let formal_holdout = r51_formal_custody
                .as_ref()
                .and_then(|custody| custody.holdout.as_ref());
            let visual_manifest_sha256 = match formal_holdout {
                Some(holdout) => holdout.freeze.manifest_sha256.clone(),
                None => {
                    let value = required(&mut get, VISUAL_MANIFEST_SHA256_ENV)?;
                    decode_sha256(&value)?;
                    value
                }
            };
            let calibration_manifest_sha256 = if r51_formal_custody.is_some() {
                let value = required_hash(&mut get, R51_CALIBRATION_MANIFEST_SHA256_ENV)?;
                if phase == Phase::CalibrationFreeze {
                    require(
                        value == visual_manifest_sha256,
                        "R51 calibration manifest commitment drift",
                    )?;
                }
                value
            } else {
                visual_manifest_sha256.clone()
            };
            let (
                visual_input,
                visual_input_sha256,
                visual_manifest,
                calibration_entry_ids,
                holdout_entry_ids,
                phase_partition_valid,
            ) = if let Some(holdout) = formal_holdout {
                (
                    holdout.plaintext_archive.clone(),
                    holdout.freeze.plaintext_archive_sha256.clone(),
                    holdout.plaintext_directory.join("manifest.json"),
                    Vec::new(),
                    r51_entry_ids('h'),
                    true,
                )
            } else {
                let visual_input = PathBuf::from(required(&mut get, VISUAL_INPUT_ENV)?);
                require_absolute_syntax(&visual_input)?;
                let visual_input_sha256 = required(&mut get, VISUAL_INPUT_SHA256_ENV)?;
                decode_sha256(&visual_input_sha256)?;
                let visual_manifest = PathBuf::from(required(&mut get, VISUAL_MANIFEST_ENV)?);
                require_absolute_canonical(&visual_manifest)?;
                let manifest_bytes = fs::read(&visual_manifest)?;
                require(
                    sha256_hex(&manifest_bytes) == visual_manifest_sha256,
                    "visual manifest sha256 drift",
                )?;
                let manifest: SelectionManifest = serde_json::from_slice(&manifest_bytes)
                    .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
                let calibration_entry_ids = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.role == EntryRole::Calibration)
                    .map(|entry| entry.id.clone())
                    .collect::<Vec<_>>();
                let holdout_entry_ids = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.role == EntryRole::Holdout)
                    .map(|entry| entry.id.clone())
                    .collect::<Vec<_>>();
                let phase_partition_valid = if r51_formal_custody.is_some() {
                    manifest.entries.len() == 4
                        && calibration_entry_ids == r51_entry_ids('c')
                        && holdout_entry_ids.is_empty()
                } else {
                    match phase {
                        Phase::CalibrationFreeze => {
                            calibration_entry_ids.len() == 4
                                && matches!(holdout_entry_ids.len(), 0 | 4)
                        }
                        Phase::Holdout => {
                            holdout_entry_ids.len() == 4
                                && matches!(calibration_entry_ids.len(), 0 | 4)
                        }
                    }
                };
                (
                    visual_input,
                    visual_input_sha256,
                    visual_manifest,
                    calibration_entry_ids,
                    holdout_entry_ids,
                    phase_partition_valid,
                )
            };
            require(
                phase_partition_valid
                    && calibration_entry_ids
                        .iter()
                        .all(|id| !holdout_entry_ids.contains(id)),
                "visual manifest calibration/holdout partition drift",
            )?;
            let source_gate_fixture_manifest_sha256 =
                required(&mut get, SOURCE_GATE_FIXTURE_SHA256_ENV)?;
            decode_sha256(&source_gate_fixture_manifest_sha256)?;
            let evidence_root = PathBuf::from(required(&mut get, VISUAL_EVIDENCE_ROOT_ENV)?);
            require_absolute_canonical(&evidence_root)?;
            let artifact = PathBuf::from(required(&mut get, ARTIFACT_ENV)?);
            let report_dir = PathBuf::from(required(&mut get, REPORT_DIR_ENV)?);
            require_future_path_below(&evidence_root, &artifact)?;
            require_future_path_below(&evidence_root, &report_dir)?;
            let artifact_parent = artifact
                .parent()
                .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
            require(
                report_dir.starts_with(artifact_parent),
                "selection report directory must be below the artifact parent",
            )?;
            if report_dir.exists() {
                require(
                    report_dir.is_dir(),
                    "selection report path must be a directory",
                )?;
            }
            let required_check_path = PathBuf::from(required(&mut get, REQUIRED_CHECK_ENV)?);
            let required_check_manifest_sha256 = r51_formal_custody
                .as_ref()
                .and_then(|custody| custody.holdout.as_ref())
                .map_or(visual_manifest_sha256.as_str(), |holdout| {
                    holdout.freeze.manifest_sha256.as_str()
                });
            let (required_check, required_check_attestation) = load_required_check(
                &evidence_root,
                &required_check_path,
                phase,
                &b0_sha,
                required_check_manifest_sha256,
                &source_gate_fixture_manifest_sha256,
            )?;
            Ok(Self {
                phase,
                r51_formal_custody,
                b0_sha,
                visual_input,
                visual_input_sha256,
                visual_manifest,
                visual_manifest_sha256,
                calibration_manifest_sha256,
                evidence_root,
                report_dir,
                source_gate_fixture_manifest_sha256,
                artifact,
                calibration_entry_ids,
                holdout_entry_ids,
                required_check,
                required_check_attestation,
            })
        }
    }

    #[derive(Debug, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct RequiredCheckAttestation {
        version: u32,
        mode: String,
        phase: String,
        b0_sha: String,
        manifest_sha256: String,
        source_gate_fixture_manifest_sha256: String,
        checker_endpoint_sha256: String,
        scanned_roots: Vec<String>,
        allowed_descriptor_roots: Vec<String>,
        policy_scan_sha256: String,
        result: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct RequiredCheck {
        phase: String,
        command: String,
        checker_endpoint_sha256: String,
        manifest_sha256: String,
        source_gate_fixture_manifest_sha256: String,
        attestation_relpath: String,
        attestation_sha256: String,
        b0_sha: String,
        result: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct FrozenRecallContract {
        candidate_set: Vec<String>,
        selected_candidate_id: String,
        ppocr_crop_local_preprocessing_sha256: String,
        inverse_mapping_rule_sha256: String,
        coverage_acceptance_rule_sha256: String,
        source_removal_preflight_rule_sha256: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Candidate {
        id: String,
        short_side_numerator: u32,
        short_side_denominator: u32,
        long_side_numerator: u32,
        long_side_denominator: u32,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct ModelArtifactHashes {
        pp_detection: String,
        pp_recognition: String,
        pp_recognition_config: String,
        vl_model: String,
        vl_mmproj: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct EnumeratedDevice {
        index: u32,
        name: String,
        description: String,
        backend: String,
        device_type: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct LoadedModelDevice {
        model_device_ordinal: u32,
        name: String,
        backend: String,
        device_type: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct LoadEvidence {
        cpu_forced: bool,
        gpu_offload_supported: bool,
        n_gpu_layers: u32,
        mtmd_use_gpu: bool,
        word_boxes_backend: String,
        raw_load_log_relpath: String,
        raw_load_log_sha256: String,
        enumerated_devices: Vec<EnumeratedDevice>,
        loaded_model_devices: Vec<LoadedModelDevice>,
        offloaded_layers: u32,
        offloadable_layers: u32,
        model_buffer_bytes_by_backend: BTreeMap<String, u64>,
        mtmd_backend: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct ProcessEvidence {
        id: String,
        phase: String,
        requested_device: String,
        paddle_instance_id: String,
        executable_sha256: String,
        model_artifact_sha256: ModelArtifactHashes,
        runtime_library_sha256: BTreeMap<String, String>,
        load_evidence: LoadEvidence,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct ExecutionEvidence {
        paddle_instance_id: String,
        context_offload_kqv: bool,
        context_op_offload: bool,
        inference_completed: bool,
        raw_inference_log_relpath: String,
        raw_inference_log_sha256: String,
        source_gate_diagnostic_relpath: String,
        source_gate_diagnostic_sha256: String,
        context_buffer_bytes_by_backend: BTreeMap<String, u64>,
        compute_buffer_bytes_by_backend: BTreeMap<String, u64>,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct RuntimeNode {
        node_id: String,
        recognition_anchor: [f64; 4],
        node_rotation: f64,
        text_rotation: f64,
        selected_as_han: bool,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct SourceCoveragePreflight {
        pp_han_scalar_count: usize,
        vl_expected_han_scalar_count: usize,
        pp_vl_complete_coverage: bool,
        rejected_after_vl: bool,
        pp_vl_incomplete_coverage: bool,
        covered_source_roi_ids: Vec<String>,
        source_text_roi_coverage: f64,
        source_removal_preflight_passed: bool,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct DerivedEvidence {
        actual_device: String,
        matched_target_ids: Vec<String>,
        selected_target_ids: Vec<String>,
        selected_protected_node_ids: Vec<String>,
        selected_rotation_target_ids: Vec<String>,
        unmatched_selected_node_ids: Vec<String>,
        target_recall: f64,
        protected_false_positive_count: u32,
        rotation_targets_excluded: bool,
        source_coverage_preflight: SourceCoveragePreflight,
        passed: bool,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct SelectionResult {
        entry_id: String,
        process_evidence_id: String,
        candidate_id: String,
        execution_evidence: ExecutionEvidence,
        runtime_nodes: Vec<RuntimeNode>,
        derived: DerivedEvidence,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct RunnerEvidence {
        selected_candidate_id: String,
        process_evidence: Vec<ProcessEvidence>,
        results: Vec<SelectionResult>,
        r51_formal: Option<R51FormalRunEvidence>,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct R51FormalRunEvidence {
        bundle_validation_receipt: Option<PublishedArtifact>,
        cells: Vec<R51TerminalCellResult>,
        first_failed_cell: Option<String>,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct PublishedArtifact {
        path: String,
        sha256: String,
        byte_length: u64,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct R51TargetRecall {
        target_total: usize,
        selected: usize,
        covered: usize,
        uncovered: usize,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct R51TerminalCellResult {
        cell_key: String,
        result: String,
        selection_result: Option<String>,
        target_recall: R51TargetRecall,
        pp_han_count: usize,
        vl_han_count: usize,
        rejection_reason: Option<String>,
        device_evidence_sha256: String,
        log_sha256: String,
        diagnostic_sha256: String,
        target_coverage_index_sha256: Option<String>,
        #[serde(skip)]
        diagnostic_cell_key: String,
        #[serde(skip)]
        phase: String,
        #[serde(skip)]
        candidate_id: String,
        #[serde(skip)]
        entry_id: String,
        #[serde(skip)]
        device: String,
        #[serde(skip)]
        terminal_reason: Option<String>,
        #[serde(skip)]
        diagnostic_path: String,
        #[serde(skip)]
        diagnostic_byte_length: u64,
        #[serde(skip)]
        target_coverage_index_path: Option<String>,
        #[serde(skip)]
        target_coverage_index_byte_length: Option<u64>,
        #[serde(skip)]
        device_evidence_path: String,
        #[serde(skip)]
        device_evidence_byte_length: u64,
        #[serde(skip)]
        log_path: String,
        #[serde(skip)]
        log_byte_length: u64,
    }

    struct CellSupportEvidence {
        width: u32,
        height: u32,
        selected_by_target: BTreeMap<String, Vec<u8>>,
        downstream_by_target: BTreeMap<String, Vec<u8>>,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct R51BundleValidationReceipt<'a> {
        contract: &'static str,
        plan_revision: u32,
        b0_sha: &'a str,
        test_executable_sha256: &'a str,
        enabled_cargo_features: [&'static str; 1],
        r51_contract_sha256: &'a str,
        freeze_receipt_sha256: &'a str,
        plaintext_archive_sha256: &'a str,
        manifest_sha256: &'a str,
        oracle_sha256: &'a str,
        hashes_sha256: &'a str,
        schema_validation_pass: bool,
        asset_binding_pass: bool,
        mask_source_clean_equality_pass: bool,
        oracle_semantics_pass: bool,
        result: &'static str,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct R51FreezeReceipt {
        contract: String,
        plan_revision: u32,
        base_b0_sha: String,
        implementation_thread_id: String,
        frozen_before_production_edit: bool,
        entry_ids: Vec<String>,
        cipher: String,
        integrity: String,
        iv_sha256: String,
        ciphertext_byte_length: u64,
        ciphertext_sha256: String,
        header_sha256: String,
        hmac_sha256: String,
        plaintext_archive_sha256_commitment: String,
        manifest_sha256_commitment: String,
        oracle_sha256_commitment: String,
        hashes_sha256_commitment: String,
        historical_inventory_sha256: String,
        formal_source_identities: Vec<serde_json::Value>,
        disclosed_challenge_exclusion_pass: bool,
        result: String,
    }

    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct R51OpenMarker {
        contract: String,
        plan_revision: u32,
        b0_sha: String,
        selected_candidate_id: String,
        freeze_receipt_sha256: String,
        ciphertext_sha256: String,
        pre_holdout_attestation_sha256: String,
        nonce_hex: String,
        result: String,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct R51TargetCoverageProof<'a> {
        contract: &'static str,
        plan_revision: u32,
        b0_sha: &'a str,
        cell_key: &'a str,
        entry_id: &'a str,
        target_id: &'a str,
        oracle_mask_raw_sha256: String,
        oracle_mask_normalized_sha256: String,
        page_width: u32,
        page_height: u32,
        support_stride_bytes: u32,
        selected_support_relpath: String,
        selected_support_byte_length: u64,
        selected_support_sha256: String,
        downstream_support_relpath: String,
        downstream_support_byte_length: u64,
        downstream_support_sha256: String,
        oracle_foreground_pixels: u64,
        selected_support_foreground_pixels: u64,
        downstream_support_foreground_pixels: u64,
        selected_covered_pixels: u64,
        downstream_covered_pixels: u64,
        missing_selected_pixels: u64,
        missing_downstream_pixels: u64,
        protected_overlap_pixels: u64,
        target_selected: bool,
        result: &'static str,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct R51TargetCoverageIndex<'a> {
        contract: &'static str,
        plan_revision: u32,
        b0_sha: &'a str,
        cell_key: &'a str,
        manifest_sha256: &'a str,
        oracle_sha256: &'a str,
        hashes_sha256: &'a str,
        records: Vec<R51TargetCoverageIndexRecord>,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct R51TargetCoverageIndexRecord {
        entry_id: String,
        target_id: String,
        proof_path: String,
        proof_sha256: String,
        proof_byte_length: u64,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct R51CompletionSummary<'a> {
        contract: &'static str,
        plan_revision: u32,
        b0_sha: &'a str,
        selected_candidate_id: &'a str,
        freeze_receipt_sha256: &'a str,
        open_marker_sha256: &'a str,
        ciphertext_sha256: &'a str,
        pre_holdout_attestation_sha256: &'a str,
        holdout_manifest_sha256: &'a str,
        bundle_validation_receipt_path: &'a str,
        bundle_validation_receipt_sha256: &'a str,
        bundle_validation_receipt_byte_length: u64,
        terminal_diagnostic_index_path: &'a str,
        terminal_diagnostic_index_sha256: &'a str,
        terminal_diagnostic_index_byte_length: u64,
        cell_results: &'a [R51TerminalCellResult],
        first_failed_cell: Option<&'a str>,
        unexecuted_cell_keys: Vec<String>,
        all_cells_terminated: bool,
        all_cells_passed: bool,
        failure_kind: Option<&'static str>,
        result: &'static str,
    }

    #[derive(Serialize)]
    struct CalibrationFailureDiagnostic<'a> {
        schema: &'static str,
        b0_sha: &'a str,
        manifest_sha256: &'a str,
        source_gate_fixture_manifest_sha256: &'a str,
        failure: &'a str,
        candidates: Vec<Candidate>,
        calibration_entry_ids: &'a [String],
        process_evidence: &'a [ProcessEvidence],
        calibration_results: &'a [SelectionResult],
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ParsedLoadLog {
        offloaded_layers: u32,
        offloadable_layers: u32,
        model_buffer_bytes_by_backend: BTreeMap<String, u64>,
        mtmd_backend: String,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ParsedInferenceLog {
        context_buffer_bytes_by_backend: BTreeMap<String, u64>,
        compute_buffer_bytes_by_backend: BTreeMap<String, u64>,
    }

    fn parse_native_load_log(bytes: &[u8]) -> io::Result<ParsedLoadLog> {
        let text = std::str::from_utf8(bytes)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        let mut offloaded = None;
        let mut model_buffers = BTreeMap::new();
        let mut mtmd_backend = None;
        for line in text.lines() {
            if let Some((loaded, total)) = parse_offloaded_layers(line) {
                offloaded = Some((loaded, total));
            }
            accumulate_buffer_line(line, "model buffer size", &mut model_buffers)?;
            if line.contains("CLIP using") && line.contains("backend") {
                mtmd_backend = canonical_backend(line).map(str::to_owned);
            }
        }
        let (offloaded_layers, offloadable_layers) =
            offloaded.ok_or_else(|| invalid_data("native load log omitted offloaded layers"))?;
        require(
            !model_buffers.is_empty(),
            "native load log omitted model buffers",
        )?;
        Ok(ParsedLoadLog {
            offloaded_layers,
            offloadable_layers,
            model_buffer_bytes_by_backend: model_buffers,
            mtmd_backend: mtmd_backend
                .ok_or_else(|| invalid_data("native load log omitted MTMD backend"))?,
        })
    }

    fn parse_native_inference_log(bytes: &[u8]) -> io::Result<ParsedInferenceLog> {
        let text = std::str::from_utf8(bytes)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        let mut context_buffers = BTreeMap::new();
        let mut compute_buffers = BTreeMap::new();
        for line in text.lines() {
            accumulate_buffer_line(line, "output buffer size", &mut context_buffers)?;
            accumulate_buffer_line(line, "KV buffer size", &mut context_buffers)?;
            accumulate_buffer_line(line, "compute buffer size", &mut compute_buffers)?;
        }
        require(
            !context_buffers.is_empty() && !compute_buffers.is_empty(),
            "native inference log omitted context or compute buffers",
        )?;
        Ok(ParsedInferenceLog {
            context_buffer_bytes_by_backend: context_buffers,
            compute_buffer_bytes_by_backend: compute_buffers,
        })
    }

    fn parse_offloaded_layers(line: &str) -> Option<(u32, u32)> {
        let suffix = line.split_once("offloaded ")?.1;
        let ratio = suffix.split_whitespace().next()?;
        let (loaded, total) = ratio.split_once('/')?;
        Some((loaded.parse().ok()?, total.parse().ok()?))
    }

    fn accumulate_buffer_line(
        line: &str,
        marker: &str,
        buffers: &mut BTreeMap<String, u64>,
    ) -> io::Result<()> {
        if !line.contains(marker) {
            return Ok(());
        }
        let Some(backend) = canonical_backend(line) else {
            return Err(invalid_data("native buffer log used an unknown backend"));
        };
        let value = line
            .split_once('=')
            .and_then(|(_, suffix)| suffix.split_whitespace().next())
            .map(str::parse::<f64>)
            .transpose()
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        let Some(value) = value else {
            return Ok(());
        };
        require(
            value.is_finite() && value >= 0.0,
            "native buffer size is invalid",
        )?;
        let bytes = (value * 1024.0 * 1024.0).round() as u64;
        *buffers.entry(backend.into()).or_default() += bytes;
        Ok(())
    }

    fn canonical_backend(line: &str) -> Option<&'static str> {
        if line.contains("MTL") || line.contains("Metal") {
            Some("Metal")
        } else if line.contains("CPU") {
            Some("CPU")
        } else {
            None
        }
    }

    #[derive(Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct FrozenArtifact {
        version: u32,
        plan_revision: u32,
        b0_sha: String,
        manifest_sha256: String,
        holdout_manifest_sha256: Option<String>,
        source_gate_fixture_manifest_sha256: String,
        image_input_contract_sha256: String,
        source_color_contract_sha256: String,
        color_constant_set_sha256: String,
        requested_devices: Vec<String>,
        enabled_cargo_features: Vec<String>,
        backend_evidence_parser_version: u32,
        required_checks: Vec<RequiredCheck>,
        frozen_recall_contract: FrozenRecallContract,
        candidates: Vec<Candidate>,
        calibration_entry_ids: Vec<String>,
        holdout_entry_ids: Vec<String>,
        process_evidence: Vec<ProcessEvidence>,
        calibration_results: Vec<SelectionResult>,
        selected_candidate_id: String,
        frozen_at_utc: String,
        frozen_payload_sha256: String,
        holdout_results: Vec<SelectionResult>,
        holdout_completed_at_utc: Option<String>,
        retuned_after_freeze: bool,
    }

    fn required(
        get: &mut impl FnMut(&str) -> Option<String>,
        name: &'static str,
    ) -> io::Result<String> {
        get(name).ok_or_else(|| invalid_data("missing Source Gate selection environment"))
    }

    fn required_hash(
        get: &mut impl FnMut(&str) -> Option<String>,
        name: &'static str,
    ) -> io::Result<String> {
        let value = required(get, name)?;
        decode_sha256(&value)?;
        Ok(value)
    }

    fn r51_entry_ids(kind: char) -> Vec<String> {
        (1..=4)
            .map(|index| format!("r51-{kind}{index:02}"))
            .collect()
    }

    fn require_future_path_below(root: &Path, path: &Path) -> io::Result<()> {
        require_absolute_syntax(path)?;
        require(path != root, "selection path must not be the evidence root")?;
        require(
            path.starts_with(root),
            "selection path must remain below the evidence root",
        )?;
        let existing = path
            .ancestors()
            .find(|ancestor| ancestor.exists())
            .ok_or_else(|| invalid_data("selection path has no existing ancestor"))?;
        require(
            fs::canonicalize(existing)? == existing,
            "selection path must be canonical-ish",
        )
    }

    fn require_absolute_syntax(path: &Path) -> io::Result<()> {
        let bytes = path.as_os_str().as_bytes();
        require(
            bytes.len() >= 2
                && bytes[0] == b'/'
                && bytes[1] != b'/'
                && !bytes.ends_with(b"/")
                && bytes[1..].split(|byte| *byte == b'/').all(|component| {
                    !component.is_empty()
                        && component != b"."
                        && component != b".."
                        && !component.contains(&0)
                }),
            "path must be absolute and canonical-ish",
        )
    }

    fn required_check_phase(phase: Phase) -> &'static str {
        match phase {
            Phase::CalibrationFreeze => "pre-calibration",
            Phase::Holdout => "pre-holdout",
        }
    }

    fn required_check_relpath(phase: Phase) -> String {
        format!(
            "source-gate-selection/checks/{}.json",
            required_check_phase(phase)
        )
    }

    fn load_required_check(
        evidence_root: &Path,
        path: &Path,
        phase: Phase,
        b0_sha: &str,
        manifest_sha256: &str,
        source_gate_fixture_manifest_sha256: &str,
    ) -> io::Result<(RequiredCheck, HeldInput)> {
        require_absolute_syntax(path)?;
        require(
            path.starts_with(evidence_root),
            "required-check attestation must remain below the evidence root",
        )?;
        let relpath = path
            .strip_prefix(evidence_root)
            .ok()
            .and_then(Path::to_str)
            .ok_or_else(|| invalid_data("required-check attestation path is invalid"))?;
        require(
            relpath == required_check_relpath(phase),
            "required-check attestation phase path drift",
        )?;
        let held = HeldInput::open(path)?;
        held.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
        let attestation: RequiredCheckAttestation = serde_json::from_slice(held.bytes())
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        require(
            canonical_json(&attestation)? == held.bytes(),
            "required-check attestation must be canonical JSON",
        )?;
        require(
            attestation.version == 1
                && attestation.mode == "b0-source-gate-anti-fixture"
                && attestation.phase == required_check_phase(phase)
                && attestation.b0_sha == b0_sha
                && attestation.manifest_sha256 == manifest_sha256
                && attestation.source_gate_fixture_manifest_sha256
                    == source_gate_fixture_manifest_sha256
                && attestation.scanned_roots
                    == ANTI_FIXTURE_SCANNED_ROOTS
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect::<Vec<_>>()
                && attestation.allowed_descriptor_roots
                    == ANTI_FIXTURE_ALLOWED_DESCRIPTOR_ROOTS
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect::<Vec<_>>()
                && attestation.result == "pass",
            "required-check attestation drift",
        )?;
        decode_sha256(&attestation.checker_endpoint_sha256)?;
        decode_sha256(&attestation.policy_scan_sha256)?;
        let required_check = RequiredCheck {
            phase: attestation.phase,
            command: REQUIRED_CHECK_COMMAND.into(),
            checker_endpoint_sha256: attestation.checker_endpoint_sha256,
            manifest_sha256: attestation.manifest_sha256,
            source_gate_fixture_manifest_sha256: attestation.source_gate_fixture_manifest_sha256,
            attestation_relpath: relpath.into(),
            attestation_sha256: held
                .sha256()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            b0_sha: attestation.b0_sha,
            result: attestation.result,
        };
        Ok((required_check, held))
    }

    fn git_head(repository: &Path) -> io::Result<String> {
        let output = Command::new("git")
            .current_dir(repository)
            .args(["rev-parse", "HEAD"])
            .output()?;
        require(output.status.success(), "git rev-parse HEAD failed")?;
        String::from_utf8(output.stdout)
            .map(|head| head.trim().to_owned())
            .map_err(|_| invalid_data("git HEAD must be utf-8"))
    }

    fn run_with(
        get: impl FnMut(&str) -> Option<String>,
        repository: &Path,
        read_head: impl FnOnce(&Path) -> io::Result<String>,
        check_fixture: impl FnOnce(&Path) -> io::Result<()>,
        model_runner: impl FnOnce(&SelectionEnvironment) -> io::Result<RunnerEvidence>,
    ) -> io::Result<()> {
        let environment = SelectionEnvironment::parse(get)?;
        require(
            read_head(repository)? == environment.b0_sha,
            "B0 HEAD drift detected",
        )?;
        require(
            sha256_file(&repository.join(CHECKER_ENDPOINT))?
                == environment.required_check.checker_endpoint_sha256,
            "required-check checker endpoint drift",
        )?;
        environment
            .required_check_attestation
            .with_revalidated_path(|_| Ok(()))?;
        match environment.phase {
            Phase::CalibrationFreeze => require(
                !environment.artifact.exists(),
                "calibration-freeze selection artifact already exists",
            )?,
            Phase::Holdout => require(
                environment.artifact.is_file(),
                "holdout selection artifact must be an existing regular file",
            )?,
        }
        check_fixture(repository)?;
        match environment.phase {
            Phase::CalibrationFreeze => {
                let evidence = model_runner(&environment)?;
                let selected_candidate_id = select_or_write_calibration_diagnostic(
                    &environment,
                    &evidence.process_evidence,
                    &evidence.results,
                )?;
                require(
                    evidence.selected_candidate_id == selected_candidate_id,
                    "runner selected candidate does not match independent selection",
                )?;
                if environment.r51_formal_custody.is_some() {
                    let formal = evidence
                        .r51_formal
                        .as_ref()
                        .ok_or_else(|| invalid_data("R51 calibration evidence is missing"))?;
                    write_r51_calibration_diagnostic_generations(&environment, formal)?;
                }
                let mut artifact = FrozenArtifact {
                    version: ARTIFACT_VERSION,
                    plan_revision: PLAN_REVISION,
                    b0_sha: environment.b0_sha.clone(),
                    manifest_sha256: environment.visual_manifest_sha256.clone(),
                    holdout_manifest_sha256: None,
                    source_gate_fixture_manifest_sha256: environment
                        .source_gate_fixture_manifest_sha256
                        .clone(),
                    image_input_contract_sha256:
                        super::super::d0_revision_46_contract::image_input_contract_sha256(),
                    source_color_contract_sha256: SOURCE_COLOR_CONTRACT_SHA256.into(),
                    color_constant_set_sha256: COLOR_CONSTANT_SET_SHA256.into(),
                    requested_devices: vec!["cpu".into(), "metal".into()],
                    enabled_cargo_features: vec!["hanonly-test-evidence".into()],
                    backend_evidence_parser_version: 1,
                    required_checks: vec![environment.required_check.clone()],
                    frozen_recall_contract: frozen_recall_contract(&selected_candidate_id),
                    candidates: candidates_schema(),
                    calibration_entry_ids: environment.calibration_entry_ids.clone(),
                    holdout_entry_ids: frozen_holdout_entry_ids(&environment),
                    process_evidence: evidence.process_evidence,
                    calibration_results: evidence.results,
                    selected_candidate_id,
                    frozen_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                    frozen_payload_sha256: String::new(),
                    holdout_results: Vec::new(),
                    holdout_completed_at_utc: None,
                    retuned_after_freeze: false,
                };
                artifact.frozen_payload_sha256 = frozen_projection_sha256(&artifact)?;
                validate_artifact(&artifact, Phase::CalibrationFreeze, &environment)?;
                write_artifact(&environment.artifact, &canonical_json(&artifact)?)
            }
            Phase::Holdout => {
                let bytes = fs::read(&environment.artifact)?;
                let mut artifact: FrozenArtifact = serde_json::from_slice(&bytes)
                    .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
                require(
                    canonical_json(&artifact)? == bytes,
                    "selection artifact must be canonical JSON",
                )?;
                if environment.r51_formal_custody.is_some() {
                    require(
                        artifact.manifest_sha256 == environment.calibration_manifest_sha256,
                        "R51 frozen calibration manifest binding drift",
                    )?;
                }
                validate_artifact(&artifact, Phase::CalibrationFreeze, &environment)?;
                let formal_holdout = environment.r51_formal_custody.is_some();
                if formal_holdout {
                    validate_r51_runner_open(&environment, &artifact.selected_candidate_id)?;
                }
                let result = (|| {
                    let evidence = model_runner(&environment)?;
                    require(
                        evidence.selected_candidate_id == artifact.selected_candidate_id,
                        "holdout selected candidate drift",
                    )?;
                    if formal_holdout {
                        let formal = evidence
                            .r51_formal
                            .as_ref()
                            .ok_or_else(|| invalid_data("R51 formal evidence is missing"))?;
                        write_r51_diagnostic_generations(
                            &environment,
                            &artifact.selected_candidate_id,
                            &artifact.manifest_sha256,
                            formal,
                        )?;
                        if formal.first_failed_cell.is_some() {
                            return Err(invalid_data("R51 formal holdout failed"));
                        }
                    }
                    require(
                        evidence.results.iter().all(|result| result.derived.passed),
                        "holdout result failed",
                    )?;
                    artifact
                        .required_checks
                        .push(environment.required_check.clone());
                    artifact.holdout_manifest_sha256 =
                        Some(environment.r51_formal_custody.as_ref().map_or_else(
                            || environment.visual_manifest_sha256.clone(),
                            |custody| {
                                custody
                                    .holdout
                                    .as_ref()
                                    .expect("formal holdout custody")
                                    .freeze
                                    .manifest_sha256
                                    .clone()
                            },
                        ));
                    artifact.process_evidence.extend(evidence.process_evidence);
                    artifact.holdout_results = evidence.results;
                    let frozen_at = chrono::DateTime::parse_from_rfc3339(&artifact.frozen_at_utc)
                        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?
                        .with_timezone(&Utc);
                    let completed_at = Utc::now().max(frozen_at + chrono::Duration::seconds(1));
                    artifact.holdout_completed_at_utc =
                        Some(completed_at.to_rfc3339_opts(SecondsFormat::Secs, true));
                    validate_artifact(&artifact, Phase::Holdout, &environment)?;
                    write_artifact(
                        &holdout_artifact_path(&environment.artifact),
                        &canonical_json(&artifact)?,
                    )
                })();
                result
            }
        }
    }

    fn candidates_schema() -> Vec<Candidate> {
        [
            ("S25L4", 1, 4, 1, 25),
            ("S25L5", 1, 4, 1, 20),
            ("S25L6", 1, 4, 3, 50),
            ("S25L7", 1, 4, 7, 100),
        ]
        .into_iter()
        .map(
            |(
                id,
                short_side_numerator,
                short_side_denominator,
                long_side_numerator,
                long_side_denominator,
            )| Candidate {
                id: id.into(),
                short_side_numerator,
                short_side_denominator,
                long_side_numerator,
                long_side_denominator,
            },
        )
        .collect()
    }

    fn frozen_recall_contract(selected_candidate_id: &str) -> FrozenRecallContract {
        FrozenRecallContract {
            candidate_set: candidates_schema()
                .into_iter()
                .map(|candidate| candidate.id)
                .collect(),
            selected_candidate_id: selected_candidate_id.into(),
            ppocr_crop_local_preprocessing_sha256: sha256_hex(
                PPOCR_PREPROCESSING_PREIMAGE.as_bytes(),
            ),
            inverse_mapping_rule_sha256: sha256_hex(INVERSE_MAPPING_PREIMAGE.as_bytes()),
            coverage_acceptance_rule_sha256: sha256_hex(COVERAGE_ACCEPTANCE_PREIMAGE.as_bytes()),
            source_removal_preflight_rule_sha256: sha256_hex(
                SOURCE_REMOVAL_PREFLIGHT_PREIMAGE.as_bytes(),
            ),
        }
    }

    fn run_real_model(environment: &SelectionEnvironment) -> io::Result<RunnerEvidence> {
        if environment.r51_formal_custody.is_some() && environment.phase == Phase::Holdout {
            return run_r51_real_model(environment);
        }
        let selected_input = HeldInput::open_bounded(&environment.visual_input, BYTE_CEILING)?;
        require(
            selected_input.sha256() == decode_sha256(&environment.visual_input_sha256)?,
            "selected regression input sha256 mismatch",
        )?;
        let decoded_fingerprint =
            canonical_decoded_rgba_blake3(selected_input.bytes()).map_err(io::Error::other)?;
        let held = load_schema_and_hold_assets(
            &environment.visual_manifest,
            &environment.visual_manifest_sha256,
            &environment.visual_input,
            &decoded_fingerprint,
            &environment.visual_input_sha256,
        )
        .map_err(io::Error::other)?;
        let validated =
            validate_visual_oracles(validate_dimensions_and_masks(held).map_err(io::Error::other)?)
                .map_err(io::Error::other)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let executable_sha256 = sha256_file(&std::env::current_exe()?)?;
        validated.upstream.held_schema.with_revalidated_paths(|| {
            let entries = validated
                .upstream
                .held_schema
                .schema
                .entries
                .iter()
                .zip(&validated.upstream.entries)
                .zip(&validated.entries)
                .map(|((schema, decoded), oracle)| RealModelEntry {
                    schema,
                    source: &decoded.source,
                    oracle,
                })
                .collect::<Vec<_>>();
            runtime.block_on(run_real_model_async(
                environment,
                &entries,
                executable_sha256,
                None,
            ))
        })
    }

    struct RealModelEntry<'a> {
        schema: &'a VisualManifestEntry,
        source: &'a RgbaImage,
        oracle: &'a OracleValidatedEntry,
    }

    fn run_r51_real_model(environment: &SelectionEnvironment) -> io::Result<RunnerEvidence> {
        let holdout = environment
            .r51_formal_custody
            .as_ref()
            .and_then(|custody| custody.holdout.as_ref())
            .ok_or_else(|| invalid_data("R51 holdout custody is unavailable"))?;
        let archive = HeldInput::open(&holdout.plaintext_archive)?;
        let validated = validate_r51_plaintext_holdout_bundle(
            &holdout.plaintext_directory,
            &holdout.plaintext_archive,
            archive.bytes(),
            R51BundleFreezeCommitments {
                plaintext_archive_sha256: &holdout.freeze.plaintext_archive_sha256,
                manifest_sha256: &holdout.freeze.manifest_sha256,
                oracle_sha256: &holdout.freeze.oracle_sha256,
                hashes_sha256: &holdout.freeze.hashes_sha256,
            },
        )?;
        let prepared = prepare_r51_execution_entries(validated.execution)?;
        let entries = prepared
            .iter()
            .map(|(schema, source, oracle)| RealModelEntry {
                schema,
                source,
                oracle,
            })
            .collect::<Vec<_>>();
        let executable_sha256 = sha256_file(&std::env::current_exe()?)?;
        let bundle_validation_receipt = write_r51_bundle_validation_receipt(
            environment,
            &executable_sha256,
            &validated.receipt,
        )?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(run_real_model_async(
            environment,
            &entries,
            executable_sha256,
            Some(bundle_validation_receipt),
        ))
    }

    fn prepare_r51_execution_entries(
        validated: R51ValidatedExecutionView,
    ) -> io::Result<Vec<(VisualManifestEntry, RgbaImage, OracleValidatedEntry)>> {
        validated
            .entries
            .into_iter()
            .map(|entry| {
                require(
                    entry.source_width == entry.clean_width
                        && entry.source_height == entry.clean_height
                        && entry.validated_source_rgba.dimensions()
                            == (entry.source_width, entry.source_height)
                        && entry.validated_clean_reference_rgba.dimensions()
                            == (entry.clean_width, entry.clean_height),
                    "R51 validated execution dimensions drift",
                )?;
                let page_len =
                    usize::try_from(u64::from(entry.source_width) * u64::from(entry.source_height))
                        .map_err(|_| invalid_data("R51 execution page length overflow"))?;
                let source_hash = sha256_hex(&entry.source_encoded_bytes);
                let source_decoded_hash = rgba_fingerprint(&DynamicImage::ImageRgba8(
                    entry.validated_source_rgba.clone(),
                ));
                let clean_hash = sha256_hex(&entry.clean_reference_encoded_bytes);
                let clean_decoded_hash = rgba_fingerprint(&DynamicImage::ImageRgba8(
                    entry.validated_clean_reference_rgba.clone(),
                ));
                let mut schema_targets = Vec::with_capacity(entry.targets.len());
                let mut oracle_targets = Vec::with_capacity(entry.targets.len());
                for target in entry.targets {
                    require(
                        target.validated_binary_mask.len() == page_len
                            && target
                                .validated_binary_mask
                                .iter()
                                .all(|pixel| matches!(pixel, 0 | 1)),
                        "R51 validated execution mask drift",
                    )?;
                    let source_roi = validated_rect(target.source_roi)?;
                    let edit_roi = validated_rect(target.clean_reference_edit_roi)?;
                    let mut local_mask = Vec::with_capacity(
                        usize::try_from(
                            u64::from(edit_roi.right - edit_roi.left)
                                * u64::from(edit_roi.bottom - edit_roi.top),
                        )
                        .map_err(|_| invalid_data("R51 execution mask length overflow"))?,
                    );
                    for y in edit_roi.top..edit_roi.bottom {
                        let start =
                            y as usize * entry.source_width as usize + edit_roi.left as usize;
                        let end =
                            y as usize * entry.source_width as usize + edit_roi.right as usize;
                        local_mask.extend_from_slice(&target.validated_binary_mask[start..end]);
                    }
                    let erase_mask_hash = sha256_hex(&target.erase_source_ink_mask_encoded_bytes);
                    let residual_mask_hash =
                        sha256_hex(&target.residual_source_ink_mask_encoded_bytes);
                    schema_targets.push(VisualManifestTarget {
                        id: target.id.clone(),
                        source_roi: target.source_roi.map(u64::from),
                        clean_reference_edit_roi: target.clean_reference_edit_roi.map(u64::from),
                        erase_source_ink_mask_path: format!(
                            "assets/masks/{}/{}-erase.png",
                            entry.id, target.id
                        ),
                        erase_source_ink_mask_sha256: erase_mask_hash,
                        residual_source_ink_mask_path: format!(
                            "assets/masks/{}/{}-residual.png",
                            entry.id, target.id
                        ),
                        residual_source_ink_mask_sha256: residual_mask_hash,
                        position: match target.position.as_str() {
                            "interior" => Position::Interior,
                            "page_edge" => Position::PageEdge,
                            _ => return Err(invalid_data("R51 execution position drift")),
                        },
                        writing: match target.writing.as_str() {
                            "horizontal" => Writing::Horizontal,
                            "vertical" => Writing::Vertical,
                            _ => return Err(invalid_data("R51 execution writing drift")),
                        },
                        effect: match target.effect.as_str() {
                            "plain" => Effect::Plain,
                            "stroke" => Effect::Stroke,
                            _ => return Err(invalid_data("R51 execution effect drift")),
                        },
                        translation_length: match target.translation_length.as_str() {
                            "short" => TranslationLength::Short,
                            "equal" => TranslationLength::Equal,
                            "2x" => TranslationLength::TwoX,
                            "3x" => TranslationLength::ThreeX,
                            _ => {
                                return Err(invalid_data("R51 execution translation length drift"));
                            }
                        },
                        expected: match target.expected.as_str() {
                            "automatic_strict" => Expected::AutomaticStrict,
                            _ => return Err(invalid_data("R51 execution expected mode drift")),
                        },
                    });
                    oracle_targets.push(OracleValidatedTarget {
                        source_roi,
                        edit_roi,
                        delta_mask: local_mask.into_boxed_slice(),
                    });
                }
                let max_side = entry.source_width.max(entry.source_height);
                let aspect = if u64::from(entry.source_width) * 10
                    > u64::from(entry.source_height) * 11
                {
                    Aspect::Landscape
                } else if u64::from(entry.source_height) * 10 > u64::from(entry.source_width) * 11 {
                    Aspect::Portrait
                } else {
                    Aspect::SquareOrNear
                };
                let dimension_bin = match max_side {
                    0..=719 => DimensionBin::Lt720,
                    720..=1439 => DimensionBin::From720To1439,
                    1440..=2159 => DimensionBin::From1440To2159,
                    _ => DimensionBin::Gte2160,
                };
                let protected_rois = entry
                    .protected_rois
                    .iter()
                    .map(|rect| rect.map(u64::from))
                    .collect();
                let oracle_protected_rois = entry
                    .protected_rois
                    .into_iter()
                    .map(validated_rect)
                    .collect::<io::Result<Vec<_>>>()?;
                let schema = VisualManifestEntry {
                    id: entry.id.clone(),
                    path: format!("assets/source/{}.withheld", entry.id),
                    sha256: source_hash.clone(),
                    decoded_rgba_blake3: source_decoded_hash.clone(),
                    clean_reference_path: format!("assets/clean/{}.withheld", entry.id),
                    clean_reference_sha256: clean_hash,
                    clean_reference_decoded_rgba_blake3: clean_decoded_hash,
                    role: EntryRole::Holdout,
                    dimension_bin,
                    aspect,
                    background: Background::Product,
                    targets: schema_targets,
                    protected_rois,
                    multi_node: oracle_targets.len() > 1,
                };
                Ok((
                    schema,
                    entry.validated_source_rgba,
                    OracleValidatedEntry {
                        protected_rois: oracle_protected_rois,
                        targets: oracle_targets,
                    },
                ))
            })
            .collect()
    }

    fn validated_rect([left, top, right, bottom]: [u32; 4]) -> io::Result<ValidatedHalfOpenRect> {
        require(
            left < right && top < bottom,
            "R51 validated execution rectangle drift",
        )?;
        Ok(ValidatedHalfOpenRect {
            left,
            top,
            right,
            bottom,
        })
    }

    async fn run_real_model_async(
        environment: &SelectionEnvironment,
        entries: &[RealModelEntry<'_>],
        executable_sha256: String,
        bundle_validation_receipt: Option<PublishedArtifact>,
    ) -> io::Result<RunnerEvidence> {
        create_secure_report_dir(&environment.report_dir)?;
        let runtime = RuntimeManager::new(
            default_app_data_root().as_std_path(),
            ComputePolicy::PreferGpu,
        )
        .map_err(io::Error::other)?;
        runtime.prepare().await.map_err(io::Error::other)?;
        let backend = crate::app::shared_llama_backend(&runtime).map_err(io::Error::other)?;

        let downloads = runtime.downloads();
        let (pp_detection, pp_recognition, pp_recognition_config) = tokio::try_join!(
            downloads.huggingface_model(PP_REPO, PP_DETECTION_MODEL),
            downloads.huggingface_model(PP_REPO, PP_RECOGNITION_MODEL),
            downloads.huggingface_model(PP_REPO, PP_RECOGNITION_CONFIG),
        )
        .map_err(io::Error::other)?;
        let pp = PpOcrV5::load(&runtime).await.map_err(io::Error::other)?;
        let runtime_library_sha256 = runtime_library_hashes(&runtime)?;
        let phase = phase_name(environment.phase);
        let selected = selected_candidates(environment)?;
        let mut process_evidence = Vec::with_capacity(2);
        let mut results = Vec::new();
        let mut formal_cells = Vec::new();
        let mut first_failed_cell = None;
        let mut runners = Vec::with_capacity(2);

        for (device, cpu) in [("cpu", true), ("metal", false)] {
            let mut logs = NativeLogCaptureGuard::start();
            let vl = PaddleOcrVl::load(&runtime, cpu, backend.clone())
                .await
                .map_err(io::Error::other)?;
            let vl_evidence = vl.device_evidence();
            let load_bytes = logs.take();
            let parsed_load = parse_native_load_log(&load_bytes)?;
            let load_log = write_raw_log(
                environment,
                &format!("source-gate/{phase}/{device}/load.log"),
                &load_bytes,
            )?;
            let enumerated_devices = enumerated_devices()?;
            let loaded_model_devices = loaded_model_devices(
                &enumerated_devices,
                &parsed_load.model_buffer_bytes_by_backend,
            )?;
            let process_id = format!("{phase}-{device}");
            let process = ProcessEvidence {
                id: process_id.clone(),
                phase: phase.into(),
                requested_device: device.into(),
                paddle_instance_id: vl_evidence.instance_id.clone(),
                executable_sha256: executable_sha256.clone(),
                model_artifact_sha256: ModelArtifactHashes {
                    pp_detection: sha256_file(&pp_detection)?,
                    pp_recognition: sha256_file(&pp_recognition)?,
                    pp_recognition_config: sha256_file(&pp_recognition_config)?,
                    vl_model: sha256_file(&vl_evidence.model_path)?,
                    vl_mmproj: sha256_file(&vl_evidence.mmproj_path)?,
                },
                runtime_library_sha256: runtime_library_sha256.clone(),
                load_evidence: LoadEvidence {
                    cpu_forced: vl_evidence.requested_cpu,
                    gpu_offload_supported: backend.supports_gpu_offload(),
                    n_gpu_layers: vl_evidence.model_n_gpu_layers,
                    mtmd_use_gpu: vl_evidence.mtmd_use_gpu,
                    word_boxes_backend: "rten_cpu".into(),
                    raw_load_log_relpath: load_log.0,
                    raw_load_log_sha256: load_log.1,
                    enumerated_devices,
                    loaded_model_devices,
                    offloaded_layers: parsed_load.offloaded_layers,
                    offloadable_layers: parsed_load.offloadable_layers,
                    model_buffer_bytes_by_backend: parsed_load.model_buffer_bytes_by_backend,
                    mtmd_backend: parsed_load.mtmd_backend,
                },
            };
            runners.push((device, vl, vl_evidence, process));
        }

        for entry in entries
            .iter()
            .filter(|entry| entry.schema.role == phase_role(environment.phase))
        {
            let schema_entry = entry.schema;
            let oracle_entry = entry.oracle;
            for (device, vl, vl_evidence, process) in &mut runners {
                for (candidate_id, policy) in &selected {
                    let mut logs = NativeLogCaptureGuard::start();
                    let mut scene = scene_for_entry(
                        schema_entry,
                        oracle_entry,
                        entry.source.width(),
                        entry.source.height(),
                    );
                    let page = *scene.pages.keys().next().expect("scene page");
                    let image = DynamicImage::ImageRgba8(entry.source.clone());
                    let _policy = SourceGateCropPolicyGuard::set(*policy);
                    let source_gate_diagnostics = SourceGateDiagnosticCapture::start();
                    let ops = dispatch_source_gate(
                        &image,
                        &scene,
                        page,
                        |_, crop| pp.observe(crop),
                        |crops| {
                            std::future::ready(
                                vl.inference_images(
                                    &crops,
                                    PaddleOcrVlTask::Ocr,
                                    VL_MAX_NEW_TOKENS,
                                )
                                .map(|outputs| {
                                    outputs.into_iter().map(|output| output.text).collect()
                                }),
                            )
                        },
                    )
                    .await
                    .map_err(io::Error::other)?;
                    let source_gate_events = source_gate_diagnostics.take();
                    for mut op in ops {
                        op.apply(&mut scene).map_err(io::Error::other)?;
                    }
                    let inference_bytes = logs.take();
                    let parsed_inference = parse_native_inference_log(&inference_bytes)?;
                    let inference_log = write_raw_log(
                        environment,
                        &format!(
                            "source-gate/{phase}/{}/{device}/{candidate_id}.log",
                            schema_entry.id
                        ),
                        &inference_bytes,
                    )?;
                    let source_gate_diagnostic = write_raw_log(
                        environment,
                        &format!(
                            "source-gate/{phase}/{}/{device}/{candidate_id}.source-gate.json",
                            schema_entry.id
                        ),
                        &canonical_json(&source_gate_events)?,
                    )?;
                    let (runtime_nodes, derived, supports) = derive_result(
                        device,
                        &scene,
                        page,
                        schema_entry,
                        oracle_entry,
                        &source_gate_events,
                    )?;
                    let mut result = SelectionResult {
                        entry_id: schema_entry.id.clone(),
                        process_evidence_id: process.id.clone(),
                        candidate_id: candidate_id.clone(),
                        execution_evidence: ExecutionEvidence {
                            paddle_instance_id: vl_evidence.instance_id.clone(),
                            context_offload_kqv: vl_evidence.context_offload_kqv,
                            context_op_offload: vl_evidence.context_op_offload,
                            inference_completed: true,
                            raw_inference_log_relpath: inference_log.0,
                            raw_inference_log_sha256: inference_log.1,
                            source_gate_diagnostic_relpath: source_gate_diagnostic.0,
                            source_gate_diagnostic_sha256: source_gate_diagnostic.1,
                            context_buffer_bytes_by_backend: parsed_inference
                                .context_buffer_bytes_by_backend,
                            compute_buffer_bytes_by_backend: parsed_inference
                                .compute_buffer_bytes_by_backend,
                        },
                        runtime_nodes,
                        derived,
                    };
                    if environment.r51_formal_custody.is_some() {
                        let cell = match environment.phase {
                            Phase::CalibrationFreeze => write_r51_calibration_cell_evidence(
                                environment,
                                process,
                                &result,
                                schema_entry,
                                oracle_entry,
                                &source_gate_events,
                            )?,
                            Phase::Holdout => write_r51_cell_evidence(
                                environment,
                                process,
                                &mut result,
                                schema_entry,
                                oracle_entry,
                                &source_gate_events,
                                &supports,
                                bundle_validation_receipt
                                    .as_ref()
                                    .ok_or_else(|| invalid_data("R51 bundle receipt is missing"))?,
                            )?,
                        };
                        let failed = cell.result != "pass";
                        if failed && environment.phase == Phase::Holdout {
                            first_failed_cell = Some(cell.cell_key.clone());
                        }
                        formal_cells.push(cell);
                        results.push(result);
                        if failed && environment.phase == Phase::Holdout {
                            break;
                        }
                    } else {
                        results.push(result);
                    }
                }
                if first_failed_cell.is_some() {
                    break;
                }
            }
            if first_failed_cell.is_some() {
                break;
            }
        }
        process_evidence.extend(runners.into_iter().map(|(_, _, _, process)| process));
        let selected_candidate_id = if environment.phase == Phase::CalibrationFreeze {
            select_or_write_calibration_diagnostic(environment, &process_evidence, &results)?
        } else {
            selected[0].0.clone()
        };
        let r51_formal = environment
            .r51_formal_custody
            .as_ref()
            .map(|_| R51FormalRunEvidence {
                bundle_validation_receipt,
                cells: formal_cells,
                first_failed_cell,
            });
        Ok(RunnerEvidence {
            selected_candidate_id,
            process_evidence,
            results,
            r51_formal,
        })
    }

    fn phase_name(phase: Phase) -> &'static str {
        match phase {
            Phase::CalibrationFreeze => "calibration",
            Phase::Holdout => "holdout",
        }
    }

    fn phase_role(phase: Phase) -> EntryRole {
        match phase {
            Phase::CalibrationFreeze => EntryRole::Calibration,
            Phase::Holdout => EntryRole::Holdout,
        }
    }

    fn frozen_holdout_entry_ids(environment: &SelectionEnvironment) -> Vec<String> {
        if environment.r51_formal_custody.is_some() {
            r51_entry_ids('h')
        } else {
            environment.holdout_entry_ids.clone()
        }
    }

    fn selected_candidates(
        environment: &SelectionEnvironment,
    ) -> io::Result<Vec<(String, SourceGateCropPolicy)>> {
        let all = [
            ("S25L4", SourceGateCropPolicy::S25L4),
            ("S25L5", SourceGateCropPolicy::S25L5),
            ("S25L6", SourceGateCropPolicy::S25L6),
            ("S25L7", SourceGateCropPolicy::S25L7),
        ];
        if environment.phase == Phase::CalibrationFreeze {
            return Ok(all
                .into_iter()
                .map(|(id, policy)| (id.into(), policy))
                .collect());
        }
        let artifact: FrozenArtifact = serde_json::from_slice(&fs::read(&environment.artifact)?)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        all.into_iter()
            .find(|(id, _)| *id == artifact.selected_candidate_id)
            .map(|(id, policy)| vec![(id.into(), policy)])
            .ok_or_else(|| invalid_data("frozen candidate is unknown"))
    }

    fn scene_for_entry(
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
        width: u32,
        height: u32,
    ) -> Scene {
        let mut page = Page::new(&schema.id, width, height);
        for (target, geometry) in schema.targets.iter().zip(&oracle.targets) {
            let roi = geometry.source_roi;
            let rotation = if target.expected == Expected::UnsupportedRotation {
                1.0
            } else {
                0.0
            };
            let id = NodeId::new();
            page.nodes.insert(
                id,
                Node {
                    id,
                    transform: Transform {
                        x: roi.left as f32,
                        y: roi.top as f32,
                        width: (roi.right - roi.left) as f32,
                        height: (roi.bottom - roi.top) as f32,
                        rotation_deg: rotation,
                    },
                    visible: true,
                    kind: NodeKind::Text(TextData {
                        confidence: 1.0,
                        rotation_deg: Some(rotation),
                        detector: Some("d0_visual_manifest".into()),
                        ..Default::default()
                    }),
                },
            );
        }
        let page_id = page.id;
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        scene
    }

    fn r51_quad_bits_rect(bits: [u32; 8]) -> io::Result<[i64; 4]> {
        let xs = [0, 2, 4, 6].map(|index| f32::from_bits(bits[index]));
        let ys = [1, 3, 5, 7].map(|index| f32::from_bits(bits[index]));
        require(
            xs.iter().chain(&ys).all(|value| value.is_finite()),
            "R51 selection geometry is non-finite",
        )?;
        Ok([
            xs.iter().copied().fold(f32::INFINITY, f32::min).floor() as i64,
            ys.iter().copied().fold(f32::INFINITY, f32::min).floor() as i64,
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i64,
            ys.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i64,
        ])
    }

    fn r51_rect_mask(width: u32, height: u32, rect: [i64; 4]) -> Vec<u8> {
        let mut bytes = vec![0_u8; width as usize * height as usize];
        let [left, top, right, bottom] = rect;
        let left = left.clamp(0, i64::from(width)) as usize;
        let right = right.clamp(0, i64::from(width)) as usize;
        let top = top.clamp(0, i64::from(height)) as usize;
        let bottom = bottom.clamp(0, i64::from(height)) as usize;
        for y in top..bottom {
            bytes[y * width as usize + left..y * width as usize + right].fill(1);
        }
        bytes
    }

    fn r51_selected_support_from_diagnostics(
        width: u32,
        height: u32,
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
        diagnostics: &[SourceGateDiagnosticEvent],
    ) -> io::Result<BTreeMap<String, Vec<u8>>> {
        let mut selected = BTreeMap::<String, Vec<u8>>::new();
        for event in diagnostics {
            let SourceGateDiagnosticEvent::SelectionGeometry { targets, .. } = event else {
                continue;
            };
            for target in targets {
                let rect = r51_quad_bits_rect(target.scene_quad_f32_bits)?;
                let center = (
                    (rect[0] + rect[2]) as f64 / 2.0,
                    (rect[1] + rect[3]) as f64 / 2.0,
                );
                let matches = schema
                    .targets
                    .iter()
                    .zip(&oracle.targets)
                    .filter(|(_, geometry)| rect_contains(geometry.source_roi, center))
                    .map(|(target, _)| target.id.as_str())
                    .collect::<Vec<_>>();
                require(
                    matches.len() == 1,
                    "R51 emitted target geometry ownership is not unique",
                )?;
                let support = selected
                    .entry(matches[0].to_owned())
                    .or_insert_with(|| vec![0; width as usize * height as usize]);
                for (pixel, addition) in support.iter_mut().zip(r51_rect_mask(width, height, rect))
                {
                    *pixel |= addition;
                }
            }
        }
        Ok(selected)
    }

    fn r51_downstream_support_from_scene(
        page: &Page,
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
    ) -> io::Result<BTreeMap<String, image::GrayImage>> {
        let mut support_by_target = BTreeMap::<String, image::GrayImage>::new();
        for node in page.nodes.values() {
            let NodeKind::Text(text) = &node.kind else {
                continue;
            };
            if !node.visible || text.detector.as_deref() != Some(SOURCE_GATE_TARGET_DETECTOR) {
                continue;
            }
            let center = (
                f64::from(node.transform.x + node.transform.width / 2.0),
                f64::from(node.transform.y + node.transform.height / 2.0),
            );
            let target = schema
                .targets
                .iter()
                .zip(&oracle.targets)
                .find(|(_, geometry)| rect_contains(geometry.source_roi, center))
                .map(|(target, _)| target)
                .ok_or_else(|| invalid_data("R51 downstream scene target is unassigned"))?;
            let lines = eligible_text_lines(&node.transform, text, page.width, page.height)
                .ok_or_else(|| invalid_data("R51 downstream scene geometry is unsupported"))?;
            let mask = line_support_mask(page.width, page.height, &lines);
            let accumulated = support_by_target
                .entry(target.id.clone())
                .or_insert_with(|| image::GrayImage::new(page.width, page.height));
            for (current, addition) in accumulated.pixels_mut().zip(mask.pixels()) {
                current.0[0] |= addition.0[0];
            }
        }
        Ok(support_by_target)
    }

    fn derive_result(
        device: &str,
        scene: &Scene,
        page: koharu_core::PageId,
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
        diagnostics: &[SourceGateDiagnosticEvent],
    ) -> io::Result<(Vec<RuntimeNode>, DerivedEvidence, CellSupportEvidence)> {
        let mut pp_han_by_node = HashMap::new();
        let mut vl_han_by_node = HashMap::new();
        let mut rejected_after_vl = false;
        let mut pp_vl_incomplete_coverage = false;
        for event in diagnostics {
            match event {
                SourceGateDiagnosticEvent::PpSummary { node_id, words, .. } => {
                    pp_han_by_node.insert(
                        *node_id,
                        words
                            .iter()
                            .map(|word| word.han_scalar_count)
                            .sum::<usize>(),
                    );
                }
                SourceGateDiagnosticEvent::VlSummary {
                    node_id,
                    han_scalar_count,
                    ..
                } => {
                    vl_han_by_node.insert(*node_id, *han_scalar_count);
                }
                SourceGateDiagnosticEvent::Decision {
                    decision: SourceGateDecision::RejectedAfterVl { reason },
                    ..
                } => {
                    rejected_after_vl = true;
                    pp_vl_incomplete_coverage |=
                        *reason == SourceGateRejectReason::PpVlIncompleteCoverage;
                }
                _ => {}
            }
        }
        let mut runtime_nodes = Vec::new();
        let mut matched = HashSet::new();
        let mut selected = HashSet::new();
        let mut selected_protected = Vec::new();
        let mut unmatched_selected = Vec::new();
        let mut pp_han_scalar_count = 0;
        let mut vl_expected_han_scalar_count = 0;
        let mut expected_diagnostic_nodes = 0;
        let page = scene
            .page(page)
            .ok_or_else(|| invalid_data("runtime scene page is missing"))?;
        for (node_id, node) in &page.nodes {
            let NodeKind::Text(text) = &node.kind else {
                continue;
            };
            let selected_as_han =
                node.visible && text.detector.as_deref() == Some(SOURCE_GATE_TARGET_DETECTOR);
            let anchor = [
                f64::from(node.transform.x),
                f64::from(node.transform.y),
                f64::from(node.transform.x + node.transform.width),
                f64::from(node.transform.y + node.transform.height),
            ];
            let center = ((anchor[0] + anchor[2]) / 2.0, (anchor[1] + anchor[3]) / 2.0);
            let target = schema
                .targets
                .iter()
                .zip(&oracle.targets)
                .find(|(_, target)| rect_contains(target.source_roi, center))
                .map(|(schema, _)| schema);
            if let Some(target) = target {
                matched.insert(target.id.clone());
                if target.expected != Expected::UnsupportedRotation {
                    let pp_count = pp_han_by_node.get(node_id).copied().unwrap_or_default();
                    let vl_count = vl_han_by_node.get(node_id).copied().unwrap_or_default();
                    pp_han_scalar_count += pp_count;
                    vl_expected_han_scalar_count += vl_count;
                    expected_diagnostic_nodes += 1;
                }
                if selected_as_han {
                    selected.insert(target.id.clone());
                }
            } else if selected_as_han {
                unmatched_selected.push(node_id.to_string());
            }
            if selected_as_han
                && oracle
                    .protected_rois
                    .iter()
                    .any(|protected| rect_intersects(*protected, anchor))
            {
                selected_protected.push(node_id.to_string());
            }
            runtime_nodes.push(RuntimeNode {
                node_id: node_id.to_string(),
                recognition_anchor: anchor,
                node_rotation: f64::from(node.transform.rotation_deg),
                text_rotation: f64::from(text.rotation_deg.unwrap_or_default()),
                selected_as_han,
            });
        }
        let mut rotation = schema
            .targets
            .iter()
            .filter(|target| {
                target.expected == Expected::UnsupportedRotation && selected.contains(&target.id)
            })
            .map(|target| target.id.clone())
            .collect::<Vec<_>>();
        let expected = schema
            .targets
            .iter()
            .filter(|target| target.expected != Expected::UnsupportedRotation)
            .map(|target| target.id.as_str())
            .collect::<HashSet<_>>();
        let selected_expected = selected
            .iter()
            .filter(|id| expected.contains(id.as_str()))
            .count();
        let recall = if expected.is_empty() {
            1.0
        } else {
            selected_expected as f64 / expected.len() as f64
        };
        let mut matched = matched.into_iter().collect::<Vec<_>>();
        let mut selected = selected.into_iter().collect::<Vec<_>>();
        matched.sort();
        selected.sort();
        rotation.sort();
        selected_protected.sort();
        unmatched_selected.sort();
        let rotation_targets_excluded = rotation.is_empty();
        let downstream_support_by_target = r51_downstream_support_from_scene(page, schema, oracle)?;
        let mut covered_source_roi_ids = schema
            .targets
            .iter()
            .zip(&oracle.targets)
            .filter(|(target, _)| target.expected != Expected::UnsupportedRotation)
            .filter_map(|(target, oracle_target)| {
                let support = downstream_support_by_target.get(&target.id)?;
                let mut delta_index = 0;
                let covered =
                    (oracle_target.edit_roi.top..oracle_target.edit_roi.bottom).all(|y| {
                        (oracle_target.edit_roi.left..oracle_target.edit_roi.right).all(|x| {
                            let oracle = oracle_target.delta_mask[delta_index];
                            delta_index += 1;
                            let page_index = y as usize * page.width as usize + x as usize;
                            oracle == 0 || support.as_raw()[page_index] != 0
                        })
                    });
                covered.then(|| target.id.clone())
            })
            .collect::<Vec<_>>();
        covered_source_roi_ids.sort();
        let source_text_roi_coverage = if expected.is_empty() {
            1.0
        } else {
            covered_source_roi_ids.len() as f64 / expected.len() as f64
        };
        let pp_vl_complete_coverage = expected_diagnostic_nodes > 0
            && !rejected_after_vl
            && !pp_vl_incomplete_coverage
            && source_text_roi_coverage == 1.0;
        let source_removal_preflight_passed = recall == 1.0 && pp_vl_complete_coverage;
        let passed = source_removal_preflight_passed
            && selected_protected.is_empty()
            && unmatched_selected.is_empty()
            && rotation_targets_excluded;
        let downstream_by_target = downstream_support_by_target
            .into_iter()
            .map(|(target, support)| {
                (
                    target,
                    support
                        .as_raw()
                        .iter()
                        .map(|pixel| u8::from(*pixel != 0))
                        .collect(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let selected_by_target = r51_selected_support_from_diagnostics(
            page.width,
            page.height,
            schema,
            oracle,
            diagnostics,
        )?;
        Ok((
            runtime_nodes,
            DerivedEvidence {
                actual_device: device.into(),
                matched_target_ids: matched,
                selected_target_ids: selected,
                selected_protected_node_ids: selected_protected.clone(),
                selected_rotation_target_ids: rotation,
                unmatched_selected_node_ids: unmatched_selected,
                target_recall: recall,
                protected_false_positive_count: selected_protected.len() as u32,
                rotation_targets_excluded,
                source_coverage_preflight: SourceCoveragePreflight {
                    pp_han_scalar_count,
                    vl_expected_han_scalar_count,
                    pp_vl_complete_coverage,
                    rejected_after_vl,
                    pp_vl_incomplete_coverage,
                    covered_source_roi_ids,
                    source_text_roi_coverage,
                    source_removal_preflight_passed,
                },
                passed,
            },
            CellSupportEvidence {
                width: page.width,
                height: page.height,
                downstream_by_target,
                selected_by_target,
            },
        ))
    }

    fn rect_contains(rect: ValidatedHalfOpenRect, point: (f64, f64)) -> bool {
        f64::from(rect.left) <= point.0
            && point.0 < f64::from(rect.right)
            && f64::from(rect.top) <= point.1
            && point.1 < f64::from(rect.bottom)
    }

    fn rect_intersects(rect: ValidatedHalfOpenRect, anchor: [f64; 4]) -> bool {
        f64::from(rect.left) < anchor[2]
            && anchor[0] < f64::from(rect.right)
            && f64::from(rect.top) < anchor[3]
            && anchor[1] < f64::from(rect.bottom)
    }

    fn create_secure_report_dir(path: &Path) -> io::Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }

    fn write_raw_log(
        environment: &SelectionEnvironment,
        suffix: &str,
        bytes: &[u8],
    ) -> io::Result<(String, String)> {
        require(!bytes.is_empty(), "native log capture is empty")?;
        let path = environment.report_dir.join(suffix);
        require(
            path.starts_with(&environment.evidence_root),
            "raw log escaped evidence root",
        )?;
        let parent = path
            .parent()
            .ok_or_else(|| invalid_data("raw log has no parent"))?;
        create_secure_report_dir(parent)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
        let artifact_parent = environment
            .artifact
            .parent()
            .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
        let relative = path
            .strip_prefix(artifact_parent)
            .map_err(|_| invalid_data("raw log is outside artifact parent"))?
            .to_str()
            .ok_or_else(|| invalid_data("raw log path is not utf-8"))?
            .to_owned();
        Ok((relative, sha256_hex(bytes)))
    }

    fn write_r51_bundle_validation_receipt(
        environment: &SelectionEnvironment,
        executable_sha256: &str,
        validated: &R51ValidatedReceiptData,
    ) -> io::Result<PublishedArtifact> {
        let custody = environment
            .r51_formal_custody
            .as_ref()
            .ok_or_else(|| invalid_data("R51 formal custody is not enabled"))?;
        let holdout = custody
            .holdout
            .as_ref()
            .ok_or_else(|| invalid_data("R51 holdout custody is unavailable"))?;
        require(
            environment.phase == Phase::Holdout && holdout.open_marker.get().is_some(),
            "R51 bundle receipt is holdout-only",
        )?;
        let receipt = R51BundleValidationReceipt {
            contract: "hanonly-r51-bundle-validation-v1",
            plan_revision: PLAN_REVISION,
            b0_sha: &environment.b0_sha,
            test_executable_sha256: executable_sha256,
            enabled_cargo_features: ["hanonly-test-evidence"],
            r51_contract_sha256: &custody.contract_sha256,
            freeze_receipt_sha256: &holdout.freeze.receipt_sha256,
            plaintext_archive_sha256: &validated.plaintext_archive_sha256,
            manifest_sha256: &validated.manifest_sha256,
            oracle_sha256: &validated.oracle_sha256,
            hashes_sha256: &validated.hashes_sha256,
            schema_validation_pass: validated.schema_validation_pass,
            asset_binding_pass: validated.asset_binding_pass,
            mask_source_clean_equality_pass: validated.mask_source_clean_equality_pass,
            oracle_semantics_pass: validated.oracle_semantics_pass,
            result: "pass",
        };
        publish_r51_artifact(
            environment,
            "r51/bundle-validation.json",
            &canonical_json(&receipt)?,
        )
    }

    fn load_r51_freeze_commitments(directory: &Path) -> io::Result<R51FreezeCommitments> {
        let held = HeldInput::open(&directory.join("holdout-freeze-receipt.json"))?;
        held.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
        let receipt: R51FreezeReceipt = serde_json::from_slice(held.bytes())
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        require(
            canonical_json(&receipt_as_value(held.bytes())?)? == held.bytes()
                && receipt.contract == "hanonly-r51-encrypted-holdout-freeze-v1"
                && receipt.plan_revision == PLAN_REVISION
                && receipt.frozen_before_production_edit
                && receipt.entry_ids == r51_entry_ids('h')
                && receipt.cipher == "aes-256-ctr"
                && receipt.integrity == "hmac-sha256-etm-v1"
                && receipt.ciphertext_byte_length > 0
                && receipt.disclosed_challenge_exclusion_pass
                && receipt.formal_source_identities.len() == 4
                && receipt.result == "pass",
            "R51 freeze receipt drift",
        )?;
        for hash in [
            &receipt.iv_sha256,
            &receipt.ciphertext_sha256,
            &receipt.header_sha256,
            &receipt.hmac_sha256,
            &receipt.plaintext_archive_sha256_commitment,
            &receipt.manifest_sha256_commitment,
            &receipt.oracle_sha256_commitment,
            &receipt.hashes_sha256_commitment,
            &receipt.historical_inventory_sha256,
        ] {
            decode_sha256(hash)?;
        }
        require(
            !receipt.base_b0_sha.is_empty() && !receipt.implementation_thread_id.is_empty(),
            "R51 freeze receipt identity is missing",
        )?;
        Ok(R51FreezeCommitments {
            receipt_sha256: hex_sha256(held.sha256()),
            ciphertext_sha256: receipt.ciphertext_sha256,
            plaintext_archive_sha256: receipt.plaintext_archive_sha256_commitment,
            manifest_sha256: receipt.manifest_sha256_commitment,
            oracle_sha256: receipt.oracle_sha256_commitment,
            hashes_sha256: receipt.hashes_sha256_commitment,
        })
    }

    fn receipt_as_value(bytes: &[u8]) -> io::Result<serde_json::Value> {
        serde_json::from_slice(bytes)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))
    }

    fn hex_sha256(bytes: [u8; 32]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn validate_r51_runner_open(
        environment: &SelectionEnvironment,
        selected_candidate_id: &str,
    ) -> io::Result<()> {
        let holdout = environment
            .r51_formal_custody
            .as_ref()
            .and_then(|custody| custody.holdout.as_ref())
            .ok_or_else(|| invalid_data("R51 holdout custody is unavailable"))?;
        require(
            holdout.open_marker.get().is_none(),
            "R51 runner open marker was already consumed",
        )?;
        let custody = R51HeldDirectory::open(&holdout.directory)?;
        validate_r51_custody_entry_state(custody.descriptor.as_fd())?;
        let descriptor = openat(
            custody.descriptor.as_fd(),
            "holdout-open.json",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?;
        let metadata = r51_descriptor_metadata(descriptor.as_fd())?;
        require(
            metadata.file_type.is_file()
                && metadata.owner == effective_owner()?
                && metadata.mode & 0o7777 == 0o600,
            "R51 runner open marker metadata is invalid",
        )?;
        let mut file = fs::File::from(descriptor);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let actual_sha256 = sha256_hex(&bytes);
        require(
            actual_sha256 == holdout.expected_open_marker_sha256,
            "R51 runner open marker hash drift",
        )?;
        let marker: R51OpenMarker = serde_json::from_slice(&bytes)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        require(
            canonical_json(&marker)? == bytes,
            "R51 runner open marker is not canonical JSON",
        )?;
        require(
            marker.contract == "hanonly-r51-encrypted-holdout-open-v1"
                && marker.plan_revision == PLAN_REVISION
                && marker.b0_sha == environment.b0_sha
                && marker.selected_candidate_id == selected_candidate_id
                && marker.freeze_receipt_sha256 == holdout.freeze.receipt_sha256
                && marker.ciphertext_sha256 == holdout.freeze.ciphertext_sha256
                && marker.pre_holdout_attestation_sha256
                    == environment.required_check.attestation_sha256
                && marker.nonce_hex.len() == 64
                && marker
                    .nonce_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                && marker.result == "opened",
            "R51 runner open marker binding drift",
        )?;
        let fresh = custody.revalidate_descriptor()?;
        let named = statat(
            fresh.as_fd(),
            "holdout-open.json",
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(io::Error::from)?;
        require(
            metadata
                == R51DescriptorMetadata {
                    dev: named.st_dev as u64,
                    ino: named.st_ino,
                    owner: named.st_uid.into(),
                    mode: named.st_mode.into(),
                    file_type: FileType::from_raw_mode(named.st_mode),
                },
            "R51 runner open marker namespace changed",
        )?;
        holdout
            .open_marker
            .set(PublishedArtifact {
                path: "holdout-open.json".into(),
                sha256: actual_sha256,
                byte_length: bytes.len() as u64,
            })
            .map_err(|_| invalid_data("R51 runner open marker was reused"))
    }

    fn validate_r51_custody_entry_state(directory: BorrowedFd<'_>) -> io::Result<()> {
        let mut names = Dir::read_from(directory)?
            .map(|entry| {
                entry.map(|entry| OsStr::from_bytes(entry.file_name().to_bytes()).to_owned())
            })
            .collect::<rustix::io::Result<Vec<_>>>()
            .map_err(io::Error::from)?;
        names.retain(|name| name != "." && name != "..");
        let invalid = names.iter().any(|name| {
            let bytes = name.as_bytes();
            name == "holdout-failure.json"
                || name == "holdout-terminal.json"
                || (bytes.starts_with(b".holdout-open.") && bytes.ends_with(b".tmp"))
                || (bytes.starts_with(b".holdout-failure.") && bytes.ends_with(b".tmp"))
                || (bytes.starts_with(b".holdout-terminal.") && bytes.ends_with(b".tmp"))
        });
        require(
            names.iter().any(|name| name == "holdout-open.json") && !invalid,
            "R51 custody entry state is not runner-open",
        )
    }

    fn r51_phase_name(phase: Phase) -> &'static str {
        match phase {
            Phase::CalibrationFreeze => "calibration-freeze",
            Phase::Holdout => "holdout",
        }
    }

    fn r51_contract_path(
        environment: &SelectionEnvironment,
        artifact: &PublishedArtifact,
    ) -> io::Result<String> {
        let artifact_parent = environment
            .artifact
            .parent()
            .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
        artifact_parent
            .join(&artifact.path)
            .strip_prefix(environment.report_dir.join("r51"))
            .map_err(|_| invalid_data("R51 artifact is outside diagnostic root"))?
            .to_str()
            .ok_or_else(|| invalid_data("R51 contract path is not utf-8"))
            .map(str::to_owned)
    }

    fn r51_mask_descriptor(width: u32, height: u32, bytes: &[u8]) -> serde_json::Value {
        debug_assert_eq!(bytes.len(), width as usize * height as usize);
        serde_json::json!({
            "width": width,
            "height": height,
            "stride": width,
            "pixel_encoding": "u8-binary",
            "row_order": "top-to-bottom-left-to-right",
            "bytes_sha256": sha256_hex(&bytes),
        })
    }

    fn r51_rect_quad(rect: [i64; 4]) -> [i64; 8] {
        let [left, top, right, bottom] = rect;
        [left, top, right, top, right, bottom, left, bottom]
    }

    fn r51_target_id_for_rect(
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
        rect: [i64; 4],
    ) -> io::Result<String> {
        let center = (
            (rect[0] + rect[2]) as f64 / 2.0,
            (rect[1] + rect[3]) as f64 / 2.0,
        );
        let matches = schema
            .targets
            .iter()
            .zip(&oracle.targets)
            .filter(|(_, geometry)| rect_contains(geometry.source_roi, center))
            .map(|(target, _)| target.id.clone())
            .collect::<Vec<_>>();
        require(
            matches.len() == 1,
            "R51 detector target ownership is not unique",
        )?;
        Ok(matches[0].clone())
    }

    fn r51_detector_diagnostics(
        environment: &SelectionEnvironment,
        result: &SelectionResult,
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
        diagnostics: &[SourceGateDiagnosticEvent],
        rejection_reason: &Option<String>,
    ) -> io::Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        String,
        Vec<serde_json::Value>,
    )> {
        let mut dimensions = None;
        let mut crop_by_node = HashMap::new();
        for event in diagnostics {
            match event {
                SourceGateDiagnosticEvent::Input { width, height, .. } => {
                    dimensions = Some((*width, *height));
                }
                SourceGateDiagnosticEvent::Crop {
                    node_id, bounds, ..
                } => {
                    crop_by_node.insert(*node_id, *bounds);
                }
                _ => {}
            }
        }
        let (width, height) = dimensions
            .ok_or_else(|| invalid_data("R51 source-gate input diagnostic is missing"))?;
        let mut raw_detector_outputs = Vec::new();
        let mut raw_bits = Vec::<[u32; 8]>::new();
        let mut canonical_lines = Vec::new();
        let mut detector_support_records = Vec::new();
        let phase = r51_phase_name(environment.phase);

        for event in diagnostics {
            let SourceGateDiagnosticEvent::PpSummary {
                node_id,
                raw_detectors,
                canonical_lines: event_lines,
                ..
            } = event
            else {
                continue;
            };
            let occurrence_offset = raw_detector_outputs.len();
            let line_offset = canonical_lines.len();
            let crop = crop_by_node
                .get(node_id)
                .copied()
                .ok_or_else(|| invalid_data("R51 detector crop diagnostic is missing"))?;
            let selection_geometry = diagnostics.iter().find_map(|candidate| match candidate {
                SourceGateDiagnosticEvent::SelectionGeometry {
                    node_id: geometry_node,
                    targets,
                    protected_lines,
                    detector_ownership,
                } if geometry_node == node_id => Some((
                    targets.as_slice(),
                    protected_lines.as_slice(),
                    detector_ownership.as_slice(),
                )),
                _ => None,
            });
            let mut recognition_by_occurrence = HashMap::<usize, Option<serde_json::Value>>::new();
            let mut line_by_occurrence = HashMap::<usize, usize>::new();
            for line in event_lines {
                let recognition = line.recognition.as_ref().map(|recognition| {
                    serde_json::json!({
                        "present": recognition.present,
                        "recognition_class": recognition.recognition_class,
                        "segment_count": recognition.segment_count,
                    })
                });
                let detector_occurrences = line
                    .detector_occurrences
                    .iter()
                    .map(|occurrence| {
                        recognition_by_occurrence
                            .insert(occurrence.occurrence_index, recognition.clone());
                        line_by_occurrence.insert(occurrence.occurrence_index, line.line_index);
                        serde_json::json!({
                            "occurrence_index": occurrence_offset + occurrence.occurrence_index,
                            "canonical_corners_f32_bits": occurrence.canonical_corners_f32_bits,
                        })
                    })
                    .collect::<Vec<_>>();
                canonical_lines.push(serde_json::json!({
                    "line_index": line_offset + line.line_index,
                    "detector_occurrences": detector_occurrences,
                    "recognition": recognition,
                }));
            }
            for detector in raw_detectors {
                let occurrence_index = occurrence_offset + detector.occurrence_index;
                let bits = detector.source_scaled_quad_f32_bits;
                raw_bits.push(bits);
                raw_detector_outputs.push(serde_json::json!({
                    "occurrence_index": occurrence_index,
                    "source_scaled_quad_f32_bits": bits,
                }));
                r51_quad_bits_rect(bits)?;
                let fallback_scene_bits = [
                    (crop[0] as f32 + f32::from_bits(bits[0])).to_bits(),
                    (crop[1] as f32 + f32::from_bits(bits[1])).to_bits(),
                    (crop[0] as f32 + f32::from_bits(bits[2])).to_bits(),
                    (crop[1] as f32 + f32::from_bits(bits[3])).to_bits(),
                    (crop[0] as f32 + f32::from_bits(bits[4])).to_bits(),
                    (crop[1] as f32 + f32::from_bits(bits[5])).to_bits(),
                    (crop[0] as f32 + f32::from_bits(bits[6])).to_bits(),
                    (crop[1] as f32 + f32::from_bits(bits[7])).to_bits(),
                ];
                let ownership = selection_geometry.and_then(|(_, _, ownership)| {
                    ownership
                        .iter()
                        .find(|value| value.occurrence_index == detector.occurrence_index)
                });
                if selection_geometry.is_some() {
                    require(
                        ownership.is_some(),
                        "R51 SelectionGeometry detector ownership is incomplete",
                    )?;
                }
                if let Some(ownership) = ownership {
                    require(
                        ownership.canonical_line_index
                            == line_by_occurrence.get(&detector.occurrence_index).copied(),
                        "R51 detector canonical-line ownership drift",
                    )?;
                }
                let emitted_bits =
                    ownership.map_or(fallback_scene_bits, |value| value.scene_quad_f32_bits);
                let emitted_rect = r51_quad_bits_rect(emitted_bits)?;
                let recognition = recognition_by_occurrence
                    .get(&detector.occurrence_index)
                    .and_then(Option::as_ref);
                let recognition_class = recognition
                    .and_then(|value| value["recognition_class"].as_str())
                    .unwrap_or("missing");
                let (
                    target_id,
                    canonical_assignment,
                    eligible_bits,
                    ownership_verdict,
                    selection_verdict,
                ) = match ownership.map(|value| value.assignment) {
                    Some(SourceGateDetectorAssignmentDiagnostic::Target { target_index }) => {
                        let (targets, _, _) = selection_geometry.expect("ownership has geometry");
                        let geometry = targets
                            .get(target_index)
                            .ok_or_else(|| invalid_data("R51 target assignment index drift"))?;
                        let line_rect = r51_quad_bits_rect(geometry.scene_quad_f32_bits)?;
                        (
                            Some(r51_target_id_for_rect(schema, oracle, line_rect)?),
                            "selected_han",
                            Some(geometry.scene_quad_f32_bits),
                            "unique",
                            "selected",
                        )
                    }
                    Some(SourceGateDetectorAssignmentDiagnostic::Protected { protected_index }) => {
                        let (_, protected, _) = selection_geometry.expect("ownership has geometry");
                        let geometry = protected
                            .get(protected_index)
                            .ok_or_else(|| invalid_data("R51 protected assignment index drift"))?;
                        (
                            None,
                            "preserved_source",
                            Some(geometry.scene_quad_f32_bits),
                            "unique",
                            "preserved",
                        )
                    }
                    Some(SourceGateDetectorAssignmentDiagnostic::Unassigned) | None => (
                        None,
                        "unassigned",
                        None,
                        "unassigned",
                        if rejection_reason.is_some() {
                            "rejected"
                        } else {
                            "preserved"
                        },
                    ),
                };
                let detector_mask = r51_rect_mask(width, height, emitted_rect);
                let line_rect = eligible_bits.map(r51_quad_bits_rect).transpose()?;
                let line_mask = line_rect.map_or_else(
                    || vec![0; detector_mask.len()],
                    |rect| r51_rect_mask(width, height, rect),
                );
                let agreed_mask = detector_mask
                    .iter()
                    .zip(&line_mask)
                    .map(|(detector, line)| detector & line)
                    .collect::<Vec<_>>();
                let line_support_equals_detector = detector_mask == line_mask;
                let agreed_mask_subset = agreed_mask
                    .iter()
                    .zip(&detector_mask)
                    .zip(&line_mask)
                    .all(|((agreed, detector), line)| *agreed <= *detector && *agreed <= *line);
                let mut protected_mask = vec![0; detector_mask.len()];
                if let Some((_, protected, _)) = selection_geometry {
                    for geometry in protected {
                        let mask = r51_rect_mask(
                            width,
                            height,
                            r51_quad_bits_rect(geometry.scene_quad_f32_bits)?,
                        );
                        for (pixel, addition) in protected_mask.iter_mut().zip(mask) {
                            *pixel |= addition;
                        }
                    }
                }
                let protected_support_pixels = detector_mask
                    .iter()
                    .zip(&protected_mask)
                    .filter(|(detector, protected)| **detector != 0 && **protected != 0)
                    .count();
                let preimage = serde_json::json!({
                    "contract": "detector-support-raster-preimage-v1",
                    "plan_revision": PLAN_REVISION,
                    "b0_sha": &environment.b0_sha,
                    "phase": phase,
                    "entry_id": &result.entry_id,
                    "device": &result.derived.actual_device,
                    "candidate_id": &result.candidate_id,
                    "target_id": target_id,
                    "raw_detector": {
                        "index": occurrence_index,
                        "source_scaled_quad_f32_bits": bits,
                        "rect": emitted_rect,
                        "recognition_present": recognition.is_some(),
                        "recognition_class": recognition_class,
                    },
                    "canonical_assignment": canonical_assignment,
                    "emitted_scene_quad": r51_rect_quad(emitted_rect),
                    "eligible_text_line_quad": line_rect.map(r51_rect_quad),
                    "detector_support_mask": r51_mask_descriptor(width, height, &detector_mask),
                    "line_support_mask": r51_mask_descriptor(width, height, &line_mask),
                    "line_support_equals_detector": line_support_equals_detector,
                    "agreed_mask": r51_mask_descriptor(width, height, &agreed_mask),
                    "agreed_mask_subset": agreed_mask_subset,
                    "protected_support_pixels": protected_support_pixels,
                    "unsupported_rotation_selected": !result.derived.selected_rotation_target_ids.is_empty(),
                    "unmatched_selected_nodes": &result.derived.unmatched_selected_node_ids,
                    "ownership_verdict": ownership_verdict,
                    "selection_verdict": selection_verdict,
                    "rejection_reason": rejection_reason,
                });
                let preimage_bytes = canonical_json(&preimage)?;
                detector_support_records.push(serde_json::json!({
                    "preimage": preimage,
                    "canonical_byte_length": preimage_bytes.len(),
                    "sha256": sha256_hex(&preimage_bytes),
                }));
            }
        }
        require(
            raw_detector_outputs.len() == raw_bits.len()
                && raw_detector_outputs.len() == detector_support_records.len(),
            "R51 detector diagnostic completeness drift",
        )?;
        Ok((
            raw_detector_outputs,
            canonical_lines,
            sha256_hex(&canonical_json(&raw_bits)?),
            detector_support_records,
        ))
    }

    fn write_r51_cell_diagnostic(
        environment: &SelectionEnvironment,
        process: &ProcessEvidence,
        result: &SelectionResult,
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
        diagnostics: &[SourceGateDiagnosticEvent],
        bundle_validation_receipt: Option<&PublishedArtifact>,
        coverage_index: Option<&PublishedArtifact>,
    ) -> io::Result<R51TerminalCellResult> {
        let phase = r51_phase_name(environment.phase);
        let device = process.requested_device.as_str();
        let diagnostic_cell_key = format!(
            "{phase}/{}/{device}/{}",
            result.candidate_id, result.entry_id
        );
        let cell_key = if environment.phase == Phase::Holdout {
            format!("{}/{device}", result.entry_id)
        } else {
            diagnostic_cell_key.clone()
        };
        let cell_root = format!(
            "r51/cells/{phase}/{}/{device}/{}",
            result.candidate_id, result.entry_id
        );
        let target_total = schema.targets.len();
        let selected = result.derived.selected_target_ids.len();
        let covered = result
            .derived
            .source_coverage_preflight
            .covered_source_roi_ids
            .len();
        let recall = R51TargetRecall {
            target_total,
            selected,
            covered,
            uncovered: target_total.saturating_sub(covered),
        };
        let rejection_reason = rejection_reason(diagnostics);
        let selection_result = if selected == target_total && target_total != 0 {
            Some("selected".to_owned())
        } else if rejection_reason.is_some() {
            Some("rejected".to_owned())
        } else {
            Some("preserved".to_owned())
        };
        let passed = result.derived.passed;
        let terminal_reason = (!passed).then(|| {
            rejection_reason
                .clone()
                .unwrap_or_else(|| "coverage_failure".into())
        });
        let device_evidence = publish_r51_artifact(
            environment,
            &format!("{cell_root}/device-evidence.json"),
            &canonical_json(process)?,
        )?;
        let artifact_parent = environment
            .artifact
            .parent()
            .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
        let source_log_path =
            artifact_parent.join(&result.execution_evidence.raw_inference_log_relpath);
        let source_log = fs::read(&source_log_path)?;
        require(
            sha256_hex(&source_log) == result.execution_evidence.raw_inference_log_sha256,
            "R51 inference log hash drift",
        )?;
        let log = publish_r51_artifact(
            environment,
            &format!("{cell_root}/inference.log"),
            &source_log,
        )?;
        let (raw_detector_outputs, canonical_lines, raw_detector_hash, support_records) =
            r51_detector_diagnostics(
                environment,
                result,
                schema,
                oracle,
                diagnostics,
                &rejection_reason,
            )?;
        let diagnostic = serde_json::json!({
            "contract": "hanonly-r50-cell-diagnostic-v1",
            "plan_revision": PLAN_REVISION,
            "b0_sha": &environment.b0_sha,
            "calibration_manifest_sha256": &environment.calibration_manifest_sha256,
            "holdout_manifest_sha256": environment
                .r51_formal_custody
                .as_ref()
                .and_then(|custody| custody.holdout.as_ref())
                .map(|holdout| holdout.freeze.manifest_sha256.as_str()),
            "fixture_manifest_sha256": &environment.source_gate_fixture_manifest_sha256,
            "phase": phase,
            "entry_id": &result.entry_id,
            "device": device,
            "candidate_id": &result.candidate_id,
            "state": if passed { "passed" } else { "failed" },
            "selection_result": &selection_result,
            "target_recall": &recall,
            "pp_han_count": result.derived.source_coverage_preflight.pp_han_scalar_count,
            "vl_han_count": result.derived.source_coverage_preflight.vl_expected_han_scalar_count,
            "rejection_reason": &rejection_reason,
            "raw_detector_outputs": raw_detector_outputs,
            "canonical_lines": canonical_lines,
            "raw_detector_count": support_records.len(),
            "raw_detector_f32_bits_multiset_sha256": raw_detector_hash,
            "detector_support_records": support_records,
            "device_evidence_sha256": &device_evidence.sha256,
            "device_evidence_byte_length": device_evidence.byte_length,
            "log_sha256": &log.sha256,
            "log_byte_length": log.byte_length,
            "terminal_reason": &terminal_reason,
            "bundle_validation_receipt_sha256": bundle_validation_receipt.map(|value| value.sha256.as_str()),
            "target_coverage_index_sha256": coverage_index.map(|value| value.sha256.as_str()),
        });
        let diagnostic = publish_r51_artifact(
            environment,
            &format!("{cell_root}/cell-diagnostic.json"),
            &canonical_json(&diagnostic)?,
        )?;
        Ok(R51TerminalCellResult {
            cell_key,
            result: if passed { "pass" } else { "fail-closed" }.into(),
            selection_result,
            target_recall: recall,
            pp_han_count: result.derived.source_coverage_preflight.pp_han_scalar_count,
            vl_han_count: result
                .derived
                .source_coverage_preflight
                .vl_expected_han_scalar_count,
            rejection_reason,
            device_evidence_sha256: device_evidence.sha256.clone(),
            log_sha256: log.sha256.clone(),
            diagnostic_sha256: diagnostic.sha256.clone(),
            target_coverage_index_sha256: coverage_index.map(|value| value.sha256.clone()),
            diagnostic_cell_key,
            phase: phase.into(),
            candidate_id: result.candidate_id.clone(),
            entry_id: result.entry_id.clone(),
            device: device.into(),
            terminal_reason,
            diagnostic_path: r51_contract_path(environment, &diagnostic)?,
            diagnostic_byte_length: diagnostic.byte_length,
            target_coverage_index_path: coverage_index
                .map(|value| r51_contract_path(environment, value))
                .transpose()?,
            target_coverage_index_byte_length: coverage_index.map(|value| value.byte_length),
            device_evidence_path: r51_contract_path(environment, &device_evidence)?,
            device_evidence_byte_length: device_evidence.byte_length,
            log_path: r51_contract_path(environment, &log)?,
            log_byte_length: log.byte_length,
        })
    }

    fn write_r51_calibration_cell_evidence(
        environment: &SelectionEnvironment,
        process: &ProcessEvidence,
        result: &SelectionResult,
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
        diagnostics: &[SourceGateDiagnosticEvent],
    ) -> io::Result<R51TerminalCellResult> {
        require(
            environment.r51_formal_custody.is_some()
                && environment.phase == Phase::CalibrationFreeze
                && result.entry_id == schema.id,
            "invalid R51 calibration cell context",
        )?;
        write_r51_cell_diagnostic(
            environment,
            process,
            result,
            schema,
            oracle,
            diagnostics,
            None,
            None,
        )
    }

    fn write_r51_cell_evidence(
        environment: &SelectionEnvironment,
        process: &ProcessEvidence,
        result: &mut SelectionResult,
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
        diagnostics: &[SourceGateDiagnosticEvent],
        supports: &CellSupportEvidence,
        bundle_validation_receipt: &PublishedArtifact,
    ) -> io::Result<R51TerminalCellResult> {
        require(
            environment.r51_formal_custody.is_some()
                && environment.phase == Phase::Holdout
                && result.entry_id == schema.id,
            "invalid R51 formal cell context",
        )?;
        let custody = &environment
            .r51_formal_custody
            .as_ref()
            .and_then(|custody| custody.holdout.as_ref())
            .ok_or_else(|| invalid_data("R51 holdout custody is unavailable"))?
            .freeze;
        let device = process.requested_device.as_str();
        let cell_key = format!(
            "holdout/{}/{device}/{}",
            result.candidate_id, result.entry_id
        );
        let cell_root = format!(
            "r51/cells/holdout/{}/{device}/{}",
            result.candidate_id, result.entry_id
        );
        let target_total = schema.targets.len();
        let selected = result.derived.selected_target_ids.len();
        let covered = result
            .derived
            .source_coverage_preflight
            .covered_source_roi_ids
            .len();
        let recall = R51TargetRecall {
            target_total,
            selected,
            covered,
            uncovered: target_total.saturating_sub(covered),
        };
        let rejection_reason = rejection_reason(diagnostics);
        let selection_result = if selected == target_total && target_total != 0 {
            Some("selected".to_owned())
        } else if rejection_reason.is_some() {
            Some("rejected".to_owned())
        } else {
            Some("preserved".to_owned())
        };
        let captured = serde_json::json!({
            "contract": "hanonly-r51-cell-capture-v1",
            "plan_revision": PLAN_REVISION,
            "b0_sha": &environment.b0_sha,
            "cell_key": &cell_key,
            "manifest_sha256": &environment.visual_manifest_sha256,
            "selection_result": &result,
            "target_recall": &recall,
            "pp_han_count": result.derived.source_coverage_preflight.pp_han_scalar_count,
            "vl_han_count": result.derived.source_coverage_preflight.vl_expected_han_scalar_count,
            "rejection_reason": &rejection_reason,
            "log_path": &result.execution_evidence.raw_inference_log_relpath,
            "log_sha256": &result.execution_evidence.raw_inference_log_sha256,
            "source_gate_diagnostics": diagnostics,
        });
        let captured = publish_r51_artifact(
            environment,
            &format!("{cell_root}/selection-result.json"),
            &canonical_json(&captured)?,
        )?;

        let mut proof_records = Vec::with_capacity(schema.targets.len());
        let mut coverage_passed = true;
        let page_len = usize::try_from(u64::from(supports.width) * u64::from(supports.height))
            .map_err(|_| invalid_data("R51 support raster length overflow"))?;
        for (target, oracle_target) in schema.targets.iter().zip(&oracle.targets) {
            let selected_support = supports
                .selected_by_target
                .get(&target.id)
                .cloned()
                .unwrap_or_else(|| vec![0; page_len]);
            let downstream_support = supports
                .downstream_by_target
                .get(&target.id)
                .cloned()
                .unwrap_or_else(|| vec![0; page_len]);
            require(
                selected_support.len() == page_len
                    && downstream_support.len() == page_len
                    && selected_support.iter().all(|pixel| matches!(pixel, 0 | 1))
                    && downstream_support
                        .iter()
                        .all(|pixel| matches!(pixel, 0 | 1)),
                "R51 support raster is not complete binary page geometry",
            )?;
            let selected_raster = publish_r51_artifact(
                environment,
                &format!("{cell_root}/{}.selected-support.bin", target.id),
                &selected_support,
            )?;
            let downstream_raster = publish_r51_artifact(
                environment,
                &format!("{cell_root}/{}.downstream-support.bin", target.id),
                &downstream_support,
            )?;
            let oracle_mask = page_oracle_mask(
                supports.width,
                supports.height,
                oracle_target.edit_roi,
                &oracle_target.delta_mask,
            )?;
            let oracle_foreground_pixels = foreground_count(&oracle_mask);
            let selected_covered_pixels = intersection_count(&oracle_mask, &selected_support)?;
            let downstream_covered_pixels = intersection_count(&oracle_mask, &downstream_support)?;
            let protected_overlap_pixels = protected_overlap_count(
                &downstream_support,
                supports.width,
                &oracle.protected_rois,
            );
            let missing_selected_pixels =
                oracle_foreground_pixels.saturating_sub(selected_covered_pixels);
            let missing_downstream_pixels =
                oracle_foreground_pixels.saturating_sub(downstream_covered_pixels);
            let target_selected = result
                .derived
                .selected_target_ids
                .iter()
                .any(|id| id == &target.id);
            let passed = target_selected
                && missing_selected_pixels == 0
                && missing_downstream_pixels == 0
                && protected_overlap_pixels == 0;
            coverage_passed &= passed;
            let proof = R51TargetCoverageProof {
                contract: "hanonly-r51-target-coverage-proof-v1",
                plan_revision: PLAN_REVISION,
                b0_sha: &environment.b0_sha,
                cell_key: &cell_key,
                entry_id: &result.entry_id,
                target_id: &target.id,
                oracle_mask_raw_sha256: target.erase_source_ink_mask_sha256.clone(),
                oracle_mask_normalized_sha256: r51_binary_mask_sha256(
                    supports.width,
                    supports.height,
                    &oracle_mask,
                ),
                page_width: supports.width,
                page_height: supports.height,
                support_stride_bytes: supports.width,
                selected_support_relpath: r51_contract_path(environment, &selected_raster)?,
                selected_support_byte_length: selected_raster.byte_length,
                selected_support_sha256: selected_raster.sha256,
                downstream_support_relpath: r51_contract_path(environment, &downstream_raster)?,
                downstream_support_byte_length: downstream_raster.byte_length,
                downstream_support_sha256: downstream_raster.sha256,
                oracle_foreground_pixels,
                selected_support_foreground_pixels: foreground_count(&selected_support),
                downstream_support_foreground_pixels: foreground_count(&downstream_support),
                selected_covered_pixels,
                downstream_covered_pixels,
                missing_selected_pixels,
                missing_downstream_pixels,
                protected_overlap_pixels,
                target_selected,
                result: if passed { "pass" } else { "fail-closed" },
            };
            let proof = publish_r51_artifact(
                environment,
                &format!("{cell_root}/{}.coverage-proof.json", target.id),
                &canonical_json(&proof)?,
            )?;
            proof_records.push(R51TargetCoverageIndexRecord {
                entry_id: result.entry_id.clone(),
                target_id: target.id.clone(),
                proof_path: r51_contract_path(environment, &proof)?,
                proof_sha256: proof.sha256,
                proof_byte_length: proof.byte_length,
            });
        }
        proof_records.sort_by(|left, right| {
            (&left.entry_id, &left.target_id).cmp(&(&right.entry_id, &right.target_id))
        });
        let coverage_index = R51TargetCoverageIndex {
            contract: "hanonly-r51-target-coverage-index-v1",
            plan_revision: PLAN_REVISION,
            b0_sha: &environment.b0_sha,
            cell_key: &cell_key,
            manifest_sha256: &custody.manifest_sha256,
            oracle_sha256: &custody.oracle_sha256,
            hashes_sha256: &custody.hashes_sha256,
            records: proof_records,
        };
        let coverage_index = publish_r51_artifact(
            environment,
            &format!("{cell_root}/target-coverage-index.json"),
            &canonical_json(&coverage_index)?,
        )?;
        result.derived.passed &= coverage_passed;
        let _ = (captured, recall, selection_result, rejection_reason);
        write_r51_cell_diagnostic(
            environment,
            process,
            result,
            schema,
            oracle,
            diagnostics,
            Some(bundle_validation_receipt),
            Some(&coverage_index),
        )
    }

    fn rejection_reason(diagnostics: &[SourceGateDiagnosticEvent]) -> Option<String> {
        diagnostics.iter().find_map(|event| match event {
            SourceGateDiagnosticEvent::Decision {
                decision:
                    SourceGateDecision::RejectedBeforeVl { reason }
                    | SourceGateDecision::RejectedAfterVl { reason },
                ..
            } => serde_json::to_value(reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned)),
            SourceGateDiagnosticEvent::Decision {
                decision: SourceGateDecision::InvalidCandidateGeometry,
                ..
            } => Some("invalid_candidate_geometry".into()),
            SourceGateDiagnosticEvent::Decision {
                decision: SourceGateDecision::VlBatchError,
                ..
            } => Some("vl_batch_error".into()),
            _ => None,
        })
    }

    fn page_oracle_mask(
        width: u32,
        height: u32,
        roi: ValidatedHalfOpenRect,
        local: &[u8],
    ) -> io::Result<Vec<u8>> {
        let roi_width = usize::try_from(roi.right - roi.left)
            .map_err(|_| invalid_data("R51 oracle width overflow"))?;
        let roi_height = usize::try_from(roi.bottom - roi.top)
            .map_err(|_| invalid_data("R51 oracle height overflow"))?;
        require(
            local.len() == roi_width * roi_height,
            "R51 oracle mask length drift",
        )?;
        let mut page = vec![0; width as usize * height as usize];
        for y in 0..roi_height {
            for x in 0..roi_width {
                page[(roi.top as usize + y) * width as usize + roi.left as usize + x] =
                    u8::from(local[y * roi_width + x] != 0);
            }
        }
        Ok(page)
    }

    fn foreground_count(mask: &[u8]) -> u64 {
        mask.iter().map(|pixel| u64::from(*pixel != 0)).sum()
    }

    fn intersection_count(left: &[u8], right: &[u8]) -> io::Result<u64> {
        require(left.len() == right.len(), "R51 mask dimensions drift")?;
        Ok(left
            .iter()
            .zip(right)
            .map(|(left, right)| u64::from(*left != 0 && *right != 0))
            .sum())
    }

    fn protected_overlap_count(
        support: &[u8],
        width: u32,
        protected_rois: &[ValidatedHalfOpenRect],
    ) -> u64 {
        protected_rois
            .iter()
            .map(|roi| {
                (roi.top..roi.bottom)
                    .flat_map(|y| (roi.left..roi.right).map(move |x| (x, y)))
                    .map(|(x, y)| u64::from(support[y as usize * width as usize + x as usize] != 0))
                    .sum::<u64>()
            })
            .sum()
    }

    fn r51_binary_mask_sha256(width: u32, height: u32, mask: &[u8]) -> String {
        let mut preimage = b"hanonly-r51-binary-mask-v1\0".to_vec();
        preimage.extend_from_slice(&width.to_be_bytes());
        preimage.extend_from_slice(&height.to_be_bytes());
        preimage.extend_from_slice(mask);
        sha256_hex(&preimage)
    }

    fn publish_r51_artifact(
        environment: &SelectionEnvironment,
        suffix: &str,
        bytes: &[u8],
    ) -> io::Result<PublishedArtifact> {
        require(
            !bytes.is_empty()
                && !suffix.starts_with('/')
                && suffix
                    .split('/')
                    .all(|component| !matches!(component, "" | "." | "..")),
            "invalid R51 artifact",
        )?;
        let report_relative = environment
            .report_dir
            .strip_prefix(&environment.evidence_root)
            .map_err(|_| invalid_data("R51 report directory escaped evidence root"))?;
        let suffix_path = Path::new(suffix);
        let parent_relative = report_relative.join(
            suffix_path
                .parent()
                .ok_or_else(|| invalid_data("R51 artifact has no parent"))?,
        );
        let file_name = suffix_path
            .file_name()
            .ok_or_else(|| invalid_data("R51 artifact name is invalid"))?;
        let published = publish_descriptor_relative(
            &environment.evidence_root,
            &parent_relative,
            file_name,
            bytes,
        )?;
        let path = environment.report_dir.join(suffix);
        let sha256 = published.sha256;
        let artifact_parent = environment
            .artifact
            .parent()
            .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
        let relative = path
            .strip_prefix(artifact_parent)
            .map_err(|_| invalid_data("R51 artifact is outside artifact parent"))?
            .to_str()
            .ok_or_else(|| invalid_data("R51 artifact path is not utf-8"))?
            .to_owned();
        Ok(PublishedArtifact {
            path: relative,
            sha256,
            byte_length: bytes.len() as u64,
        })
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct R51DescriptorMetadata {
        dev: u64,
        ino: u64,
        owner: u64,
        mode: u32,
        file_type: FileType,
    }

    struct R51HeldDirectory {
        slash: OwnedFd,
        descriptor: OwnedFd,
        absolute_components: Vec<OsString>,
        chain: Vec<R51DescriptorMetadata>,
    }

    struct R51PublishedDescriptor {
        sha256: String,
    }

    impl R51HeldDirectory {
        fn open(path: &Path) -> io::Result<Self> {
            require_absolute_canonical(path)?;
            let slash = open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(io::Error::from)?;
            let absolute_components = path
                .components()
                .skip(1)
                .map(|component| component.as_os_str().to_owned())
                .collect::<Vec<_>>();
            let (descriptor, chain) =
                r51_walk_directories(slash.as_fd(), &absolute_components, false, false)?;
            let root = chain
                .last()
                .ok_or_else(|| invalid_data("R51 evidence root is unavailable"))?;
            require(
                root.file_type.is_dir()
                    && root.owner == effective_owner()?
                    && root.mode & 0o7777 == 0o700,
                "invalid R51 evidence root",
            )?;
            Ok(Self {
                slash,
                descriptor,
                absolute_components,
                chain,
            })
        }

        fn open_or_create_child(&self, relative: &Path) -> io::Result<R51HeldDirectoryChild> {
            let components = relative
                .components()
                .map(|component| component.as_os_str().to_owned())
                .collect::<Vec<_>>();
            let (descriptor, chain) =
                r51_walk_directories(self.descriptor.as_fd(), &components, true, true)?;
            fsync(&self.descriptor).map_err(io::Error::from)?;
            Ok(R51HeldDirectoryChild {
                descriptor,
                components,
                chain,
            })
        }

        fn revalidate_descriptor(&self) -> io::Result<OwnedFd> {
            let (fresh, chain) =
                r51_walk_directories(self.slash.as_fd(), &self.absolute_components, false, false)?;
            require(chain == self.chain, "R51 evidence root namespace changed")?;
            Ok(fresh)
        }

        fn revalidate_child(&self, child: &R51HeldDirectoryChild) -> io::Result<OwnedFd> {
            self.revalidate_descriptor()?;
            let (fresh, chain) =
                r51_walk_directories(self.descriptor.as_fd(), &child.components, false, true)?;
            require(chain == child.chain, "R51 publication namespace changed")?;
            Ok(fresh)
        }
    }

    struct R51HeldDirectoryChild {
        descriptor: OwnedFd,
        components: Vec<OsString>,
        chain: Vec<R51DescriptorMetadata>,
    }

    fn r51_walk_directories(
        start: BorrowedFd<'_>,
        components: &[OsString],
        create: bool,
        require_secure: bool,
    ) -> io::Result<(OwnedFd, Vec<R51DescriptorMetadata>)> {
        let mut current = openat(
            start,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?;
        let mut chain = Vec::with_capacity(components.len());
        for component in components {
            require(
                component != OsStr::new("")
                    && component != OsStr::new(".")
                    && component != OsStr::new(".."),
                "invalid R51 directory component",
            )?;
            let next = match openat(
                current.as_fd(),
                component,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(value) => value,
                Err(error) if create && error == rustix::io::Errno::NOENT => {
                    mkdirat(current.as_fd(), component, Mode::from_raw_mode(0o700))
                        .map_err(io::Error::from)?;
                    fsync(&current).map_err(io::Error::from)?;
                    openat(
                        current.as_fd(),
                        component,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(io::Error::from)?
                }
                Err(error) => return Err(error.into()),
            };
            let metadata = r51_descriptor_metadata(next.as_fd())?;
            require(
                metadata.file_type.is_dir(),
                "R51 path component is not a directory",
            )?;
            if require_secure {
                require(
                    metadata.owner == effective_owner()?,
                    "R51 publication directory owner mismatch",
                )?;
                require(
                    metadata.mode & 0o7777 == 0o700,
                    "R51 publication directory mode mismatch",
                )?;
            }
            chain.push(metadata);
            current = next;
        }
        Ok((current, chain))
    }

    fn r51_descriptor_metadata(fd: BorrowedFd<'_>) -> io::Result<R51DescriptorMetadata> {
        let stat = fstat(fd).map_err(io::Error::from)?;
        Ok(R51DescriptorMetadata {
            dev: stat.st_dev as u64,
            ino: stat.st_ino,
            owner: stat.st_uid.into(),
            mode: stat.st_mode.into(),
            file_type: FileType::from_raw_mode(stat.st_mode),
        })
    }

    fn publish_descriptor_relative(
        root: &Path,
        parent_relative: &Path,
        final_name: &OsStr,
        bytes: &[u8],
    ) -> io::Result<R51PublishedDescriptor> {
        require(!bytes.is_empty(), "R51 publication bytes are empty")?;
        let root = R51HeldDirectory::open(root)?;
        let parent = root.open_or_create_child(parent_relative)?;
        let sha256 = sha256_hex(bytes);
        let temporary = OsString::from(format!(".{}.{}.tmp", final_name.to_string_lossy(), sha256));
        r51_require_absent(parent.descriptor.as_fd(), final_name)?;
        r51_require_absent(parent.descriptor.as_fd(), &temporary)?;
        let result = (|| {
            let descriptor = openat(
                parent.descriptor.as_fd(),
                &temporary,
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(io::Error::from)?;
            let mut temporary_file = fs::File::from(descriptor);
            temporary_file.write_all(bytes)?;
            temporary_file.sync_all()?;
            linkat(
                parent.descriptor.as_fd(),
                &temporary,
                parent.descriptor.as_fd(),
                final_name,
                AtFlags::empty(),
            )
            .map_err(io::Error::from)?;
            let final_descriptor = openat(
                parent.descriptor.as_fd(),
                final_name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(io::Error::from)?;
            let temporary_metadata = r51_descriptor_metadata(temporary_file.as_fd())?;
            let final_metadata = r51_descriptor_metadata(final_descriptor.as_fd())?;
            let mut final_file = fs::File::from(final_descriptor);
            let mut actual = Vec::new();
            final_file.read_to_end(&mut actual)?;
            require(
                temporary_metadata == final_metadata
                    && final_metadata.file_type.is_file()
                    && final_metadata.owner == effective_owner()?
                    && final_metadata.mode & 0o7777 == 0o600
                    && actual == bytes,
                "R51 artifact publication verification failed",
            )?;
            final_file.sync_all()?;
            fsync(&parent.descriptor).map_err(io::Error::from)?;
            unlinkat(parent.descriptor.as_fd(), &temporary, AtFlags::empty())
                .map_err(io::Error::from)?;
            fsync(&parent.descriptor).map_err(io::Error::from)?;
            let fresh_parent = root.revalidate_child(&parent)?;
            let named = statat(fresh_parent.as_fd(), final_name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(io::Error::from)?;
            require(
                r51_descriptor_metadata(final_file.as_fd())?
                    == R51DescriptorMetadata {
                        dev: named.st_dev as u64,
                        ino: named.st_ino,
                        owner: named.st_uid.into(),
                        mode: named.st_mode.into(),
                        file_type: FileType::from_raw_mode(named.st_mode),
                    },
                "R51 artifact final namespace changed",
            )
        })();
        if result.is_err()
            && statat(
                parent.descriptor.as_fd(),
                final_name,
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .is_err()
        {
            let _ = unlinkat(parent.descriptor.as_fd(), &temporary, AtFlags::empty());
            let _ = fsync(&parent.descriptor);
        }
        result?;
        Ok(R51PublishedDescriptor { sha256 })
    }

    fn r51_require_absent(parent: BorrowedFd<'_>, name: &OsStr) -> io::Result<()> {
        match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Ok(_) => Err(invalid_data("R51 create-new publication collision")),
            Err(error) => Err(error.into()),
        }
    }

    fn r51_diagnostic_record(cell: &R51TerminalCellResult, state: &str) -> serde_json::Value {
        serde_json::json!({
            "cell_key": &cell.diagnostic_cell_key,
            "phase": &cell.phase,
            "candidate_id": &cell.candidate_id,
            "entry_id": &cell.entry_id,
            "device": &cell.device,
            "state": state,
            "diagnostic_path": &cell.diagnostic_path,
            "diagnostic_sha256": &cell.diagnostic_sha256,
            "diagnostic_byte_length": cell.diagnostic_byte_length,
            "selection_result": &cell.selection_result,
            "target_recall": &cell.target_recall,
            "pp_han_count": cell.pp_han_count,
            "vl_han_count": cell.vl_han_count,
            "rejection_reason": &cell.rejection_reason,
            "device_evidence_path": &cell.device_evidence_path,
            "device_evidence_sha256": &cell.device_evidence_sha256,
            "device_evidence_byte_length": cell.device_evidence_byte_length,
            "log_path": &cell.log_path,
            "log_sha256": &cell.log_sha256,
            "log_byte_length": cell.log_byte_length,
            "terminal_reason": if state == "captured_unclassified" {
                None
            } else {
                cell.terminal_reason.as_deref()
            },
            "target_coverage_index_path": &cell.target_coverage_index_path,
            "target_coverage_index_sha256": &cell.target_coverage_index_sha256,
            "target_coverage_index_byte_length": cell.target_coverage_index_byte_length,
        })
    }

    fn write_r51_cell_transitions(
        environment: &SelectionEnvironment,
        cells: &[R51TerminalCellResult],
        mut generation: u64,
        mut previous: Option<PublishedArtifact>,
        records: &mut Vec<serde_json::Value>,
        calibration_manifest_sha256: &str,
        holdout_manifest_sha256: Option<&str>,
        bundle_validation_receipt: Option<&PublishedArtifact>,
    ) -> io::Result<PublishedArtifact> {
        for cell in cells {
            generation += 1;
            records.push(r51_diagnostic_record(cell, "captured_unclassified"));
            records
                .sort_by(|left, right| left["cell_key"].as_str().cmp(&right["cell_key"].as_str()));
            previous = Some(write_r51_diagnostic_generation(
                environment,
                generation,
                previous.as_ref(),
                calibration_manifest_sha256,
                holdout_manifest_sha256,
                bundle_validation_receipt,
                records,
            )?);

            generation += 1;
            let terminal = records
                .iter_mut()
                .find(|record| record["cell_key"] == cell.diagnostic_cell_key)
                .ok_or_else(|| invalid_data("R51 diagnostic record is missing"))?;
            *terminal = r51_diagnostic_record(
                cell,
                if cell.result == "pass" {
                    "passed"
                } else {
                    "failed"
                },
            );
            previous = Some(write_r51_diagnostic_generation(
                environment,
                generation,
                previous.as_ref(),
                calibration_manifest_sha256,
                holdout_manifest_sha256,
                bundle_validation_receipt,
                records,
            )?);
        }
        previous.ok_or_else(|| invalid_data("R51 diagnostic chain is empty"))
    }

    fn write_r51_calibration_diagnostic_generations(
        environment: &SelectionEnvironment,
        formal: &R51FormalRunEvidence,
    ) -> io::Result<PublishedArtifact> {
        require(
            environment.phase == Phase::CalibrationFreeze
                && formal.bundle_validation_receipt.is_none()
                && formal.first_failed_cell.is_none()
                && formal.cells.len() == 32,
            "R51 calibration diagnostic matrix drift",
        )?;
        let expected = candidates_schema()
            .into_iter()
            .flat_map(|candidate| {
                ["cpu", "metal"].into_iter().flat_map(move |device| {
                    r51_entry_ids('c').into_iter().map({
                        let candidate = candidate.id.clone();
                        move |entry| format!("calibration-freeze/{candidate}/{device}/{entry}")
                    })
                })
            })
            .collect::<HashSet<_>>();
        require(
            formal
                .cells
                .iter()
                .map(|cell| cell.diagnostic_cell_key.clone())
                .collect::<HashSet<_>>()
                == expected,
            "R51 calibration diagnostic identities drift",
        )?;
        let mut cells = formal.cells.clone();
        cells.sort_by(|left, right| left.diagnostic_cell_key.cmp(&right.diagnostic_cell_key));
        write_r51_cell_transitions(
            environment,
            &cells,
            0,
            None,
            &mut Vec::new(),
            &environment.visual_manifest_sha256,
            None,
            None,
        )
    }

    fn read_r51_calibration_terminal(
        environment: &SelectionEnvironment,
        calibration_manifest_sha256: &str,
    ) -> io::Result<(PublishedArtifact, Vec<serde_json::Value>)> {
        let path = environment
            .report_dir
            .join("r51/diagnostic-index.generations/00000064.json");
        let bytes = fs::read(&path)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        require(
            canonical_json(&value)? == bytes
                && value["contract"] == "hanonly-r50-diagnostic-index-v1"
                && value["generation"] == 64
                && value["expected_cell_count"] == 32
                && value["calibration_manifest_sha256"] == calibration_manifest_sha256
                && value["holdout_manifest_sha256"].is_null()
                && value["bundle_validation_receipt_path"].is_null()
                && value["bundle_validation_receipt_sha256"].is_null()
                && value["bundle_validation_receipt_byte_length"].is_null(),
            "R51 frozen calibration diagnostic index drift",
        )?;
        let records = value["records"]
            .as_array()
            .cloned()
            .ok_or_else(|| invalid_data("R51 calibration diagnostic records are invalid"))?;
        require(
            records.len() == 32
                && records.iter().all(|record| {
                    record["phase"] == "calibration-freeze"
                        && matches!(record["state"].as_str(), Some("passed" | "failed"))
                }),
            "R51 calibration diagnostic terminal records drift",
        )?;
        let artifact_parent = environment
            .artifact
            .parent()
            .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
        let relative = path
            .strip_prefix(artifact_parent)
            .map_err(|_| invalid_data("R51 calibration index escaped artifact parent"))?
            .to_str()
            .ok_or_else(|| invalid_data("R51 calibration index path is not utf-8"))?
            .to_owned();
        Ok((
            PublishedArtifact {
                path: relative,
                sha256: sha256_hex(&bytes),
                byte_length: bytes.len() as u64,
            },
            records,
        ))
    }

    fn write_r51_diagnostic_generations(
        environment: &SelectionEnvironment,
        selected_candidate_id: &str,
        calibration_manifest_sha256: &str,
        formal: &R51FormalRunEvidence,
    ) -> io::Result<PublishedArtifact> {
        let expected = r51_entry_ids('h')
            .into_iter()
            .flat_map(|entry| {
                ["cpu", "metal"]
                    .into_iter()
                    .map(move |device| format!("{entry}/{device}"))
            })
            .collect::<Vec<_>>();
        require(
            !formal.cells.is_empty()
                && formal.cells.len() <= expected.len()
                && formal
                    .cells
                    .iter()
                    .zip(&expected)
                    .all(|(cell, expected)| &cell.cell_key == expected)
                && formal.cells.iter().all(|cell| {
                    cell.diagnostic_cell_key
                        == format!(
                            "holdout/{selected_candidate_id}/{}/{}",
                            cell.device, cell.entry_id
                        )
                }),
            "R51 formal cells are not an exact ordered prefix",
        )?;
        let first_failure = formal.cells.iter().position(|cell| cell.result != "pass");
        require(
            match (first_failure, formal.first_failed_cell.as_deref()) {
                (None, None) => formal.cells.len() == expected.len(),
                (Some(index), Some(key)) => {
                    index + 1 == formal.cells.len() && formal.cells[index].cell_key == key
                }
                _ => false,
            },
            "R51 formal first-failure boundary drift",
        )?;
        let bundle = formal
            .bundle_validation_receipt
            .as_ref()
            .ok_or_else(|| invalid_data("R51 bundle validation receipt is missing"))?;
        let custody = environment
            .r51_formal_custody
            .as_ref()
            .ok_or_else(|| invalid_data("R51 formal custody is not enabled"))?;
        let holdout = custody
            .holdout
            .as_ref()
            .ok_or_else(|| invalid_data("R51 holdout custody is unavailable"))?;
        let open_marker = holdout
            .open_marker
            .get()
            .ok_or_else(|| invalid_data("R51 runner open marker was not validated"))?;
        let (previous, mut records) =
            read_r51_calibration_terminal(environment, calibration_manifest_sha256)?;
        let terminal_generation = 64 + formal.cells.len() as u64 * 2;
        let terminal_generation_artifact = write_r51_cell_transitions(
            environment,
            &formal.cells,
            64,
            Some(previous),
            &mut records,
            calibration_manifest_sha256,
            Some(&holdout.freeze.manifest_sha256),
            Some(bundle),
        )?;
        let generation_bytes = fs::read(
            environment
                .artifact
                .parent()
                .ok_or_else(|| invalid_data("selection artifact has no parent"))?
                .join(&terminal_generation_artifact.path),
        )?;
        let terminal_index =
            publish_r51_artifact(environment, "r51/diagnostic-index.json", &generation_bytes)?;
        require(
            records.len() == 32 + formal.cells.len()
                && terminal_generation == 64 + formal.cells.len() as u64 * 2,
            "R51 terminal diagnostic count drift",
        )?;
        let unexecuted_cell_keys = expected[formal.cells.len()..].to_vec();
        let all_cells_passed =
            formal.cells.len() == expected.len() && formal.first_failed_cell.is_none();
        let bundle_path = r51_contract_path(environment, bundle)?;
        let terminal_index_path = r51_contract_path(environment, &terminal_index)?;
        let completion_summary = R51CompletionSummary {
            contract: "hanonly-r51-b0-completion-summary-v1",
            plan_revision: PLAN_REVISION,
            b0_sha: &environment.b0_sha,
            selected_candidate_id,
            freeze_receipt_sha256: &holdout.freeze.receipt_sha256,
            open_marker_sha256: &open_marker.sha256,
            ciphertext_sha256: &holdout.freeze.ciphertext_sha256,
            pre_holdout_attestation_sha256: &environment.required_check.attestation_sha256,
            holdout_manifest_sha256: &holdout.freeze.manifest_sha256,
            bundle_validation_receipt_path: &bundle_path,
            bundle_validation_receipt_sha256: &bundle.sha256,
            bundle_validation_receipt_byte_length: bundle.byte_length,
            terminal_diagnostic_index_path: &terminal_index_path,
            terminal_diagnostic_index_sha256: &terminal_index.sha256,
            terminal_diagnostic_index_byte_length: terminal_index.byte_length,
            cell_results: &formal.cells,
            first_failed_cell: formal.first_failed_cell.as_deref(),
            unexecuted_cell_keys,
            all_cells_terminated: all_cells_passed,
            all_cells_passed,
            failure_kind: (!all_cells_passed).then_some("cell_failure"),
            result: if all_cells_passed {
                "pass"
            } else {
                "fail-closed"
            },
        };
        let summary = publish_r51_artifact(
            environment,
            "r51/completion-summary.json",
            &canonical_json(&completion_summary)?,
        )?;
        println!("{}", r51_completion_summary_stdout_line(&summary)?);
        Ok(terminal_index)
    }

    fn r51_completion_summary_stdout_line(summary: &PublishedArtifact) -> io::Result<String> {
        let binding = String::from_utf8(canonical_json(summary)?)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        Ok(format!("{R51_COMPLETION_SUMMARY_STDOUT_PREFIX}{binding}"))
    }

    fn write_r51_diagnostic_generation(
        environment: &SelectionEnvironment,
        generation: u64,
        previous: Option<&PublishedArtifact>,
        calibration_manifest_sha256: &str,
        holdout_manifest_sha256: Option<&str>,
        bundle_validation_receipt: Option<&PublishedArtifact>,
        records: &[serde_json::Value],
    ) -> io::Result<PublishedArtifact> {
        let bundle_path = bundle_validation_receipt
            .map(|value| r51_contract_path(environment, value))
            .transpose()?;
        let index = serde_json::json!({
            "contract": "hanonly-r50-diagnostic-index-v1",
            "plan_revision": PLAN_REVISION,
            "b0_sha": &environment.b0_sha,
            "calibration_manifest_sha256": calibration_manifest_sha256,
            "holdout_manifest_sha256": holdout_manifest_sha256,
            "fixture_manifest_sha256": &environment.source_gate_fixture_manifest_sha256,
            "bundle_validation_receipt_path": bundle_path,
            "bundle_validation_receipt_sha256": bundle_validation_receipt.map(|value| value.sha256.as_str()),
            "bundle_validation_receipt_byte_length": bundle_validation_receipt.map(|value| value.byte_length),
            "generation": generation,
            "previous_index_path": previous.map(|_| format!(
                "diagnostic-index.generations/{:08}.json",
                generation - 1
            )),
            "previous_index_sha256": previous.map(|value| value.sha256.as_str()),
            "previous_index_byte_length": previous.map(|value| value.byte_length),
            "expected_cell_count": records.len(),
            "records": records,
        });
        publish_r51_artifact(
            environment,
            &format!("r51/diagnostic-index.generations/{generation:08}.json"),
            &canonical_json(&index)?,
        )
    }

    fn sha256_file(path: &Path) -> io::Result<String> {
        Ok(sha256_hex(&fs::read(path)?))
    }

    fn runtime_library_hashes(runtime: &RuntimeManager) -> io::Result<BTreeMap<String, String>> {
        let mut hashes = BTreeMap::new();
        for entry in fs::read_dir(runtime.llama_directory().map_err(io::Error::other)?)? {
            let path = entry?.path();
            if path.is_file() {
                let canonical = fs::canonicalize(&path)?;
                hashes.insert(
                    canonical.to_string_lossy().into_owned(),
                    sha256_file(&canonical)?,
                );
            }
        }
        require(!hashes.is_empty(), "llama runtime library set is empty")?;
        Ok(hashes)
    }

    fn enumerated_devices() -> io::Result<Vec<EnumeratedDevice>> {
        list_llama_ggml_backend_devices()
            .into_iter()
            .map(|device| {
                Ok(EnumeratedDevice {
                    index: u32::try_from(device.index)
                        .map_err(|_| invalid_data("device index overflow"))?,
                    name: device.name,
                    description: device.description,
                    backend: device.backend,
                    device_type: device_type(device.device_type).into(),
                })
            })
            .collect()
    }

    fn loaded_model_devices(
        enumerated: &[EnumeratedDevice],
        buffers: &BTreeMap<String, u64>,
    ) -> io::Result<Vec<LoadedModelDevice>> {
        buffers
            .iter()
            .filter(|(_, bytes)| **bytes > 0)
            .enumerate()
            .map(|(ordinal, (backend, _))| {
                let device = enumerated
                    .iter()
                    .find(|device| canonical_device_backend(&device.backend) == Some(backend))
                    .ok_or_else(|| invalid_data("loaded backend was not enumerated"))?;
                Ok(LoadedModelDevice {
                    model_device_ordinal: ordinal as u32,
                    name: if device.name.is_empty() {
                        device.description.clone()
                    } else {
                        device.name.clone()
                    },
                    backend: backend.clone(),
                    device_type: device.device_type.clone(),
                })
            })
            .collect()
    }

    fn canonical_device_backend(backend: &str) -> Option<&'static str> {
        let lower = backend.to_ascii_lowercase();
        if lower.contains("metal") || lower.contains("mtl") {
            Some("Metal")
        } else if lower.contains("cpu") {
            Some("CPU")
        } else {
            None
        }
    }

    fn device_type(device_type: LlamaBackendDeviceType) -> &'static str {
        match device_type {
            LlamaBackendDeviceType::Cpu => "cpu",
            LlamaBackendDeviceType::Accelerator => "accelerator",
            LlamaBackendDeviceType::Gpu => "gpu",
            LlamaBackendDeviceType::IntegratedGpu => "integrated_gpu",
            LlamaBackendDeviceType::Unknown => "unknown",
        }
    }

    fn select_smallest_all_pass(
        results: &[SelectionResult],
        entry_ids: &[String],
    ) -> io::Result<String> {
        let mut failures = Vec::new();
        for candidate in candidates_schema() {
            let cells = results
                .iter()
                .filter(|result| result.candidate_id == candidate.id)
                .collect::<Vec<_>>();
            let expected_cells = entry_ids.len() * 2;
            if cells.len() != expected_cells {
                failures.push(format!(
                    "{}: incomplete={}/{}",
                    candidate.id,
                    cells.len(),
                    expected_cells
                ));
                continue;
            }
            let failed = cells
                .iter()
                .filter(|result| !result.derived.passed)
                .map(|result| {
                    format!(
                        "{}/{} recall={:.3} protected={} unmatched={} rotation_excluded={}",
                        result.entry_id,
                        result
                            .process_evidence_id
                            .rsplit('-')
                            .next()
                            .unwrap_or("unknown"),
                        result.derived.target_recall,
                        result.derived.protected_false_positive_count,
                        result.derived.unmatched_selected_node_ids.len(),
                        result.derived.rotation_targets_excluded
                    )
                })
                .collect::<Vec<_>>();
            if !failed.is_empty() {
                failures.push(format!("{}: {}", candidate.id, failed.join(", ")));
                continue;
            }
            let observed = cells
                .iter()
                .map(|result| {
                    (
                        result.entry_id.as_str(),
                        result.process_evidence_id.rsplit('-').next().unwrap_or(""),
                    )
                })
                .collect::<HashSet<_>>();
            if observed.len() == expected_cells
                && entry_ids.iter().all(|entry_id| {
                    ["cpu", "metal"]
                        .iter()
                        .all(|device| observed.contains(&(entry_id.as_str(), *device)))
                })
            {
                return Ok(candidate.id);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "no all-pass Source Gate crop candidate; {}",
                failures.join("; ")
            ),
        ))
    }

    fn select_or_write_calibration_diagnostic(
        environment: &SelectionEnvironment,
        process_evidence: &[ProcessEvidence],
        results: &[SelectionResult],
    ) -> io::Result<String> {
        match select_smallest_all_pass(results, &environment.calibration_entry_ids) {
            Ok(candidate_id) => Ok(candidate_id),
            Err(error) => {
                write_calibration_failure_diagnostic(
                    environment,
                    process_evidence,
                    results,
                    &error.to_string(),
                )?;
                Err(error)
            }
        }
    }

    fn write_calibration_failure_diagnostic(
        environment: &SelectionEnvironment,
        process_evidence: &[ProcessEvidence],
        results: &[SelectionResult],
        failure: &str,
    ) -> io::Result<()> {
        let diagnostic = CalibrationFailureDiagnostic {
            schema: "hanonly-source-gate-calibration-diagnostic-v1",
            b0_sha: &environment.b0_sha,
            manifest_sha256: &environment.visual_manifest_sha256,
            source_gate_fixture_manifest_sha256: &environment.source_gate_fixture_manifest_sha256,
            failure,
            candidates: candidates_schema(),
            calibration_entry_ids: &environment.calibration_entry_ids,
            process_evidence,
            calibration_results: results,
        };
        let path = environment
            .artifact
            .with_file_name("calibration-diagnostic.json");
        write_artifact(&path, &canonical_json(&diagnostic)?)
    }

    fn synthetic_entry_ids(phase: &str) -> Vec<String> {
        let prefix = match phase {
            "calibration" => "c",
            "holdout" => "h",
            _ => unreachable!("synthetic phase is closed"),
        };
        (1..=4)
            .map(|index| format!("r51-{prefix}{index:02}"))
            .collect()
    }

    fn validate_artifact(
        artifact: &FrozenArtifact,
        phase: Phase,
        environment: &SelectionEnvironment,
    ) -> io::Result<()> {
        require(
            artifact.version == ARTIFACT_VERSION && artifact.plan_revision == PLAN_REVISION,
            "selection artifact version or plan revision mismatch",
        )?;
        require(
            artifact.b0_sha == environment.b0_sha
                && artifact.source_gate_fixture_manifest_sha256
                    == environment.source_gate_fixture_manifest_sha256,
            "selection artifact frozen input drift",
        )?;
        let manifest_binding_valid = match phase {
            Phase::CalibrationFreeze => {
                artifact.holdout_manifest_sha256.is_none()
                    && (environment.phase == Phase::Holdout
                        || artifact.manifest_sha256 == environment.visual_manifest_sha256)
            }
            Phase::Holdout => {
                let expected_manifest = environment
                    .r51_formal_custody
                    .as_ref()
                    .and_then(|custody| custody.holdout.as_ref())
                    .map_or(environment.visual_manifest_sha256.as_str(), |holdout| {
                        holdout.freeze.manifest_sha256.as_str()
                    });
                artifact.holdout_manifest_sha256.as_deref() == Some(expected_manifest)
                    && artifact.holdout_entry_ids == r51_entry_ids('h')
            }
        };
        require(manifest_binding_valid, "selection manifest binding drift")?;
        require(
            artifact
                .candidates
                .iter()
                .any(|candidate| candidate.id == artifact.selected_candidate_id),
            "invalid selected candidate",
        )?;
        require(
            artifact.candidates == candidates_schema()
                && artifact.requested_devices == ["cpu", "metal"]
                && artifact.enabled_cargo_features == ["hanonly-test-evidence"]
                && artifact.backend_evidence_parser_version == 1
                && (environment.phase == Phase::Holdout
                    || artifact.calibration_entry_ids == environment.calibration_entry_ids)
                && artifact.holdout_entry_ids.len() == 4
                && !artifact.retuned_after_freeze,
            "candidate ratios drift",
        )?;
        require(
            artifact.frozen_recall_contract
                == frozen_recall_contract(&artifact.selected_candidate_id),
            "frozen recall contract drift",
        )?;
        validate_required_checks(artifact, phase, environment)?;
        for hash in [
            &artifact.image_input_contract_sha256,
            &artifact.source_color_contract_sha256,
            &artifact.color_constant_set_sha256,
            &artifact.frozen_payload_sha256,
            &artifact
                .frozen_recall_contract
                .ppocr_crop_local_preprocessing_sha256,
            &artifact.frozen_recall_contract.inverse_mapping_rule_sha256,
            &artifact
                .frozen_recall_contract
                .coverage_acceptance_rule_sha256,
            &artifact
                .frozen_recall_contract
                .source_removal_preflight_rule_sha256,
        ] {
            decode_sha256(hash)?;
        }
        let expected = match phase {
            Phase::CalibrationFreeze => (2, 32, 0, true),
            Phase::Holdout => (4, 32, 8, false),
        };
        require(
            (
                artifact.process_evidence.len(),
                artifact.calibration_results.len(),
                artifact.holdout_results.len(),
                artifact.holdout_completed_at_utc.is_none(),
            ) == expected,
            "selection artifact matrix counts mismatch",
        )?;
        require(
            frozen_projection_sha256(artifact)? == artifact.frozen_payload_sha256,
            "frozen payload sha256 mismatch",
        )?;
        validate_result_matrix(artifact)
    }

    fn validate_required_checks(
        artifact: &FrozenArtifact,
        phase: Phase,
        environment: &SelectionEnvironment,
    ) -> io::Result<()> {
        let expected_phases = match phase {
            Phase::CalibrationFreeze => [Some(Phase::CalibrationFreeze), None],
            Phase::Holdout => [Some(Phase::CalibrationFreeze), Some(Phase::Holdout)],
        };
        require(
            artifact.required_checks.len()
                == expected_phases
                    .iter()
                    .filter(|phase| phase.is_some())
                    .count(),
            "required-check phase count drift",
        )?;
        for (stored, expected_phase) in artifact
            .required_checks
            .iter()
            .zip(expected_phases.into_iter().flatten())
        {
            let path = environment.evidence_root.join(&stored.attestation_relpath);
            let (current, held) = load_required_check(
                &environment.evidence_root,
                &path,
                expected_phase,
                &artifact.b0_sha,
                &stored.manifest_sha256,
                &artifact.source_gate_fixture_manifest_sha256,
            )?;
            require(&current == stored, "required-check artifact entry drift")?;
            held.with_revalidated_path(|_| Ok(()))?;
        }
        Ok(())
    }

    fn frozen_projection_sha256(artifact: &FrozenArtifact) -> io::Result<String> {
        let mut process_evidence = artifact
            .process_evidence
            .iter()
            .filter(|process| process.phase == "calibration")
            .cloned()
            .collect::<Vec<_>>();
        process_evidence.sort_by_key(|process| process.id.clone());
        let mut calibration_results = artifact.calibration_results.clone();
        calibration_results.sort_by_key(|result| {
            (
                result.entry_id.clone(),
                result.process_evidence_id.clone(),
                result.candidate_id.clone(),
            )
        });
        let required_checks = artifact
            .required_checks
            .iter()
            .filter(|check| check.phase == "pre-calibration")
            .cloned()
            .collect::<Vec<_>>();
        let projection = serde_json::json!({
            "version": artifact.version,
            "plan_revision": artifact.plan_revision,
            "b0_sha": &artifact.b0_sha,
            "manifest_sha256": &artifact.manifest_sha256,
            "source_gate_fixture_manifest_sha256": &artifact.source_gate_fixture_manifest_sha256,
            "image_input_contract_sha256": &artifact.image_input_contract_sha256,
            "source_color_contract_sha256": &artifact.source_color_contract_sha256,
            "color_constant_set_sha256": &artifact.color_constant_set_sha256,
            "requested_devices": &artifact.requested_devices,
            "enabled_cargo_features": &artifact.enabled_cargo_features,
            "backend_evidence_parser_version": artifact.backend_evidence_parser_version,
            "required_checks": required_checks,
            "frozen_recall_contract": &artifact.frozen_recall_contract,
            "candidates": &artifact.candidates,
            "calibration_entry_ids": &artifact.calibration_entry_ids,
            "holdout_entry_ids": &artifact.holdout_entry_ids,
            "process_evidence": process_evidence,
            "calibration_results": calibration_results,
            "selected_candidate_id": &artifact.selected_candidate_id,
            "frozen_at_utc": &artifact.frozen_at_utc,
            "retuned_after_freeze": artifact.retuned_after_freeze,
        });
        let digest = Sha256::digest(canonical_json(&projection)?);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    fn validate_result_matrix(artifact: &FrozenArtifact) -> io::Result<()> {
        let processes = artifact
            .process_evidence
            .iter()
            .map(|process| (process.id.as_str(), process))
            .collect::<HashMap<_, _>>();
        require(
            processes.len() == artifact.process_evidence.len()
                && matches!(artifact.process_evidence.len(), 2 | 4),
            "duplicate process evidence id",
        )?;
        for process in &artifact.process_evidence {
            validate_text(&process.id)?;
            require(
                matches!(process.phase.as_str(), "calibration" | "holdout")
                    && matches!(process.requested_device.as_str(), "cpu" | "metal")
                    && process.paddle_instance_id.len() == 32
                    && process
                        .paddle_instance_id
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    && !process.runtime_library_sha256.is_empty(),
                "invalid process evidence identity",
            )?;
            decode_sha256(&process.executable_sha256)?;
            for hash in [
                &process.model_artifact_sha256.pp_detection,
                &process.model_artifact_sha256.pp_recognition,
                &process.model_artifact_sha256.pp_recognition_config,
                &process.model_artifact_sha256.vl_model,
                &process.model_artifact_sha256.vl_mmproj,
                &process.load_evidence.raw_load_log_sha256,
            ] {
                decode_sha256(hash)?;
            }
            for (path, hash) in &process.runtime_library_sha256 {
                validate_text(path)?;
                decode_sha256(hash)?;
            }
            require(
                !process.load_evidence.loaded_model_devices.is_empty()
                    && process
                        .load_evidence
                        .loaded_model_devices
                        .iter()
                        .enumerate()
                        .all(|(ordinal, device)| {
                            device.model_device_ordinal == ordinal as u32
                                && !device.name.is_empty()
                                && matches!(device.backend.as_str(), "CPU" | "Metal")
                        }),
                "invalid loaded model devices",
            )?;
        }
        let candidate_ids = artifact
            .candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<HashSet<_>>();
        let mut calibration_cells = HashSet::new();
        for result in &artifact.calibration_results {
            let process = validate_result(result, &processes, "calibration")?;
            require(
                artifact.calibration_entry_ids.contains(&result.entry_id)
                    && candidate_ids.contains(result.candidate_id.as_str())
                    && calibration_cells.insert((
                        result.entry_id.as_str(),
                        process.requested_device.as_str(),
                        result.candidate_id.as_str(),
                    )),
                "invalid or duplicate calibration cell",
            )?;
        }
        let mut holdout_cells = HashSet::new();
        for result in &artifact.holdout_results {
            let process = validate_result(result, &processes, "holdout")?;
            require(
                artifact.holdout_entry_ids.contains(&result.entry_id)
                    && result.candidate_id == artifact.selected_candidate_id
                    && holdout_cells
                        .insert((result.entry_id.as_str(), process.requested_device.as_str())),
                "invalid or duplicate holdout cell",
            )?;
        }
        require(
            calibration_cells.len() == 32
                && (artifact.holdout_results.is_empty() || holdout_cells.len() == 8),
            "selection result matrix is incomplete",
        )
    }

    fn validate_result<'a>(
        result: &SelectionResult,
        processes: &HashMap<&str, &'a ProcessEvidence>,
        phase: &str,
    ) -> io::Result<&'a ProcessEvidence> {
        let process = processes
            .get(result.process_evidence_id.as_str())
            .copied()
            .ok_or_else(|| invalid_data("missing process evidence"))?;
        let load = &process.load_evidence;
        let execution = &result.execution_evidence;
        let coverage = &result.derived.source_coverage_preflight;
        let expected_complete_coverage = !coverage.rejected_after_vl
            && !coverage.pp_vl_incomplete_coverage
            && coverage.source_text_roi_coverage == 1.0;
        let expected_preflight = result.derived.target_recall == 1.0 && expected_complete_coverage;
        let expected_pass = expected_preflight
            && result.derived.protected_false_positive_count == 0
            && result.derived.selected_protected_node_ids.is_empty()
            && result.derived.selected_rotation_target_ids.is_empty()
            && result.derived.unmatched_selected_node_ids.is_empty()
            && result.derived.rotation_targets_excluded;
        require(
            process.phase == phase
                && execution.paddle_instance_id == process.paddle_instance_id
                && execution.inference_completed
                && result.derived.actual_device == process.requested_device
                && coverage.pp_vl_complete_coverage == expected_complete_coverage
                && coverage.source_removal_preflight_passed == expected_preflight
                && result.derived.passed == expected_pass,
            "selection result instance or device mismatch",
        )?;
        decode_sha256(&execution.raw_inference_log_sha256)?;
        validate_text(&execution.source_gate_diagnostic_relpath)?;
        decode_sha256(&execution.source_gate_diagnostic_sha256)?;
        let positive = |map: &BTreeMap<String, u64>, backend: &str| {
            map.get(backend).copied().unwrap_or_default() > 0
        };
        if process.requested_device == "cpu" {
            require(
                load.cpu_forced
                    && load.n_gpu_layers == 0
                    && !load.mtmd_use_gpu
                    && !execution.context_offload_kqv
                    && !execution.context_op_offload
                    && load.offloaded_layers == 0
                    && load.mtmd_backend == "CPU"
                    && load
                        .loaded_model_devices
                        .iter()
                        .all(|device| device.backend == "CPU" && device.device_type == "cpu")
                    && positive(&load.model_buffer_bytes_by_backend, "CPU")
                    && positive(&execution.context_buffer_bytes_by_backend, "CPU")
                    && positive(&execution.compute_buffer_bytes_by_backend, "CPU")
                    && [
                        &load.model_buffer_bytes_by_backend,
                        &execution.context_buffer_bytes_by_backend,
                        &execution.compute_buffer_bytes_by_backend,
                    ]
                    .into_iter()
                    .all(|map| {
                        map.iter()
                            .all(|(backend, bytes)| backend == "CPU" || *bytes == 0)
                    }),
                "invalid CPU derivation",
            )?;
        } else {
            require(
                !load.cpu_forced
                    && load.n_gpu_layers == B0_DEFAULT_GPU_LAYERS
                    && load.mtmd_use_gpu
                    && execution.context_offload_kqv
                    && execution.context_op_offload
                    && load.offloaded_layers > 0
                    && load.mtmd_backend == "Metal"
                    && load
                        .loaded_model_devices
                        .iter()
                        .any(|device| device.backend == "Metal")
                    && load
                        .loaded_model_devices
                        .iter()
                        .all(|device| matches!(device.backend.as_str(), "CPU" | "Metal"))
                    && positive(&load.model_buffer_bytes_by_backend, "Metal")
                    && positive(&execution.context_buffer_bytes_by_backend, "Metal")
                    && positive(&execution.compute_buffer_bytes_by_backend, "Metal"),
                "invalid Metal derivation",
            )?;
        }
        Ok(process)
    }

    fn validate_text(value: &str) -> io::Result<()> {
        require(
            !value.is_empty()
                && !value
                    .chars()
                    .any(|character| matches!(character, '\0' | '\r' | '\n')),
            "selection evidence text is invalid",
        )
    }

    fn canonical_json(value: &impl Serialize) -> io::Result<Vec<u8>> {
        fn write(value: &serde_json::Value, output: &mut Vec<u8>) -> io::Result<()> {
            match value {
                serde_json::Value::Array(values) => {
                    output.push(b'[');
                    for (index, value) in values.iter().enumerate() {
                        if index != 0 {
                            output.push(b',');
                        }
                        write(value, output)?;
                    }
                    output.push(b']');
                }
                serde_json::Value::Object(values) => {
                    output.push(b'{');
                    let mut entries = values.iter().collect::<Vec<_>>();
                    entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
                    for (index, (key, value)) in entries.into_iter().enumerate() {
                        if index != 0 {
                            output.push(b',');
                        }
                        serde_json::to_writer(&mut *output, key)
                            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
                        output.push(b':');
                        write(value, output)?;
                    }
                    output.push(b'}');
                }
                _ => serde_json::to_writer(output, value)
                    .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?,
            }
            Ok(())
        }

        let value = serde_json::to_value(value)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        let mut output = Vec::new();
        write(&value, &mut output)?;
        Ok(output)
    }

    fn holdout_artifact_path(calibration_artifact: &Path) -> PathBuf {
        let name = calibration_artifact
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("selection.json");
        calibration_artifact.with_file_name(format!("{name}.holdout.json"))
    }

    fn write_artifact(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
        require(parent.is_dir(), "selection artifact parent must exist")?;
        let digest = sha256_hex(bytes);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid_data("selection artifact name is invalid"))?;
        let temporary = path.with_file_name(format!(".{name}.{digest}.tmp"));
        require(!path.exists(), "selection artifact already exists")?;
        require(
            !temporary.exists(),
            "selection artifact temporary already exists",
        )?;
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::hard_link(&temporary, path)?;
            let temporary_metadata = fs::symlink_metadata(&temporary)?;
            let final_metadata = fs::symlink_metadata(path)?;
            require(
                temporary_metadata.dev() == final_metadata.dev()
                    && temporary_metadata.ino() == final_metadata.ino()
                    && final_metadata.mode() & 0o777 == 0o600
                    && fs::read(path)? == bytes,
                "selection artifact create-new verification failed",
            )?;
            OpenOptions::new().read(true).open(path)?.sync_all()?;
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
            fs::remove_file(&temporary)?;
            OpenOptions::new().read(true).open(parent)?.sync_all()
        })();
        if result.is_err() && !path.exists() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    type RasterBounds = (u32, u32, u32, u32);

    fn candidates(
        bbox: (f64, f64, f64, f64),
        page: (u32, u32),
    ) -> [(&'static str, RasterBounds); 4] {
        const RATIOS: [(&str, u32, u32); 4] = [
            ("S25L4", 1, 25),
            ("S25L5", 1, 20),
            ("S25L6", 3, 50),
            ("S25L7", 7, 100),
        ];
        let short_side = (bbox.2 - bbox.0).min(bbox.3 - bbox.1);
        let long_side = (bbox.2 - bbox.0).max(bbox.3 - bbox.1);
        RATIOS.map(|(name, numerator, denominator)| {
            let padding = (short_side / 4.0)
                .max(long_side * f64::from(numerator) / f64::from(denominator))
                .ceil()
                .max(1.0);
            (
                name,
                (
                    (bbox.0 - padding).floor().clamp(0.0, f64::from(page.0)) as u32,
                    (bbox.1 - padding).floor().clamp(0.0, f64::from(page.1)) as u32,
                    (bbox.2 + padding).ceil().clamp(0.0, f64::from(page.0)) as u32,
                    (bbox.3 + padding).ceil().clamp(0.0, f64::from(page.1)) as u32,
                ),
            )
        })
    }

    #[test]
    fn source_gate_selection_candidates_quantize_outward_and_clip() {
        assert_eq!(
            candidates((10.2, 20.2, 110.8, 30.8), (200, 100)),
            [
                ("S25L4", (5, 15, 116, 36)),
                ("S25L5", (4, 14, 117, 37)),
                ("S25L6", (3, 13, 118, 38)),
                ("S25L7", (2, 12, 119, 39)),
            ]
        );
        assert_eq!(
            candidates((0.2, 0.2, 9.8, 9.8), (10, 10))[3].1,
            (0, 0, 10, 10)
        );
    }

    #[test]
    fn source_gate_native_log_parser_derives_cpu_and_metal_buffers() {
        let cpu_load = br#"
load_tensors: offloaded 0/19 layers to GPU
load_tensors: CPU_Mapped model buffer size = 890.14 MiB
clip_ctx: CLIP using CPU backend
"#;
        assert_eq!(
            parse_native_load_log(cpu_load).unwrap(),
            ParsedLoadLog {
                offloaded_layers: 0,
                offloadable_layers: 19,
                model_buffer_bytes_by_backend: BTreeMap::from([(
                    "CPU".into(),
                    (890.14_f64 * 1024.0 * 1024.0).round() as u64,
                )]),
                mtmd_backend: "CPU".into(),
            }
        );

        let metal_load = br#"
load_tensors: offloaded 18/19 layers to GPU
load_tensors: MTL0 model buffer size = 840.00 MiB
load_tensors: CPU_Mapped model buffer size = 50.14 MiB
clip_ctx: CLIP using MTL0 backend
"#;
        let parsed = parse_native_load_log(metal_load).unwrap();
        assert_eq!(
            (parsed.offloaded_layers, parsed.offloadable_layers),
            (18, 19)
        );
        assert!(parsed.model_buffer_bytes_by_backend["Metal"] > 0);
        assert!(parsed.model_buffer_bytes_by_backend["CPU"] > 0);
        assert_eq!(parsed.mtmd_backend, "Metal");

        let inference = br#"
llama_context: CPU output buffer size = 0.39 MiB
llama_context: CPU output buffer size pending allocation
llama_kv_cache: MTL0 KV buffer size = 9.00 MiB
sched_reserve: MTL0 compute buffer size = 63.75 MiB
sched_reserve: CPU compute buffer size = 1.57 MiB
"#;
        let parsed = parse_native_inference_log(inference).unwrap();
        assert!(parsed.context_buffer_bytes_by_backend["CPU"] > 0);
        assert!(parsed.context_buffer_bytes_by_backend["Metal"] > 0);
        assert!(parsed.compute_buffer_bytes_by_backend["CPU"] > 0);
        assert!(parsed.compute_buffer_bytes_by_backend["Metal"] > 0);
    }

    #[test]
    fn source_gate_native_log_parser_fails_closed_on_missing_actual_evidence() {
        assert!(parse_native_load_log(b"requested metal").is_err());
        assert!(parse_native_inference_log(b"inference completed").is_err());
        assert!(parse_native_inference_log(b"Vulkan compute buffer size = 1.00 MiB").is_err());
    }

    #[test]
    fn source_gate_manifest_roi_matching_is_half_open_and_overlap_is_strict() {
        let roi = ValidatedHalfOpenRect {
            left: 10,
            top: 20,
            right: 30,
            bottom: 40,
        };
        assert!(rect_contains(roi, (10.0, 20.0)));
        assert!(rect_contains(roi, (29.999, 39.999)));
        assert!(!rect_contains(roi, (30.0, 40.0)));
        assert!(rect_intersects(roi, [29.0, 39.0, 31.0, 41.0]));
        assert!(!rect_intersects(roi, [30.0, 40.0, 31.0, 41.0]));
    }

    #[test]
    fn source_gate_loaded_devices_come_from_enumerated_buffer_backends() {
        let enumerated = vec![
            EnumeratedDevice {
                index: 0,
                name: "CPU".into(),
                description: "Host CPU".into(),
                backend: "CPU".into(),
                device_type: "cpu".into(),
            },
            EnumeratedDevice {
                index: 1,
                name: "MTL0".into(),
                description: "Apple GPU".into(),
                backend: "MTL".into(),
                device_type: "integrated_gpu".into(),
            },
        ];
        let loaded = loaded_model_devices(
            &enumerated,
            &BTreeMap::from([("CPU".into(), 1), ("Metal".into(), 2)]),
        )
        .unwrap();
        assert_eq!(
            loaded
                .iter()
                .map(|device| device.backend.as_str())
                .collect::<Vec<_>>(),
            ["CPU", "Metal"]
        );
        assert!(loaded_model_devices(&enumerated, &BTreeMap::from([("CUDA".into(), 1)])).is_err());
    }

    #[test]
    fn source_gate_selection_reports_each_failed_candidate_cell() {
        let mut evidence = calibration_evidence();
        for candidate in candidates_schema() {
            let failed = evidence
                .results
                .iter_mut()
                .find(|result| {
                    result.entry_id == "r51-c01"
                        && result.process_evidence_id == "calibration-cpu"
                        && result.candidate_id == candidate.id
                })
                .unwrap();
            failed.derived.target_recall = 0.0;
            failed.derived.passed = false;
        }
        let error =
            select_smallest_all_pass(&evidence.results, &synthetic_entry_ids("calibration"))
                .unwrap_err()
                .to_string();
        for candidate in candidates_schema() {
            assert!(error.contains(&format!(
                "{}: r51-c01/cpu recall=0.000 protected=0 unmatched=0 rotation_excluded=true",
                candidate.id
            )));
        }
    }

    fn valid_environment(root: &Path) -> HashMap<&'static str, String> {
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        let manifest = root.join("visual-manifest.json");
        let manifest_bytes = serde_json::to_vec(&serde_json::json!({
            "entries": [
                {"id": "regression", "role": "regression"},
                {"id": "r51-c01", "role": "calibration"},
                {"id": "r51-c02", "role": "calibration"},
                {"id": "r51-c03", "role": "calibration"},
                {"id": "r51-c04", "role": "calibration"},
                {"id": "r51-h01", "role": "holdout"},
                {"id": "r51-h02", "role": "holdout"},
                {"id": "r51-h03", "role": "holdout"},
                {"id": "r51-h04", "role": "holdout"}
            ]
        }))
        .unwrap();
        fs::write(&manifest, &manifest_bytes).unwrap();
        let manifest_sha256 = sha256_hex(&manifest_bytes);
        let b0_sha = "a".repeat(40);
        let fixture_sha256 = "2".repeat(64);
        let required_check = write_required_check(
            root,
            Phase::CalibrationFreeze,
            &b0_sha,
            &manifest_sha256,
            &fixture_sha256,
        );
        HashMap::from([
            (PHASE_ENV, "calibration-freeze".into()),
            (B0_SHA_ENV, b0_sha),
            (
                VISUAL_INPUT_ENV,
                root.join("regression.png").to_string_lossy().into_owned(),
            ),
            (VISUAL_INPUT_SHA256_ENV, "0".repeat(64)),
            (
                VISUAL_EVIDENCE_ROOT_ENV,
                root.to_string_lossy().into_owned(),
            ),
            (VISUAL_MANIFEST_ENV, manifest.to_string_lossy().into_owned()),
            (VISUAL_MANIFEST_SHA256_ENV, manifest_sha256),
            (SOURCE_GATE_FIXTURE_SHA256_ENV, fixture_sha256),
            (
                ARTIFACT_ENV,
                root.join("selection.json").to_string_lossy().into_owned(),
            ),
            (
                REPORT_DIR_ENV,
                root.join("reports").to_string_lossy().into_owned(),
            ),
            (
                REQUIRED_CHECK_ENV,
                required_check.to_string_lossy().into_owned(),
            ),
        ])
    }

    fn formal_environment(root: &Path, phase: Phase) -> HashMap<&'static str, String> {
        let mut values = valid_environment(root);
        let manifest = root.join("visual-manifest.json");
        let (kind, role) = match phase {
            Phase::CalibrationFreeze => ('c', "calibration"),
            Phase::Holdout => ('h', "holdout"),
        };
        let entries = r51_entry_ids(kind)
            .into_iter()
            .map(|id| serde_json::json!({"id": id, "role": role}))
            .collect::<Vec<_>>();
        let manifest_bytes = serde_json::to_vec(&serde_json::json!({"entries": entries})).unwrap();
        fs::write(&manifest, &manifest_bytes).unwrap();
        let manifest_sha256 = sha256_hex(&manifest_bytes);
        values.insert(
            PHASE_ENV,
            phase_name(phase).replace("calibration", "calibration-freeze"),
        );
        values.insert(VISUAL_MANIFEST_SHA256_ENV, manifest_sha256);
        values.insert(R51_FORMAL_CUSTODY_ENV, "1".into());
        let calibration_entries = r51_entry_ids('c')
            .into_iter()
            .map(|id| serde_json::json!({"id": id, "role": "calibration"}))
            .collect::<Vec<_>>();
        let calibration_manifest_sha256 = sha256_hex(
            &serde_json::to_vec(&serde_json::json!({"entries": calibration_entries})).unwrap(),
        );
        values.insert(
            R51_CALIBRATION_MANIFEST_SHA256_ENV,
            calibration_manifest_sha256,
        );
        if phase == Phase::Holdout {
            let custody = root.join("r51-custody");
            let plaintext = root.join("r51-plaintext");
            fs::create_dir(&custody).unwrap();
            fs::create_dir(&plaintext).unwrap();
            fs::set_permissions(&custody, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&plaintext, fs::Permissions::from_mode(0o700)).unwrap();
            let archive = plaintext.join("bundle.tar");
            fs::write(&archive, b"synthetic archive").unwrap();
            fs::set_permissions(&archive, fs::Permissions::from_mode(0o600)).unwrap();
            let freeze = serde_json::json!({
                "contract": "hanonly-r51-encrypted-holdout-freeze-v1",
                "plan_revision": PLAN_REVISION,
                "base_b0_sha": values.get(B0_SHA_ENV).unwrap(),
                "implementation_thread_id": "synthetic-test",
                "frozen_before_production_edit": true,
                "entry_ids": r51_entry_ids('h'),
                "cipher": "aes-256-ctr",
                "integrity": "hmac-sha256-etm-v1",
                "iv_sha256": synthetic_hash(20),
                "ciphertext_byte_length": 1,
                "ciphertext_sha256": synthetic_hash(21),
                "header_sha256": synthetic_hash(22),
                "hmac_sha256": synthetic_hash(23),
                "plaintext_archive_sha256_commitment": sha256_file(&archive).unwrap(),
                "manifest_sha256_commitment": values.get(VISUAL_MANIFEST_SHA256_ENV).unwrap(),
                "oracle_sha256_commitment": synthetic_hash(25),
                "hashes_sha256_commitment": synthetic_hash(26),
                "historical_inventory_sha256": synthetic_hash(27),
                "formal_source_identities": [{}, {}, {}, {}],
                "disclosed_challenge_exclusion_pass": true,
                "result": "pass",
            });
            let receipt = custody.join("holdout-freeze-receipt.json");
            fs::write(&receipt, canonical_json(&freeze).unwrap()).unwrap();
            fs::set_permissions(&receipt, fs::Permissions::from_mode(0o600)).unwrap();
            values.insert(
                R51_CUSTODY_DIRECTORY_ENV,
                custody.to_string_lossy().into_owned(),
            );
            values.insert(
                R51_PLAINTEXT_DIRECTORY_ENV,
                plaintext.to_string_lossy().into_owned(),
            );
            values.insert(
                R51_PLAINTEXT_ARCHIVE_ENV,
                archive.to_string_lossy().into_owned(),
            );
        }
        let required_check = write_required_check(
            root,
            phase,
            values.get(B0_SHA_ENV).unwrap(),
            values.get(VISUAL_MANIFEST_SHA256_ENV).unwrap(),
            values.get(SOURCE_GATE_FIXTURE_SHA256_ENV).unwrap(),
        );
        values.insert(
            REQUIRED_CHECK_ENV,
            required_check.to_string_lossy().into_owned(),
        );
        if phase == Phase::Holdout {
            let custody = PathBuf::from(values.get(R51_CUSTODY_DIRECTORY_ENV).unwrap());
            let receipt = custody.join("holdout-freeze-receipt.json");
            let marker = R51OpenMarker {
                contract: "hanonly-r51-encrypted-holdout-open-v1".into(),
                plan_revision: PLAN_REVISION,
                b0_sha: values.get(B0_SHA_ENV).unwrap().clone(),
                selected_candidate_id: "S25L4".into(),
                freeze_receipt_sha256: sha256_file(&receipt).unwrap(),
                ciphertext_sha256: synthetic_hash(21),
                pre_holdout_attestation_sha256: sha256_file(&required_check).unwrap(),
                nonce_hex: synthetic_hash(30),
                result: "opened".into(),
            };
            let bytes = canonical_json(&marker).unwrap();
            let open = custody.join("holdout-open.json");
            fs::write(&open, &bytes).unwrap();
            fs::set_permissions(&open, fs::Permissions::from_mode(0o600)).unwrap();
            values.insert(R51_OPEN_MARKER_SHA256_ENV, sha256_hex(&bytes));
        }
        values
    }

    fn write_required_check(
        root: &Path,
        phase: Phase,
        b0_sha: &str,
        manifest_sha256: &str,
        fixture_sha256: &str,
    ) -> PathBuf {
        let directory = root.join("source-gate-selection/checks");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .unwrap();
        fs::set_permissions(
            root.join("source-gate-selection"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let attestation = RequiredCheckAttestation {
            version: 1,
            mode: "b0-source-gate-anti-fixture".into(),
            phase: required_check_phase(phase).into(),
            b0_sha: b0_sha.into(),
            manifest_sha256: manifest_sha256.into(),
            source_gate_fixture_manifest_sha256: fixture_sha256.into(),
            checker_endpoint_sha256: sha256_file(
                &repository_root().unwrap().join(CHECKER_ENDPOINT),
            )
            .unwrap(),
            scanned_roots: ANTI_FIXTURE_SCANNED_ROOTS
                .iter()
                .map(|value| (*value).into())
                .collect(),
            allowed_descriptor_roots: ANTI_FIXTURE_ALLOWED_DESCRIPTOR_ROOTS
                .iter()
                .map(|value| (*value).into())
                .collect(),
            policy_scan_sha256: "3".repeat(64),
            result: "pass".into(),
        };
        let path = root.join(required_check_relpath(phase));
        fs::write(&path, canonical_json(&attestation).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    fn set_required_check(values: &mut HashMap<&'static str, String>, root: &Path, phase: Phase) {
        let path = write_required_check(
            root,
            phase,
            values.get(B0_SHA_ENV).unwrap(),
            values.get(VISUAL_MANIFEST_SHA256_ENV).unwrap(),
            values.get(SOURCE_GATE_FIXTURE_SHA256_ENV).unwrap(),
        );
        values.insert(REQUIRED_CHECK_ENV, path.to_string_lossy().into_owned());
    }

    fn test_repository() -> PathBuf {
        repository_root().unwrap()
    }

    fn synthetic_hash(value: u8) -> String {
        format!("{value:064x}")
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn synthetic_process(phase: &str, device: &str) -> ProcessEvidence {
        let metal = device == "metal";
        ProcessEvidence {
            id: format!("{phase}-{device}"),
            phase: phase.into(),
            requested_device: device.into(),
            paddle_instance_id: if metal {
                "2".repeat(32)
            } else {
                "1".repeat(32)
            },
            executable_sha256: synthetic_hash(1),
            model_artifact_sha256: ModelArtifactHashes {
                pp_detection: synthetic_hash(2),
                pp_recognition: synthetic_hash(3),
                pp_recognition_config: synthetic_hash(4),
                vl_model: synthetic_hash(5),
                vl_mmproj: synthetic_hash(6),
            },
            runtime_library_sha256: BTreeMap::from([(
                "/usr/lib/libsynthetic.dylib".into(),
                synthetic_hash(7),
            )]),
            load_evidence: LoadEvidence {
                cpu_forced: !metal,
                gpu_offload_supported: metal,
                n_gpu_layers: if metal { B0_DEFAULT_GPU_LAYERS } else { 0 },
                mtmd_use_gpu: metal,
                word_boxes_backend: "rten_cpu".into(),
                raw_load_log_relpath: format!("source-gate/{phase}/{device}/load.log"),
                raw_load_log_sha256: synthetic_hash(8),
                enumerated_devices: Vec::new(),
                loaded_model_devices: vec![LoadedModelDevice {
                    model_device_ordinal: 0,
                    name: if metal { "Apple GPU" } else { "CPU" }.into(),
                    backend: if metal { "Metal" } else { "CPU" }.into(),
                    device_type: if metal { "integrated_gpu" } else { "cpu" }.into(),
                }],
                offloaded_layers: if metal { 32 } else { 0 },
                offloadable_layers: 39,
                model_buffer_bytes_by_backend: BTreeMap::from([
                    ("CPU".into(), 1),
                    ("Metal".into(), u64::from(metal)),
                ]),
                mtmd_backend: if metal { "Metal" } else { "CPU" }.into(),
            },
        }
    }

    fn synthetic_result(
        phase: &str,
        entry_id: &str,
        device: &str,
        candidate_id: &str,
    ) -> SelectionResult {
        let metal = device == "metal";
        SelectionResult {
            entry_id: entry_id.into(),
            process_evidence_id: format!("{phase}-{device}"),
            candidate_id: candidate_id.into(),
            execution_evidence: ExecutionEvidence {
                paddle_instance_id: if metal {
                    "2".repeat(32)
                } else {
                    "1".repeat(32)
                },
                context_offload_kqv: metal,
                context_op_offload: metal,
                inference_completed: true,
                raw_inference_log_relpath: format!(
                    "source-gate/{phase}/{entry_id}/{device}/{candidate_id}.log"
                ),
                raw_inference_log_sha256: synthetic_hash(9),
                source_gate_diagnostic_relpath: format!(
                    "source-gate/{phase}/{entry_id}/{device}/{candidate_id}.source-gate.json"
                ),
                source_gate_diagnostic_sha256: synthetic_hash(10),
                context_buffer_bytes_by_backend: BTreeMap::from([
                    ("CPU".into(), 1),
                    ("Metal".into(), u64::from(metal)),
                ]),
                compute_buffer_bytes_by_backend: BTreeMap::from([
                    ("CPU".into(), 1),
                    ("Metal".into(), u64::from(metal)),
                ]),
            },
            runtime_nodes: vec![RuntimeNode {
                node_id: format!("{entry_id}-node"),
                recognition_anchor: [0.0, 0.0, 1.0, 1.0],
                node_rotation: 0.0,
                text_rotation: 0.0,
                selected_as_han: true,
            }],
            derived: DerivedEvidence {
                actual_device: device.into(),
                matched_target_ids: vec!["target".into()],
                selected_target_ids: vec!["target".into()],
                selected_protected_node_ids: Vec::new(),
                selected_rotation_target_ids: Vec::new(),
                unmatched_selected_node_ids: Vec::new(),
                target_recall: 1.0,
                protected_false_positive_count: 0,
                rotation_targets_excluded: true,
                source_coverage_preflight: SourceCoveragePreflight {
                    pp_han_scalar_count: 1,
                    vl_expected_han_scalar_count: 1,
                    pp_vl_complete_coverage: true,
                    rejected_after_vl: false,
                    pp_vl_incomplete_coverage: false,
                    covered_source_roi_ids: vec!["target".into()],
                    source_text_roi_coverage: 1.0,
                    source_removal_preflight_passed: true,
                },
                passed: true,
            },
        }
    }

    #[test]
    fn source_gate_coverage_uses_raster_proof_not_pp_vl_count_equality() {
        let process = synthetic_process("calibration", "cpu");
        let processes = HashMap::from([(process.id.as_str(), &process)]);
        let mut result = synthetic_result("calibration", "r51-c01", "cpu", "S25L4");
        result.derived.source_coverage_preflight.pp_han_scalar_count = 0;
        result
            .derived
            .source_coverage_preflight
            .vl_expected_han_scalar_count = 4;

        assert!(validate_result(&result, &processes, "calibration").is_ok());
    }

    fn r51_test_schema_and_oracle() -> (VisualManifestEntry, OracleValidatedEntry) {
        let schema = serde_json::from_value(serde_json::json!({
            "id": "r51-c01",
            "path": "source.png",
            "sha256": synthetic_hash(40),
            "decoded_rgba_blake3": synthetic_hash(41),
            "clean_reference_path": "clean.png",
            "clean_reference_sha256": synthetic_hash(42),
            "clean_reference_decoded_rgba_blake3": synthetic_hash(43),
            "role": "calibration",
            "dimension_bin": "lt720",
            "aspect": "square_or_near",
            "background": "pure",
            "targets": [{
                "id": "target",
                "source_roi": [0, 0, 50, 50],
                "clean_reference_edit_roi": [0, 0, 50, 50],
                "erase_source_ink_mask_path": "erase.bin",
                "erase_source_ink_mask_sha256": synthetic_hash(44),
                "residual_source_ink_mask_path": "residual.bin",
                "residual_source_ink_mask_sha256": synthetic_hash(45),
                "position": "interior",
                "writing": "horizontal",
                "effect": "plain",
                "translation_length": "equal",
                "expected": "automatic_strict"
            }],
            "protected_rois": [[50, 0, 64, 64]],
            "multi_node": false
        }))
        .unwrap();
        let oracle = OracleValidatedEntry {
            protected_rois: vec![ValidatedHalfOpenRect {
                left: 50,
                top: 0,
                right: 64,
                bottom: 64,
            }],
            targets: vec![OracleValidatedTarget {
                source_roi: ValidatedHalfOpenRect {
                    left: 0,
                    top: 0,
                    right: 50,
                    bottom: 50,
                },
                edit_roi: ValidatedHalfOpenRect {
                    left: 0,
                    top: 0,
                    right: 50,
                    bottom: 50,
                },
                delta_mask: vec![1; 50 * 50].into_boxed_slice(),
            }],
        };
        (schema, oracle)
    }

    fn r51_test_quad_bits(left: f32, top: f32, right: f32, bottom: f32) -> [u32; 8] {
        [
            left.to_bits(),
            top.to_bits(),
            right.to_bits(),
            top.to_bits(),
            right.to_bits(),
            bottom.to_bits(),
            left.to_bits(),
            bottom.to_bits(),
        ]
    }

    #[test]
    fn r51_selection_geometry_closes_detector_ownership_preimages() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = valid_environment(&root);
        let environment = SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap();
        let (schema, oracle) = r51_test_schema_and_oracle();
        let result = synthetic_result("calibration", "r51-c01", "cpu", "S25L4");
        let node_id = NodeId::new();
        let target_bits = r51_test_quad_bits(10.0, 10.0, 20.0, 20.0);
        let protected_bits = r51_test_quad_bits(52.0, 10.0, 60.0, 20.0);
        let diagnostics = vec![
            SourceGateDiagnosticEvent::Input {
                backend: "pp-ocr-v5",
                width: 64,
                height: 64,
                decoded_rgba_hash: synthetic_hash(46),
            },
            SourceGateDiagnosticEvent::Crop {
                candidate_index: 0,
                node_id,
                bounds: [0, 0, 64, 64],
                crop_rgba_hash: synthetic_hash(47),
            },
            SourceGateDiagnosticEvent::PpSummary {
                node_id,
                words: Vec::new(),
                raw_detectors: vec![
                    PpDetectorDiagnostic {
                        occurrence_index: 0,
                        source_scaled_quad_f32_bits: target_bits,
                    },
                    PpDetectorDiagnostic {
                        occurrence_index: 1,
                        source_scaled_quad_f32_bits: protected_bits,
                    },
                ],
                canonical_lines: vec![
                    PpCanonicalLineDiagnostic {
                        line_index: 0,
                        detector_occurrences: vec![PpCanonicalOccurrenceDiagnostic {
                            occurrence_index: 0,
                            canonical_corners_f32_bits: target_bits,
                        }],
                        recognition: Some(PpRecognitionDiagnostic {
                            present: true,
                            recognition_class: "han",
                            segment_count: 1,
                        }),
                    },
                    PpCanonicalLineDiagnostic {
                        line_index: 1,
                        detector_occurrences: vec![PpCanonicalOccurrenceDiagnostic {
                            occurrence_index: 1,
                            canonical_corners_f32_bits: protected_bits,
                        }],
                        recognition: Some(PpRecognitionDiagnostic {
                            present: true,
                            recognition_class: "protected_latin",
                            segment_count: 1,
                        }),
                    },
                ],
            },
            SourceGateDiagnosticEvent::SelectionGeometry {
                node_id,
                targets: vec![SourceGateTargetGeometryDiagnostic {
                    scene_quad_f32_bits: target_bits,
                }],
                protected_lines: vec![SourceGateTargetGeometryDiagnostic {
                    scene_quad_f32_bits: protected_bits,
                }],
                detector_ownership: vec![
                    SourceGateDetectorOwnershipDiagnostic {
                        occurrence_index: 0,
                        canonical_line_index: Some(0),
                        scene_quad_f32_bits: target_bits,
                        assignment: SourceGateDetectorAssignmentDiagnostic::Target {
                            target_index: 0,
                        },
                    },
                    SourceGateDetectorOwnershipDiagnostic {
                        occurrence_index: 1,
                        canonical_line_index: Some(1),
                        scene_quad_f32_bits: protected_bits,
                        assignment: SourceGateDetectorAssignmentDiagnostic::Protected {
                            protected_index: 0,
                        },
                    },
                ],
            },
        ];
        let (_, _, _, records) =
            r51_detector_diagnostics(&environment, &result, &schema, &oracle, &diagnostics, &None)
                .unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["preimage"]["target_id"], "target");
        assert_eq!(
            records[0]["preimage"]["canonical_assignment"],
            "selected_han"
        );
        assert_eq!(records[0]["preimage"]["ownership_verdict"], "unique");
        assert_eq!(
            records[0]["preimage"]["detector_support_mask"],
            records[0]["preimage"]["line_support_mask"]
        );
        assert_eq!(
            records[1]["preimage"]["canonical_assignment"],
            "preserved_source"
        );
        assert!(
            records[1]["preimage"]["protected_support_pixels"]
                .as_u64()
                .unwrap()
                > 0
        );
        let rejected_reason = Some("pp_vl_incomplete_coverage".to_owned());
        let (_, _, _, rejected) = r51_detector_diagnostics(
            &environment,
            &result,
            &schema,
            &oracle,
            &diagnostics[..3],
            &rejected_reason,
        )
        .unwrap();
        assert_eq!(rejected.len(), 2);
        assert!(rejected.iter().all(|record| {
            record["preimage"]["ownership_verdict"] == "unassigned"
                && record["preimage"]["selection_verdict"] == "rejected"
                && record["preimage"]["emitted_scene_quad"].is_array()
                && record["preimage"]["detector_support_mask"].is_object()
                && record["preimage"]["line_support_mask"].is_object()
                && record["preimage"]["agreed_mask"].is_object()
        }));
    }

    #[test]
    fn r51_validated_execution_view_preserves_local_coverage_mask() {
        let mut page_mask = vec![0_u8; 64 * 64];
        for y in 10..20 {
            page_mask[y * 64 + 10..y * 64 + 20].fill(1);
        }
        let prepared = prepare_r51_execution_entries(R51ValidatedExecutionView {
            entries: vec![R51ValidatedExecutionEntry {
                id: "r51-h01".into(),
                source_encoded_bytes: vec![1].into_boxed_slice(),
                clean_reference_encoded_bytes: vec![2].into_boxed_slice(),
                validated_source_rgba: RgbaImage::new(64, 64),
                validated_clean_reference_rgba: RgbaImage::new(64, 64),
                source_width: 64,
                source_height: 64,
                clean_width: 64,
                clean_height: 64,
                protected_rois: vec![[40, 40, 50, 50]],
                targets: vec![R51ValidatedExecutionTarget {
                    id: "target".into(),
                    source_roi: [10, 10, 20, 20],
                    clean_reference_edit_roi: [10, 10, 20, 20],
                    erase_source_ink_mask_encoded_bytes: vec![3].into_boxed_slice(),
                    residual_source_ink_mask_encoded_bytes: vec![3].into_boxed_slice(),
                    validated_binary_mask: page_mask.into_boxed_slice(),
                    expected: "automatic_strict".into(),
                    writing: "horizontal".into(),
                    effect: "plain".into(),
                    position: "interior".into(),
                    translation_length: "short".into(),
                }],
            }],
        })
        .unwrap();

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].0.id, "r51-h01");
        assert_eq!(prepared[0].0.targets[0].id, "target");
        assert_eq!(&*prepared[0].2.targets[0].delta_mask, &[1; 100]);
        assert_eq!(
            prepared[0].2.protected_rois[0],
            ValidatedHalfOpenRect {
                left: 40,
                top: 40,
                right: 50,
                bottom: 50,
            }
        );
    }

    #[test]
    fn r51_selected_and_downstream_support_have_independent_geometry_sources() {
        let (schema, oracle) = r51_test_schema_and_oracle();
        let node_id = NodeId::new();
        let diagnostics = vec![SourceGateDiagnosticEvent::SelectionGeometry {
            node_id,
            targets: vec![SourceGateTargetGeometryDiagnostic {
                scene_quad_f32_bits: r51_test_quad_bits(10.0, 10.0, 20.0, 20.0),
            }],
            protected_lines: Vec::new(),
            detector_ownership: Vec::new(),
        }];
        let selected =
            r51_selected_support_from_diagnostics(64, 64, &schema, &oracle, &diagnostics).unwrap();
        let mut page = Page::new("r51-c01", 64, 64);
        page.nodes.insert(
            node_id,
            Node {
                id: node_id,
                transform: Transform {
                    x: 10.0,
                    y: 10.0,
                    width: 20.0,
                    height: 20.0,
                    rotation_deg: 0.0,
                },
                visible: true,
                kind: NodeKind::Text(TextData {
                    text: Some("汉".into()),
                    detector: Some(SOURCE_GATE_TARGET_DETECTOR.into()),
                    line_polygons: Some(vec![[
                        [10.0, 10.0],
                        [30.0, 10.0],
                        [30.0, 30.0],
                        [10.0, 30.0],
                    ]]),
                    ..Default::default()
                }),
            },
        );
        let downstream = r51_downstream_support_from_scene(&page, &schema, &oracle).unwrap();
        assert_ne!(
            selected["target"].as_slice(),
            downstream["target"].as_raw().as_slice()
        );
        assert_eq!(
            selected["target"]
                .iter()
                .map(|value| u64::from(*value))
                .sum::<u64>(),
            100
        );
        assert!(
            downstream["target"]
                .as_raw()
                .iter()
                .filter(|value| **value != 0)
                .count()
                > 100
        );
    }

    fn synthetic_formal_cell(entry: &str, device: &str, passed: bool) -> R51TerminalCellResult {
        let candidate_id = "S25L4";
        R51TerminalCellResult {
            cell_key: format!("{entry}/{device}"),
            result: if passed { "pass" } else { "fail-closed" }.into(),
            selection_result: Some(if passed { "selected" } else { "rejected" }.into()),
            target_recall: R51TargetRecall {
                target_total: 1,
                selected: usize::from(passed),
                covered: usize::from(passed),
                uncovered: usize::from(!passed),
            },
            pp_han_count: 1,
            vl_han_count: 1,
            rejection_reason: (!passed).then(|| "coverage_failure".into()),
            device_evidence_sha256: synthetic_hash(11),
            log_sha256: synthetic_hash(12),
            diagnostic_sha256: synthetic_hash(13),
            target_coverage_index_sha256: Some(synthetic_hash(14)),
            diagnostic_cell_key: format!("holdout/{candidate_id}/{device}/{entry}"),
            phase: "holdout".into(),
            candidate_id: candidate_id.into(),
            entry_id: entry.into(),
            device: device.into(),
            terminal_reason: (!passed).then(|| "coverage_failure".into()),
            diagnostic_path: format!(
                "cells/holdout/{candidate_id}/{device}/{entry}/cell-diagnostic.json"
            ),
            diagnostic_byte_length: 1,
            target_coverage_index_path: Some(format!(
                "cells/holdout/{candidate_id}/{device}/{entry}/target-coverage-index.json"
            )),
            target_coverage_index_byte_length: Some(1),
            device_evidence_path: format!(
                "cells/holdout/{candidate_id}/{device}/{entry}/device-evidence.json"
            ),
            device_evidence_byte_length: 1,
            log_path: format!("cells/holdout/{candidate_id}/{device}/{entry}/inference.log"),
            log_byte_length: 1,
        }
    }

    fn synthetic_formal_run(cells: Vec<R51TerminalCellResult>) -> R51FormalRunEvidence {
        let first_failed_cell = cells
            .iter()
            .find(|cell| cell.result != "pass")
            .map(|cell| cell.cell_key.clone());
        R51FormalRunEvidence {
            bundle_validation_receipt: Some(PublishedArtifact {
                path: "reports/r51/bundle-validation.json".into(),
                sha256: synthetic_hash(15),
                byte_length: 1,
            }),
            cells,
            first_failed_cell,
        }
    }

    fn seed_r51_calibration_generations(root: &Path) -> String {
        let values = formal_environment(root, Phase::CalibrationFreeze);
        let environment = SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap();
        let cells = candidates_schema()
            .into_iter()
            .flat_map(|candidate| {
                ["cpu", "metal"].into_iter().flat_map(move |device| {
                    r51_entry_ids('c').into_iter().map({
                        let candidate = candidate.id.clone();
                        move |entry| R51TerminalCellResult {
                            cell_key: format!(
                                "calibration-freeze/{candidate}/{device}/{entry}"
                            ),
                            result: "pass".into(),
                            selection_result: Some("selected".into()),
                            target_recall: R51TargetRecall {
                                target_total: 1,
                                selected: 1,
                                covered: 1,
                                uncovered: 0,
                            },
                            pp_han_count: 1,
                            vl_han_count: 1,
                            rejection_reason: None,
                            device_evidence_sha256: synthetic_hash(31),
                            log_sha256: synthetic_hash(32),
                            diagnostic_sha256: synthetic_hash(33),
                            target_coverage_index_sha256: None,
                            diagnostic_cell_key: format!(
                                "calibration-freeze/{candidate}/{device}/{entry}"
                            ),
                            phase: "calibration-freeze".into(),
                            candidate_id: candidate.clone(),
                            entry_id: entry.clone(),
                            device: device.into(),
                            terminal_reason: None,
                            diagnostic_path: format!(
                                "cells/calibration-freeze/{candidate}/{device}/{entry}/cell-diagnostic.json"
                            ),
                            diagnostic_byte_length: 1,
                            target_coverage_index_path: None,
                            target_coverage_index_byte_length: None,
                            device_evidence_path: format!(
                                "cells/calibration-freeze/{candidate}/{device}/{entry}/device-evidence.json"
                            ),
                            device_evidence_byte_length: 1,
                            log_path: format!(
                                "cells/calibration-freeze/{candidate}/{device}/{entry}/inference.log"
                            ),
                            log_byte_length: 1,
                        }
                    })
                })
            })
            .collect();
        let formal = R51FormalRunEvidence {
            bundle_validation_receipt: None,
            cells,
            first_failed_cell: None,
        };
        let terminal = write_r51_calibration_diagnostic_generations(&environment, &formal).unwrap();
        assert!(terminal.path.ends_with("00000064.json"));
        environment.visual_manifest_sha256
    }

    #[test]
    fn r51_publication_is_create_new_mode_0600_and_canonical_without_newline() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = valid_environment(&root);
        let environment = SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap();
        let bytes = canonical_json(&serde_json::json!({"b": 2, "a": 1})).unwrap();
        let published = publish_r51_artifact(&environment, "r51/publication.json", &bytes).unwrap();
        let path = root.join(&published.path);

        assert_eq!(bytes, br#"{"a":1,"b":2}"#);
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert!(publish_r51_artifact(&environment, "r51/publication.json", &bytes).is_err());
        assert!(!fs::read_dir(path.parent().unwrap()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[test]
    fn r51_diagnostics_require_eight_passes_or_stop_at_first_failure() {
        let expected = r51_entry_ids('h')
            .into_iter()
            .flat_map(|entry| {
                ["cpu", "metal"]
                    .into_iter()
                    .map(move |device| (entry.clone(), device))
            })
            .collect::<Vec<_>>();

        let incomplete_temp = tempfile::tempdir().unwrap();
        let incomplete_root = fs::canonicalize(incomplete_temp.path()).unwrap();
        let incomplete_calibration_manifest = seed_r51_calibration_generations(&incomplete_root);
        let incomplete_values = formal_environment(&incomplete_root, Phase::Holdout);
        let incomplete =
            SelectionEnvironment::parse(|name| incomplete_values.get(name).cloned()).unwrap();
        validate_r51_runner_open(&incomplete, "S25L4").unwrap();
        let seven = synthetic_formal_run(
            expected[..7]
                .iter()
                .map(|(entry, device)| synthetic_formal_cell(entry, device, true))
                .collect(),
        );
        assert!(
            write_r51_diagnostic_generations(
                &incomplete,
                "S25L4",
                &incomplete_calibration_manifest,
                &seven,
            )
            .is_err()
        );
        assert!(
            !incomplete
                .report_dir
                .join("r51/diagnostic-index.generations/00000065.json")
                .exists()
        );

        let failure_temp = tempfile::tempdir().unwrap();
        let failure_root = fs::canonicalize(failure_temp.path()).unwrap();
        let failure_calibration_manifest = seed_r51_calibration_generations(&failure_root);
        let failure_values = formal_environment(&failure_root, Phase::Holdout);
        let failure =
            SelectionEnvironment::parse(|name| failure_values.get(name).cloned()).unwrap();
        validate_r51_runner_open(&failure, "S25L4").unwrap();
        let failed = synthetic_formal_run(vec![synthetic_formal_cell(
            &expected[0].0,
            expected[0].1,
            false,
        )]);
        write_r51_diagnostic_generations(&failure, "S25L4", &failure_calibration_manifest, &failed)
            .unwrap();
        assert!(
            failure
                .report_dir
                .join("r51/diagnostic-index.generations/00000066.json")
                .is_file()
        );
        assert!(
            !failure
                .report_dir
                .join("r51/diagnostic-index.generations/00000067.json")
                .exists()
        );

        let pass_temp = tempfile::tempdir().unwrap();
        let pass_root = fs::canonicalize(pass_temp.path()).unwrap();
        let pass_calibration_manifest = seed_r51_calibration_generations(&pass_root);
        let pass_values = formal_environment(&pass_root, Phase::Holdout);
        let pass = SelectionEnvironment::parse(|name| pass_values.get(name).cloned()).unwrap();
        validate_r51_runner_open(&pass, "S25L4").unwrap();
        let complete = synthetic_formal_run(
            expected
                .iter()
                .map(|(entry, device)| synthetic_formal_cell(entry, device, true))
                .collect(),
        );
        let terminal =
            write_r51_diagnostic_generations(&pass, "S25L4", &pass_calibration_manifest, &complete)
                .unwrap();
        assert!(terminal.path.ends_with("diagnostic-index.json"));
        let terminal_bytes = fs::read(pass.artifact.parent().unwrap().join(terminal.path)).unwrap();
        let terminal_json: serde_json::Value = serde_json::from_slice(&terminal_bytes).unwrap();
        assert_eq!(terminal_json["contract"], "hanonly-r50-diagnostic-index-v1");
        assert_eq!(terminal_json["generation"], 80);
        assert_eq!(terminal_json["records"].as_array().unwrap().len(), 40);
        assert!(
            terminal_json["records"]
                .as_array()
                .unwrap()
                .iter()
                .all(|record| record["state"] == "passed")
        );
        let summary_bytes = fs::read(pass.report_dir.join("r51/completion-summary.json")).unwrap();
        let summary: serde_json::Value = serde_json::from_slice(&summary_bytes).unwrap();
        assert_eq!(summary["result"], "pass");
        assert!(summary["failure_kind"].is_null());
        assert!(summary["first_failed_cell"].is_null());
        assert_eq!(summary["unexecuted_cell_keys"], serde_json::json!([]));
        assert_eq!(summary["all_cells_terminated"], true);
        assert_eq!(summary["all_cells_passed"], true);
        let published = PublishedArtifact {
            path: "reports/r51/completion-summary.json".into(),
            sha256: sha256_hex(&summary_bytes),
            byte_length: summary_bytes.len() as u64,
        };
        assert!(
            r51_completion_summary_stdout_line(&published)
                .unwrap()
                .starts_with(R51_COMPLETION_SUMMARY_STDOUT_PREFIX)
        );
    }

    fn calibration_evidence() -> RunnerEvidence {
        RunnerEvidence {
            selected_candidate_id: "S25L4".into(),
            process_evidence: ["cpu", "metal"]
                .map(|device| synthetic_process("calibration", device))
                .into(),
            results: synthetic_entry_ids("calibration")
                .iter()
                .flat_map(|entry_id| {
                    ["cpu", "metal"].into_iter().flat_map(move |device| {
                        candidates_schema().into_iter().map(move |candidate| {
                            synthetic_result("calibration", entry_id, device, &candidate.id)
                        })
                    })
                })
                .collect(),
            r51_formal: None,
        }
    }

    fn holdout_evidence() -> RunnerEvidence {
        RunnerEvidence {
            selected_candidate_id: "S25L4".into(),
            process_evidence: ["cpu", "metal"]
                .map(|device| synthetic_process("holdout", device))
                .into(),
            results: synthetic_entry_ids("holdout")
                .iter()
                .flat_map(|entry_id| {
                    ["cpu", "metal"]
                        .into_iter()
                        .map(move |device| synthetic_result("holdout", entry_id, device, "S25L4"))
                })
                .collect(),
            r51_formal: None,
        }
    }

    #[test]
    fn source_gate_selection_preflight_fails_before_model_runner() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let model_calls = std::cell::Cell::new(0);
        let run = |values: &HashMap<&'static str, String>,
                   head: io::Result<String>,
                   fixture: io::Result<()>| {
            run_with(
                |name| values.get(name).cloned(),
                &test_repository(),
                |_| head,
                |_| fixture,
                |_| {
                    model_calls.set(model_calls.get() + 1);
                    Ok(calibration_evidence())
                },
            )
        };

        let mut missing = valid_environment(&root);
        missing.remove(PHASE_ENV);
        assert!(run(&missing, Ok("a".repeat(40)), Ok(())).is_err());

        let valid = valid_environment(&root);
        let mut missing_check = valid.clone();
        missing_check.remove(REQUIRED_CHECK_ENV);
        assert!(run(&missing_check, Ok("a".repeat(40)), Ok(())).is_err());
        let mut invalid = valid.clone();
        invalid.insert(PHASE_ENV, "selection".into());
        assert!(run(&invalid, Ok("a".repeat(40)), Ok(())).is_err());
        invalid.insert(PHASE_ENV, "calibration-freeze".into());
        invalid.insert(B0_SHA_ENV, "A".repeat(40));
        assert!(run(&invalid, Ok("a".repeat(40)), Ok(())).is_err());
        assert!(run(&valid, Ok("b".repeat(40)), Ok(())).is_err());

        fs::write(root.join("selection.json"), b"frozen").unwrap();
        assert!(run(&valid, Ok("a".repeat(40)), Ok(())).is_err());
        fs::remove_file(root.join("selection.json")).unwrap();

        let mut holdout = valid.clone();
        holdout.insert(PHASE_ENV, "holdout".into());
        assert!(run(&holdout, Ok("a".repeat(40)), Ok(())).is_err());
        fs::create_dir(root.join("selection.json")).unwrap();
        assert!(run(&holdout, Ok("a".repeat(40)), Ok(())).is_err());
        fs::remove_dir(root.join("selection.json")).unwrap();

        assert!(
            run(
                &valid,
                Ok("a".repeat(40)),
                Err(io::Error::other("fixed fixture is dirty")),
            )
            .is_err()
        );
        assert_eq!(model_calls.get(), 0);

        let result = run_with(
            |name| valid.get(name).cloned(),
            &test_repository(),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| {
                model_calls.set(model_calls.get() + 1);
                Err(io::Error::other(
                    "Source Gate model runner is not implemented",
                ))
            },
        );
        assert!(result.is_err());
        assert_eq!(model_calls.get(), 1);
        assert!(!root.join("selection.json").exists());
    }

    #[test]
    fn r51_formal_custody_is_default_off_and_accepts_only_phase_manifest_ids() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let default_values = valid_environment(&root);
        assert!(SelectionEnvironment::parse(|name| default_values.get(name).cloned()).is_ok());

        let calibration = formal_environment(&root, Phase::CalibrationFreeze);
        assert!(!calibration.contains_key(R51_OPEN_MARKER_SHA256_ENV));
        let parsed = SelectionEnvironment::parse(|name| calibration.get(name).cloned()).unwrap();
        assert_eq!(parsed.calibration_entry_ids, r51_entry_ids('c'));
        assert!(matches!(parsed.holdout_entry_ids.len(), 0 | 4));
        assert_eq!(frozen_holdout_entry_ids(&parsed), r51_entry_ids('h'));

        let holdout = formal_environment(&root, Phase::Holdout);
        assert!(holdout.contains_key(R51_OPEN_MARKER_SHA256_ENV));
        let parsed = SelectionEnvironment::parse(|name| holdout.get(name).cloned()).unwrap();
        assert!(parsed.calibration_entry_ids.is_empty());
        assert_eq!(parsed.holdout_entry_ids, r51_entry_ids('h'));

        let mut missing_runtime_paths = holdout.clone();
        for name in [
            VISUAL_INPUT_ENV,
            VISUAL_INPUT_SHA256_ENV,
            VISUAL_MANIFEST_ENV,
            VISUAL_MANIFEST_SHA256_ENV,
        ] {
            missing_runtime_paths.remove(name);
        }
        assert!(
            SelectionEnvironment::parse(|name| missing_runtime_paths.get(name).cloned()).is_ok()
        );

        let mut unreadable_runtime_paths = holdout.clone();
        unreadable_runtime_paths.insert(
            VISUAL_INPUT_ENV,
            root.join("missing-input").to_string_lossy().into_owned(),
        );
        unreadable_runtime_paths.insert(VISUAL_INPUT_SHA256_ENV, "not-a-hash".into());
        unreadable_runtime_paths.insert(
            VISUAL_MANIFEST_ENV,
            root.join("missing-manifest").to_string_lossy().into_owned(),
        );
        unreadable_runtime_paths.insert(VISUAL_MANIFEST_SHA256_ENV, "not-a-hash".into());
        assert!(
            SelectionEnvironment::parse(|name| unreadable_runtime_paths.get(name).cloned()).is_ok()
        );

        let mut missing_open_hash = holdout.clone();
        missing_open_hash.remove(R51_OPEN_MARKER_SHA256_ENV);
        assert!(SelectionEnvironment::parse(|name| missing_open_hash.get(name).cloned()).is_err());

        let mut invalid = holdout;
        invalid.insert(R51_FORMAL_CUSTODY_ENV, "yes".into());
        assert!(SelectionEnvironment::parse(|name| invalid.get(name).cloned()).is_err());
    }

    #[test]
    fn r51_holdout_consumes_runner_open_without_publishing_custody_markers() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = formal_environment(&root, Phase::Holdout);
        let environment = SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap();

        validate_r51_runner_open(&environment, "S25L4").unwrap();
        let custody = environment
            .r51_formal_custody
            .as_ref()
            .unwrap()
            .holdout
            .as_ref()
            .unwrap();
        assert_eq!(
            fs::metadata(custody.directory.join("holdout-open.json"))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
        assert!(validate_r51_runner_open(&environment, "S25L4").is_err());
        assert!(!custody.directory.join("holdout-failure.json").exists());
        assert!(!custody.directory.join("holdout-terminal.json").exists());
    }

    #[test]
    fn r51_holdout_rejects_open_hash_and_custody_terminal_state_drift() {
        for forbidden in [
            ".holdout-open.synthetic.tmp",
            "holdout-failure.json",
            ".holdout-failure.synthetic.tmp",
            "holdout-terminal.json",
            ".holdout-terminal.synthetic.tmp",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(temp.path()).unwrap();
            let values = formal_environment(&root, Phase::Holdout);
            let path =
                PathBuf::from(values.get(R51_CUSTODY_DIRECTORY_ENV).unwrap()).join(forbidden);
            fs::write(&path, b"forbidden").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let environment =
                SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap();
            assert!(validate_r51_runner_open(&environment, "S25L4").is_err());
        }

        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let mut values = formal_environment(&root, Phase::Holdout);
        values.insert(R51_OPEN_MARKER_SHA256_ENV, synthetic_hash(31));
        let environment = SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap();
        assert!(validate_r51_runner_open(&environment, "S25L4").is_err());
    }

    #[test]
    fn r51_diagnostic_generations_are_ordered_create_new_and_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let calibration_manifest_sha256 = seed_r51_calibration_generations(&root);
        let values = formal_environment(&root, Phase::Holdout);
        let environment = SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap();
        validate_r51_runner_open(&environment, "S25L4").unwrap();
        let bundle = publish_r51_artifact(
            &environment,
            "r51/bundle-validation.json",
            br#"{"result":"pass"}"#,
        )
        .unwrap();
        let cell = |cell_key: &str, result: &str| {
            let (entry_id, device) = cell_key.split_once('/').unwrap();
            R51TerminalCellResult {
                cell_key: cell_key.into(),
                result: result.into(),
                selection_result: Some("selected".into()),
                target_recall: R51TargetRecall {
                    target_total: 1,
                    selected: 1,
                    covered: usize::from(result == "pass"),
                    uncovered: usize::from(result != "pass"),
                },
                pp_han_count: 0,
                vl_han_count: 1,
                rejection_reason: (result != "pass").then(|| "coverage_failure".into()),
                device_evidence_sha256: synthetic_hash(25),
                log_sha256: synthetic_hash(26),
                diagnostic_sha256: synthetic_hash(27),
                target_coverage_index_sha256: Some(synthetic_hash(28)),
                diagnostic_cell_key: format!("holdout/S25L4/{device}/{entry_id}"),
                phase: "holdout".into(),
                candidate_id: "S25L4".into(),
                entry_id: entry_id.into(),
                device: device.into(),
                terminal_reason: (result != "pass").then(|| "coverage_failure".into()),
                diagnostic_path: format!("cells/holdout/S25L4/{device}/{entry_id}/diagnostic.json"),
                diagnostic_byte_length: 1,
                target_coverage_index_path: Some(format!(
                    "cells/holdout/S25L4/{device}/{entry_id}/coverage.json"
                )),
                target_coverage_index_byte_length: Some(1),
                device_evidence_path: format!(
                    "cells/holdout/S25L4/{device}/{entry_id}/device.json"
                ),
                device_evidence_byte_length: 1,
                log_path: format!("cells/holdout/S25L4/{device}/{entry_id}/inference.log"),
                log_byte_length: 1,
            }
        };
        let formal = R51FormalRunEvidence {
            bundle_validation_receipt: Some(bundle),
            cells: vec![
                cell("r51-h01/cpu", "pass"),
                cell("r51-h01/metal", "fail-closed"),
            ],
            first_failed_cell: Some("r51-h01/metal".into()),
        };

        let terminal = write_r51_diagnostic_generations(
            &environment,
            "S25L4",
            &calibration_manifest_sha256,
            &formal,
        )
        .unwrap();
        assert_eq!(terminal.path, "reports/r51/diagnostic-index.json");
        let summary_bytes = fs::read(root.join("reports/r51/completion-summary.json")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&summary_bytes).unwrap();
        assert_eq!(summary_bytes, canonical_json(&value).unwrap());
        assert_eq!(value["result"], "fail-closed");
        assert_eq!(value["failure_kind"], "cell_failure");
        assert_eq!(value["first_failed_cell"], "r51-h01/metal");
        assert_eq!(
            value["cell_results"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| cell["cell_key"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["r51-h01/cpu", "r51-h01/metal"]
        );
        assert_eq!(
            value["unexecuted_cell_keys"],
            serde_json::json!([
                "r51-h02/cpu",
                "r51-h02/metal",
                "r51-h03/cpu",
                "r51-h03/metal",
                "r51-h04/cpu",
                "r51-h04/metal"
            ])
        );
        assert_eq!(value["all_cells_terminated"], false);
        assert_eq!(value["all_cells_passed"], false);
        let published = PublishedArtifact {
            path: "reports/r51/completion-summary.json".into(),
            sha256: sha256_hex(&summary_bytes),
            byte_length: summary_bytes.len() as u64,
        };
        let stdout = r51_completion_summary_stdout_line(&published).unwrap();
        assert!(stdout.starts_with(R51_COMPLETION_SUMMARY_STDOUT_PREFIX));
        assert_eq!(
            serde_json::from_str::<PublishedArtifact>(
                stdout
                    .strip_prefix(R51_COMPLETION_SUMMARY_STDOUT_PREFIX)
                    .unwrap()
            )
            .unwrap(),
            published
        );
        assert!(
            write_r51_diagnostic_generations(
                &environment,
                "S25L4",
                &calibration_manifest_sha256,
                &formal,
            )
            .is_err(),
            "formal evidence must never replace an existing generation"
        );
    }

    #[test]
    fn source_gate_selection_calibration_writes_synced_canonical_pre_holdout_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = valid_environment(&root);

        run_with(
            |name| values.get(name).cloned(),
            &test_repository(),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(calibration_evidence()),
        )
        .unwrap();

        let bytes = fs::read(root.join("selection.json")).unwrap();
        let artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(bytes, canonical_json(&artifact).unwrap());
        assert_ne!(bytes.last(), Some(&b'\n'));
        assert_eq!(artifact.process_evidence.len(), 2);
        assert_eq!(artifact.calibration_results.len(), 32);
        assert_eq!(artifact.required_checks.len(), 1);
        assert_eq!(artifact.holdout_entry_ids, r51_entry_ids('h'));
        assert_eq!(artifact.enabled_cargo_features, ["hanonly-test-evidence"]);
        assert_eq!(
            fs::metadata(root.join("selection.json")).unwrap().mode() & 0o777,
            0o600
        );
        assert_eq!(
            artifact.frozen_recall_contract,
            frozen_recall_contract(&artifact.selected_candidate_id)
        );
        assert!(artifact.holdout_results.is_empty());
        assert!(artifact.holdout_completed_at_utc.is_none());
    }

    #[test]
    fn source_gate_selection_holdout_builds_closed_final_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let mut values = valid_environment(&root);
        run_with(
            |name| values.get(name).cloned(),
            &test_repository(),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(calibration_evidence()),
        )
        .unwrap();
        let calibration_bytes = fs::read(root.join("selection.json")).unwrap();

        values.insert(PHASE_ENV, "holdout".into());
        set_required_check(&mut values, &root, Phase::Holdout);
        run_with(
            |name| values.get(name).cloned(),
            &test_repository(),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(holdout_evidence()),
        )
        .unwrap();

        assert_eq!(
            fs::read(root.join("selection.json")).unwrap(),
            calibration_bytes
        );
        let bytes = fs::read(root.join("selection.json.holdout.json")).unwrap();
        let artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(bytes, canonical_json(&artifact).unwrap());
        assert_eq!(artifact.process_evidence.len(), 4);
        assert_eq!(artifact.calibration_results.len(), 32);
        assert_eq!(artifact.holdout_results.len(), 8);
        assert_eq!(artifact.required_checks.len(), 2);
        assert_eq!(artifact.holdout_entry_ids, r51_entry_ids('h'));
        assert_eq!(artifact.enabled_cargo_features, ["hanonly-test-evidence"]);
        assert!(!artifact.retuned_after_freeze);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from_iter([
                "version".into(),
                "plan_revision".into(),
                "b0_sha".into(),
                "manifest_sha256".into(),
                "holdout_manifest_sha256".into(),
                "source_gate_fixture_manifest_sha256".into(),
                "image_input_contract_sha256".into(),
                "source_color_contract_sha256".into(),
                "color_constant_set_sha256".into(),
                "requested_devices".into(),
                "enabled_cargo_features".into(),
                "backend_evidence_parser_version".into(),
                "required_checks".into(),
                "frozen_recall_contract".into(),
                "candidates".into(),
                "calibration_entry_ids".into(),
                "holdout_entry_ids".into(),
                "process_evidence".into(),
                "calibration_results".into(),
                "selected_candidate_id".into(),
                "frozen_at_utc".into(),
                "frozen_payload_sha256".into(),
                "holdout_results".into(),
                "holdout_completed_at_utc".into(),
                "retuned_after_freeze".into(),
            ])
        );
    }

    #[test]
    fn source_gate_selection_rejects_invalid_candidate_missing_cell_and_device_evidence() {
        for evidence in [
            RunnerEvidence {
                selected_candidate_id: "R100".into(),
                ..calibration_evidence()
            },
            {
                let mut evidence = calibration_evidence();
                evidence.results.pop();
                evidence
            },
            {
                let mut evidence = calibration_evidence();
                evidence.process_evidence[0]
                    .load_evidence
                    .loaded_model_devices
                    .clear();
                evidence
            },
            {
                let mut evidence = calibration_evidence();
                evidence.results[0].execution_evidence.paddle_instance_id = "9".repeat(32);
                evidence
            },
            {
                let mut evidence = calibration_evidence();
                evidence.process_evidence[1].load_evidence.n_gpu_layers = 32;
                evidence
            },
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(temp.path()).unwrap();
            let values = valid_environment(&root);
            assert!(
                run_with(
                    |name| values.get(name).cloned(),
                    &test_repository(),
                    |_| Ok("a".repeat(40)),
                    |_| Ok(()),
                    |_| Ok(evidence),
                )
                .is_err()
            );
            assert!(!root.join("selection.json").exists());
        }
    }

    #[test]
    fn source_gate_selection_writes_calibration_diagnostic_when_no_candidate_passes() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = valid_environment(&root);
        let mut evidence = calibration_evidence();
        for result in &mut evidence.results {
            result.derived.passed = false;
        }

        let error = run_with(
            |name| values.get(name).cloned(),
            &test_repository(),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(evidence),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("no all-pass Source Gate crop candidate")
        );
        assert!(!root.join("selection.json").exists());
        let bytes = fs::read(root.join("calibration-diagnostic.json")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(bytes, canonical_json(&value).unwrap());
        assert_eq!(
            value["schema"],
            "hanonly-source-gate-calibration-diagnostic-v1"
        );
        assert_eq!(value["calibration_results"].as_array().unwrap().len(), 32);
        assert!(
            value["failure"]
                .as_str()
                .unwrap()
                .contains("no all-pass Source Gate crop candidate")
        );
    }

    #[test]
    fn source_gate_selection_rejects_frozen_projection_hash_drift() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = valid_environment(&root);
        run_with(
            |name| values.get(name).cloned(),
            &test_repository(),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(calibration_evidence()),
        )
        .unwrap();

        let bytes = fs::read(root.join("selection.json")).unwrap();
        let mut artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
        artifact.selected_candidate_id = "S25L6".into();
        assert!(
            validate_artifact(&artifact, Phase::CalibrationFreeze, &{
                SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap()
            })
            .is_err()
        );
    }

    #[test]
    fn source_gate_selection_rejects_root_and_escaping_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        for artifact in [root.clone(), root.join("../escape")] {
            let mut values = valid_environment(&root);
            values.insert(ARTIFACT_ENV, artifact.to_string_lossy().into_owned());
            assert!(SelectionEnvironment::parse(|name| values.get(name).cloned()).is_err());
        }
    }

    #[test]
    fn hanonly_test_evidence_bridge_reachable() {
        let accessor: for<'a> fn(
            &'a koharu_ml::aot_inpainting::AotInpainting,
        ) -> &'a koharu_ml::Device = koharu_ml::aot_inpainting::AotInpainting::device;
        let _ = accessor;
    }

    #[test]
    #[ignore = "requires frozen B0 selection environment and installed Source Gate models"]
    fn han_only_source_gate_crop_selection_matrix() {
        let repository = repository_root().expect("repository root");
        run_with(
            |name| std::env::var(name).ok(),
            &repository,
            git_head,
            require_fixture_clean,
            run_real_model,
        )
        .expect("Source Gate selection harness failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use serde_json::json;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    struct TestAssets(HeldInput);

    impl RevalidatedManifestAssets for TestAssets {
        fn with_revalidated_paths<T>(
            &self,
            action: impl FnOnce() -> io::Result<T>,
        ) -> io::Result<T> {
            self.0
                .with_revalidated_path(|validation| validation.with_current_namespace(action))
        }
    }

    fn hash(byte: u8) -> String {
        format!("{:x}", Sha256::digest([byte]))
    }

    fn environment() -> FrozenEnvironment {
        FrozenEnvironment {
            visual_input: "/external/input.png".into(),
            visual_input_sha256: hash(1),
            visual_manifest: "/external/manifest.json".into(),
            visual_manifest_sha256: hash(2),
            evidence_root: "/external/evidence".into(),
            source_gate_fixture_manifest_sha256: hash(3),
        }
    }

    fn ledger_value() -> serde_json::Value {
        let environment = environment();
        json!({
            "version": 1,
            "visual_input": environment.visual_input,
            "visual_input_sha256": environment.visual_input_sha256,
            "visual_manifest": environment.visual_manifest,
            "visual_manifest_sha256": environment.visual_manifest_sha256,
            "source_gate_fixture_manifest_sha256":
                environment.source_gate_fixture_manifest_sha256,
            "evidence_root": environment.evidence_root,
        })
    }

    fn report_fixture() -> (EvidenceLedger, String, HarnessSummary) {
        let environment = environment();
        let ledger = EvidenceLedger::parse_and_validate(
            &serde_json::to_vec(&ledger_value()).unwrap(),
            &environment,
        )
        .unwrap();
        (
            ledger,
            hash(4),
            HarnessSummary {
                entries: 9,
                targets: 12,
                masks: 24,
                protected_rois: 3,
                retained_bytes: 456,
            },
        )
    }

    fn orchestration_fixture() -> (
        TempDir,
        PathBuf,
        PathBuf,
        HeldInput,
        HeldInput,
        HeldInput,
        TestAssets,
    ) {
        let temp = TempDir::new().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let mut held = Vec::new();
        let mut paths = Vec::new();
        for (name, bytes) in [
            ("evidence-ledger.json", b"ledger".as_slice()),
            ("selected-input", b"selected".as_slice()),
            ("fixture-manifest.json", b"fixture".as_slice()),
            ("manifest-asset", b"asset".as_slice()),
        ] {
            let path = root.join(name);
            fs::write(&path, bytes).unwrap();
            held.push(HeldInput::open(&path).unwrap());
            paths.push(path);
        }
        let assets = TestAssets(held.pop().unwrap());
        let fixture = held.pop().unwrap();
        let selected = held.pop().unwrap();
        let ledger = held.pop().unwrap();
        (
            temp,
            root,
            paths.remove(0),
            ledger,
            selected,
            fixture,
            assets,
        )
    }

    #[test]
    fn d0_visual_manifest_harness_ledger_schema_and_environment_are_closed() {
        let environment = environment();
        EvidenceLedger::parse_and_validate(
            &serde_json::to_vec(&ledger_value()).unwrap(),
            &environment,
        )
        .unwrap();

        let mut missing = ledger_value();
        missing.as_object_mut().unwrap().remove("visual_input");
        assert!(
            EvidenceLedger::parse_and_validate(
                &serde_json::to_vec(&missing).unwrap(),
                &environment
            )
            .is_err()
        );

        let mut unknown = ledger_value();
        unknown["unexpected"] = true.into();
        assert!(
            EvidenceLedger::parse_and_validate(
                &serde_json::to_vec(&unknown).unwrap(),
                &environment
            )
            .is_err()
        );

        for (field, value) in [
            ("version", json!(2)),
            ("visual_input_sha256", json!("A".repeat(64))),
            ("visual_manifest_sha256", json!("0".repeat(63))),
            ("source_gate_fixture_manifest_sha256", json!("g".repeat(64))),
        ] {
            let mut invalid = ledger_value();
            invalid[field] = value;
            assert!(
                EvidenceLedger::parse_and_validate(
                    &serde_json::to_vec(&invalid).unwrap(),
                    &environment
                )
                .is_err(),
                "{field}"
            );
        }

        for field in [
            "visual_input",
            "visual_input_sha256",
            "visual_manifest",
            "visual_manifest_sha256",
            "source_gate_fixture_manifest_sha256",
            "evidence_root",
        ] {
            let mut mismatch = ledger_value();
            mismatch[field] = if field.ends_with("sha256") {
                json!("f".repeat(64))
            } else {
                json!("/external/other")
            };
            assert!(
                EvidenceLedger::parse_and_validate(
                    &serde_json::to_vec(&mismatch).unwrap(),
                    &environment
                )
                .is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn d0_visual_manifest_harness_anonymous_socket_owner_matches_current_user_artifact() {
        let temp = TempDir::new().unwrap();
        let artifact_owner = u64::from(fs::metadata(temp.path()).unwrap().uid());
        assert_eq!(effective_owner().unwrap(), artifact_owner);
    }

    #[test]
    fn d0_visual_manifest_harness_report_is_canonical_closed_and_sanitized() {
        let (ledger, decoded, summary) = report_fixture();
        let bytes = canonical_report(&ledger, &decoded, summary).unwrap();
        let expected = format!(
            concat!(
                "{{\"schema\":\"hanonly-d0-manifest-preflight-v1\",",
                "\"image_input_contract\":\"image-input-contract-v1\",",
                "\"visual_input_sha256\":\"{}\",",
                "\"visual_input_decoded_rgba_blake3\":\"{}\",",
                "\"visual_manifest_sha256\":\"{}\",",
                "\"source_gate_fixture_manifest_sha256\":\"{}\",",
                "\"entries\":9,\"targets\":12,\"masks\":24,",
                "\"protected_rois\":3,\"retained_bytes\":456}}\n"
            ),
            ledger.visual_input_sha256,
            decoded,
            ledger.visual_manifest_sha256,
            ledger.source_gate_fixture_manifest_sha256,
        );
        assert_eq!(bytes, expected.as_bytes());

        let report: ManifestPreflightReport = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report.schema, "hanonly-d0-manifest-preflight-v1");
        assert_eq!(report.image_input_contract, "image-input-contract-v1");
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 11);
        for forbidden in [
            "path",
            "text",
            "node",
            "node_id",
            "line_count",
            "text_count",
            "glyph_count",
            "target_id",
        ] {
            assert!(!object.contains_key(forbidden));
            assert!(!expected.contains("/external/"));
        }

        let mut unknown = value.clone();
        unknown["unexpected"] = true.into();
        assert!(serde_json::from_value::<ManifestPreflightReport>(unknown).is_err());
        let mut missing = value;
        missing.as_object_mut().unwrap().remove("schema");
        assert!(serde_json::from_value::<ManifestPreflightReport>(missing).is_err());
    }

    #[test]
    fn d0_visual_manifest_harness_validation_failure_creates_no_output() {
        let (_temp, root, _ledger_path, ledger, selected, fixture, assets) =
            orchestration_fixture();
        let publisher_fault_calls = Cell::new(0);
        let success_calls = Cell::new(0);
        let revalidation = PreflightRevalidation {
            ledger: &ledger,
            selected_input: &selected,
            fixture: &fixture,
            assets: &assets,
        };
        let result = publish_revalidated_report(
            &revalidation,
            &root,
            b"{\"validated\":true}\n",
            || Err(io::Error::other("late validation failed")),
            &mut |_| {
                publisher_fault_calls.set(publisher_fault_calls.get() + 1);
                Ok(())
            },
            |_| {
                success_calls.set(success_calls.get() + 1);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(publisher_fault_calls.get(), 0);
        assert_eq!(success_calls.get(), 0);
        assert!(!root.join("d0-manifest-preflight").exists());
    }

    #[test]
    fn d0_visual_manifest_harness_postpublication_revalidation_blocks_ledger_race() {
        let (_temp, root, ledger_path, ledger, selected, fixture, assets) = orchestration_fixture();
        let replaced = Cell::new(false);
        let success_calls = Cell::new(0);
        let revalidation = PreflightRevalidation {
            ledger: &ledger,
            selected_input: &selected,
            fixture: &fixture,
            assets: &assets,
        };
        let result = publish_revalidated_report(
            &revalidation,
            &root,
            b"{\"validated\":true}\n",
            || Ok(()),
            &mut |point| {
                if point == FaultPoint::DirectoryFsync && !replaced.replace(true) {
                    fs::rename(&ledger_path, ledger_path.with_file_name("old-ledger"))?;
                    fs::write(&ledger_path, b"ledger")?;
                }
                Ok(())
            },
            |_| {
                success_calls.set(success_calls.get() + 1);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(replaced.get());
        assert_eq!(success_calls.get(), 0);
        assert!(
            root.join("d0-manifest-preflight")
                .join("report.json")
                .is_file()
        );
    }

    #[test]
    fn d0_visual_manifest_harness_requires_secure_external_evidence_root() {
        let temp = TempDir::new().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let ledger_path = root.join(LEDGER_NAME);
        fs::write(&ledger_path, b"{}").unwrap();
        fs::set_permissions(&ledger_path, fs::Permissions::from_mode(0o600)).unwrap();
        let owner = u64::from(fs::metadata(&ledger_path).unwrap().uid());

        let ledger = HeldInput::open(&ledger_path).unwrap();
        ledger
            .require_file_and_parent_security(owner, 0o600, 0o700)
            .unwrap();
        assert!(
            ledger
                .require_file_and_parent_security(owner.wrapping_add(1), 0o600, 0o700)
                .is_err()
        );

        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        let insecure = HeldInput::open(&ledger_path).unwrap();
        assert!(
            insecure
                .require_file_and_parent_security(owner, 0o600, 0o700)
                .is_err()
        );
        assert!(require_absolute_canonical(Path::new(&format!("{}/.", root.display()))).is_err());
        assert!(
            require(
                !root.starts_with(&root),
                "evidence root must be outside the repository"
            )
            .is_err()
        );
        assert!(
            require(
                root.join("nested").join(LEDGER_NAME).parent() == Some(root.as_path()),
                "ledger parent must be the evidence root"
            )
            .is_err()
        );
    }

    #[test]
    fn d0_visual_manifest_harness_fixture_schema_and_git_status_are_closed() {
        validate_fixture_manifest(br#"{"fixtures":[{}]}"#).unwrap();
        assert!(validate_fixture_manifest(br#"[]"#).is_err());
        assert!(validate_fixture_manifest(br#"{"fixtures":[]}"#).is_err());
        assert!(validate_fixture_manifest(br#"{"fixtures":{}}"#).is_err());
        require_clean_status(true, b"").unwrap();
        assert!(require_clean_status(false, b"").is_err());
        assert!(require_clean_status(true, b" M fixed.json\n").is_err());
    }
}
