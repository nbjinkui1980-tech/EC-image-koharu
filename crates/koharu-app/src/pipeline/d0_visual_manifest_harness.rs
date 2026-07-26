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
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fs::OpenOptions;
    use std::io::Write;

    const PHASE_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_PHASE";
    const B0_SHA_ENV: &str = "HANONLY_B0_SHA";
    const ARTIFACT_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_ARTIFACT";
    const REPORT_DIR_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_REPORT_DIR";
    const ARTIFACT_VERSION: &str = "hanonly-b0-frozen-artifact-v1";
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
                matches!(b0_sha.len(), 40 | 64)
                    && b0_sha
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "B0 sha must be 40 or 64 lowercase hex characters",
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

    #[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(rename_all = "lowercase")]
    enum EntryRole {
        Regression,
        Calibration,
        Holdout,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct ModelEvidence {
        provider: String,
        backend: String,
        identifier_sha256: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct RuntimeEvidence {
        os: String,
        device: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct DeviceEvidence {
        actual_device: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct RawEvidence {
        load_device: String,
        model_backend: String,
        layer_or_buffer: String,
        context_or_offload: String,
        runtime_node_count: u32,
        device_load_confirmed: bool,
        diagnostic_sha256: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct OutputHashes {
        source: String,
        segment_mask: String,
        inpainted: String,
        rendered: String,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Assertions {
        source_text_removed: bool,
        target_rendered: bool,
        protected_pixels_preserved: bool,
        english_roi_preserved: bool,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct SelectionEntry {
        case_id: String,
        input_sha256: String,
        role: EntryRole,
        #[serde(skip_serializing_if = "Option::is_none")]
        selected_candidate_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase_result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<ModelEvidence>,
        #[serde(skip_serializing_if = "Option::is_none")]
        runtime: Option<RuntimeEvidence>,
        #[serde(skip_serializing_if = "Option::is_none")]
        device: Option<DeviceEvidence>,
        #[serde(skip_serializing_if = "Option::is_none")]
        raw_evidence: Option<RawEvidence>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_hashes: Option<OutputHashes>,
        #[serde(skip_serializing_if = "Option::is_none")]
        assertions: Option<Assertions>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RunnerEvidence {
        selected_candidate_id: String,
        entries: Vec<SelectionEntry>,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
    #[serde(deny_unknown_fields)]
    struct FrozenArtifact {
        version: String,
        plan_revision: u32,
        b0_sha: String,
        visual_manifest_sha256: String,
        source_gate_fixture_manifest_sha256: String,
        selected_candidate_id: String,
        candidate_ratios: BTreeMap<String, String>,
        entries: Vec<SelectionEntry>,
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
                let artifact = FrozenArtifact {
                    version: ARTIFACT_VERSION.into(),
                    plan_revision: PLAN_REVISION,
                    b0_sha: environment.b0_sha.clone(),
                    visual_manifest_sha256: environment.visual_manifest_sha256.clone(),
                    source_gate_fixture_manifest_sha256: environment
                        .source_gate_fixture_manifest_sha256
                        .clone(),
                    selected_candidate_id: evidence.selected_candidate_id,
                    candidate_ratios: candidate_ratios(),
                    entries: evidence.entries,
                };
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
                artifact.entries.extend(evidence.entries);
                validate_artifact(&artifact, Phase::Holdout, &environment)?;
                write_artifact(&environment.artifact, &canonical_json(&artifact)?, true)
            }
        }
    }

    fn candidate_ratios() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("R0".into(), "0".into()),
            ("R025".into(), "1/40".into()),
            ("R05".into(), "1/20".into()),
            ("R10".into(), "1/10".into()),
        ])
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
                && artifact.visual_manifest_sha256 == environment.visual_manifest_sha256
                && artifact.source_gate_fixture_manifest_sha256
                    == environment.source_gate_fixture_manifest_sha256,
            "selection artifact frozen input drift",
        )?;
        require(
            candidate_ratios().contains_key(&artifact.selected_candidate_id),
            "invalid selected candidate",
        )?;
        require(
            artifact.candidate_ratios == candidate_ratios(),
            "candidate ratios drift",
        )?;
        let expected = match phase {
            Phase::CalibrationFreeze => (5, 1, 4, 0),
            Phase::Holdout => (9, 1, 4, 4),
        };
        let mut roles = [0_usize; 3];
        let mut case_ids = HashSet::new();
        for entry in &artifact.entries {
            validate_text(&entry.case_id)?;
            require(
                case_ids.insert(entry.case_id.as_str()),
                "duplicate selection case id",
            )?;
            decode_sha256(&entry.input_sha256)?;
            match entry.role {
                EntryRole::Regression => {
                    roles[0] += 1;
                    require(
                        entry.selected_candidate_id.is_none()
                            && entry.phase_result.is_none()
                            && entry.model.is_none()
                            && entry.runtime.is_none()
                            && entry.device.is_none()
                            && entry.raw_evidence.is_none()
                            && entry.output_hashes.is_none()
                            && entry.assertions.is_none(),
                        "regression entry must contain only identity fields",
                    )?;
                }
                EntryRole::Calibration | EntryRole::Holdout => {
                    roles[usize::from(entry.role == EntryRole::Holdout) + 1] += 1;
                    validate_model_entry(entry, &artifact.selected_candidate_id)?;
                }
            }
        }
        require(
            (artifact.entries.len(), roles[0], roles[1], roles[2]) == expected,
            "selection artifact entry role counts mismatch",
        )
    }

    fn validate_model_entry(entry: &SelectionEntry, selected_candidate_id: &str) -> io::Result<()> {
        require(
            entry.selected_candidate_id.as_deref() == Some(selected_candidate_id)
                && entry.phase_result.as_deref() == Some("pass"),
            "selection evidence did not pass for the selected candidate",
        )?;
        let model = entry
            .model
            .as_ref()
            .ok_or_else(|| invalid_data("missing model evidence"))?;
        let runtime = entry
            .runtime
            .as_ref()
            .ok_or_else(|| invalid_data("missing runtime evidence"))?;
        let device = entry
            .device
            .as_ref()
            .ok_or_else(|| invalid_data("missing device evidence"))?;
        let raw_evidence = entry
            .raw_evidence
            .as_ref()
            .ok_or_else(|| invalid_data("missing raw evidence"))?;
        let output_hashes = entry
            .output_hashes
            .as_ref()
            .ok_or_else(|| invalid_data("missing output hashes"))?;
        let assertions = entry
            .assertions
            .as_ref()
            .ok_or_else(|| invalid_data("missing assertions"))?;
        for value in [
            &model.provider,
            &model.backend,
            &runtime.os,
            &runtime.device,
            &device.actual_device,
            &raw_evidence.load_device,
            &raw_evidence.model_backend,
            &raw_evidence.layer_or_buffer,
            &raw_evidence.context_or_offload,
        ] {
            validate_text(value)?;
        }
        for hash in [
            &model.identifier_sha256,
            &raw_evidence.diagnostic_sha256,
            &output_hashes.source,
            &output_hashes.segment_mask,
            &output_hashes.inpainted,
            &output_hashes.rendered,
        ] {
            decode_sha256(hash)?;
        }
        require(
            raw_evidence.runtime_node_count > 0 && raw_evidence.device_load_confirmed,
            "raw selection evidence is invalid",
        )?;
        require(
            assertions.source_text_removed
                && assertions.target_rendered
                && assertions.protected_pixels_preserved
                && assertions.english_roi_preserved,
            "selection assertions must all pass",
        )
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

    fn synthetic_entries(role: EntryRole, count: usize, prefix: &str) -> Vec<SelectionEntry> {
        (0..count)
            .map(|index| SelectionEntry {
                case_id: format!("{prefix}-{index}"),
                input_sha256: synthetic_hash(index as u8 + 1),
                role,
                selected_candidate_id: Some("R05".into()),
                phase_result: Some("pass".into()),
                model: Some(ModelEvidence {
                    provider: "synthetic-test-provider".into(),
                    backend: "synthetic-test-backend".into(),
                    identifier_sha256: synthetic_hash(10),
                }),
                runtime: Some(RuntimeEvidence {
                    os: "synthetic-test-os".into(),
                    device: "synthetic-test-runtime-device".into(),
                }),
                device: Some(DeviceEvidence {
                    actual_device: "synthetic-test-device".into(),
                }),
                raw_evidence: Some(RawEvidence {
                    load_device: "synthetic-test-load-device".into(),
                    model_backend: "synthetic-test-model-backend".into(),
                    layer_or_buffer: "synthetic-test-layer-or-buffer".into(),
                    context_or_offload: "synthetic-test-context-or-offload".into(),
                    runtime_node_count: 24,
                    device_load_confirmed: true,
                    diagnostic_sha256: synthetic_hash(15),
                }),
                output_hashes: Some(OutputHashes {
                    source: synthetic_hash(11),
                    segment_mask: synthetic_hash(12),
                    inpainted: synthetic_hash(13),
                    rendered: synthetic_hash(14),
                }),
                assertions: Some(Assertions {
                    source_text_removed: true,
                    target_rendered: true,
                    protected_pixels_preserved: true,
                    english_roi_preserved: true,
                }),
            })
            .collect()
    }

    fn calibration_evidence() -> RunnerEvidence {
        let mut entries = vec![SelectionEntry {
            case_id: "regression-0".into(),
            input_sha256: synthetic_hash(20),
            role: EntryRole::Regression,
            selected_candidate_id: None,
            phase_result: None,
            model: None,
            runtime: None,
            device: None,
            raw_evidence: None,
            output_hashes: None,
            assertions: None,
        }];
        entries.extend(synthetic_entries(EntryRole::Calibration, 4, "calibration"));
        RunnerEvidence {
            selected_candidate_id: "R05".into(),
            entries,
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
        assert_eq!(artifact.entries.len(), 5);
        assert_eq!(
            artifact
                .entries
                .iter()
                .filter(|entry| entry.role == EntryRole::Calibration)
                .count(),
            4
        );
    }

    #[test]
    fn source_gate_selection_holdout_builds_closed_nine_entry_final_artifact() {
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
            |_| {
                Ok(RunnerEvidence {
                    selected_candidate_id: "R05".into(),
                    entries: synthetic_entries(EntryRole::Holdout, 4, "holdout"),
                })
            },
        )
        .unwrap();

        let bytes = fs::read(root.join("selection.json")).unwrap();
        let artifact: FrozenArtifact = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(bytes, canonical_json(&artifact).unwrap());
        assert_eq!(artifact.entries.len(), 9);
        assert_eq!(
            artifact
                .entries
                .iter()
                .map(|entry| entry.case_id.as_str())
                .collect::<HashSet<_>>()
                .len(),
            9
        );
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
                "visual_manifest_sha256".into(),
                "source_gate_fixture_manifest_sha256".into(),
                "selected_candidate_id".into(),
                "candidate_ratios".into(),
                "entries".into(),
            ])
        );
        let evidence = &value["entries"][1];
        assert_eq!(
            evidence
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from_iter([
                "case_id".into(),
                "input_sha256".into(),
                "role".into(),
                "selected_candidate_id".into(),
                "phase_result".into(),
                "model".into(),
                "runtime".into(),
                "device".into(),
                "raw_evidence".into(),
                "output_hashes".into(),
                "assertions".into(),
            ])
        );
        for (field, expected) in [
            (
                "model",
                HashSet::from_iter(["provider", "backend", "identifier_sha256"]),
            ),
            ("runtime", HashSet::from_iter(["os", "device"])),
            ("device", HashSet::from_iter(["actual_device"])),
            (
                "raw_evidence",
                HashSet::from_iter([
                    "load_device",
                    "model_backend",
                    "layer_or_buffer",
                    "context_or_offload",
                    "runtime_node_count",
                    "device_load_confirmed",
                    "diagnostic_sha256",
                ]),
            ),
            (
                "output_hashes",
                HashSet::from_iter(["source", "segment_mask", "inpainted", "rendered"]),
            ),
            (
                "assertions",
                HashSet::from_iter([
                    "source_text_removed",
                    "target_rendered",
                    "protected_pixels_preserved",
                    "english_roi_preserved",
                ]),
            ),
        ] {
            assert_eq!(
                evidence[field]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<HashSet<_>>(),
                expected
            );
        }
        assert_eq!(
            artifact
                .entries
                .iter()
                .filter(|entry| entry.role == EntryRole::Regression)
                .count(),
            1
        );
        assert_eq!(
            artifact
                .entries
                .iter()
                .filter(|entry| entry.role == EntryRole::Calibration)
                .count(),
            4
        );
        assert_eq!(
            artifact
                .entries
                .iter()
                .filter(|entry| entry.role == EntryRole::Holdout)
                .count(),
            4
        );
    }

    #[test]
    fn source_gate_selection_rejects_invalid_candidate_and_missing_calibration_entry() {
        for evidence in [
            RunnerEvidence {
                selected_candidate_id: "R100".into(),
                entries: calibration_evidence().entries,
            },
            {
                let mut evidence = calibration_evidence();
                evidence.entries.pop();
                evidence
            },
            {
                let mut evidence = calibration_evidence();
                evidence.entries[1].raw_evidence = None;
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
