use std::{collections::HashSet, sync::Mutex};

#[cfg(test)]
use std::sync::{Arc, Condvar, OnceLock};

use anyhow::{Result, ensure};
use async_trait::async_trait;
use image::DynamicImage;
use koharu_core::{
    ImageRole, MaskRole, Node, NodeDataPatch, NodeId, NodeKind, NodePatch, Op, PageId, Scene,
    TextData, TextDataPatch, Transform,
};
use koharu_llm::paddleocr_vl::{PaddleOcrVl, PaddleOcrVlTask};
use koharu_ml::pp_ocr_v5::{PpOcrV5, PpOcrV5Observation, PpOcrWordBox};
use serde::{Deserialize, Serialize};

use crate::app::shared_llama_backend;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    SOURCE_GATE_PROTECTED_DETECTOR, SOURCE_GATE_TARGET_DETECTOR, contains_han,
    contains_protected_latin_word, find_mask_node, load_source_image, support_bboxes_overlap,
};

const MIN_WORD_CONFIDENCE: f32 = 0.5;
const MAX_NEW_TOKENS: usize = 256;

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ValidatedWord {
    pub(super) line_index: usize,
    pub(super) text: String,
    pub(super) bbox: [f32; 4],
    pub(super) protected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SourceTarget {
    pub(super) text: String,
    pub(super) bbox: [f32; 4],
    pub(super) line_polygons: Vec<[[f32; 2]; 4]>,
    pub(super) detector_occurrences: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SourceProtectedLine {
    pub(super) text: String,
    pub(super) bbox: [f32; 4],
    pub(super) line_polygons: Vec<[[f32; 2]; 4]>,
    pub(super) detector_occurrences: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SourceSelection {
    pub(super) targets: Vec<SourceTarget>,
    pub(super) protected_lines: Vec<SourceProtectedLine>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::pipeline) enum SourceGateRejectReason {
    PpNoWords,
    PpNoHanProtectedLatin,
    PpNoHanUnprotected,
    PpNonFiniteConfidence,
    PpLowConfidenceHan,
    PpLowConfidenceNonHan,
    PpVlCharacterMismatch,
    PpVlLineMismatch,
    PpBboxInvalid,
    PpOrderInvalid,
    ProtectedLatinHanConflict,
    PpVlIncompleteCoverage,
    NoSafeHanRun,
    ProtectedGeometryOverlap,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(in crate::pipeline) enum SourceGateDecision {
    InvalidCandidateGeometry,
    RejectedBeforeVl {
        reason: SourceGateRejectReason,
    },
    RejectedAfterVl {
        reason: SourceGateRejectReason,
    },
    AcceptedPrimary {
        target_count: usize,
        protected_count: usize,
    },
    AcceptedDetectorFallback {
        target_count: usize,
        protected_count: usize,
    },
    AcceptedIsolatedProtectedLatinGeometry {
        target_count: usize,
        protected_count: usize,
    },
    VlBatchError,
}

impl SourceGateDecision {
    #[cfg(test)]
    pub(in crate::pipeline) fn pp_calls(&self) -> u8 {
        (!matches!(self, Self::InvalidCandidateGeometry)) as u8
    }

    #[cfg(test)]
    pub(in crate::pipeline) fn vl_calls(&self) -> u8 {
        matches!(
            self,
            Self::RejectedAfterVl { .. }
                | Self::AcceptedPrimary { .. }
                | Self::AcceptedDetectorFallback { .. }
                | Self::AcceptedIsolatedProtectedLatinGeometry { .. }
                | Self::VlBatchError
        ) as u8
    }

    #[cfg(test)]
    pub(in crate::pipeline) fn vl_stage(&self) -> &'static str {
        match self {
            Self::InvalidCandidateGeometry | Self::RejectedBeforeVl { .. } => "not_requested",
            Self::RejectedAfterVl { .. }
            | Self::AcceptedPrimary { .. }
            | Self::AcceptedDetectorFallback { .. }
            | Self::AcceptedIsolatedProtectedLatinGeometry { .. } => "completed",
            Self::VlBatchError => "batch_error",
        }
    }

    #[cfg(test)]
    pub(in crate::pipeline) fn fallback(&self) -> &'static str {
        match self {
            Self::AcceptedDetectorFallback { .. } => "layout_geometry",
            Self::AcceptedIsolatedProtectedLatinGeometry { .. } => {
                "isolated_protected_latin_geometry"
            }
            _ => "none",
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(in crate::pipeline) enum SourceGateDiagnosticEvent {
    Input {
        backend: &'static str,
        width: u32,
        height: u32,
        decoded_rgba_hash: String,
    },
    LayoutCandidate {
        candidate_index: usize,
        node_id: NodeId,
        confidence: f32,
        bbox: [f32; 4],
    },
    Crop {
        candidate_index: usize,
        node_id: NodeId,
        bounds: [u32; 4],
        crop_rgba_hash: String,
        vl_bounds: [u32; 4],
        vl_crop_rgba_hash: String,
    },
    PpSummary {
        node_id: NodeId,
        words: Vec<PpWordDiagnostic>,
        raw_detectors: Vec<PpDetectorDiagnostic>,
        canonical_lines: Vec<PpCanonicalLineDiagnostic>,
    },
    VlSummary {
        node_id: NodeId,
        contains_han: bool,
        han_scalar_count: usize,
        character_count: usize,
        line_count: usize,
    },
    SelectionGeometry {
        node_id: NodeId,
        targets: Vec<SourceGateTargetGeometryDiagnostic>,
        protected_lines: Vec<SourceGateTargetGeometryDiagnostic>,
        detector_ownership: Vec<SourceGateDetectorOwnershipDiagnostic>,
    },
    Decision {
        node_id: NodeId,
        decision: SourceGateDecision,
    },
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
pub(in crate::pipeline) struct SourceGateTargetGeometryDiagnostic {
    pub scene_quad_f32_bits: [u32; 8],
    pub eligible_line_quads_f32_bits: Vec<[u32; 8]>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "ownership", rename_all = "snake_case")]
pub(in crate::pipeline) enum SourceGateDetectorAssignmentDiagnostic {
    Target { target_index: usize },
    Protected { protected_index: usize },
    Unassigned,
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
pub(in crate::pipeline) struct SourceGateDetectorOwnershipDiagnostic {
    pub occurrence_index: usize,
    pub canonical_line_index: Option<usize>,
    pub scene_quad_f32_bits: [u32; 8],
    pub eligible_text_line_quad_f32_bits: Option<[u32; 8]>,
    pub assignment: SourceGateDetectorAssignmentDiagnostic,
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
pub(in crate::pipeline) struct PpWordDiagnostic {
    pub line_index: usize,
    pub han_scalar_count: usize,
    pub character_count: usize,
    pub script: &'static str,
    pub confidence: f32,
    pub bbox: [f32; 4],
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
pub(in crate::pipeline) struct PpDetectorDiagnostic {
    pub occurrence_index: usize,
    pub source_scaled_quad_f32_bits: [u32; 8],
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
pub(in crate::pipeline) struct PpCanonicalOccurrenceDiagnostic {
    pub occurrence_index: usize,
    pub canonical_corners_f32_bits: [u32; 8],
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
pub(in crate::pipeline) struct PpRecognitionDiagnostic {
    pub present: bool,
    pub recognition_class: &'static str,
    pub segment_count: usize,
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
pub(in crate::pipeline) struct PpCanonicalLineDiagnostic {
    pub line_index: usize,
    pub detector_occurrences: Vec<PpCanonicalOccurrenceDiagnostic>,
    pub recognition: Option<PpRecognitionDiagnostic>,
}

#[cfg(test)]
type DiagnosticSink = Arc<Mutex<Vec<SourceGateDiagnosticEvent>>>;

#[cfg(test)]
struct DiagnosticCaptureState {
    owner: std::thread::ThreadId,
    sink: DiagnosticSink,
}

#[cfg(test)]
static DIAGNOSTIC_SINK: OnceLock<(Mutex<Option<DiagnosticCaptureState>>, Condvar)> =
    OnceLock::new();

#[cfg(test)]
pub(in crate::pipeline) struct SourceGateDiagnosticCapture {
    events: DiagnosticSink,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(in crate::pipeline) enum SourceGateCropPolicy {
    C0,
    C1,
    C2,
    C4,
    Q2,
    S25L4,
    S25L5,
    S25L6,
    S25L7,
}

const PRIMARY_CROP_POLICY: SourceGateCropPolicy = SourceGateCropPolicy::S25L4;
const B0_CROP_POLICIES: [SourceGateCropPolicy; 4] = [
    PRIMARY_CROP_POLICY,
    SourceGateCropPolicy::S25L5,
    SourceGateCropPolicy::S25L6,
    SourceGateCropPolicy::S25L7,
];

impl SourceGateCropPolicy {
    #[cfg(test)]
    pub(in crate::pipeline) const fn production() -> Self {
        PRIMARY_CROP_POLICY
    }
}

#[cfg(test)]
static TEST_CROP_POLICY: OnceLock<Mutex<Option<SourceGateCropPolicy>>> = OnceLock::new();

#[cfg(test)]
pub(in crate::pipeline) struct SourceGateCropPolicyGuard {
    previous: Option<SourceGateCropPolicy>,
}

#[cfg(test)]
impl SourceGateCropPolicyGuard {
    pub(in crate::pipeline) fn set(policy: SourceGateCropPolicy) -> Self {
        let mut active = TEST_CROP_POLICY
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("source gate crop policy mutex poisoned");
        let previous = *active;
        *active = Some(policy);
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for SourceGateCropPolicyGuard {
    fn drop(&mut self) {
        *TEST_CROP_POLICY
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("source gate crop policy mutex poisoned") = self.previous;
    }
}

#[cfg(test)]
impl SourceGateDiagnosticCapture {
    pub(in crate::pipeline) fn start() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let owner = std::thread::current().id();
        let (sink, available) = DIAGNOSTIC_SINK.get_or_init(|| (Mutex::new(None), Condvar::new()));
        let mut active = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Some(state) = active.as_ref() {
            assert_ne!(
                state.owner, owner,
                "source gate diagnostic capture already active"
            );
            active = available
                .wait(active)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        *active = Some(DiagnosticCaptureState {
            owner,
            sink: events.clone(),
        });
        Self { events }
    }

    pub(in crate::pipeline) fn take(&self) -> Vec<SourceGateDiagnosticEvent> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

#[cfg(test)]
impl Drop for SourceGateDiagnosticCapture {
    fn drop(&mut self) {
        let (sink, available) = DIAGNOSTIC_SINK.get_or_init(|| (Mutex::new(None), Condvar::new()));
        let mut active = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if active
            .as_ref()
            .is_some_and(|state| Arc::ptr_eq(&state.sink, &self.events))
        {
            *active = None;
            available.notify_one();
        }
    }
}

#[cfg(test)]
fn record_diagnostic(event: SourceGateDiagnosticEvent) {
    let sink = DIAGNOSTIC_SINK
        .get_or_init(|| (Mutex::new(None), Condvar::new()))
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .filter(|state| state.owner == std::thread::current().id())
        .map(|state| state.sink.clone());
    if let Some(sink) = sink {
        sink.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
    }
}

#[cfg(test)]
fn diagnostic_corner_bits(corners: [[f32; 2]; 4]) -> [u32; 8] {
    let mut bits = [0; 8];
    for (index, [x, y]) in corners.into_iter().enumerate() {
        bits[index * 2] = x.to_bits();
        bits[index * 2 + 1] = y.to_bits();
    }
    bits
}

#[cfg(test)]
fn diagnostic_recognition_class(text: &str) -> &'static str {
    let has_han = contains_han(text);
    let has_latin = contains_latin_alphabetic(text);
    match (has_han, has_latin, contains_protected_latin_word(text)) {
        (true, true, _) => "ambiguous_latin",
        (true, false, _) => "han",
        (false, _, true) => "protected_latin",
        (false, true, false) => "ambiguous_latin",
        (false, false, false) => "neutral",
    }
}

#[cfg(test)]
fn diagnostic_observation(
    observation: &PpOcrV5Observation,
) -> (Vec<PpDetectorDiagnostic>, Vec<PpCanonicalLineDiagnostic>) {
    let raw_detectors = observation
        .detectors
        .iter()
        .map(|detector| PpDetectorDiagnostic {
            occurrence_index: detector.occurrence_index,
            source_scaled_quad_f32_bits: diagnostic_corner_bits(detector.corners),
        })
        .collect();
    let canonical_lines = observation
        .lines
        .iter()
        .enumerate()
        .map(|(line_index, line)| PpCanonicalLineDiagnostic {
            line_index,
            detector_occurrences: line
                .detector_indices
                .iter()
                .map(|occurrence_index| PpCanonicalOccurrenceDiagnostic {
                    occurrence_index: *occurrence_index,
                    canonical_corners_f32_bits: observation
                        .detectors
                        .get(*occurrence_index)
                        .map_or([0; 8], |detector| diagnostic_corner_bits(detector.corners)),
                })
                .collect(),
            recognition: line
                .recognition
                .as_deref()
                .map(|recognition| PpRecognitionDiagnostic {
                    present: true,
                    recognition_class: diagnostic_recognition_class(recognition),
                    segment_count: observation
                        .word_boxes
                        .iter()
                        .filter(|word| word.line_index == line_index)
                        .count(),
                }),
        })
        .collect();
    (raw_detectors, canonical_lines)
}

#[cfg(test)]
fn diagnostic_selection_geometry(
    node_id: NodeId,
    observation: &PpOcrV5Observation,
    crop_bounds: [u32; 4],
    selection: &SourceSelection,
) -> SourceGateDiagnosticEvent {
    let [crop_left, crop_top, _, _] = crop_bounds;
    let targets = selection
        .targets
        .iter()
        .map(|target| SourceGateTargetGeometryDiagnostic {
            scene_quad_f32_bits: diagnostic_corner_bits(bbox_quad(target.bbox)),
            eligible_line_quads_f32_bits: target
                .line_polygons
                .iter()
                .copied()
                .map(diagnostic_corner_bits)
                .collect(),
        })
        .collect::<Vec<_>>();
    let protected_lines = selection
        .protected_lines
        .iter()
        .map(|line| SourceGateTargetGeometryDiagnostic {
            scene_quad_f32_bits: diagnostic_corner_bits(bbox_quad(line.bbox)),
            eligible_line_quads_f32_bits: line
                .line_polygons
                .iter()
                .copied()
                .map(diagnostic_corner_bits)
                .collect(),
        })
        .collect::<Vec<_>>();
    let detector_ownership = observation
        .detectors
        .iter()
        .map(|detector| {
            let scene_quad = detector
                .corners
                .map(|[x, y]| [crop_left as f32 + x, crop_top as f32 + y]);
            let target_matches = selection
                .targets
                .iter()
                .enumerate()
                .filter_map(|(index, target)| {
                    target
                        .detector_occurrences
                        .iter()
                        .position(|occurrence| *occurrence == detector.occurrence_index)
                        .map(|position| (index, target.line_polygons[position]))
                })
                .collect::<Vec<_>>();
            let protected_matches = selection
                .protected_lines
                .iter()
                .enumerate()
                .filter_map(|(index, line)| {
                    line.detector_occurrences
                        .iter()
                        .position(|occurrence| *occurrence == detector.occurrence_index)
                        .map(|position| (index, line.line_polygons[position]))
                })
                .collect::<Vec<_>>();
            let (assignment, eligible_text_line_quad_f32_bits) =
                match (target_matches.as_slice(), protected_matches.as_slice()) {
                    ([(target_index, line_polygon)], []) => (
                        SourceGateDetectorAssignmentDiagnostic::Target {
                            target_index: *target_index,
                        },
                        Some(diagnostic_corner_bits(*line_polygon)),
                    ),
                    ([], [(protected_index, line_polygon)]) => (
                        SourceGateDetectorAssignmentDiagnostic::Protected {
                            protected_index: *protected_index,
                        },
                        Some(diagnostic_corner_bits(*line_polygon)),
                    ),
                    _ => (SourceGateDetectorAssignmentDiagnostic::Unassigned, None),
                };
            let canonical_line_index = observation
                .lines
                .iter()
                .position(|line| line.detector_indices.contains(&detector.occurrence_index));
            SourceGateDetectorOwnershipDiagnostic {
                occurrence_index: detector.occurrence_index,
                canonical_line_index,
                scene_quad_f32_bits: diagnostic_corner_bits(scene_quad),
                eligible_text_line_quad_f32_bits,
                assignment,
            }
        })
        .collect();
    SourceGateDiagnosticEvent::SelectionGeometry {
        node_id,
        targets,
        protected_lines,
        detector_ownership,
    }
}

fn classify_pp_words(words: &[PpOcrWordBox]) -> Result<(), SourceGateRejectReason> {
    if words.is_empty() {
        return Err(SourceGateRejectReason::PpNoWords);
    }
    if words.iter().any(|word| !word.confidence.is_finite()) {
        return Err(SourceGateRejectReason::PpNonFiniteConfidence);
    }
    if words
        .iter()
        .any(|word| !contains_han(&word.text) && word.confidence < MIN_WORD_CONFIDENCE)
    {
        return Err(SourceGateRejectReason::PpLowConfidenceNonHan);
    }
    if words
        .iter()
        .any(|word| contains_han(&word.text) && word.confidence < MIN_WORD_CONFIDENCE)
    {
        return Err(SourceGateRejectReason::PpLowConfidenceHan);
    }
    let has_han = words.iter().any(|word| contains_han(&word.text));
    if !has_han {
        return Err(
            if words
                .iter()
                .any(|word| contains_protected_latin_word(&word.text))
            {
                SourceGateRejectReason::PpNoHanProtectedLatin
            } else {
                SourceGateRejectReason::PpNoHanUnprotected
            },
        );
    }
    if words
        .iter()
        .any(|word| contains_han(&word.text) && contains_protected_latin_word(&word.text))
    {
        return Err(SourceGateRejectReason::ProtectedLatinHanConflict);
    }
    Ok(())
}

fn contains_latin_alphabetic(text: &str) -> bool {
    use icu_properties::{CodePointMapData, props::Script};

    let scripts = CodePointMapData::<Script>::new();
    text.chars()
        .any(|ch| ch.is_alphabetic() && scripts.get(ch) == Script::Latin)
}

fn protected_latin_tokens(text: &str) -> Vec<String> {
    use icu_properties::{CodePointMapData, props::Script};

    let scripts = CodePointMapData::<Script>::new();
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut letters = 0;
    for character in text.chars().chain(std::iter::once(' ')) {
        if character.is_alphabetic() && scripts.get(character) == Script::Latin {
            token.extend(character.to_lowercase());
            letters += 1;
        } else if matches!(character, '-' | '\'' | '’') && letters > 0 {
            token.push(character);
        } else {
            if letters >= 2 {
                tokens.push(std::mem::take(&mut token));
            } else {
                token.clear();
            }
            letters = 0;
        }
    }
    tokens.sort();
    tokens
}

fn bbox_quad([left, top, right, bottom]: [f32; 4]) -> [[f32; 2]; 4] {
    [[left, top], [right, top], [right, bottom], [left, bottom]]
}

#[cfg(test)]
fn bbox_union(words: &[ValidatedWord], indices: &[usize]) -> Option<[f32; 4]> {
    let first = words.get(*indices.first()?)?.bbox;
    Some(indices.iter().skip(1).fold(first, |mut bbox, index| {
        let item = words[*index].bbox;
        bbox[0] = bbox[0].min(item[0]);
        bbox[1] = bbox[1].min(item[1]);
        bbox[2] = bbox[2].max(item[2]);
        bbox[3] = bbox[3].max(item[3]);
        bbox
    }))
}

fn bboxes_intersect(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3])
}

#[cfg(test)]
fn isolated_latin_scalar_allowed(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '\'')
}

#[cfg(test)]
fn isolated_protected_latin_line_is_safe(
    line_words: &[PpOcrWordBox],
    all_words: &[PpOcrWordBox],
    vl_line: &str,
    vl_chars: &[char],
    crop_width: f32,
    crop_height: f32,
) -> bool {
    if line_words.is_empty()
        || line_words.iter().any(|word| {
            word.text.chars().all(char::is_whitespace)
                || !contains_protected_latin_word(&word.text)
                || contains_han(&word.text)
                || word.bbox[0] <= 0.0
                || word.bbox[1] <= 0.0
                || word.bbox[2] >= crop_width
                || word.bbox[3] >= crop_height
        })
        || contains_han(vl_line)
    {
        return false;
    }

    let pp_line = line_words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if !contains_protected_latin_word(&pp_line) || !contains_protected_latin_word(vl_line) {
        return false;
    }
    let pp_chars = pp_line
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<Vec<_>>();
    if pp_chars.len() != vl_chars.len()
        || pp_chars.len() <= 1
        || pp_chars
            .iter()
            .any(|ch| !isolated_latin_scalar_allowed(*ch))
        || vl_chars
            .iter()
            .any(|ch| !isolated_latin_scalar_allowed(*ch))
    {
        return false;
    }
    let mismatches = pp_chars
        .iter()
        .zip(vl_chars)
        .filter(|(pp, vl)| pp != vl)
        .collect::<Vec<_>>();
    if !matches!(mismatches.as_slice(), [(pp, vl)] if pp.is_ascii_alphabetic() && vl.is_ascii_alphabetic())
    {
        return false;
    }

    let han_bboxes = all_words
        .iter()
        .filter(|word| contains_han(&word.text))
        .map(|word| word.bbox)
        .collect::<Vec<_>>();
    line_words.iter().all(|protected| {
        han_bboxes
            .iter()
            .all(|han| !bboxes_intersect(protected.bbox, *han))
    })
}

#[cfg(test)]
fn validate_pp_vl_alignment(
    vl_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> std::result::Result<Vec<ValidatedWord>, SourceGateRejectReason> {
    validate_pp_vl_alignment_internal(
        vl_text,
        words,
        crop_bounds,
        image_width,
        image_height,
        false,
    )
    .map(|(validated, _)| validated)
}

#[cfg(test)]
fn validate_pp_vl_alignment_internal(
    vl_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
    allow_isolated_protected_latin_geometry: bool,
) -> std::result::Result<(Vec<ValidatedWord>, bool), SourceGateRejectReason> {
    let [crop_left, crop_top, crop_right, crop_bottom] = crop_bounds;
    if crop_left >= crop_right
        || crop_top >= crop_bottom
        || crop_right > image_width
        || crop_bottom > image_height
    {
        return Err(SourceGateRejectReason::PpBboxInvalid);
    }
    classify_pp_words(words)?;

    let crop_width = (crop_right - crop_left) as f32;
    let crop_height = (crop_bottom - crop_top) as f32;
    let mut previous_line = None;
    let mut previous_right = 0.0;

    for word in words {
        let [left, top, right, bottom] = word.bbox;
        if word.bbox.iter().any(|value| !value.is_finite())
            || left < 0.0
            || top < 0.0
            || right > crop_width
            || bottom > crop_height
            || left >= right
            || top >= bottom
        {
            return Err(SourceGateRejectReason::PpBboxInvalid);
        }
        if let Some(line_index) = previous_line
            && (word.line_index < line_index
                || (word.line_index == line_index && left < previous_right))
        {
            return Err(SourceGateRejectReason::PpOrderInvalid);
        }
        previous_line = Some(word.line_index);
        previous_right = right;
    }

    let vl_lines = vl_text
        .lines()
        .filter_map(|line| {
            let chars = line
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<Vec<_>>();
            (!chars.is_empty()).then_some((line, chars))
        })
        .collect::<Vec<_>>();
    let pp_line_count = words
        .iter()
        .map(|word| word.line_index)
        .collect::<HashSet<_>>()
        .len();
    if vl_lines.len() != pp_line_count {
        return Err(SourceGateRejectReason::PpVlLineMismatch);
    }

    let mut validated = Vec::with_capacity(words.len());
    let mut word_start = 0;
    let mut used_isolated_protected_latin_geometry = false;
    for (vl_line, vl_chars) in vl_lines {
        let line_index = words[word_start].line_index;
        let mut word_end = word_start + 1;
        while word_end < words.len() && words[word_end].line_index == line_index {
            word_end += 1;
        }
        let line_words = &words[word_start..word_end];
        let use_isolated_protected_latin_geometry = allow_isolated_protected_latin_geometry
            && isolated_protected_latin_line_is_safe(
                line_words,
                words,
                vl_line,
                &vl_chars,
                crop_width,
                crop_height,
            );
        if use_isolated_protected_latin_geometry && used_isolated_protected_latin_geometry {
            return Err(SourceGateRejectReason::PpVlCharacterMismatch);
        }
        used_isolated_protected_latin_geometry |= use_isolated_protected_latin_geometry;

        let mut vl_offset = 0_usize;
        for word in line_words {
            let pp_chars = word
                .text
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<Vec<_>>();
            let Some(end) = vl_offset.checked_add(pp_chars.len()) else {
                return Err(SourceGateRejectReason::PpVlCharacterMismatch);
            };
            let Some(authoritative) = vl_chars.get(vl_offset..end) else {
                return Err(SourceGateRejectReason::PpVlCharacterMismatch);
            };
            if !use_isolated_protected_latin_geometry {
                let mismatch = pp_chars.iter().zip(authoritative).position(|(pp, vl)| {
                    pp != vl && !(contains_han(&pp.to_string()) && contains_han(&vl.to_string()))
                });
                if let Some(offset) = mismatch {
                    tracing::debug!(
                        target: "koharu::source_gate",
                        line_index = word.line_index,
                        word_offset = vl_offset,
                        scalar_offset = offset,
                        pp_is_han = contains_han(&pp_chars[offset].to_string()),
                        vl_is_han = contains_han(&authoritative[offset].to_string()),
                        pp_is_ascii_alphabetic = pp_chars[offset].is_ascii_alphabetic(),
                        vl_is_ascii_alphabetic = authoritative[offset].is_ascii_alphabetic(),
                        pp_is_ascii_punctuation = pp_chars[offset].is_ascii_punctuation(),
                        vl_is_ascii_punctuation = authoritative[offset].is_ascii_punctuation(),
                        "source_gate.alignment_scalar_mismatch"
                    );
                    return Err(SourceGateRejectReason::PpVlCharacterMismatch);
                }
            }
            if pp_chars.is_empty() {
                return Err(SourceGateRejectReason::PpVlCharacterMismatch);
            }
            vl_offset = end;

            let text = if use_isolated_protected_latin_geometry {
                word.text.clone()
            } else {
                authoritative.iter().collect::<String>()
            };
            let protected = contains_protected_latin_word(&text);
            if protected && contains_han(&text) {
                return Err(SourceGateRejectReason::ProtectedLatinHanConflict);
            }
            let [left, top, right, bottom] = word.bbox;
            validated.push(ValidatedWord {
                line_index: word.line_index,
                text,
                bbox: [
                    crop_left as f32 + left,
                    crop_top as f32 + top,
                    crop_left as f32 + right,
                    crop_top as f32 + bottom,
                ],
                protected,
            });
        }
        if vl_offset != vl_chars.len() {
            tracing::debug!(
                target: "koharu::source_gate",
                line_index,
                covered_scalars = vl_offset,
                authoritative_scalars = vl_chars.len(),
                "source_gate.alignment_incomplete_coverage"
            );
            return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
        }
        word_start = word_end;
    }

    Ok((validated, used_isolated_protected_latin_geometry))
}

#[cfg(test)]
fn select_chinese_target(
    vl_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> std::result::Result<SourceSelection, SourceGateRejectReason> {
    let validated =
        validate_pp_vl_alignment(vl_text, words, crop_bounds, image_width, image_height)?;
    select_validated_chinese_target(validated)
}

#[cfg(test)]
fn select_validated_chinese_target(
    validated: Vec<ValidatedWord>,
) -> std::result::Result<SourceSelection, SourceGateRejectReason> {
    let mut selected = vec![false; validated.len()];
    let mut indexed_targets = Vec::new();
    let mut line_start = 0;

    while line_start < validated.len() {
        let line_index = validated[line_start].line_index;
        let mut line_end = line_start + 1;
        while line_end < validated.len() && validated[line_end].line_index == line_index {
            line_end += 1;
        }
        let line = &validated[line_start..line_end];
        if line.iter().any(|word| contains_han(&word.text)) {
            let target_indices = if line.iter().all(|word| !word.protected) {
                (line_start..line_end).collect::<Vec<_>>()
            } else {
                let mut han_runs = Vec::new();
                let mut run_start = line_start;
                while run_start < line_end {
                    while run_start < line_end && validated[run_start].protected {
                        run_start += 1;
                    }
                    if run_start == line_end {
                        break;
                    }
                    let mut run_end = run_start + 1;
                    while run_end < line_end && !validated[run_end].protected {
                        run_end += 1;
                    }
                    if validated[run_start..run_end]
                        .iter()
                        .any(|word| contains_han(&word.text))
                    {
                        han_runs.push((run_start..run_end).collect::<Vec<_>>());
                    }
                    run_start = run_end;
                }
                if han_runs.len() != 1 {
                    return Err(SourceGateRejectReason::NoSafeHanRun);
                }
                han_runs.pop().ok_or(SourceGateRejectReason::NoSafeHanRun)?
            };
            let bbox = bbox_union(&validated, &target_indices)
                .ok_or(SourceGateRejectReason::PpBboxInvalid)?;
            let text = target_indices
                .iter()
                .map(|index| validated[*index].text.as_str())
                .collect::<String>();
            if text.is_empty() || !contains_han(&text) {
                return Err(SourceGateRejectReason::NoSafeHanRun);
            }
            for index in &target_indices {
                selected[*index] = true;
            }
            indexed_targets.push((
                line_index,
                bbox[0],
                SourceTarget {
                    text,
                    bbox,
                    line_polygons: vec![bbox_quad(bbox)],
                    detector_occurrences: Vec::new(),
                },
            ));
        }
        line_start = line_end;
    }

    if indexed_targets.is_empty() {
        return Err(SourceGateRejectReason::NoSafeHanRun);
    }
    let protected_lines = validated
        .iter()
        .enumerate()
        .filter(|(index, _)| !selected[*index])
        .map(|(_, word)| SourceProtectedLine {
            text: word.text.clone(),
            bbox: word.bbox,
            line_polygons: vec![bbox_quad(word.bbox)],
            detector_occurrences: Vec::new(),
        })
        .collect::<Vec<_>>();
    if indexed_targets.iter().any(|(_, _, target)| {
        protected_lines
            .iter()
            .any(|line| bboxes_intersect(target.bbox, line.bbox))
    }) {
        return Err(SourceGateRejectReason::ProtectedGeometryOverlap);
    }

    indexed_targets.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    Ok(SourceSelection {
        targets: indexed_targets
            .into_iter()
            .map(|(_, _, target)| target)
            .collect(),
        protected_lines,
    })
}

#[cfg(test)]
fn select_chinese_target_with_fallback(
    vl_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> std::result::Result<(SourceSelection, SourceGateDecision), SourceGateRejectReason> {
    match select_chinese_target(vl_text, words, crop_bounds, image_width, image_height) {
        Ok(selection) => {
            let decision = SourceGateDecision::AcceptedPrimary {
                target_count: selection.targets.len(),
                protected_count: selection.protected_lines.len(),
            };
            Ok((selection, decision))
        }
        Err(
            primary_reason @ (SourceGateRejectReason::PpVlCharacterMismatch
            | SourceGateRejectReason::PpVlIncompleteCoverage),
        ) => {
            let Ok((validated, true)) = validate_pp_vl_alignment_internal(
                vl_text,
                words,
                crop_bounds,
                image_width,
                image_height,
                true,
            ) else {
                return Err(primary_reason);
            };
            let Ok(selection) = select_validated_chinese_target(validated) else {
                return Err(primary_reason);
            };
            let decision = SourceGateDecision::AcceptedIsolatedProtectedLatinGeometry {
                target_count: selection.targets.len(),
                protected_count: selection.protected_lines.len(),
            };
            Ok((selection, decision))
        }
        Err(reason) => Err(reason),
    }
}

fn detector_bbox(
    corners: [[f32; 2]; 4],
    crop_width: f32,
    crop_height: f32,
) -> std::result::Result<[f32; 4], SourceGateRejectReason> {
    if corners
        .iter()
        .flatten()
        .any(|coordinate| !coordinate.is_finite())
    {
        return Err(SourceGateRejectReason::PpBboxInvalid);
    }
    let left = corners
        .iter()
        .map(|corner| corner[0])
        .min_by(f32::total_cmp)
        .ok_or(SourceGateRejectReason::PpBboxInvalid)?;
    let top = corners
        .iter()
        .map(|corner| corner[1])
        .min_by(f32::total_cmp)
        .ok_or(SourceGateRejectReason::PpBboxInvalid)?;
    let right = corners
        .iter()
        .map(|corner| corner[0])
        .max_by(f32::total_cmp)
        .ok_or(SourceGateRejectReason::PpBboxInvalid)?;
    let bottom = corners
        .iter()
        .map(|corner| corner[1])
        .max_by(f32::total_cmp)
        .ok_or(SourceGateRejectReason::PpBboxInvalid)?;
    let distinct_corners = corners
        .iter()
        .enumerate()
        .all(|(index, corner)| corners.iter().skip(index + 1).all(|other| corner != other));
    let turns: [f32; 4] = std::array::from_fn(|index| {
        let current = corners[index];
        let next = corners[(index + 1) % corners.len()];
        let after = corners[(index + 2) % corners.len()];
        (next[0] - current[0]) * (after[1] - next[1])
            - (next[1] - current[1]) * (after[0] - next[0])
    });
    let convex = turns.iter().all(|turn| *turn > f32::EPSILON)
        || turns.iter().all(|turn| *turn < -f32::EPSILON);
    let area_twice = (0..corners.len())
        .map(|index| {
            let next = (index + 1) % corners.len();
            corners[index][0] * corners[next][1] - corners[next][0] * corners[index][1]
        })
        .sum::<f32>()
        .abs();
    if !distinct_corners
        || !convex
        || left < 0.0
        || top < 0.0
        || left >= right
        || top >= bottom
        || right > crop_width
        || bottom > crop_height
        || area_twice <= f32::EPSILON
    {
        return Err(SourceGateRejectReason::PpBboxInvalid);
    }
    Ok([left.floor(), top.floor(), right.ceil(), bottom.ceil()])
}

fn bbox_contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

fn validate_observation_selection(
    selection: &SourceSelection,
    detector_occurrences: &[usize],
) -> std::result::Result<(), SourceGateRejectReason> {
    let mut unassigned = detector_occurrences.iter().copied().collect::<HashSet<_>>();
    if unassigned.len() != detector_occurrences.len() {
        return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
    }
    for (polygons, occurrences) in selection
        .targets
        .iter()
        .map(|target| (&target.line_polygons, &target.detector_occurrences))
        .chain(
            selection
                .protected_lines
                .iter()
                .map(|line| (&line.line_polygons, &line.detector_occurrences)),
        )
    {
        if polygons.is_empty() || polygons.len() != occurrences.len() {
            return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
        }
        for occurrence in occurrences {
            if !unassigned.remove(occurrence) {
                return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
            }
        }
    }
    if unassigned.is_empty() {
        Ok(())
    } else {
        Err(SourceGateRejectReason::PpVlIncompleteCoverage)
    }
}

fn quantized_bbox_contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    let inner_left = inner[0].ceil();
    let inner_top = inner[1].ceil();
    let inner_right = inner[2].floor();
    let inner_bottom = inner[3].floor();
    inner_left <= inner_right
        && inner_top <= inner_bottom
        && outer[0].floor() <= inner_left
        && outer[1].floor() <= inner_top
        && outer[2].ceil() >= inner_right
        && outer[3].ceil() >= inner_bottom
}

struct ActivePpLayout {
    detector_bboxes: Vec<[f32; 4]>,
    detector_occurrences: Vec<usize>,
    line_for_detector: Vec<Option<usize>>,
    line_indices: HashSet<usize>,
}

fn active_pp_layout(
    observation: &PpOcrV5Observation,
    crop_bounds: [u32; 4],
    layout_bbox: [f32; 4],
    image_width: u32,
    image_height: u32,
) -> std::result::Result<ActivePpLayout, SourceGateRejectReason> {
    let [crop_left, crop_top, crop_right, crop_bottom] = crop_bounds;
    if crop_left >= crop_right
        || crop_top >= crop_bottom
        || crop_right > image_width
        || crop_bottom > image_height
    {
        return Err(SourceGateRejectReason::PpBboxInvalid);
    }
    if observation.detectors.is_empty() || observation.word_boxes.is_empty() {
        return Err(SourceGateRejectReason::PpNoWords);
    }
    if observation
        .detectors
        .iter()
        .enumerate()
        .any(|(index, detector)| detector.occurrence_index != index)
    {
        return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
    }
    let crop_width = (crop_right - crop_left) as f32;
    let crop_height = (crop_bottom - crop_top) as f32;
    let detector_bboxes = observation
        .detectors
        .iter()
        .map(|detector| detector_bbox(detector.corners, crop_width, crop_height))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let detector_scene_bboxes = detector_bboxes
        .iter()
        .map(|[left, top, right, bottom]| {
            [
                crop_left as f32 + left,
                crop_top as f32 + top,
                crop_left as f32 + right,
                crop_top as f32 + bottom,
            ]
        })
        .collect::<Vec<_>>();
    let mut detector_index_map = vec![None; detector_bboxes.len()];
    let mut active_detector_bboxes = Vec::new();
    let mut active_detector_occurrences = Vec::new();
    for (index, (detector_bbox, scene_bbox)) in detector_bboxes
        .iter()
        .zip(&detector_scene_bboxes)
        .enumerate()
    {
        if bbox_contains(layout_bbox, *scene_bbox) {
            detector_index_map[index] = Some(active_detector_bboxes.len());
            active_detector_bboxes.push(*detector_bbox);
            active_detector_occurrences.push(index);
        } else if bboxes_intersect(layout_bbox, *scene_bbox) {
            return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
        }
    }
    if active_detector_bboxes.is_empty() {
        return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
    }
    if active_detector_bboxes
        .iter()
        .enumerate()
        .any(|(index, bbox)| {
            active_detector_bboxes.iter().skip(index + 1).any(|other| {
                bboxes_intersect(*bbox, *other)
                    || support_bboxes_overlap(
                        *bbox,
                        *other,
                        crop_right - crop_left,
                        crop_bottom - crop_top,
                    )
            })
        })
    {
        return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
    }

    let mut line_for_detector = vec![None; active_detector_bboxes.len()];
    for (line_index, line) in observation.lines.iter().enumerate() {
        let mut active_line_detectors = Vec::new();
        for detector_index in &line.detector_indices {
            let Some(active_index) = detector_index_map.get(*detector_index).copied().flatten()
            else {
                if *detector_index >= detector_index_map.len() {
                    return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
                }
                continue;
            };
            active_line_detectors.push(active_index);
        }
        if active_line_detectors.is_empty() {
            continue;
        }
        if line.recognition.is_none() {
            return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
        }
        for active_index in active_line_detectors {
            let Some(slot) = line_for_detector.get_mut(active_index) else {
                return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
            };
            if slot.replace(line_index).is_some() {
                return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
            }
        }
    }
    if line_for_detector.iter().any(Option::is_none) {
        return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
    }
    let line_indices = line_for_detector.iter().flatten().copied().collect();

    Ok(ActivePpLayout {
        detector_bboxes: active_detector_bboxes,
        detector_occurrences: active_detector_occurrences,
        line_for_detector,
        line_indices,
    })
}

fn select_chinese_target_from_observation_with_layout(
    vl_text: &str,
    observation: &PpOcrV5Observation,
    crop_bounds: [u32; 4],
    layout_bbox: [f32; 4],
    image_width: u32,
    image_height: u32,
) -> std::result::Result<(SourceSelection, SourceGateDecision), SourceGateRejectReason> {
    let [crop_left, crop_top, _, _] = crop_bounds;
    let active = active_pp_layout(
        observation,
        crop_bounds,
        layout_bbox,
        image_width,
        image_height,
    )?;
    let detector_bboxes = active.detector_bboxes;
    let active_detector_occurrences = active.detector_occurrences;
    let line_for_detector = active.line_for_detector;
    let active_lines = observation
        .lines
        .iter()
        .enumerate()
        .filter(|(line_index, _)| active.line_indices.contains(line_index))
        .map(|(_, line)| line)
        .collect::<Vec<_>>();
    let active_words = observation
        .word_boxes
        .iter()
        .filter(|word| active.line_indices.contains(&word.line_index))
        .cloned()
        .collect::<Vec<_>>();

    let mut owned_words = vec![Vec::<&PpOcrWordBox>::new(); detector_bboxes.len()];
    for word in &active_words {
        if word.bbox.iter().any(|coordinate| !coordinate.is_finite())
            || word.bbox[0] >= word.bbox[2]
            || word.bbox[1] >= word.bbox[3]
        {
            return Err(SourceGateRejectReason::PpBboxInvalid);
        }
        let owners = detector_bboxes
            .iter()
            .enumerate()
            .filter(|(detector_index, bbox)| {
                line_for_detector[*detector_index] == Some(word.line_index)
                    && quantized_bbox_contains(**bbox, word.bbox)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
        };
        owned_words[*owner].push(word);
    }
    if owned_words.iter().any(Vec::is_empty) {
        return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
    }

    let pp_has_han = active_words.iter().any(|word| contains_han(&word.text));
    match classify_pp_words(&active_words) {
        Ok(()) => {}
        Err(SourceGateRejectReason::PpNoHanUnprotected) if !pp_has_han => {}
        Err(reason) => return Err(reason),
    }
    if !contains_han(vl_text) {
        return Err(SourceGateRejectReason::NoSafeHanRun);
    }
    let vl_protected_latin = protected_latin_tokens(vl_text);
    let mut pp_protected_latin = active_lines
        .iter()
        .flat_map(|line| {
            line.recognition
                .as_deref()
                .into_iter()
                .flat_map(protected_latin_tokens)
        })
        .collect::<Vec<_>>();
    pp_protected_latin.sort();
    if vl_protected_latin != pp_protected_latin {
        return Err(SourceGateRejectReason::ProtectedLatinHanConflict);
    }

    for words in &mut owned_words {
        words.sort_by(|left, right| {
            left.bbox[1]
                .total_cmp(&right.bbox[1])
                .then_with(|| left.bbox[0].total_cmp(&right.bbox[0]))
        });
    }
    let detector_texts = owned_words
        .iter()
        .map(|words| {
            words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    if detector_texts
        .iter()
        .any(|text| contains_han(text) && contains_protected_latin_word(text))
    {
        return Err(SourceGateRejectReason::ProtectedLatinHanConflict);
    }

    let mut reading_order = (0..detector_bboxes.len()).collect::<Vec<_>>();
    reading_order.sort_by(|left, right| {
        line_for_detector[*left]
            .cmp(&line_for_detector[*right])
            .then_with(|| detector_bboxes[*left][0].total_cmp(&detector_bboxes[*right][0]))
    });
    let pp_scalars = reading_order
        .iter()
        .flat_map(|index| detector_texts[*index].chars())
        .filter(|character| !character.is_whitespace())
        .collect::<Vec<_>>();
    let vl_scalars = vl_text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<Vec<_>>();
    let scalar_alignment_proven = pp_scalars.len() == vl_scalars.len()
        && pp_scalars.iter().zip(&vl_scalars).all(|(pp, vl)| {
            pp == vl || (contains_han(&pp.to_string()) && contains_han(&vl.to_string()))
        });
    let single_complete_han_detector = detector_bboxes.len() == 1
        && active_lines.len() == 1
        && active_lines[0].detector_indices.as_slice() == [active_detector_occurrences[0]]
        && active_lines[0].recognition.as_deref().is_some_and(|text| {
            let mut scalars = text.chars().filter(|character| !character.is_whitespace());
            scalars.clone().next().is_some()
                && scalars.all(|character| contains_han(&character.to_string()))
        })
        && !pp_scalars.is_empty()
        && pp_scalars
            .iter()
            .all(|character| contains_han(&character.to_string()))
        && !vl_scalars.is_empty()
        && vl_scalars
            .iter()
            .all(|character| contains_han(&character.to_string()));
    if !pp_has_han {
        let pp_contains_latin = active_words
            .iter()
            .any(|word| contains_latin_alphabetic(&word.text))
            || active_lines
                .iter()
                .filter_map(|line| line.recognition.as_deref())
                .any(contains_latin_alphabetic);
        if pp_contains_latin || contains_latin_alphabetic(vl_text) {
            return Err(SourceGateRejectReason::PpNoHanProtectedLatin);
        }
        let single_complete_detector = detector_bboxes.len() == 1
            && active_lines.len() == 1
            && active_lines[0].detector_indices.as_slice() == [active_detector_occurrences[0]]
            && active_lines[0]
                .recognition
                .as_deref()
                .is_some_and(|recognition| !recognition.trim().is_empty());
        if !single_complete_detector {
            return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
        }
        let text = vl_scalars.iter().collect::<String>();
        if text.is_empty() || !contains_han(&text) {
            return Err(SourceGateRejectReason::NoSafeHanRun);
        }
        let [left, top, right, bottom] = detector_bboxes[0];
        let bbox = [
            crop_left as f32 + left,
            crop_top as f32 + top,
            crop_left as f32 + right,
            crop_top as f32 + bottom,
        ];
        let selection = SourceSelection {
            targets: vec![SourceTarget {
                text,
                bbox,
                line_polygons: vec![bbox_quad(bbox)],
                detector_occurrences: active_detector_occurrences.clone(),
            }],
            protected_lines: Vec::new(),
        };
        validate_observation_selection(&selection, &active_detector_occurrences)?;
        return Ok((
            selection,
            SourceGateDecision::AcceptedDetectorFallback {
                target_count: 1,
                protected_count: 0,
            },
        ));
    }
    if !scalar_alignment_proven && !single_complete_han_detector {
        return Err(SourceGateRejectReason::PpVlIncompleteCoverage);
    }

    let mut selected_texts = detector_texts.clone();
    if single_complete_han_detector {
        selected_texts[0] = vl_scalars.iter().collect();
    }
    let mut vl_offset = 0;
    if !single_complete_han_detector {
        for detector_index in &reading_order {
            let scalar_count = detector_texts[*detector_index]
                .chars()
                .filter(|character| !character.is_whitespace())
                .count();
            selected_texts[*detector_index] = vl_scalars[vl_offset..vl_offset + scalar_count]
                .iter()
                .collect();
            vl_offset += scalar_count;
        }
    }

    let mut attached_to = vec![None; detector_bboxes.len()];
    for line_index in 0..observation.lines.len() {
        let mut detectors = detector_bboxes
            .iter()
            .enumerate()
            .filter(|(detector_index, _)| line_for_detector[*detector_index] == Some(line_index))
            .map(|(detector_index, _)| detector_index)
            .collect::<Vec<_>>();
        detectors.sort_by(|left, right| {
            detector_bboxes[*left][0].total_cmp(&detector_bboxes[*right][0])
        });
        for (position, detector_index) in detectors.iter().copied().enumerate() {
            let text = &selected_texts[detector_index];
            if contains_han(text) || contains_protected_latin_word(text) {
                continue;
            }
            let mut adjacent_han = [
                position.checked_sub(1).map(|index| detectors[index]),
                detectors.get(position + 1).copied(),
            ]
            .into_iter()
            .flatten()
            .filter(|candidate| contains_han(&selected_texts[*candidate]))
            .collect::<Vec<_>>();
            adjacent_han.sort_by(|left, right| {
                let gap = |candidate: usize| {
                    if detector_bboxes[candidate][2] <= detector_bboxes[detector_index][0] {
                        detector_bboxes[detector_index][0] - detector_bboxes[candidate][2]
                    } else {
                        detector_bboxes[candidate][0] - detector_bboxes[detector_index][2]
                    }
                };
                gap(*left).total_cmp(&gap(*right))
            });
            attached_to[detector_index] = adjacent_han.first().copied();
        }
    }

    let mut targets = Vec::new();
    let mut protected_lines = Vec::new();
    for detector_index in 0..detector_bboxes.len() {
        if attached_to[detector_index].is_some()
            && !contains_latin_alphabetic(&selected_texts[detector_index])
        {
            continue;
        }
        let mut members = std::iter::once(detector_index)
            .chain(attached_to.iter().enumerate().filter_map(|(index, owner)| {
                (*owner == Some(detector_index)
                    && !contains_latin_alphabetic(&selected_texts[index]))
                .then_some(index)
            }))
            .collect::<Vec<_>>();
        members.sort_by(|left, right| {
            detector_bboxes[*left][1]
                .total_cmp(&detector_bboxes[*right][1])
                .then_with(|| detector_bboxes[*left][0].total_cmp(&detector_bboxes[*right][0]))
        });
        let text = members
            .iter()
            .map(|index| selected_texts[*index].as_str())
            .collect::<String>();
        let [left, top, right, bottom] = members.iter().skip(1).fold(
            detector_bboxes[members[0]],
            |[left, top, right, bottom], index| {
                let bbox = detector_bboxes[*index];
                [
                    left.min(bbox[0]),
                    top.min(bbox[1]),
                    right.max(bbox[2]),
                    bottom.max(bbox[3]),
                ]
            },
        );
        let bbox = [
            crop_left as f32 + left,
            crop_top as f32 + top,
            crop_left as f32 + right,
            crop_top as f32 + bottom,
        ];
        let line_polygons = members
            .iter()
            .map(|index| {
                let [left, top, right, bottom] = detector_bboxes[*index];
                bbox_quad([
                    crop_left as f32 + left,
                    crop_top as f32 + top,
                    crop_left as f32 + right,
                    crop_top as f32 + bottom,
                ])
            })
            .collect::<Vec<_>>();
        if contains_han(&text) || attached_to[detector_index].is_some() {
            targets.push(SourceTarget {
                text,
                bbox,
                line_polygons,
                detector_occurrences: members
                    .iter()
                    .map(|index| active_detector_occurrences[*index])
                    .collect(),
            });
        } else {
            protected_lines.push(SourceProtectedLine {
                text,
                bbox,
                line_polygons,
                detector_occurrences: members
                    .iter()
                    .map(|index| active_detector_occurrences[*index])
                    .collect(),
            });
        }
    }
    if targets.is_empty() {
        return Err(SourceGateRejectReason::NoSafeHanRun);
    }
    targets.sort_by(|left, right| {
        left.bbox[1]
            .total_cmp(&right.bbox[1])
            .then_with(|| left.bbox[0].total_cmp(&right.bbox[0]))
    });
    let target_count = targets.len();
    let protected_count = protected_lines.len();
    let selection = SourceSelection {
        targets,
        protected_lines,
    };
    validate_observation_selection(&selection, &active_detector_occurrences)?;
    Ok((
        selection,
        SourceGateDecision::AcceptedPrimary {
            target_count,
            protected_count,
        },
    ))
}

#[cfg(test)]
fn select_chinese_target_from_observation(
    vl_text: &str,
    observation: &PpOcrV5Observation,
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> std::result::Result<(SourceSelection, SourceGateDecision), SourceGateRejectReason> {
    select_chinese_target_from_observation_with_layout(
        vl_text,
        observation,
        crop_bounds,
        [
            crop_bounds[0] as f32,
            crop_bounds[1] as f32,
            crop_bounds[2] as f32,
            crop_bounds[3] as f32,
        ],
        image_width,
        image_height,
    )
}

struct SourceGateCandidate {
    node_id: NodeId,
    crop: DynamicImage,
    vl_crop: DynamicImage,
    crop_bounds: [u32; 4],
    layout_bbox: [f32; 4],
}

pub(in crate::pipeline) fn rgba_fingerprint(image: &DynamicImage) -> String {
    let rgba = image.to_rgba8();
    let mut hasher = blake3::Hasher::new();
    hasher.update(&rgba.width().to_le_bytes());
    hasher.update(&rgba.height().to_le_bytes());
    hasher.update(rgba.as_raw());
    hasher.finalize().to_hex().to_string()
}

fn trace_decision(node_id: NodeId, decision: &SourceGateDecision) {
    #[cfg(test)]
    record_diagnostic(SourceGateDiagnosticEvent::Decision {
        node_id,
        decision: decision.clone(),
    });
    if tracing::enabled!(target: "koharu::source_gate", tracing::Level::DEBUG) {
        let decision =
            serde_json::to_string(decision).unwrap_or_else(|_| "serialization_error".into());
        tracing::debug!(
            target: "koharu::source_gate",
            node_id = ?node_id,
            decision,
            "source_gate.decision"
        );
    }
}

fn is_gate_marker(text: &TextData) -> bool {
    matches!(
        text.detector.as_deref(),
        Some(SOURCE_GATE_TARGET_DETECTOR | SOURCE_GATE_PROTECTED_DETECTOR)
    )
}

pub(crate) fn has_gate_candidates(scene: &Scene, page: PageId) -> bool {
    scene.page(page).is_some_and(|page| {
        page.nodes
            .values()
            .any(|node| matches!(&node.kind, NodeKind::Text(text) if !is_gate_marker(text)))
    })
}

fn active_crop_policies() -> Vec<SourceGateCropPolicy> {
    #[cfg(test)]
    if let Some(policy) = *TEST_CROP_POLICY
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("source gate crop policy mutex poisoned")
    {
        return vec![policy];
    }
    B0_CROP_POLICIES.to_vec()
}

fn crop_policy_parameters(policy: SourceGateCropPolicy, transform: &Transform) -> (f32, bool) {
    let short_side = transform.width.min(transform.height).max(0.0);
    let long_side = transform.width.max(transform.height).max(0.0);
    match policy {
        SourceGateCropPolicy::C0 => (0.0, false),
        SourceGateCropPolicy::C1 => (1.0, false),
        SourceGateCropPolicy::C2 => (2.0, false),
        SourceGateCropPolicy::C4 => (4.0, false),
        SourceGateCropPolicy::Q2 => (2.0, true),
        SourceGateCropPolicy::S25L4 => ((short_side / 4.0).max(long_side / 25.0), false),
        SourceGateCropPolicy::S25L5 => ((short_side / 4.0).max(long_side / 20.0), false),
        SourceGateCropPolicy::S25L6 => ((short_side / 4.0).max(long_side * 3.0 / 50.0), false),
        SourceGateCropPolicy::S25L7 => ((short_side / 4.0).max(long_side * 7.0 / 100.0), false),
    }
}

fn safe_crop_bounds_with_policy(
    transform: &Transform,
    image_width: u32,
    image_height: u32,
    policy: SourceGateCropPolicy,
) -> Option<[u32; 4]> {
    let policy = crop_policy_parameters(policy, transform);
    compute_safe_crop_bounds(transform, image_width, image_height, policy)
}

#[cfg(test)]
pub(in crate::pipeline) fn primary_crop_bounds_for_test(
    transform: &Transform,
    image_width: u32,
    image_height: u32,
) -> Option<[u32; 4]> {
    safe_crop_bounds_with_policy(transform, image_width, image_height, PRIMARY_CROP_POLICY)
}

fn compute_safe_crop_bounds(
    transform: &Transform,
    image_width: u32,
    image_height: u32,
    (padding, snap_to_two): (f32, bool),
) -> Option<[u32; 4]> {
    if image_width == 0
        || image_height == 0
        || [
            transform.x,
            transform.y,
            transform.width,
            transform.height,
            transform.rotation_deg,
        ]
        .iter()
        .any(|value| !value.is_finite())
        || transform.width <= 0.0
        || transform.height <= 0.0
        || transform.rotation_deg != 0.0
    {
        return None;
    }
    let mut left = (transform.x - padding).floor();
    let mut top = (transform.y - padding).floor();
    let mut right = (transform.x + transform.width + padding).ceil().max(0.0);
    let mut bottom = (transform.y + transform.height + padding).ceil().max(0.0);
    if snap_to_two {
        left = (left / 2.0).floor() * 2.0;
        top = (top / 2.0).floor() * 2.0;
        right = (right / 2.0).ceil() * 2.0;
        bottom = (bottom / 2.0).ceil() * 2.0;
    }
    let left = left.max(0.0).min(image_width as f32) as u32;
    let top = top.max(0.0).min(image_height as f32) as u32;
    let right = right.min(image_width as f32) as u32;
    let bottom = bottom.min(image_height as f32) as u32;
    (left < right && top < bottom).then_some([left, top, right, bottom])
}

fn strict_layout_geometry(
    transform: &Transform,
    image_width: u32,
    image_height: u32,
) -> Option<([f32; 4], [u32; 4])> {
    let right = transform.x + transform.width;
    let bottom = transform.y + transform.height;
    if image_width == 0
        || image_height == 0
        || [
            transform.x,
            transform.y,
            transform.width,
            transform.height,
            transform.rotation_deg,
            right,
            bottom,
        ]
        .iter()
        .any(|value| !value.is_finite())
        || transform.rotation_deg != 0.0
        || transform.x < 0.0
        || transform.y < 0.0
        || transform.x >= right
        || transform.y >= bottom
        || right > image_width as f32
        || bottom > image_height as f32
    {
        return None;
    }
    Some((
        [transform.x, transform.y, right, bottom],
        [
            transform.x.floor() as u32,
            transform.y.floor() as u32,
            right.ceil() as u32,
            bottom.ceil() as u32,
        ],
    ))
}

fn source_gate_candidates(
    image: &DynamicImage,
    scene: &Scene,
    page: PageId,
) -> Result<(Vec<SourceGateCandidate>, Vec<NodeId>)> {
    let page_ref = scene
        .page(page)
        .ok_or_else(|| anyhow::anyhow!("page not found"))?;
    let mut candidates = Vec::new();
    let mut invalid = Vec::new();
    let mut candidate_index = 0;
    for (node_id, node) in &page_ref.nodes {
        let NodeKind::Text(text) = &node.kind else {
            continue;
        };
        if is_gate_marker(text) {
            continue;
        }
        tracing::debug!(
            target: "koharu::source_gate",
            candidate_index,
            node_id = ?node_id,
            confidence = text.confidence,
            x = node.transform.x,
            y = node.transform.y,
            width = node.transform.width,
            height = node.transform.height,
            rotation_deg = node.transform.rotation_deg,
            "source_gate.layout_candidate"
        );
        #[cfg(test)]
        record_diagnostic(SourceGateDiagnosticEvent::LayoutCandidate {
            candidate_index,
            node_id: *node_id,
            confidence: text.confidence,
            bbox: [
                node.transform.x,
                node.transform.y,
                node.transform.width,
                node.transform.height,
            ],
        });
        let Some((layout_bbox, [vl_left, vl_top, vl_right, vl_bottom])) =
            strict_layout_geometry(&node.transform, image.width(), image.height())
        else {
            invalid.push(*node_id);
            candidate_index += active_crop_policies().len();
            continue;
        };
        let vl_crop = image.crop_imm(vl_left, vl_top, vl_right - vl_left, vl_bottom - vl_top);
        let vl_crop_rgba_hash = rgba_fingerprint(&vl_crop);
        let mut valid = false;
        for policy in active_crop_policies() {
            let Some([left, top, right, bottom]) = safe_crop_bounds_with_policy(
                &node.transform,
                image.width(),
                image.height(),
                policy,
            ) else {
                candidate_index += 1;
                continue;
            };
            valid = true;
            let crop = image.crop_imm(left, top, right - left, bottom - top);
            if tracing::enabled!(target: "koharu::source_gate", tracing::Level::DEBUG) {
                tracing::debug!(
                    target: "koharu::source_gate",
                    candidate_index,
                    node_id = ?node_id,
                    left,
                    top,
                    right,
                    bottom,
                    crop_rgba_hash = %rgba_fingerprint(&crop),
                    vl_left,
                    vl_top,
                    vl_right,
                    vl_bottom,
                    vl_crop_rgba_hash,
                    "source_gate.crop"
                );
            }
            #[cfg(test)]
            record_diagnostic(SourceGateDiagnosticEvent::Crop {
                candidate_index,
                node_id: *node_id,
                bounds: [left, top, right, bottom],
                crop_rgba_hash: rgba_fingerprint(&crop),
                vl_bounds: [vl_left, vl_top, vl_right, vl_bottom],
                vl_crop_rgba_hash: vl_crop_rgba_hash.clone(),
            });
            candidates.push(SourceGateCandidate {
                node_id: *node_id,
                crop,
                vl_crop: vl_crop.clone(),
                crop_bounds: [left, top, right, bottom],
                layout_bbox,
            });
            candidate_index += 1;
        }
        if !valid {
            invalid.push(*node_id);
        }
    }
    Ok((candidates, invalid))
}

fn target_transform([left, top, right, bottom]: [f32; 4]) -> Transform {
    Transform {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
        rotation_deg: 0.0,
    }
}

fn final_target_detected_font_size(transform: &Transform) -> f32 {
    transform.width.min(transform.height).max(1.0)
}

fn target_text_data(target: SourceTarget, detector: &str, transform: &Transform) -> TextData {
    TextData {
        source_lang: (detector == SOURCE_GATE_TARGET_DETECTOR).then(|| "zh".into()),
        rotation_deg: Some(0.0),
        detector: Some(detector.into()),
        text: Some(target.text),
        line_polygons: Some(target.line_polygons),
        detected_font_size_px: (detector == SOURCE_GATE_TARGET_DETECTOR)
            .then(|| final_target_detected_font_size(transform)),
        ..Default::default()
    }
}

fn update_target_ops(
    page: PageId,
    node_id: NodeId,
    selection: SourceSelection,
    next_at: &mut usize,
) -> Result<Vec<Op>> {
    let mut targets = selection.targets.into_iter();
    let first = targets
        .next()
        .ok_or_else(|| anyhow::anyhow!("source gate selection has no targets"))?;
    let first_transform = target_transform(first.bbox);
    let first_text = first.text;
    let first_polygons = first.line_polygons;
    let mut ops = vec![Op::UpdateNode {
        page,
        id: node_id,
        patch: NodePatch {
            transform: Some(first_transform),
            visible: Some(true),
            data: Some(NodeDataPatch::Text(TextDataPatch {
                source_lang: Some(Some("zh".into())),
                source_direction: Some(None),
                rendered_direction: Some(None),
                line_polygons: Some(Some(first_polygons)),
                rotation_deg: Some(Some(0.0)),
                detector: Some(Some(SOURCE_GATE_TARGET_DETECTOR.into())),
                text: Some(Some(first_text)),
                translation: Some(None),
                style: Some(None),
                font_prediction: Some(None),
                detected_font_size_px: Some(Some(final_target_detected_font_size(
                    &first_transform,
                ))),
                sprite: Some(None),
                sprite_transform: Some(None),
                lock_layout_box: Some(false),
                typography_plan_verified: Some(false),
                ..Default::default()
            })),
        },
        prev: NodePatch::default(),
    }];

    for target in targets {
        let transform = target_transform(target.bbox);
        let node = Node {
            id: NodeId::new(),
            transform,
            visible: true,
            kind: NodeKind::Text(target_text_data(
                target,
                SOURCE_GATE_TARGET_DETECTOR,
                &transform,
            )),
        };
        ops.push(Op::AddNode {
            page,
            node,
            at: *next_at,
        });
        *next_at += 1;
    }
    for line in selection.protected_lines {
        let target = SourceTarget {
            text: line.text,
            bbox: line.bbox,
            line_polygons: line.line_polygons,
            detector_occurrences: line.detector_occurrences,
        };
        let transform = target_transform(target.bbox);
        let node = Node {
            id: NodeId::new(),
            transform,
            visible: false,
            kind: NodeKind::Text(target_text_data(
                target,
                SOURCE_GATE_PROTECTED_DETECTOR,
                &transform,
            )),
        };
        ops.push(Op::AddNode {
            page,
            node,
            at: *next_at,
        });
        *next_at += 1;
    }
    Ok(ops)
}

fn remove_node(scene: &Scene, page: PageId, id: NodeId) -> Result<Op> {
    let page_ref = scene
        .page(page)
        .ok_or_else(|| anyhow::anyhow!("page not found"))?;
    let (prev_index, (_, prev_node)) = page_ref
        .nodes
        .iter()
        .enumerate()
        .find(|(_, (node_id, _))| **node_id == id)
        .ok_or_else(|| anyhow::anyhow!("node not found"))?;
    Ok(Op::RemoveNode {
        page,
        id,
        prev_node: prev_node.clone(),
        prev_index,
    })
}

fn zero_target_cleanup(scene: &Scene, page: PageId) -> Result<Vec<Op>> {
    let page_ref = scene
        .page(page)
        .ok_or_else(|| anyhow::anyhow!("page not found"))?;
    let keep_inpainted = find_mask_node(scene, page, MaskRole::BrushInpaint).is_some();
    page_ref
        .nodes
        .iter()
        .filter_map(|(id, node)| {
            let remove = match &node.kind {
                NodeKind::Image(image) => {
                    image.role == ImageRole::Rendered
                        || (image.role == ImageRole::Inpainted && !keep_inpainted)
                }
                NodeKind::Mask(mask) => matches!(mask.role, MaskRole::Segment | MaskRole::Bubble),
                NodeKind::Text(text) => {
                    text.detector.as_deref() == Some(SOURCE_GATE_PROTECTED_DETECTOR)
                }
            };
            remove.then_some(*id)
        })
        .map(|id| remove_node(scene, page, id))
        .collect()
}

pub(crate) async fn dispatch_source_gate<Observe, Validate, Fut>(
    image: &DynamicImage,
    scene: &Scene,
    page: PageId,
    mut observe: Observe,
    mut validate: Validate,
) -> Result<Vec<Op>>
where
    Observe: FnMut(NodeId, &DynamicImage) -> Result<PpOcrV5Observation>,
    Validate: FnMut(Vec<DynamicImage>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>>>,
{
    #[cfg(test)]
    record_diagnostic(SourceGateDiagnosticEvent::Input {
        backend: "dispatch",
        width: image.width(),
        height: image.height(),
        decoded_rgba_hash: rgba_fingerprint(image),
    });
    let (candidates, invalid) = source_gate_candidates(image, scene, page)?;
    for node_id in &invalid {
        trace_decision(*node_id, &SourceGateDecision::InvalidCandidateGeometry);
    }
    let mut rejected = invalid;
    let mut candidate_failures = HashSet::new();
    let mut resolved = HashSet::new();
    let mut accepted = scene
        .page(page)
        .into_iter()
        .flat_map(|page| page.nodes.values())
        .filter(|node| {
            node.visible
                && matches!(&node.kind, NodeKind::Text(text) if text.detector.as_deref() == Some(SOURCE_GATE_TARGET_DETECTOR))
        })
        .count();
    let mut next_at = scene.page(page).map(|page| page.nodes.len()).unwrap_or(0);
    let mut mutations = Vec::new();

    for candidate in candidates {
        if resolved.contains(&candidate.node_id) {
            continue;
        }
        let observation = observe(candidate.node_id, &candidate.crop)?;
        let words = &observation.word_boxes;
        tracing::debug!(
            target: "koharu::source_gate",
            node_id = ?candidate.node_id,
            word_count = words.len(),
            detector_count = observation.detectors.len(),
            missing_recognition_count = observation
                .lines
                .iter()
                .filter(|line| line.recognition.is_none())
                .count(),
            "source_gate.pp_summary"
        );
        #[cfg(test)]
        {
            let (raw_detectors, canonical_lines) = diagnostic_observation(&observation);
            record_diagnostic(SourceGateDiagnosticEvent::PpSummary {
                node_id: candidate.node_id,
                words: words
                    .iter()
                    .map(|word| {
                        let has_han = contains_han(&word.text);
                        let has_protected_latin = contains_protected_latin_word(&word.text);
                        let script = match (has_han, has_protected_latin) {
                            (true, true) => "han_protected_latin",
                            (true, false) => "han",
                            (false, true) => "protected_latin",
                            (false, false) => "other",
                        };
                        PpWordDiagnostic {
                            line_index: word.line_index,
                            han_scalar_count: word
                                .text
                                .chars()
                                .filter(|ch| contains_han(&ch.to_string()))
                                .count(),
                            character_count: word
                                .text
                                .chars()
                                .filter(|ch| !ch.is_whitespace())
                                .count(),
                            script,
                            confidence: word.confidence,
                            bbox: word.bbox,
                        }
                    })
                    .collect(),
                raw_detectors,
                canonical_lines,
            });
        }
        for word in words {
            let has_han = contains_han(&word.text);
            let has_protected_latin = contains_protected_latin_word(&word.text);
            let script = match (has_han, has_protected_latin) {
                (true, true) => "han_protected_latin",
                (true, false) => "han",
                (false, true) => "protected_latin",
                (false, false) => "other",
            };
            tracing::debug!(
                target: "koharu::source_gate",
                node_id = ?candidate.node_id,
                line_index = word.line_index,
                character_count = word.text.chars().filter(|ch| !ch.is_whitespace()).count(),
                confidence = word.confidence,
                left = word.bbox[0],
                top = word.bbox[1],
                right = word.bbox[2],
                bottom = word.bbox[3],
                script,
                "source_gate.pp_word"
            );
        }
        let active = match active_pp_layout(
            &observation,
            candidate.crop_bounds,
            candidate.layout_bbox,
            image.width(),
            image.height(),
        ) {
            Ok(active) => active,
            Err(reason) => {
                trace_decision(
                    candidate.node_id,
                    &SourceGateDecision::RejectedBeforeVl { reason },
                );
                candidate_failures.insert(candidate.node_id);
                continue;
            }
        };
        let active_words = words
            .iter()
            .filter(|word| active.line_indices.contains(&word.line_index))
            .cloned()
            .collect::<Vec<_>>();
        match classify_pp_words(&active_words) {
            Ok(()) => {}
            Err(SourceGateRejectReason::PpNoHanUnprotected) => {}
            Err(reason) => {
                trace_decision(
                    candidate.node_id,
                    &SourceGateDecision::RejectedBeforeVl { reason },
                );
                candidate_failures.insert(candidate.node_id);
                continue;
            }
        }

        let mut vl_texts = validate(vec![candidate.vl_crop.clone()]).await?;
        if vl_texts.len() != 1 {
            trace_decision(candidate.node_id, &SourceGateDecision::VlBatchError);
            ensure!(false, "source gate OCR count mismatch");
        }
        let vl_text = vl_texts.remove(0);
        tracing::debug!(
            target: "koharu::source_gate",
            node_id = ?candidate.node_id,
            contains_han = contains_han(&vl_text),
            character_count = vl_text.chars().filter(|ch| !ch.is_whitespace()).count(),
            line_count = vl_text.lines().filter(|line| !line.trim().is_empty()).count(),
            "source_gate.vl_summary"
        );
        #[cfg(test)]
        record_diagnostic(SourceGateDiagnosticEvent::VlSummary {
            node_id: candidate.node_id,
            contains_han: contains_han(&vl_text),
            han_scalar_count: vl_text
                .chars()
                .filter(|ch| contains_han(&ch.to_string()))
                .count(),
            character_count: vl_text.chars().filter(|ch| !ch.is_whitespace()).count(),
            line_count: vl_text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
        });
        let selection = select_chinese_target_from_observation_with_layout(
            &vl_text,
            &observation,
            candidate.crop_bounds,
            candidate.layout_bbox,
            image.width(),
            image.height(),
        );
        match selection {
            Ok((selection, decision)) => {
                #[cfg(test)]
                record_diagnostic(diagnostic_selection_geometry(
                    candidate.node_id,
                    &observation,
                    candidate.crop_bounds,
                    &selection,
                ));
                trace_decision(candidate.node_id, &decision);
                if !resolved.insert(candidate.node_id) {
                    continue;
                }
                accepted += selection.targets.len();
                mutations.extend(update_target_ops(
                    page,
                    candidate.node_id,
                    selection,
                    &mut next_at,
                )?);
            }
            Err(reason) => {
                let decision = SourceGateDecision::RejectedAfterVl { reason };
                trace_decision(candidate.node_id, &decision);
                candidate_failures.insert(candidate.node_id);
            }
        }
    }
    rejected.extend(
        candidate_failures
            .into_iter()
            .filter(|node_id| !resolved.contains(node_id)),
    );

    let mut removed = HashSet::new();
    let mut ops = mutations;
    for node_id in rejected {
        if removed.insert(node_id) {
            ops.push(remove_node(scene, page, node_id)?);
        }
    }
    if accepted == 0 {
        for op in zero_target_cleanup(scene, page)? {
            let node_id = match &op {
                Op::RemoveNode { id, .. } => Some(*id),
                _ => None,
            };
            if node_id.is_none_or(|id| removed.insert(id)) {
                ops.push(op);
            }
        }
    }
    Ok(ops)
}

pub struct Model {
    pub(super) vl: tokio::sync::OnceCell<Mutex<PaddleOcrVl>>,
    pub(super) word_boxes: tokio::sync::Mutex<PpOcrV5>,
    pub(super) cpu: bool,
}

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        let image = load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
        if tracing::enabled!(target: "koharu::source_gate", tracing::Level::DEBUG) {
            let decoded_rgba_hash = rgba_fingerprint(&image);
            tracing::debug!(
                target: "koharu::source_gate",
                backend = if self.cpu { "cpu" } else { "prefer_gpu" },
                width = image.width(),
                height = image.height(),
                decoded_rgba_hash,
                "source_gate.input"
            );
        }
        let pp = self.word_boxes.lock().await;
        dispatch_source_gate(
            &image,
            ctx.scene,
            ctx.page,
            |_, crop| pp.observe(crop),
            |crops| async move {
                let vl = self
                    .vl
                    .get_or_try_init(|| async {
                        let backend = shared_llama_backend(ctx.runtime)?;
                        let loaded = PaddleOcrVl::load(ctx.runtime, self.cpu, backend).await?;
                        Ok::<_, anyhow::Error>(Mutex::new(loaded))
                    })
                    .await?;
                let mut vl = vl
                    .lock()
                    .map_err(|_| anyhow::anyhow!("PaddleOCR mutex poisoned"))?;
                Ok(vl
                    .inference_images(&crops, PaddleOcrVlTask::Ocr, MAX_NEW_TOKENS)?
                    .into_iter()
                    .map(|output| output.text)
                    .collect())
            },
        )
        .await
    }
}

inventory::submit! {
    EngineInfo {
        id: "pp-ocr-v5-source-gate",
        name: "PP-OCRv5 Source Gate",
        needs: &[Artifact::TextBoxes],
        produces: &[Artifact::SourceTextBoxes],
        load: |runtime, cpu| Box::pin(async move {
            let word_boxes = PpOcrV5::load(runtime).await?;
            Ok(Box::new(Model {
                vl: tokio::sync::OnceCell::new(),
                word_boxes: tokio::sync::Mutex::new(word_boxes),
                cpu,
            }) as Box<dyn Engine>)
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use image::{DynamicImage, RgbImage};
    use koharu_core::{
        BlobRef, ImageData, MaskData, Node, NodeId, NodeKind, Page, Scene, TextData, Transform,
    };
    use koharu_ml::pp_ocr_v5::{
        PpOcrDetectorOccurrence, PpOcrLineObservation, PpOcrV5Observation, PpOcrWordBox,
    };

    use super::*;

    #[derive(Clone)]
    struct CapturedLogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturedLogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogWriter {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn word(
        text: &str,
        line_index: usize,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    ) -> PpOcrWordBox {
        PpOcrWordBox {
            line_index,
            text: text.into(),
            bbox: [left, top, right, bottom],
            confidence: 0.9,
        }
    }

    fn detector(
        occurrence_index: usize,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    ) -> PpOcrDetectorOccurrence {
        PpOcrDetectorOccurrence {
            occurrence_index,
            corners: [[left, top], [right, top], [right, bottom], [left, bottom]],
        }
    }

    #[test]
    fn detector_bbox_accepts_skewed_in_bounds_quad_as_aabb() {
        assert_eq!(
            detector_bbox(
                [[10.0, 12.0], [50.0, 10.0], [52.0, 30.0], [11.0, 32.0]],
                64.0,
                64.0,
            ),
            Ok([10.0, 10.0, 52.0, 32.0])
        );
        assert_eq!(
            detector_bbox(
                [[10.25, 12.5], [50.25, 10.25], [52.5, 30.25], [11.0, 32.5]],
                64.0,
                64.0,
            ),
            Ok([10.0, 10.0, 53.0, 33.0])
        );
    }

    #[test]
    fn detector_bbox_rejects_unsafe_quad_shapes() {
        assert_eq!(
            detector_bbox(
                [[f32::NAN, 10.0], [50.0, 10.0], [52.0, 30.0], [11.0, 32.0],],
                64.0,
                64.0,
            ),
            Err(SourceGateRejectReason::PpBboxInvalid)
        );
        assert_eq!(
            detector_bbox(
                [[-1.0, 12.0], [50.0, 10.0], [52.0, 30.0], [11.0, 32.0]],
                64.0,
                64.0,
            ),
            Err(SourceGateRejectReason::PpBboxInvalid)
        );
        assert_eq!(
            detector_bbox(
                [[10.0, 10.0], [20.0, 10.0], [30.0, 10.0], [40.0, 10.0]],
                64.0,
                64.0,
            ),
            Err(SourceGateRejectReason::PpBboxInvalid)
        );
        assert_eq!(
            detector_bbox(
                [[10.0, 10.0], [40.0, 10.0], [40.0, 30.0], [10.0, 10.0]],
                64.0,
                64.0,
            ),
            Err(SourceGateRejectReason::PpBboxInvalid)
        );
        assert_eq!(
            detector_bbox(
                [[10.0, 10.0], [50.0, 30.0], [10.0, 40.0], [40.0, 10.0]],
                64.0,
                64.0,
            ),
            Err(SourceGateRejectReason::PpBboxInvalid)
        );
    }

    #[test]
    fn detector_ownership_accepts_subpixel_word_bbox_drift() {
        let observed = observation(
            vec![PpOcrDetectorOccurrence {
                occurrence_index: 0,
                corners: [[10.5, 10.5], [50.0, 10.5], [50.0, 30.0], [10.5, 30.0]],
            }],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: Some("中文".into()),
            }],
            vec![word("中文", 0, 10.0, 10.0, 50.5, 30.5)],
        );

        let (selection, decision) =
            select_chinese_target_from_observation("中文", &observed, [0, 0, 100, 50], 100, 50)
                .unwrap();

        assert_eq!(selection.targets.len(), 1);
        assert!(matches!(
            decision,
            SourceGateDecision::AcceptedPrimary { .. }
        ));
    }

    #[test]
    fn detector_ownership_rejects_word_bbox_beyond_tolerance() {
        let observed = observation(
            vec![PpOcrDetectorOccurrence {
                occurrence_index: 0,
                corners: [[10.5, 10.5], [50.0, 10.5], [50.0, 30.0], [10.5, 30.0]],
            }],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: Some("中文".into()),
            }],
            vec![word("中文", 0, 8.9, 10.0, 50.5, 30.5)],
        );

        assert_eq!(
            select_chinese_target_from_observation("中文", &observed, [0, 0, 100, 50], 100, 50,),
            Err(SourceGateRejectReason::PpVlIncompleteCoverage)
        );
    }

    #[test]
    fn detector_ownership_ignores_outside_unrecognized_detector() {
        let observed = observation(
            vec![
                detector(0, 10.0, 10.0, 50.0, 30.0),
                detector(1, 120.0, 60.0, 150.0, 80.0),
            ],
            vec![
                PpOcrLineObservation {
                    detector_indices: vec![0],
                    recognition: Some("中文".into()),
                },
                PpOcrLineObservation {
                    detector_indices: vec![1],
                    recognition: None,
                },
            ],
            vec![word("中文", 0, 12.0, 12.0, 48.0, 28.0)],
        );

        let (selection, decision) = select_chinese_target_from_observation_with_layout(
            "中文",
            &observed,
            [0, 0, 160, 100],
            [0.0, 0.0, 100.0, 50.0],
            160,
            100,
        )
        .unwrap();

        assert_eq!(selection.targets.len(), 1);
        assert_eq!(selection.targets[0].text, "中文");
        assert!(matches!(
            decision,
            SourceGateDecision::AcceptedPrimary { .. }
        ));

        let recognized_outside = observation(
            vec![
                detector(0, 10.0, 10.0, 50.0, 30.0),
                detector(1, 120.0, 60.0, 150.0, 80.0),
            ],
            vec![
                PpOcrLineObservation {
                    detector_indices: vec![0],
                    recognition: Some("中文".into()),
                },
                PpOcrLineObservation {
                    detector_indices: vec![1],
                    recognition: Some("PRODUCT".into()),
                },
            ],
            vec![
                word("中文", 0, 12.0, 12.0, 48.0, 28.0),
                word("PRODUCT", 1, 122.0, 62.0, 148.0, 78.0),
            ],
        );
        let (selection, decision) = select_chinese_target_from_observation_with_layout(
            "中文",
            &recognized_outside,
            [0, 0, 160, 100],
            [0.0, 0.0, 100.0, 50.0],
            160,
            100,
        )
        .unwrap();
        assert_eq!(selection.targets.len(), 1);
        assert!(selection.protected_lines.is_empty());
        assert!(matches!(
            decision,
            SourceGateDecision::AcceptedPrimary {
                target_count: 1,
                protected_count: 0,
            }
        ));

        let intersecting_outside = observation(
            vec![
                detector(0, 10.0, 10.0, 50.0, 30.0),
                detector(1, 90.0, 40.0, 120.0, 60.0),
            ],
            vec![
                PpOcrLineObservation {
                    detector_indices: vec![0],
                    recognition: Some("中文".into()),
                },
                PpOcrLineObservation {
                    detector_indices: vec![1],
                    recognition: Some("PRODUCT".into()),
                },
            ],
            vec![
                word("中文", 0, 12.0, 12.0, 48.0, 28.0),
                word("PRODUCT", 1, 92.0, 42.0, 118.0, 58.0),
            ],
        );
        assert_eq!(
            select_chinese_target_from_observation_with_layout(
                "中文",
                &intersecting_outside,
                [0, 0, 160, 100],
                [0.0, 0.0, 100.0, 50.0],
                160,
                100,
            ),
            Err(SourceGateRejectReason::PpVlIncompleteCoverage)
        );
    }

    #[test]
    fn detector_ownership_rejects_out_of_range_inactive_line_index() {
        let observed = observation(
            vec![detector(0, 10.0, 10.0, 50.0, 30.0)],
            vec![
                PpOcrLineObservation {
                    detector_indices: vec![0],
                    recognition: Some("中文".into()),
                },
                PpOcrLineObservation {
                    detector_indices: vec![1],
                    recognition: None,
                },
            ],
            vec![word("中文", 0, 12.0, 12.0, 48.0, 28.0)],
        );

        assert_eq!(
            select_chinese_target_from_observation_with_layout(
                "中文",
                &observed,
                [0, 0, 160, 100],
                [0.0, 0.0, 100.0, 50.0],
                160,
                100,
            ),
            Err(SourceGateRejectReason::PpVlIncompleteCoverage)
        );
    }

    fn observation(
        detectors: Vec<PpOcrDetectorOccurrence>,
        lines: Vec<PpOcrLineObservation>,
        word_boxes: Vec<PpOcrWordBox>,
    ) -> PpOcrV5Observation {
        PpOcrV5Observation {
            detectors,
            lines,
            word_boxes,
        }
    }

    fn observation_from_words(mut words: Vec<PpOcrWordBox>) -> PpOcrV5Observation {
        let line_indices = words
            .iter()
            .map(|word| word.line_index)
            .collect::<std::collections::BTreeSet<_>>();
        for word in &mut words {
            word.line_index = line_indices
                .iter()
                .position(|line_index| *line_index == word.line_index)
                .expect("word line index is present");
        }
        let detectors = words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                detector(
                    index,
                    word.bbox[0],
                    word.bbox[1],
                    word.bbox[2],
                    word.bbox[3],
                )
            })
            .collect::<Vec<_>>();
        let mut by_line = std::collections::BTreeMap::<usize, Vec<usize>>::new();
        for (index, word) in words.iter().enumerate() {
            by_line.entry(word.line_index).or_default().push(index);
        }
        let lines = by_line
            .into_values()
            .map(|detector_indices| PpOcrLineObservation {
                recognition: Some(
                    detector_indices
                        .iter()
                        .map(|index| words[*index].text.as_str())
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                detector_indices,
            })
            .collect();
        observation(detectors, lines, words)
    }

    #[test]
    fn detector_support_promotes_numeric_pp_when_vl_confirms_han() {
        let observed = observation(
            vec![detector(0, 2.0, 3.0, 82.0, 23.0)],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: Some("360".into()),
            }],
            vec![word("360", 0, 4.0, 4.0, 80.0, 22.0)],
        );

        let (selection, decision) = select_chinese_target_from_observation(
            "360度呵护",
            &observed,
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();

        assert_eq!(selection.targets.len(), 1);
        assert_eq!(selection.targets[0].text, "360度呵护");
        assert_eq!(selection.targets[0].bbox, [12.0, 23.0, 92.0, 43.0]);
        assert!(matches!(
            decision,
            SourceGateDecision::AcceptedDetectorFallback {
                target_count: 1,
                protected_count: 0
            }
        ));
    }

    #[test]
    fn selection_diagnostic_binds_detector_to_emitted_scene_geometry() {
        let observed = observation(
            vec![detector(0, 2.0, 3.0, 82.0, 23.0)],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: Some("360".into()),
            }],
            vec![word("360", 0, 4.0, 4.0, 80.0, 22.0)],
        );
        let (selection, _) = select_chinese_target_from_observation(
            "360度呵护",
            &observed,
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();

        let SourceGateDiagnosticEvent::SelectionGeometry {
            targets,
            protected_lines,
            detector_ownership,
            ..
        } = diagnostic_selection_geometry(NodeId::new(), &observed, [10, 20, 110, 70], &selection)
        else {
            unreachable!()
        };

        assert_eq!(targets.len(), 1);
        assert!(protected_lines.is_empty());
        assert_eq!(detector_ownership.len(), 1);
        assert_eq!(detector_ownership[0].canonical_line_index, Some(0));
        assert_eq!(
            detector_ownership[0].scene_quad_f32_bits,
            diagnostic_corner_bits([[12.0, 23.0], [92.0, 23.0], [92.0, 43.0], [12.0, 43.0]])
        );
        assert!(matches!(
            detector_ownership[0].assignment,
            SourceGateDetectorAssignmentDiagnostic::Target { target_index: 0 }
        ));
    }

    #[test]
    fn detector_support_keeps_adjacent_selected_detectors_separate() {
        let observed = observation(
            vec![
                detector(0, 2.0, 3.0, 12.0, 23.0),
                detector(1, 12.0, 3.0, 82.0, 23.0),
            ],
            vec![PpOcrLineObservation {
                detector_indices: vec![0, 1],
                recognition: Some("S型曲线".into()),
            }],
            vec![
                word("S", 0, 2.0, 3.0, 12.0, 23.0),
                word("型曲线", 0, 12.0, 3.0, 82.0, 23.0),
            ],
        );

        let (selection, decision) = select_chinese_target_from_observation(
            "S型曲线",
            &observed,
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();

        assert_eq!(selection.targets.len(), 2);
        assert_eq!(selection.targets[0].text, "S");
        assert_eq!(selection.targets[0].bbox, [12.0, 23.0, 22.0, 43.0]);
        assert_eq!(selection.targets[1].text, "型曲线");
        assert_eq!(selection.targets[1].bbox, [22.0, 23.0, 92.0, 43.0]);
        assert!(selection.protected_lines.is_empty());
        assert!(matches!(
            decision,
            SourceGateDecision::AcceptedPrimary {
                target_count: 2,
                protected_count: 0
            }
        ));
    }

    #[test]
    fn detector_ownership_ignores_line_breaks_and_word_order_and_uses_vl_han() {
        let detectors = vec![
            detector(0, 2.0, 4.0, 42.0, 24.0),
            detector(1, 48.0, 2.0, 88.0, 22.0),
        ];
        let lines = vec![PpOcrLineObservation {
            detector_indices: vec![0, 1],
            recognition: Some("科学选鞋".into()),
        }];
        let ordered = observation(
            detectors.clone(),
            lines.clone(),
            vec![
                word("科学", 0, 4.0, 4.0, 40.0, 22.0),
                word("选鞋", 0, 50.0, 4.0, 86.0, 22.0),
            ],
        );
        let reversed = observation(
            detectors,
            lines,
            vec![
                word("选鞋", 0, 50.0, 4.0, 86.0, 22.0),
                word("科学", 0, 4.0, 4.0, 40.0, 22.0),
            ],
        );

        let select = |observed| {
            select_chinese_target_from_observation("科學\n选鞋", observed, [0, 0, 100, 50], 100, 50)
                .unwrap()
                .0
        };

        assert_eq!(select(&ordered), select(&reversed));
        assert_eq!(select(&ordered).targets.len(), 2);
        let selection = select(&ordered);
        assert_eq!(
            selection
                .targets
                .iter()
                .find(|target| target.bbox[0] < 45.0)
                .unwrap()
                .text,
            "科學"
        );
        assert_eq!(
            selection
                .targets
                .iter()
                .find(|target| target.bbox[0] > 45.0)
                .unwrap()
                .text,
            "选鞋"
        );
    }

    #[test]
    fn single_detector_han_scalar_count_mismatch_uses_vl_text() {
        let observed = observation_from_words(vec![word("安全", 0, 12.0, 8.0, 72.0, 28.0)]);
        let (selection, decision) = select_chinese_target_from_observation_with_layout(
            "安全规范",
            &observed,
            [10, 20, 110, 70],
            [20.25, 25.5, 100.75, 60.5],
            200,
            100,
        )
        .unwrap();

        assert_eq!(selection.targets.len(), 1);
        assert_eq!(selection.targets[0].text, "安全规范");
        assert!(matches!(
            decision,
            SourceGateDecision::AcceptedPrimary {
                target_count: 1,
                protected_count: 0,
            }
        ));
    }

    #[test]
    fn multiple_detectors_han_scalar_count_mismatch_stays_fail_closed() {
        let observed = observation(
            vec![
                detector(0, 2.0, 3.0, 42.0, 23.0),
                detector(1, 48.0, 3.0, 88.0, 23.0),
            ],
            vec![PpOcrLineObservation {
                detector_indices: vec![0, 1],
                recognition: Some("安全".into()),
            }],
            vec![
                word("安", 0, 4.0, 4.0, 40.0, 22.0),
                word("全", 0, 50.0, 4.0, 86.0, 22.0),
            ],
        );

        assert_eq!(
            select_chinese_target_from_observation("安全规范", &observed, [0, 0, 100, 50], 100, 50,),
            Err(SourceGateRejectReason::PpVlIncompleteCoverage)
        );
    }

    #[test]
    fn single_detector_scalar_count_mismatch_with_latin_stays_fail_closed() {
        let observed = observation(
            vec![detector(0, 12.0, 8.0, 72.0, 28.0)],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: Some("安全A".into()),
            }],
            vec![word("安全", 0, 12.0, 8.0, 72.0, 28.0)],
        );

        assert_eq!(
            select_chinese_target_from_observation_with_layout(
                "安全规范",
                &observed,
                [10, 20, 110, 70],
                [20.25, 25.5, 100.75, 60.5],
                200,
                100,
            ),
            Err(SourceGateRejectReason::PpVlIncompleteCoverage)
        );
    }

    #[test]
    fn numeric_pp_fallback_requires_one_complete_detector() {
        let observed = observation(
            vec![
                detector(0, 2.0, 3.0, 42.0, 23.0),
                detector(1, 48.0, 3.0, 88.0, 23.0),
            ],
            vec![PpOcrLineObservation {
                detector_indices: vec![0, 1],
                recognition: Some("360".into()),
            }],
            vec![
                word("3", 0, 4.0, 4.0, 40.0, 22.0),
                word("60", 0, 50.0, 4.0, 86.0, 22.0),
            ],
        );

        let reason = select_chinese_target_from_observation(
            "360度呵护",
            &observed,
            [0, 0, 100, 50],
            100,
            50,
        )
        .unwrap_err();

        assert_eq!(reason, SourceGateRejectReason::PpVlIncompleteCoverage);
    }

    #[test]
    fn merged_target_preserves_each_detector_as_an_eligible_scene_line() {
        let observed = observation(
            vec![
                detector(0, 2.0, 3.0, 42.0, 23.0),
                detector(1, 48.0, 3.0, 88.0, 23.0),
            ],
            vec![PpOcrLineObservation {
                detector_indices: vec![0, 1],
                recognition: Some("型曲线3-6".into()),
            }],
            vec![
                word("型曲线", 0, 4.0, 4.0, 40.0, 22.0),
                word("3-6", 0, 50.0, 4.0, 86.0, 22.0),
            ],
        );
        let (selection, _) = select_chinese_target_from_observation(
            "型曲线3-6",
            &observed,
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();

        assert_eq!(selection.targets.len(), 1);
        assert_eq!(selection.targets[0].detector_occurrences, [0, 1]);
        assert_eq!(
            selection.targets[0].line_polygons,
            [
                bbox_quad([12.0, 23.0, 52.0, 43.0]),
                bbox_quad([58.0, 23.0, 98.0, 43.0]),
            ]
        );
        let SourceGateDiagnosticEvent::SelectionGeometry {
            targets,
            detector_ownership,
            ..
        } = diagnostic_selection_geometry(NodeId::new(), &observed, [10, 20, 110, 70], &selection)
        else {
            unreachable!()
        };
        assert_eq!(targets[0].eligible_line_quads_f32_bits.len(), 2);
        assert_eq!(
            detector_ownership
                .iter()
                .map(|ownership| ownership.eligible_text_line_quad_f32_bits)
                .collect::<Vec<_>>(),
            targets[0]
                .eligible_line_quads_f32_bits
                .iter()
                .copied()
                .map(Some)
                .collect::<Vec<_>>()
        );
        let transform = target_transform(selection.targets[0].bbox);
        let text = target_text_data(
            selection.targets[0].clone(),
            SOURCE_GATE_TARGET_DETECTOR,
            &transform,
        );
        let eligible =
            crate::pipeline::engines::support::eligible_text_lines(&transform, &text, 200, 100)
                .unwrap();
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].region.line_polygons.as_ref().unwrap().len(), 2);
        let mask = crate::pipeline::engines::support::line_support_mask(200, 100, &eligible);
        assert_ne!(mask.get_pixel(20, 30).0[0], 0);
        assert_eq!(mask.get_pixel(55, 30).0[0], 0);
        assert_ne!(mask.get_pixel(70, 30).0[0], 0);
    }

    #[test]
    fn diagnostics_preserve_raw_occurrences_and_missing_recognition_without_text() {
        let duplicate = detector(0, 2.0, 3.0, 42.0, 23.0);
        let observed = observation(
            vec![
                duplicate.clone(),
                PpOcrDetectorOccurrence {
                    occurrence_index: 1,
                    corners: duplicate.corners,
                },
            ],
            vec![PpOcrLineObservation {
                detector_indices: vec![0, 1],
                recognition: None,
            }],
            Vec::new(),
        );

        let (raw, lines) = diagnostic_observation(&observed);
        assert_eq!(raw.len(), 2);
        assert_eq!(
            raw[0].source_scaled_quad_f32_bits,
            raw[1].source_scaled_quad_f32_bits
        );
        assert_eq!(lines[0].detector_occurrences.len(), 2);
        assert!(lines[0].recognition.is_none());
        let encoded = serde_json::to_string(&(raw, lines)).unwrap();
        assert!(encoded.contains("\"recognition\":null"));
        assert!(!encoded.contains("\"text\""));
    }

    #[test]
    fn detector_support_keeps_protected_latin_and_incomplete_coverage_fail_closed() {
        let protected = observation(
            vec![detector(0, 2.0, 3.0, 82.0, 23.0)],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: Some("PRODUCT ID".into()),
            }],
            vec![word("PRODUCT", 0, 4.0, 4.0, 50.0, 22.0)],
        );
        assert_eq!(
            select_chinese_target_from_observation(
                "PRODUCT ID",
                &protected,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpNoHanProtectedLatin)
        );

        let single_latin = observation_from_words(vec![word("S", 0, 4.0, 4.0, 50.0, 22.0)]);
        assert_eq!(
            select_chinese_target_from_observation("安全", &single_latin, [0, 0, 100, 50], 100, 50,),
            Err(SourceGateRejectReason::PpNoHanProtectedLatin)
        );

        let numeric = observation_from_words(vec![word("360", 0, 4.0, 4.0, 50.0, 22.0)]);
        assert_eq!(
            select_chinese_target_from_observation("S型安全", &numeric, [0, 0, 100, 50], 100, 50,),
            Err(SourceGateRejectReason::PpNoHanProtectedLatin)
        );

        let filtered_latin_segment = observation(
            vec![detector(0, 2.0, 3.0, 82.0, 23.0)],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: Some("360 PRODUCT".into()),
            }],
            vec![word("360", 0, 4.0, 4.0, 50.0, 22.0)],
        );
        assert_eq!(
            select_chinese_target_from_observation(
                "安全",
                &filtered_latin_segment,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::ProtectedLatinHanConflict)
        );

        let missing_recognition = observation(
            vec![detector(0, 2.0, 3.0, 82.0, 23.0)],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: None,
            }],
            vec![word("360", 0, 4.0, 4.0, 50.0, 22.0)],
        );
        assert_eq!(
            select_chinese_target_from_observation(
                "360度呵护",
                &missing_recognition,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpVlIncompleteCoverage)
        );

        let unassigned_detector = observation(
            vec![
                detector(0, 2.0, 3.0, 42.0, 23.0),
                detector(1, 48.0, 3.0, 88.0, 23.0),
            ],
            vec![PpOcrLineObservation {
                detector_indices: vec![0, 1],
                recognition: Some("中文".into()),
            }],
            vec![word("中文", 0, 4.0, 4.0, 40.0, 22.0)],
        );
        assert_eq!(
            select_chinese_target_from_observation(
                "中文",
                &unassigned_detector,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpVlIncompleteCoverage)
        );

        let mut low_confidence = observation_from_words(vec![word("360", 0, 4.0, 4.0, 50.0, 22.0)]);
        low_confidence.word_boxes[0].confidence = 0.4;
        assert_eq!(
            select_chinese_target_from_observation(
                "360度呵护",
                &low_confidence,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpLowConfidenceNonHan)
        );

        let subpixel_overlap = observation(
            vec![
                detector(0, 2.1, 3.0, 10.1, 23.0),
                detector(1, 10.2, 3.0, 18.2, 23.0),
            ],
            vec![PpOcrLineObservation {
                detector_indices: vec![0, 1],
                recognition: Some("中文".into()),
            }],
            vec![
                word("中", 0, 2.1, 3.0, 10.1, 23.0),
                word("文", 0, 10.2, 3.0, 18.2, 23.0),
            ],
        );
        assert_eq!(
            select_chinese_target_from_observation(
                "中文",
                &subpixel_overlap,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpVlIncompleteCoverage)
        );

        let pp_han_vl_latin = observation_from_words(vec![word("中文", 0, 4.0, 4.0, 50.0, 22.0)]);
        assert_eq!(
            select_chinese_target_from_observation(
                "PRODUCT ID",
                &pp_han_vl_latin,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::NoSafeHanRun)
        );
        assert_eq!(
            select_chinese_target_from_observation(
                "中文 PRODUCT ID",
                &pp_han_vl_latin,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::ProtectedLatinHanConflict)
        );

        let pp_han_with_protected = observation(
            vec![
                detector(0, 2.0, 3.0, 42.0, 23.0),
                detector(1, 48.0, 3.0, 98.0, 23.0),
            ],
            vec![PpOcrLineObservation {
                detector_indices: vec![0, 1],
                recognition: Some("中文 PRODUCT".into()),
            }],
            vec![
                word("中文", 0, 4.0, 4.0, 40.0, 22.0),
                word("PRODUCT", 0, 50.0, 4.0, 96.0, 22.0),
            ],
        );
        assert!(
            select_chinese_target_from_observation(
                "中文 PRODUCT",
                &pp_han_with_protected,
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_ok()
        );
        assert_eq!(
            select_chinese_target_from_observation(
                "中文 BRAND",
                &pp_han_with_protected,
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::ProtectedLatinHanConflict)
        );
    }

    #[test]
    fn gate_pp_prefilter_rejects_pure_english_without_vl() {
        let words = [
            word("SLENDER", 0, 0.0, 0.0, 40.0, 20.0),
            word("WAIST", 0, 45.0, 0.0, 80.0, 20.0),
        ];
        assert_eq!(
            classify_pp_words(&words),
            Err(SourceGateRejectReason::PpNoHanProtectedLatin)
        );
    }

    #[test]
    fn gate_vl_validation_keeps_same_line_single_label_and_excludes_other_lines() {
        let same_line = select_chinese_target(
            "S型曲线",
            &[
                word("S", 0, 0.0, 0.0, 8.0, 20.0),
                word("型曲线", 0, 8.0, 0.0, 40.0, 20.0),
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();
        assert_eq!(same_line.targets.len(), 1);
        assert_eq!(same_line.targets[0].text, "S型曲线");
        assert_eq!(same_line.targets[0].bbox, [10.0, 20.0, 50.0, 40.0]);

        let other_line = select_chinese_target(
            "S\n中文",
            &[
                word("S", 0, 0.0, 0.0, 8.0, 10.0),
                word("中文", 1, 10.0, 15.0, 40.0, 35.0),
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();
        assert_eq!(other_line.targets.len(), 1);
        assert_eq!(other_line.targets[0].text, "中文");
        assert_eq!(other_line.targets[0].bbox, [20.0, 35.0, 50.0, 55.0]);
        assert_eq!(
            other_line.protected_lines,
            vec![SourceProtectedLine {
                text: "S".into(),
                bbox: [10.0, 20.0, 18.0, 30.0],
                line_polygons: vec![bbox_quad([10.0, 20.0, 18.0, 30.0])],
                detector_occurrences: Vec::new(),
            }]
        );
    }

    #[test]
    fn gate_vl_validation_keeps_only_han_beside_complete_english() {
        let target = select_chinese_target(
            "Peach蜜桃臀",
            &[
                word("Peach", 0, 0.0, 0.0, 40.0, 20.0),
                word("蜜桃臀", 0, 45.0, 0.0, 100.0, 20.0),
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();
        assert_eq!(target.targets.len(), 1);
        assert_eq!(target.targets[0].text, "蜜桃臀");
        assert_eq!(target.targets[0].bbox, [55.0, 20.0, 110.0, 40.0]);
        assert_eq!(
            target.protected_lines,
            vec![SourceProtectedLine {
                text: "Peach".into(),
                bbox: [10.0, 20.0, 50.0, 40.0],
                line_polygons: vec![bbox_quad([10.0, 20.0, 50.0, 40.0])],
                detector_occurrences: Vec::new(),
            }]
        );
    }

    #[test]
    fn gate_vl_validation_rejects_mismatch_unseparated_and_invalid_geometry() {
        assert_eq!(
            select_chinese_target(
                "Peach蜜桃臀",
                &[
                    word("Peacx", 0, 0.0, 0.0, 40.0, 20.0),
                    word("蜜桃臀", 0, 45.0, 0.0, 100.0, 20.0),
                ],
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpVlCharacterMismatch)
        );
        assert_eq!(
            select_chinese_target(
                "AI智能塑形",
                &[word("AI智能塑形", 0, 0.0, 0.0, 100.0, 20.0)],
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::ProtectedLatinHanConflict)
        );
        assert_eq!(
            select_chinese_target(
                "中文",
                &[word("中文", 0, f32::NAN, 0.0, 40.0, 20.0)],
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpBboxInvalid)
        );
    }

    #[test]
    fn isolated_protected_latin_geometry_allows_only_one_isolated_ascii_letter_mismatch() {
        let words = [
            word("PEACH", 0, 2.0, 2.0, 40.0, 18.0),
            word("HIp", 0, 48.0, 2.0, 72.0, 18.0),
            word("蜜桃臀", 1, 2.0, 25.0, 45.0, 45.0),
        ];
        let (selection, decision) = select_chinese_target_with_fallback(
            "PEACH HIP\n蜜桃臀",
            &words,
            [10, 20, 130, 80],
            200,
            100,
        )
        .unwrap();
        assert!(matches!(
            decision,
            SourceGateDecision::AcceptedIsolatedProtectedLatinGeometry {
                target_count: 1,
                protected_count: 2,
            }
        ));
        assert_eq!(selection.targets[0].text, "蜜桃臀");
        assert_eq!(
            selection.protected_lines,
            vec![
                SourceProtectedLine {
                    text: "PEACH".into(),
                    bbox: [12.0, 22.0, 50.0, 38.0],
                    line_polygons: vec![bbox_quad([12.0, 22.0, 50.0, 38.0])],
                    detector_occurrences: Vec::new(),
                },
                SourceProtectedLine {
                    text: "HIp".into(),
                    bbox: [58.0, 22.0, 82.0, 38.0],
                    line_polygons: vec![bbox_quad([58.0, 22.0, 82.0, 38.0])],
                    detector_occurrences: Vec::new(),
                },
            ]
        );

        let (_, exact) = select_chinese_target_with_fallback(
            "PEACH HIP\n蜜桃臀",
            &[
                word("PEACH", 0, 2.0, 2.0, 40.0, 18.0),
                word("HIP", 0, 48.0, 2.0, 72.0, 18.0),
                word("蜜桃臀", 1, 2.0, 25.0, 45.0, 45.0),
            ],
            [10, 20, 130, 80],
            200,
            100,
        )
        .unwrap();
        assert!(matches!(exact, SourceGateDecision::AcceptedPrimary { .. }));

        let rejected = [
            vec![
                word("PEAXH", 0, 2.0, 2.0, 40.0, 18.0),
                word("HIQ", 0, 48.0, 2.0, 72.0, 18.0),
                word("蜜桃臀", 1, 2.0, 25.0, 45.0, 45.0),
            ],
            vec![
                word("PEACH", 0, 2.0, 2.0, 40.0, 18.0),
                word("HI8", 0, 48.0, 2.0, 72.0, 18.0),
                word("蜜桃臀", 1, 2.0, 25.0, 45.0, 45.0),
            ],
            vec![
                word("PEACH", 0, 2.0, 2.0, 40.0, 18.0),
                word("S", 0, 48.0, 2.0, 58.0, 18.0),
                word("蜜桃臀", 1, 2.0, 25.0, 45.0, 45.0),
            ],
            vec![
                word("PEACH@HIp", 0, 2.0, 2.0, 72.0, 18.0),
                word("蜜桃臀", 1, 2.0, 25.0, 45.0, 45.0),
            ],
            vec![
                word("PEACH", 0, 0.0, 2.0, 40.0, 18.0),
                word("HIp", 0, 48.0, 2.0, 72.0, 18.0),
                word("蜜桃臀", 1, 2.0, 25.0, 45.0, 45.0),
            ],
        ];
        let authoritative = [
            "PEACH HIP\n蜜桃臀",
            "PEACH HI9\n蜜桃臀",
            "PEACH T\n蜜桃臀",
            "PEACH#HIP\n蜜桃臀",
            "PEACH HIP\n蜜桃臀",
        ];
        for (words, authoritative) in rejected.into_iter().zip(authoritative) {
            assert_eq!(
                select_chinese_target_with_fallback(
                    authoritative,
                    &words,
                    [10, 20, 130, 80],
                    200,
                    100,
                ),
                Err(SourceGateRejectReason::PpVlCharacterMismatch)
            );
        }

        assert_eq!(
            select_chinese_target_with_fallback(
                "PEACH HIP\nSLIM WAIST\n蜜桃臀",
                &[
                    word("PEACH", 0, 2.0, 2.0, 40.0, 12.0),
                    word("HIp", 0, 48.0, 2.0, 72.0, 12.0),
                    word("SLIM", 1, 2.0, 16.0, 32.0, 26.0),
                    word("WAISx", 1, 40.0, 16.0, 78.0, 26.0),
                    word("蜜桃臀", 2, 2.0, 30.0, 45.0, 45.0),
                ],
                [10, 20, 130, 80],
                200,
                100,
            ),
            Err(SourceGateRejectReason::PpVlCharacterMismatch)
        );
    }

    #[test]
    fn source_gate_rejection_reason_precedence_is_stable() {
        let mut non_finite = word("中文", 0, 0.0, 0.0, 40.0, 20.0);
        non_finite.confidence = f32::NAN;
        let mut low_han = word("中文", 0, 0.0, 0.0, 40.0, 20.0);
        low_han.confidence = 0.4999;
        let mut low_non_han = word("!", 0, 41.0, 0.0, 45.0, 20.0);
        low_non_han.confidence = 0.4999;
        let mut low_single_latin = word("S", 0, 0.0, 0.0, 8.0, 20.0);
        low_single_latin.confidence = 0.1;

        let cases = [
            (Vec::new(), SourceGateRejectReason::PpNoWords),
            (
                vec![word("English", 0, 0.0, 0.0, 50.0, 20.0)],
                SourceGateRejectReason::PpNoHanProtectedLatin,
            ),
            (
                vec![word("", 0, 0.0, 0.0, 8.0, 20.0)],
                SourceGateRejectReason::PpNoHanUnprotected,
            ),
            (
                vec![word("   ", 0, 0.0, 0.0, 8.0, 20.0)],
                SourceGateRejectReason::PpNoHanUnprotected,
            ),
            (
                vec![low_single_latin],
                SourceGateRejectReason::PpLowConfidenceNonHan,
            ),
            (
                vec![non_finite],
                SourceGateRejectReason::PpNonFiniteConfidence,
            ),
            (
                vec![word("中文", 0, 0.0, 0.0, 40.0, 20.0), low_non_han],
                SourceGateRejectReason::PpLowConfidenceNonHan,
            ),
            (vec![low_han], SourceGateRejectReason::PpLowConfidenceHan),
        ];

        for (words, expected) in cases {
            assert_eq!(classify_pp_words(&words), Err(expected));
        }
    }

    #[test]
    fn gate_rejects_pp_vl_line_mismatch() {
        assert_eq!(
            select_chinese_target(
                "中文二",
                &[
                    word("中文", 0, 0.0, 0.0, 30.0, 10.0),
                    word("二", 1, 0.0, 12.0, 10.0, 22.0),
                ],
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpVlLineMismatch)
        );
    }

    #[test]
    fn source_gate_rejection_reason_covers_bounds_crossing_and_order() {
        assert_eq!(
            select_chinese_target(
                "中文",
                &[word("中文", 0, 0.0, 0.0, 101.0, 20.0)],
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpBboxInvalid)
        );
        assert_eq!(
            select_chinese_target(
                "中文",
                &[
                    word("中", 0, 0.0, 0.0, 30.0, 20.0),
                    word("文", 0, 20.0, 0.0, 45.0, 20.0),
                ],
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpOrderInvalid)
        );
        assert_eq!(
            select_chinese_target(
                "中\n文",
                &[
                    word("中", 1, 0.0, 0.0, 20.0, 10.0),
                    word("文", 0, 0.0, 12.0, 20.0, 22.0),
                ],
                [0, 0, 100, 50],
                100,
                50,
            ),
            Err(SourceGateRejectReason::PpOrderInvalid)
        );
    }

    #[test]
    fn gate_decision_is_serializable_without_flat_invalid_states() {
        let encoded = serde_json::to_string(&SourceGateDecision::InvalidCandidateGeometry).unwrap();
        assert_eq!(
            serde_json::from_str::<SourceGateDecision>(&encoded).unwrap(),
            SourceGateDecision::InvalidCandidateGeometry
        );

        let accepted = SourceGateDecision::AcceptedIsolatedProtectedLatinGeometry {
            target_count: 1,
            protected_count: 2,
        };
        let encoded = serde_json::to_string(&accepted).unwrap();
        assert_eq!(
            serde_json::from_str::<SourceGateDecision>(&encoded).unwrap(),
            accepted
        );
        assert_eq!(accepted.fallback(), "isolated_protected_latin_geometry");
    }

    #[test]
    fn real_crop_fixture_manifest_matches_decoded_pixels() {
        let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/source-gate-deterministic-recall");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture_dir.join("fixture-manifest.json"))
                .expect("fixture manifest must exist"),
        )
        .expect("fixture manifest must be valid JSON");
        assert_eq!(
            manifest["source_raw_blake3"],
            "d913ecaa34ae2490e8d5aff6b38f5c927b960e78645c40eb57ac07fbda9b6842"
        );
        for fixture in manifest["fixtures"]
            .as_array()
            .expect("fixtures must be an array")
        {
            let name = fixture["name"].as_str().expect("fixture name");
            let image = image::open(fixture_dir.join(name)).expect("fixture must decode");
            let expected_size = fixture["size"].as_array().expect("fixture size");
            assert_eq!(image.width(), expected_size[0].as_u64().unwrap() as u32);
            assert_eq!(image.height(), expected_size[1].as_u64().unwrap() as u32);
            assert_eq!(
                rgba_fingerprint(&image),
                fixture["decoded_rgba_blake3"].as_str().unwrap(),
                "fixture hash drift: {name}"
            );
        }
    }

    #[test]
    fn safe_crop_bounds_are_stable_for_observed_and_boundary_cases() {
        let observed = Transform {
            x: 331.73453,
            y: 818.80286,
            width: 125.895874,
            height: 77.2738,
            rotation_deg: 0.0,
        };
        for (policy, expected) in [
            (SourceGateCropPolicy::C0, [331, 818, 458, 897]),
            (SourceGateCropPolicy::C1, [330, 817, 459, 898]),
            (SourceGateCropPolicy::C2, [329, 816, 460, 899]),
            (SourceGateCropPolicy::C4, [327, 814, 462, 901]),
            (SourceGateCropPolicy::Q2, [328, 816, 460, 900]),
            (SourceGateCropPolicy::S25L4, [312, 799, 477, 916]),
            (SourceGateCropPolicy::S25L5, [312, 799, 477, 916]),
            (SourceGateCropPolicy::S25L6, [312, 799, 477, 916]),
            (SourceGateCropPolicy::S25L7, [312, 799, 477, 916]),
        ] {
            assert_eq!(
                safe_crop_bounds_with_policy(&observed, 790, 1023, policy),
                Some(expected)
            );
        }

        let mut edge_cases = Vec::new();
        for (edge, delta, expected) in [
            (0, -2.0, [8, 20, 41, 61]),
            (0, -1.0, [9, 20, 41, 61]),
            (0, 1.0, [11, 20, 41, 61]),
            (0, 2.0, [12, 20, 41, 61]),
            (2, -2.0, [10, 20, 39, 61]),
            (2, -1.0, [10, 20, 40, 61]),
            (2, 1.0, [10, 20, 42, 61]),
            (2, 2.0, [10, 20, 43, 61]),
            (1, -2.0, [10, 18, 41, 61]),
            (1, -1.0, [10, 19, 41, 61]),
            (1, 1.0, [10, 21, 41, 61]),
            (1, 2.0, [10, 22, 41, 61]),
            (3, -2.0, [10, 20, 41, 59]),
            (3, -1.0, [10, 20, 41, 60]),
            (3, 1.0, [10, 20, 41, 62]),
            (3, 2.0, [10, 20, 41, 63]),
        ] {
            let mut edges = [10.25, 20.25, 40.75, 60.75];
            edges[edge] += delta;
            edge_cases.push((edges, expected));
        }
        for ([left, top, right, bottom], expected) in edge_cases {
            let transform = Transform {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
                rotation_deg: 0.0,
            };
            assert_eq!(
                safe_crop_bounds_with_policy(&transform, 100, 100, SourceGateCropPolicy::C0),
                Some(expected)
            );
        }
        assert_eq!(
            safe_crop_bounds_with_policy(
                &Transform {
                    x: -1.2,
                    y: -2.1,
                    width: 5.0,
                    height: 6.0,
                    rotation_deg: 0.0,
                },
                100,
                100,
                SourceGateCropPolicy::C2,
            ),
            Some([0, 0, 6, 6])
        );
    }

    #[test]
    fn ratio_crop_bounds_scale_with_target_short_side() {
        let small = Transform {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 20.0,
            rotation_deg: 0.0,
        };
        let large = Transform {
            x: 20.0,
            y: 40.0,
            width: 200.0,
            height: 40.0,
            rotation_deg: 0.0,
        };
        assert_eq!(
            safe_crop_bounds_with_policy(&small, 400, 400, SourceGateCropPolicy::S25L4),
            Some([5, 15, 115, 45])
        );
        assert_eq!(
            safe_crop_bounds_with_policy(&large, 400, 400, SourceGateCropPolicy::S25L4),
            Some([10, 30, 230, 90])
        );
        assert_eq!(
            safe_crop_bounds_with_policy(&small, 400, 400, SourceGateCropPolicy::S25L7),
            Some([3, 13, 117, 47])
        );
        assert_eq!(
            safe_crop_bounds_with_policy(&large, 400, 400, SourceGateCropPolicy::S25L7),
            Some([6, 26, 234, 94])
        );
    }

    #[test]
    fn gate_keeps_protected_runs_disjoint_on_both_sides_of_han() {
        let selection = select_chinese_target(
            "Slim蜜桃臀Fit",
            &[
                word("Slim", 0, 0.0, 0.0, 25.0, 20.0),
                word("蜜桃臀", 0, 30.0, 0.0, 65.0, 20.0),
                word("Fit", 0, 70.0, 0.0, 95.0, 20.0),
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();
        assert_eq!(selection.targets[0].bbox, [40.0, 20.0, 75.0, 40.0]);
        assert_eq!(selection.protected_lines.len(), 2);
        assert!(selection.protected_lines.iter().all(|line| {
            line.bbox[2] <= selection.targets[0].bbox[0]
                || line.bbox[0] >= selection.targets[0].bbox[2]
        }));
    }

    #[test]
    fn gate_splits_han_lines_around_an_english_line() {
        let selection = select_chinese_target(
            "中文一\nEnglish\n中文二",
            &[
                word("中文一", 0, 0.0, 0.0, 40.0, 15.0),
                word("English", 1, 0.0, 20.0, 60.0, 35.0),
                word("中文二", 2, 0.0, 40.0, 40.0, 55.0),
            ],
            [10, 20, 110, 90],
            200,
            120,
        )
        .unwrap();
        assert_eq!(selection.targets.len(), 2);
        assert_eq!(selection.targets[0].bbox, [10.0, 20.0, 50.0, 35.0]);
        assert_eq!(selection.targets[1].bbox, [10.0, 60.0, 50.0, 75.0]);
        assert_eq!(selection.protected_lines.len(), 1);
    }

    #[test]
    fn final_target_detected_font_size_uses_short_axis_with_pixel_floor() {
        for (width, height, expected) in
            [(173.0, 31.0, 31.0), (19.0, 117.0, 19.0), (0.25, 8.0, 1.0)]
        {
            assert_eq!(
                final_target_detected_font_size(&Transform {
                    width,
                    height,
                    ..Default::default()
                }),
                expected
            );
        }
    }

    #[test]
    fn final_han_targets_refresh_detected_size_without_touching_protected_latin() {
        let reused = NodeId::new();
        let mut original = candidate(reused, [0.0, 0.0, 300.0, 166.50424], false, "pp-doclayout");
        let NodeKind::Text(text) = &mut original.kind else {
            unreachable!()
        };
        text.detected_font_size_px = Some(166.50424);
        let (scene, page) = scene_with_nodes(vec![original]);
        let mut next_at = 1;
        let ops = update_target_ops(
            page,
            reused,
            SourceSelection {
                targets: vec![
                    SourceTarget {
                        text: "全身塑形".into(),
                        bbox: [10.0, 20.0, 218.0, 59.0],
                        line_polygons: vec![bbox_quad([10.0, 20.0, 218.0, 59.0])],
                        detector_occurrences: Vec::new(),
                    },
                    SourceTarget {
                        text: "蜜桃臀".into(),
                        bbox: [30.0, 70.0, 150.0, 94.0],
                        line_polygons: vec![bbox_quad([30.0, 70.0, 150.0, 94.0])],
                        detector_occurrences: Vec::new(),
                    },
                ],
                protected_lines: vec![SourceProtectedLine {
                    text: "PEACH HIP".into(),
                    bbox: [30.0, 5.0, 150.0, 18.0],
                    line_polygons: vec![bbox_quad([30.0, 5.0, 150.0, 18.0])],
                    detector_occurrences: Vec::new(),
                }],
            },
            &mut next_at,
        )
        .unwrap();

        let scene = apply_ops(scene, ops);
        let page = scene.pages.get(&page).unwrap();
        let mut han_targets = page
            .nodes
            .iter()
            .filter_map(|node| {
                let (id, node) = node;
                let NodeKind::Text(text) = &node.kind else {
                    return None;
                };
                (text.detector.as_deref() == Some(SOURCE_GATE_TARGET_DETECTOR)).then_some((
                    *id,
                    node.transform,
                    text,
                ))
            })
            .collect::<Vec<_>>();
        han_targets.sort_by_key(|(_, _, text)| text.text.as_deref().unwrap());
        assert_eq!(han_targets.len(), 2);
        let (first_id, first_transform, first_text) = han_targets[0];
        assert_eq!(first_id, reused);
        assert_eq!(first_text.text.as_deref(), Some("全身塑形"));
        assert_eq!(first_text.detected_font_size_px, Some(39.0));
        assert_eq!(
            (
                first_transform.x,
                first_transform.y,
                first_transform.width,
                first_transform.height,
                first_transform.rotation_deg,
            ),
            (10.0, 20.0, 208.0, 39.0, 0.0)
        );
        let (second_id, second_transform, second_text) = han_targets[1];
        assert_ne!(second_id, reused);
        assert_eq!(second_text.text.as_deref(), Some("蜜桃臀"));
        assert_eq!(second_text.detected_font_size_px, Some(24.0));
        assert_eq!(
            (
                second_transform.x,
                second_transform.y,
                second_transform.width,
                second_transform.height,
                second_transform.rotation_deg,
            ),
            (30.0, 70.0, 120.0, 24.0, 0.0)
        );

        let protected = page
            .nodes
            .values()
            .filter_map(|node| {
                let NodeKind::Text(text) = &node.kind else {
                    return None;
                };
                (text.detector.as_deref() == Some(SOURCE_GATE_PROTECTED_DETECTOR)).then_some(text)
            })
            .collect::<Vec<_>>();
        assert_eq!(protected.len(), 1);
        assert_eq!(protected[0].detected_font_size_px, None);
    }

    fn candidate(id: NodeId, bbox: [f32; 4], visible: bool, detector: &str) -> Node {
        Node {
            id,
            transform: Transform {
                x: bbox[0],
                y: bbox[1],
                width: bbox[2] - bbox[0],
                height: bbox[3] - bbox[1],
                rotation_deg: 0.0,
            },
            visible,
            kind: NodeKind::Text(TextData {
                detector: Some(detector.into()),
                ..Default::default()
            }),
        }
    }

    fn scene_with_nodes(nodes: Vec<Node>) -> (Scene, koharu_core::PageId) {
        let mut page = Page::new("page", 200, 100);
        let page_id = page.id;
        page.nodes = nodes.into_iter().map(|node| (node.id, node)).collect();
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        (scene, page_id)
    }

    #[test]
    #[ignore = "hanonly-pre-b1-red"]
    fn hanonly_pre_b1_red_t2_source_gate_ratio_contract() {
        let expected = [
            ("S25L4", [60, 80, 1140, 260]),
            ("S25L5", [50, 70, 1150, 270]),
            ("S25L6", [40, 60, 1160, 280]),
            ("S25L7", [30, 50, 1170, 290]),
        ];
        let node_id = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![candidate(
            node_id,
            [100.0, 120.0, 1100.0, 220.0],
            false,
            "detector",
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(1400, 400));
        let (actual, invalid) = source_gate_candidates(&image, &scene, page).unwrap();

        assert!(invalid.is_empty());
        assert_eq!(
            actual.len(),
            expected.len(),
            "production must emit one ordered candidate for each ratio"
        );
        for (index, (id, expected_bounds)) in expected.into_iter().enumerate() {
            assert_eq!(
                actual[index].node_id, node_id,
                "{id} candidate at index {index} must retain source identity and order"
            );
            assert_eq!(
                actual[index].crop_bounds, expected_bounds,
                "{id} candidate at index {index} must use exact outward-quantized bounds"
            );
        }
    }

    fn apply_ops(mut scene: Scene, ops: Vec<koharu_core::Op>) -> Scene {
        for mut op in ops {
            op.apply(&mut scene).unwrap();
        }
        scene
    }

    #[tokio::test]
    async fn observation_dispatch_preserves_detector_support_in_scene_mask() {
        let candidate_id = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![candidate(
            candidate_id,
            [20.25, 20.5, 120.75, 50.5],
            false,
            "detector",
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));
        let (candidates, invalid) = source_gate_candidates(&image, &scene, page).unwrap();
        assert!(invalid.is_empty());
        let crop = candidates
            .iter()
            .find(|candidate| candidate.node_id == candidate_id)
            .unwrap()
            .crop_bounds;
        let observed = observation(
            vec![detector(0, 9.25, 8.5, 99.5, 29.25)],
            vec![PpOcrLineObservation {
                detector_indices: vec![0],
                recognition: Some("中文".into()),
            }],
            vec![word("中文", 0, 11.0, 9.0, 98.0, 29.0)],
        );

        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| Ok(observed.clone()),
            |crops| {
                assert_eq!(crops.len(), 1);
                assert_eq!((crops[0].width(), crops[0].height()), (101, 31));
                std::future::ready(Ok(vec!["中文".into()]))
            },
        )
        .await
        .unwrap();
        let scene = apply_ops(scene, ops);
        let (lines, unsupported) =
            crate::pipeline::engines::support::eligible_lines_for_page(&scene, page);
        assert!(unsupported.is_empty());
        let selected = lines
            .into_iter()
            .filter(|(node_id, _)| {
                matches!(
                    scene.node(page, *node_id).map(|node| &node.kind),
                    Some(NodeKind::Text(text))
                        if text.detector.as_deref() == Some(SOURCE_GATE_TARGET_DETECTOR)
                )
            })
            .map(|(_, line)| line)
            .collect::<Vec<_>>();
        let mask = crate::pipeline::engines::support::line_support_mask(
            image.width(),
            image.height(),
            &selected,
        );
        let expected = [
            crop[0] as f32 + 9.0,
            crop[1] as f32 + 8.0,
            crop[0] as f32 + 100.0,
            crop[1] as f32 + 30.0,
        ];
        assert_ne!(
            crop,
            [
                expected[0].floor() as u32,
                expected[1].floor() as u32,
                expected[2].ceil() as u32,
                expected[3].ceil() as u32,
            ]
        );
        let target = scene.node(page, candidate_id).unwrap();
        assert_eq!(
            [
                target.transform.x,
                target.transform.y,
                target.transform.x + target.transform.width,
                target.transform.y + target.transform.height,
            ],
            expected
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].region.line_polygons,
            Some(vec![bbox_quad(expected)])
        );
        let expected_mask = crate::pipeline::engines::support::line_support_mask(
            image.width(),
            image.height(),
            &[selected[0].clone()],
        );
        assert_eq!(mask, expected_mask);
        let detector_mask = crate::pipeline::engines::support::line_support_mask(
            image.width(),
            image.height(),
            &[selected[0].clone()],
        );
        assert_eq!(mask, detector_mask);
    }

    #[tokio::test]
    async fn dispatch_classifies_only_words_from_active_layout_lines() {
        let candidate_id = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![candidate(
            candidate_id,
            [60.0, 35.0, 140.0, 65.0],
            false,
            "detector",
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));
        let (candidates, invalid) = source_gate_candidates(&image, &scene, page).unwrap();
        assert!(invalid.is_empty());
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.node_id == candidate_id)
            .unwrap();
        let [crop_left, crop_top, crop_right, _] = candidate.crop_bounds;
        let [layout_left, layout_top, layout_right, layout_bottom] = candidate.layout_bbox;
        let layout_local_left = layout_left - crop_left as f32;
        let layout_local_right = layout_right - crop_left as f32;
        let active_left = layout_left - crop_left as f32 + 5.0;
        let active_top = layout_top - crop_top as f32 + 5.0;
        let (outside_left, outside_right) =
            if (crop_right - crop_left) as f32 - layout_local_right >= 5.0 {
                (layout_local_right + 1.0, layout_local_right + 5.0)
            } else {
                assert!(layout_local_left >= 5.0);
                (layout_local_left - 5.0, layout_local_left - 1.0)
            };
        let mut outside_word = word(
            "PRODUCT",
            1,
            outside_left,
            active_top,
            outside_right,
            active_top + 10.0,
        );
        outside_word.confidence = 0.1;
        let observed = observation(
            vec![
                detector(
                    0,
                    active_left,
                    active_top,
                    layout_right - crop_left as f32 - 5.0,
                    layout_bottom - crop_top as f32 - 5.0,
                ),
                detector(
                    1,
                    outside_left,
                    active_top,
                    outside_right,
                    active_top + 10.0,
                ),
            ],
            vec![
                PpOcrLineObservation {
                    detector_indices: vec![0],
                    recognition: Some("中文".into()),
                },
                PpOcrLineObservation {
                    detector_indices: vec![1],
                    recognition: Some("PRODUCT".into()),
                },
            ],
            vec![
                word(
                    "中文",
                    0,
                    active_left + 1.0,
                    active_top + 1.0,
                    layout_right - crop_left as f32 - 6.0,
                    layout_bottom - crop_top as f32 - 6.0,
                ),
                outside_word,
            ],
        );

        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| Ok(observed.clone()),
            |_| std::future::ready(Ok(vec!["中文规范".into()])),
        )
        .await
        .unwrap();

        let scene = apply_ops(scene, ops);
        assert!(matches!(
            scene.node(page, candidate_id).map(|node| &node.kind),
            Some(NodeKind::Text(text))
                if text.detector.as_deref() == Some(SOURCE_GATE_TARGET_DETECTOR)
                    && text.text.as_deref() == Some("中文规范")
        ));
    }

    #[test]
    fn diagnostic_capture_rejects_nested_start_and_recovers() {
        let outer = SourceGateDiagnosticCapture::start();
        assert!(std::panic::catch_unwind(SourceGateDiagnosticCapture::start).is_err());
        drop(outer);
        let recovered = SourceGateDiagnosticCapture::start();
        drop(recovered);
    }

    #[tokio::test]
    async fn dispatch_records_single_input_diagnostic() {
        let candidate_id = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![candidate(
            candidate_id,
            [20.0, 20.0, 120.0, 50.0],
            false,
            "detector",
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));
        let expected_hash = rgba_fingerprint(&image);
        let diagnostics = SourceGateDiagnosticCapture::start();

        let _ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| {
                Ok(observation_from_words(vec![word(
                    "中文", 0, 0.0, 0.0, 80.0, 20.0,
                )]))
            },
            |_| std::future::ready(Ok(vec!["中文".into()])),
        )
        .await
        .unwrap();

        let input_events = diagnostics
            .take()
            .into_iter()
            .filter_map(|event| match event {
                SourceGateDiagnosticEvent::Input {
                    width,
                    height,
                    decoded_rgba_hash,
                    ..
                } => Some((width, height, decoded_rgba_hash)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(input_events, vec![(200, 100, expected_hash)]);
    }

    #[tokio::test]
    async fn invalid_layout_is_rejected_before_pp_and_vl() {
        let candidate_id = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![candidate(
            candidate_id,
            [-1.0, 10.0, 80.0, 40.0],
            false,
            "detector",
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));
        let pp_calls = AtomicUsize::new(0);
        let vl_calls = AtomicUsize::new(0);

        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| {
                pp_calls.fetch_add(1, Ordering::Relaxed);
                Ok(observation_from_words(vec![word(
                    "中文", 0, 0.0, 0.0, 40.0, 20.0,
                )]))
            },
            |crops| {
                vl_calls.fetch_add(crops.len(), Ordering::Relaxed);
                std::future::ready(Ok(vec!["中文".into()]))
            },
        )
        .await
        .unwrap();

        let scene = apply_ops(scene, ops);
        assert_eq!(pp_calls.load(Ordering::Relaxed), 0);
        assert_eq!(vl_calls.load(Ordering::Relaxed), 0);
        assert!(scene.node(page, candidate_id).is_none());
    }

    #[tokio::test]
    async fn padding_detector_never_becomes_an_accepted_target() {
        let candidate_id = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![candidate(
            candidate_id,
            [20.0, 20.0, 120.0, 50.0],
            false,
            "detector",
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));

        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| {
                Ok(observation_from_words(vec![word(
                    "邻字", 0, 2.0, 3.0, 10.0, 12.0,
                )]))
            },
            |_| std::future::ready(Ok(vec!["正文".into()])),
        )
        .await
        .unwrap();

        let scene = apply_ops(scene, ops);
        assert!(scene.node(page, candidate_id).is_none());
        assert!(
            crate::pipeline::engines::support::text_nodes(&scene, page)
                .into_iter()
                .all(|(_, _, text)| {
                    text.detector.as_deref() != Some(SOURCE_GATE_TARGET_DETECTOR)
                })
        );
    }

    #[tokio::test]
    async fn production_gate_removes_english_and_keeps_only_chinese() {
        let english = NodeId::new();
        let mixed = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![
            candidate(english, [0.0, 0.0, 80.0, 20.0], false, "detector"),
            candidate(mixed, [10.0, 30.0, 110.0, 50.0], false, "detector"),
        ]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));
        let vl_calls = AtomicUsize::new(0);

        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |node_id, _| {
                if node_id == english {
                    Ok(observation_from_words(vec![word(
                        "English", 0, 0.0, 0.0, 80.0, 20.0,
                    )]))
                } else {
                    Ok(observation_from_words(vec![
                        word("Peach", 0, 5.0, 5.0, 45.0, 25.0),
                        word("蜜桃臀", 0, 50.0, 5.0, 105.0, 25.0),
                    ]))
                }
            },
            |crops| {
                vl_calls.fetch_add(crops.len(), Ordering::Relaxed);
                std::future::ready(Ok(vec!["Peach蜜桃臀".to_string()]))
            },
        )
        .await
        .unwrap();

        let scene = apply_ops(scene, ops);
        assert_eq!(vl_calls.load(Ordering::Relaxed), 1);
        assert!(scene.node(page, english).is_none());
        let mixed_node = scene.node(page, mixed).unwrap();
        assert!(mixed_node.visible);
        let NodeKind::Text(mixed_text) = &mixed_node.kind else {
            panic!("expected text")
        };
        assert_eq!(mixed_text.text.as_deref(), Some("蜜桃臀"));
        assert_eq!(mixed_text.line_polygons.as_ref().unwrap().len(), 1);
        assert_eq!(
            crate::pipeline::engines::support::text_nodes(&scene, page).len(),
            1
        );
        let protected =
            crate::pipeline::engines::support::protected_source_lines_for_page(&scene, page)
                .into_iter()
                .filter(|(node_id, _)| {
                    matches!(
                        scene.node(page, *node_id).map(|node| &node.kind),
                        Some(NodeKind::Text(text))
                            if text.detector.as_deref() == Some(SOURCE_GATE_PROTECTED_DETECTOR)
                    )
                })
                .collect::<Vec<_>>();
        assert_eq!(
            protected
                .iter()
                .filter(|(id, _)| {
                    matches!(
                        scene.node(page, *id).map(|node| &node.kind),
                        Some(NodeKind::Text(text))
                            if text.detector.as_deref() == Some(SOURCE_GATE_PROTECTED_DETECTOR)
                    )
                })
                .count(),
            1
        );
        assert!(protected.iter().any(|(_, line)| line.text == "Peach"));
    }

    #[tokio::test]
    async fn production_gate_h3_fallback_uses_exact_disjoint_pp_word_masks() {
        let candidate_id = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![candidate(
            candidate_id,
            [10.0, 10.0, 110.0, 60.0],
            false,
            "detector",
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));
        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| {
                Ok(observation_from_words(vec![
                    word("PEACH", 0, 12.0, 12.0, 45.0, 28.0),
                    word("HIp", 0, 53.0, 12.0, 78.0, 28.0),
                    word("蜜桃臀", 1, 12.0, 35.0, 50.0, 55.0),
                ]))
            },
            |_| std::future::ready(Ok(vec!["PEACH HIp\n蜜桃臀".into()])),
        )
        .await
        .unwrap();

        let scene = apply_ops(scene, ops);
        let protected =
            crate::pipeline::engines::support::protected_source_lines_for_page(&scene, page)
                .into_iter()
                .filter(|(node_id, _)| {
                    matches!(
                        scene.node(page, *node_id).map(|node| &node.kind),
                        Some(NodeKind::Text(text))
                            if text.detector.as_deref() == Some(SOURCE_GATE_PROTECTED_DETECTOR)
                    )
                })
                .collect::<Vec<_>>();
        assert_eq!(protected.len(), 2);
        let mask = crate::pipeline::engines::support::line_support_mask(
            image.width(),
            image.height(),
            &protected
                .iter()
                .map(|(_, line)| line.clone())
                .collect::<Vec<_>>(),
        );
        for y in 0..image.height() {
            for x in 0..image.width() {
                let expected =
                    ((12..45).contains(&x) || (53..78).contains(&x)) && (12..28).contains(&y);
                assert_eq!(mask.get_pixel(x, y).0[0] != 0, expected, "pixel {x},{y}");
            }
        }
    }

    #[tokio::test]
    async fn production_gate_is_idempotent_for_already_accepted_nodes() {
        let accepted = NodeId::new();
        let protected = NodeId::new();
        let mut accepted_node = candidate(
            accepted,
            [20.0, 20.0, 60.0, 40.0],
            true,
            "pp-ocr-v5-source-gate",
        );
        let NodeKind::Text(text) = &mut accepted_node.kind else {
            unreachable!()
        };
        text.text = Some("中文".into());
        let mut protected_node = candidate(
            protected,
            [0.0, 20.0, 18.0, 40.0],
            false,
            "pp-ocr-v5-source-gate-protected",
        );
        let NodeKind::Text(text) = &mut protected_node.kind else {
            unreachable!()
        };
        text.text = Some("AI".into());
        let (scene, page) = scene_with_nodes(vec![accepted_node, protected_node]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));
        let pp_calls = AtomicUsize::new(0);

        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| {
                pp_calls.fetch_add(1, Ordering::Relaxed);
                Ok(observation_from_words(Vec::new()))
            },
            |_| std::future::ready(Err(anyhow::anyhow!("accepted target must not call VL"))),
        )
        .await
        .unwrap();

        assert!(ops.is_empty());
        assert_eq!(pp_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn production_gate_empty_targets_preserves_repair_brush_and_inpainted_result() {
        let english = NodeId::new();
        let source = Node {
            id: NodeId::new(),
            transform: Transform::default(),
            visible: true,
            kind: NodeKind::Image(ImageData {
                role: ImageRole::Source,
                blob: BlobRef::new("source"),
                opacity: 1.0,
                natural_width: 200,
                natural_height: 100,
                name: None,
            }),
        };
        let image_node = |role, name: &str| Node {
            id: NodeId::new(),
            transform: Transform::default(),
            visible: true,
            kind: NodeKind::Image(ImageData {
                role,
                blob: BlobRef::new(name),
                opacity: 1.0,
                natural_width: 200,
                natural_height: 100,
                name: None,
            }),
        };
        let mask_node = |role, name: &str| Node {
            id: NodeId::new(),
            transform: Transform::default(),
            visible: true,
            kind: NodeKind::Mask(MaskData {
                role,
                blob: BlobRef::new(name),
            }),
        };
        let (scene, page) = scene_with_nodes(vec![
            source,
            image_node(ImageRole::Inpainted, "inpainted"),
            image_node(ImageRole::Rendered, "rendered"),
            mask_node(MaskRole::BrushInpaint, "brush"),
            mask_node(MaskRole::Segment, "segment"),
            mask_node(MaskRole::Bubble, "bubble"),
            candidate(english, [0.0, 0.0, 80.0, 20.0], false, "detector"),
        ]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));

        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| {
                Ok(observation_from_words(vec![word(
                    "English", 0, 0.0, 0.0, 80.0, 20.0,
                )]))
            },
            |_| std::future::ready(Err(anyhow::anyhow!("pure English must not call VL"))),
        )
        .await
        .unwrap();
        let scene = apply_ops(scene, ops);

        assert!(
            crate::pipeline::engines::support::find_image_node(&scene, page, ImageRole::Source)
                .is_some()
        );
        assert!(
            crate::pipeline::engines::support::find_mask_node(&scene, page, MaskRole::BrushInpaint)
                .is_some()
        );
        assert!(
            crate::pipeline::engines::support::find_image_node(&scene, page, ImageRole::Inpainted)
                .is_some()
        );
        assert!(
            crate::pipeline::engines::support::find_image_node(&scene, page, ImageRole::Rendered)
                .is_none()
        );
        assert!(
            crate::pipeline::engines::support::find_mask_node(&scene, page, MaskRole::Segment)
                .is_none()
        );
        assert!(
            crate::pipeline::engines::support::find_mask_node(&scene, page, MaskRole::Bubble)
                .is_none()
        );
    }

    #[tokio::test]
    async fn source_gate_rejection_reason_vl_batch_mismatch_is_atomic() {
        let first = NodeId::new();
        let second = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![
            candidate(first, [0.0, 0.0, 50.0, 20.0], false, "detector"),
            candidate(second, [60.0, 0.0, 110.0, 20.0], false, "detector"),
        ]);
        let before = serde_json::to_vec(&scene).unwrap();
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));

        let error = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| {
                Ok(observation_from_words(vec![word(
                    "中文", 0, 0.0, 0.0, 40.0, 20.0,
                )]))
            },
            |_| std::future::ready(Ok(Vec::new())),
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("source gate OCR count mismatch"));
        assert_eq!(serde_json::to_vec(&scene).unwrap(), before);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn source_gate_debug_tracing_omits_ocr_and_translation_body() {
        let secret_ocr = "机密正文不可记录";
        let secret_translation = "SECRET_TRANSLATION_BODY_MUST_NOT_APPEAR";
        let node_id = NodeId::new();
        let (scene, page) = scene_with_nodes(vec![candidate(
            node_id,
            [10.0, 10.0, 110.0, 40.0],
            false,
            "detector",
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::new(200, 100));
        let (candidates, invalid) = source_gate_candidates(&image, &scene, page).unwrap();
        assert!(invalid.is_empty());
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.node_id == node_id)
            .unwrap();
        let [crop_left, crop_top, _, _] = candidate.crop_bounds;
        let [layout_left, layout_top, layout_right, layout_bottom] = candidate.layout_bbox;
        let observed = observation_from_words(vec![word(
            secret_ocr,
            0,
            layout_left - crop_left as f32,
            layout_top - crop_top as f32,
            layout_right - crop_left as f32,
            layout_bottom - crop_top as f32,
        )]);
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(CapturedLogWriter(bytes.clone()))
            .finish();
        let _subscriber = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        let diagnostics = SourceGateDiagnosticCapture::start();

        let ops = dispatch_source_gate(
            &image,
            &scene,
            page,
            |_, _| Ok(observed.clone()),
            |_| std::future::ready(Ok(vec![secret_ocr.to_string()])),
        )
        .await
        .unwrap();
        assert!(!ops.is_empty());

        let diagnostic_events = diagnostics.take();
        tracing::debug!(
            target: "koharu::source_gate",
            event_count = diagnostic_events.len(),
            "source_gate.privacy_capture"
        );
        let logs = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        let diagnostic_json = serde_json::to_string(&diagnostic_events).unwrap();
        for output in [&logs, &diagnostic_json] {
            assert!(!output.contains(secret_ocr));
            assert!(!output.contains(secret_translation));
        }
        assert!(logs.contains("source_gate.privacy_capture"));
        for event in [
            "layout_candidate",
            "crop",
            "pp_summary",
            "vl_summary",
            "decision",
        ] {
            assert!(
                diagnostic_json.contains(&format!("\"event\":\"{event}\"")),
                "missing diagnostic event: {event}"
            );
        }
        assert!(diagnostic_json.contains("\"crop_rgba_hash\""));
    }
}
