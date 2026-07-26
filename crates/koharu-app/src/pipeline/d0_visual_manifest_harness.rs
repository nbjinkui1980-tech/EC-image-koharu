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
    use super::super::d0_visual_manifest_oracles::{
        OracleValidatedEntry, OracleValidatedManifest, ValidatedHalfOpenRect,
    };
    use super::super::d0_visual_manifest_schema::{EntryRole, Expected, VisualManifestEntry};
    use super::super::engines::source_language_gate::{
        SourceGateCropPolicy, SourceGateCropPolicyGuard, dispatch_source_gate,
    };
    use super::super::engines::support::SOURCE_GATE_TARGET_DETECTOR;
    use super::*;
    use chrono::{SecondsFormat, Utc};
    use image::DynamicImage;
    use koharu_core::{Node, NodeId, NodeKind, Page, Scene, TextData, Transform};
    use koharu_llm::NativeLogCaptureGuard;
    use koharu_llm::paddleocr_vl::{PaddleOcrVl, PaddleOcrVlTask};
    use koharu_llm::safe::{LlamaBackendDeviceType, list_llama_ggml_backend_devices};
    use koharu_ml::pp_ocr_v5::PpOcrV5;
    use koharu_runtime::{ComputePolicy, RuntimeManager, default_app_data_root};
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

    const PHASE_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_PHASE";
    const B0_SHA_ENV: &str = "HANONLY_B0_SHA";
    const ARTIFACT_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_ARTIFACT";
    const REPORT_DIR_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_REPORT_DIR";
    const ARTIFACT_VERSION: u32 = 1;
    const PLAN_REVISION: u32 = 46;
    const B0_DEFAULT_GPU_LAYERS: u32 = 1000;
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
        b0_sha: String,
        visual_input: PathBuf,
        visual_input_sha256: String,
        visual_manifest: PathBuf,
        visual_manifest_sha256: String,
        evidence_root: PathBuf,
        report_dir: PathBuf,
        source_gate_fixture_manifest_sha256: String,
        artifact: PathBuf,
        calibration_entry_ids: Vec<String>,
        holdout_entry_ids: Vec<String>,
    }

    impl SelectionEnvironment {
        fn parse(mut get: impl FnMut(&str) -> Option<String>) -> io::Result<Self> {
            let phase = match required(&mut get, PHASE_ENV)?.as_str() {
                "calibration-freeze" => Phase::CalibrationFreeze,
                "holdout" => Phase::Holdout,
                _ => return Err(invalid_data("invalid Source Gate selection phase")),
            };
            let b0_sha = required(&mut get, B0_SHA_ENV)?;
            require(
                b0_sha.len() == 40
                    && b0_sha
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "B0 sha must be 40 lowercase hex characters",
            )?;
            let visual_manifest_sha256 = required(&mut get, VISUAL_MANIFEST_SHA256_ENV)?;
            decode_sha256(&visual_manifest_sha256)?;
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
            require(
                calibration_entry_ids.len() == 4
                    && holdout_entry_ids.len() == 4
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
            Ok(Self {
                phase,
                b0_sha,
                visual_input,
                visual_input_sha256,
                visual_manifest,
                visual_manifest_sha256,
                evidence_root,
                report_dir,
                source_gate_fixture_manifest_sha256,
                artifact,
                calibration_entry_ids,
                holdout_entry_ids,
            })
        }
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Candidate {
        id: String,
        numerator: u32,
        denominator: u32,
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
            .ok_or_else(|| invalid_data("native buffer log omitted size"))?
            .parse::<f64>()
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
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
        source_gate_fixture_manifest_sha256: String,
        image_input_contract_sha256: String,
        source_color_contract_sha256: String,
        color_constant_set_sha256: String,
        requested_devices: Vec<String>,
        enabled_cargo_features: Vec<String>,
        backend_evidence_parser_version: u32,
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
                let selected_candidate_id = select_smallest_all_pass(
                    &evidence.results,
                    &environment.calibration_entry_ids,
                )?;
                require(
                    evidence.selected_candidate_id == selected_candidate_id,
                    "runner selected candidate does not match independent selection",
                )?;
                let mut artifact = FrozenArtifact {
                    version: ARTIFACT_VERSION,
                    plan_revision: PLAN_REVISION,
                    b0_sha: environment.b0_sha.clone(),
                    manifest_sha256: environment.visual_manifest_sha256.clone(),
                    source_gate_fixture_manifest_sha256: environment
                        .source_gate_fixture_manifest_sha256
                        .clone(),
                    image_input_contract_sha256:
                        super::super::d0_revision_46_contract::image_input_contract_sha256(),
                    source_color_contract_sha256: SOURCE_COLOR_CONTRACT_SHA256.into(),
                    color_constant_set_sha256: COLOR_CONSTANT_SET_SHA256.into(),
                    requested_devices: vec!["cpu".into(), "metal".into()],
                    enabled_cargo_features: vec!["hanonly-test-evidence".into(), "metal".into()],
                    backend_evidence_parser_version: 1,
                    candidates: candidates_schema(),
                    calibration_entry_ids: environment.calibration_entry_ids.clone(),
                    holdout_entry_ids: environment.holdout_entry_ids.clone(),
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
                write_artifact(&environment.artifact, &canonical_json(&artifact)?, false)
            }
            Phase::Holdout => {
                let bytes = fs::read(&environment.artifact)?;
                let mut artifact: FrozenArtifact = serde_json::from_slice(&bytes)
                    .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
                require(
                    canonical_json(&artifact)? == bytes,
                    "selection artifact must be canonical JSON",
                )?;
                validate_artifact(&artifact, Phase::CalibrationFreeze, &environment)?;
                let evidence = model_runner(&environment)?;
                require(
                    evidence.selected_candidate_id == artifact.selected_candidate_id,
                    "holdout selected candidate drift",
                )?;
                require(
                    evidence.results.iter().all(|result| result.derived.passed),
                    "holdout result failed",
                )?;
                artifact.process_evidence.extend(evidence.process_evidence);
                artifact.holdout_results = evidence.results;
                let frozen_at = chrono::DateTime::parse_from_rfc3339(&artifact.frozen_at_utc)
                    .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?
                    .with_timezone(&Utc);
                let completed_at = Utc::now().max(frozen_at + chrono::Duration::seconds(1));
                artifact.holdout_completed_at_utc =
                    Some(completed_at.to_rfc3339_opts(SecondsFormat::Secs, true));
                validate_artifact(&artifact, Phase::Holdout, &environment)?;
                write_artifact(&environment.artifact, &canonical_json(&artifact)?, true)
            }
        }
    }

    fn candidates_schema() -> Vec<Candidate> {
        [
            ("R0", 0, 1),
            ("R025", 1, 40),
            ("R05", 1, 20),
            ("R10", 1, 10),
        ]
        .into_iter()
        .map(|(id, numerator, denominator)| Candidate {
            id: id.into(),
            numerator,
            denominator,
        })
        .collect()
    }

    fn run_real_model(environment: &SelectionEnvironment) -> io::Result<RunnerEvidence> {
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
        validated.upstream.held_schema.with_revalidated_paths(|| {
            runtime.block_on(run_real_model_async(environment, &validated))
        })
    }

    async fn run_real_model_async(
        environment: &SelectionEnvironment,
        manifest: &OracleValidatedManifest,
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
        let executable_sha256 = sha256_file(&std::env::current_exe()?)?;
        let runtime_library_sha256 = runtime_library_hashes(&runtime)?;
        let phase = phase_name(environment.phase);
        let selected = selected_candidates(environment)?;
        let mut process_evidence = Vec::with_capacity(2);
        let mut results = Vec::new();

        for (device, cpu) in [("cpu", true), ("metal", false)] {
            let mut logs = NativeLogCaptureGuard::start();
            let mut vl = PaddleOcrVl::load(&runtime, cpu, backend.clone())
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

            for (schema_entry, decoded_entry, oracle_entry) in manifest
                .upstream
                .held_schema
                .schema
                .entries
                .iter()
                .zip(&manifest.upstream.entries)
                .zip(&manifest.entries)
                .map(|((schema, decoded), oracle)| (schema, decoded, oracle))
                .filter(|(entry, _, _)| entry.role == phase_role(environment.phase))
            {
                for (candidate_id, policy) in &selected {
                    logs.clear();
                    let mut scene = scene_for_entry(schema_entry, oracle_entry);
                    let page = *scene.pages.keys().next().expect("scene page");
                    let image = DynamicImage::ImageRgba8(decoded_entry.source.clone());
                    let _policy = SourceGateCropPolicyGuard::set(*policy);
                    let ops = dispatch_source_gate(
                        &image,
                        &scene,
                        page,
                        |_, crop| pp.word_boxes(crop),
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
                    let (runtime_nodes, derived) =
                        derive_result(device, &scene, page, schema_entry, oracle_entry)?;
                    results.push(SelectionResult {
                        entry_id: schema_entry.id.clone(),
                        process_evidence_id: process_id.clone(),
                        candidate_id: candidate_id.clone(),
                        execution_evidence: ExecutionEvidence {
                            paddle_instance_id: vl_evidence.instance_id.clone(),
                            context_offload_kqv: vl_evidence.context_offload_kqv,
                            context_op_offload: vl_evidence.context_op_offload,
                            inference_completed: true,
                            raw_inference_log_relpath: inference_log.0,
                            raw_inference_log_sha256: inference_log.1,
                            context_buffer_bytes_by_backend: parsed_inference
                                .context_buffer_bytes_by_backend,
                            compute_buffer_bytes_by_backend: parsed_inference
                                .compute_buffer_bytes_by_backend,
                        },
                        runtime_nodes,
                        derived,
                    });
                }
            }
            process_evidence.push(process);
        }
        let entry_ids = match environment.phase {
            Phase::CalibrationFreeze => &environment.calibration_entry_ids,
            Phase::Holdout => &environment.holdout_entry_ids,
        };
        let selected_candidate_id = if environment.phase == Phase::CalibrationFreeze {
            select_smallest_all_pass(&results, entry_ids)?
        } else {
            selected[0].0.clone()
        };
        Ok(RunnerEvidence {
            selected_candidate_id,
            process_evidence,
            results,
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

    fn selected_candidates(
        environment: &SelectionEnvironment,
    ) -> io::Result<Vec<(String, SourceGateCropPolicy)>> {
        let all = [
            ("R0", SourceGateCropPolicy::R0),
            ("R025", SourceGateCropPolicy::R025),
            ("R05", SourceGateCropPolicy::R05),
            ("R10", SourceGateCropPolicy::R10),
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

    fn scene_for_entry(schema: &VisualManifestEntry, oracle: &OracleValidatedEntry) -> Scene {
        let width = oracle
            .targets
            .iter()
            .map(|target| target.source_roi.right)
            .chain(oracle.protected_rois.iter().map(|roi| roi.right))
            .max()
            .unwrap_or(1);
        let height = oracle
            .targets
            .iter()
            .map(|target| target.source_roi.bottom)
            .chain(oracle.protected_rois.iter().map(|roi| roi.bottom))
            .max()
            .unwrap_or(1);
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

    fn derive_result(
        device: &str,
        scene: &Scene,
        page: koharu_core::PageId,
        schema: &VisualManifestEntry,
        oracle: &OracleValidatedEntry,
    ) -> io::Result<(Vec<RuntimeNode>, DerivedEvidence)> {
        let mut runtime_nodes = Vec::new();
        let mut matched = HashSet::new();
        let mut selected = HashSet::new();
        let mut selected_protected = Vec::new();
        let mut unmatched_selected = Vec::new();
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
        let passed = recall == 1.0
            && selected_protected.is_empty()
            && unmatched_selected.is_empty()
            && rotation_targets_excluded;
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
                passed,
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
                let backend = canonical_device_backend(&device.backend)
                    .ok_or_else(|| invalid_data("unsupported enumerated backend"))?;
                Ok(EnumeratedDevice {
                    index: u32::try_from(device.index)
                        .map_err(|_| invalid_data("device index overflow"))?,
                    name: device.name,
                    description: device.description,
                    backend: backend.into(),
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
                    .find(|device| device.backend == *backend)
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
        if lower.contains("metal") {
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
        for candidate in candidates_schema() {
            let cells = results
                .iter()
                .filter(|result| result.candidate_id == candidate.id)
                .collect::<Vec<_>>();
            let expected_cells = entry_ids.len() * 2;
            if cells.len() != expected_cells || !cells.iter().all(|result| result.derived.passed) {
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
        Err(invalid_data("no all-pass Source Gate crop candidate"))
    }

    fn synthetic_entry_ids(phase: &str) -> Vec<String> {
        let prefix = match phase {
            "calibration" => "c",
            "holdout" => "h",
            _ => unreachable!("synthetic phase is closed"),
        };
        (1..=4).map(|index| format!("{prefix}{index:02}")).collect()
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
                && artifact.manifest_sha256 == environment.visual_manifest_sha256
                && artifact.source_gate_fixture_manifest_sha256
                    == environment.source_gate_fixture_manifest_sha256,
            "selection artifact frozen input drift",
        )?;
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
                && artifact.enabled_cargo_features == ["hanonly-test-evidence", "metal"]
                && artifact.backend_evidence_parser_version == 1
                && artifact.calibration_entry_ids == environment.calibration_entry_ids
                && artifact.holdout_entry_ids == environment.holdout_entry_ids
                && !artifact.retuned_after_freeze,
            "candidate ratios drift",
        )?;
        for hash in [
            &artifact.image_input_contract_sha256,
            &artifact.source_color_contract_sha256,
            &artifact.color_constant_set_sha256,
            &artifact.frozen_payload_sha256,
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
        require(
            process.phase == phase
                && execution.paddle_instance_id == process.paddle_instance_id
                && execution.inference_completed
                && result.derived.actual_device == process.requested_device,
            "selection result instance or device mismatch",
        )?;
        decode_sha256(&execution.raw_inference_log_sha256)?;
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
        output.push(b'\n');
        Ok(output)
    }

    fn write_artifact(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
        require(parent.is_dir(), "selection artifact parent must exist")?;
        if replace {
            let temporary = path.with_extension("tmp");
            let result = (|| {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                file.write_all(bytes)?;
                file.sync_all()?;
                fs::rename(&temporary, path)?;
                OpenOptions::new().read(true).open(parent)?.sync_all()
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            result
        } else {
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            OpenOptions::new().read(true).open(parent)?.sync_all()
        }
    }

    type RasterBounds = (u32, u32, u32, u32);

    fn candidates(
        bbox: (f64, f64, f64, f64),
        page: (u32, u32),
    ) -> [(&'static str, RasterBounds); 4] {
        const RATIOS: [(&str, u32, u32); 4] = [
            ("R0", 0, 1),
            ("R025", 1, 40),
            ("R05", 1, 20),
            ("R10", 1, 10),
        ];
        let short_side = (bbox.2 - bbox.0).min(bbox.3 - bbox.1);
        RATIOS.map(|(name, numerator, denominator)| {
            let padding = if numerator == 0 {
                0.0
            } else {
                (short_side * f64::from(numerator) / f64::from(denominator))
                    .ceil()
                    .max(1.0)
            };
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
            candidates((10.2, 20.2, 30.8, 30.8), (100, 100)),
            [
                ("R0", (10, 20, 31, 31)),
                ("R025", (9, 19, 32, 32)),
                ("R05", (9, 19, 32, 32)),
                ("R10", (8, 18, 33, 33)),
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
                name: "Metal0".into(),
                description: "Apple GPU".into(),
                backend: "Metal".into(),
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

    fn valid_environment(root: &Path) -> HashMap<&'static str, String> {
        let manifest = root.join("visual-manifest.json");
        let manifest_bytes = serde_json::to_vec(&serde_json::json!({
            "entries": [
                {"id": "c01", "role": "calibration"},
                {"id": "c02", "role": "calibration"},
                {"id": "c03", "role": "calibration"},
                {"id": "c04", "role": "calibration"},
                {"id": "h01", "role": "holdout"},
                {"id": "h02", "role": "holdout"},
                {"id": "h03", "role": "holdout"},
                {"id": "h04", "role": "holdout"}
            ]
        }))
        .unwrap();
        fs::write(&manifest, &manifest_bytes).unwrap();
        HashMap::from([
            (PHASE_ENV, "calibration-freeze".into()),
            (B0_SHA_ENV, "a".repeat(40)),
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
            (VISUAL_MANIFEST_SHA256_ENV, sha256_hex(&manifest_bytes)),
            (SOURCE_GATE_FIXTURE_SHA256_ENV, "2".repeat(64)),
            (
                ARTIFACT_ENV,
                root.join("selection.json").to_string_lossy().into_owned(),
            ),
            (
                REPORT_DIR_ENV,
                root.join("reports").to_string_lossy().into_owned(),
            ),
        ])
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
                passed: true,
            },
        }
    }

    fn calibration_evidence() -> RunnerEvidence {
        RunnerEvidence {
            selected_candidate_id: "R0".into(),
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
        }
    }

    fn holdout_evidence() -> RunnerEvidence {
        RunnerEvidence {
            selected_candidate_id: "R0".into(),
            process_evidence: ["cpu", "metal"]
                .map(|device| synthetic_process("holdout", device))
                .into(),
            results: synthetic_entry_ids("holdout")
                .iter()
                .flat_map(|entry_id| {
                    ["cpu", "metal"]
                        .into_iter()
                        .map(move |device| synthetic_result("holdout", entry_id, device, "R0"))
                })
                .collect(),
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
                Path::new("/repository"),
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
            Path::new("/repository"),
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
    fn source_gate_selection_calibration_writes_synced_canonical_pre_holdout_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = valid_environment(&root);

        run_with(
            |name| values.get(name).cloned(),
            Path::new("/repository"),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(calibration_evidence()),
        )
        .unwrap();

        let bytes = fs::read(root.join("selection.json")).unwrap();
        let artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(bytes, canonical_json(&artifact).unwrap());
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert_eq!(artifact.process_evidence.len(), 2);
        assert_eq!(artifact.calibration_results.len(), 32);
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
            Path::new("/repository"),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(calibration_evidence()),
        )
        .unwrap();

        values.insert(PHASE_ENV, "holdout".into());
        run_with(
            |name| values.get(name).cloned(),
            Path::new("/repository"),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(holdout_evidence()),
        )
        .unwrap();

        let bytes = fs::read(root.join("selection.json")).unwrap();
        let artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(bytes, canonical_json(&artifact).unwrap());
        assert_eq!(artifact.process_evidence.len(), 4);
        assert_eq!(artifact.calibration_results.len(), 32);
        assert_eq!(artifact.holdout_results.len(), 8);
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
                "source_gate_fixture_manifest_sha256".into(),
                "image_input_contract_sha256".into(),
                "source_color_contract_sha256".into(),
                "color_constant_set_sha256".into(),
                "requested_devices".into(),
                "enabled_cargo_features".into(),
                "backend_evidence_parser_version".into(),
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
                    Path::new("/repository"),
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
    fn source_gate_selection_rejects_frozen_projection_hash_drift() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let values = valid_environment(&root);
        run_with(
            |name| values.get(name).cloned(),
            Path::new("/repository"),
            |_| Ok("a".repeat(40)),
            |_| Ok(()),
            |_| Ok(calibration_evidence()),
        )
        .unwrap();

        let bytes = fs::read(root.join("selection.json")).unwrap();
        let mut artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
        artifact.selected_candidate_id = "R10".into();
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
