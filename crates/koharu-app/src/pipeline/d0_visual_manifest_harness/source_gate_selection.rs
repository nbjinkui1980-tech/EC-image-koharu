use super::super::d0_r59_holdout_bundle::{
    R59FreezeCommitments as R59BundleFreezeCommitments, R59ValidatedExecutionEntry,
    R59ValidatedExecutionTarget, R59ValidatedExecutionView, R59ValidatedReceiptData,
    validate_r60_plaintext_holdout_bundle,
};
use super::super::d0_visual_manifest_oracles::{
    OracleValidatedEntry, OracleValidatedTarget, ValidatedHalfOpenRect,
};
use super::super::d0_visual_manifest_schema::{
    Aspect, Background, DimensionBin, Effect, EntryRole, Expected, Position, TranslationLength,
    VisualManifestEntry, VisualManifestTarget, Writing,
};
use super::super::engines::bubble_segmentation::bubble_mask_from_result;
use super::super::engines::ctd_segment::dispatch_segment;
use super::super::engines::source_language_gate::{
    PpCanonicalLineDiagnostic, PpCanonicalOccurrenceDiagnostic, PpDetectorDiagnostic,
    PpRecognitionDiagnostic, SourceGateCropPolicy, SourceGateCropPolicyGuard,
    SourceGateDecision, SourceGateDetectorAssignmentDiagnostic,
    SourceGateDetectorOwnershipDiagnostic, SourceGateDiagnosticCapture,
    SourceGateDiagnosticEvent, SourceGateRejectReason, SourceGateTargetGeometryDiagnostic,
    dispatch_source_gate, rgba_fingerprint,
};
use super::super::engines::support::{
    EraseDiagnosticBranch, EraseDiagnosticCapture, EraseDiagnosticStage, EraseStageMask,
    PreparedInpaintMask, SOURCE_GATE_TARGET_DETECTOR, build_han_only_translation_ops,
    eligible_lines_for_page, eligible_text_lines, line_support_mask, prepare_inpaint_mask,
    protected_source_lines_for_page,
};
use super::*;
use base64::Engine as _;
use chrono::{SecondsFormat, Utc};
use image::{DynamicImage, GrayImage, RgbaImage};
use koharu_core::{Node, NodeId, NodeKind, Page, Scene, TextData, Transform};
use koharu_llm::NativeLogCaptureGuard;
use koharu_llm::paddleocr_vl::{PaddleOcrVl, PaddleOcrVlTask};
use koharu_llm::safe::{LlamaBackendDeviceType, list_llama_ggml_backend_devices};
use koharu_ml::{
    comic_text_detector::ComicTextDetector, inpainting::expand_mask_for_inpainting,
    pp_ocr_v5::PpOcrV5, speech_bubble_segmentation::SpeechBubbleSegmentation,
};
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
use std::process::Stdio;

use crate::config::{PipelineConfig, SourceTextPolicy};

const PHASE_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_PHASE";
const R52_BRIDGE_REQUEST_ENV: &str = "HANONLY_R52_BRIDGE_REQUEST";
const R52_PLAN_REVISION: u32 = 52;
const R52_CHALLENGE_MANIFEST: &str =
    "/Users/jinkui/ec-image-Koharu/hanonly-r51-challenge/challenge-manifest.json";
const R52_CHALLENGE_MANIFEST_SHA256: &str =
    "88fc92474502514d29b09e9863a4907d89adaeab7e67808a7e1890e5835d86b6";
const R52_CHALLENGE_HASHES: &str =
    "/Users/jinkui/ec-image-Koharu/hanonly-r51-challenge/challenge-hashes.json";
const R52_CHALLENGE_HASHES_SHA256: &str =
    "07ce42c60d6f6c7f7c2ea27b9ca9e13afc4edbf458ad4b359af7e16baf4822bb";
const R49_VISUAL_MANIFEST: &str =
    "/Users/jinkui/ec-image-Koharu/hanonly-r49-corpus/evidence-assets/visual-manifest.json";
const R49_VISUAL_MANIFEST_SHA256: &str =
    "fe7e4782fe7dfeaa953e0fc538509f53b287d023328c518dd8ac8b27e690945c";
const R60_FORMAL_CUSTODY_ENV: &str = "HANONLY_R60_FORMAL_CUSTODY";
const R59_CALIBRATION_MANIFEST_SHA256_ENV: &str = "HANONLY_R59_CALIBRATION_MANIFEST_SHA256";
const R60_START_MARKER_SHA256_ENV: &str = "HANONLY_R60_START_MARKER_SHA256";
const R60_COMPLETION_SUMMARY_STDOUT_PREFIX: &str = "HANONLY_R60_COMPLETION_SUMMARY=";
const HISTORICAL_CUSTODY_COMMAND_RETIRED: &str = "historical_custody_command_retired";
const R59_CALIBRATION_ARTIFACT_SHA256: &str =
    "7006eecae1aab6a7f178fc64c0979db0ec155ce3239122c280db750b8f90a3dc";
const R60_PUBLIC_DIRECTORY: &str = "/Users/Shared/hanonly-r60-public";
const R60_PUBLIC_COMMITMENT_PATH: &str =
    "/Users/Shared/hanonly-r60-public/r60-public-commitment.json";
const R60_SUCCESSOR_COMMITMENT_PATH: &str =
    "/Users/Shared/hanonly-r60-public/r60-successor-commitment.json";
const R60_START_MARKER_NAME: &str = "r60-holdout-start.json";
const R60_LAYOUT_RECEIPT_NAME: &str = "r60-layout-receipt.json";
const R60_RUNTIME_COMMITMENT_NAME: &str = "r60-runtime-commitment.json";
const R60_TERMINAL_RECEIPT_NAME: &str = "r60-holdout-terminal.json";
const R60_CLEANUP_RECEIPT_NAME: &str = "r60-cleanup-receipt.json";
const R60_PLAINTEXT_ROOT: &str = "/Users/koharu-custody/r60-plaintext";
const R59_RUNTIME_ARCHIVE_NAME: &str = "bundle.tar";
const R60_CONTRACT_SHA256: &str =
    "4bc1a9d74e2f9e7b705159ead282fe1517b1737e49a09a4962f74bac921cba79";
const R60_TEST_SPEC_SHA256: &str =
    "22d901ec1b96d96ec7b063422c9d7292b0cb3ba13074f407844886bdce3e80d7";
const R60_SOURCE_B0_SHA: &str = "693597c955a481e57f8df79a09bc5462314c634a";
const B0_SHA_ENV: &str = "HANONLY_B0_SHA";
const ARTIFACT_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_ARTIFACT";
const REPORT_DIR_ENV: &str = "HANONLY_SOURCE_GATE_SELECTION_REPORT_DIR";
const REQUIRED_CHECK_ENV: &str = "HANONLY_SOURCE_GATE_REQUIRED_CHECK_ATTESTATION";
const ARTIFACT_VERSION: u32 = 2;
const PLAN_REVISION: u32 = 59;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormalRevision {
    R60,
}

impl FormalRevision {
    fn plan_revision(self) -> u32 {
        60
    }

    fn entry_ids(self) -> Vec<String> {
        let revision = self.plan_revision();
        (1..=4)
            .map(|slot| format!("r{revision}-h0{slot}"))
            .collect()
    }

    fn plaintext_root(self) -> &'static str {
        R60_PLAINTEXT_ROOT
    }

    fn start_marker_name(self) -> &'static str {
        R60_START_MARKER_NAME
    }

    fn runtime_commitment_name(self) -> &'static str {
        R60_RUNTIME_COMMITMENT_NAME
    }

    fn terminal_receipt_name(self) -> &'static str {
        R60_TERMINAL_RECEIPT_NAME
    }

    fn cleanup_receipt_name(self) -> &'static str {
        R60_CLEANUP_RECEIPT_NAME
    }

    fn artifact_namespace(self) -> &'static str {
        "r60"
    }

    fn contract_sha256(self) -> &'static str {
        R60_CONTRACT_SHA256
    }

    fn contract_path(self) -> &'static str {
        ".omx/plans/archive/hanonly-r60-b0-custody-contract.json"
    }

    fn test_spec_path(self) -> &'static str {
        ".omx/plans/archive/test-spec-hanonly-r60-b0-custody.md"
    }

    fn start_marker_sha256_env(self) -> &'static str {
        R60_START_MARKER_SHA256_ENV
    }

    fn external_device(self, device: &str) -> &str {
        if device == "metal" {
            "actual-metal"
        } else {
            device
        }
    }

    fn completion_summary_contract(self) -> &'static str {
        "hanonly-r60-b0-completion-summary-v1"
    }

    fn completion_summary_stdout_prefix(self) -> &'static str {
        R60_COMPLETION_SUMMARY_STDOUT_PREFIX
    }

    fn formal_cell_keys(self) -> Vec<String> {
        self.entry_ids()
            .into_iter()
            .flat_map(|entry| {
                ["cpu", "metal"]
                    .into_iter()
                    .map(move |device| format!("{entry}/{}", self.external_device(device)))
            })
            .collect()
    }

    fn diagnostic_generation_bounds(self) -> (u64, u64) {
        (0, 16)
    }
}

fn select_formal_revision(
    retired_r59: Option<&str>,
    r60: Option<&str>,
) -> io::Result<Option<FormalRevision>> {
    if matches!(retired_r59, Some("1")) || matches!(r60, Some("1")) {
        return Err(invalid_data(HISTORICAL_CUSTODY_COMMAND_RETIRED));
    }
    require(
        retired_r59.is_none() || retired_r59 == Some("0"),
        "invalid retired R59 formal custody mode",
    )?;
    match r60 {
        None | Some("0") => Ok(false),
        Some(_) => return Err(invalid_data("invalid R60 formal custody mode")),
    }
    .map(|_| None)
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
    formal_custody: Option<FormalCustody>,
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
    selected_candidate_override: Option<String>,
    held_calibration_artifact_sha256: OnceCell<String>,
    frozen_candidate_id: OnceCell<String>,
}

struct FormalCustody {
    revision: FormalRevision,
    contract_sha256: String,
    holdout: Option<HoldoutCustody>,
}

struct HoldoutCustody {
    directory: PathBuf,
    plaintext_directory: PathBuf,
    plaintext_archive: PathBuf,
    freeze: FreezeCommitments,
    expected_start_marker_sha256: String,
    open_marker: OnceCell<PublishedArtifact>,
    runtime_commitment: OnceCell<RuntimeCommitments>,
}

struct FreezeCommitments {
    receipt_sha256: String,
    original_public_commitment_sha256: String,
    original_b0_sha: String,
    successor_b0_sha: String,
    calibration_artifact_sha256: String,
    ciphertext_sha256: String,
    private_manifest_commitment_sha256: String,
    r60_layout: Option<R60LayoutBindings>,
}

struct R60LayoutBindings {
    layout_receipt_sha256: String,
    layout_validator_sha256: String,
    manifest_sha256: String,
    member_name_digest_sha256: String,
}

impl FreezeCommitments {
    fn accepts_calibration_artifact(
        &self,
        artifact_b0_sha: &str,
        requested_b0_sha: &str,
        artifact_sha256: &str,
    ) -> bool {
        self.original_b0_sha == artifact_b0_sha
            && self.successor_b0_sha == requested_b0_sha
            && self.calibration_artifact_sha256 == artifact_sha256
    }
}

struct RuntimeCommitments {
    receipt: PublishedArtifact,
    plaintext_archive_sha256: String,
    manifest_sha256: String,
    oracle_sha256: String,
    hashes_sha256: String,
}

struct FormalPublicPaths {
    original: PathBuf,
    directory: PathBuf,
    successor: PathBuf,
}

impl FormalPublicPaths {
    fn frozen() -> Self {
        Self {
            original: R60_PUBLIC_COMMITMENT_PATH.into(),
            directory: R60_PUBLIC_DIRECTORY.into(),
            successor: R60_SUCCESSOR_COMMITMENT_PATH.into(),
        }
    }
}

impl SelectionEnvironment {
    fn parse(get: impl FnMut(&str) -> Option<String>) -> io::Result<Self> {
        Self::parse_with_formal_paths(get, None)
    }

    fn parse_with_formal_paths(
        mut get: impl FnMut(&str) -> Option<String>,
        test_paths: Option<FormalPublicPaths>,
    ) -> io::Result<Self> {
        let phase = match required(&mut get, PHASE_ENV)?.as_str() {
            "calibration-freeze" => Phase::CalibrationFreeze,
            "holdout" => Phase::Holdout,
            _ => return Err(invalid_data("invalid Source Gate selection phase")),
        };
        let b0_sha = required(&mut get, B0_SHA_ENV)?;
        require_git_sha(&b0_sha)?;
        let retired_r59_mode = get("HANONLY_R59_FORMAL_CUSTODY");
        let r60_mode = get(R60_FORMAL_CUSTODY_ENV);
        let formal_revision =
            select_formal_revision(retired_r59_mode.as_deref(), r60_mode.as_deref())?;
        require(
            formal_revision.is_none() || phase == Phase::Holdout,
            "R60 formal custody is holdout-only",
        )?;
        let formal_custody = match formal_revision {
            None => None,
            Some(revision) => {
                let paths = test_paths.unwrap_or_else(FormalPublicPaths::frozen);
                let contract_path = repository_root()?.join(revision.contract_path());
                let contract_sha256 = sha256_file(&contract_path)?;
                require(
                    contract_sha256 == revision.contract_sha256(),
                    "formal custody contract hash drift",
                )?;
                let holdout = if phase == Phase::Holdout {
                    let plaintext_directory = PathBuf::from(revision.plaintext_root());
                    let plaintext_archive = plaintext_directory.join(R59_RUNTIME_ARCHIVE_NAME);
                    require_absolute_canonical(&paths.directory)?;
                    Some(HoldoutCustody {
                        freeze: load_formal_successor_commitments(
                            &paths.original,
                            &paths.successor,
                            &b0_sha,
                            &contract_sha256,
                            &repository_root()?.join(revision.test_spec_path()),
                        )?,
                        directory: paths.directory,
                        plaintext_directory,
                        plaintext_archive,
                        expected_start_marker_sha256: required_hash(
                            &mut get,
                            revision.start_marker_sha256_env(),
                        )?,
                        open_marker: OnceCell::new(),
                        runtime_commitment: OnceCell::new(),
                    })
                } else {
                    None
                };
                Some(FormalCustody {
                    revision,
                    contract_sha256,
                    holdout,
                })
            }
        };
        let formal_holdout = formal_custody
            .as_ref()
            .and_then(|custody| custody.holdout.as_ref());
        let visual_manifest_sha256 = match formal_holdout {
            Some(holdout) => holdout.freeze.private_manifest_commitment_sha256.clone(),
            None => {
                let value = required(&mut get, VISUAL_MANIFEST_SHA256_ENV)?;
                decode_sha256(&value)?;
                value
            }
        };
        let calibration_manifest_sha256 = if formal_custody.is_some() {
            let value = required_hash(&mut get, R59_CALIBRATION_MANIFEST_SHA256_ENV)?;
            if phase == Phase::CalibrationFreeze {
                require(
                    value == visual_manifest_sha256,
                    "R59 calibration manifest commitment drift",
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
                holdout.freeze.ciphertext_sha256.clone(),
                holdout.plaintext_directory.join("manifest.json"),
                Vec::new(),
                formal_custody
                    .as_ref()
                    .expect("formal custody")
                    .revision
                    .entry_ids(),
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
            let mut calibration_entry_ids = manifest
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
            let phase_partition_valid = if formal_custody.is_some() {
                if phase == Phase::CalibrationFreeze {
                    calibration_entry_ids = calibration_slot_entry_ids(&manifest.entries)?;
                    holdout_entry_ids.is_empty()
                } else {
                    manifest.entries.len() == 4
                        && holdout_entry_ids
                            == formal_custody
                                .as_ref()
                                .expect("formal custody")
                                .revision
                                .entry_ids()
                        && calibration_entry_ids.is_empty()
                }
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
        let required_check_manifest_sha256 = formal_custody
            .as_ref()
            .and_then(|custody| custody.holdout.as_ref())
            .map_or(visual_manifest_sha256.as_str(), |holdout| {
                holdout.freeze.private_manifest_commitment_sha256.as_str()
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
            formal_custody,
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
            selected_candidate_override: None,
            held_calibration_artifact_sha256: OnceCell::new(),
            frozen_candidate_id: OnceCell::new(),
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
    formal: Option<R59FormalRunEvidence>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RealModelRunMode {
    Matrix,
    EraseStageProbe,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct EraseStageTargetMetric {
    target_id: String,
    oracle_pixels: u64,
    intersection_pixels: u64,
    missing_pixels: u64,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct EraseStageMetric {
    stage: EraseDiagnosticStage,
    branch: EraseDiagnosticBranch,
    grayscale_blake3: String,
    nonzero_pixels: u64,
    protected_overlap_pixels: u64,
    targets: Vec<EraseStageTargetMetric>,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct EraseStageProbeReport {
    version: u8,
    entry_id: String,
    candidate_id: String,
    device: String,
    width: u32,
    height: u32,
    stages: Vec<EraseStageMetric>,
}

#[derive(Clone, Debug, PartialEq)]
struct R59FormalRunEvidence {
    bundle_validation_receipt: Option<PublishedArtifact>,
    cells: Vec<R59TerminalCellResult>,
    first_failed_cell: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PublishedArtifact {
    path: String,
    sha256: String,
    byte_length: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct R52BridgeRequest {
    contract: String,
    plan_revision: u32,
    mode: String,
    b0_sha: String,
    repo_root: PathBuf,
    evidence_root: PathBuf,
    result_path: PathBuf,
    selected_candidate_id: String,
    challenge_manifest_path: PathBuf,
    challenge_manifest_sha256: String,
    challenge_hash_record_path: PathBuf,
    challenge_hash_record_sha256: String,
    r49_visual_manifest_path: PathBuf,
    r49_visual_manifest_sha256: String,
    source_gate_fixture_manifest_sha256: String,
    calibration_selection_artifact_path: PathBuf,
    b0_preflight_attestation_path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct R52ChallengeManifest {
    contract: String,
    entries: Vec<R52ChallengeManifestEntry>,
    oracle_corrections: Vec<R52OracleCorrection>,
    plan_revision: u32,
    role: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct R52ChallengeManifestEntry {
    id: String,
    notes_path: Option<PathBuf>,
    notes_sha256: Option<String>,
    prior_role: String,
    source_path: PathBuf,
    source_sha256: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct R52OracleCorrection {
    entry_id: String,
    expected_decision: String,
    expected_rejection_reason: String,
    r49_corpus_immutable: bool,
    source_script_class: String,
    target_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct R52ChallengeHashRecord {
    contract: String,
    manifest_sha256: String,
    plan_revision: u32,
}

#[derive(Debug, Deserialize)]
struct R52SupplementalNote {
    id: String,
    role: String,
    width: u32,
    height: u32,
    multi_node: bool,
    protected_rois: Vec<[u64; 4]>,
    targets: Vec<R52SupplementalTarget>,
}

#[derive(Debug, Deserialize)]
struct R52SupplementalTarget {
    id: String,
    source_roi: [u64; 4],
    clean_reference_edit_roi: [u64; 4],
    erase_source_ink_mask_path: PathBuf,
    residual_source_ink_mask_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct R52ChallengeCell {
    ordinal: usize,
    entry_id: String,
    device: String,
    kind: String,
    candidate_id: String,
    selection_result_path: String,
    selection_result_sha256: String,
    target_recall: Option<R59TargetRecall>,
    pp_count: usize,
    vl_count: usize,
    rejection_reason: Option<String>,
    diagnostic_path: String,
    diagnostic_sha256: String,
    process_evidence_path: String,
    process_evidence_sha256: String,
    log_path: String,
    log_sha256: String,
    result: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct R52BridgeResult<'a> {
    contract: &'static str,
    plan_revision: u32,
    b0_sha: &'a str,
    selected_candidate_id: &'a str,
    ordered_cell_results: &'a [R52ChallengeCell],
    result: &'static str,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct R59TargetRecall {
    target_total: usize,
    selected: usize,
    covered: usize,
    uncovered: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct R59TerminalCellResult {
    cell_key: String,
    result: String,
    selection_result: Option<String>,
    target_recall: R59TargetRecall,
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
    scene_by_target: BTreeMap<String, Vec<SceneSupportEvidence>>,
    selected_scene_rotations_zero: bool,
    runtime_inpainter_id: String,
    bubble_segmenter_id: String,
    bubble_support_sha256: String,
    removal_support: Vec<u8>,
}

struct SceneSupportEvidence {
    rect: [i64; 4],
    mask: Vec<u8>,
    downstream_mask: Vec<u8>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct R60BundleValidationReceipt<'a> {
    contract: &'static str,
    runtime_bundle_schema: &'static str,
    plan_revision: u32,
    b0_sha: &'a str,
    test_executable_sha256: &'a str,
    enabled_cargo_features: [&'static str; 1],
    r60_contract_sha256: &'a str,
    public_commitment_sha256: &'a str,
    successor_commitment_sha256: &'a str,
    source_b0_sha: &'a str,
    successor_b0_sha: &'a str,
    private_manifest_commitment_sha256: &'a str,
    runtime_commitment_receipt_sha256: &'a str,
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct R60PublicCommitment {
    schema: String,
    plan_revision: u32,
    source_b0_sha: String,
    ciphertext_sha256: String,
    layout_receipt_sha256: String,
    layout_validator_sha256: String,
    manifest_sha256: String,
    member_name_digest_sha256: String,
    private_manifest_commitment_sha256: String,
    entry_ids: Vec<String>,
    cleanup_pass: bool,
    restricted_values_disclosed: bool,
    start_marker_absent: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct R60SuccessorCommitment {
    schema: String,
    plan_revision: u32,
    public_commitment_sha256: String,
    source_b0_sha: String,
    successor_b0_sha: String,
    contract_sha256: String,
    test_spec_sha256: String,
    calibration_artifact_sha256: String,
    selected_candidate_id: String,
    ciphertext_sha256: String,
    layout_receipt_sha256: String,
    layout_validator_sha256: String,
    manifest_sha256: String,
    member_name_digest_sha256: String,
    private_manifest_commitment_sha256: String,
    entry_ids: Vec<String>,
    package_unchanged: bool,
    start_marker_absent: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct R60LayoutReceipt {
    schema: String,
    plan_revision: u32,
    manifest_sha256: String,
    private_manifest_commitment_sha256: String,
    member_name_digest_sha256: String,
    ciphertext_sha256: String,
    layout_validator_sha256: String,
    entry_ids: Vec<String>,
    required_root_present: bool,
    wrapper_absent: bool,
    canonical_ustar_pass: bool,
    manifest_binding_pass: bool,
    same_archive_object_pass: bool,
    layout_pass: bool,
    restricted_values_disclosed: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct R60OpenMarker {
    schema: String,
    plan_revision: u32,
    b0_sha: String,
    public_commitment_sha256: String,
    successor_commitment_sha256: String,
    calibration_artifact_sha256: String,
    selected_candidate_id: String,
    entry_ids: Vec<String>,
    pre_holdout_attestation_sha256: String,
    nonce_hex: String,
    state: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct R60RuntimeCommitment {
    schema: String,
    plan_revision: u32,
    b0_sha: String,
    start_marker_sha256: String,
    successor_commitment_sha256: String,
    ciphertext_sha256: String,
    layout_receipt_sha256: String,
    layout_validator_sha256: String,
    member_name_digest_sha256: String,
    private_manifest_commitment_sha256: String,
    calibration_artifact_sha256: String,
    selected_candidate_id: String,
    plaintext_archive_sha256: String,
    manifest_sha256: String,
    oracle_sha256: String,
    hashes_sha256: String,
    entry_ids: Vec<String>,
    decrypt_pass: bool,
    package_unchanged: bool,
    restricted_values_disclosed: bool,
    state: String,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct R57SourceInkCoverageProof<'a> {
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
    runtime_removal_support_relpath: String,
    runtime_removal_support_byte_length: u64,
    runtime_removal_support_sha256: String,
    spatial_validation_receipt_relpath: String,
    spatial_validation_receipt_byte_length: u64,
    spatial_validation_receipt_sha256: String,
    protected_geometry_sha256: String,
    runtime_inpainter_id: &'a str,
    bubble_segmenter_id: &'a str,
    bubble_support_sha256: &'a str,
    oracle_foreground_pixels: u64,
    runtime_removal_support_foreground_pixels: u64,
    runtime_removal_covered_pixels: u64,
    missing_runtime_removal_pixels: u64,
    protected_overlap_pixels: u64,
    target_selected: bool,
    result: &'static str,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct R59TargetCoverageIndex<'a> {
    contract: &'static str,
    plan_revision: u32,
    b0_sha: &'a str,
    cell_key: &'a str,
    manifest_sha256: &'a str,
    oracle_sha256: &'a str,
    hashes_sha256: &'a str,
    records: Vec<R59TargetCoverageIndexRecord>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct R59TargetCoverageIndexRecord {
    entry_id: String,
    target_id: String,
    proof_path: String,
    proof_sha256: String,
    proof_byte_length: u64,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct R59CompletionSummary<'a> {
    contract: &'static str,
    plan_revision: u32,
    b0_sha: &'a str,
    selected_candidate_id: &'a str,
    original_public_commitment_sha256: &'a str,
    successor_commitment_sha256: &'a str,
    successor_b0_sha: &'a str,
    start_marker_sha256: &'a str,
    ciphertext_sha256: &'a str,
    private_manifest_commitment_sha256: &'a str,
    runtime_commitment_receipt_sha256: &'a str,
    pre_holdout_attestation_sha256: &'a str,
    holdout_manifest_sha256: &'a str,
    bundle_validation_receipt_path: &'a str,
    bundle_validation_receipt_sha256: &'a str,
    bundle_validation_receipt_byte_length: u64,
    terminal_diagnostic_index_path: &'a str,
    terminal_diagnostic_index_sha256: &'a str,
    terminal_diagnostic_index_byte_length: u64,
    cell_results: &'a [R59TerminalCellResult],
    first_failed_cell: Option<&'a str>,
    unexecuted_cell_keys: Vec<String>,
    all_cells_terminated: bool,
    all_cells_passed: bool,
    failure_kind: Option<&'static str>,
    authorization_state: &'static str,
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

fn require_git_sha(value: &str) -> io::Result<()> {
    require(
        value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "B0 sha must be 40 lowercase hex characters",
    )
}

fn require_formal_plaintext_root(revision: FormalRevision, path: &Path) -> io::Result<()> {
    require(
        path == Path::new(revision.plaintext_root()),
        "formal plaintext root must be the fixed custody path",
    )
}

fn r59_entry_ids(kind: char) -> Vec<String> {
    (1..=4)
        .map(|index| format!("r59-{kind}{index:02}"))
        .collect()
}

fn calibration_slot_entry_ids(entries: &[SelectionManifestEntry]) -> io::Result<Vec<String>> {
    require(
        entries.len() == 4,
        "visual manifest calibration partition must contain four entries",
    )?;
    let mut prefix = None::<&str>;
    let mut ids = [const { None::<String> }; 4];
    for entry in entries {
        require(
            entry.role == EntryRole::Calibration,
            "visual manifest calibration partition contains non-calibration entry",
        )?;
        let (entry_prefix, slot) = calibration_slot(&entry.id)?;
        if let Some(prefix) = prefix {
            require(
                prefix == entry_prefix,
                "visual manifest calibration slots use mixed revision prefixes",
            )?;
        } else {
            require(
                !entry_prefix.is_empty(),
                "visual manifest calibration revision prefix is empty",
            )?;
            prefix = Some(entry_prefix);
        }
        require(
            ids[slot].is_none(),
            "visual manifest calibration slot is duplicated",
        )?;
        ids[slot] = Some(entry.id.clone());
    }
    ids.into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| invalid_data("visual manifest calibration slot is missing"))
}

fn calibration_slot(id: &str) -> io::Result<(&str, usize)> {
    let (prefix, suffix) = id
        .rsplit_once("-c")
        .ok_or_else(|| invalid_data("visual manifest calibration id slot is invalid"))?;
    require(
        matches!(suffix, "01" | "02" | "03" | "04"),
        "visual manifest calibration id slot is invalid",
    )?;
    Ok((prefix, suffix[1..].parse::<usize>().unwrap() - 1))
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
    let mut canonical = canonical_json(&attestation)?;
    let canonical_without_lf = canonical == held.bytes();
    canonical.push(b'\n');
    require(
        canonical_without_lf || canonical == held.bytes(),
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
            if environment.formal_custody.is_some() {
                let formal = evidence
                    .formal
                    .as_ref()
                    .ok_or_else(|| invalid_data("R59 calibration evidence is missing"))?;
                write_r59_calibration_diagnostic_generations(&environment, formal)?;
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
            let artifact_input = HeldInput::open(&environment.artifact)?;
            let bytes = artifact_input.bytes();
            let mut artifact: FrozenArtifact = serde_json::from_slice(&bytes)
                .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
            require(
                canonical_json(&artifact)? == bytes,
                "selection artifact must be canonical JSON",
            )?;
            hold_calibration_artifact(&environment, &artifact_input, &artifact)?;
            validate_artifact(&artifact, Phase::CalibrationFreeze, &environment)?;
            let formal_holdout = environment.formal_custody.is_some();
            if formal_holdout {
                validate_formal_runner_open(&environment, &artifact.selected_candidate_id)?;
                validate_formal_runtime_commitment(&environment)?;
            }
            let result = (|| {
                let evidence =
                    artifact_input.with_revalidated_path(|_| model_runner(&environment))?;
                artifact_input.with_revalidated_path(|_| Ok(()))?;
                require(
                    evidence.selected_candidate_id == artifact.selected_candidate_id,
                    "holdout selected candidate drift",
                )?;
                if formal_holdout {
                    let formal = evidence
                        .formal
                        .as_ref()
                        .ok_or_else(|| invalid_data("R59 formal evidence is missing"))?;
                    write_r59_diagnostic_generations(
                        &environment,
                        &artifact.selected_candidate_id,
                        &artifact.manifest_sha256,
                        formal,
                    )?;
                    if formal.first_failed_cell.is_some() {
                        return Err(invalid_data("R59 formal holdout failed"));
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
                    Some(environment.formal_custody.as_ref().map_or_else(
                        || environment.visual_manifest_sha256.clone(),
                        |custody| {
                            custody
                                .holdout
                                .as_ref()
                                .expect("formal holdout custody")
                                .runtime_commitment
                                .get()
                                .expect("validated runtime commitment")
                                .manifest_sha256
                                .clone()
                        },
                    ));
                artifact.holdout_entry_ids = terminal_holdout_entry_ids(&environment);
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
    run_real_model_with_mode(environment, RealModelRunMode::Matrix)
}

fn run_erase_stage_probe(environment: &SelectionEnvironment) -> io::Result<RunnerEvidence> {
    require(
        environment.phase == Phase::CalibrationFreeze && environment.formal_custody.is_none(),
        "erase-stage probe only accepts public calibration input",
    )?;
    run_real_model_with_mode(environment, RealModelRunMode::EraseStageProbe)
}

fn run_real_model_with_mode(
    environment: &SelectionEnvironment,
    mode: RealModelRunMode,
) -> io::Result<RunnerEvidence> {
    if environment.formal_custody.is_some() && environment.phase == Phase::Holdout {
        require(
            mode == RealModelRunMode::Matrix,
            "erase-stage probe cannot enter formal holdout",
        )?;
        return run_formal_model(environment);
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
                source_ink_masks: decoded
                    .targets
                    .iter()
                    .map(|target| {
                        SourceInkMask::page(
                            &target.agreed_mask,
                            decoded.source.width(),
                            decoded.source.height(),
                        )
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        runtime.block_on(run_real_model_async(
            environment,
            &entries,
            executable_sha256,
            None,
            mode,
        ))
    })
}

struct RealModelEntry<'a> {
    schema: &'a VisualManifestEntry,
    source: &'a RgbaImage,
    oracle: &'a OracleValidatedEntry,
    source_ink_masks: Vec<SourceInkMask<'a>>,
}

struct R52OwnedChallengeEntry {
    schema: VisualManifestEntry,
    source: RgbaImage,
    oracle: OracleValidatedEntry,
    held_source: HeldInput,
    held_note: HeldInput,
}

fn load_r52_bridge_request_path(
    request_path: PathBuf,
) -> io::Result<(PathBuf, HeldInput, R52BridgeRequest)> {
    require_absolute_canonical(&request_path)?;
    let held = HeldInput::open_bounded(&request_path, BYTE_CEILING)?;
    held.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    let request: R52BridgeRequest = serde_json::from_slice(held.bytes())
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    require(
        canonical_json(&request)? == held.bytes(),
        "R52 bridge request is not canonical JSON",
    )?;
    Ok((request_path, held, request))
}

fn load_r52_bridge_request() -> io::Result<(PathBuf, HeldInput, R52BridgeRequest)> {
    load_r52_bridge_request_path(PathBuf::from(
        std::env::var(R52_BRIDGE_REQUEST_ENV)
            .map_err(|_| invalid_data("missing R52 bridge request path"))?,
    ))
}

fn validate_r52_bridge_request(
    request_path: &Path,
    request: &R52BridgeRequest,
) -> io::Result<()> {
    require(
        request.contract == "hanonly-r52-evidence-bridge-request-v1"
            && request.plan_revision == R52_PLAN_REVISION
            && request.mode == "challenge",
        "R52 bridge request contract drift",
    )?;
    require(
        request.b0_sha.len() == 40
            && request
                .b0_sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "R52 bridge B0 sha drift",
    )?;
    for hash in [
        &request.challenge_manifest_sha256,
        &request.challenge_hash_record_sha256,
        &request.r49_visual_manifest_sha256,
        &request.source_gate_fixture_manifest_sha256,
    ] {
        decode_sha256(hash)?;
    }
    for path in [
        &request.repo_root,
        &request.evidence_root,
        &request.challenge_manifest_path,
        &request.challenge_hash_record_path,
        &request.r49_visual_manifest_path,
        &request.calibration_selection_artifact_path,
        &request.b0_preflight_attestation_path,
    ] {
        require_absolute_canonical(path)?;
    }
    require_future_path_below(&request.evidence_root, &request.result_path)?;
    require(
        request_path.parent() == Some(request.evidence_root.as_path())
            && request.result_path.parent() == Some(request.evidence_root.as_path())
            && request.result_path != request_path
            && request
                .result_path
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| {
                    name.starts_with(".r52-challenge-result-") && name.ends_with(".tmp")
                }),
        "R52 bridge request/result path binding drift",
    )?;
    require(
        request.repo_root == repository_root()?
            && git_head(&request.repo_root)? == request.b0_sha,
        "R52 bridge executable worktree lineage drift",
    )?;
    let fixture = request.repo_root.join(FIXTURE_RELATIVE_PATH);
    require(
        sha256_file(&fixture)? == request.source_gate_fixture_manifest_sha256,
        "R52 bridge Source Gate fixture hash drift",
    )?;
    let selection = HeldInput::open(&request.calibration_selection_artifact_path)?;
    selection.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    let artifact: FrozenArtifact = serde_json::from_slice(selection.bytes())
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    require(
        artifact.plan_revision == PLAN_REVISION
            && artifact.b0_sha == request.b0_sha
            && artifact.selected_candidate_id == request.selected_candidate_id
            && artifact.source_gate_fixture_manifest_sha256
                == request.source_gate_fixture_manifest_sha256,
        "R52 bridge frozen selection binding drift",
    )?;
    let preflight = HeldInput::open(&request.b0_preflight_attestation_path)?;
    preflight.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    let preflight_value: serde_json::Value = serde_json::from_slice(preflight.bytes())
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let executable = fs::canonicalize(std::env::current_exe()?)?;
    let executable_sha256 = sha256_file(&executable)?;
    require(
        canonical_json(&preflight_value)? == preflight.bytes()
            && preflight_value
                .get("plan_revision")
                .and_then(|value| value.as_u64())
                == Some(R52_PLAN_REVISION.into())
            && preflight_value
                .get("b0_sha")
                .and_then(|value| value.as_str())
                == Some(request.b0_sha.as_str())
            && preflight_value
                .get("result")
                .and_then(|value| value.as_str())
                == Some("pass")
            && preflight_value
                .get("evidence_test_executable_path")
                .and_then(|value| value.as_str())
                == executable.to_str()
            && preflight_value
                .get("evidence_test_executable_sha256")
                .and_then(|value| value.as_str())
                == Some(executable_sha256.as_str()),
        "R52 bridge preflight binding drift",
    )
}

fn load_r52_challenge_manifest(
    request: &R52BridgeRequest,
) -> io::Result<(HeldInput, R52ChallengeManifest)> {
    require(
        request.mode == "challenge"
            && request.challenge_manifest_path == Path::new(R52_CHALLENGE_MANIFEST)
            && request.challenge_manifest_sha256 == R52_CHALLENGE_MANIFEST_SHA256
            && request.challenge_hash_record_path == Path::new(R52_CHALLENGE_HASHES)
            && request.challenge_hash_record_sha256 == R52_CHALLENGE_HASHES_SHA256
            && request.r49_visual_manifest_path == Path::new(R49_VISUAL_MANIFEST)
            && request.r49_visual_manifest_sha256 == R49_VISUAL_MANIFEST_SHA256,
        "R52 challenge frozen path/hash binding drift",
    )?;
    let manifest = HeldInput::open(&request.challenge_manifest_path)?;
    manifest.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    require(
        sha256_hex(manifest.bytes()) == request.challenge_manifest_sha256,
        "R52 challenge manifest hash drift",
    )?;
    let value: R52ChallengeManifest = serde_json::from_slice(manifest.bytes())
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    require(
        canonical_json(
            &serde_json::from_slice::<serde_json::Value>(manifest.bytes())
                .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?,
        )? == manifest.bytes(),
        "R52 challenge manifest is not canonical JSON",
    )?;
    let expected_ids = [
        "r49-h01", "r49-h02", "r49-h03", "r49-h04", "x01", "x02", "x03", "x04", "x05",
    ];
    require(
        value.contract == "hanonly-r51-disclosed-challenge-manifest-v1"
            && value.plan_revision == PLAN_REVISION
            && value.role == "challenge"
            && value
                .entries
                .iter()
                .map(|entry| entry.id.as_str())
                .eq(expected_ids)
            && value.oracle_corrections
                == [R52OracleCorrection {
                    entry_id: "r49-h04".into(),
                    expected_decision: "reject".into(),
                    expected_rejection_reason: "pp_no_han_protected_latin".into(),
                    r49_corpus_immutable: true,
                    source_script_class: "protected_latin".into(),
                    target_id: "product-id".into(),
                }],
        "R52 challenge manifest schema/order drift",
    )?;
    for (ordinal, entry) in value.entries.iter().enumerate() {
        require_absolute_canonical(&entry.source_path)?;
        decode_sha256(&entry.source_sha256)?;
        require(
            entry.prior_role
                == if ordinal < 4 {
                    "r49_disclosed_holdout"
                } else {
                    "r49_disclosed_challenge"
                },
            "R52 challenge prior role drift",
        )?;
        if ordinal < 4 {
            require(
                entry.notes_path.is_none() && entry.notes_sha256.is_none(),
                "R52 regression challenge unexpectedly has notes",
            )?;
        } else {
            let note_path = entry
                .notes_path
                .as_ref()
                .ok_or_else(|| invalid_data("R52 supplemental notes are missing"))?;
            require_absolute_canonical(note_path)?;
            decode_sha256(
                entry
                    .notes_sha256
                    .as_deref()
                    .ok_or_else(|| invalid_data("R52 supplemental notes hash is missing"))?,
            )?;
        }
    }
    let hashes = HeldInput::open(&request.challenge_hash_record_path)?;
    hashes.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    require(
        sha256_hex(hashes.bytes()) == request.challenge_hash_record_sha256,
        "R52 challenge hash-record hash drift",
    )?;
    let hashes_value: R52ChallengeHashRecord = serde_json::from_slice(hashes.bytes())
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    require(
        hashes_value.contract == "hanonly-r51-disclosed-challenge-hashes-v1"
            && hashes_value.plan_revision == PLAN_REVISION
            && hashes_value.manifest_sha256 == request.challenge_manifest_sha256,
        "R52 challenge hash-record binding drift",
    )?;
    Ok((manifest, value))
}

fn checked_r52_rect(
    rect: [u64; 4],
    width: u32,
    height: u32,
) -> io::Result<ValidatedHalfOpenRect> {
    let [left, top, right, bottom] = rect;
    require(
        left < right
            && top < bottom
            && right <= u64::from(width)
            && bottom <= u64::from(height)
            && right <= u64::from(u32::MAX)
            && bottom <= u64::from(u32::MAX),
        "R52 supplemental rectangle drift",
    )?;
    Ok(ValidatedHalfOpenRect {
        left: left as u32,
        top: top as u32,
        right: right as u32,
        bottom: bottom as u32,
    })
}

fn load_r52_supplemental_entry(
    manifest_entry: &R52ChallengeManifestEntry,
) -> io::Result<R52OwnedChallengeEntry> {
    let held_source = HeldInput::open_bounded(&manifest_entry.source_path, BYTE_CEILING)?;
    held_source.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    require(
        sha256_hex(held_source.bytes()) == manifest_entry.source_sha256,
        "R52 supplemental source hash drift",
    )?;
    let note_path = manifest_entry
        .notes_path
        .as_ref()
        .ok_or_else(|| invalid_data("R52 supplemental note path is missing"))?;
    let held_note = HeldInput::open_bounded(note_path, BYTE_CEILING)?;
    held_note.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    require(
        sha256_hex(held_note.bytes())
            == manifest_entry
                .notes_sha256
                .as_deref()
                .ok_or_else(|| invalid_data("R52 supplemental note hash is missing"))?,
        "R52 supplemental note hash drift",
    )?;
    let note: R52SupplementalNote = serde_json::from_slice(held_note.bytes())
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    require(
        note.id == format!("r49-{}", manifest_entry.id)
            && note.role == "challenge"
            && !note.targets.is_empty(),
        "R52 supplemental note identity drift",
    )?;
    let source = image::load_from_memory(held_source.bytes())
        .map_err(io::Error::other)?
        .to_rgba8();
    require(
        source.dimensions() == (note.width, note.height),
        "R52 supplemental source dimensions drift",
    )?;
    let mut targets = Vec::with_capacity(note.targets.len());
    let mut oracle_targets = Vec::with_capacity(note.targets.len());
    for target in note.targets {
        let source_roi = checked_r52_rect(target.source_roi, note.width, note.height)?;
        let edit_roi =
            checked_r52_rect(target.clean_reference_edit_roi, note.width, note.height)?;
        let mask_len = usize::try_from(
            u64::from(edit_roi.right - edit_roi.left)
                * u64::from(edit_roi.bottom - edit_roi.top),
        )
        .map_err(|_| invalid_data("R52 supplemental geometry overflow"))?;
        targets.push(VisualManifestTarget {
            id: target.id,
            source_roi: target.source_roi,
            clean_reference_edit_roi: target.clean_reference_edit_roi,
            erase_source_ink_mask_path: target
                .erase_source_ink_mask_path
                .to_string_lossy()
                .into_owned(),
            erase_source_ink_mask_sha256: String::new(),
            residual_source_ink_mask_path: target
                .residual_source_ink_mask_path
                .to_string_lossy()
                .into_owned(),
            residual_source_ink_mask_sha256: String::new(),
            position: Position::Interior,
            writing: Writing::Horizontal,
            effect: Effect::Plain,
            translation_length: TranslationLength::Equal,
            expected: Expected::AutomaticStrict,
        });
        oracle_targets.push(OracleValidatedTarget {
            source_roi,
            edit_roi,
            delta_mask: vec![1; mask_len].into_boxed_slice(),
        });
    }
    let protected_rois = note
        .protected_rois
        .iter()
        .copied()
        .map(|rect| checked_r52_rect(rect, note.width, note.height))
        .collect::<io::Result<Vec<_>>>()?;
    let max_side = note.width.max(note.height);
    let dimension_bin = match max_side {
        0..=719 => DimensionBin::Lt720,
        720..=1439 => DimensionBin::From720To1439,
        1440..=2159 => DimensionBin::From1440To2159,
        _ => DimensionBin::Gte2160,
    };
    let aspect = if u64::from(note.width) * 10 > u64::from(note.height) * 11 {
        Aspect::Landscape
    } else if u64::from(note.height) * 10 > u64::from(note.width) * 11 {
        Aspect::Portrait
    } else {
        Aspect::SquareOrNear
    };
    Ok(R52OwnedChallengeEntry {
        schema: VisualManifestEntry {
            id: manifest_entry.id.clone(),
            path: manifest_entry.source_path.to_string_lossy().into_owned(),
            sha256: manifest_entry.source_sha256.clone(),
            decoded_rgba_blake3: rgba_fingerprint(&DynamicImage::ImageRgba8(source.clone())),
            clean_reference_path: String::new(),
            clean_reference_sha256: String::new(),
            clean_reference_decoded_rgba_blake3: String::new(),
            role: EntryRole::Holdout,
            dimension_bin,
            aspect,
            background: Background::Product,
            targets,
            protected_rois: note.protected_rois,
            multi_node: note.multi_node,
        },
        source,
        oracle: OracleValidatedEntry {
            protected_rois,
            targets: oracle_targets,
        },
        held_source,
        held_note,
    })
}

fn apply_r52_protected_latin_correction(
    schema: &mut VisualManifestEntry,
    oracle: &mut OracleValidatedEntry,
) -> io::Result<()> {
    require(
        schema.id == "r49-h04" && schema.targets.len() == oracle.targets.len(),
        "R52 protected Latin correction entry drift",
    )?;
    let matching = schema
        .targets
        .iter()
        .enumerate()
        .filter(|(_, target)| target.id == "product-id")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    require(
        matching.len() == 1,
        "R52 protected Latin correction target drift",
    )?;
    let index = matching[0];
    let target = schema.targets.remove(index);
    let geometry = oracle.targets.remove(index);
    require(
        target.source_roi
            == [
                u64::from(geometry.source_roi.left),
                u64::from(geometry.source_roi.top),
                u64::from(geometry.source_roi.right),
                u64::from(geometry.source_roi.bottom),
            ],
        "R52 protected Latin correction geometry drift",
    )?;
    schema.protected_rois.push(target.source_roi);
    oracle.protected_rois.push(geometry.source_roi);
    Ok(())
}

fn r52_challenge_cell_passed(
    result: &SelectionResult,
    schema: &VisualManifestEntry,
    rejection: Option<&str>,
    kind: &str,
) -> bool {
    if result.entry_id == "r49-h04" {
        let expected_targets = schema
            .targets
            .iter()
            .filter(|target| target.expected != Expected::UnsupportedRotation)
            .count();
        return rejection == Some("pp_no_han_protected_latin")
            && result.execution_evidence.inference_completed
            && result.derived.target_recall == 1.0
            && result
                .derived
                .source_coverage_preflight
                .covered_source_roi_ids
                .len()
                == expected_targets
            && result
                .derived
                .source_coverage_preflight
                .source_text_roi_coverage
                == 1.0
            && result.derived.protected_false_positive_count == 0
            && result.derived.selected_protected_node_ids.is_empty()
            && result.derived.unmatched_selected_node_ids.is_empty()
            && result.derived.rotation_targets_excluded;
    }
    if kind == "supplemental" {
        return result.execution_evidence.inference_completed
            && result.derived.protected_false_positive_count == 0
            && result.derived.selected_protected_node_ids.is_empty();
    }
    result.derived.passed
}

fn write_r52_challenge_cell(
    environment: &SelectionEnvironment,
    ordinal: usize,
    process: &ProcessEvidence,
    result: &SelectionResult,
    schema: &VisualManifestEntry,
    oracle: &OracleValidatedEntry,
    diagnostics: &[SourceGateDiagnosticEvent],
) -> io::Result<R52ChallengeCell> {
    let device = process.requested_device.as_str();
    let kind = if ordinal < 8 {
        "regression"
    } else {
        "supplemental"
    };
    let cell_root = format!("r52/challenge/{ordinal:02}-{}-{device}", result.entry_id);
    let mut cell_process = process.clone();
    cell_process.phase = "challenge".into();
    cell_process.id = format!(
        "challenge/{}/{}/{device}",
        result.candidate_id, result.entry_id
    );
    let process_artifact = publish_r59_artifact(
        environment,
        &format!("{cell_root}/process.json"),
        &canonical_json(&cell_process)?,
    )?;
    let artifact_parent = environment
        .artifact
        .parent()
        .ok_or_else(|| invalid_data("R52 bridge result has no parent"))?;
    let original_log =
        artifact_parent.join(&result.execution_evidence.raw_inference_log_relpath);
    let log_bytes = fs::read(&original_log)?;
    require(
        sha256_hex(&log_bytes) == result.execution_evidence.raw_inference_log_sha256,
        "R52 challenge inference log hash drift",
    )?;
    let log_artifact = publish_r59_artifact(
        environment,
        &format!("{cell_root}/inference.log"),
        &log_bytes,
    )?;
    let rejection = rejection_reason(diagnostics);
    let selected = result.derived.selected_target_ids.len();
    let covered = result
        .derived
        .source_coverage_preflight
        .covered_source_roi_ids
        .len();
    let target_recall = (kind == "regression").then(|| R59TargetRecall {
        target_total: schema.targets.len(),
        selected,
        covered,
        uncovered: schema.targets.len().saturating_sub(covered),
    });
    let passed = r52_challenge_cell_passed(result, schema, rejection.as_deref(), kind);
    let (
        raw_detector_outputs,
        canonical_lines,
        raw_detector_hash,
        support_records,
        _detector_geometry_passed,
    ) = r59_detector_diagnostics(
        environment,
        result,
        schema,
        oracle,
        diagnostics,
        &rejection,
        None,
    )?;
    let diagnostic = serde_json::json!({
        "contract": "hanonly-r52-challenge-cell-diagnostic-v1",
        "plan_revision": R52_PLAN_REVISION,
        "b0_sha": &environment.b0_sha,
        "calibration_manifest_sha256": &environment.calibration_manifest_sha256,
        "holdout_manifest_sha256": serde_json::Value::Null,
        "fixture_manifest_sha256": &environment.source_gate_fixture_manifest_sha256,
        "phase": "challenge",
        "entry_id": &result.entry_id,
        "device": device,
        "candidate_id": &result.candidate_id,
        "state": if passed { "passed" } else { "failed" },
        "selection_result": if rejection.is_some() { "rejected" } else { "selected" },
        "target_recall": &target_recall,
        "pp_han_count": result.derived.source_coverage_preflight.pp_han_scalar_count,
        "vl_han_count": result.derived.source_coverage_preflight.vl_expected_han_scalar_count,
        "rejection_reason": &rejection,
        "raw_detector_outputs": raw_detector_outputs,
        "canonical_lines": canonical_lines,
        "raw_detector_count": support_records.len(),
        "raw_detector_f32_bits_multiset_sha256": raw_detector_hash,
        "detector_support_records": support_records,
        "device_evidence_sha256": &process_artifact.sha256,
        "device_evidence_byte_length": process_artifact.byte_length,
        "log_sha256": &log_artifact.sha256,
        "log_byte_length": log_artifact.byte_length,
        "terminal_reason": if passed { None } else { Some(rejection.as_deref().unwrap_or("coverage_failure")) },
        "bundle_validation_receipt_sha256": serde_json::Value::Null,
        "target_coverage_index_sha256": serde_json::Value::Null,
    });
    let diagnostic_artifact = publish_r59_artifact(
        environment,
        &format!("{cell_root}/diagnostic.json"),
        &canonical_json(&diagnostic)?,
    )?;
    let mut selection = result.clone();
    selection.process_evidence_id = cell_process.id;
    selection.execution_evidence.raw_inference_log_relpath = log_artifact.path.clone();
    selection.execution_evidence.raw_inference_log_sha256 = log_artifact.sha256.clone();
    selection.execution_evidence.source_gate_diagnostic_relpath =
        diagnostic_artifact.path.clone();
    selection.execution_evidence.source_gate_diagnostic_sha256 =
        diagnostic_artifact.sha256.clone();
    selection.derived.passed = passed;
    let selection_artifact = publish_r59_artifact(
        environment,
        &format!("{cell_root}/selection.json"),
        &canonical_json(&selection)?,
    )?;
    Ok(R52ChallengeCell {
        ordinal,
        entry_id: result.entry_id.clone(),
        device: device.into(),
        kind: kind.into(),
        candidate_id: result.candidate_id.clone(),
        selection_result_path: selection_artifact.path,
        selection_result_sha256: selection_artifact.sha256,
        target_recall,
        pp_count: result.derived.source_coverage_preflight.pp_han_scalar_count,
        vl_count: result
            .derived
            .source_coverage_preflight
            .vl_expected_han_scalar_count,
        rejection_reason: rejection,
        diagnostic_path: diagnostic_artifact.path,
        diagnostic_sha256: diagnostic_artifact.sha256,
        process_evidence_path: process_artifact.path,
        process_evidence_sha256: process_artifact.sha256,
        log_path: log_artifact.path,
        log_sha256: log_artifact.sha256,
        result: if passed { "pass" } else { "fail" },
    })
}

fn publish_r52_bridge_result(
    environment: &SelectionEnvironment,
    cells: &[R52ChallengeCell],
) -> io::Result<()> {
    let passed = cells.len() == 18 && cells.iter().all(|cell| cell.result == "pass");
    let result = R52BridgeResult {
        contract: "hanonly-r52-pinned-evaluator-result-v1",
        plan_revision: R52_PLAN_REVISION,
        b0_sha: &environment.b0_sha,
        selected_candidate_id: environment
            .selected_candidate_override
            .as_deref()
            .ok_or_else(|| invalid_data("R52 selected candidate override is missing"))?,
        ordered_cell_results: cells,
        result: if passed { "pass" } else { "fail" },
    };
    let parent = environment
        .artifact
        .parent()
        .ok_or_else(|| invalid_data("R52 bridge result has no parent"))?;
    require(
        parent == environment.evidence_root,
        "R52 bridge result parent drift",
    )?;
    let name = environment
        .artifact
        .file_name()
        .ok_or_else(|| invalid_data("R52 bridge result name is missing"))?;
    publish_descriptor_relative(
        &environment.evidence_root,
        Path::new(""),
        name,
        &canonical_json(&result)?,
    )?;
    Ok(())
}

fn run_r52_challenge_bridge(request: &R52BridgeRequest) -> io::Result<()> {
    let (_challenge_input, challenge) = load_r52_challenge_manifest(request)?;
    let h01 = &challenge.entries[0];
    let selected_input = HeldInput::open_bounded(&h01.source_path, BYTE_CEILING)?;
    selected_input.require_file_and_parent_security(effective_owner()?, 0o600, 0o700)?;
    require(
        sha256_hex(selected_input.bytes()) == h01.source_sha256,
        "R52 challenge selected input hash drift",
    )?;
    let decoded_fingerprint =
        canonical_decoded_rgba_blake3(selected_input.bytes()).map_err(io::Error::other)?;
    let held_r49 = load_schema_and_hold_assets(
        &request.r49_visual_manifest_path,
        &request.r49_visual_manifest_sha256,
        &h01.source_path,
        &decoded_fingerprint,
        &h01.source_sha256,
    )
    .map_err(io::Error::other)?;
    let mut validated_r49 = validate_visual_oracles(
        validate_dimensions_and_masks(held_r49).map_err(io::Error::other)?,
    )
    .map_err(io::Error::other)?;
    let h04_index = validated_r49
        .upstream
        .held_schema
        .schema
        .entries
        .iter()
        .position(|entry| entry.id == "r49-h04")
        .ok_or_else(|| invalid_data("R52 regression challenge h04 is missing"))?;
    apply_r52_protected_latin_correction(
        &mut validated_r49.upstream.held_schema.schema.entries[h04_index],
        &mut validated_r49.entries[h04_index],
    )?;
    let supplemental = challenge.entries[4..]
        .iter()
        .map(load_r52_supplemental_entry)
        .collect::<io::Result<Vec<_>>>()?;
    for entry in &supplemental {
        entry
            .held_source
            .with_revalidated_path(|validation| validation.with_current_namespace(|| Ok(())))?;
        entry
            .held_note
            .with_revalidated_path(|validation| validation.with_current_namespace(|| Ok(())))?;
    }
    let artifact_bytes = fs::read(&request.calibration_selection_artifact_path)?;
    let artifact: FrozenArtifact = serde_json::from_slice(&artifact_bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let preflight = HeldInput::open(&request.b0_preflight_attestation_path)?;
    let environment = SelectionEnvironment {
        phase: Phase::Holdout,
        formal_custody: None,
        b0_sha: request.b0_sha.clone(),
        visual_input: h01.source_path.clone(),
        visual_input_sha256: h01.source_sha256.clone(),
        visual_manifest: request.r49_visual_manifest_path.clone(),
        visual_manifest_sha256: request.r49_visual_manifest_sha256.clone(),
        calibration_manifest_sha256: artifact.manifest_sha256,
        evidence_root: request.evidence_root.clone(),
        report_dir: request.evidence_root.join("r52-challenge-bridge"),
        source_gate_fixture_manifest_sha256: request
            .source_gate_fixture_manifest_sha256
            .clone(),
        artifact: request.result_path.clone(),
        calibration_entry_ids: Vec::new(),
        holdout_entry_ids: challenge
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect(),
        required_check: RequiredCheck {
            phase: "challenge".into(),
            command: REQUIRED_CHECK_COMMAND.into(),
            checker_endpoint_sha256: String::new(),
            manifest_sha256: request.challenge_manifest_sha256.clone(),
            source_gate_fixture_manifest_sha256: request
                .source_gate_fixture_manifest_sha256
                .clone(),
            attestation_relpath: request
                .b0_preflight_attestation_path
                .strip_prefix(&request.evidence_root)
                .ok()
                .and_then(Path::to_str)
                .unwrap_or("r52-b0-preflight.json")
                .into(),
            attestation_sha256: sha256_hex(preflight.bytes()),
            b0_sha: request.b0_sha.clone(),
            result: "pass".into(),
        },
        required_check_attestation: preflight,
        selected_candidate_override: Some(request.selected_candidate_id.clone()),
        held_calibration_artifact_sha256: OnceCell::new(),
        frozen_candidate_id: OnceCell::new(),
    };
    let executable_sha256 = sha256_file(&std::env::current_exe()?)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    validated_r49
        .upstream
        .held_schema
        .with_revalidated_paths(|| {
            let mut entries = Vec::with_capacity(9);
            for ((schema, decoded), oracle) in validated_r49
                .upstream
                .held_schema
                .schema
                .entries
                .iter()
                .zip(&validated_r49.upstream.entries)
                .zip(&validated_r49.entries)
            {
                if let Some(expected) = challenge.entries[..4]
                    .iter()
                    .find(|entry| entry.id == schema.id)
                {
                    require(
                        schema.role == EntryRole::Holdout
                            && schema.path == expected.source_path.to_string_lossy()
                            && schema.sha256 == expected.source_sha256,
                        "R52 regression challenge R49 binding drift",
                    )?;
                    entries.push(RealModelEntry {
                        schema,
                        source: &decoded.source,
                        oracle,
                        source_ink_masks: decoded
                            .targets
                            .iter()
                            .map(|target| {
                                SourceInkMask::page(
                                    &target.agreed_mask,
                                    decoded.source.width(),
                                    decoded.source.height(),
                                )
                            })
                            .collect(),
                    });
                }
            }
            require(
                entries.len() == 4
                    && entries
                        .iter()
                        .map(|entry| entry.schema.id.as_str())
                        .eq(["r49-h01", "r49-h02", "r49-h03", "r49-h04"])
                    && entries[3].schema.protected_rois.iter().any(|roi| {
                        entries[3].oracle.protected_rois.iter().any(|protected| {
                            *roi == [
                                u64::from(protected.left),
                                u64::from(protected.top),
                                u64::from(protected.right),
                                u64::from(protected.bottom),
                            ]
                        })
                    })
                    && entries[3]
                        .schema
                        .targets
                        .iter()
                        .all(|target| target.id != "product-id"),
                "R52 regression challenge oracle set drift",
            )?;
            entries.extend(supplemental.iter().map(|entry| {
                RealModelEntry {
                    schema: &entry.schema,
                    source: &entry.source,
                    oracle: &entry.oracle,
                    source_ink_masks: entry
                        .oracle
                        .targets
                        .iter()
                        .map(|target| {
                            SourceInkMask::edit_roi(&target.delta_mask, target.edit_roi)
                        })
                        .collect(),
                }
            }));
            runtime
                .block_on(run_real_model_async(
                    &environment,
                    &entries,
                    executable_sha256,
                    None,
                    RealModelRunMode::Matrix,
                ))
                .map(|_| ())
        })
}

fn run_r52_evidence_bridge() -> io::Result<()> {
    require(false, HISTORICAL_CUSTODY_COMMAND_RETIRED)?;
    let (request_path, request_input, request) = load_r52_bridge_request()?;
    validate_r52_bridge_request(&request_path, &request)?;
    request_input.with_revalidated_path(|validation| {
        validation.with_current_namespace(|| run_r52_challenge_bridge(&request))
    })
}

#[derive(Clone, Copy)]
struct SourceInkMask<'a> {
    bytes: &'a [u8],
    origin: [u32; 2],
    width: u32,
    height: u32,
}

impl<'a> SourceInkMask<'a> {
    fn page(bytes: &'a [u8], width: u32, height: u32) -> Self {
        Self {
            bytes,
            origin: [0, 0],
            width,
            height,
        }
    }

    fn edit_roi(bytes: &'a [u8], rect: ValidatedHalfOpenRect) -> Self {
        Self {
            bytes,
            origin: [rect.left, rect.top],
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        }
    }

    fn get(self, x: u32, y: u32) -> Option<u8> {
        let x = x.checked_sub(self.origin[0])?;
        let y = y.checked_sub(self.origin[1])?;
        if x >= self.width || y >= self.height {
            return None;
        }
        self.bytes
            .get(y as usize * self.width as usize + x as usize)
            .copied()
    }
}

fn run_formal_model(environment: &SelectionEnvironment) -> io::Result<RunnerEvidence> {
    let custody = environment
        .formal_custody
        .as_ref()
        .ok_or_else(|| invalid_data("formal custody is unavailable"))?;
    let holdout = custody
        .holdout
        .as_ref()
        .ok_or_else(|| invalid_data("formal holdout custody is unavailable"))?;
    require(
        holdout.open_marker.get().is_some() && holdout.runtime_commitment.get().is_some(),
        "R59 start marker and runtime commitment must be validated before bundle access",
    )?;
    let runtime = holdout
        .runtime_commitment
        .get()
        .ok_or_else(|| invalid_data("R59 runtime commitment is unavailable"))?;
    require_formal_plaintext_root(custody.revision, &holdout.plaintext_directory)?;
    let archive = HeldInput::open(&holdout.plaintext_archive)?;
    let freeze = R59BundleFreezeCommitments {
        plaintext_archive_sha256: &runtime.plaintext_archive_sha256,
        manifest_sha256: &runtime.manifest_sha256,
        oracle_sha256: &runtime.oracle_sha256,
        hashes_sha256: &runtime.hashes_sha256,
    };
    let validated = validate_r60_plaintext_holdout_bundle(
        &holdout.plaintext_directory,
        &holdout.plaintext_archive,
        archive.bytes(),
        freeze,
    )?;
    let prepared = prepare_r59_execution_entries(validated.execution)?;
    let entries = prepared
        .iter()
        .map(|(schema, source, oracle)| RealModelEntry {
            schema,
            source,
            oracle,
            source_ink_masks: oracle
                .targets
                .iter()
                .map(|target| SourceInkMask::edit_roi(&target.delta_mask, target.edit_roi))
                .collect(),
        })
        .collect::<Vec<_>>();
    let executable_sha256 = sha256_file(&std::env::current_exe()?)?;
    let bundle_validation_receipt = write_r59_bundle_validation_receipt(
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
        RealModelRunMode::Matrix,
    ))
}

fn prepare_r59_execution_entries(
    validated: R59ValidatedExecutionView,
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
                "R59 validated execution dimensions drift",
            )?;
            let page_len =
                usize::try_from(u64::from(entry.source_width) * u64::from(entry.source_height))
                    .map_err(|_| invalid_data("R59 execution page length overflow"))?;
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
                    "R59 validated execution mask drift",
                )?;
                let source_roi = validated_rect(target.source_roi)?;
                let edit_roi = validated_rect(target.clean_reference_edit_roi)?;
                let mut local_mask = Vec::with_capacity(
                    usize::try_from(
                        u64::from(edit_roi.right - edit_roi.left)
                            * u64::from(edit_roi.bottom - edit_roi.top),
                    )
                    .map_err(|_| invalid_data("R59 execution mask length overflow"))?,
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
                        _ => return Err(invalid_data("R59 execution position drift")),
                    },
                    writing: match target.writing.as_str() {
                        "horizontal" => Writing::Horizontal,
                        "vertical" => Writing::Vertical,
                        _ => return Err(invalid_data("R59 execution writing drift")),
                    },
                    effect: match target.effect.as_str() {
                        "plain" => Effect::Plain,
                        "stroke" => Effect::Stroke,
                        _ => return Err(invalid_data("R59 execution effect drift")),
                    },
                    translation_length: match target.translation_length.as_str() {
                        "short" => TranslationLength::Short,
                        "equal" => TranslationLength::Equal,
                        "2x" => TranslationLength::TwoX,
                        "3x" => TranslationLength::ThreeX,
                        _ => {
                            return Err(invalid_data("R59 execution translation length drift"));
                        }
                    },
                    expected: match target.expected.as_str() {
                        "automatic_strict" => Expected::AutomaticStrict,
                        _ => return Err(invalid_data("R59 execution expected mode drift")),
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
        "R59 validated execution rectangle drift",
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
    mode: RealModelRunMode,
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
    let selected = match mode {
        RealModelRunMode::Matrix => selected_candidates(environment)?,
        RealModelRunMode::EraseStageProbe => {
            vec![("S25L4".into(), SourceGateCropPolicy::S25L4)]
        }
    };
    let pipeline_config = PipelineConfig::default();
    require(
        pipeline_config.inpainter == "lama-manga"
            && pipeline_config.bubble_segmenter == "speech-bubble-segmentation",
        "R57 frozen runtime removal configuration drift",
    )?;
    let mut process_evidence = Vec::with_capacity(2);
    let mut results = Vec::new();
    let mut formal_cells = Vec::new();
    let mut first_failed_cell = None;
    let mut r52_cells = Vec::new();
    let mut r52_failed = false;
    let mut runners = Vec::with_capacity(2);

    for (device, cpu) in [("cpu", true), ("metal", false)] {
        let evidence_device = formal_revision(environment)
            .map_or(device, |revision| revision.external_device(device));
        let mut logs = NativeLogCaptureGuard::start();
        let vl = PaddleOcrVl::load(&runtime, cpu, backend.clone())
            .await
            .map_err(io::Error::other)?;
        let vl_evidence = vl.device_evidence();
        let load_bytes = logs.take();
        let parsed_load = parse_native_load_log(&load_bytes)?;
        let load_log = write_raw_log(
            environment,
            &format!("source-gate/{phase}/{evidence_device}/load.log"),
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
        let segmenter = ComicTextDetector::load_segmentation_only(&runtime, cpu)
            .await
            .map_err(io::Error::other)?;
        let bubble_segmenter = SpeechBubbleSegmentation::load(&runtime, cpu)
            .await
            .map_err(io::Error::other)?;
        runners.push((
            device,
            vl,
            vl_evidence,
            segmenter,
            bubble_segmenter,
            process,
        ));
    }

    for entry in entries
        .iter()
        .filter(|entry| entry.schema.role == phase_role(environment.phase))
    {
        let schema_entry = entry.schema;
        let oracle_entry = entry.oracle;
        for (device, vl, vl_evidence, segmenter, bubble_segmenter, process) in &mut runners {
            let evidence_device = formal_revision(environment)
                .map_or(*device, |revision| revision.external_device(device));
            let image = DynamicImage::ImageRgba8(entry.source.clone());
            let bubble_result = bubble_segmenter
                .inference(&image)
                .map_err(io::Error::other)?;
            let bubble_support = bubble_mask_from_result(&bubble_result);
            for (candidate_id, policy) in &selected {
                let mut logs = NativeLogCaptureGuard::start();
                let mut scene = scene_for_entry(
                    schema_entry,
                    oracle_entry,
                    entry.source.width(),
                    entry.source.height(),
                );
                let page = *scene.pages.keys().next().expect("scene page");
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
                let eligible_lines = eligible_lines_for_page(&scene, page).0;
                let readiness = vec!["translation-ready".into(); eligible_lines.len()];
                for mut op in build_han_only_translation_ops(
                    &scene,
                    page,
                    None,
                    &eligible_lines,
                    &readiness,
                )
                .map_err(io::Error::other)?
                {
                    op.apply(&mut scene).map_err(io::Error::other)?;
                }
                let erase_diagnostics = if mode == RealModelRunMode::EraseStageProbe {
                    Some(EraseDiagnosticCapture::start().map_err(|_| {
                        invalid_data("erase-stage diagnostic capture is already active")
                    })?)
                } else {
                    None
                };
                let segment_support = dispatch_segment(
                    &image,
                    &scene,
                    page,
                    SourceTextPolicy::HanOnly,
                    |image| segmenter.inference_segmentation(image),
                )
                .map_err(io::Error::other)?;
                let protected_lines = protected_source_lines_for_page(&scene, page);
                let removal_support = removal_support_from_prepared(
                    prepare_inpaint_mask(
                        &DynamicImage::ImageLuma8(segment_support),
                        &DynamicImage::ImageLuma8(bubble_support.clone()),
                        &[],
                        &eligible_lines,
                        &protected_lines,
                        SourceTextPolicy::HanOnly,
                        None,
                        expand_mask_for_inpainting,
                    )
                    .map_err(io::Error::other)?,
                    image.width(),
                    image.height(),
                );
                if let Some(capture) = erase_diagnostics.as_ref() {
                    write_erase_stage_probe(
                        environment,
                        schema_entry,
                        oracle_entry,
                        &entry.source_ink_masks,
                        device,
                        candidate_id,
                        capture.take_stage_masks(),
                        &removal_support,
                    )?;
                }
                let inference_bytes = logs.take();
                let parsed_inference = parse_native_inference_log(&inference_bytes)?;
                let inference_log = write_raw_log(
                    environment,
                    &format!(
                        "source-gate/{phase}/{}/{evidence_device}/{candidate_id}.log",
                        schema_entry.id
                    ),
                    &inference_bytes,
                )?;
                let source_gate_diagnostic = write_raw_log(
                    environment,
                    &format!(
                        "source-gate/{phase}/{}/{evidence_device}/{candidate_id}.source-gate.json",
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
                    &entry.source_ink_masks,
                    &removal_support,
                    &pipeline_config.inpainter,
                    &pipeline_config.bubble_segmenter,
                    &sha256_hex(bubble_support.as_raw()),
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
                if environment.selected_candidate_override.is_some() {
                    let cell = write_r52_challenge_cell(
                        environment,
                        r52_cells.len(),
                        process,
                        &result,
                        schema_entry,
                        oracle_entry,
                        &source_gate_events,
                    )?;
                    r52_failed = cell.result == "fail";
                    result.derived.passed = !r52_failed;
                    r52_cells.push(cell);
                }
                if environment.formal_custody.is_some() {
                    let cell = match environment.phase {
                        Phase::CalibrationFreeze => write_r59_calibration_cell_evidence(
                            environment,
                            process,
                            &result,
                            schema_entry,
                            oracle_entry,
                            &source_gate_events,
                            &supports,
                        )?,
                        Phase::Holdout => write_r59_cell_evidence(
                            environment,
                            process,
                            &mut result,
                            schema_entry,
                            oracle_entry,
                            &source_gate_events,
                            &supports,
                            bundle_validation_receipt
                                .as_ref()
                                .ok_or_else(|| invalid_data("R59 bundle receipt is missing"))?,
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
                if r52_failed {
                    break;
                }
            }
            if first_failed_cell.is_some() || r52_failed {
                break;
            }
        }
        if first_failed_cell.is_some() || r52_failed {
            break;
        }
    }
    process_evidence.extend(runners.into_iter().map(|(_, _, _, _, _, process)| process));
    let selected_candidate_id = match mode {
        RealModelRunMode::EraseStageProbe => selected[0].0.clone(),
        RealModelRunMode::Matrix if environment.phase == Phase::CalibrationFreeze => {
            select_or_write_calibration_diagnostic(environment, &process_evidence, &results)?
        }
        RealModelRunMode::Matrix => selected[0].0.clone(),
    };
    let formal = environment
        .formal_custody
        .as_ref()
        .map(|_| R59FormalRunEvidence {
            bundle_validation_receipt,
            cells: formal_cells,
            first_failed_cell,
        });
    if environment.selected_candidate_override.is_some() {
        publish_r52_bridge_result(environment, &r52_cells)?;
    }
    Ok(RunnerEvidence {
        selected_candidate_id,
        process_evidence,
        results,
        formal,
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
    environment.formal_custody.as_ref().map_or_else(
        || environment.holdout_entry_ids.clone(),
        |_| r59_entry_ids('h'),
    )
}

fn terminal_holdout_entry_ids(environment: &SelectionEnvironment) -> Vec<String> {
    environment.formal_custody.as_ref().map_or_else(
        || environment.holdout_entry_ids.clone(),
        |custody| custody.revision.entry_ids(),
    )
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
    if let Some(selected) = &environment.selected_candidate_override {
        return all
            .into_iter()
            .find(|(id, _)| *id == selected)
            .map(|(id, policy)| vec![(id.into(), policy)])
            .ok_or_else(|| invalid_data("R52 bridge selected candidate is unknown"));
    }
    if environment.phase == Phase::CalibrationFreeze {
        return Ok(all
            .into_iter()
            .map(|(id, policy)| (id.into(), policy))
            .collect());
    }
    all.into_iter()
        .find(|(id, _)| environment.frozen_candidate_id.get().map(String::as_str) == Some(*id))
        .map(|(id, policy)| vec![(id.into(), policy)])
        .ok_or_else(|| invalid_data("frozen candidate is unknown"))
}

fn removal_support_from_prepared(
    prepared: PreparedInpaintMask,
    width: u32,
    height: u32,
) -> GrayImage {
    match prepared {
        PreparedInpaintMask::Prepared { mask, .. } => mask.to_luma8(),
        PreparedInpaintMask::NoEligibleHanTargets | PreparedInpaintMask::EmptyMask => {
            GrayImage::new(width, height)
        }
    }
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

fn r59_quad_bits_rect(bits: [u32; 8]) -> io::Result<[i64; 4]> {
    let xs = [0, 2, 4, 6].map(|index| f32::from_bits(bits[index]));
    let ys = [1, 3, 5, 7].map(|index| f32::from_bits(bits[index]));
    require(
        xs.iter().chain(&ys).all(|value| value.is_finite()),
        "R59 selection geometry is non-finite",
    )?;
    Ok([
        xs.iter().copied().fold(f32::INFINITY, f32::min).floor() as i64,
        ys.iter().copied().fold(f32::INFINITY, f32::min).floor() as i64,
        xs.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i64,
        ys.iter().copied().fold(f32::NEG_INFINITY, f32::max).ceil() as i64,
    ])
}

fn r59_rect_mask(width: u32, height: u32, rect: [i64; 4]) -> Vec<u8> {
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

fn r59_selected_support_from_diagnostics(
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
            let rect = r59_quad_bits_rect(target.scene_quad_f32_bits)?;
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
                "R59 emitted target geometry ownership is not unique",
            )?;
            let support = selected
                .entry(matches[0].to_owned())
                .or_insert_with(|| vec![0; width as usize * height as usize]);
            for (pixel, addition) in support.iter_mut().zip(r59_rect_mask(width, height, rect))
            {
                *pixel |= addition;
            }
        }
    }
    Ok(selected)
}

fn r59_downstream_support_from_scene(
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
            .ok_or_else(|| invalid_data("R59 downstream scene target is unassigned"))?;
        let lines = eligible_text_lines(&node.transform, text, page.width, page.height)
            .ok_or_else(|| invalid_data("R59 downstream scene geometry is unsupported"))?;
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

fn source_ink_is_covered(
    page_width: u32,
    edit_roi: ValidatedHalfOpenRect,
    source_ink_mask: SourceInkMask<'_>,
    support: &[u8],
) -> bool {
    (edit_roi.top..edit_roi.bottom).all(|y| {
        (edit_roi.left..edit_roi.right).all(|x| {
            source_ink_mask.get(x, y).is_some_and(|ink| {
                ink == 0
                    || support
                        .get(y as usize * page_width as usize + x as usize)
                        .is_some_and(|pixel| *pixel != 0)
            })
        })
    })
}

fn write_erase_stage_probe(
    environment: &SelectionEnvironment,
    schema: &VisualManifestEntry,
    oracle: &OracleValidatedEntry,
    source_ink_masks: &[SourceInkMask<'_>],
    device: &str,
    candidate_id: &str,
    stage_masks: Vec<EraseStageMask>,
    removal_support: &GrayImage,
) -> io::Result<()> {
    const EXPECTED: [EraseDiagnosticStage; 9] = [
        EraseDiagnosticStage::SegmentProbability,
        EraseDiagnosticStage::SegmentRefined,
        EraseDiagnosticStage::SegmentAllowedSupport,
        EraseDiagnosticStage::SegmentFinal,
        EraseDiagnosticStage::InpaintInputSegment,
        EraseDiagnosticStage::InpaintAllowedSupport,
        EraseDiagnosticStage::InpaintPreExpandFiltered,
        EraseDiagnosticStage::InpaintBackendExpanded,
        EraseDiagnosticStage::InpaintFinal,
    ];
    require(
        stage_masks.len() == EXPECTED.len()
            && stage_masks.iter().zip(EXPECTED).all(|(snapshot, stage)| {
                snapshot.stage == stage && snapshot.branch == EraseDiagnosticBranch::HanOnly
            }),
        "erase-stage probe sequence drift",
    )?;
    let segment_final = &stage_masks[3].mask;
    let inpaint_input = &stage_masks[4].mask;
    let inpaint_final = &stage_masks[8].mask;
    require(
        segment_final.as_raw() == inpaint_input.as_raw()
            && inpaint_final.as_raw() == removal_support.as_raw(),
        "erase-stage probe boundary bytes drift",
    )?;
    let (width, height) = removal_support.dimensions();
    let stages = stage_masks
        .into_iter()
        .map(|snapshot| {
            require(
                snapshot.mask.dimensions() == (width, height),
                "erase-stage probe dimensions drift",
            )?;
            let targets = schema
                .targets
                .iter()
                .zip(&oracle.targets)
                .zip(source_ink_masks)
                .map(|((target, geometry), source_ink)| {
                    let mut oracle_pixels = 0;
                    let mut intersection_pixels = 0;
                    for y in geometry.edit_roi.top..geometry.edit_roi.bottom {
                        for x in geometry.edit_roi.left..geometry.edit_roi.right {
                            if source_ink.get(x, y).is_some_and(|pixel| pixel != 0) {
                                oracle_pixels += 1;
                                if snapshot.mask.get_pixel(x, y).0[0] != 0 {
                                    intersection_pixels += 1;
                                }
                            }
                        }
                    }
                    EraseStageTargetMetric {
                        target_id: target.id.clone(),
                        oracle_pixels,
                        intersection_pixels,
                        missing_pixels: oracle_pixels - intersection_pixels,
                    }
                })
                .collect();
            Ok(EraseStageMetric {
                stage: snapshot.stage,
                branch: snapshot.branch,
                grayscale_blake3: blake3::hash(snapshot.mask.as_raw()).to_hex().to_string(),
                nonzero_pixels: snapshot
                    .mask
                    .pixels()
                    .filter(|pixel| pixel.0[0] != 0)
                    .count() as u64,
                protected_overlap_pixels: protected_overlap_count(
                    snapshot.mask.as_raw(),
                    width,
                    &oracle.protected_rois,
                ),
                targets,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let final_stage = stages
        .last()
        .ok_or_else(|| invalid_data("erase-stage probe final stage is missing"))?;
    require(
        r57_final_erase_stage_passed(final_stage),
        "erase-stage probe final coverage failed",
    )?;
    let report = EraseStageProbeReport {
        version: 1,
        entry_id: schema.id.clone(),
        candidate_id: candidate_id.into(),
        device: device.into(),
        width,
        height,
        stages,
    };
    write_raw_log(
        environment,
        &format!(
            "erase-stage-probe/{}/{device}/{candidate_id}.erase-stages.json",
            schema.id
        ),
        &canonical_json(&report)?,
    )?;
    Ok(())
}

fn r57_final_erase_stage_passed(stage: &EraseStageMetric) -> bool {
    stage.protected_overlap_pixels == 0
        && stage
            .targets
            .iter()
            .all(|target| target.missing_pixels == 0)
}

fn r57_cell_passed(
    derived_passed: bool,
    detector_geometry_passed: bool,
    protected_overlap_pixels: u64,
) -> bool {
    derived_passed && detector_geometry_passed && protected_overlap_pixels == 0
}

fn derive_result(
    device: &str,
    scene: &Scene,
    page: koharu_core::PageId,
    schema: &VisualManifestEntry,
    oracle: &OracleValidatedEntry,
    source_ink_masks: &[SourceInkMask<'_>],
    removal_support: &GrayImage,
    runtime_inpainter_id: &str,
    bubble_segmenter_id: &str,
    bubble_support_sha256: &str,
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
    let mut scene_by_target = BTreeMap::new();
    let mut selected_scene_rotations_zero = true;
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
                match r57_actual_scene_support(page.width, page.height, &node.transform, text) {
                    Some((support, rotations_zero)) => {
                        selected_scene_rotations_zero &= rotations_zero;
                        scene_by_target
                            .entry(target.id.clone())
                            .or_insert_with(Vec::new)
                            .push(support);
                    }
                    None => selected_scene_rotations_zero = false,
                }
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
    let downstream_support_by_target = r59_downstream_support_from_scene(page, schema, oracle)?;
    require(
        removal_support.dimensions() == (page.width, page.height),
        "runtime source-ink support dimensions drift",
    )?;
    let mut covered_source_roi_ids = schema
        .targets
        .iter()
        .zip(&oracle.targets)
        .zip(source_ink_masks)
        .filter(|((target, _), _)| target.expected != Expected::UnsupportedRotation)
        .filter_map(|((target, oracle_target), source_ink_mask)| {
            source_ink_is_covered(
                page.width,
                oracle_target.edit_roi,
                *source_ink_mask,
                removal_support.as_raw(),
            )
            .then(|| target.id.clone())
        })
        .collect::<Vec<_>>();
    covered_source_roi_ids.sort();
    let source_text_roi_coverage = if expected.is_empty() {
        1.0
    } else {
        covered_source_roi_ids.len() as f64 / expected.len() as f64
    };
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
    let selected_by_target = r59_selected_support_from_diagnostics(
        page.width,
        page.height,
        schema,
        oracle,
        diagnostics,
    )?;
    pp_vl_incomplete_coverage |= selected_by_target != downstream_by_target;
    let pp_vl_complete_coverage = expected_diagnostic_nodes > 0
        && !rejected_after_vl
        && !pp_vl_incomplete_coverage
        && source_text_roi_coverage == 1.0;
    let source_removal_preflight_passed = recall == 1.0 && pp_vl_complete_coverage;
    let passed = source_removal_preflight_passed
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
            scene_by_target,
            selected_scene_rotations_zero,
            runtime_inpainter_id: runtime_inpainter_id.to_owned(),
            bubble_segmenter_id: bubble_segmenter_id.to_owned(),
            bubble_support_sha256: bubble_support_sha256.to_owned(),
            removal_support: removal_support
                .as_raw()
                .iter()
                .map(|pixel| u8::from(*pixel != 0))
                .collect(),
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
    let suffix = formal_external_evidence_suffix(formal_revision(environment), suffix);
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

fn write_r59_bundle_validation_receipt(
    environment: &SelectionEnvironment,
    executable_sha256: &str,
    validated: &R59ValidatedReceiptData,
) -> io::Result<PublishedArtifact> {
    let custody = environment
        .formal_custody
        .as_ref()
        .ok_or_else(|| invalid_data("R59 formal custody is not enabled"))?;
    let holdout = custody
        .holdout
        .as_ref()
        .ok_or_else(|| invalid_data("R59 holdout custody is unavailable"))?;
    require(
        environment.phase == Phase::Holdout && holdout.open_marker.get().is_some(),
        "R59 bundle receipt is holdout-only",
    )?;
    let runtime_receipt_sha256 = &holdout
        .runtime_commitment
        .get()
        .ok_or_else(|| invalid_data("formal runtime commitment is unavailable"))?
        .receipt
        .sha256;
    let bytes = canonical_json(&R60BundleValidationReceipt {
        contract: "hanonly-r60-bundle-validation-v1",
        runtime_bundle_schema: "hanonly-r60-runtime-bundle-v1",
        plan_revision: 60,
        b0_sha: &environment.b0_sha,
        test_executable_sha256: executable_sha256,
        enabled_cargo_features: ["hanonly-test-evidence"],
        r60_contract_sha256: &custody.contract_sha256,
        public_commitment_sha256: &holdout.freeze.original_public_commitment_sha256,
        successor_commitment_sha256: &holdout.freeze.receipt_sha256,
        source_b0_sha: &holdout.freeze.original_b0_sha,
        successor_b0_sha: &holdout.freeze.successor_b0_sha,
        private_manifest_commitment_sha256: &holdout.freeze.private_manifest_commitment_sha256,
        runtime_commitment_receipt_sha256: runtime_receipt_sha256,
        plaintext_archive_sha256: &validated.plaintext_archive_sha256,
        manifest_sha256: &validated.manifest_sha256,
        oracle_sha256: &validated.oracle_sha256,
        hashes_sha256: &validated.hashes_sha256,
        schema_validation_pass: validated.schema_validation_pass,
        asset_binding_pass: validated.asset_binding_pass,
        mask_source_clean_equality_pass: validated.mask_source_clean_equality_pass,
        oracle_semantics_pass: validated.oracle_semantics_pass,
        result: "pass",
    })?;
    publish_r59_artifact(environment, "r59/bundle-validation.json", &bytes)
}

fn load_formal_successor_commitments(
    original_path: &Path,
    successor_path: &Path,
    requested_b0_sha: &str,
    contract_sha256: &str,
    test_spec_path: &Path,
) -> io::Result<FreezeCommitments> {
    let original = HeldInput::open(original_path)?;
    let successor = HeldInput::open(successor_path)?;
    let test_spec_sha256 = sha256_file(test_spec_path)?;
    require(
        test_spec_sha256 == R60_TEST_SPEC_SHA256,
        "formal custody test spec hash drift",
    )?;
    let layout = HeldInput::open(
        &original_path
            .parent()
            .ok_or_else(|| invalid_data("R60 public commitment has no parent"))?
            .join(R60_LAYOUT_RECEIPT_NAME),
    )?;
    validate_r60_successor_commitments(
        layout.bytes(),
        original.bytes(),
        &hex_sha256(original.sha256()),
        successor.bytes(),
        &hex_sha256(successor.sha256()),
        requested_b0_sha,
        contract_sha256,
        &test_spec_sha256,
    )
}

fn validate_r60_successor_commitments(
    layout_bytes: &[u8],
    public_bytes: &[u8],
    public_sha256: &str,
    successor_bytes: &[u8],
    successor_sha256: &str,
    requested_b0_sha: &str,
    contract_sha256: &str,
    test_spec_sha256: &str,
) -> io::Result<FreezeCommitments> {
    let layout: R60LayoutReceipt = serde_json::from_slice(layout_bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let layout_sha256 = sha256_hex(layout_bytes);
    let public: R60PublicCommitment = serde_json::from_slice(public_bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    require(
        canonical_json(&layout)? == layout_bytes
            && layout.schema == "hanonly.r60.layout-receipt.v1"
            && layout.plan_revision == 60
            && layout.manifest_sha256 == layout.private_manifest_commitment_sha256
            && layout.entry_ids == FormalRevision::R60.entry_ids()
            && layout.required_root_present
            && layout.wrapper_absent
            && layout.canonical_ustar_pass
            && layout.manifest_binding_pass
            && layout.same_archive_object_pass
            && layout.layout_pass
            && !layout.restricted_values_disclosed
            && canonical_json(&public)? == public_bytes
            && public.schema == "hanonly.r60.public-commitment.v1"
            && public.plan_revision == 60
            && public.source_b0_sha == R60_SOURCE_B0_SHA
            && public.manifest_sha256 == public.private_manifest_commitment_sha256
            && public.entry_ids == FormalRevision::R60.entry_ids()
            && public.cleanup_pass
            && !public.restricted_values_disclosed
            && public.start_marker_absent,
        "R60 layout or public commitment drift",
    )?;
    let successor: R60SuccessorCommitment = serde_json::from_slice(successor_bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    require(
        canonical_json(&successor)? == successor_bytes
            && successor.schema == "hanonly.r60.successor-commitment.v1"
            && successor.plan_revision == 60
            && successor.public_commitment_sha256 == public_sha256
            && successor.source_b0_sha == public.source_b0_sha
            && successor.successor_b0_sha == requested_b0_sha
            && successor.contract_sha256 == contract_sha256
            && successor.test_spec_sha256 == test_spec_sha256
            && successor.calibration_artifact_sha256 == R59_CALIBRATION_ARTIFACT_SHA256
            && successor.selected_candidate_id == "S25L4"
            && successor.ciphertext_sha256 == public.ciphertext_sha256
            && successor.ciphertext_sha256 == layout.ciphertext_sha256
            && successor.layout_receipt_sha256 == layout_sha256
            && successor.layout_receipt_sha256 == public.layout_receipt_sha256
            && successor.layout_validator_sha256 == layout.layout_validator_sha256
            && successor.layout_validator_sha256 == public.layout_validator_sha256
            && successor.manifest_sha256 == layout.manifest_sha256
            && successor.manifest_sha256 == public.manifest_sha256
            && successor.member_name_digest_sha256 == layout.member_name_digest_sha256
            && successor.member_name_digest_sha256 == public.member_name_digest_sha256
            && successor.private_manifest_commitment_sha256
                == layout.private_manifest_commitment_sha256
            && successor.private_manifest_commitment_sha256
                == public.private_manifest_commitment_sha256
            && successor.entry_ids == FormalRevision::R60.entry_ids()
            && successor.package_unchanged
            && successor.start_marker_absent,
        "R60 successor commitment drift",
    )?;
    for hash in [
        public_sha256,
        successor_sha256,
        &successor.contract_sha256,
        &successor.test_spec_sha256,
        &successor.calibration_artifact_sha256,
        &successor.ciphertext_sha256,
        &successor.layout_receipt_sha256,
        &successor.layout_validator_sha256,
        &successor.manifest_sha256,
        &successor.member_name_digest_sha256,
        &successor.private_manifest_commitment_sha256,
    ] {
        decode_sha256(hash)?;
    }
    require_git_sha(&successor.source_b0_sha)?;
    require_git_sha(&successor.successor_b0_sha)?;
    Ok(FreezeCommitments {
        receipt_sha256: successor_sha256.to_owned(),
        original_public_commitment_sha256: public_sha256.to_owned(),
        original_b0_sha: successor.source_b0_sha,
        successor_b0_sha: successor.successor_b0_sha,
        calibration_artifact_sha256: successor.calibration_artifact_sha256,
        ciphertext_sha256: successor.ciphertext_sha256,
        private_manifest_commitment_sha256: successor.private_manifest_commitment_sha256,
        r60_layout: Some(R60LayoutBindings {
            layout_receipt_sha256: successor.layout_receipt_sha256,
            layout_validator_sha256: successor.layout_validator_sha256,
            manifest_sha256: successor.manifest_sha256,
            member_name_digest_sha256: successor.member_name_digest_sha256,
        }),
    })
}

fn hex_sha256(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_formal_runner_open(
    environment: &SelectionEnvironment,
    selected_candidate_id: &str,
) -> io::Result<()> {
    let formal = environment
        .formal_custody
        .as_ref()
        .ok_or_else(|| invalid_data("formal custody is unavailable"))?;
    let holdout = formal
        .holdout
        .as_ref()
        .ok_or_else(|| invalid_data("formal holdout custody is unavailable"))?;
    require(
        holdout.open_marker.get().is_none(),
        "R59 runner open marker was already consumed",
    )?;
    let custody = R59HeldDirectory::open(&holdout.directory)?;
    validate_formal_custody_entry_state(formal.revision, custody.descriptor.as_fd())?;
    let descriptor = openat(
        custody.descriptor.as_fd(),
        formal.revision.start_marker_name(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io::Error::from)?;
    let metadata = r59_descriptor_metadata(descriptor.as_fd())?;
    require(
        metadata.file_type.is_file()
            && metadata.owner == effective_owner()?
            && metadata.mode & 0o7777 == 0o600,
        "R59 runner open marker metadata is invalid",
    )?;
    let mut file = fs::File::from(descriptor);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let actual_sha256 = sha256_hex(&bytes);
    require(
        actual_sha256 == holdout.expected_start_marker_sha256,
        "R59 start marker hash drift",
    )?;
    validate_r60_start_receipt(
        &bytes,
        &environment.b0_sha,
        selected_candidate_id,
        &holdout.freeze,
        &environment.required_check.attestation_sha256,
    )?;
    let fresh = custody.revalidate_descriptor()?;
    let named = statat(
        fresh.as_fd(),
        formal.revision.start_marker_name(),
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(io::Error::from)?;
    require(
        metadata
            == R59DescriptorMetadata {
                dev: named.st_dev as u64,
                ino: named.st_ino,
                owner: named.st_uid.into(),
                mode: named.st_mode.into(),
                file_type: FileType::from_raw_mode(named.st_mode),
            },
        "R59 runner open marker namespace changed",
    )?;
    holdout
        .open_marker
        .set(PublishedArtifact {
            path: formal.revision.start_marker_name().into(),
            sha256: actual_sha256,
            byte_length: bytes.len() as u64,
        })
        .map_err(|_| invalid_data("R59 start marker was already consumed"))
}

fn valid_nonce(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_r60_start_receipt(
    bytes: &[u8],
    b0_sha: &str,
    selected_candidate_id: &str,
    freeze: &FreezeCommitments,
    pre_holdout_attestation_sha256: &str,
) -> io::Result<()> {
    let marker: R60OpenMarker = serde_json::from_slice(bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    require(
        canonical_json(&marker)? == bytes
            && marker.schema == "hanonly.r60.holdout-start.v1"
            && marker.plan_revision == 60
            && marker.b0_sha == b0_sha
            && marker.public_commitment_sha256 == freeze.original_public_commitment_sha256
            && marker.successor_commitment_sha256 == freeze.receipt_sha256
            && marker.calibration_artifact_sha256 == freeze.calibration_artifact_sha256
            && marker.selected_candidate_id == selected_candidate_id
            && marker.entry_ids == FormalRevision::R60.entry_ids()
            && marker.pre_holdout_attestation_sha256 == pre_holdout_attestation_sha256
            && valid_nonce(&marker.nonce_hex)
            && marker.state == "started",
        "R60 start marker binding drift",
    )
}

struct ValidatedRuntimeReceipt {
    plaintext_archive_sha256: String,
    manifest_sha256: String,
    oracle_sha256: String,
    hashes_sha256: String,
}

fn validate_formal_runtime_commitment(environment: &SelectionEnvironment) -> io::Result<()> {
    let formal = environment
        .formal_custody
        .as_ref()
        .ok_or_else(|| invalid_data("formal custody is unavailable"))?;
    let holdout = formal
        .holdout
        .as_ref()
        .ok_or_else(|| invalid_data("formal holdout custody is unavailable"))?;
    require(
        holdout.runtime_commitment.get().is_none(),
        "R59 runtime commitment was already consumed",
    )?;
    let marker = holdout
        .open_marker
        .get()
        .ok_or_else(|| invalid_data("R59 start marker must precede runtime commitment"))?;
    let custody = R59HeldDirectory::open(&holdout.directory)?;
    let descriptor = openat(
        custody.descriptor.as_fd(),
        formal.revision.runtime_commitment_name(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io::Error::from)?;
    let metadata = r59_descriptor_metadata(descriptor.as_fd())?;
    require(
        metadata.file_type.is_file()
            && metadata.owner == effective_owner()?
            && metadata.mode & 0o7777 == 0o600,
        "R59 runtime commitment metadata is invalid",
    )?;
    let mut file = fs::File::from(descriptor);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let validated = validate_r60_runtime_receipt(
        &bytes,
        &environment.b0_sha,
        &marker.sha256,
        &holdout.freeze,
    )?;
    let fresh = custody.revalidate_descriptor()?;
    let named = statat(
        fresh.as_fd(),
        formal.revision.runtime_commitment_name(),
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(io::Error::from)?;
    require(
        metadata
            == R59DescriptorMetadata {
                dev: named.st_dev as u64,
                ino: named.st_ino,
                owner: named.st_uid.into(),
                mode: named.st_mode.into(),
                file_type: FileType::from_raw_mode(named.st_mode),
            },
        "R59 runtime commitment namespace changed",
    )?;
    holdout
        .runtime_commitment
        .set(RuntimeCommitments {
            receipt: PublishedArtifact {
                path: formal.revision.runtime_commitment_name().into(),
                sha256: sha256_hex(&bytes),
                byte_length: bytes.len() as u64,
            },
            plaintext_archive_sha256: validated.plaintext_archive_sha256,
            manifest_sha256: validated.manifest_sha256,
            oracle_sha256: validated.oracle_sha256,
            hashes_sha256: validated.hashes_sha256,
        })
        .map_err(|_| invalid_data("R59 runtime commitment was already consumed"))
}

fn validate_r60_runtime_receipt(
    bytes: &[u8],
    b0_sha: &str,
    start_marker_sha256: &str,
    freeze: &FreezeCommitments,
) -> io::Result<ValidatedRuntimeReceipt> {
    let receipt: R60RuntimeCommitment = serde_json::from_slice(bytes)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let layout = freeze
        .r60_layout
        .as_ref()
        .ok_or_else(|| invalid_data("R60 layout bindings are unavailable"))?;
    require(
        canonical_json(&receipt)? == bytes
            && receipt.schema == "hanonly.r60.runtime-commitment.v1"
            && receipt.plan_revision == 60
            && receipt.b0_sha == b0_sha
            && receipt.start_marker_sha256 == start_marker_sha256
            && receipt.successor_commitment_sha256 == freeze.receipt_sha256
            && receipt.ciphertext_sha256 == freeze.ciphertext_sha256
            && receipt.layout_receipt_sha256 == layout.layout_receipt_sha256
            && receipt.layout_validator_sha256 == layout.layout_validator_sha256
            && receipt.member_name_digest_sha256 == layout.member_name_digest_sha256
            && receipt.private_manifest_commitment_sha256
                == freeze.private_manifest_commitment_sha256
            && receipt.calibration_artifact_sha256 == freeze.calibration_artifact_sha256
            && receipt.selected_candidate_id == "S25L4"
            && receipt.manifest_sha256 == layout.manifest_sha256
            && receipt.entry_ids == FormalRevision::R60.entry_ids()
            && receipt.decrypt_pass
            && receipt.package_unchanged
            && !receipt.restricted_values_disclosed
            && receipt.state == "runtime_committed",
        "R60 runtime commitment binding drift",
    )?;
    for hash in [
        &receipt.start_marker_sha256,
        &receipt.successor_commitment_sha256,
        &receipt.ciphertext_sha256,
        &receipt.layout_receipt_sha256,
        &receipt.layout_validator_sha256,
        &receipt.member_name_digest_sha256,
        &receipt.private_manifest_commitment_sha256,
        &receipt.calibration_artifact_sha256,
        &receipt.plaintext_archive_sha256,
        &receipt.manifest_sha256,
        &receipt.oracle_sha256,
        &receipt.hashes_sha256,
    ] {
        decode_sha256(hash)?;
    }
    Ok(ValidatedRuntimeReceipt {
        plaintext_archive_sha256: receipt.plaintext_archive_sha256,
        manifest_sha256: receipt.manifest_sha256,
        oracle_sha256: receipt.oracle_sha256,
        hashes_sha256: receipt.hashes_sha256,
    })
}

fn validate_formal_custody_entry_state(
    revision: FormalRevision,
    directory: BorrowedFd<'_>,
) -> io::Result<()> {
    let mut names = Dir::read_from(directory)?
        .map(|entry| {
            entry.map(|entry| OsStr::from_bytes(entry.file_name().to_bytes()).to_owned())
        })
        .collect::<rustix::io::Result<Vec<_>>>()
        .map_err(io::Error::from)?;
    names.retain(|name| name != "." && name != "..");
    let namespace = revision.artifact_namespace();
    let start_temporary = format!(".{namespace}-holdout-start.");
    let terminal_temporary = format!(".{namespace}-holdout-terminal.");
    let cleanup_temporary = format!(".{namespace}-cleanup-receipt.");
    let invalid = names.iter().any(|name| {
        let bytes = name.as_bytes();
        name == revision.terminal_receipt_name()
            || name == revision.cleanup_receipt_name()
            || (bytes.starts_with(start_temporary.as_bytes()) && bytes.ends_with(b".tmp"))
            || (bytes.starts_with(terminal_temporary.as_bytes()) && bytes.ends_with(b".tmp"))
            || (bytes.starts_with(cleanup_temporary.as_bytes()) && bytes.ends_with(b".tmp"))
    });
    require(
        names
            .iter()
            .any(|name| name == revision.start_marker_name())
            && !invalid,
        "R59 custody entry state is not started",
    )
}

fn r59_phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::CalibrationFreeze => "calibration-freeze",
        Phase::Holdout => "holdout",
    }
}

fn formal_revision(environment: &SelectionEnvironment) -> Option<FormalRevision> {
    environment
        .formal_custody
        .as_ref()
        .map(|custody| custody.revision)
}

fn formal_external_evidence_suffix(revision: Option<FormalRevision>, suffix: &str) -> String {
    revision.map_or_else(|| suffix.to_owned(), |_| format!("r60/{suffix}"))
}

fn formal_artifact_suffix(revision: Option<FormalRevision>, suffix: &str) -> String {
    suffix.strip_prefix("r59/").map_or_else(
        || suffix.to_owned(),
        |rest| {
            revision.map_or_else(
                || suffix.to_owned(),
                |revision| format!("{}/{rest}", revision.artifact_namespace()),
            )
        },
    )
}

fn r59_contract_path(
    environment: &SelectionEnvironment,
    artifact: &PublishedArtifact,
) -> io::Result<String> {
    let namespace = environment
        .formal_custody
        .as_ref()
        .map_or("r59", |custody| custody.revision.artifact_namespace());
    let artifact_parent = environment
        .artifact
        .parent()
        .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
    artifact_parent
        .join(&artifact.path)
        .strip_prefix(environment.report_dir.join(namespace))
        .map_err(|_| invalid_data("R59 artifact is outside diagnostic root"))?
        .to_str()
        .ok_or_else(|| invalid_data("R59 contract path is not utf-8"))
        .map(str::to_owned)
}

fn r59_mask_descriptor(width: u32, height: u32, bytes: &[u8]) -> serde_json::Value {
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

fn r59_rect_quad(rect: [i64; 4]) -> [i64; 8] {
    let [left, top, right, bottom] = rect;
    [left, top, right, top, right, bottom, left, bottom]
}

fn r57_actual_scene_support(
    width: u32,
    height: u32,
    transform: &Transform,
    text: &TextData,
) -> Option<(SceneSupportEvidence, bool)> {
    let values = [
        transform.x,
        transform.y,
        transform.x + transform.width,
        transform.y + transform.height,
    ];
    if values
        .iter()
        .any(|value| !value.is_finite() || value.fract() != 0.0)
    {
        return None;
    }
    let rect = values.map(|value| value as i64);
    if rect[0] < 0
        || rect[1] < 0
        || rect[0] >= rect[2]
        || rect[1] >= rect[3]
        || rect[2] > i64::from(width)
        || rect[3] > i64::from(height)
    {
        return None;
    }
    Some((
        SceneSupportEvidence {
            rect,
            mask: r59_rect_mask(width, height, rect),
            downstream_mask: line_support_mask(
                width,
                height,
                &eligible_text_lines(transform, text, width, height)?,
            )
            .as_raw()
            .iter()
            .map(|pixel| u8::from(*pixel != 0))
            .collect(),
        },
        transform.rotation_deg == 0.0 && text.rotation_deg.unwrap_or_default() == 0.0,
    ))
}

fn r57_detector_supports_equal(
    detector: &[u8],
    scene: &[u8],
    eligible: &[u8],
    downstream: &[u8],
) -> bool {
    detector == scene && detector == eligible && detector == downstream
}

fn r59_target_id_for_rect(
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
        "R59 detector target ownership is not unique",
    )?;
    Ok(matches[0].clone())
}

fn r59_detector_diagnostics(
    environment: &SelectionEnvironment,
    result: &SelectionResult,
    schema: &VisualManifestEntry,
    oracle: &OracleValidatedEntry,
    diagnostics: &[SourceGateDiagnosticEvent],
    rejection_reason: &Option<String>,
    supports: Option<&CellSupportEvidence>,
) -> io::Result<(
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    String,
    Vec<serde_json::Value>,
    bool,
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
        .ok_or_else(|| invalid_data("R59 source-gate input diagnostic is missing"))?;
    let mut raw_detector_outputs = Vec::new();
    let mut raw_bits = Vec::<[u32; 8]>::new();
    let mut canonical_lines = Vec::new();
    let mut detector_support_records = Vec::new();
    let mut selected_target_records = Vec::new();
    let mut used_scene_supports = BTreeMap::<String, HashSet<usize>>::new();
    let mut detector_geometry_passed =
        supports.map_or(true, |supports| supports.selected_scene_rotations_zero);
    let phase = r59_phase_name(environment.phase);

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
            .ok_or_else(|| invalid_data("R59 detector crop diagnostic is missing"))?;
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
            r59_quad_bits_rect(bits)?;
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
                    "R59 SelectionGeometry detector ownership is incomplete",
                )?;
            }
            if let Some(ownership) = ownership {
                require(
                    ownership.canonical_line_index
                        == line_by_occurrence.get(&detector.occurrence_index).copied(),
                    "R59 detector canonical-line ownership drift",
                )?;
            }
            let raw_rect = r59_quad_bits_rect(fallback_scene_bits)?;
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
                        .ok_or_else(|| invalid_data("R59 target assignment index drift"))?;
                    let line_rect = r59_quad_bits_rect(geometry.scene_quad_f32_bits)?;
                    (
                        Some(r59_target_id_for_rect(schema, oracle, line_rect)?),
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
                        .ok_or_else(|| invalid_data("R59 protected assignment index drift"))?;
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
            let detector_mask = r59_rect_mask(width, height, raw_rect);
            let emitted_scene = target_id.as_ref().and_then(|target_id| {
                let candidates = supports?.scene_by_target.get(target_id)?;
                let used = used_scene_supports.entry(target_id.clone()).or_default();
                let (index, support) =
                    candidates.iter().enumerate().find(|(index, support)| {
                        !used.contains(index) && support.mask == detector_mask
                    })?;
                used.insert(index);
                Some(support)
            });
            let emitted_scene_mask = emitted_scene
                .map(|support| support.mask.clone())
                .unwrap_or_else(|| vec![0; detector_mask.len()]);
            let emitted_scene_quad = emitted_scene.map(|support| r59_rect_quad(support.rect));
            let line_rect = eligible_bits.map(r59_quad_bits_rect).transpose()?;
            let line_mask = line_rect.map_or_else(
                || vec![0; detector_mask.len()],
                |rect| r59_rect_mask(width, height, rect),
            );
            let downstream_mask = emitted_scene
                .map(|support| support.downstream_mask.clone())
                .unwrap_or_else(|| vec![0; detector_mask.len()]);
            let agreed_mask = detector_mask
                .iter()
                .zip(&emitted_scene_mask)
                .zip(&line_mask)
                .zip(&downstream_mask)
                .map(|(((detector, scene), line), downstream)| {
                    detector & scene & line & downstream
                })
                .collect::<Vec<_>>();
            let line_support_equals_detector = r57_detector_supports_equal(
                &detector_mask,
                &emitted_scene_mask,
                &line_mask,
                &downstream_mask,
            );
            if let Some(target_id) = &target_id {
                selected_target_records.push(target_id.clone());
                detector_geometry_passed &= line_support_equals_detector;
            }
            let agreed_mask_subset = agreed_mask
                .iter()
                .zip(&detector_mask)
                .zip(&emitted_scene_mask)
                .zip(&line_mask)
                .zip(&downstream_mask)
                .all(|((((agreed, detector), scene), line), downstream)| {
                    *agreed <= *detector
                        && *agreed <= *scene
                        && *agreed <= *line
                        && *agreed <= *downstream
                });
            let mut protected_mask = vec![0; detector_mask.len()];
            if let Some((_, protected, _)) = selection_geometry {
                for geometry in protected {
                    let mask = r59_rect_mask(
                        width,
                        height,
                        r59_quad_bits_rect(geometry.scene_quad_f32_bits)?,
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
                    "rect": raw_rect,
                    "recognition_present": recognition.is_some(),
                    "recognition_class": recognition_class,
                },
                "canonical_assignment": canonical_assignment,
                "emitted_scene_quad": emitted_scene_quad,
                "eligible_text_line_quad": line_rect.map(r59_rect_quad),
                "detector_support_mask": r59_mask_descriptor(width, height, &detector_mask),
                "emitted_scene_support_mask": r59_mask_descriptor(width, height, &emitted_scene_mask),
                "line_support_mask": r59_mask_descriptor(width, height, &line_mask),
                "downstream_line_support_mask": r59_mask_descriptor(width, height, &downstream_mask),
                "line_support_equals_detector": line_support_equals_detector,
                "agreed_mask": r59_mask_descriptor(width, height, &agreed_mask),
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
        "R59 detector diagnostic completeness drift",
    )?;
    selected_target_records.sort();
    let mut expected_target_records = supports
        .map(|supports| {
            supports
                .scene_by_target
                .iter()
                .flat_map(|(target, values)| std::iter::repeat_n(target.clone(), values.len()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| result.derived.selected_target_ids.clone());
    expected_target_records.sort();
    detector_geometry_passed &= selected_target_records == expected_target_records;
    Ok((
        raw_detector_outputs,
        canonical_lines,
        sha256_hex(&canonical_json(&raw_bits)?),
        detector_support_records,
        detector_geometry_passed,
    ))
}

fn write_r59_cell_diagnostic(
    environment: &SelectionEnvironment,
    process: &ProcessEvidence,
    result: &SelectionResult,
    schema: &VisualManifestEntry,
    oracle: &OracleValidatedEntry,
    diagnostics: &[SourceGateDiagnosticEvent],
    supports: &CellSupportEvidence,
    bundle_validation_receipt: Option<&PublishedArtifact>,
    coverage_index: Option<&PublishedArtifact>,
) -> io::Result<R59TerminalCellResult> {
    let phase = r59_phase_name(environment.phase);
    let revision = formal_revision(environment);
    let device = revision.map_or(process.requested_device.as_str(), |revision| {
        revision.external_device(&process.requested_device)
    });
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
        "r59/cells/{phase}/{}/{device}/{}",
        result.candidate_id, result.entry_id
    );
    let target_total = schema.targets.len();
    let selected = result.derived.selected_target_ids.len();
    let covered = result
        .derived
        .source_coverage_preflight
        .covered_source_roi_ids
        .len();
    let recall = R59TargetRecall {
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
    let device_evidence = publish_r59_artifact(
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
        "R59 inference log hash drift",
    )?;
    let log = publish_r59_artifact(
        environment,
        &format!("{cell_root}/inference.log"),
        &source_log,
    )?;
    let (
        raw_detector_outputs,
        canonical_lines,
        raw_detector_hash,
        support_records,
        detector_geometry_passed,
    ) = r59_detector_diagnostics(
        environment,
        result,
        schema,
        oracle,
        diagnostics,
        &rejection_reason,
        Some(supports),
    )?;
    let protected_overlap_pixels = protected_overlap_count(
        &supports.removal_support,
        supports.width,
        &oracle.protected_rois,
    );
    let passed = r57_cell_passed(
        result.derived.passed,
        detector_geometry_passed,
        protected_overlap_pixels,
    );
    let terminal_reason = (!passed).then(|| {
        if protected_overlap_pixels != 0 {
            "protected_overlap".into()
        } else {
            rejection_reason
                .clone()
                .unwrap_or_else(|| "coverage_failure".into())
        }
    });
    let diagnostic = serde_json::json!({
        "contract": "hanonly-r50-cell-diagnostic-v1",
        "plan_revision": revision.map_or(PLAN_REVISION, FormalRevision::plan_revision),
        "b0_sha": &environment.b0_sha,
        "calibration_manifest_sha256": &environment.calibration_manifest_sha256,
        "holdout_manifest_sha256": environment
            .formal_custody
            .as_ref()
            .and_then(|custody| custody.holdout.as_ref())
            .and_then(|holdout| holdout.runtime_commitment.get())
            .map(|runtime| runtime.manifest_sha256.as_str()),
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
        "detector_geometry_passed": detector_geometry_passed,
        "selected_scene_rotations_zero": supports.selected_scene_rotations_zero,
        "runtime_inpainter_id": &supports.runtime_inpainter_id,
        "bubble_segmenter_id": &supports.bubble_segmenter_id,
        "bubble_support_sha256": &supports.bubble_support_sha256,
        "runtime_removal_support_sha256": sha256_hex(&supports.removal_support),
        "protected_overlap_pixels": protected_overlap_pixels,
        "device_evidence_sha256": &device_evidence.sha256,
        "device_evidence_byte_length": device_evidence.byte_length,
        "log_sha256": &log.sha256,
        "log_byte_length": log.byte_length,
        "terminal_reason": &terminal_reason,
        "bundle_validation_receipt_sha256": bundle_validation_receipt.map(|value| value.sha256.as_str()),
        "target_coverage_index_sha256": coverage_index.map(|value| value.sha256.as_str()),
    });
    let diagnostic = publish_r59_artifact(
        environment,
        &format!("{cell_root}/cell-diagnostic.json"),
        &canonical_json(&diagnostic)?,
    )?;
    Ok(R59TerminalCellResult {
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
        diagnostic_path: r59_contract_path(environment, &diagnostic)?,
        diagnostic_byte_length: diagnostic.byte_length,
        target_coverage_index_path: coverage_index
            .map(|value| r59_contract_path(environment, value))
            .transpose()?,
        target_coverage_index_byte_length: coverage_index.map(|value| value.byte_length),
        device_evidence_path: r59_contract_path(environment, &device_evidence)?,
        device_evidence_byte_length: device_evidence.byte_length,
        log_path: r59_contract_path(environment, &log)?,
        log_byte_length: log.byte_length,
    })
}

fn write_r59_calibration_cell_evidence(
    environment: &SelectionEnvironment,
    process: &ProcessEvidence,
    result: &SelectionResult,
    schema: &VisualManifestEntry,
    oracle: &OracleValidatedEntry,
    diagnostics: &[SourceGateDiagnosticEvent],
    supports: &CellSupportEvidence,
) -> io::Result<R59TerminalCellResult> {
    require(
        environment.formal_custody.is_some()
            && environment.phase == Phase::CalibrationFreeze
            && result.entry_id == schema.id,
        "invalid R59 calibration cell context",
    )?;
    write_r59_cell_diagnostic(
        environment,
        process,
        result,
        schema,
        oracle,
        diagnostics,
        supports,
        None,
        None,
    )
}

fn r57_validate_source_ink_with_python(
    environment: &SelectionEnvironment,
    cell_root: &str,
    cell_key: &str,
    entry_id: &str,
    target_id: &str,
    width: u32,
    height: u32,
    oracle_mask_raw_sha256: &str,
    oracle_mask: &[u8],
    protected_rois: &[ValidatedHalfOpenRect],
    runtime_support: &[u8],
    runtime_support_sha256: &str,
    runtime_inpainter_id: &str,
    bubble_segmenter_id: &str,
    bubble_support_sha256: &str,
) -> io::Result<(PublishedArtifact, serde_json::Value)> {
    let protected = protected_rois
        .iter()
        .map(|rect| [rect.left, rect.top, rect.right, rect.bottom])
        .collect::<Vec<_>>();
    let protected_geometry_sha256 = sha256_hex(&canonical_json(&protected)?);
    let payload = serde_json::json!({
        "contract": "hanonly-r57-source-ink-validation-input-v1",
        "b0_sha": &environment.b0_sha,
        "cell_key": cell_key,
        "entry_id": entry_id,
        "target_id": target_id,
        "page_width": width,
        "page_height": height,
        "support_stride_bytes": width,
        "oracle_mask_base64": base64::engine::general_purpose::STANDARD.encode(oracle_mask),
        "oracle_mask_raw_sha256": oracle_mask_raw_sha256,
        "oracle_mask_normalized_sha256": r59_binary_mask_sha256(width, height, oracle_mask),
        "protected_rois": protected,
        "protected_geometry_sha256": protected_geometry_sha256,
        "runtime_inpainter_id": runtime_inpainter_id,
        "bubble_segmenter_id": bubble_segmenter_id,
        "bubble_support_sha256": bubble_support_sha256,
        "runtime_removal_support_base64": base64::engine::general_purpose::STANDARD.encode(runtime_support),
        "runtime_removal_support_sha256": runtime_support_sha256,
    });
    let payload = canonical_json(&payload)?;
    let script = repository_root()?.join("scripts/hanonly_evidence_ledger.py");
    let mut child = Command::new("/usr/bin/python3")
        .arg(script)
        .arg("r57-validate-source-ink")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| invalid_data("R57 validator stdin is unavailable"))?
        .write_all(&payload)?;
    let output = child.wait_with_output()?;
    require(output.status.success(), "R57 source-ink validator failed")?;
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| invalid_data("R57 source-ink validator receipt is invalid"))?;
    require(
        canonical_json(&receipt)? == output.stdout
            && receipt["contract"] == "hanonly-r57-source-ink-validation-receipt-v1"
            && receipt["b0_sha"] == environment.b0_sha
            && receipt["cell_key"] == cell_key
            && receipt["entry_id"] == entry_id
            && receipt["target_id"] == target_id
            && receipt["protected_geometry_sha256"] == protected_geometry_sha256
            && receipt["runtime_inpainter_id"] == runtime_inpainter_id
            && receipt["bubble_segmenter_id"] == bubble_segmenter_id
            && receipt["bubble_support_sha256"] == bubble_support_sha256
            && receipt["runtime_removal_support_sha256"] == runtime_support_sha256,
        "R57 source-ink validator receipt binding drift",
    )?;
    let artifact = publish_r59_artifact(
        environment,
        &format!("{cell_root}/{target_id}.source-ink-validation.json"),
        &output.stdout,
    )?;
    Ok((artifact, receipt))
}

fn write_r59_cell_evidence(
    environment: &SelectionEnvironment,
    process: &ProcessEvidence,
    result: &mut SelectionResult,
    schema: &VisualManifestEntry,
    oracle: &OracleValidatedEntry,
    diagnostics: &[SourceGateDiagnosticEvent],
    supports: &CellSupportEvidence,
    bundle_validation_receipt: &PublishedArtifact,
) -> io::Result<R59TerminalCellResult> {
    require(
        environment.formal_custody.is_some()
            && environment.phase == Phase::Holdout
            && result.entry_id == schema.id,
        "invalid R59 formal cell context",
    )?;
    let custody = environment
        .formal_custody
        .as_ref()
        .and_then(|custody| custody.holdout.as_ref())
        .and_then(|holdout| holdout.runtime_commitment.get())
        .ok_or_else(|| invalid_data("R59 runtime commitment is unavailable"))?;
    let revision = formal_revision(environment);
    let device = revision
        .expect("formal custody")
        .external_device(&process.requested_device);
    let cell_key = format!(
        "holdout/{}/{device}/{}",
        result.candidate_id, result.entry_id
    );
    let cell_root = format!(
        "r59/cells/holdout/{}/{device}/{}",
        result.candidate_id, result.entry_id
    );
    let target_total = schema.targets.len();
    let selected = result.derived.selected_target_ids.len();
    let covered = result
        .derived
        .source_coverage_preflight
        .covered_source_roi_ids
        .len();
    let recall = R59TargetRecall {
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
        "contract": "hanonly-r59-cell-capture-v1",
        "plan_revision": revision.map_or(PLAN_REVISION, FormalRevision::plan_revision),
        "b0_sha": &environment.b0_sha,
        "cell_key": &cell_key,
        "manifest_sha256": &custody.manifest_sha256,
        "selection_result": &result,
        "target_recall": &recall,
        "pp_han_count": result.derived.source_coverage_preflight.pp_han_scalar_count,
        "vl_han_count": result.derived.source_coverage_preflight.vl_expected_han_scalar_count,
        "rejection_reason": &rejection_reason,
        "log_path": &result.execution_evidence.raw_inference_log_relpath,
        "log_sha256": &result.execution_evidence.raw_inference_log_sha256,
        "source_gate_diagnostics": diagnostics,
    });
    let captured = publish_r59_artifact(
        environment,
        &format!("{cell_root}/selection-result.json"),
        &canonical_json(&captured)?,
    )?;

    let mut proof_records = Vec::with_capacity(schema.targets.len());
    let mut coverage_passed = true;
    let page_len = usize::try_from(u64::from(supports.width) * u64::from(supports.height))
        .map_err(|_| invalid_data("R59 support raster length overflow"))?;
    for (target, oracle_target) in schema.targets.iter().zip(&oracle.targets) {
        let runtime_removal_support = supports.removal_support.clone();
        require(
            runtime_removal_support.len() == page_len
                && runtime_removal_support
                    .iter()
                    .all(|pixel| matches!(pixel, 0 | 1)),
            "R59 support raster is not complete binary page geometry",
        )?;
        let runtime_removal_raster = publish_r59_artifact(
            environment,
            &format!("{cell_root}/{}.runtime-removal-support.bin", target.id),
            &runtime_removal_support,
        )?;
        let oracle_mask = page_oracle_mask(
            supports.width,
            supports.height,
            oracle_target.edit_roi,
            &oracle_target.delta_mask,
        )?;
        let (spatial_receipt, spatial_validation) = r57_validate_source_ink_with_python(
            environment,
            &cell_root,
            &cell_key,
            &result.entry_id,
            &target.id,
            supports.width,
            supports.height,
            &target.erase_source_ink_mask_sha256,
            &oracle_mask,
            &oracle.protected_rois,
            &runtime_removal_support,
            &runtime_removal_raster.sha256,
            &supports.runtime_inpainter_id,
            &supports.bubble_segmenter_id,
            &supports.bubble_support_sha256,
        )?;
        let oracle_foreground_pixels = foreground_count(&oracle_mask);
        let runtime_removal_covered_pixels =
            intersection_count(&oracle_mask, &runtime_removal_support)?;
        let protected_overlap_pixels = protected_overlap_count(
            &runtime_removal_support,
            supports.width,
            &oracle.protected_rois,
        );
        let missing_runtime_removal_pixels =
            oracle_foreground_pixels.saturating_sub(runtime_removal_covered_pixels);
        let target_selected = result
            .derived
            .selected_target_ids
            .iter()
            .any(|id| id == &target.id);
        let python_passed = spatial_validation["result"] == "pass"
            && spatial_validation["oracle_foreground_pixels"] == oracle_foreground_pixels
            && spatial_validation["runtime_removal_covered_pixels"]
                == runtime_removal_covered_pixels
            && spatial_validation["missing_runtime_removal_pixels"]
                == missing_runtime_removal_pixels
            && spatial_validation["protected_overlap_pixels"] == protected_overlap_pixels;
        let passed = python_passed
            && target_selected
            && missing_runtime_removal_pixels == 0
            && protected_overlap_pixels == 0;
        coverage_passed &= passed;
        let proof = R57SourceInkCoverageProof {
            contract: "hanonly-r57-source-ink-coverage-proof-v1",
            plan_revision: revision.map_or(PLAN_REVISION, FormalRevision::plan_revision),
            b0_sha: &environment.b0_sha,
            cell_key: &cell_key,
            entry_id: &result.entry_id,
            target_id: &target.id,
            oracle_mask_raw_sha256: target.erase_source_ink_mask_sha256.clone(),
            oracle_mask_normalized_sha256: r59_binary_mask_sha256(
                supports.width,
                supports.height,
                &oracle_mask,
            ),
            page_width: supports.width,
            page_height: supports.height,
            support_stride_bytes: supports.width,
            runtime_removal_support_relpath: r59_contract_path(
                environment,
                &runtime_removal_raster,
            )?,
            runtime_removal_support_byte_length: runtime_removal_raster.byte_length,
            runtime_removal_support_sha256: runtime_removal_raster.sha256,
            spatial_validation_receipt_relpath: r59_contract_path(
                environment,
                &spatial_receipt,
            )?,
            spatial_validation_receipt_byte_length: spatial_receipt.byte_length,
            spatial_validation_receipt_sha256: spatial_receipt.sha256,
            protected_geometry_sha256: spatial_validation["protected_geometry_sha256"]
                .as_str()
                .ok_or_else(|| invalid_data("R57 protected geometry hash is missing"))?
                .to_owned(),
            runtime_inpainter_id: &supports.runtime_inpainter_id,
            bubble_segmenter_id: &supports.bubble_segmenter_id,
            bubble_support_sha256: &supports.bubble_support_sha256,
            oracle_foreground_pixels,
            runtime_removal_support_foreground_pixels: foreground_count(
                &runtime_removal_support,
            ),
            runtime_removal_covered_pixels,
            missing_runtime_removal_pixels,
            protected_overlap_pixels,
            target_selected,
            result: if passed { "pass" } else { "fail-closed" },
        };
        let proof = publish_r59_artifact(
            environment,
            &format!("{cell_root}/{}.coverage-proof.json", target.id),
            &canonical_json(&proof)?,
        )?;
        proof_records.push(R59TargetCoverageIndexRecord {
            entry_id: result.entry_id.clone(),
            target_id: target.id.clone(),
            proof_path: r59_contract_path(environment, &proof)?,
            proof_sha256: proof.sha256,
            proof_byte_length: proof.byte_length,
        });
    }
    proof_records.sort_by(|left, right| {
        (&left.entry_id, &left.target_id).cmp(&(&right.entry_id, &right.target_id))
    });
    let coverage_index = R59TargetCoverageIndex {
        contract: "hanonly-r57-source-ink-coverage-index-v1",
        plan_revision: revision.map_or(PLAN_REVISION, FormalRevision::plan_revision),
        b0_sha: &environment.b0_sha,
        cell_key: &cell_key,
        manifest_sha256: &custody.manifest_sha256,
        oracle_sha256: &custody.oracle_sha256,
        hashes_sha256: &custody.hashes_sha256,
        records: proof_records,
    };
    let coverage_index = publish_r59_artifact(
        environment,
        &format!("{cell_root}/target-coverage-index.json"),
        &canonical_json(&coverage_index)?,
    )?;
    result.derived.passed &= coverage_passed;
    let _ = (captured, recall, selection_result, rejection_reason);
    write_r59_cell_diagnostic(
        environment,
        process,
        result,
        schema,
        oracle,
        diagnostics,
        supports,
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
        .map_err(|_| invalid_data("R59 oracle width overflow"))?;
    let roi_height = usize::try_from(roi.bottom - roi.top)
        .map_err(|_| invalid_data("R59 oracle height overflow"))?;
    require(
        local.len() == roi_width * roi_height,
        "R59 oracle mask length drift",
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
    require(left.len() == right.len(), "R59 mask dimensions drift")?;
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

fn r59_binary_mask_sha256(width: u32, height: u32, mask: &[u8]) -> String {
    let mut preimage = b"hanonly-r59-binary-mask-v1\0".to_vec();
    preimage.extend_from_slice(&width.to_be_bytes());
    preimage.extend_from_slice(&height.to_be_bytes());
    preimage.extend_from_slice(mask);
    sha256_hex(&preimage)
}

fn publish_r59_artifact(
    environment: &SelectionEnvironment,
    suffix: &str,
    bytes: &[u8],
) -> io::Result<PublishedArtifact> {
    let suffix = formal_artifact_suffix(formal_revision(environment), suffix);
    require(
        !bytes.is_empty()
            && !suffix.starts_with('/')
            && suffix
                .split('/')
                .all(|component| !matches!(component, "" | "." | "..")),
        "invalid R59 artifact",
    )?;
    let report_relative = environment
        .report_dir
        .strip_prefix(&environment.evidence_root)
        .map_err(|_| invalid_data("R59 report directory escaped evidence root"))?;
    let suffix_path = Path::new(&suffix);
    let parent_relative = report_relative.join(
        suffix_path
            .parent()
            .ok_or_else(|| invalid_data("R59 artifact has no parent"))?,
    );
    let file_name = suffix_path
        .file_name()
        .ok_or_else(|| invalid_data("R59 artifact name is invalid"))?;
    let published = publish_descriptor_relative(
        &environment.evidence_root,
        &parent_relative,
        file_name,
        bytes,
    )?;
    let path = environment.report_dir.join(&suffix);
    let sha256 = published.sha256;
    let artifact_parent = environment
        .artifact
        .parent()
        .ok_or_else(|| invalid_data("selection artifact has no parent"))?;
    let relative = path
        .strip_prefix(artifact_parent)
        .map_err(|_| invalid_data("R59 artifact is outside artifact parent"))?
        .to_str()
        .ok_or_else(|| invalid_data("R59 artifact path is not utf-8"))?
        .to_owned();
    Ok(PublishedArtifact {
        path: relative,
        sha256,
        byte_length: bytes.len() as u64,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct R59DescriptorMetadata {
    dev: u64,
    ino: u64,
    owner: u64,
    mode: u32,
    file_type: FileType,
}

struct R59HeldDirectory {
    slash: OwnedFd,
    descriptor: OwnedFd,
    absolute_components: Vec<OsString>,
    chain: Vec<R59DescriptorMetadata>,
}

struct R59PublishedDescriptor {
    sha256: String,
}

impl R59HeldDirectory {
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
            r59_walk_directories(slash.as_fd(), &absolute_components, false, false)?;
        let root = chain
            .last()
            .ok_or_else(|| invalid_data("R59 evidence root is unavailable"))?;
        require(
            root.file_type.is_dir()
                && root.owner == effective_owner()?
                && root.mode & 0o7777 == 0o700,
            "invalid R59 evidence root",
        )?;
        Ok(Self {
            slash,
            descriptor,
            absolute_components,
            chain,
        })
    }

    fn open_or_create_child(&self, relative: &Path) -> io::Result<R59HeldDirectoryChild> {
        let components = relative
            .components()
            .map(|component| component.as_os_str().to_owned())
            .collect::<Vec<_>>();
        let (descriptor, chain) =
            r59_walk_directories(self.descriptor.as_fd(), &components, true, true)?;
        fsync(&self.descriptor).map_err(io::Error::from)?;
        Ok(R59HeldDirectoryChild {
            descriptor,
            components,
            chain,
        })
    }

    fn revalidate_descriptor(&self) -> io::Result<OwnedFd> {
        let (fresh, chain) =
            r59_walk_directories(self.slash.as_fd(), &self.absolute_components, false, false)?;
        require(chain == self.chain, "R59 evidence root namespace changed")?;
        Ok(fresh)
    }

    fn revalidate_child(&self, child: &R59HeldDirectoryChild) -> io::Result<OwnedFd> {
        self.revalidate_descriptor()?;
        let (fresh, chain) =
            r59_walk_directories(self.descriptor.as_fd(), &child.components, false, true)?;
        require(chain == child.chain, "R59 publication namespace changed")?;
        Ok(fresh)
    }
}

struct R59HeldDirectoryChild {
    descriptor: OwnedFd,
    components: Vec<OsString>,
    chain: Vec<R59DescriptorMetadata>,
}

fn r59_walk_directories(
    start: BorrowedFd<'_>,
    components: &[OsString],
    create: bool,
    require_secure: bool,
) -> io::Result<(OwnedFd, Vec<R59DescriptorMetadata>)> {
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
            "invalid R59 directory component",
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
        let metadata = r59_descriptor_metadata(next.as_fd())?;
        require(
            metadata.file_type.is_dir(),
            "R59 path component is not a directory",
        )?;
        if require_secure {
            require(
                metadata.owner == effective_owner()?,
                "R59 publication directory owner mismatch",
            )?;
            require(
                metadata.mode & 0o7777 == 0o700,
                "R59 publication directory mode mismatch",
            )?;
        }
        chain.push(metadata);
        current = next;
    }
    Ok((current, chain))
}

fn r59_descriptor_metadata(fd: BorrowedFd<'_>) -> io::Result<R59DescriptorMetadata> {
    let stat = fstat(fd).map_err(io::Error::from)?;
    Ok(R59DescriptorMetadata {
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
) -> io::Result<R59PublishedDescriptor> {
    require(!bytes.is_empty(), "R59 publication bytes are empty")?;
    let root = R59HeldDirectory::open(root)?;
    let parent = root.open_or_create_child(parent_relative)?;
    let sha256 = sha256_hex(bytes);
    let temporary = OsString::from(format!(".{}.{}.tmp", final_name.to_string_lossy(), sha256));
    r59_require_absent(parent.descriptor.as_fd(), final_name)?;
    r59_require_absent(parent.descriptor.as_fd(), &temporary)?;
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
        let temporary_metadata = r59_descriptor_metadata(temporary_file.as_fd())?;
        let final_metadata = r59_descriptor_metadata(final_descriptor.as_fd())?;
        let mut final_file = fs::File::from(final_descriptor);
        let mut actual = Vec::new();
        final_file.read_to_end(&mut actual)?;
        require(
            temporary_metadata == final_metadata
                && final_metadata.file_type.is_file()
                && final_metadata.owner == effective_owner()?
                && final_metadata.mode & 0o7777 == 0o600
                && actual == bytes,
            "R59 artifact publication verification failed",
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
            r59_descriptor_metadata(final_file.as_fd())?
                == R59DescriptorMetadata {
                    dev: named.st_dev as u64,
                    ino: named.st_ino,
                    owner: named.st_uid.into(),
                    mode: named.st_mode.into(),
                    file_type: FileType::from_raw_mode(named.st_mode),
                },
            "R59 artifact final namespace changed",
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
    Ok(R59PublishedDescriptor { sha256 })
}

fn r59_require_absent(parent: BorrowedFd<'_>, name: &OsStr) -> io::Result<()> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err(invalid_data("R59 create-new publication collision")),
        Err(error) => Err(error.into()),
    }
}

fn r59_diagnostic_record(cell: &R59TerminalCellResult, state: &str) -> serde_json::Value {
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

fn write_r59_cell_transitions(
    environment: &SelectionEnvironment,
    cells: &[R59TerminalCellResult],
    mut generation: u64,
    mut previous: Option<PublishedArtifact>,
    records: &mut Vec<serde_json::Value>,
    calibration_manifest_sha256: &str,
    holdout_manifest_sha256: Option<&str>,
    bundle_validation_receipt: Option<&PublishedArtifact>,
) -> io::Result<PublishedArtifact> {
    for cell in cells {
        generation += 1;
        records.push(r59_diagnostic_record(cell, "captured_unclassified"));
        records
            .sort_by(|left, right| left["cell_key"].as_str().cmp(&right["cell_key"].as_str()));
        previous = Some(write_r59_diagnostic_generation(
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
            .ok_or_else(|| invalid_data("R59 diagnostic record is missing"))?;
        *terminal = r59_diagnostic_record(
            cell,
            if cell.result == "pass" {
                "passed"
            } else {
                "failed"
            },
        );
        previous = Some(write_r59_diagnostic_generation(
            environment,
            generation,
            previous.as_ref(),
            calibration_manifest_sha256,
            holdout_manifest_sha256,
            bundle_validation_receipt,
            records,
        )?);
    }
    previous.ok_or_else(|| invalid_data("R59 diagnostic chain is empty"))
}

fn write_r59_calibration_diagnostic_generations(
    environment: &SelectionEnvironment,
    formal: &R59FormalRunEvidence,
) -> io::Result<PublishedArtifact> {
    require(
        environment.phase == Phase::CalibrationFreeze
            && formal.bundle_validation_receipt.is_none()
            && formal.first_failed_cell.is_none()
            && formal.cells.len() == 32,
        "R59 calibration diagnostic matrix drift",
    )?;
    let expected = candidates_schema()
        .into_iter()
        .flat_map(|candidate| {
            ["cpu", "metal"].into_iter().flat_map(move |device| {
                environment.calibration_entry_ids.clone().into_iter().map({
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
        "R59 calibration diagnostic identities drift",
    )?;
    let mut cells = formal.cells.clone();
    cells.sort_by(|left, right| left.diagnostic_cell_key.cmp(&right.diagnostic_cell_key));
    write_r59_cell_transitions(
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

fn write_r59_diagnostic_generations(
    environment: &SelectionEnvironment,
    selected_candidate_id: &str,
    calibration_manifest_sha256: &str,
    formal: &R59FormalRunEvidence,
) -> io::Result<PublishedArtifact> {
    let revision = formal_revision(environment)
        .ok_or_else(|| invalid_data("formal revision is unavailable"))?;
    let expected = validate_formal_terminal_closure(revision, selected_candidate_id, formal)?;
    let bundle = formal
        .bundle_validation_receipt
        .as_ref()
        .ok_or_else(|| invalid_data("R59 bundle validation receipt is missing"))?;
    let custody = environment
        .formal_custody
        .as_ref()
        .ok_or_else(|| invalid_data("R59 formal custody is not enabled"))?;
    let holdout = custody
        .holdout
        .as_ref()
        .ok_or_else(|| invalid_data("R59 holdout custody is unavailable"))?;
    let open_marker = holdout
        .open_marker
        .get()
        .ok_or_else(|| invalid_data("R59 runner open marker was not validated"))?;
    let runtime = holdout
        .runtime_commitment
        .get()
        .ok_or_else(|| invalid_data("R59 runtime commitment was not validated"))?;
    let (previous, mut records) = (
        write_r59_diagnostic_generation(
            environment,
            0,
            None,
            calibration_manifest_sha256,
            Some(&runtime.manifest_sha256),
            Some(bundle),
            &[],
        )?,
        Vec::new(),
    );
    let (starting_generation, complete_generation) = revision.diagnostic_generation_bounds();
    let terminal_generation = starting_generation + formal.cells.len() as u64 * 2;
    let terminal_generation_artifact = write_r59_cell_transitions(
        environment,
        &formal.cells,
        starting_generation,
        Some(previous),
        &mut records,
        calibration_manifest_sha256,
        Some(&runtime.manifest_sha256),
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
        publish_r59_artifact(environment, "r59/diagnostic-index.json", &generation_bytes)?;
    require(
        records.len() == formal.cells.len()
            && terminal_generation == starting_generation + formal.cells.len() as u64 * 2
            && (formal.cells.len() != expected.len()
                || terminal_generation == complete_generation),
        "R59 terminal diagnostic count drift",
    )?;
    let unexecuted_cell_keys = expected[formal.cells.len()..].to_vec();
    let all_cells_passed =
        formal.cells.len() == expected.len() && formal.first_failed_cell.is_none();
    let (authorization_state, result) = pre_cleanup_completion_state(all_cells_passed);
    let bundle_path = r59_contract_path(environment, bundle)?;
    let terminal_index_path = r59_contract_path(environment, &terminal_index)?;
    let completion_summary = R59CompletionSummary {
        contract: revision.completion_summary_contract(),
        plan_revision: revision.plan_revision(),
        b0_sha: &environment.b0_sha,
        selected_candidate_id,
        original_public_commitment_sha256: &holdout.freeze.original_public_commitment_sha256,
        successor_commitment_sha256: &holdout.freeze.receipt_sha256,
        successor_b0_sha: &holdout.freeze.successor_b0_sha,
        start_marker_sha256: &open_marker.sha256,
        ciphertext_sha256: &holdout.freeze.ciphertext_sha256,
        private_manifest_commitment_sha256: &holdout.freeze.private_manifest_commitment_sha256,
        runtime_commitment_receipt_sha256: &holdout
            .runtime_commitment
            .get()
            .ok_or_else(|| invalid_data("R59 runtime commitment is unavailable"))?
            .receipt
            .sha256,
        pre_holdout_attestation_sha256: &environment.required_check.attestation_sha256,
        holdout_manifest_sha256: &holdout
            .runtime_commitment
            .get()
            .ok_or_else(|| invalid_data("R59 runtime commitment is unavailable"))?
            .manifest_sha256,
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
        authorization_state,
        result,
    };
    let summary = publish_r59_artifact(
        environment,
        "r59/completion-summary.json",
        &canonical_json(&completion_summary)?,
    )?;
    println!(
        "{}",
        formal_completion_summary_stdout_line(revision, &summary)?
    );
    Ok(terminal_index)
}

fn validate_formal_terminal_closure(
    revision: FormalRevision,
    selected_candidate_id: &str,
    formal: &R59FormalRunEvidence,
) -> io::Result<Vec<String>> {
    let expected = revision.formal_cell_keys();
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
        "R59 formal cells are not an exact ordered prefix",
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
        "R59 formal first-failure boundary drift",
    )?;
    Ok(expected)
}

fn pre_cleanup_completion_state(all_cells_passed: bool) -> (&'static str, &'static str) {
    (
        "incomplete_non_authorizing",
        if all_cells_passed {
            "terminal_pass_cleanup_pending"
        } else {
            "completed_fail"
        },
    )
}

fn formal_completion_summary_stdout_line(
    revision: FormalRevision,
    summary: &PublishedArtifact,
) -> io::Result<String> {
    let binding = String::from_utf8(canonical_json(summary)?)
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    Ok(format!(
        "{}{binding}",
        revision.completion_summary_stdout_prefix()
    ))
}

fn write_r59_diagnostic_generation(
    environment: &SelectionEnvironment,
    generation: u64,
    previous: Option<&PublishedArtifact>,
    calibration_manifest_sha256: &str,
    holdout_manifest_sha256: Option<&str>,
    bundle_validation_receipt: Option<&PublishedArtifact>,
    records: &[serde_json::Value],
) -> io::Result<PublishedArtifact> {
    let bundle_path = bundle_validation_receipt
        .map(|value| r59_contract_path(environment, value))
        .transpose()?;
    let index = serde_json::json!({
        "contract": "hanonly-r50-diagnostic-index-v1",
        "plan_revision": formal_revision(environment).map_or(PLAN_REVISION, FormalRevision::plan_revision),
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
    publish_r59_artifact(
        environment,
        &format!("r59/diagnostic-index.generations/{generation:08}.json"),
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
        .map(|index| format!("r59-{prefix}{index:02}"))
        .collect()
}

fn hold_calibration_artifact(
    environment: &SelectionEnvironment,
    input: &HeldInput,
    artifact: &FrozenArtifact,
) -> io::Result<()> {
    if let Some(holdout) = environment
        .formal_custody
        .as_ref()
        .and_then(|custody| custody.holdout.as_ref())
    {
        let artifact_sha256 = hex_sha256(input.sha256());
        require(
            holdout.freeze.accepts_calibration_artifact(
                &artifact.b0_sha,
                &environment.b0_sha,
                &artifact_sha256,
            ) && artifact.manifest_sha256 == environment.calibration_manifest_sha256,
            "R59 frozen calibration artifact binding drift",
        )?;
        environment
            .held_calibration_artifact_sha256
            .set(artifact_sha256)
            .map_err(|_| invalid_data("R59 calibration artifact already held"))?;
    }
    environment
        .frozen_candidate_id
        .set(artifact.selected_candidate_id.clone())
        .map_err(|_| invalid_data("holdout candidate already held"))
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
    let b0_binding_valid = if artifact.b0_sha == environment.b0_sha {
        true
    } else if let Some(holdout) = environment
        .formal_custody
        .as_ref()
        .and_then(|custody| custody.holdout.as_ref())
    {
        environment
            .held_calibration_artifact_sha256
            .get()
            .is_some_and(|artifact_sha256| {
                holdout.freeze.accepts_calibration_artifact(
                    &artifact.b0_sha,
                    &environment.b0_sha,
                    artifact_sha256,
                )
            })
    } else {
        false
    };
    require(
        b0_binding_valid
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
                .formal_custody
                .as_ref()
                .and_then(|custody| custody.holdout.as_ref())
                .and_then(|holdout| holdout.runtime_commitment.get())
                .map_or(environment.visual_manifest_sha256.as_str(), |runtime| {
                    runtime.manifest_sha256.as_str()
                });
            let expected_entry_ids = formal_revision(environment)
                .map_or_else(|| r59_entry_ids('h'), FormalRevision::entry_ids);
            artifact.holdout_manifest_sha256.as_deref() == Some(expected_manifest)
                && artifact.holdout_entry_ids == expected_entry_ids
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
    let frozen_holdout_entry_ids = if phase == Phase::Holdout
        && formal_revision(environment) == Some(FormalRevision::R60)
    {
        r59_entry_ids('h')
    } else {
        artifact.holdout_entry_ids.clone()
    };
    require(
        frozen_projection_sha256_with_holdout_ids(artifact, &frozen_holdout_entry_ids)?
            == artifact.frozen_payload_sha256,
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
        let expected_b0_sha = match expected_phase {
            Phase::CalibrationFreeze => &artifact.b0_sha,
            Phase::Holdout => &environment.b0_sha,
        };
        let path = environment.evidence_root.join(&stored.attestation_relpath);
        let (current, held) = load_required_check(
            &environment.evidence_root,
            &path,
            expected_phase,
            expected_b0_sha,
            &stored.manifest_sha256,
            &artifact.source_gate_fixture_manifest_sha256,
        )?;
        require(&current == stored, "required-check artifact entry drift")?;
        held.with_revalidated_path(|_| Ok(()))?;
    }
    Ok(())
}

fn frozen_projection_sha256(artifact: &FrozenArtifact) -> io::Result<String> {
    frozen_projection_sha256_with_holdout_ids(artifact, &artifact.holdout_entry_ids)
}

fn frozen_projection_sha256_with_holdout_ids(
    artifact: &FrozenArtifact,
    holdout_entry_ids: &[String],
) -> io::Result<String> {
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
        "holdout_entry_ids": holdout_entry_ids,
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
fn source_ink_coverage_requires_every_ink_pixel() {
    let edit_roi = ValidatedHalfOpenRect {
        left: 2,
        top: 2,
        right: 5,
        bottom: 5,
    };
    let source_ink = [0, 1, 0, 0, 0, 0, 0, 1, 0];
    let mask = SourceInkMask::edit_roi(&source_ink, edit_roi);
    let mut support = [0; 64];

    support[2 * 8 + 3] = 1;
    support[4 * 8 + 3] = 1;
    assert!(source_ink_is_covered(8, edit_roi, mask, &support));

    support.fill(0);
    assert!(!source_ink_is_covered(8, edit_roi, mask, &support));

    support[2 * 8 + 3] = 1;
    assert!(!source_ink_is_covered(8, edit_roi, mask, &support));

    let mut page_source_ink = [0; 64];
    page_source_ink[2 * 8 + 3] = 1;
    page_source_ink[4 * 8 + 3] = 1;
    support[4 * 8 + 3] = 1;
    assert!(source_ink_is_covered(
        8,
        edit_roi,
        SourceInkMask::page(&page_source_ink, 8, 8),
        &support,
    ));
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
                result.entry_id == "r59-c01"
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
            "{}: r59-c01/cpu recall=0.000 protected=0 unmatched=0 rotation_excluded=true",
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
            {"id": "r59-c01", "role": "calibration"},
            {"id": "r59-c02", "role": "calibration"},
            {"id": "r59-c03", "role": "calibration"},
            {"id": "r59-c04", "role": "calibration"},
            {"id": "r59-h01", "role": "holdout"},
            {"id": "r59-h02", "role": "holdout"},
            {"id": "r59-h03", "role": "holdout"},
            {"id": "r59-h04", "role": "holdout"}
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

fn parse_test_environment(
    values: &HashMap<&'static str, String>,
) -> io::Result<SelectionEnvironment> {
    SelectionEnvironment::parse_with_formal_paths(
        |name| values.get(name).cloned(),
        Some(FormalPublicPaths::frozen()),
    )
}

#[test]
fn formal_protocol_selection_retires_historical_custody_before_access() {
    assert_eq!(select_formal_revision(None, None).unwrap(), None);
    assert_eq!(select_formal_revision(None, Some("0")).unwrap(), None);
    assert_eq!(select_formal_revision(Some("0"), None).unwrap(), None);
    assert_eq!(
        select_formal_revision(None, Some("1"))
            .unwrap_err()
            .to_string(),
        HISTORICAL_CUSTODY_COMMAND_RETIRED
    );
    assert_eq!(
        select_formal_revision(Some("1"), None)
            .unwrap_err()
            .to_string(),
        HISTORICAL_CUSTODY_COMMAND_RETIRED
    );
    assert!(select_formal_revision(None, Some("true")).is_err());
}

#[test]
fn r52_bridge_retires_before_request_access() {
    assert_eq!(
        run_r52_evidence_bridge().unwrap_err().to_string(),
        HISTORICAL_CUSTODY_COMMAND_RETIRED
    );
}

#[test]
fn formal_protocol_environment_retires_before_model_access() {
    let root = tempfile::tempdir().unwrap();
    let mut values = valid_environment(root.path());
    values.insert(R60_FORMAL_CUSTODY_ENV, "1".into());

    let error = parse_test_environment(&values).err().expect("must reject");
    assert_eq!(error.to_string(), HISTORICAL_CUSTODY_COMMAND_RETIRED);
}

#[test]
fn r60_receipts_are_strict_and_dispatch_to_the_r60_bundle_validator() {
    let layout = R60LayoutReceipt {
        schema: "hanonly.r60.layout-receipt.v1".into(),
        plan_revision: 60,
        manifest_sha256: synthetic_hash(4),
        private_manifest_commitment_sha256: synthetic_hash(4),
        member_name_digest_sha256: synthetic_hash(5),
        ciphertext_sha256: synthetic_hash(1),
        layout_validator_sha256: synthetic_hash(3),
        entry_ids: FormalRevision::R60.entry_ids(),
        required_root_present: true,
        wrapper_absent: true,
        canonical_ustar_pass: true,
        manifest_binding_pass: true,
        same_archive_object_pass: true,
        layout_pass: true,
        restricted_values_disclosed: false,
    };
    let layout_bytes = canonical_json(&layout).unwrap();
    let layout_sha256 = sha256_hex(&layout_bytes);
    let public = R60PublicCommitment {
        schema: "hanonly.r60.public-commitment.v1".into(),
        plan_revision: 60,
        source_b0_sha: R60_SOURCE_B0_SHA.into(),
        ciphertext_sha256: synthetic_hash(1),
        layout_receipt_sha256: layout_sha256.clone(),
        layout_validator_sha256: synthetic_hash(3),
        manifest_sha256: synthetic_hash(4),
        member_name_digest_sha256: synthetic_hash(5),
        private_manifest_commitment_sha256: synthetic_hash(4),
        entry_ids: FormalRevision::R60.entry_ids(),
        cleanup_pass: true,
        restricted_values_disclosed: false,
        start_marker_absent: true,
    };
    let public_bytes = canonical_json(&public).unwrap();
    let public_sha256 = sha256_hex(&public_bytes);
    let successor = R60SuccessorCommitment {
        schema: "hanonly.r60.successor-commitment.v1".into(),
        plan_revision: 60,
        public_commitment_sha256: public_sha256.clone(),
        source_b0_sha: R60_SOURCE_B0_SHA.into(),
        successor_b0_sha: "b".repeat(40),
        contract_sha256: R60_CONTRACT_SHA256.into(),
        test_spec_sha256: R60_TEST_SPEC_SHA256.into(),
        calibration_artifact_sha256: R59_CALIBRATION_ARTIFACT_SHA256.into(),
        selected_candidate_id: "S25L4".into(),
        ciphertext_sha256: synthetic_hash(1),
        layout_receipt_sha256: layout_sha256.clone(),
        layout_validator_sha256: synthetic_hash(3),
        manifest_sha256: synthetic_hash(4),
        member_name_digest_sha256: synthetic_hash(5),
        private_manifest_commitment_sha256: synthetic_hash(4),
        entry_ids: FormalRevision::R60.entry_ids(),
        package_unchanged: true,
        start_marker_absent: true,
    };
    let successor_bytes = canonical_json(&successor).unwrap();
    let freeze = validate_r60_successor_commitments(
        &layout_bytes,
        &public_bytes,
        &public_sha256,
        &successor_bytes,
        &sha256_hex(&successor_bytes),
        &successor.successor_b0_sha,
        R60_CONTRACT_SHA256,
        R60_TEST_SPEC_SHA256,
    )
    .unwrap();

    let start = R60OpenMarker {
        schema: "hanonly.r60.holdout-start.v1".into(),
        plan_revision: 60,
        b0_sha: successor.successor_b0_sha.clone(),
        public_commitment_sha256: public_sha256,
        successor_commitment_sha256: freeze.receipt_sha256.clone(),
        calibration_artifact_sha256: R59_CALIBRATION_ARTIFACT_SHA256.into(),
        selected_candidate_id: "S25L4".into(),
        entry_ids: FormalRevision::R60.entry_ids(),
        pre_holdout_attestation_sha256: synthetic_hash(6),
        nonce_hex: synthetic_hash(7),
        state: "started".into(),
    };
    let start_bytes = canonical_json(&start).unwrap();
    validate_r60_start_receipt(
        &start_bytes,
        &successor.successor_b0_sha,
        "S25L4",
        &freeze,
        &start.pre_holdout_attestation_sha256,
    )
    .unwrap();

    let runtime = R60RuntimeCommitment {
        schema: "hanonly.r60.runtime-commitment.v1".into(),
        plan_revision: 60,
        b0_sha: successor.successor_b0_sha.clone(),
        start_marker_sha256: sha256_hex(&start_bytes),
        successor_commitment_sha256: freeze.receipt_sha256.clone(),
        ciphertext_sha256: synthetic_hash(1),
        layout_receipt_sha256: layout_sha256,
        layout_validator_sha256: synthetic_hash(3),
        member_name_digest_sha256: synthetic_hash(5),
        private_manifest_commitment_sha256: synthetic_hash(4),
        calibration_artifact_sha256: R59_CALIBRATION_ARTIFACT_SHA256.into(),
        selected_candidate_id: "S25L4".into(),
        plaintext_archive_sha256: synthetic_hash(8),
        manifest_sha256: synthetic_hash(4),
        oracle_sha256: synthetic_hash(9),
        hashes_sha256: synthetic_hash(10),
        entry_ids: FormalRevision::R60.entry_ids(),
        decrypt_pass: true,
        package_unchanged: true,
        restricted_values_disclosed: false,
        state: "runtime_committed".into(),
    };
    let runtime_bytes = canonical_json(&runtime).unwrap();
    let validated = validate_r60_runtime_receipt(
        &runtime_bytes,
        &successor.successor_b0_sha,
        &runtime.start_marker_sha256,
        &freeze,
    )
    .unwrap();
    assert_eq!(validated.manifest_sha256, runtime.manifest_sha256);
    assert_eq!(validated.oracle_sha256, runtime.oracle_sha256);
    assert_eq!(validated.hashes_sha256, runtime.hashes_sha256);

    let mut invalid_archive_hash: serde_json::Value =
        serde_json::from_slice(&runtime_bytes).unwrap();
    invalid_archive_hash["plaintext_archive_sha256"] = serde_json::json!("not-a-sha");
    assert!(
        validate_r60_runtime_receipt(
            &canonical_json(&invalid_archive_hash).unwrap(),
            &successor.successor_b0_sha,
            &runtime.start_marker_sha256,
            &freeze,
        )
        .is_err()
    );

    let mut open_runtime: serde_json::Value = serde_json::from_slice(&runtime_bytes).unwrap();
    open_runtime["runtime_manifest_sha256"] = serde_json::json!(synthetic_hash(12));
    assert!(
        validate_r60_runtime_receipt(
            &canonical_json(&open_runtime).unwrap(),
            &successor.successor_b0_sha,
            &runtime.start_marker_sha256,
            &freeze,
        )
        .is_err()
    );
}

#[test]
fn r60_cells_use_actual_metal_and_execution_only_generations() {
    let expected = vec![
        "r60-h01/cpu",
        "r60-h01/actual-metal",
        "r60-h02/cpu",
        "r60-h02/actual-metal",
        "r60-h03/cpu",
        "r60-h03/actual-metal",
        "r60-h04/cpu",
        "r60-h04/actual-metal",
    ];
    assert_eq!(FormalRevision::R60.formal_cell_keys(), expected);
    assert_eq!(FormalRevision::R60.diagnostic_generation_bounds(), (0, 16));
    assert_eq!(FormalRevision::R60.artifact_namespace(), "r60");
    assert_eq!(
        FormalRevision::R60.completion_summary_contract(),
        "hanonly-r60-b0-completion-summary-v1"
    );
    assert_eq!(
        FormalRevision::R60.completion_summary_stdout_prefix(),
        R60_COMPLETION_SUMMARY_STDOUT_PREFIX
    );
    assert_eq!(
        formal_external_evidence_suffix(
            Some(FormalRevision::R60),
            &format!(
                "source-gate/holdout/{}/load.log",
                FormalRevision::R60.external_device("metal")
            ),
        ),
        "r60/source-gate/holdout/actual-metal/load.log"
    );
    assert_eq!(
        formal_artifact_suffix(Some(FormalRevision::R60), "r59/diagnostic-index.json"),
        "r60/diagnostic-index.json"
    );

    let pass = synthetic_formal_run(
        expected
            .iter()
            .map(|key| {
                let (entry, device) = key.split_once('/').unwrap();
                synthetic_formal_cell(entry, device, true)
            })
            .collect(),
    );
    assert_eq!(
        validate_formal_terminal_closure(FormalRevision::R60, "S25L4", &pass).unwrap(),
        expected
    );

    let mut cells = pass.cells[..2].to_vec();
    cells[1] = synthetic_formal_cell("r60-h01", "actual-metal", false);
    assert!(
        validate_formal_terminal_closure(
            FormalRevision::R60,
            "S25L4",
            &synthetic_formal_run(cells.clone()),
        )
        .is_ok()
    );
    cells.push(synthetic_formal_cell("r60-h02", "cpu", true));
    assert!(
        validate_formal_terminal_closure(
            FormalRevision::R60,
            "S25L4",
            &synthetic_formal_run(cells),
        )
        .is_err()
    );
}

#[test]
fn formal_successor_accepts_only_exact_calibration_artifact_binding() {
    let original_b0_sha = "a".repeat(40);
    let successor_b0_sha = "b".repeat(40);
    let artifact_sha256 = synthetic_hash(5);
    let freeze = FreezeCommitments {
        receipt_sha256: synthetic_hash(1),
        original_public_commitment_sha256: synthetic_hash(2),
        original_b0_sha: original_b0_sha.clone(),
        successor_b0_sha: successor_b0_sha.clone(),
        calibration_artifact_sha256: artifact_sha256.clone(),
        ciphertext_sha256: synthetic_hash(3),
        private_manifest_commitment_sha256: synthetic_hash(4),
        r60_layout: None,
    };

    assert!(freeze.accepts_calibration_artifact(
        &original_b0_sha,
        &successor_b0_sha,
        &artifact_sha256,
    ));
    assert!(!freeze.accepts_calibration_artifact(
        &"c".repeat(40),
        &successor_b0_sha,
        &artifact_sha256
    ));
    assert!(!freeze.accepts_calibration_artifact(
        &original_b0_sha,
        &"d".repeat(40),
        &artifact_sha256
    ));
    assert!(!freeze.accepts_calibration_artifact(
        &original_b0_sha,
        &successor_b0_sha,
        &synthetic_hash(6),
    ));
}

#[test]
fn formal_successor_holds_exact_artifact_and_uses_phase_b0_attestations() {
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

    let artifact_path = root.join("selection.json");
    let input = HeldInput::open(&artifact_path).unwrap();
    let mut artifact: FrozenArtifact = serde_json::from_slice(input.bytes()).unwrap();
    let original_b0_sha = artifact.b0_sha.clone();
    let successor_b0_sha = "b".repeat(40);
    let mut environment = parse_test_environment(&values).unwrap();
    environment.phase = Phase::Holdout;
    environment.b0_sha.clone_from(&successor_b0_sha);
    environment.formal_custody = Some(FormalCustody {
        revision: FormalRevision::R60,
        contract_sha256: synthetic_hash(7),
        holdout: Some(HoldoutCustody {
            directory: root.join("custody"),
            plaintext_directory: root.join("plaintext"),
            plaintext_archive: root.join("plaintext/bundle.tar"),
            freeze: FreezeCommitments {
                receipt_sha256: synthetic_hash(8),
                original_public_commitment_sha256: synthetic_hash(9),
                original_b0_sha: original_b0_sha.clone(),
                successor_b0_sha: successor_b0_sha.clone(),
                calibration_artifact_sha256: hex_sha256(input.sha256()),
                ciphertext_sha256: synthetic_hash(10),
                private_manifest_commitment_sha256: synthetic_hash(11),
                r60_layout: None,
            },
            expected_start_marker_sha256: synthetic_hash(12),
            open_marker: OnceCell::new(),
            runtime_commitment: OnceCell::new(),
        }),
    });

    hold_calibration_artifact(&environment, &input, &artifact).unwrap();
    validate_artifact(&artifact, Phase::CalibrationFreeze, &environment).unwrap();
    input.with_revalidated_path(|_| Ok(())).unwrap();

    values.insert(PHASE_ENV, "holdout".into());
    values.insert(B0_SHA_ENV, successor_b0_sha);
    set_required_check(&mut values, &root, Phase::Holdout);
    let holdout_environment = parse_test_environment(&values).unwrap();
    artifact
        .required_checks
        .push(holdout_environment.required_check.clone());
    validate_required_checks(&artifact, Phase::Holdout, &holdout_environment).unwrap();

    fs::write(&artifact_path, b"drifted calibration").unwrap();
    assert!(input.with_revalidated_path(|_| Ok(())).is_err());
}

#[test]
fn formal_terminal_pass_stays_non_authorizing_until_cleanup_receipt() {
    assert_eq!(
        pre_cleanup_completion_state(true),
        (
            "incomplete_non_authorizing",
            "terminal_pass_cleanup_pending"
        )
    );
    assert_eq!(
        pre_cleanup_completion_state(false),
        ("incomplete_non_authorizing", "completed_fail")
    );
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
    let mut result = synthetic_result("calibration", "r59-c01", "cpu", "S25L4");
    result.derived.source_coverage_preflight.pp_han_scalar_count = 0;
    result
        .derived
        .source_coverage_preflight
        .vl_expected_han_scalar_count = 4;

    assert!(validate_result(&result, &processes, "calibration").is_ok());
}

fn r59_test_schema_and_oracle() -> (VisualManifestEntry, OracleValidatedEntry) {
    let schema = serde_json::from_value(serde_json::json!({
        "id": "r59-c01",
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

#[test]
fn empty_prepared_masks_record_zero_support_and_failed_coverage() {
    for prepared in [
        PreparedInpaintMask::NoEligibleHanTargets,
        PreparedInpaintMask::EmptyMask,
    ] {
        let support = removal_support_from_prepared(prepared, 64, 64);
        assert_eq!(support.dimensions(), (64, 64));
        assert!(support.pixels().all(|pixel| pixel.0[0] == 0));
    }

    let mut prepared = GrayImage::new(3, 2);
    prepared.put_pixel(1, 1, image::Luma([255]));
    let support = removal_support_from_prepared(
        PreparedInpaintMask::Prepared {
            mask: DynamicImage::ImageLuma8(prepared.clone()),
            blocks: Vec::new(),
        },
        64,
        64,
    );
    assert_eq!(support.as_raw(), prepared.as_raw());

    let (schema, oracle) = r59_test_schema_and_oracle();
    let scene = scene_for_entry(&schema, &oracle, 64, 64);
    let page = *scene.pages.keys().next().unwrap();
    let ink = vec![255; 64 * 64];
    let (_, derived, _) = derive_result(
        "cpu",
        &scene,
        page,
        &schema,
        &oracle,
        &[SourceInkMask::page(&ink, 64, 64)],
        &GrayImage::new(64, 64),
        "lama-manga",
        "speech-bubble-segmentation",
        &synthetic_hash(90),
        &[],
    )
    .unwrap();
    assert_eq!(
        derived.source_coverage_preflight.source_text_roi_coverage,
        0.0
    );
    assert!(
        !derived
            .source_coverage_preflight
            .source_removal_preflight_passed
    );
    assert!(!derived.passed);
}

fn r59_test_quad_bits(left: f32, top: f32, right: f32, bottom: f32) -> [u32; 8] {
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
fn r57_detector_geometry_rejects_each_one_pixel_mutation() {
    let baseline = vec![0, 1, 1, 0];
    assert!(r57_detector_supports_equal(
        &baseline, &baseline, &baseline, &baseline
    ));
    for changed in 0..4 {
        let mut supports = [
            baseline.clone(),
            baseline.clone(),
            baseline.clone(),
            baseline.clone(),
        ];
        supports[changed][0] = 1;
        assert!(!r57_detector_supports_equal(
            &supports[0],
            &supports[1],
            &supports[2],
            &supports[3],
        ));
    }
}

#[test]
fn r57_calibration_and_probe_require_zero_protected_overlap() {
    assert!(r57_cell_passed(true, true, 0));
    assert!(!r57_cell_passed(true, true, 1));

    let stage = |missing_pixels, protected_overlap_pixels| EraseStageMetric {
        stage: EraseDiagnosticStage::InpaintFinal,
        branch: EraseDiagnosticBranch::HanOnly,
        grayscale_blake3: synthetic_hash(91),
        nonzero_pixels: 1,
        protected_overlap_pixels,
        targets: vec![EraseStageTargetMetric {
            target_id: "target".into(),
            oracle_pixels: 1,
            intersection_pixels: 1_u64.saturating_sub(missing_pixels),
            missing_pixels,
        }],
    };
    assert!(r57_final_erase_stage_passed(&stage(0, 0)));
    assert!(!r57_final_erase_stage_passed(&stage(1, 0)));
    assert!(!r57_final_erase_stage_passed(&stage(0, 1)));
}

#[test]
fn r57_actual_scene_support_rejects_transform_and_rotation_drift() {
    let baseline = Transform {
        x: 10.0,
        y: 10.0,
        width: 20.0,
        height: 20.0,
        rotation_deg: 0.0,
    };
    let text = TextData::default();
    let (support, rotations_zero) = r57_actual_scene_support(64, 64, &baseline, &text).unwrap();
    assert!(rotations_zero);

    let expanded = Transform {
        width: 21.0,
        ..baseline.clone()
    };
    assert_ne!(
        support.mask,
        r57_actual_scene_support(64, 64, &expanded, &text)
            .unwrap()
            .0
            .mask
    );

    let node_rotated = Transform {
        rotation_deg: 1.0,
        ..baseline.clone()
    };
    assert!(
        !r57_actual_scene_support(64, 64, &node_rotated, &text)
            .unwrap()
            .1
    );

    let text_rotated = TextData {
        rotation_deg: Some(1.0),
        ..Default::default()
    };
    assert!(
        !r57_actual_scene_support(64, 64, &baseline, &text_rotated)
            .unwrap()
            .1
    );
}

#[test]
fn r59_selection_geometry_closes_detector_ownership_preimages() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let values = valid_environment(&root);
    let environment = parse_test_environment(&values).unwrap();
    let (schema, oracle) = r59_test_schema_and_oracle();
    let result = synthetic_result("calibration", "r59-c01", "cpu", "S25L4");
    let node_id = NodeId::new();
    let target_bits = r59_test_quad_bits(10.0, 10.0, 20.0, 20.0);
    let second_target_bits = r59_test_quad_bits(25.0, 10.0, 35.0, 20.0);
    let protected_bits = r59_test_quad_bits(52.0, 10.0, 60.0, 20.0);
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
            vl_bounds: [0, 0, 64, 64],
            vl_crop_rgba_hash: synthetic_hash(48),
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
                    source_scaled_quad_f32_bits: second_target_bits,
                },
                PpDetectorDiagnostic {
                    occurrence_index: 2,
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
                        canonical_corners_f32_bits: second_target_bits,
                    }],
                    recognition: Some(PpRecognitionDiagnostic {
                        present: true,
                        recognition_class: "han",
                        segment_count: 1,
                    }),
                },
                PpCanonicalLineDiagnostic {
                    line_index: 2,
                    detector_occurrences: vec![PpCanonicalOccurrenceDiagnostic {
                        occurrence_index: 2,
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
            targets: vec![
                SourceGateTargetGeometryDiagnostic {
                    scene_quad_f32_bits: target_bits,
                    eligible_line_quads_f32_bits: vec![target_bits],
                },
                SourceGateTargetGeometryDiagnostic {
                    scene_quad_f32_bits: second_target_bits,
                    eligible_line_quads_f32_bits: vec![second_target_bits],
                },
            ],
            protected_lines: vec![SourceGateTargetGeometryDiagnostic {
                scene_quad_f32_bits: protected_bits,
                eligible_line_quads_f32_bits: vec![protected_bits],
            }],
            detector_ownership: vec![
                SourceGateDetectorOwnershipDiagnostic {
                    occurrence_index: 0,
                    canonical_line_index: Some(0),
                    scene_quad_f32_bits: target_bits,
                    eligible_text_line_quad_f32_bits: Some(target_bits),
                    assignment: SourceGateDetectorAssignmentDiagnostic::Target {
                        target_index: 0,
                    },
                },
                SourceGateDetectorOwnershipDiagnostic {
                    occurrence_index: 1,
                    canonical_line_index: Some(1),
                    scene_quad_f32_bits: second_target_bits,
                    eligible_text_line_quad_f32_bits: Some(second_target_bits),
                    assignment: SourceGateDetectorAssignmentDiagnostic::Target {
                        target_index: 1,
                    },
                },
                SourceGateDetectorOwnershipDiagnostic {
                    occurrence_index: 2,
                    canonical_line_index: Some(2),
                    scene_quad_f32_bits: protected_bits,
                    eligible_text_line_quad_f32_bits: Some(protected_bits),
                    assignment: SourceGateDetectorAssignmentDiagnostic::Protected {
                        protected_index: 0,
                    },
                },
            ],
        },
    ];
    let target_support = r59_rect_mask(64, 64, r59_quad_bits_rect(target_bits).unwrap());
    let second_target_support =
        r59_rect_mask(64, 64, r59_quad_bits_rect(second_target_bits).unwrap());
    let supports = CellSupportEvidence {
        width: 64,
        height: 64,
        scene_by_target: BTreeMap::from([(
            "target".to_owned(),
            vec![
                SceneSupportEvidence {
                    rect: [10, 10, 20, 20],
                    mask: target_support.clone(),
                    downstream_mask: target_support.clone(),
                },
                SceneSupportEvidence {
                    rect: [25, 10, 35, 20],
                    mask: second_target_support.clone(),
                    downstream_mask: second_target_support,
                },
            ],
        )]),
        selected_scene_rotations_zero: true,
        runtime_inpainter_id: "lama-manga".to_owned(),
        bubble_segmenter_id: "speech-bubble-segmentation".to_owned(),
        bubble_support_sha256: synthetic_hash(90),
        removal_support: vec![0; 64 * 64],
    };
    let (_, _, _, records, geometry_passed) = r59_detector_diagnostics(
        &environment,
        &result,
        &schema,
        &oracle,
        &diagnostics,
        &None,
        Some(&supports),
    )
    .unwrap();
    assert!(geometry_passed);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["preimage"]["target_id"], "target");
    assert_eq!(
        records[0]["preimage"]["canonical_assignment"],
        "selected_han"
    );
    assert_eq!(records[0]["preimage"]["ownership_verdict"], "unique");
    assert_eq!(
        records[0]["preimage"]["emitted_scene_quad"],
        serde_json::json!([10, 10, 20, 10, 20, 20, 10, 20])
    );
    assert_eq!(
        records[0]["preimage"]["detector_support_mask"],
        records[0]["preimage"]["line_support_mask"]
    );
    assert_eq!(
        records[0]["preimage"]["detector_support_mask"],
        records[0]["preimage"]["emitted_scene_support_mask"]
    );
    assert_eq!(
        records[0]["preimage"]["detector_support_mask"],
        records[0]["preimage"]["downstream_line_support_mask"]
    );
    assert_eq!(
        records[1]["preimage"]["canonical_assignment"],
        "selected_han"
    );
    assert_eq!(
        records[2]["preimage"]["canonical_assignment"],
        "preserved_source"
    );
    assert!(
        records[2]["preimage"]["protected_support_pixels"]
            .as_u64()
            .unwrap()
            > 0
    );
    let rejected_reason = Some("pp_vl_incomplete_coverage".to_owned());
    let (_, _, _, rejected, _) = r59_detector_diagnostics(
        &environment,
        &result,
        &schema,
        &oracle,
        &diagnostics[..3],
        &rejected_reason,
        None,
    )
    .unwrap();
    assert_eq!(rejected.len(), 3);
    assert!(rejected.iter().all(|record| {
        record["preimage"]["ownership_verdict"] == "unassigned"
            && record["preimage"]["selection_verdict"] == "rejected"
            && record["preimage"]["emitted_scene_quad"].is_null()
            && record["preimage"]["detector_support_mask"].is_object()
            && record["preimage"]["line_support_mask"].is_object()
            && record["preimage"]["agreed_mask"].is_object()
    }));
}

#[test]
fn r59_validated_execution_view_preserves_local_coverage_mask() {
    let mut page_mask = vec![0_u8; 64 * 64];
    for y in 10..20 {
        page_mask[y * 64 + 10..y * 64 + 20].fill(1);
    }
    let prepared = prepare_r59_execution_entries(R59ValidatedExecutionView {
        entries: vec![R59ValidatedExecutionEntry {
            id: "r59-h01".into(),
            source_encoded_bytes: vec![1].into_boxed_slice(),
            clean_reference_encoded_bytes: vec![2].into_boxed_slice(),
            validated_source_rgba: RgbaImage::new(64, 64),
            validated_clean_reference_rgba: RgbaImage::new(64, 64),
            source_width: 64,
            source_height: 64,
            clean_width: 64,
            clean_height: 64,
            protected_rois: vec![[40, 40, 50, 50]],
            targets: vec![R59ValidatedExecutionTarget {
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
    assert_eq!(prepared[0].0.id, "r59-h01");
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
fn r59_selected_and_downstream_support_have_independent_geometry_sources() {
    let (schema, oracle) = r59_test_schema_and_oracle();
    let node_id = NodeId::new();
    let diagnostics = vec![SourceGateDiagnosticEvent::SelectionGeometry {
        node_id,
        targets: vec![SourceGateTargetGeometryDiagnostic {
            scene_quad_f32_bits: r59_test_quad_bits(10.0, 10.0, 20.0, 20.0),
            eligible_line_quads_f32_bits: vec![r59_test_quad_bits(10.0, 10.0, 20.0, 20.0)],
        }],
        protected_lines: Vec::new(),
        detector_ownership: Vec::new(),
    }];
    let selected =
        r59_selected_support_from_diagnostics(64, 64, &schema, &oracle, &diagnostics).unwrap();
    let mut page = Page::new("r59-c01", 64, 64);
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
    let downstream = r59_downstream_support_from_scene(&page, &schema, &oracle).unwrap();
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

fn synthetic_formal_cell(entry: &str, device: &str, passed: bool) -> R59TerminalCellResult {
    let candidate_id = "S25L4";
    R59TerminalCellResult {
        cell_key: format!("{entry}/{device}"),
        result: if passed { "pass" } else { "fail-closed" }.into(),
        selection_result: Some(if passed { "selected" } else { "rejected" }.into()),
        target_recall: R59TargetRecall {
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

fn synthetic_formal_run(cells: Vec<R59TerminalCellResult>) -> R59FormalRunEvidence {
    let first_failed_cell = cells
        .iter()
        .find(|cell| cell.result != "pass")
        .map(|cell| cell.cell_key.clone());
    R59FormalRunEvidence {
        bundle_validation_receipt: Some(PublishedArtifact {
            path: "reports/r59/bundle-validation.json".into(),
            sha256: synthetic_hash(15),
            byte_length: 1,
        }),
        cells,
        first_failed_cell,
    }
}

#[test]
fn r59_publication_is_create_new_mode_0600_and_canonical_without_newline() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let values = valid_environment(&root);
    let environment = parse_test_environment(&values).unwrap();
    let bytes = canonical_json(&serde_json::json!({"b": 2, "a": 1})).unwrap();
    let published = publish_r59_artifact(&environment, "r59/publication.json", &bytes).unwrap();
    let path = root.join(&published.path);

    assert_eq!(bytes, br#"{"a":1,"b":2}"#);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert!(publish_r59_artifact(&environment, "r59/publication.json", &bytes).is_err());
    assert!(!fs::read_dir(path.parent().unwrap()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
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
        formal: None,
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
        formal: None,
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
    assert_eq!(artifact.holdout_entry_ids, r59_entry_ids('h'));
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
    assert_eq!(artifact.holdout_entry_ids, r59_entry_ids('h'));
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
            parse_test_environment(&values).unwrap()
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
        assert!(parse_test_environment(&values).is_err());
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
fn r52_bridge_request_schema_is_closed() {
    let request_value = serde_json::json!({
        "contract": "hanonly-r52-evidence-bridge-request-v1",
        "plan_revision": 52,
        "mode": "challenge",
        "b0_sha": "a".repeat(40),
        "repo_root": "/repo",
        "evidence_root": "/evidence",
        "result_path": "/evidence/.r52-challenge-result-1.tmp",
        "selected_candidate_id": "S25L4",
        "challenge_manifest_path": "/challenge/manifest.json",
        "challenge_manifest_sha256": R52_CHALLENGE_MANIFEST_SHA256,
        "challenge_hash_record_path": "/challenge/hashes.json",
        "challenge_hash_record_sha256": R52_CHALLENGE_HASHES_SHA256,
        "r49_visual_manifest_path": R49_VISUAL_MANIFEST,
        "r49_visual_manifest_sha256": R49_VISUAL_MANIFEST_SHA256,
        "source_gate_fixture_manifest_sha256": "b".repeat(64),
        "calibration_selection_artifact_path": "/evidence/selection.json",
        "b0_preflight_attestation_path": "/evidence/preflight.json",
    });
    let request: R52BridgeRequest =
        serde_json::from_value(request_value.clone()).expect("closed R52 request");
    let temp = tempfile::tempdir().expect("R52 request temp");
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
        .expect("secure R52 request parent");
    let path = temp.path().join("request.json");
    fs::write(
        &path,
        canonical_json(&request).expect("canonical R52 request"),
    )
    .expect("write R52 request");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("secure R52 request");
    assert!(load_r52_bridge_request_path(fs::canonicalize(&path).unwrap()).is_ok());
    let mut unknown = request_value;
    unknown["operator"] = serde_json::json!("forbidden");
    assert!(serde_json::from_value::<R52BridgeRequest>(unknown).is_err());
}

#[test]
fn r52_bridge_applies_only_exact_protected_latin_correction() {
    let (mut schema, mut oracle) = r59_test_schema_and_oracle();
    schema.id = "r49-h04".into();
    schema.protected_rois.clear();
    oracle.protected_rois.clear();
    schema.targets.push(VisualManifestTarget {
        id: "product-id".into(),
        source_roi: [50, 0, 64, 64],
        clean_reference_edit_roi: [50, 0, 64, 64],
        erase_source_ink_mask_path: "product-id-erase.bin".into(),
        erase_source_ink_mask_sha256: synthetic_hash(46),
        residual_source_ink_mask_path: "product-id-residual.bin".into(),
        residual_source_ink_mask_sha256: synthetic_hash(47),
        position: Position::Interior,
        writing: Writing::Horizontal,
        effect: Effect::Plain,
        translation_length: TranslationLength::Equal,
        expected: Expected::AutomaticStrict,
    });
    oracle.targets.push(OracleValidatedTarget {
        source_roi: ValidatedHalfOpenRect {
            left: 50,
            top: 0,
            right: 64,
            bottom: 64,
        },
        edit_roi: ValidatedHalfOpenRect {
            left: 50,
            top: 0,
            right: 64,
            bottom: 64,
        },
        delta_mask: vec![1; 14 * 64].into_boxed_slice(),
    });
    apply_r52_protected_latin_correction(&mut schema, &mut oracle)
        .expect("apply protected Latin correction");
    assert!(
        schema
            .targets
            .iter()
            .all(|target| target.id != "product-id")
    );
    assert_eq!(schema.targets.len(), oracle.targets.len());

    let mut result = synthetic_result("holdout", "r49-h04", "cpu", "S25L4");
    result.derived.passed = false;
    result.derived.source_coverage_preflight.rejected_after_vl = true;
    result
        .derived
        .source_coverage_preflight
        .pp_vl_complete_coverage = false;
    result
        .derived
        .source_coverage_preflight
        .source_removal_preflight_passed = false;
    result.derived.protected_false_positive_count = 0;
    assert!(r52_challenge_cell_passed(
        &result,
        &schema,
        Some("pp_no_han_protected_latin"),
        "regression"
    ));
    result
        .derived
        .source_coverage_preflight
        .covered_source_roi_ids
        .clear();
    assert!(!r52_challenge_cell_passed(
        &result,
        &schema,
        Some("pp_no_han_protected_latin"),
        "regression"
    ));
    result
        .derived
        .source_coverage_preflight
        .covered_source_roi_ids = vec!["target".into()];
    assert!(!r52_challenge_cell_passed(
        &result,
        &schema,
        Some("pp_no_han_unprotected"),
        "regression"
    ));
    result.entry_id = "r49-h03".into();
    assert!(!r52_challenge_cell_passed(
        &result,
        &schema,
        Some("pp_no_han_protected_latin"),
        "regression"
    ));
}

#[test]
#[ignore = "requires a canonical R52 bridge request and installed Source Gate models"]
fn han_only_r52_evidence_bridge() {
    run_r52_evidence_bridge().expect("R52 evidence bridge failed");
}

#[test]
#[ignore = "requires frozen B0 selection environment and installed Source Gate models"]
fn han_only_source_ink_erase_stage_probe() {
    let environment = SelectionEnvironment::parse(|name| std::env::var(name).ok())
        .expect("erase-stage probe environment");
    assert!(!environment.artifact.exists());
    let evidence = run_erase_stage_probe(&environment).expect("erase-stage probe failed");
    assert_eq!(evidence.selected_candidate_id, "S25L4");
    assert_eq!(evidence.results.len(), 8);
    assert!(evidence.results.iter().all(|result| {
        environment.calibration_entry_ids.contains(&result.entry_id)
            && result.candidate_id == "S25L4"
    }));
    assert!(!environment.artifact.exists());
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
