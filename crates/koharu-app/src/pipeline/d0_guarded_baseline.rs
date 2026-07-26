//! Test-only G001/D0 guarded baseline schema. This checkpoint creates bytes only.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;

use image::{GenericImageView, ImageFormat};
use koharu_core::NodeId;
use rustix::fs::{FileType, Mode, OFlags, fstat, open};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::renderer::RendererDiagnosticEvent;
use crate::typography::TypographyTargetDiagnostic;

use super::d0_visual_manifest_schema::{EntryRole, Expected, HeldVisualManifestSchema};
use super::engines::support::{EraseDiagnosticEvent, EraseDiagnosticStage};

const REPORT_SCHEMA: &str = "hanonly-d0-guarded-baseline-v1";
const MAP_SCHEMA: &str = "hanonly-d0-target-correlation-map-v1";
const CHECKPOINT: &str = "G001/D0";
const EVIDENCE_CLASS: &str = "guarded_detail";
const SEMANTICS: &str = "observational_current_baseline";
const PUBLICATION_CONTRACT: &str = "d0-atomic-runtime-bundle-v1";
const IMAGE_INPUT_CONTRACT: &str = "image-input-contract-v1";
const PLAN_REVISION: u32 = 46;
const RANDOM_ATTEMPTS: usize = 16;
const TOKEN_LIMIT: usize = 128;

pub(super) type D0Result<T> = Result<T, String>;
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TargetCorrelationMap {
    pub(super) schema: String,
    pub(super) manifest_sha256: String,
    pub(super) records: Vec<TargetCorrelationRecord>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TargetCorrelationRecord {
    pub(super) entry_id: String,
    pub(super) target_id: String,
    pub(super) target_correlation_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GuardedBaselineReport {
    pub(super) schema: String,
    pub(super) plan_revision: u32,
    pub(super) checkpoint: String,
    pub(super) evidence_class: String,
    pub(super) semantics: String,
    pub(super) publication_contract: String,
    pub(super) target_correlation_map_sha256: String,
    pub(super) provenance: Provenance,
    pub(super) descriptors: DescriptorSnapshot,
    pub(super) entries: Vec<EntryReport>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Provenance {
    pub(super) git_head: String,
    pub(super) executable_sha256: String,
    pub(super) ledger_sha256: String,
    pub(super) manifest_sha256: String,
    pub(super) fixture_manifest_sha256: String,
    pub(super) app_config_sha256: String,
    pub(super) translation_provider_id: String,
    pub(super) translation_model_id: String,
    pub(super) typography_provider_id: String,
    pub(super) typography_model_id: String,
    pub(super) target_language: String,
    pub(super) requested_backend_id: String,
    pub(super) observed_backend_id: String,
    pub(super) engines: EngineIds,
    pub(super) image_input_contract: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EngineIds {
    pub(super) detector: String,
    pub(super) source_gate: String,
    pub(super) font_detector: String,
    pub(super) segmenter: String,
    pub(super) bubble_segmenter: String,
    pub(super) translator: String,
    pub(super) typography_planner: String,
    pub(super) inpainter: String,
    pub(super) renderer: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DescriptorSnapshot {
    pub(super) ledger: FileDescriptor,
    pub(super) visual_manifest: FileDescriptor,
    pub(super) fixture_manifest: FileDescriptor,
    pub(super) app_config: FileDescriptor,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileDescriptor {
    pub(super) sha256: String,
    pub(super) dev: i128,
    pub(super) ino: u64,
    pub(super) file_type: DescriptorFileType,
    pub(super) owner: u64,
    pub(super) mode: u64,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DescriptorFileType {
    RegularFile,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EntryReport {
    pub(super) index: u32,
    pub(super) entry_id: String,
    pub(super) role: ReportEntryRole,
    pub(super) source: ImageIdentity,
    pub(super) clean: ImageIdentity,
    pub(super) artifacts: Vec<Artifact>,
    pub(super) warning_count: u32,
    pub(super) warnings: Vec<Warning>,
    pub(super) erase: PageEraseDiagnostics,
    pub(super) targets: Vec<TargetReport>,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ReportEntryRole {
    Regression,
    Calibration,
    Holdout,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Dimensions {
    pub(super) width: u32,
    pub(super) height: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImageIdentity {
    pub(super) descriptor: FileDescriptor,
    pub(super) decoded_rgba_blake3: String,
    pub(super) dimensions: Dimensions,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub(super) kind: ArtifactKind,
    pub(super) dimensions: Dimensions,
    pub(super) encoded_sha256: String,
    pub(super) content_blake3: String,
    pub(super) byte_count: u64,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ArtifactKind {
    Source,
    RawSegmentMask,
    FinalEraseMask,
    Inpainted,
    Rendered,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Warning {
    pub(super) step_id: String,
    pub(super) message_sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PageEraseDiagnostics {
    pub(super) raw_segment_artifact_blake3: String,
    pub(super) final_erase_artifact_blake3: String,
    pub(super) inpainted_artifact_blake3: String,
    pub(super) events: Vec<EraseDiagnosticEvent>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TargetReport {
    pub(super) target_correlation_id: String,
    pub(super) node_id: NodeId,
    pub(super) source_line_count: u32,
    pub(super) ocr_line_count: u32,
    pub(super) translated_line_count: u32,
    pub(super) expected: ReportExpected,
    pub(super) observed_terminal: ObservedTerminal,
    pub(super) expectation_relation: ExpectationRelation,
    pub(super) source_gate: SourceGateFacts,
    pub(super) erase_observation: TargetEraseObservation,
    pub(super) erase_source_ink_mask: FileDescriptor,
    pub(super) residual_source_ink_mask: FileDescriptor,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(super) renderer_diagnostic: Option<RendererDiagnosticEvent>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(super) typography_diagnostic: Option<TypographyTargetDiagnostic>,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReportExpected {
    AutomaticStrict,
    ManualOverride,
    UnsupportedSourceColor,
    UnsupportedRotation,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ObservedTerminal {
    Rendered,
    PreservedWithoutSprite,
    MutatedWithoutSprite,
    ErasedWithoutSprite,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ExpectationRelation {
    SameTerminalClass,
    DifferentTerminalClass,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceGateFacts {
    pub(super) candidate_index: u32,
    pub(super) anchor_bbox: [f32; 4],
    pub(super) crop_bounds: [u32; 4],
    pub(super) crop_rgba_blake3: String,
    pub(super) pp_word_count: u32,
    pub(super) pp_character_count: u32,
    pub(super) pp_line_count: u32,
    pub(super) vl_contains_han: bool,
    pub(super) vl_character_count: u32,
    pub(super) vl_line_count: u32,
    pub(super) selected: bool,
    pub(super) terminal_class: SourceGateTerminalClass,
    pub(super) fallback: SourceGateFallback,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SourceGateTerminalClass {
    InvalidCandidateGeometry,
    RejectedBeforeVl,
    RejectedAfterVl,
    AcceptedPrimary,
    AcceptedFallback,
    VlBatchError,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SourceGateFallback {
    None,
    IsolatedProtectedLatinGeometry,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TargetEraseObservation {
    pub(super) raw_segment_mask_blake3: String,
    pub(super) final_erase_mask_blake3: String,
    pub(super) target_mask_pixels: u64,
    pub(super) final_erase_overlap_pixels: u64,
    pub(super) source_ink_outside_final_mask_pixels: u64,
    pub(super) protected_overlap_pixels: u64,
    pub(super) clean_residual_pixels: u64,
    pub(super) source_inpainted_roi_equal: bool,
    pub(super) source_rendered_roi_equal: bool,
    pub(super) sprite_present: bool,
}
pub(super) struct RuntimeLineEvidence<'a> {
    pub(super) target_correlation_id: &'a str,
    pub(super) raw_source: &'a str,
    pub(super) raw_translated: &'a str,
    pub(super) raw_ocr_line_count: u32,
}
pub(super) struct RuntimeArtifactEvidence<'a> {
    pub(super) entry_id: &'a str,
    pub(super) bytes: [&'a [u8]; 5],
}

impl TargetCorrelationMap {
    pub(super) fn correlation_id(&self, entry_id: &str, target_id: &str) -> Option<&str> {
        self.records
            .iter()
            .find(|record| record.entry_id == entry_id && record.target_id == target_id)
            .map(|record| record.target_correlation_id.as_str())
    }

    pub(super) fn canonical_sha256(&self) -> D0Result<String> {
        canonical_target_correlation_map_bytes(self).map(|bytes| sha256_hex(&bytes))
    }
}

impl GuardedBaselineReport {
    pub(super) fn new(
        map: &TargetCorrelationMap,
        provenance: Provenance,
        descriptors: DescriptorSnapshot,
        entries: Vec<EntryReport>,
    ) -> D0Result<Self> {
        Ok(Self {
            schema: REPORT_SCHEMA.into(),
            plan_revision: PLAN_REVISION,
            checkpoint: CHECKPOINT.into(),
            evidence_class: EVIDENCE_CLASS.into(),
            semantics: SEMANTICS.into(),
            publication_contract: PUBLICATION_CONTRACT.into(),
            target_correlation_map_sha256: map.canonical_sha256()?,
            provenance,
            descriptors,
            entries,
        })
    }
}

pub(super) fn generate_target_correlation_map(
    held: &HeldVisualManifestSchema,
) -> D0Result<TargetCorrelationMap> {
    let fd = open(
        "/dev/urandom",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let stat = fstat(&fd).map_err(|error| error.to_string())?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::CharacterDevice {
        return Err("OS random source is not a character device".into());
    }
    let mut source = File::from(fd);
    generate_target_correlation_map_with(held, |bytes| {
        source.read_exact(bytes).map_err(|error| error.to_string())
    })
}

fn generate_target_correlation_map_with(
    held: &HeldVisualManifestSchema,
    mut fill: impl FnMut(&mut [u8]) -> D0Result<()>,
) -> D0Result<TargetCorrelationMap> {
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    for entry in &held.schema.entries {
        for target in &entry.targets {
            let mut id = None;
            for _ in 0..RANDOM_ATTEMPTS {
                let mut random = [0_u8; 16];
                fill(&mut random)?;
                let candidate = hex(&random);
                if seen.insert(candidate.clone()) {
                    id = Some(candidate);
                    break;
                }
            }
            records.push(TargetCorrelationRecord {
                entry_id: entry.id.clone(),
                target_id: target.id.clone(),
                target_correlation_id: id
                    .ok_or_else(|| "correlation ID collision retry exhausted".to_string())?,
            });
        }
    }
    let map = TargetCorrelationMap {
        schema: MAP_SCHEMA.into(),
        manifest_sha256: hex(&held.manifest.sha256()),
        records,
    };
    validate_map(&map)?;
    Ok(map)
}

pub(super) fn canonical_target_correlation_map_bytes(
    map: &TargetCorrelationMap,
) -> D0Result<Vec<u8>> {
    validate_map(map)?;
    canonical_bytes(map)
}

pub(super) fn parse_target_correlation_map(
    held: &HeldVisualManifestSchema,
    bytes: &[u8],
) -> D0Result<TargetCorrelationMap> {
    parse_canonical(bytes, |map| validate_map_for_held(held, map))
}

pub(super) fn canonical_guarded_baseline_report_bytes(
    held: &HeldVisualManifestSchema,
    map: &TargetCorrelationMap,
    report: &GuardedBaselineReport,
    runtime_lines: &[RuntimeLineEvidence<'_>],
    runtime_artifacts: &[RuntimeArtifactEvidence<'_>],
) -> D0Result<Vec<u8>> {
    validate_map_for_held(held, map)?;
    validate_report_for_held(held, report)?;
    validate_report(report, map, runtime_lines, runtime_artifacts)?;
    canonical_bytes(report)
}

pub(super) fn parse_guarded_baseline_report(
    held: &HeldVisualManifestSchema,
    bytes: &[u8],
    map: &TargetCorrelationMap,
    runtime_lines: &[RuntimeLineEvidence<'_>],
    runtime_artifacts: &[RuntimeArtifactEvidence<'_>],
) -> D0Result<GuardedBaselineReport> {
    validate_map_for_held(held, map)?;
    parse_canonical(bytes, |report| {
        validate_report_for_held(held, report)?;
        validate_report(report, map, runtime_lines, runtime_artifacts)
    })
}

fn canonical_bytes<T: Serialize>(value: &T) -> D0Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn parse_canonical<T>(bytes: &[u8], validate: impl FnOnce(&T) -> D0Result<()>) -> D0Result<T>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    if bytes.last() != Some(&b'\n') || bytes[..bytes.len() - 1].last() == Some(&b'\n') {
        return Err("canonical JSON must have exactly one trailing newline".into());
    }
    let value: T =
        serde_json::from_slice(&bytes[..bytes.len() - 1]).map_err(|error| error.to_string())?;
    validate(&value)?;
    if canonical_bytes(&value)? != bytes {
        return Err("noncanonical JSON bytes".into());
    }
    Ok(value)
}

fn validate_map(map: &TargetCorrelationMap) -> D0Result<()> {
    require(map.schema == MAP_SCHEMA, "map schema drift")?;
    require(is_sha256(&map.manifest_sha256), "invalid manifest sha256")?;
    require(!map.records.is_empty(), "map records are empty")?;
    let mut triples = HashSet::new();
    let mut correlations = HashSet::new();
    for record in &map.records {
        require(token(&record.entry_id), "invalid entry ID")?;
        require(token(&record.target_id), "invalid target ID")?;
        require(
            is_correlation_id(&record.target_correlation_id),
            "invalid correlation ID",
        )?;
        require(
            triples.insert((&record.entry_id, &record.target_id)),
            "duplicate map target",
        )?;
        require(
            correlations.insert(&record.target_correlation_id),
            "duplicate correlation ID",
        )?;
    }
    Ok(())
}

fn validate_map_for_held(
    held: &HeldVisualManifestSchema,
    map: &TargetCorrelationMap,
) -> D0Result<()> {
    validate_map(map)?;
    require(
        map.manifest_sha256 == hex(&held.manifest.sha256()),
        "held manifest hash mismatch",
    )?;
    let expected = held
        .schema
        .entries
        .iter()
        .flat_map(|entry| {
            entry
                .targets
                .iter()
                .map(move |target| (&entry.id, &target.id))
        })
        .collect::<Vec<_>>();
    require(
        expected.len() == map.records.len(),
        "map cardinality mismatch",
    )?;
    require(
        expected
            .iter()
            .zip(&map.records)
            .all(|((entry, target), record)| {
                *entry == &record.entry_id && *target == &record.target_id
            }),
        "map order or ID mismatch",
    )
}

fn validate_report_for_held(
    held: &HeldVisualManifestSchema,
    report: &GuardedBaselineReport,
) -> D0Result<()> {
    require(
        held.schema.entries.len() == report.entries.len(),
        "held/report entry cardinality mismatch",
    )?;
    for (held_entry, report_entry) in held.schema.entries.iter().zip(&report.entries) {
        require(
            held_entry.id == report_entry.entry_id
                && ReportEntryRole::from(held_entry.role) == report_entry.role
                && held_entry.targets.len() == report_entry.targets.len()
                && held_entry.sha256 == report_entry.source.descriptor.sha256
                && held_entry.decoded_rgba_blake3 == report_entry.source.decoded_rgba_blake3
                && held_entry.clean_reference_sha256 == report_entry.clean.descriptor.sha256
                && held_entry.clean_reference_decoded_rgba_blake3
                    == report_entry.clean.decoded_rgba_blake3,
            "held/report entry mismatch",
        )?;
        for (held_target, report_target) in held_entry.targets.iter().zip(&report_entry.targets) {
            require(
                ReportExpected::from(held_target.expected) == report_target.expected,
                "held/report target expectation mismatch",
            )?;
        }
    }
    Ok(())
}

fn validate_report(
    report: &GuardedBaselineReport,
    map: &TargetCorrelationMap,
    runtime_lines: &[RuntimeLineEvidence<'_>],
    runtime_artifacts: &[RuntimeArtifactEvidence<'_>],
) -> D0Result<()> {
    validate_map(map)?;
    require(
        report.schema == REPORT_SCHEMA
            && report.plan_revision == PLAN_REVISION
            && report.checkpoint == CHECKPOINT
            && report.evidence_class == EVIDENCE_CLASS
            && report.semantics == SEMANTICS
            && report.publication_contract == PUBLICATION_CONTRACT
            && report.target_correlation_map_sha256 == map.canonical_sha256()?,
        "report constants drift",
    )?;
    validate_provenance(report, map)?;
    require(
        report.entries.len() == 9,
        "report must contain nine entries",
    )?;
    let roles = report.entries.iter().fold([0_u8; 3], |mut count, entry| {
        count[match entry.role {
            ReportEntryRole::Regression => 0,
            ReportEntryRole::Calibration => 1,
            ReportEntryRole::Holdout => 2,
        }] += 1;
        count
    });
    require(roles == [1, 4, 4], "entry roles must be 1+4+4")?;
    let artifact_evidence = runtime_artifacts
        .iter()
        .map(|evidence| (evidence.entry_id, evidence))
        .collect::<HashMap<_, _>>();
    require(
        artifact_evidence.len() == runtime_artifacts.len()
            && artifact_evidence.len() == report.entries.len(),
        "artifact evidence cardinality mismatch",
    )?;

    let map_entries =
        map.records
            .iter()
            .fold(HashMap::<&str, Vec<&str>>::new(), |mut grouped, record| {
                grouped
                    .entry(&record.entry_id)
                    .or_default()
                    .push(&record.target_correlation_id);
                grouped
            });
    let mut entry_ids = HashSet::new();
    let mut target_ids = HashSet::new();
    for (index, entry) in report.entries.iter().enumerate() {
        require(entry.index == index as u32, "noncontiguous entry index")?;
        require(
            token(&entry.entry_id) && entry_ids.insert(&entry.entry_id),
            "invalid or duplicate entry ID",
        )?;
        require(
            entry.warning_count as usize == entry.warnings.len(),
            "warning cardinality mismatch",
        )?;
        let mut warning_steps = HashSet::new();
        for warning in &entry.warnings {
            require(
                token(&warning.step_id)
                    && warning_steps.insert(&warning.step_id)
                    && is_sha256(&warning.message_sha256),
                "invalid warning",
            )?;
        }
        validate_image_identity(&entry.source)?;
        validate_image_identity(&entry.clean)?;
        require(
            entry.source.dimensions == entry.clean.dimensions,
            "source and clean dimensions differ",
        )?;
        let artifacts = artifact_evidence
            .get(entry.entry_id.as_str())
            .ok_or_else(|| "entry artifact evidence is missing".to_string())?;
        validate_artifacts(entry, artifacts)?;
        validate_page_erase(entry)?;
        let expected_targets = map_entries
            .get(entry.entry_id.as_str())
            .ok_or_else(|| "entry missing from map".to_string())?;
        require(
            expected_targets.len() == entry.targets.len(),
            "entry target cardinality mismatch",
        )?;
        let mut node_ids = HashSet::new();
        let mut candidate_indices = HashSet::new();
        for (expected_id, target) in expected_targets.iter().zip(&entry.targets) {
            require(
                *expected_id == target.target_correlation_id
                    && target_ids.insert(&target.target_correlation_id)
                    && node_ids.insert(target.node_id)
                    && candidate_indices.insert(target.source_gate.candidate_index),
                "target correlation mismatch",
            )?;
            validate_target(target, entry.source.dimensions, &entry.erase)?;
        }
    }
    require(
        entry_ids.len() == map_entries.len(),
        "entry ID set mismatch",
    )?;
    require(
        target_ids.len() == map.records.len(),
        "correlation set mismatch",
    )?;
    validate_runtime(report, runtime_lines)
}

fn validate_provenance(report: &GuardedBaselineReport, map: &TargetCorrelationMap) -> D0Result<()> {
    let p = &report.provenance;
    for hash in [
        &p.executable_sha256,
        &p.ledger_sha256,
        &p.manifest_sha256,
        &p.fixture_manifest_sha256,
        &p.app_config_sha256,
    ] {
        require(is_sha256(hash), "invalid provenance hash")?;
    }
    require(is_git_head(&p.git_head), "invalid git head")?;
    let metadata = [
        &p.translation_provider_id,
        &p.translation_model_id,
        &p.typography_provider_id,
        &p.typography_model_id,
        &p.target_language,
        &p.requested_backend_id,
        &p.observed_backend_id,
        &p.engines.detector,
        &p.engines.source_gate,
        &p.engines.font_detector,
        &p.engines.segmenter,
        &p.engines.bubble_segmenter,
        &p.engines.translator,
        &p.engines.typography_planner,
        &p.engines.inpainter,
        &p.engines.renderer,
    ];
    for value in metadata {
        require(token(value), "invalid metadata token")?;
    }
    let engines = [
        &p.engines.detector,
        &p.engines.source_gate,
        &p.engines.font_detector,
        &p.engines.segmenter,
        &p.engines.bubble_segmenter,
        &p.engines.translator,
        &p.engines.typography_planner,
        &p.engines.inpainter,
        &p.engines.renderer,
    ];
    require(
        engines.into_iter().collect::<HashSet<_>>().len() == engines.len(),
        "engine IDs must be unique",
    )?;
    require(
        p.image_input_contract == IMAGE_INPUT_CONTRACT,
        "image input contract drift",
    )?;
    require(
        p.manifest_sha256 == map.manifest_sha256
            && p.ledger_sha256 == report.descriptors.ledger.sha256
            && p.manifest_sha256 == report.descriptors.visual_manifest.sha256
            && p.fixture_manifest_sha256 == report.descriptors.fixture_manifest.sha256
            && p.app_config_sha256 == report.descriptors.app_config.sha256,
        "provenance descriptor mismatch",
    )?;
    for descriptor in [
        &report.descriptors.ledger,
        &report.descriptors.visual_manifest,
        &report.descriptors.fixture_manifest,
        &report.descriptors.app_config,
    ] {
        validate_descriptor(descriptor)?;
    }
    Ok(())
}

fn validate_artifacts(entry: &EntryReport, evidence: &RuntimeArtifactEvidence<'_>) -> D0Result<()> {
    let expected = [
        ArtifactKind::Source,
        ArtifactKind::RawSegmentMask,
        ArtifactKind::FinalEraseMask,
        ArtifactKind::Inpainted,
        ArtifactKind::Rendered,
    ];
    require(
        entry.artifacts.len() == expected.len(),
        "artifact count drift",
    )?;
    require(
        evidence.entry_id == entry.entry_id,
        "artifact entry mismatch",
    )?;
    for ((artifact, kind), bytes) in entry.artifacts.iter().zip(expected).zip(evidence.bytes) {
        require(artifact.kind == kind, "artifact order or kind drift")?;
        require(
            artifact.dimensions == entry.source.dimensions,
            "artifact dimensions differ",
        )?;
        require(
            is_sha256(&artifact.encoded_sha256)
                && is_blake3(&artifact.content_blake3)
                && artifact.byte_count == bytes.len() as u64
                && artifact.encoded_sha256 == sha256_hex(bytes),
            "invalid artifact identity",
        )?;
        let (dimensions, content_blake3) = inspect_png_artifact(bytes, kind)?;
        require(
            dimensions == artifact.dimensions && content_blake3 == artifact.content_blake3,
            "artifact decoded identity mismatch",
        )?;
    }
    require(
        entry.artifacts[0].content_blake3 == entry.source.decoded_rgba_blake3,
        "source artifact content mismatch",
    )
}

fn inspect_png_artifact(bytes: &[u8], kind: ArtifactKind) -> D0Result<(Dimensions, String)> {
    require(
        image::guess_format(bytes).ok() == Some(ImageFormat::Png),
        "artifact must be PNG",
    )?;
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .map_err(|_| "artifact PNG decode failed".to_string())?;
    let (width, height) = image.dimensions();
    let content_blake3 = match kind {
        ArtifactKind::RawSegmentMask | ArtifactKind::FinalEraseMask => {
            blake3::hash(image.to_luma8().as_raw()).to_hex().to_string()
        }
        ArtifactKind::Source | ArtifactKind::Inpainted | ArtifactKind::Rendered => {
            blake3::hash(image.to_rgba8().as_raw()).to_hex().to_string()
        }
    };
    Ok((Dimensions { width, height }, content_blake3))
}

fn validate_page_erase(entry: &EntryReport) -> D0Result<()> {
    use EraseDiagnosticStage::{
        InpaintAllowedSupport, InpaintBackendExpanded, InpaintFinal, InpaintInputSegment,
        InpaintPreExpandFiltered, SegmentAllowedSupport, SegmentFinal, SegmentProbability,
        SegmentRefined,
    };
    let erase = &entry.erase;
    let expected = [
        SegmentProbability,
        SegmentRefined,
        SegmentAllowedSupport,
        SegmentFinal,
        InpaintInputSegment,
        InpaintAllowedSupport,
        InpaintPreExpandFiltered,
        InpaintBackendExpanded,
        InpaintFinal,
    ];
    require(
        is_blake3(&erase.raw_segment_artifact_blake3)
            && is_blake3(&erase.final_erase_artifact_blake3)
            && is_blake3(&erase.inpainted_artifact_blake3)
            && erase.raw_segment_artifact_blake3 == entry.artifacts[1].content_blake3
            && erase.final_erase_artifact_blake3 == entry.artifacts[2].content_blake3
            && erase.inpainted_artifact_blake3 == entry.artifacts[3].content_blake3
            && erase.events.len() == expected.len(),
        "erase stage artifact mismatch",
    )?;
    let branch = erase.events.first().map(|event| event.branch);
    for (index, (event, expected_stage)) in erase.events.iter().zip(expected).enumerate() {
        require(
            event.stage == expected_stage && Some(event.branch) == branch,
            "erase diagnostic order or branch mismatch",
        )?;
        require(
            if index + 1 == erase.events.len() {
                event.returns_some.is_some()
            } else {
                event.returns_some.is_none()
            },
            "erase diagnostic returns_some mismatch",
        )?;
        if let Some(mask) = &event.mask {
            require(
                Dimensions {
                    width: mask.width,
                    height: mask.height,
                } == entry.source.dimensions
                    && is_blake3(&mask.grayscale_blake3)
                    && mask.nonzero_pixels <= u64::from(mask.width) * u64::from(mask.height),
                "invalid erase diagnostic mask",
            )?;
        }
    }
    let segment_final = erase.events[3]
        .mask
        .as_ref()
        .ok_or_else(|| "SegmentFinal mask is missing".to_string())?;
    let inpaint_final = erase.events[8]
        .mask
        .as_ref()
        .ok_or_else(|| "InpaintFinal mask is missing".to_string())?;
    require(
        segment_final.grayscale_blake3 == erase.raw_segment_artifact_blake3
            && inpaint_final.grayscale_blake3 == erase.final_erase_artifact_blake3,
        "erase diagnostic artifact content mismatch",
    )
}

fn validate_target(
    target: &TargetReport,
    dimensions: Dimensions,
    erase: &PageEraseDiagnostics,
) -> D0Result<()> {
    require(
        is_correlation_id(&target.target_correlation_id),
        "invalid target correlation ID",
    )?;
    require(
        target.source_line_count > 0
            && target.ocr_line_count > 0
            && target.translated_line_count > 0,
        "line counts must be positive",
    )?;
    let expected_terminal = match target.expected {
        ReportExpected::AutomaticStrict | ReportExpected::ManualOverride => {
            ObservedTerminal::Rendered
        }
        ReportExpected::UnsupportedSourceColor | ReportExpected::UnsupportedRotation => {
            ObservedTerminal::PreservedWithoutSprite
        }
    };
    let expected_relation = if target.observed_terminal == expected_terminal {
        ExpectationRelation::SameTerminalClass
    } else {
        ExpectationRelation::DifferentTerminalClass
    };
    require(
        target.expectation_relation == expected_relation,
        "expectation relation mismatch",
    )?;
    validate_descriptor(&target.erase_source_ink_mask)?;
    validate_descriptor(&target.residual_source_ink_mask)?;
    require(
        is_blake3(&target.erase_observation.raw_segment_mask_blake3)
            && is_blake3(&target.erase_observation.final_erase_mask_blake3)
            && target.erase_observation.raw_segment_mask_blake3
                == erase.raw_segment_artifact_blake3
            && target.erase_observation.final_erase_mask_blake3
                == erase.final_erase_artifact_blake3,
        "invalid target erase observation",
    )?;
    let observation = &target.erase_observation;
    let page_pixels = u64::from(dimensions.width) * u64::from(dimensions.height);
    let final_mask_pixels = erase.events[8]
        .mask
        .as_ref()
        .ok_or_else(|| "missing InpaintFinal mask".to_string())?
        .nonzero_pixels;
    require(
        observation.target_mask_pixels > 0
            && observation.target_mask_pixels <= page_pixels
            && observation
                .final_erase_overlap_pixels
                .checked_add(observation.source_ink_outside_final_mask_pixels)
                == Some(observation.target_mask_pixels)
            && observation.final_erase_overlap_pixels <= final_mask_pixels
            && observation.protected_overlap_pixels <= final_mask_pixels
            && observation.clean_residual_pixels <= observation.target_mask_pixels,
        "invalid target erase pixel counts",
    )?;
    let observed_terminal = if observation.sprite_present {
        ObservedTerminal::Rendered
    } else if observation.source_rendered_roi_equal {
        ObservedTerminal::PreservedWithoutSprite
    } else if observation.final_erase_overlap_pixels > 0 && observation.clean_residual_pixels == 0 {
        ObservedTerminal::ErasedWithoutSprite
    } else {
        ObservedTerminal::MutatedWithoutSprite
    };
    require(
        target.observed_terminal == observed_terminal,
        "observed terminal mismatch",
    )?;
    validate_source_gate(target, dimensions)?;
    if let Some(renderer) = &target.renderer_diagnostic {
        require(
            renderer.node_id == target.node_id
                && renderer.sprite_width > 0
                && renderer.sprite_height > 0
                && is_blake3(&renderer.alpha_blake3),
            "invalid renderer diagnostic",
        )?;
    }
    require(
        observation.sprite_present == target.renderer_diagnostic.is_some(),
        "terminal sprite state mismatch",
    )?;
    if let Some(typography) = &target.typography_diagnostic {
        require(
            typography.node_id == target.node_id,
            "typography NodeId mismatch",
        )?;
    }
    Ok(())
}

fn validate_source_gate(target: &TargetReport, dimensions: Dimensions) -> D0Result<()> {
    let facts = &target.source_gate;
    let [x, y, width, height] = facts.anchor_bbox;
    require(
        [x, y, width, height].into_iter().all(f32::is_finite)
            && x >= 0.0
            && y >= 0.0
            && width > 0.0
            && height > 0.0
            && x + width <= dimensions.width as f32
            && y + height <= dimensions.height as f32,
        "invalid source gate anchor bbox",
    )?;
    let [left, top, right, bottom] = facts.crop_bounds;
    require(
        left < right
            && top < bottom
            && right <= dimensions.width
            && bottom <= dimensions.height
            && is_blake3(&facts.crop_rgba_blake3),
        "invalid source gate crop",
    )?;
    require(
        facts.pp_word_count > 0
            && facts.pp_character_count > 0
            && facts.pp_line_count > 0
            && facts.vl_contains_han
            && facts.vl_character_count > 0
            && facts.vl_line_count > 0
            && target.ocr_line_count == facts.vl_line_count,
        "invalid source gate counts",
    )?;
    let accepted = matches!(
        facts.terminal_class,
        SourceGateTerminalClass::AcceptedPrimary | SourceGateTerminalClass::AcceptedFallback
    );
    require(facts.selected && accepted, "source gate selected mismatch")?;
    require(
        matches!(
            (facts.terminal_class, facts.fallback),
            (
                SourceGateTerminalClass::AcceptedFallback,
                SourceGateFallback::IsolatedProtectedLatinGeometry
            ) | (_, SourceGateFallback::None)
        ),
        "source gate fallback mismatch",
    )
}

fn validate_runtime(
    report: &GuardedBaselineReport,
    runtime: &[RuntimeLineEvidence<'_>],
) -> D0Result<()> {
    let evidence = runtime
        .iter()
        .map(|item| (item.target_correlation_id, item))
        .collect::<HashMap<_, _>>();
    let target_count = report
        .entries
        .iter()
        .map(|entry| entry.targets.len())
        .sum::<usize>();
    require(
        evidence.len() == runtime.len() && evidence.len() == target_count,
        "runtime evidence cardinality mismatch",
    )?;
    for target in report.entries.iter().flat_map(|entry| &entry.targets) {
        let item = evidence
            .get(target.target_correlation_id.as_str())
            .ok_or_else(|| "runtime evidence correlation mismatch".to_string())?;
        require(
            nonempty_lines(item.raw_source) == target.source_line_count
                && nonempty_lines(item.raw_translated) == target.translated_line_count
                && item.raw_ocr_line_count == target.ocr_line_count
                && target.ocr_line_count == target.source_gate.vl_line_count,
            "runtime line evidence mismatch",
        )?;
    }
    Ok(())
}

fn validate_descriptor(value: &FileDescriptor) -> D0Result<()> {
    require(
        is_sha256(&value.sha256)
            && value.dev >= 0
            && value.ino > 0
            && value.file_type == DescriptorFileType::RegularFile
            && value.mode > 0,
        "invalid regular-file descriptor",
    )
}

fn validate_image_identity(value: &ImageIdentity) -> D0Result<()> {
    validate_descriptor(&value.descriptor)?;
    validate_dimensions(value.dimensions)?;
    require(
        is_blake3(&value.decoded_rgba_blake3),
        "invalid decoded RGBA identity",
    )
}

fn validate_dimensions(value: Dimensions) -> D0Result<()> {
    require(value.width > 0 && value.height > 0, "invalid dimensions")
}

fn nonempty_lines(value: &str) -> u32 {
    value.lines().filter(|line| !line.trim().is_empty()).count() as u32
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut value, byte| {
            write!(value, "{byte:02x}").unwrap();
            value
        })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    is_lower_hex_64(value)
}

fn is_blake3(value: &str) -> bool {
    is_lower_hex_64(value)
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_git_head(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_correlation_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= TOKEN_LIMIT
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'+' | b'-')
        })
}

fn require(condition: bool, message: &'static str) -> D0Result<()> {
    condition.then_some(()).ok_or_else(|| message.into())
}

impl From<EntryRole> for ReportEntryRole {
    fn from(value: EntryRole) -> Self {
        match value {
            EntryRole::Regression => Self::Regression,
            EntryRole::Calibration => Self::Calibration,
            EntryRole::Holdout => Self::Holdout,
        }
    }
}

impl From<Expected> for ReportExpected {
    fn from(value: Expected) -> Self {
        match value {
            Expected::AutomaticStrict => Self::AutomaticStrict,
            Expected::ManualOverride => Self::ManualOverride,
            Expected::UnsupportedSourceColor => Self::UnsupportedSourceColor,
            Expected::UnsupportedRotation => Self::UnsupportedRotation,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::Path;

    use image::{DynamicImage, GrayImage, ImageFormat, Luma, Rgba, RgbaImage};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;
    use crate::pipeline::d0_held_input::HeldInput;
    use crate::pipeline::d0_visual_manifest_pixels::canonical_decoded_rgba_blake3;
    use crate::pipeline::d0_visual_manifest_schema::HeldVisualManifestEntry;
    use crate::pipeline::engines::support::{EraseDiagnosticBranch, EraseMaskDiagnostic};

    fn hash(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn descriptor(byte: u8) -> FileDescriptor {
        FileDescriptor {
            sha256: hash(byte),
            dev: 1,
            ino: u64::from(byte) + 1,
            file_type: DescriptorFileType::RegularFile,
            owner: 501,
            mode: 0o100600,
        }
    }

    fn held() -> (TempDir, HeldVisualManifestSchema) {
        held_with_manifest(b"held-manifest")
    }

    fn held_with_manifest(manifest_bytes: &[u8]) -> (TempDir, HeldVisualManifestSchema) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("held.bin");
        std::fs::write(&path, manifest_bytes).unwrap();
        let canonical = std::fs::canonicalize(path).unwrap();
        let dimensions = Dimensions {
            width: 100,
            height: 80,
        };
        let artifacts = artifact_bytes(dimensions);
        let source_content = inspect_png_artifact(&artifacts[0], ArtifactKind::Source)
            .unwrap()
            .1;
        let mut entries = Vec::new();
        for index in 0..9 {
            let role = match index {
                0 => "regression",
                1..=4 => "calibration",
                _ => "holdout",
            };
            entries.push(json!({
                "id": format!("entry-{index}"), "path": "/held/source", "sha256": hash(1),
                "decoded_rgba_blake3": source_content, "clean_reference_path": "/held/clean",
                "clean_reference_sha256": hash(3),
                "clean_reference_decoded_rgba_blake3": hash(4), "role": role,
                "dimension_bin": "lt720", "aspect": "square_or_near", "background": "pure",
                "targets": [{
                    "id": "shared-target", "source_roi": [0, 0, 1, 1],
                    "clean_reference_edit_roi": [0, 0, 1, 1],
                    "erase_source_ink_mask_path": "/held/erase",
                    "erase_source_ink_mask_sha256": hash(5),
                    "residual_source_ink_mask_path": "/held/residual",
                    "residual_source_ink_mask_sha256": hash(6), "position": "interior",
                    "writing": "horizontal", "effect": "plain", "translation_length": "equal",
                    "expected": "automatic_strict"
                }], "protected_rois": [], "multi_node": false
            }));
        }
        (
            temp,
            HeldVisualManifestSchema {
                schema: serde_json::from_value(json!({"version": 1, "entries": entries})).unwrap(),
                manifest: HeldInput::open(Path::new(&canonical)).unwrap(),
                entries: Vec::<HeldVisualManifestEntry>::new(),
            },
        )
    }

    fn deterministic_map(held: &HeldVisualManifestSchema) -> TargetCorrelationMap {
        let mut next = 1_u128;
        generate_target_correlation_map_with(held, |bytes| {
            bytes.copy_from_slice(&next.to_be_bytes());
            next += 1;
            Ok(())
        })
        .unwrap()
    }

    fn identity(
        descriptor_byte: u8,
        decoded_rgba_blake3: String,
        dimensions: Dimensions,
    ) -> ImageIdentity {
        ImageIdentity {
            descriptor: descriptor(descriptor_byte),
            decoded_rgba_blake3,
            dimensions,
        }
    }

    fn artifact(kind: ArtifactKind, bytes: &[u8], dimensions: Dimensions) -> Artifact {
        Artifact {
            kind,
            dimensions,
            encoded_sha256: sha256_hex(bytes),
            content_blake3: inspect_png_artifact(bytes, kind).unwrap().1,
            byte_count: bytes.len() as u64,
        }
    }

    fn encode_png(image: DynamicImage) -> Vec<u8> {
        encode_image(image, ImageFormat::Png)
    }

    fn encode_image(image: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    fn artifact_bytes(dimensions: Dimensions) -> [Vec<u8>; 5] {
        let rgba = |pixel| RgbaImage::from_pixel(dimensions.width, dimensions.height, Rgba(pixel));
        let mask = |nonzero: u64| {
            let mut image =
                GrayImage::from_pixel(dimensions.width, dimensions.height, Luma([0_u8]));
            for index in 0..nonzero {
                let index = index as u32;
                image.put_pixel(
                    index % dimensions.width,
                    index / dimensions.width,
                    Luma([255]),
                );
            }
            image
        };
        [
            encode_png(DynamicImage::ImageRgba8(rgba([1, 2, 3, 255]))),
            encode_png(DynamicImage::ImageLuma8(mask(3))),
            encode_png(DynamicImage::ImageLuma8(mask(8))),
            encode_png(DynamicImage::ImageRgba8(rgba([4, 5, 6, 255]))),
            encode_png(DynamicImage::ImageRgba8(rgba([7, 8, 9, 255]))),
        ]
    }

    fn page_erase(dimensions: Dimensions, artifact_bytes: &[Vec<u8>; 5]) -> PageEraseDiagnostics {
        use EraseDiagnosticStage::*;
        let raw_segment = inspect_png_artifact(&artifact_bytes[1], ArtifactKind::RawSegmentMask)
            .unwrap()
            .1;
        let final_erase = inspect_png_artifact(&artifact_bytes[2], ArtifactKind::FinalEraseMask)
            .unwrap()
            .1;
        let inpainted = inspect_png_artifact(&artifact_bytes[3], ArtifactKind::Inpainted)
            .unwrap()
            .1;
        let stages = [
            SegmentProbability,
            SegmentRefined,
            SegmentAllowedSupport,
            SegmentFinal,
            InpaintInputSegment,
            InpaintAllowedSupport,
            InpaintPreExpandFiltered,
            InpaintBackendExpanded,
            InpaintFinal,
        ];
        let events = stages
            .into_iter()
            .enumerate()
            .map(|(index, stage)| {
                let content = match index {
                    3 => raw_segment.clone(),
                    8 => final_erase.clone(),
                    _ => hash(40 + index as u8),
                };
                EraseDiagnosticEvent {
                    stage,
                    branch: EraseDiagnosticBranch::HanOnly,
                    mask: Some(EraseMaskDiagnostic {
                        width: dimensions.width,
                        height: dimensions.height,
                        grayscale_blake3: content,
                        nonzero_pixels: index as u64,
                    }),
                    returns_some: (index == 8).then_some(true),
                }
            })
            .collect();
        PageEraseDiagnostics {
            raw_segment_artifact_blake3: raw_segment,
            final_erase_artifact_blake3: final_erase,
            inpainted_artifact_blake3: inpainted,
            events,
        }
    }

    fn report(
        held: &HeldVisualManifestSchema,
        map: &TargetCorrelationMap,
        artifact_bytes: &[Vec<u8>; 5],
    ) -> GuardedBaselineReport {
        let manifest_hash = hex(&held.manifest.sha256());
        let entries = held
            .schema
            .entries
            .iter()
            .enumerate()
            .map(|(index, schema_entry)| {
                let correlation = map
                    .correlation_id(&schema_entry.id, &schema_entry.targets[0].id)
                    .unwrap()
                    .to_owned();
                let dimensions = Dimensions {
                    width: 100,
                    height: 80,
                };
                EntryReport {
                    index: index as u32,
                    entry_id: schema_entry.id.clone(),
                    role: schema_entry.role.into(),
                    source: identity(
                        1,
                        inspect_png_artifact(&artifact_bytes[0], ArtifactKind::Source)
                            .unwrap()
                            .1,
                        dimensions,
                    ),
                    clean: identity(3, hash(4), dimensions),
                    artifacts: [
                        ArtifactKind::Source,
                        ArtifactKind::RawSegmentMask,
                        ArtifactKind::FinalEraseMask,
                        ArtifactKind::Inpainted,
                        ArtifactKind::Rendered,
                    ]
                    .into_iter()
                    .zip(artifact_bytes)
                    .map(|(kind, bytes)| artifact(kind, bytes, dimensions))
                    .collect(),
                    warning_count: 1,
                    warnings: vec![Warning {
                        step_id: "render".into(),
                        message_sha256: hash(36),
                    }],
                    erase: page_erase(dimensions, artifact_bytes),
                    targets: vec![TargetReport {
                        target_correlation_id: correlation,
                        node_id: serde_json::from_value(json!(
                            "00000000-0000-0000-0000-000000000001"
                        ))
                        .unwrap(),
                        source_line_count: 2,
                        ocr_line_count: 2,
                        translated_line_count: 2,
                        expected: schema_entry.targets[0].expected.into(),
                        observed_terminal: ObservedTerminal::MutatedWithoutSprite,
                        expectation_relation: ExpectationRelation::DifferentTerminalClass,
                        source_gate: SourceGateFacts {
                            candidate_index: 0,
                            anchor_bbox: [90.0, 2.0, 10.0, 70.0],
                            crop_bounds: [1, 2, 90, 70],
                            crop_rgba_blake3: hash(37),
                            pp_word_count: 2,
                            pp_character_count: 8,
                            pp_line_count: 2,
                            vl_contains_han: true,
                            vl_character_count: 8,
                            vl_line_count: 2,
                            selected: true,
                            terminal_class: SourceGateTerminalClass::AcceptedPrimary,
                            fallback: SourceGateFallback::None,
                        },
                        erase_observation: TargetEraseObservation {
                            raw_segment_mask_blake3: inspect_png_artifact(
                                &artifact_bytes[1],
                                ArtifactKind::RawSegmentMask,
                            )
                            .unwrap()
                            .1,
                            final_erase_mask_blake3: inspect_png_artifact(
                                &artifact_bytes[2],
                                ArtifactKind::FinalEraseMask,
                            )
                            .unwrap()
                            .1,
                            target_mask_pixels: 9,
                            final_erase_overlap_pixels: 6,
                            source_ink_outside_final_mask_pixels: 3,
                            protected_overlap_pixels: 2,
                            clean_residual_pixels: 1,
                            source_inpainted_roi_equal: false,
                            source_rendered_roi_equal: false,
                            sprite_present: false,
                        },
                        erase_source_ink_mask: descriptor(17),
                        residual_source_ink_mask: descriptor(18),
                        renderer_diagnostic: None,
                        typography_diagnostic: None,
                    }],
                }
            })
            .collect();
        GuardedBaselineReport::new(
            map,
            Provenance {
                git_head: "5".repeat(40),
                executable_sha256: hash(21),
                ledger_sha256: hash(22),
                manifest_sha256: manifest_hash.clone(),
                fixture_manifest_sha256: hash(23),
                app_config_sha256: hash(24),
                translation_provider_id: "translation-provider".into(),
                translation_model_id: "translation-model".into(),
                typography_provider_id: "typography-provider".into(),
                typography_model_id: "typography-model".into(),
                target_language: "zh-CN".into(),
                requested_backend_id: "requested".into(),
                observed_backend_id: "observed".into(),
                engines: EngineIds {
                    detector: "detector-v1".into(),
                    source_gate: "source-gate-v1".into(),
                    font_detector: "font-detector-v1".into(),
                    segmenter: "segmenter-v1".into(),
                    bubble_segmenter: "bubble-segmenter-v1".into(),
                    translator: "translator-v1".into(),
                    typography_planner: "typography-planner-v1".into(),
                    inpainter: "inpainter-v1".into(),
                    renderer: "renderer-v1".into(),
                },
                image_input_contract: IMAGE_INPUT_CONTRACT.into(),
            },
            DescriptorSnapshot {
                ledger: descriptor(22),
                visual_manifest: FileDescriptor {
                    sha256: manifest_hash,
                    ..descriptor(25)
                },
                fixture_manifest: descriptor(23),
                app_config: descriptor(24),
            },
            entries,
        )
        .unwrap()
    }

    fn runtime<'a>(map: &'a TargetCorrelationMap) -> Vec<RuntimeLineEvidence<'a>> {
        map.records
            .iter()
            .map(|record| RuntimeLineEvidence {
                target_correlation_id: &record.target_correlation_id,
                raw_source: "PRIVATE_SOURCE_A\nPRIVATE_SOURCE_B",
                raw_translated: "PRIVATE_TRANSLATED_A\nPRIVATE_TRANSLATED_B",
                raw_ocr_line_count: 2,
            })
            .collect()
    }

    fn runtime_artifacts<'a>(
        report: &'a GuardedBaselineReport,
        bytes: &'a [Vec<u8>; 5],
    ) -> Vec<RuntimeArtifactEvidence<'a>> {
        report
            .entries
            .iter()
            .map(|entry| RuntimeArtifactEvidence {
                entry_id: &entry.entry_id,
                bytes: std::array::from_fn(|index| bytes[index].as_slice()),
            })
            .collect()
    }

    fn json_bytes(value: &Value) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(value).unwrap();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn d0_guarded_baseline_correlation_ids_retry_exhaust_and_ignore_content() {
        let (_temp, held) = held();
        let mut calls = 0;
        let map = generate_target_correlation_map_with(&held, |bytes| {
            calls += 1;
            bytes.fill(if calls <= 2 { 0 } else { calls as u8 });
            Ok(())
        })
        .unwrap();
        assert!(calls > map.records.len());
        assert!(
            map.records
                .iter()
                .all(|record| is_correlation_id(&record.target_correlation_id))
        );
        assert!(
            generate_target_correlation_map_with(&held, |bytes| {
                bytes.fill(0);
                Ok(())
            })
            .is_err()
        );
        let first = deterministic_map(&held);
        let mut byte = 200_u8;
        let second = generate_target_correlation_map_with(&held, |bytes| {
            bytes.fill(byte);
            byte = byte.wrapping_add(1);
            Ok(())
        })
        .unwrap();
        assert_ne!(first.records, second.records);
        assert_eq!(first.manifest_sha256, second.manifest_sha256);

        let (_other_temp, other_held) = held_with_manifest(b"different-held-manifest");
        let other = deterministic_map(&other_held);
        assert_eq!(
            first
                .records
                .iter()
                .map(|record| &record.target_correlation_id)
                .collect::<Vec<_>>(),
            other
                .records
                .iter()
                .map(|record| &record.target_correlation_id)
                .collect::<Vec<_>>(),
            "the same random stream must produce the same IDs for different content"
        );
        assert_ne!(first.manifest_sha256, other.manifest_sha256);
    }

    #[test]
    fn d0_guarded_baseline_map_accepts_entry_local_target_ids_and_is_canonical_closed() {
        let (_temp, held) = held();
        let map = deterministic_map(&held);
        let bytes = canonical_target_correlation_map_bytes(&map).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert_eq!(parse_target_correlation_map(&held, &bytes).unwrap(), map);
        assert_eq!(
            map.records
                .iter()
                .map(|record| &record.target_id)
                .collect::<HashSet<_>>()
                .len(),
            1
        );
        assert!(map.correlation_id("entry-8", "shared-target").is_some());

        let mut pretty = serde_json::to_vec_pretty(&map).unwrap();
        pretty.push(b'\n');
        assert!(parse_target_correlation_map(&held, &pretty).is_err());
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        value["future"] = true.into();
        assert!(parse_target_correlation_map(&held, &json_bytes(&value)).is_err());
        value.as_object_mut().unwrap().remove("future");
        value["records"][0]["target_correlation_id"] = "ABC".into();
        assert!(parse_target_correlation_map(&held, &json_bytes(&value)).is_err());
        value["records"][0]["target_correlation_id"] =
            map.records[0].target_correlation_id.clone().into();
        value.as_object_mut().unwrap().remove("manifest_sha256");
        assert!(parse_target_correlation_map(&held, &json_bytes(&value)).is_err());
    }

    #[test]
    fn d0_guarded_baseline_report_is_closed_canonical_minimal_and_private() {
        let (_temp, held) = held();
        let map = deterministic_map(&held);
        let runtime = runtime(&map);
        let artifact_bytes = artifact_bytes(Dimensions {
            width: 100,
            height: 80,
        });
        let report = report(&held, &map, &artifact_bytes);
        let runtime_artifacts = runtime_artifacts(&report, &artifact_bytes);
        let bytes = canonical_guarded_baseline_report_bytes(
            &held,
            &map,
            &report,
            &runtime,
            &runtime_artifacts,
        )
        .unwrap();
        parse_guarded_baseline_report(&held, &bytes, &map, &runtime, &runtime_artifacts).unwrap();
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.artifacts.len() == 5)
        );
        assert_eq!(
            report
                .entries
                .iter()
                .map(|entry| entry.targets[0].node_id)
                .collect::<HashSet<_>>()
                .len(),
            1,
            "NodeId uniqueness is entry-local"
        );
        let map_bytes = canonical_target_correlation_map_bytes(&map).unwrap();
        assert_eq!(report.target_correlation_map_sha256, sha256_hex(&map_bytes));
        assert_ne!(
            report.target_correlation_map_sha256,
            sha256_hex(&map_bytes[..map_bytes.len() - 1])
        );
        assert_ne!(
            report.entries[0].source.descriptor.sha256,
            report.entries[0].artifacts[0].encoded_sha256
        );
        let observed = &report.entries[0].targets[0].erase_observation;
        assert!(
            observed.source_ink_outside_final_mask_pixels > 0
                && observed.protected_overlap_pixels > 0
                && !observed.source_inpainted_roi_equal
                && !observed.source_rendered_roi_equal
                && !observed.sprite_present
        );
        let text = std::str::from_utf8(&bytes).unwrap();
        for sentinel in [
            "PRIVATE_SOURCE_A",
            "PRIVATE_TRANSLATED_A",
            "font_family",
            "font_path",
            "filesystem_path",
            "elapsed",
            "timestamp",
            "GREEN",
        ] {
            assert!(!text.contains(sentinel), "{sentinel}");
        }

        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        value["green_verdict"] = "GREEN".into();
        assert!(
            parse_guarded_baseline_report(
                &held,
                &json_bytes(&value),
                &map,
                &runtime,
                &runtime_artifacts,
            )
            .is_err()
        );
        value.as_object_mut().unwrap().remove("green_verdict");
        value["entries"][0]["future_t6"] = true.into();
        assert!(
            parse_guarded_baseline_report(
                &held,
                &json_bytes(&value),
                &map,
                &runtime,
                &runtime_artifacts,
            )
            .is_err()
        );
    }

    #[test]
    fn d0_guarded_baseline_report_rejects_identity_count_and_dimension_drift() {
        let (_temp, held) = held();
        let map = deterministic_map(&held);
        let runtime = runtime(&map);
        let artifact_bytes = artifact_bytes(Dimensions {
            width: 100,
            height: 80,
        });
        let report = report(&held, &map, &artifact_bytes);
        let runtime_artifacts = runtime_artifacts(&report, &artifact_bytes);
        let bytes = canonical_guarded_baseline_report_bytes(
            &held,
            &map,
            &report,
            &runtime,
            &runtime_artifacts,
        )
        .unwrap();
        let baseline: Value = serde_json::from_slice(&bytes).unwrap();
        for (pointer, replacement) in [
            ("/schema", json!("future-v2")),
            ("/target_correlation_map_sha256", json!(hash(99))),
            ("/provenance/executable_sha256", json!("ABC")),
            ("/provenance/engines/renderer", json!("detector-v1")),
            ("/entries/0/targets/0/source_line_count", json!(0)),
            ("/entries/0/targets/0/source_gate/vl_line_count", json!(1)),
            ("/entries/0/targets/0/source_gate/anchor_bbox/2", json!(0.0)),
            ("/entries/0/targets/0/source_gate/crop_bounds/2", json!(1)),
            (
                "/entries/0/targets/0/erase_observation/final_erase_overlap_pixels",
                json!(9),
            ),
            (
                "/entries/0/targets/0/expectation_relation",
                json!("same_terminal_class"),
            ),
            (
                "/entries/0/targets/0/observed_terminal",
                json!("preserved_without_sprite"),
            ),
            (
                "/entries/0/targets/0/observed_terminal",
                json!("erased_without_sprite"),
            ),
            (
                "/entries/0/targets/0/erase_observation/source_rendered_roi_equal",
                json!(true),
            ),
            ("/entries/0/artifacts/0/content_blake3", json!(hash(99))),
            (
                "/entries/0/erase/events/3/mask/grayscale_blake3",
                json!(hash(99)),
            ),
            ("/entries/0/artifacts/1/dimensions/width", json!(99)),
            ("/entries/1/entry_id", json!("entry-0")),
        ] {
            let mut value = baseline.clone();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                parse_guarded_baseline_report(
                    &held,
                    &json_bytes(&value),
                    &map,
                    &runtime,
                    &runtime_artifacts,
                )
                .is_err(),
                "{pointer}"
            );
        }
        let mut missing = baseline;
        missing["entries"][0]
            .as_object_mut()
            .unwrap()
            .remove("clean");
        assert!(
            parse_guarded_baseline_report(
                &held,
                &json_bytes(&missing),
                &map,
                &runtime,
                &runtime_artifacts,
            )
            .is_err()
        );
    }

    #[test]
    fn d0_guarded_baseline_binds_actual_png_artifacts_and_input_decoded_identity() {
        let (_temp, held) = held();
        let map = deterministic_map(&held);
        let runtime = runtime(&map);
        let dimensions = Dimensions {
            width: 100,
            height: 80,
        };
        let artifact_bytes = artifact_bytes(dimensions);
        let report = report(&held, &map, &artifact_bytes);

        let mut non_png = artifact_bytes.clone();
        non_png[0] = b"not-a-png".to_vec();
        let mut forged = report.clone();
        forged.entries[0].artifacts[0].encoded_sha256 = sha256_hex(&non_png[0]);
        forged.entries[0].artifacts[0].byte_count = non_png[0].len() as u64;
        let non_png_runtime = runtime_artifacts(&forged, &non_png);
        assert!(
            canonical_guarded_baseline_report_bytes(
                &held,
                &map,
                &forged,
                &runtime,
                &non_png_runtime,
            )
            .is_err()
        );

        let source = RgbaImage::from_fn(8, 6, |x, y| {
            Rgba([(x * 17) as u8, (y * 23) as u8, ((x + y) * 11) as u8, 255])
        });
        for format in [ImageFormat::Jpeg, ImageFormat::WebP] {
            let encoded_input = encode_image(DynamicImage::ImageRgba8(source.clone()), format);
            let decoded = image::load_from_memory(&encoded_input)
                .unwrap()
                .into_rgba8();
            let source_png = encode_png(DynamicImage::ImageRgba8(decoded));
            assert_eq!(
                canonical_decoded_rgba_blake3(&encoded_input).unwrap(),
                inspect_png_artifact(&source_png, ArtifactKind::Source)
                    .unwrap()
                    .1,
                "{format:?}"
            );
            assert!(inspect_png_artifact(&encoded_input, ArtifactKind::Source).is_err());
        }
    }

    #[test]
    fn d0_guarded_baseline_recomputes_runtime_counts_and_rejects_drift() {
        let (_temp, held) = held();
        let map = deterministic_map(&held);
        let artifact_bytes = artifact_bytes(Dimensions {
            width: 100,
            height: 80,
        });
        let report = report(&held, &map, &artifact_bytes);
        let runtime_artifacts = runtime_artifacts(&report, &artifact_bytes);
        let mut runtime = runtime(&map);
        assert!(
            canonical_guarded_baseline_report_bytes(
                &held,
                &map,
                &report,
                &runtime,
                &runtime_artifacts,
            )
            .is_ok()
        );
        runtime[0].raw_source = "one";
        assert!(
            canonical_guarded_baseline_report_bytes(
                &held,
                &map,
                &report,
                &runtime,
                &runtime_artifacts,
            )
            .is_err()
        );
        runtime[0].raw_source = "one\ntwo";
        runtime[0].raw_ocr_line_count = 0;
        assert!(
            canonical_guarded_baseline_report_bytes(
                &held,
                &map,
                &report,
                &runtime,
                &runtime_artifacts,
            )
            .is_err()
        );
        runtime[0].raw_ocr_line_count = 2;
        runtime[0].target_correlation_id = &map.records[1].target_correlation_id;
        assert!(
            canonical_guarded_baseline_report_bytes(
                &held,
                &map,
                &report,
                &runtime,
                &runtime_artifacts,
            )
            .is_err()
        );
    }

    #[test]
    fn d0_guarded_baseline_os_random_smoke_has_shape_and_unique_ids() {
        let (_temp, held) = held();
        let map = generate_target_correlation_map(&held).unwrap();
        assert_eq!(map.records.len(), 9);
        assert_eq!(
            map.records
                .iter()
                .map(|record| &record.target_correlation_id)
                .collect::<HashSet<_>>()
                .len(),
            9
        );
    }
}
