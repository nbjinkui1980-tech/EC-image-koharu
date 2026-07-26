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
    use super::*;
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fs::OpenOptions;
    use std::io::Write;

    const PHASE_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_PHASE";
    const B0_SHA_ENV: &str = "HANONLY_B0_SHA";
    const ARTIFACT_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_ARTIFACT";
    const REPORT_DIR_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_REPORT_DIR";
    const ARTIFACT_VERSION: u32 = 1;
    const PLAN_REVISION: u32 = 46;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Phase {
        CalibrationFreeze,
        Holdout,
    }

    struct SelectionEnvironment {
        phase: Phase,
        b0_sha: String,
        visual_manifest_sha256: String,
        source_gate_fixture_manifest_sha256: String,
        artifact: PathBuf,
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
            let source_gate_fixture_manifest_sha256 =
                required(&mut get, SOURCE_GATE_FIXTURE_SHA256_ENV)?;
            decode_sha256(&source_gate_fixture_manifest_sha256)?;
            let evidence_root = PathBuf::from(required(&mut get, VISUAL_EVIDENCE_ROOT_ENV)?);
            require_absolute_canonical(&evidence_root)?;
            let artifact = PathBuf::from(required(&mut get, ARTIFACT_ENV)?);
            let report_dir = PathBuf::from(required(&mut get, REPORT_DIR_ENV)?);
            require_future_path_below(&evidence_root, &artifact)?;
            require_future_path_below(&evidence_root, &report_dir)?;
            if report_dir.exists() {
                require(
                    report_dir.is_dir(),
                    "selection report path must be a directory",
                )?;
            }
            Ok(Self {
                phase,
                b0_sha,
                visual_manifest_sha256,
                source_gate_fixture_manifest_sha256,
                artifact,
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
                let mut artifact = FrozenArtifact {
                    version: ARTIFACT_VERSION,
                    plan_revision: PLAN_REVISION,
                    b0_sha: environment.b0_sha.clone(),
                    manifest_sha256: environment.visual_manifest_sha256.clone(),
                    source_gate_fixture_manifest_sha256: environment
                        .source_gate_fixture_manifest_sha256
                        .clone(),
                    image_input_contract_sha256: synthetic_hash(21),
                    source_color_contract_sha256: synthetic_hash(22),
                    color_constant_set_sha256: synthetic_hash(23),
                    requested_devices: vec!["cpu".into(), "metal".into()],
                    enabled_cargo_features: vec!["hanonly-test-evidence".into(), "metal".into()],
                    backend_evidence_parser_version: 1,
                    candidates: candidates_schema(),
                    calibration_entry_ids: entry_ids("calibration"),
                    holdout_entry_ids: entry_ids("holdout"),
                    process_evidence: evidence.process_evidence,
                    calibration_results: evidence.results,
                    selected_candidate_id: evidence.selected_candidate_id,
                    frozen_at_utc: "2026-07-26T00:00:00Z".into(),
                    frozen_payload_sha256: synthetic_hash(24),
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
                artifact.process_evidence.extend(evidence.process_evidence);
                artifact.holdout_results = evidence.results;
                artifact.holdout_completed_at_utc = Some("2026-07-26T00:01:00Z".into());
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

    fn entry_ids(phase: &str) -> Vec<String> {
        (0..4).map(|index| format!("{phase}-{index}")).collect()
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
                && artifact.calibration_entry_ids == entry_ids("calibration")
                && artifact.holdout_entry_ids == entry_ids("holdout")
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
                    && load.n_gpu_layers > 0
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

    fn valid_environment(root: &Path) -> HashMap<&'static str, String> {
        HashMap::from([
            (PHASE_ENV, "calibration-freeze".into()),
            (B0_SHA_ENV, "a".repeat(40)),
            (
                VISUAL_EVIDENCE_ROOT_ENV,
                root.to_string_lossy().into_owned(),
            ),
            (VISUAL_MANIFEST_SHA256_ENV, "1".repeat(64)),
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
                n_gpu_layers: if metal { 32 } else { 0 },
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
            selected_candidate_id: "R05".into(),
            process_evidence: ["cpu", "metal"]
                .map(|device| synthetic_process("calibration", device))
                .into(),
            results: entry_ids("calibration")
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
            selected_candidate_id: "R05".into(),
            process_evidence: ["cpu", "metal"]
                .map(|device| synthetic_process("holdout", device))
                .into(),
            results: entry_ids("holdout")
                .iter()
                .flat_map(|entry_id| {
                    ["cpu", "metal"]
                        .into_iter()
                        .map(move |device| synthetic_result("holdout", entry_id, device, "R05"))
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
        assert!(validate_artifact(&artifact, Phase::CalibrationFreeze, &{
            SelectionEnvironment::parse(|name| values.get(name).cloned()).unwrap()
        })
        .is_err());
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
    #[ignore = "requires frozen B0 selection environment and installed Source Gate models"]
    fn han_only_source_gate_crop_selection_matrix() {
        let repository = repository_root().expect("repository root");
        run_with(
            |name| std::env::var(name).ok(),
            &repository,
            git_head,
            require_fixture_clean,
            |_| {
                Err(io::Error::other(
                    "Source Gate model runner is not implemented",
                ))
            },
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
