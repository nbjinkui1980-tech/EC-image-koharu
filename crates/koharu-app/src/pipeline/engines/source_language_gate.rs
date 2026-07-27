use std::{collections::HashSet, sync::Mutex};

#[cfg(test)]
use std::sync::{Arc, OnceLock};

use anyhow::{Result, ensure};
use async_trait::async_trait;
use image::DynamicImage;
use koharu_core::{
    ImageRole, MaskRole, Node, NodeDataPatch, NodeId, NodeKind, NodePatch, Op, PageId, Scene,
    TextData, TextDataPatch, Transform,
};
use koharu_llm::paddleocr_vl::{PaddleOcrVl, PaddleOcrVlTask};
use koharu_ml::pp_ocr_v5::{PpOcrV5, PpOcrWordBox};
use serde::{Deserialize, Serialize};

use crate::app::shared_llama_backend;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    SOURCE_GATE_PROTECTED_DETECTOR, SOURCE_GATE_TARGET_DETECTOR, contains_han,
    contains_protected_latin_word, find_mask_node, load_source_image,
};

const MIN_WORD_CONFIDENCE: f32 = 0.5;
const MAX_NEW_TOKENS: usize = 256;

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
    pub(super) line_polygon: [[f32; 2]; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SourceSelection {
    pub(super) targets: Vec<SourceTarget>,
    pub(super) protected_lines: Vec<(String, [f32; 4])>,
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
            | Self::AcceptedIsolatedProtectedLatinGeometry { .. } => "completed",
            Self::VlBatchError => "batch_error",
        }
    }

    #[cfg(test)]
    pub(in crate::pipeline) fn fallback(&self) -> &'static str {
        if matches!(self, Self::AcceptedIsolatedProtectedLatinGeometry { .. }) {
            "isolated_protected_latin_geometry"
        } else {
            "none"
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
    },
    PpSummary {
        node_id: NodeId,
        words: Vec<PpWordDiagnostic>,
    },
    VlSummary {
        node_id: NodeId,
        contains_han: bool,
        character_count: usize,
        line_count: usize,
    },
    Decision {
        node_id: NodeId,
        decision: SourceGateDecision,
    },
}

#[cfg(test)]
#[derive(Clone, Debug, Serialize)]
pub(in crate::pipeline) struct PpWordDiagnostic {
    pub line_index: usize,
    pub character_count: usize,
    pub script: &'static str,
    pub confidence: f32,
    pub bbox: [f32; 4],
}

#[cfg(test)]
type DiagnosticSink = Arc<Mutex<Vec<SourceGateDiagnosticEvent>>>;

#[cfg(test)]
static DIAGNOSTIC_SINK: OnceLock<Mutex<Option<DiagnosticSink>>> = OnceLock::new();

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
        let mut active = DIAGNOSTIC_SINK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("source gate diagnostic sink mutex poisoned");
        assert!(
            active.is_none(),
            "source gate diagnostic capture already active"
        );
        *active = Some(events.clone());
        Self { events }
    }

    pub(in crate::pipeline) fn take(&self) -> Vec<SourceGateDiagnosticEvent> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .expect("source gate diagnostic events mutex poisoned"),
        )
    }
}

#[cfg(test)]
impl Drop for SourceGateDiagnosticCapture {
    fn drop(&mut self) {
        let mut active = DIAGNOSTIC_SINK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("source gate diagnostic sink mutex poisoned");
        if active
            .as_ref()
            .is_some_and(|sink| Arc::ptr_eq(sink, &self.events))
        {
            *active = None;
        }
    }
}

#[cfg(test)]
fn record_diagnostic(event: SourceGateDiagnosticEvent) {
    let sink = DIAGNOSTIC_SINK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("source gate diagnostic sink mutex poisoned")
        .clone();
    if let Some(sink) = sink {
        sink.lock()
            .expect("source gate diagnostic events mutex poisoned")
            .push(event);
    }
}

fn classify_pp_words(words: &[PpOcrWordBox]) -> Result<(), SourceGateRejectReason> {
    if words.is_empty() {
        return Err(SourceGateRejectReason::PpNoWords);
    }
    if words.iter().any(|word| !word.confidence.is_finite()) {
        return Err(SourceGateRejectReason::PpNonFiniteConfidence);
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
    Ok(())
}

fn bbox_quad([left, top, right, bottom]: [f32; 4]) -> [[f32; 2]; 4] {
    [[left, top], [right, top], [right, bottom], [left, bottom]]
}

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

fn isolated_latin_scalar_allowed(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '\'')
}

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
                    line_polygon: bbox_quad(bbox),
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
        .map(|(_, word)| (word.text.clone(), word.bbox))
        .collect::<Vec<_>>();
    if indexed_targets.iter().any(|(_, _, target)| {
        protected_lines
            .iter()
            .any(|(_, bbox)| bboxes_intersect(target.bbox, *bbox))
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

struct SourceGateCandidate {
    node_id: NodeId,
    crop: DynamicImage,
    crop_bounds: [u32; 4],
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
                    "source_gate.crop"
                );
            }
            #[cfg(test)]
            record_diagnostic(SourceGateDiagnosticEvent::Crop {
                candidate_index,
                node_id: *node_id,
                bounds: [left, top, right, bottom],
                crop_rgba_hash: rgba_fingerprint(&crop),
            });
            candidates.push(SourceGateCandidate {
                node_id: *node_id,
                crop,
                crop_bounds: [left, top, right, bottom],
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
        line_polygons: Some(vec![target.line_polygon]),
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
    let first_polygon = first.line_polygon;
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
                line_polygons: Some(Some(vec![first_polygon])),
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
    for (text, bbox) in selection.protected_lines {
        let target = SourceTarget {
            text,
            bbox,
            line_polygon: bbox_quad(bbox),
        };
        let transform = target_transform(bbox);
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

pub(crate) async fn dispatch_source_gate<WordBoxes, Validate, Fut>(
    image: &DynamicImage,
    scene: &Scene,
    page: PageId,
    mut word_boxes: WordBoxes,
    mut validate: Validate,
) -> Result<Vec<Op>>
where
    WordBoxes: FnMut(NodeId, &DynamicImage) -> Result<Vec<PpOcrWordBox>>,
    Validate: FnMut(Vec<DynamicImage>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>>>,
{
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
        let words = word_boxes(candidate.node_id, &candidate.crop)?;
        tracing::debug!(
            target: "koharu::source_gate",
            node_id = ?candidate.node_id,
            word_count = words.len(),
            "source_gate.pp_summary"
        );
        #[cfg(test)]
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
                        character_count: word.text.chars().filter(|ch| !ch.is_whitespace()).count(),
                        script,
                        confidence: word.confidence,
                        bbox: word.bbox,
                    }
                })
                .collect(),
        });
        for word in &words {
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
        match classify_pp_words(&words) {
            Ok(()) => {}
            Err(reason) => {
                trace_decision(
                    candidate.node_id,
                    &SourceGateDecision::RejectedBeforeVl { reason },
                );
                candidate_failures.insert(candidate.node_id);
                continue;
            }
        }

        let mut vl_texts = validate(vec![candidate.crop.clone()]).await?;
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
            character_count: vl_text.chars().filter(|ch| !ch.is_whitespace()).count(),
            line_count: vl_text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
        });
        match select_chinese_target_with_fallback(
            &vl_text,
            &words,
            candidate.crop_bounds,
            image.width(),
            image.height(),
        ) {
            Ok((selection, decision)) => {
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
        #[cfg(test)]
        record_diagnostic(SourceGateDiagnosticEvent::Input {
            backend: if self.cpu { "cpu" } else { "prefer_gpu" },
            width: image.width(),
            height: image.height(),
            decoded_rgba_hash: rgba_fingerprint(&image),
        });
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
            |_, crop| pp.word_boxes(crop),
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
    use koharu_ml::pp_ocr_v5::PpOcrWordBox;

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
            vec![("S".into(), [10.0, 20.0, 18.0, 30.0])]
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
            vec![("Peach".into(), [10.0, 20.0, 50.0, 40.0])]
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
                ("PEACH".into(), [12.0, 22.0, 50.0, 38.0]),
                ("HIp".into(), [58.0, 22.0, 82.0, 38.0]),
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
                SourceGateRejectReason::PpNoHanUnprotected,
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
        assert!(selection.protected_lines.iter().all(|(_, bbox)| {
            bbox[2] <= selection.targets[0].bbox[0] || bbox[0] >= selection.targets[0].bbox[2]
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
                        line_polygon: bbox_quad([10.0, 20.0, 218.0, 59.0]),
                    },
                    SourceTarget {
                        text: "蜜桃臀".into(),
                        bbox: [30.0, 70.0, 150.0, 94.0],
                        line_polygon: bbox_quad([30.0, 70.0, 150.0, 94.0]),
                    },
                ],
                protected_lines: vec![("PEACH HIP".into(), [30.0, 5.0, 150.0, 18.0])],
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
                    Ok(vec![word("English", 0, 0.0, 0.0, 80.0, 20.0)])
                } else {
                    Ok(vec![
                        word("Peach", 0, 0.0, 0.0, 40.0, 20.0),
                        word("蜜桃臀", 0, 45.0, 0.0, 100.0, 20.0),
                    ])
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
                Ok(vec![
                    word("PEACH", 0, 2.0, 2.0, 35.0, 18.0),
                    word("HIp", 0, 43.0, 2.0, 68.0, 18.0),
                    word("蜜桃臀", 1, 2.0, 25.0, 40.0, 45.0),
                ])
            },
            |_| std::future::ready(Ok(vec!["PEACH HIP\n蜜桃臀".into()])),
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
                    ((2..35).contains(&x) || (43..68).contains(&x)) && (2..18).contains(&y);
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
                Ok(Vec::new())
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
            |_, _| Ok(vec![word("English", 0, 0.0, 0.0, 80.0, 20.0)]),
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
            |_, _| Ok(vec![word("中文", 0, 0.0, 0.0, 40.0, 20.0)]),
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
            |_, _| Ok(vec![word(secret_ocr, 0, 0.0, 0.0, 80.0, 20.0)]),
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
