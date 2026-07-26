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
