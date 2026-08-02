//! Shared helpers used by multiple engine implementations.
//!
//! The patterns here map `koharu-ml` / `koharu-llm` outputs (plain
//! `TextRegion`s, `DynamicImage`s) into `Op` sequences that mutate the scene.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{Context, Result, bail, ensure};
use image::{DynamicImage, GenericImageView, GrayImage, Luma};
use imageproc::{drawing::draw_polygon_mut, point::Point};
use koharu_core::{
    BlobRef, ImageData, ImageRole, MaskData, MaskRole, Node, NodeDataPatch, NodeId, NodeKind,
    NodePatch, Op, PageId, ReadingOrder, Region, Scene, TextData, TextDataPatch, Transform,
};
use koharu_ml::types::TextRegion;

use crate::{blobs::BlobStore, config::SourceTextPolicy};

#[cfg(test)]
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::{
    sync::{Arc, Mutex, OnceLock},
    thread::ThreadId,
};

pub const SOURCE_GATE_TARGET_DETECTOR: &str = "pp-ocr-v5-source-gate";
pub const SOURCE_GATE_PROTECTED_DETECTOR: &str = "pp-ocr-v5-source-gate-protected";

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::pipeline) enum EraseDiagnosticStage {
    SegmentProbability,
    SegmentRefined,
    SegmentAllowedSupport,
    SegmentFinal,
    InpaintInputSegment,
    InpaintAllowedSupport,
    InpaintPreExpandFiltered,
    InpaintBackendExpanded,
    InpaintFinal,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::pipeline) enum EraseDiagnosticBranch {
    Region,
    HanOnly,
    AllText,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::pipeline) struct EraseMaskDiagnostic {
    pub width: u32,
    pub height: u32,
    pub grayscale_blake3: String,
    pub nonzero_pixels: u64,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::pipeline) struct EraseDiagnosticEvent {
    pub stage: EraseDiagnosticStage,
    pub branch: EraseDiagnosticBranch,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub mask: Option<EraseMaskDiagnostic>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub returns_some: Option<bool>,
}

#[cfg(test)]
fn deserialize_required_option<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

#[cfg(test)]
type EraseDiagnosticSink = Arc<Mutex<Vec<EraseDiagnosticEvent>>>;

#[cfg(test)]
type EraseFinalMaskSink = Arc<Mutex<Option<GrayImage>>>;

#[cfg(test)]
struct ActiveEraseDiagnosticSink {
    owner: ThreadId,
    events: EraseDiagnosticSink,
    final_mask: EraseFinalMaskSink,
}

#[cfg(test)]
static ERASE_DIAGNOSTIC_SINK: OnceLock<Mutex<Option<ActiveEraseDiagnosticSink>>> = OnceLock::new();

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::pipeline) struct EraseDiagnosticCaptureActive;

#[cfg(test)]
pub(in crate::pipeline) struct EraseDiagnosticCapture {
    owner: ThreadId,
    events: EraseDiagnosticSink,
    final_mask: EraseFinalMaskSink,
}

#[cfg(test)]
impl EraseDiagnosticCapture {
    pub(in crate::pipeline) fn start() -> std::result::Result<Self, EraseDiagnosticCaptureActive> {
        let owner = std::thread::current().id();
        let events = Arc::new(Mutex::new(Vec::new()));
        let final_mask = Arc::new(Mutex::new(None));
        let mut active = ERASE_DIAGNOSTIC_SINK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.is_some() {
            return Err(EraseDiagnosticCaptureActive);
        }
        *active = Some(ActiveEraseDiagnosticSink {
            owner,
            events: events.clone(),
            final_mask: final_mask.clone(),
        });
        Ok(Self {
            owner,
            events,
            final_mask,
        })
    }

    pub(in crate::pipeline) fn take(&self) -> Vec<EraseDiagnosticEvent> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    pub(in crate::pipeline) fn take_inpaint_final_mask(&self) -> Option<GrayImage> {
        self.final_mask
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

#[cfg(test)]
impl Drop for EraseDiagnosticCapture {
    fn drop(&mut self) {
        let mut active = ERASE_DIAGNOSTIC_SINK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active
            .as_ref()
            .is_some_and(|sink| sink.owner == self.owner && Arc::ptr_eq(&sink.events, &self.events))
        {
            *active = None;
        }
    }
}

#[cfg(test)]
pub(in crate::pipeline) fn record_erase_diagnostic(
    stage: EraseDiagnosticStage,
    branch: EraseDiagnosticBranch,
    mask: Option<&GrayImage>,
    returns_some: Option<bool>,
) {
    let owner = std::thread::current().id();
    let sink = ERASE_DIAGNOSTIC_SINK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|sink| sink.owner == owner)
        .map(|sink| (sink.events.clone(), sink.final_mask.clone()));
    if let Some((sink, final_mask)) = sink {
        if stage == EraseDiagnosticStage::InpaintFinal
            && let Some(mask) = mask
        {
            *final_mask
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(mask.clone());
        }
        let mask = mask.map(|mask| EraseMaskDiagnostic {
            width: mask.width(),
            height: mask.height(),
            grayscale_blake3: blake3::hash(mask.as_raw()).to_hex().to_string(),
            nonzero_pixels: mask.pixels().filter(|pixel| pixel.0[0] != 0).count() as u64,
        });
        sink.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(EraseDiagnosticEvent {
                stage,
                branch,
                mask,
                returns_some,
            });
    }
}

#[derive(Clone, Debug)]
pub struct EligibleTextLine {
    pub line_index: usize,
    pub text: String,
    pub region: koharu_ml::types::TextRegion,
}

#[derive(Clone, Debug)]
pub struct UnsupportedTextGeometry {
    pub node_id: NodeId,
    pub direction: Option<koharu_core::TextDirection>,
    pub rotation_deg: f32,
    pub line_count: usize,
}

// ---------------------------------------------------------------------------
// Read helpers
// ---------------------------------------------------------------------------

/// Find the Source image node on `page`. Returns `(node_id, image_data)`.
/// Every valid page has exactly one; absence means the page is malformed.
pub fn source_node(scene: &Scene, page: PageId) -> Result<(NodeId, &ImageData)> {
    let page = scene
        .page(page)
        .with_context(|| format!("page {} not found", page))?;
    for (id, node) in page.nodes.iter() {
        if let NodeKind::Image(img) = &node.kind
            && img.role == ImageRole::Source
        {
            return Ok((*id, img));
        }
    }
    anyhow::bail!("page has no Source image node")
}

/// Load the source image bytes + decoded image for `page`.
pub fn load_source_image(scene: &Scene, page: PageId, blobs: &BlobStore) -> Result<DynamicImage> {
    let (_, img_data) = source_node(scene, page)?;
    blobs.load_image(&img_data.blob)
}

/// Zero every mask pixel outside `region` while preserving the source
/// dimensions. Inpainting engines use this to limit repair-brush work.
pub fn clip_mask_to_region(mask: &DynamicImage, region: &koharu_core::Region) -> DynamicImage {
    DynamicImage::ImageLuma8(clip_gray_mask_to_region(&mask.to_luma8(), region))
}

/// Gray-image variant of [`clip_mask_to_region`].
pub fn clip_gray_mask_to_region(src: &GrayImage, region: &koharu_core::Region) -> GrayImage {
    let (width, height) = src.dimensions();
    let x0 = region.x.min(width);
    let y0 = region.y.min(height);
    let x1 = region.x.saturating_add(region.width).min(width);
    let y1 = region.y.saturating_add(region.height).min(height);

    let mut clipped = GrayImage::new(width, height);
    for y in y0..y1 {
        for x in x0..x1 {
            clipped.put_pixel(x, y, Luma([src.get_pixel(x, y).0[0]]));
        }
    }
    clipped
}

/// Find a node of `Image { role }` on `page`, if any.
pub fn find_image_node(scene: &Scene, page: PageId, role: ImageRole) -> Option<(NodeId, BlobRef)> {
    let page = scene.page(page)?;
    page.nodes.iter().find_map(|(id, node)| match &node.kind {
        NodeKind::Image(img) if img.role == role => Some((*id, img.blob.clone())),
        _ => None,
    })
}

/// Find a node of `Mask { role }` on `page`, if any.
pub fn find_mask_node(scene: &Scene, page: PageId, role: MaskRole) -> Option<(NodeId, BlobRef)> {
    let page = scene.page(page)?;
    page.nodes.iter().find_map(|(id, node)| match &node.kind {
        NodeKind::Mask(mask) if mask.role == role => Some((*id, mask.blob.clone())),
        _ => None,
    })
}

/// Collect `(NodeId, &Transform, &TextData)` for every text node on `page`,
/// in stacking order.
pub fn text_nodes(scene: &Scene, page: PageId) -> Vec<(NodeId, &Transform, &TextData)> {
    let Some(page) = scene.page(page) else {
        return Vec::new();
    };
    page.nodes
        .iter()
        .filter(|(_, node)| node.visible)
        .filter_map(|(id, node)| match &node.kind {
            NodeKind::Text(t) => Some((*id, &node.transform, t)),
            _ => None,
        })
        .collect()
}

pub fn contains_han(text: &str) -> bool {
    use icu_properties::{CodePointMapData, props::Script};

    let scripts = CodePointMapData::<Script>::new();
    text.chars().any(|ch| scripts.get(ch) == Script::Han)
}

pub fn contains_protected_latin_word(text: &str) -> bool {
    use icu_properties::{CodePointMapData, props::Script};

    let scripts = CodePointMapData::<Script>::new();
    let mut letters = 0;
    for ch in text.chars() {
        if scripts.get(ch) == Script::Latin && ch.is_alphabetic() {
            letters += 1;
        } else if matches!(ch, '-' | '\'' | '’') && letters > 0 {
            continue;
        } else {
            if letters >= 2 {
                return true;
            }
            letters = 0;
        }
    }
    letters >= 2
}

pub fn eligible_text_lines(
    transform: &Transform,
    text: &TextData,
    image_width: u32,
    image_height: u32,
) -> Option<Vec<EligibleTextLine>> {
    let body = text.text.as_deref().unwrap_or_default();
    let lines = body
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            (!line.is_empty()).then_some((index, line))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return Some(Vec::new());
    }

    if lines
        .iter()
        .any(|(_, line)| contains_han(line) && contains_protected_latin_word(line))
    {
        return None;
    }

    let node_bbox = safe_node_bbox(transform, image_width, image_height)?;
    let han_count = lines.iter().filter(|(_, line)| contains_han(line)).count();
    if han_count == 0 {
        return Some(Vec::new());
    }

    if lines.len() == 1
        && han_count == 1
        && text.detector.as_deref() == Some(SOURCE_GATE_TARGET_DETECTOR)
        && text
            .line_polygons
            .as_ref()
            .is_some_and(|value| value.len() > 1)
    {
        let polygons = text
            .line_polygons
            .as_ref()?
            .iter()
            .map(|quad| {
                safe_mixed_line_bbox(quad, transform, image_width, image_height).map(bbox_quad)
            })
            .collect::<Option<Vec<_>>>()?;
        let (line_index, line) = lines[0];
        let mut eligible = eligible_line(text, line_index, line, node_bbox, false);
        eligible.region.line_polygons = Some(polygons);
        return Some(vec![eligible]);
    }

    let safe_polygons = text.line_polygons.as_ref().and_then(|polygons| {
        if polygons.len() != lines.len() {
            return None;
        }
        polygons
            .iter()
            .map(|quad| safe_mixed_line_bbox(quad, transform, image_width, image_height))
            .collect::<Option<Vec<_>>>()
    });

    if han_count != lines.len() {
        if text
            .rotation_deg
            .is_some_and(|rotation| !rotation.is_finite() || rotation != 0.0)
        {
            return None;
        }
        let boxes = safe_polygons?;
        return Some(
            lines
                .into_iter()
                .zip(boxes)
                .filter(|((_, line), _)| contains_han(line))
                .map(|((line_index, line), bbox)| eligible_line(text, line_index, line, bbox, true))
                .collect(),
        );
    }

    if let Some(boxes) = safe_polygons {
        return Some(
            lines
                .into_iter()
                .zip(boxes)
                .map(|((line_index, line), bbox)| eligible_line(text, line_index, line, bbox, true))
                .collect(),
        );
    }

    Some(
        lines
            .into_iter()
            .map(|(line_index, line)| eligible_line(text, line_index, line, node_bbox, false))
            .collect(),
    )
}

pub fn eligible_lines_for_page(
    scene: &Scene,
    page: PageId,
) -> (
    Vec<(NodeId, EligibleTextLine)>,
    Vec<UnsupportedTextGeometry>,
) {
    let Some(page_ref) = scene.page(page) else {
        return (Vec::new(), Vec::new());
    };
    let (image_width, image_height) = (page_ref.width, page_ref.height);
    let mut lines = Vec::new();
    let mut unsupported = Vec::new();
    for (id, transform, text) in text_nodes(scene, page) {
        match eligible_text_lines(transform, text, image_width, image_height) {
            Some(found) => lines.extend(found.into_iter().map(|line| (id, line))),
            None => unsupported.push(UnsupportedTextGeometry {
                node_id: id,
                direction: text.source_direction,
                rotation_deg: text.rotation_deg.unwrap_or(transform.rotation_deg),
                line_count: text
                    .text
                    .as_deref()
                    .map(|body| body.lines().count())
                    .unwrap_or(0),
            }),
        }
    }
    (lines, unsupported)
}

/// Return regions skipped by HanOnly that must retain Source pixels.
pub fn protected_source_lines_for_page(
    scene: &Scene,
    page: PageId,
) -> Vec<(NodeId, EligibleTextLine)> {
    let Some(page_ref) = scene.page(page) else {
        return Vec::new();
    };
    let (image_width, image_height) = (page_ref.width, page_ref.height);
    let mut protected = Vec::new();

    for (node_id, node) in &page_ref.nodes {
        let NodeKind::Text(text) = &node.kind else {
            continue;
        };
        let node_id = *node_id;
        let transform = &node.transform;
        let body = text.text.as_deref().unwrap_or_default().trim();
        if body.is_empty() {
            continue;
        }
        if text.detector.as_deref() == Some(SOURCE_GATE_PROTECTED_DETECTOR) {
            let Some(bbox) = safe_source_restore_bbox(transform, image_width, image_height) else {
                continue;
            };
            protected.push((node_id, eligible_line(text, 0, body, bbox, false)));
            continue;
        }
        if !node.visible {
            continue;
        }
        let eligible = eligible_text_lines(transform, text, image_width, image_height);
        if eligible.as_ref().is_none_or(Vec::is_empty) {
            let Some(bbox) = safe_source_restore_bbox(transform, image_width, image_height) else {
                continue;
            };
            protected.push((node_id, eligible_line(text, 0, body, bbox, false)));
            continue;
        }
        let eligible = eligible.expect("non-empty eligibility checked above");
        let translated_line_count = text
            .translation
            .as_deref()
            .into_iter()
            .flat_map(str::lines)
            .filter(|line| !line.trim().is_empty())
            .count();
        if translated_line_count != eligible.len() {
            protected.extend(eligible.into_iter().map(|line| (node_id, line)));
        }

        let lines = text
            .text
            .as_deref()
            .unwrap_or_default()
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let line = line.trim();
                (!line.is_empty()).then_some((index, line))
            })
            .collect::<Vec<_>>();
        let Some(polygons) = text.line_polygons.as_ref() else {
            continue;
        };
        if polygons.len() != lines.len() {
            continue;
        }

        for ((line_index, line), quad) in lines.into_iter().zip(polygons) {
            if contains_han(line) {
                continue;
            }
            let Some(bbox) = safe_mixed_line_bbox(quad, transform, image_width, image_height)
            else {
                continue;
            };
            protected.push((node_id, eligible_line(text, line_index, line, bbox, true)));
        }
    }

    protected
}

pub(super) fn forbidden_han_lines_for_page(
    scene: &Scene,
    page: PageId,
) -> Vec<(NodeId, EligibleTextLine)> {
    let eligible = eligible_lines_for_page(scene, page)
        .0
        .into_iter()
        .map(|(node_id, line)| (node_id, line.line_index))
        .collect::<HashSet<_>>();
    let protected_detector_nodes = scene
        .page(page)
        .into_iter()
        .flat_map(|page| &page.nodes)
        .filter_map(|(node_id, node)| match &node.kind {
            NodeKind::Text(text)
                if text.detector.as_deref() == Some(SOURCE_GATE_PROTECTED_DETECTOR) =>
            {
                Some(*node_id)
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    protected_source_lines_for_page(scene, page)
        .into_iter()
        .filter(|(node_id, line)| {
            protected_detector_nodes.contains(node_id)
                || !eligible.contains(&(*node_id, line.line_index))
        })
        .collect()
}

fn eligible_line(
    text: &TextData,
    line_index: usize,
    line: &str,
    bbox: [f32; 4],
    with_polygon: bool,
) -> EligibleTextLine {
    let mut region = text_node_to_region(
        &Transform {
            x: bbox[0],
            y: bbox[1],
            width: bbox[2] - bbox[0],
            height: bbox[3] - bbox[1],
            rotation_deg: 0.0,
        },
        text,
    );
    region.rotation_deg = Some(0.0);
    region.line_polygons = with_polygon.then(|| vec![bbox_quad(bbox)]);
    EligibleTextLine {
        line_index,
        text: line.to_string(),
        region,
    }
}

fn safe_node_bbox(transform: &Transform, image_width: u32, image_height: u32) -> Option<[f32; 4]> {
    let values = [
        transform.x,
        transform.y,
        transform.width,
        transform.height,
        transform.rotation_deg,
    ];
    if values.iter().any(|value| !value.is_finite())
        || transform.width <= 0.0
        || transform.height <= 0.0
        || transform.rotation_deg != 0.0
    {
        return None;
    }
    intersect_bbox(
        [
            transform.x,
            transform.y,
            transform.x + transform.width,
            transform.y + transform.height,
        ],
        [0.0, 0.0, image_width as f32, image_height as f32],
    )
}

fn safe_source_restore_bbox(
    transform: &Transform,
    image_width: u32,
    image_height: u32,
) -> Option<[f32; 4]> {
    let values = [
        transform.x,
        transform.y,
        transform.width,
        transform.height,
        transform.rotation_deg,
    ];
    if values.iter().any(|value| !value.is_finite())
        || transform.width <= 0.0
        || transform.height <= 0.0
    {
        return None;
    }

    let center_x = transform.x + transform.width * 0.5;
    let center_y = transform.y + transform.height * 0.5;
    let half_width = transform.width * 0.5;
    let half_height = transform.height * 0.5;
    let (sin, cos) = transform.rotation_deg.to_radians().sin_cos();
    let corners = [
        (-half_width, -half_height),
        (half_width, -half_height),
        (half_width, half_height),
        (-half_width, half_height),
    ];
    let mut bbox = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for (x, y) in corners {
        let rotated_x = center_x + x * cos - y * sin;
        let rotated_y = center_y + x * sin + y * cos;
        bbox[0] = bbox[0].min(rotated_x);
        bbox[1] = bbox[1].min(rotated_y);
        bbox[2] = bbox[2].max(rotated_x);
        bbox[3] = bbox[3].max(rotated_y);
    }
    intersect_bbox(bbox, [0.0, 0.0, image_width as f32, image_height as f32])
}

fn safe_mixed_line_bbox(
    quad: &[[f32; 2]; 4],
    transform: &Transform,
    image_width: u32,
    image_height: u32,
) -> Option<[f32; 4]> {
    if quad.iter().flatten().any(|value| !value.is_finite()) || !quad_is_axis_aligned(quad) {
        return None;
    }
    let area = (0..4)
        .map(|index| {
            let next = (index + 1) % 4;
            quad[index][0] * quad[next][1] - quad[next][0] * quad[index][1]
        })
        .sum::<f32>()
        .abs()
        * 0.5;
    if area <= f32::EPSILON {
        return None;
    }

    let bbox = [
        quad.iter()
            .map(|point| point[0])
            .fold(f32::INFINITY, f32::min),
        quad.iter()
            .map(|point| point[1])
            .fold(f32::INFINITY, f32::min),
        quad.iter()
            .map(|point| point[0])
            .fold(f32::NEG_INFINITY, f32::max),
        quad.iter()
            .map(|point| point[1])
            .fold(f32::NEG_INFINITY, f32::max),
    ];
    let node_bbox = safe_node_bbox(transform, image_width, image_height)?;
    intersect_bbox(bbox, node_bbox)
}

fn quad_is_axis_aligned(quad: &[[f32; 2]; 4]) -> bool {
    (0..4).all(|index| {
        let next = (index + 1) % 4;
        quad[index][0] == quad[next][0] || quad[index][1] == quad[next][1]
    })
}

fn intersect_bbox(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let bbox = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (bbox[2] > bbox[0] && bbox[3] > bbox[1]).then_some(bbox)
}

fn bbox_quad(bbox: [f32; 4]) -> [[f32; 2]; 4] {
    [
        [bbox[0], bbox[1]],
        [bbox[2], bbox[1]],
        [bbox[2], bbox[3]],
        [bbox[0], bbox[3]],
    ]
}

pub fn line_support_mask(
    width: u32,
    height: u32,
    eligible_lines: &[EligibleTextLine],
) -> GrayImage {
    let mut mask = GrayImage::new(width, height);
    for line in eligible_lines {
        let bboxes = match line.region.line_polygons.as_deref() {
            Some(polygons) => polygons
                .iter()
                .filter_map(|quad| {
                    if quad.iter().flatten().any(|value| !value.is_finite())
                        || !quad_is_axis_aligned(quad)
                    {
                        return None;
                    }
                    let area = (0..4)
                        .map(|index| {
                            let next = (index + 1) % 4;
                            quad[index][0] * quad[next][1] - quad[next][0] * quad[index][1]
                        })
                        .sum::<f32>()
                        .abs()
                        * 0.5;
                    (area > f32::EPSILON).then(|| {
                        [
                            quad.iter()
                                .map(|point| point[0])
                                .fold(f32::INFINITY, f32::min),
                            quad.iter()
                                .map(|point| point[1])
                                .fold(f32::INFINITY, f32::min),
                            quad.iter()
                                .map(|point| point[0])
                                .fold(f32::NEG_INFINITY, f32::max),
                            quad.iter()
                                .map(|point| point[1])
                                .fold(f32::NEG_INFINITY, f32::max),
                        ]
                    })
                })
                .collect::<Vec<_>>(),
            None => vec![[
                line.region.x,
                line.region.y,
                line.region.x + line.region.width,
                line.region.y + line.region.height,
            ]],
        };
        for bbox in bboxes {
            let Some([left, top, right, bottom]) = pixel_bbox(bbox, width, height) else {
                continue;
            };
            let polygon = [
                Point::new(left, top),
                Point::new(right, top),
                Point::new(right, bottom),
                Point::new(left, bottom),
            ];
            draw_polygon_mut(&mut mask, &polygon, Luma([255]));
        }
    }
    mask
}

fn pixel_bbox(bbox: [f32; 4], width: u32, height: u32) -> Option<[i32; 4]> {
    if width == 0 || height == 0 || bbox.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let clipped = intersect_bbox(bbox, [0.0, 0.0, width as f32, height as f32])?;
    let left = clipped[0].floor().max(0.0) as i32;
    let top = clipped[1].floor().max(0.0) as i32;
    let right = (clipped[2].ceil().min(width as f32) as i32 - 1).max(left);
    let bottom = (clipped[3].ceil().min(height as f32) as i32 - 1).max(top);
    Some([left, top, right, bottom])
}

pub(crate) fn support_bboxes_overlap(
    first: [f32; 4],
    second: [f32; 4],
    width: u32,
    height: u32,
) -> bool {
    let (Some(first), Some(second)) = (
        pixel_bbox(first, width, height),
        pixel_bbox(second, width, height),
    ) else {
        return true;
    };
    first[0] <= second[2] && second[0] <= first[2] && first[1] <= second[3] && second[1] <= first[3]
}

pub fn intersect_gray_masks(source: &GrayImage, allowed: &GrayImage) -> GrayImage {
    assert_eq!(source.dimensions(), allowed.dimensions());
    GrayImage::from_fn(source.width(), source.height(), |x, y| {
        Luma([if allowed.get_pixel(x, y).0[0] == 0 {
            0
        } else {
            source.get_pixel(x, y).0[0]
        }])
    })
}

pub(super) enum PreparedInpaintMask {
    Prepared {
        mask: DynamicImage,
        blocks: Vec<TextRegion>,
    },
    NoEligibleHanTargets,
    EmptyMask,
}

pub(super) fn canonical_han_mask(
    source: &GrayImage,
    eligible_lines: &[(NodeId, EligibleTextLine)],
    protected_lines: &[(NodeId, EligibleTextLine)],
) -> Result<(GrayImage, GrayImage)> {
    ensure!(
        !eligible_lines.is_empty(),
        "canonical Han mask requires eligible targets"
    );

    let (width, height) = source.dimensions();
    let protected = line_support_mask(
        width,
        height,
        &protected_lines
            .iter()
            .map(|(_, line)| line.clone())
            .collect::<Vec<_>>(),
    );
    let mut eligible_by_node: Vec<(NodeId, Vec<EligibleTextLine>)> = Vec::new();
    let mut eligible_indexes = HashMap::new();
    for (node_id, line) in eligible_lines {
        let index = *eligible_indexes.entry(*node_id).or_insert_with(|| {
            eligible_by_node.push((*node_id, Vec::new()));
            eligible_by_node.len() - 1
        });
        eligible_by_node[index].1.push(line.clone());
    }
    let owner_support = eligible_by_node
        .iter()
        .map(|(node_id, lines)| (*node_id, line_support_mask(width, height, lines)))
        .collect::<Vec<_>>();
    let mut ink = source.clone();
    for (pixel, protected_pixel) in ink.pixels_mut().zip(protected.pixels()) {
        if protected_pixel.0[0] != 0 {
            pixel.0[0] = 0;
        }
    }

    let mut eligible_support = GrayImage::new(width, height);
    for (_, support) in &owner_support {
        for (pixel, support_pixel) in eligible_support.pixels_mut().zip(support.pixels()) {
            if support_pixel.0[0] != 0 {
                pixel.0[0] = 255;
            }
        }
    }
    for (pixel, protected_pixel) in eligible_support.pixels_mut().zip(protected.pixels()) {
        if protected_pixel.0[0] != 0 {
            pixel.0[0] = 0;
        }
    }
    let mut retained = GrayImage::new(width, height);
    let mut allowed = GrayImage::new(width, height);
    let mut visited = vec![false; width as usize * height as usize];
    let mut owned_nodes = HashSet::new();
    const NEIGHBORS: [(i32, i32); 8] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];

    for y in 0..height {
        for x in 0..width {
            let index = y as usize * width as usize + x as usize;
            if visited[index] || ink.get_pixel(x, y).0[0] == 0 {
                continue;
            }
            visited[index] = true;
            let mut queue = VecDeque::from([(x, y)]);
            let mut component = Vec::new();
            while let Some((cx, cy)) = queue.pop_front() {
                component.push((cx, cy));
                for (dx, dy) in NEIGHBORS {
                    let nx = cx as i64 + i64::from(dx);
                    let ny = cy as i64 + i64::from(dy);
                    if nx < 0 || ny < 0 || nx >= i64::from(width) || ny >= i64::from(height) {
                        continue;
                    }
                    let (nx, ny) = (nx as u32, ny as u32);
                    let index = ny as usize * width as usize + nx as usize;
                    if !visited[index] && ink.get_pixel(nx, ny).0[0] != 0 {
                        visited[index] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            let owners = owner_support
                .iter()
                .filter(|(_, support)| {
                    component
                        .iter()
                        .any(|&(cx, cy)| support.get_pixel(cx, cy).0[0] != 0)
                })
                .map(|(node_id, _)| *node_id)
                .collect::<Vec<_>>();
            match owners.as_slice() {
                [] => {}
                [owner] => {
                    owned_nodes.insert(*owner);
                    for &(cx, cy) in &component {
                        if eligible_support.get_pixel(cx, cy).0[0] != 0 {
                            let pixel = *ink.get_pixel(cx, cy);
                            retained.put_pixel(cx, cy, pixel);
                            allowed.put_pixel(cx, cy, pixel);
                        }
                    }
                }
                _ => bail!("unsafe Han mask: component has multiple eligible owners"),
            }
        }
    }

    ensure!(
        eligible_by_node
            .iter()
            .all(|(node_id, _)| owned_nodes.contains(node_id)),
        "unsafe Han mask: eligible target has no allowed ink"
    );
    Ok((retained, allowed))
}

pub(super) fn prepare_inpaint_mask<Expand>(
    mask: &DynamicImage,
    bubble_mask: &DynamicImage,
    all_blocks: &[TextRegion],
    eligible_lines: &[(NodeId, EligibleTextLine)],
    protected_lines: &[(NodeId, EligibleTextLine)],
    policy: SourceTextPolicy,
    region: Option<Region>,
    expand: Expand,
) -> Result<PreparedInpaintMask>
where
    Expand: FnOnce(&DynamicImage, &DynamicImage, &[TextRegion]) -> GrayImage,
{
    #[cfg(test)]
    let diagnostic_branch = if region.is_some() {
        EraseDiagnosticBranch::Region
    } else if policy == SourceTextPolicy::HanOnly {
        EraseDiagnosticBranch::HanOnly
    } else {
        EraseDiagnosticBranch::AllText
    };
    #[cfg(test)]
    record_erase_diagnostic(
        EraseDiagnosticStage::InpaintInputSegment,
        diagnostic_branch,
        Some(&mask.to_luma8()),
        None,
    );

    let inference_blocks = if region.is_none() && policy == SourceTextPolicy::HanOnly {
        eligible_lines
            .iter()
            .map(|(_, line)| line.region.clone())
            .collect::<Vec<_>>()
    } else {
        all_blocks.to_vec()
    };

    let final_mask = if let Some(region) = region {
        let clipped_mask = clip_mask_to_region(mask, &region);
        let clipped_bubble = clip_mask_to_region(bubble_mask, &region);
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintAllowedSupport,
            diagnostic_branch,
            None,
            None,
        );
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintPreExpandFiltered,
            diagnostic_branch,
            Some(&clipped_mask.to_luma8()),
            None,
        );
        let expanded = expand(&clipped_mask, &clipped_bubble, &inference_blocks);
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintBackendExpanded,
            diagnostic_branch,
            Some(&expanded),
            None,
        );
        DynamicImage::ImageLuma8(clip_gray_mask_to_region(&expanded, &region))
    } else if policy == SourceTextPolicy::HanOnly {
        if eligible_lines.is_empty() {
            return Ok(PreparedInpaintMask::NoEligibleHanTargets);
        }
        let (filtered, allowed) =
            canonical_han_mask(&mask.to_luma8(), eligible_lines, protected_lines)?;
        let filtered = DynamicImage::ImageLuma8(filtered);
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintAllowedSupport,
            diagnostic_branch,
            Some(&allowed),
            None,
        );
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintPreExpandFiltered,
            diagnostic_branch,
            Some(&filtered.to_luma8()),
            None,
        );
        let expanded = expand(&filtered, bubble_mask, &inference_blocks);
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintBackendExpanded,
            diagnostic_branch,
            Some(&expanded),
            None,
        );
        DynamicImage::ImageLuma8(intersect_gray_masks(&expanded, &allowed))
    } else {
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintAllowedSupport,
            diagnostic_branch,
            None,
            None,
        );
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintPreExpandFiltered,
            diagnostic_branch,
            Some(&mask.to_luma8()),
            None,
        );
        let expanded = expand(mask, bubble_mask, &inference_blocks);
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::InpaintBackendExpanded,
            diagnostic_branch,
            Some(&expanded),
            None,
        );
        DynamicImage::ImageLuma8(expanded)
    };

    let returns_some = final_mask.to_luma8().pixels().any(|pixel| pixel.0[0] != 0);
    #[cfg(test)]
    record_erase_diagnostic(
        EraseDiagnosticStage::InpaintFinal,
        diagnostic_branch,
        Some(&final_mask.to_luma8()),
        Some(returns_some),
    );
    Ok(if returns_some {
        PreparedInpaintMask::Prepared {
            mask: final_mask,
            blocks: inference_blocks,
        }
    } else {
        PreparedInpaintMask::EmptyMask
    })
}

pub fn build_han_only_translation_ops(
    scene: &Scene,
    page: PageId,
    allowed_ids: Option<&[NodeId]>,
    targets: &[(NodeId, EligibleTextLine)],
    translations: &[String],
) -> Result<Vec<Op>> {
    anyhow::ensure!(
        targets.len() == translations.len(),
        "translation count does not match Han line targets"
    );

    let mut seen = HashSet::with_capacity(targets.len());
    let mut mapped = Vec::with_capacity(targets.len());
    for ((node_id, line), translation) in targets.iter().zip(translations) {
        anyhow::ensure!(
            allowed_ids.is_none_or(|ids| ids.contains(node_id)),
            "translation target outside requested scope"
        );
        anyhow::ensure!(
            matches!(
                scene.node(page, *node_id).map(|node| &node.kind),
                Some(NodeKind::Text(_))
            ),
            "translation target is not a text node on the page"
        );
        anyhow::ensure!(
            seen.insert((*node_id, line.line_index)),
            "duplicate Han line translation target"
        );
        anyhow::ensure!(!translation.trim().is_empty(), "empty Han line translation");
        mapped.push(((*node_id, line.line_index), translation.clone()));
    }
    mapped.sort_by_key(|((node_id, line_index), _)| (*node_id, *line_index));

    let mut by_node: HashMap<NodeId, Vec<String>> = HashMap::new();
    for ((node_id, _), translation) in mapped {
        by_node.entry(node_id).or_default().push(translation);
    }

    let page_ref = scene
        .page(page)
        .ok_or_else(|| anyhow::anyhow!("page {page} not found"))?;
    let mut ops = Vec::new();
    for (node_id, node) in &page_ref.nodes {
        if !matches!(node.kind, NodeKind::Text(_))
            || allowed_ids.is_some_and(|ids| !ids.contains(node_id))
        {
            continue;
        }
        let translation = by_node.remove(node_id).map(|lines| lines.join("\n"));
        ops.push(Op::UpdateNode {
            page,
            id: *node_id,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    translation: Some(translation),
                    sprite: Some(None),
                    sprite_transform: Some(None),
                    ..Default::default()
                })),
                transform: None,
                visible: None,
            },
            prev: NodePatch::default(),
        });
    }
    anyhow::ensure!(by_node.is_empty(), "unmapped Han line translation target");
    Ok(ops)
}

/// Convert a scene `(Transform, TextData)` pair into a `koharu-ml` `TextRegion`
/// for passing back through detector helpers that need geometry + language
/// hints (e.g. CTD's `refine_segmentation_mask`, OCR's `extract_text_block_regions`).
pub fn text_node_to_region(transform: &Transform, text: &TextData) -> koharu_ml::types::TextRegion {
    koharu_ml::types::TextRegion {
        x: transform.x,
        y: transform.y,
        width: transform.width,
        height: transform.height,
        confidence: text.confidence,
        line_polygons: text.line_polygons.clone(),
        source_direction: text.source_direction.map(core_text_direction_to_ml),
        rotation_deg: text.rotation_deg,
        detected_font_size_px: text.detected_font_size_px,
        detector: text.detector.clone(),
    }
}

/// Inverse of `ml_text_direction_to_core`.
pub fn core_text_direction_to_ml(d: koharu_core::TextDirection) -> koharu_ml::types::TextDirection {
    match d {
        koharu_core::TextDirection::Horizontal => koharu_ml::types::TextDirection::Horizontal,
        koharu_core::TextDirection::Vertical => koharu_ml::types::TextDirection::Vertical,
    }
}

// ---------------------------------------------------------------------------
// Op constructors
// ---------------------------------------------------------------------------

/// Build an `AddNode` for a new `Image { role }` layer.
#[allow(clippy::too_many_arguments)]
pub fn add_image_node_op(
    page: PageId,
    role: ImageRole,
    blob: BlobRef,
    natural_width: u32,
    natural_height: u32,
    transform: Transform,
    visible: bool,
    at: usize,
) -> Op {
    let node = Node {
        id: NodeId::new(),
        transform,
        visible,
        kind: NodeKind::Image(ImageData {
            role,
            blob,
            opacity: 1.0,
            natural_width,
            natural_height,
            name: None,
        }),
    };
    Op::AddNode { page, node, at }
}

/// Build an `AddNode` for a new `Mask { role }` layer.
pub fn add_mask_node_op(
    page: PageId,
    role: MaskRole,
    blob: BlobRef,
    transform: Transform,
    visible: bool,
    at: usize,
) -> Op {
    let node = Node {
        id: NodeId::new(),
        transform,
        visible,
        kind: NodeKind::Mask(MaskData { role, blob }),
    };
    Op::AddNode { page, node, at }
}

/// Replace or add an `Image { role }` blob for `page`. If a node already
/// exists with that role, emits an `UpdateNode` with `ImageDataPatch`.
/// Otherwise emits `AddNode` at the top of the stack (renderer role) or
/// after Source (inpainted/custom role).
pub fn upsert_image_blob(
    scene: &Scene,
    page: PageId,
    role: ImageRole,
    blob: BlobRef,
    natural_width: u32,
    natural_height: u32,
) -> Op {
    if let Some((node_id, _)) = find_image_node(scene, page, role) {
        Op::UpdateNode {
            page,
            id: node_id,
            patch: koharu_core::NodePatch {
                data: Some(NodeDataPatch::Image(koharu_core::ImageDataPatch {
                    blob: Some(blob),
                    opacity: None,
                    name: None,
                    natural_width: Some(natural_width),
                    natural_height: Some(natural_height),
                })),
                transform: None,
                visible: None,
            },
            prev: koharu_core::NodePatch::default(),
        }
    } else {
        let at = {
            let page_ref = scene.page(page);
            let base = page_ref.map(|p| p.nodes.len()).unwrap_or(0);
            match role {
                // Rendered on top.
                ImageRole::Rendered => base,
                // Inpainted directly after source (index 1 if source is present).
                ImageRole::Inpainted => 1.min(base),
                // Custom / Source → append.
                _ => base,
            }
        };
        add_image_node_op(
            page,
            role,
            blob,
            natural_width,
            natural_height,
            Transform::default(),
            role != ImageRole::Rendered, // hide Rendered by default; make a toggle explicit
            at,
        )
    }
}

/// Replace or add a `Mask { role }` blob for `page`.
pub fn upsert_mask_blob(scene: &Scene, page: PageId, role: MaskRole, blob: BlobRef) -> Op {
    if let Some((node_id, _)) = find_mask_node(scene, page, role) {
        Op::UpdateNode {
            page,
            id: node_id,
            patch: koharu_core::NodePatch {
                data: Some(NodeDataPatch::Mask(koharu_core::MaskDataPatch {
                    blob: Some(blob),
                })),
                transform: None,
                visible: None,
            },
            prev: koharu_core::NodePatch::default(),
        }
    } else {
        let at = scene.page(page).map(|p| p.nodes.len()).unwrap_or(0);
        let visible = matches!(role, MaskRole::BrushInpaint);
        add_mask_node_op(page, role, blob, Transform::default(), visible, at)
    }
}

/// Build a `Node` ready to be added for a new Text region.
pub fn new_text_node(bbox: [f32; 4], text_data: TextData, visible: bool) -> Node {
    Node {
        id: NodeId::new(),
        transform: Transform {
            x: bbox[0],
            y: bbox[1],
            width: bbox[2] - bbox[0],
            height: bbox[3] - bbox[1],
            rotation_deg: text_data.rotation_deg.unwrap_or(0.0),
        },
        visible,
        kind: NodeKind::Text(text_data),
    }
}

/// Small helper: decoded image dimensions.
pub fn image_dimensions(image: &DynamicImage) -> (u32, u32) {
    image.dimensions()
}

/// Translate the `koharu-ml` `TextDirection` primitive into the scene-layer one.
pub fn ml_text_direction_to_core(d: koharu_ml::types::TextDirection) -> koharu_core::TextDirection {
    match d {
        koharu_ml::types::TextDirection::Horizontal => koharu_core::TextDirection::Horizontal,
        koharu_ml::types::TextDirection::Vertical => koharu_core::TextDirection::Vertical,
    }
}

/// Translate a `koharu-ml::TextRegion` (detector output) into a scene-layer
/// `(bbox, TextData)` pair ready for `new_text_node`.
pub fn text_region_to_pair(
    r: koharu_ml::types::TextRegion,
    default_detector: &'static str,
) -> ([f32; 4], TextData) {
    let bbox = [r.x, r.y, r.x + r.width, r.y + r.height];
    let data = TextData {
        confidence: r.confidence,
        source_direction: r.source_direction.map(ml_text_direction_to_core),
        line_polygons: r.line_polygons,
        rotation_deg: r.rotation_deg,
        detected_font_size_px: r.detected_font_size_px,
        detector: r.detector.or_else(|| Some(default_detector.to_string())),
        ..Default::default()
    };
    (bbox, data)
}

/// Current node count on `page`, or 0 if the page doesn't exist.
pub fn page_node_count(scene: &Scene, page: PageId) -> usize {
    scene.page(page).map(|p| p.nodes.len()).unwrap_or(0)
}

/// Emit `RemoveNode` ops for every text node currently on `page` when a
/// detector has replacements ready. Empty detections preserve the previous
/// blocks instead of destructively clearing the page.
pub fn clear_text_nodes_ops(
    scene: &Scene,
    page: PageId,
    replacement_count: usize,
    clear_on_empty: bool,
) -> Vec<Op> {
    if replacement_count == 0 && !clear_on_empty {
        return Vec::new();
    }
    let Some(page_ref) = scene.page(page) else {
        return Vec::new();
    };
    page_ref
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, (_, node))| matches!(&node.kind, NodeKind::Text(_)))
        .map(|(idx, (id, node))| Op::RemoveNode {
            page,
            id: *id,
            prev_node: node.clone(),
            prev_index: idx,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Manga reading-order sort (Recursive XY-Cut)
//
// Right-to-left columns, top-to-bottom within each column. Shared by every
// detector that emits text blocks (CTD, comic-text-bubble, PP-DocLayout).
// ---------------------------------------------------------------------------

/// Sort `(bbox, data)` pairs in a reading order (RTL or LTR).
pub fn sort_manga_reading_order<T>(blocks: &mut [([f32; 4], T)], order: ReadingOrder) {
    #[derive(Debug, PartialEq, Clone, Copy)]
    enum Axis {
        X,
        Y,
    }

    if blocks.len() <= 1 {
        return;
    }

    let mut widths: Vec<f32> = blocks.iter().map(|(b, _)| b[2] - b[0]).collect();
    let mut heights: Vec<f32> = blocks.iter().map(|(b, _)| b[3] - b[1]).collect();
    widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let median_w = widths[widths.len() / 2].max(1.0);
    let median_h = heights[heights.len() / 2].max(1.0);
    let min_gap_x = (median_w * 0.15).max(10.0);
    let min_gap_y = (median_h * 0.10).max(8.0);

    fn xy_cut_recursive<T>(
        blocks: &mut [([f32; 4], T)],
        min_gap_x: f32,
        min_gap_y: f32,
        order: ReadingOrder,
    ) {
        use std::cmp::Ordering;
        if blocks.len() <= 1 {
            return;
        }
        let cut = find_best_cut(blocks, min_gap_x, min_gap_y);
        let Some((axis, gap)) = cut else {
            let row_height = min_gap_y * 4.0;
            blocks.sort_by(|a, b| {
                let row_a = (a.0[1] / row_height).floor();
                let row_b = (b.0[1] / row_height).floor();
                row_a
                    .partial_cmp(&row_b)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| match order {
                        ReadingOrder::Rtl => b.0[0].partial_cmp(&a.0[0]).unwrap_or(Ordering::Equal),
                        ReadingOrder::Ltr => a.0[0].partial_cmp(&b.0[0]).unwrap_or(Ordering::Equal),
                    })
            });
            return;
        };

        let cut_coord = (gap.0 + gap.1) / 2.0;
        blocks.sort_by_key(|(b, _)| {
            if axis == Axis::X {
                let center_x = b[0] + (b[2] - b[0]) * 0.5;
                match order {
                    ReadingOrder::Rtl => center_x < cut_coord, // Right first
                    ReadingOrder::Ltr => center_x > cut_coord, // Left first
                }
            } else {
                // Top partition first: items whose center is BELOW cut go second.
                (b[1] + (b[3] - b[1]) * 0.5) > cut_coord
            }
        });

        let group1_len = blocks
            .iter()
            .filter(|(b, _)| {
                if axis == Axis::X {
                    let center_x = b[0] + (b[2] - b[0]) * 0.5;
                    match order {
                        ReadingOrder::Rtl => center_x >= cut_coord,
                        ReadingOrder::Ltr => center_x <= cut_coord,
                    }
                } else {
                    (b[1] + (b[3] - b[1]) * 0.5) <= cut_coord
                }
            })
            .count();

        if group1_len == 0 || group1_len == blocks.len() {
            blocks.sort_by(|a, b| match order {
                ReadingOrder::Rtl => b.0[0].partial_cmp(&a.0[0]).unwrap_or(Ordering::Equal),
                ReadingOrder::Ltr => a.0[0].partial_cmp(&b.0[0]).unwrap_or(Ordering::Equal),
            });
            return;
        }

        let (left, right) = blocks.split_at_mut(group1_len);
        xy_cut_recursive(left, min_gap_x, min_gap_y, order);
        xy_cut_recursive(right, min_gap_x, min_gap_y, order);
    }

    fn find_best_cut<T>(
        blocks: &[([f32; 4], T)],
        min_gap_x: f32,
        min_gap_y: f32,
    ) -> Option<(Axis, (f32, f32))> {
        let mut x_intervals: Vec<(f32, f32)> = blocks.iter().map(|(b, _)| (b[0], b[2])).collect();
        let mut y_intervals: Vec<(f32, f32)> = blocks.iter().map(|(b, _)| (b[1], b[3])).collect();
        x_intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        y_intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let gap_x = find_largest_gap(&x_intervals, min_gap_x);
        let gap_y = find_largest_gap(&y_intervals, min_gap_y);
        match (gap_x, gap_y) {
            (Some(gx), Some(gy)) => {
                let width_y = gy.1 - gy.0;
                let width_x = gx.1 - gx.0;
                if width_y > 12.0 || width_y > (width_x * 0.4) {
                    Some((Axis::Y, gy))
                } else {
                    Some((Axis::X, gx))
                }
            }
            (None, Some(gy)) => Some((Axis::Y, gy)),
            (Some(gx), None) => Some((Axis::X, gx)),
            (None, None) => None,
        }
    }

    fn find_largest_gap(intervals: &[(f32, f32)], min_gap: f32) -> Option<(f32, f32)> {
        if intervals.is_empty() {
            return None;
        }
        let mut largest: Option<(f32, f32)> = None;
        let mut current_max_end = intervals[0].1;
        for interval in intervals.iter().skip(1) {
            if interval.0 > current_max_end {
                let gap = interval.0 - current_max_end;
                if gap >= min_gap
                    && match largest {
                        Some(best) => gap > best.1 - best.0,
                        None => true,
                    }
                {
                    largest = Some((current_max_end, interval.0));
                }
            }
            current_max_end = current_max_end.max(interval.1);
        }
        largest
    }

    xy_cut_recursive(blocks, min_gap_x, min_gap_y, order);
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, sync::Barrier};

    use super::*;
    use koharu_core::{BlobRef, Page, ReadingOrder, Region, TextDirection};
    use koharu_ml::inpainting::{
        expand_mask_for_inpainting, mask::expand_mask_to_bubble_region_for_inpainting,
    };

    use crate::config::SourceTextPolicy;

    fn prepare_inpaint_mask<Expand>(
        mask: &DynamicImage,
        bubble_mask: &DynamicImage,
        all_blocks: &[TextRegion],
        eligible_lines: &[EligibleTextLine],
        policy: SourceTextPolicy,
        region: Option<Region>,
        expand: Expand,
    ) -> Option<(DynamicImage, Vec<TextRegion>)>
    where
        Expand: FnOnce(&DynamicImage, &DynamicImage, &[TextRegion]) -> GrayImage,
    {
        let owner = NodeId::new();
        let eligible = eligible_lines
            .iter()
            .cloned()
            .map(|line| (owner, line))
            .collect::<Vec<_>>();
        match super::prepare_inpaint_mask(
            mask,
            bubble_mask,
            all_blocks,
            &eligible,
            &[],
            policy,
            region,
            expand,
        )
        .ok()?
        {
            PreparedInpaintMask::Prepared { mask, blocks } => Some((mask, blocks)),
            PreparedInpaintMask::NoEligibleHanTargets | PreparedInpaintMask::EmptyMask => None,
        }
    }

    fn start_erase_capture() -> EraseDiagnosticCapture {
        loop {
            match EraseDiagnosticCapture::start() {
                Ok(capture) => return capture,
                Err(EraseDiagnosticCaptureActive) => std::thread::yield_now(),
            }
        }
    }

    fn text_data(text: &str, line_polygons: Option<Vec<[[f32; 2]; 4]>>) -> TextData {
        TextData {
            text: Some(text.to_string()),
            line_polygons,
            source_direction: Some(TextDirection::Horizontal),
            confidence: 0.9,
            ..Default::default()
        }
    }

    fn transform() -> Transform {
        Transform {
            x: 10.0,
            y: 20.0,
            width: 80.0,
            height: 60.0,
            rotation_deg: 0.0,
        }
    }

    fn quad(x1: f32, y1: f32, x2: f32, y2: f32) -> [[f32; 2]; 4] {
        [[x1, y1], [x2, y1], [x2, y2], [x1, y2]]
    }

    #[test]
    fn eligible_detects_unicode_han_only() {
        assert!(contains_han("中文123，。"));
        assert!(contains_han("S型曲线"));
        assert!(!contains_han("S-CURVE 123"));
    }

    #[test]
    fn protected_latin_word_distinguishes_words_from_single_letter_labels() {
        for value in [
            "AI智能",
            "Peach蜜桃",
            "S-CURVE型",
            "don't塑形",
            "S’CURVE曲线",
        ] {
            assert!(
                contains_protected_latin_word(value),
                "expected word in {value}"
            );
        }
        for value in ["S型曲线", "A版", "中文", "123"] {
            assert!(
                !contains_protected_latin_word(value),
                "unexpected protected word in {value}"
            );
        }
    }

    #[test]
    fn eligible_single_latin_label_with_han_targets_the_whole_line() {
        let text = text_data("S型曲线", None);

        let lines = eligible_text_lines(&transform(), &text, 100, 100).expect("eligible line");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_index, 0);
        assert_eq!(lines[0].text, "S型曲线");
    }

    #[test]
    fn eligible_inline_english_word_without_word_boxes_is_unsupported() {
        for value in ["AI智能塑形", "Peach蜜桃臀", "S-CURVE型曲线"] {
            let text = text_data(value, None);
            assert!(eligible_text_lines(&transform(), &text, 100, 100).is_none());
        }
    }

    #[test]
    fn eligible_word_box_inline_mixed_targets_only_han_units() {
        let text = text_data(
            "Peach\n蜜桃臀",
            Some(vec![
                quad(12.0, 22.0, 48.0, 38.0),
                quad(52.0, 22.0, 88.0, 38.0),
            ]),
        );

        let lines = eligible_text_lines(&transform(), &text, 100, 100).expect("safe word boxes");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_index, 1);
        assert_eq!(lines[0].text, "蜜桃臀");
        assert_eq!(
            lines[0].region.line_polygons.as_deref(),
            Some([quad(52.0, 22.0, 88.0, 38.0)].as_slice())
        );
    }

    #[test]
    fn protected_source_lines_keep_only_validated_english_word_boxes() {
        let id = NodeId::new();
        let mut data = text_data(
            "Peach\n蜜桃臀",
            Some(vec![
                quad(12.0, 22.0, 48.0, 38.0),
                quad(52.0, 22.0, 88.0, 38.0),
            ]),
        );
        data.translation = Some("Sweet butt".to_string());
        let node = Node {
            id,
            transform: transform(),
            visible: true,
            kind: NodeKind::Text(data),
        };
        let (scene, page) = translation_scene(vec![node]);

        let protected = protected_source_lines_for_page(&scene, page);

        assert_eq!(protected.len(), 1);
        assert_eq!(protected[0].0, id);
        assert_eq!(protected[0].1.text, "Peach");
        assert_eq!(
            protected[0].1.region.line_polygons.as_deref(),
            Some([quad(12.0, 22.0, 48.0, 38.0)].as_slice())
        );
    }

    #[test]
    fn protected_source_lines_restore_rotated_unsupported_node() {
        let id = NodeId::new();
        let mut node_transform = Transform {
            x: 40.0,
            y: 40.0,
            width: 20.0,
            height: 10.0,
            rotation_deg: 45.0,
        };
        let node = Node {
            id,
            transform: node_transform,
            visible: true,
            kind: NodeKind::Text(text_data("Peach\n蜜桃臀", None)),
        };
        let (scene, page) = translation_scene(vec![node]);

        let protected = protected_source_lines_for_page(&scene, page);

        assert_eq!(protected.len(), 1);
        assert_eq!(protected[0].0, id);
        let mask = line_support_mask(100, 100, std::slice::from_ref(&protected[0].1));
        assert_ne!(mask.get_pixel(40, 35).0[0], 0);
        assert!(
            eligible_text_lines(&node_transform, &text_data("Peach\n蜜桃臀", None), 100, 100)
                .is_none()
        );

        node_transform.rotation_deg = f32::NAN;
        let invalid = Node {
            id: NodeId::new(),
            transform: node_transform,
            visible: true,
            kind: NodeKind::Text(text_data("Peach\n蜜桃臀", None)),
        };
        let (scene, page) = translation_scene(vec![invalid]);
        assert!(protected_source_lines_for_page(&scene, page).is_empty());
    }

    #[test]
    fn eligible_mixed_node_returns_only_han_line_with_canonical_geometry() {
        let text = text_data(
            "S-CURVE\nS型曲线",
            Some(vec![
                quad(12.0, 22.0, 80.0, 38.0),
                quad(8.0, 42.0, 95.0, 65.0),
            ]),
        );

        let lines = eligible_text_lines(&transform(), &text, 100, 100).expect("safe geometry");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_index, 1);
        assert_eq!(lines[0].text, "S型曲线");
        assert_eq!(
            (
                lines[0].region.x,
                lines[0].region.y,
                lines[0].region.width,
                lines[0].region.height,
            ),
            (10.0, 42.0, 80.0, 23.0)
        );
        assert_eq!(
            lines[0].region.line_polygons.as_deref(),
            Some([quad(10.0, 42.0, 90.0, 65.0)].as_slice())
        );
    }

    #[test]
    fn eligible_mixed_node_rejects_missing_or_mismatched_polygons() {
        let missing = text_data("English\n中文", None);
        assert!(eligible_text_lines(&transform(), &missing, 100, 100).is_none());

        let mut vertical = text_data("English\n中文", None);
        vertical.source_direction = Some(TextDirection::Vertical);
        assert!(eligible_text_lines(&transform(), &vertical, 100, 100).is_none());

        let mut rotated = transform();
        rotated.rotation_deg = 15.0;
        assert!(eligible_text_lines(&rotated, &missing, 100, 100).is_none());

        let mismatched = text_data("English\n中文", Some(vec![quad(12.0, 22.0, 80.0, 38.0)]));
        assert!(eligible_text_lines(&transform(), &mismatched, 100, 100).is_none());
    }

    #[test]
    fn eligible_mixed_node_rejects_unsafe_geometry() {
        let cases = [
            quad(f32::NAN, 22.0, 80.0, 38.0),
            quad(12.0, 22.0, f32::INFINITY, 38.0),
            quad(12.0, 22.0, 12.0, 38.0),
            quad(120.0, 122.0, 180.0, 138.0),
            [[12.0, 22.0], [80.0, 24.0], [78.0, 38.0], [10.0, 36.0]],
        ];
        for unsafe_quad in cases {
            let text = text_data(
                "English\n中文",
                Some(vec![quad(12.0, 22.0, 80.0, 38.0), unsafe_quad]),
            );
            assert!(eligible_text_lines(&transform(), &text, 200, 200).is_none());
        }

        let mut rotated = transform();
        rotated.rotation_deg = 5.0;
        let text = text_data(
            "English\n中文",
            Some(vec![
                quad(12.0, 22.0, 80.0, 38.0),
                quad(12.0, 42.0, 80.0, 58.0),
            ]),
        );
        assert!(eligible_text_lines(&rotated, &text, 100, 100).is_none());
    }

    #[test]
    fn eligible_pure_han_without_polygons_uses_clipped_node_region() {
        let mut transform = transform();
        transform.x = -5.0;
        let text = text_data("第一行\n第二行", None);

        let lines = eligible_text_lines(&transform, &text, 70, 70).expect("pure Han fallback");

        assert_eq!(lines.len(), 2);
        assert_eq!([lines[0].line_index, lines[1].line_index], [0, 1]);
        for line in lines {
            assert_eq!(
                (
                    line.region.x,
                    line.region.y,
                    line.region.width,
                    line.region.height
                ),
                (0.0, 20.0, 70.0, 50.0)
            );
            assert!(line.region.line_polygons.is_none());
        }
    }

    #[test]
    fn eligible_pure_english_without_polygons_is_empty() {
        let text = text_data("S-CURVE\nPEACH BODY", None);
        let lines = eligible_text_lines(&transform(), &text, 100, 100).expect("valid node");
        assert!(lines.is_empty());
    }

    #[test]
    fn eligible_pure_han_rejects_invalid_node_region() {
        let text = text_data("中文", None);
        let invalid = [
            Transform {
                width: 0.0,
                ..transform()
            },
            Transform {
                x: f32::NAN,
                ..transform()
            },
            Transform {
                x: 200.0,
                ..transform()
            },
        ];
        for transform in invalid {
            assert!(eligible_text_lines(&transform, &text, 100, 100).is_none());
        }
    }

    fn support_line(bbox: [f32; 4], line_polygons: Option<Vec<[[f32; 2]; 4]>>) -> EligibleTextLine {
        EligibleTextLine {
            line_index: 0,
            text: "中文".to_string(),
            region: koharu_ml::types::TextRegion {
                x: bbox[0],
                y: bbox[1],
                width: bbox[2] - bbox[0],
                height: bbox[3] - bbox[1],
                line_polygons,
                ..Default::default()
            },
        }
    }

    #[test]
    fn line_support_mask_handles_empty_and_zero_dimensions() {
        assert_eq!(line_support_mask(0, 0, &[]).dimensions(), (0, 0));
        assert!(
            line_support_mask(8, 8, &[])
                .pixels()
                .all(|pixel| pixel.0[0] == 0)
        );
    }

    #[test]
    fn line_support_mask_rasterizes_polygon_and_bbox_fallback() {
        let polygon = support_line([2.0, 2.0, 6.0, 6.0], Some(vec![quad(2.0, 2.0, 6.0, 6.0)]));
        let fallback = support_line([-2.0, 7.0, 4.0, 12.0], None);

        let mask = line_support_mask(10, 10, &[polygon, fallback]);

        assert_ne!(mask.get_pixel(3, 3).0[0], 0);
        assert_ne!(mask.get_pixel(1, 8).0[0], 0);
        assert_eq!(mask.get_pixel(8, 8).0[0], 0);
    }

    #[test]
    fn line_support_mask_rejects_invalid_geometry() {
        let invalid = [
            support_line(
                [0.0, 0.0, 1.0, 1.0],
                Some(vec![quad(f32::NAN, 0.0, 1.0, 1.0)]),
            ),
            support_line([0.0, 0.0, 1.0, 1.0], Some(vec![quad(1.0, 1.0, 1.0, 2.0)])),
            support_line(
                [20.0, 20.0, 30.0, 30.0],
                Some(vec![quad(20.0, 20.0, 30.0, 30.0)]),
            ),
            support_line([f32::INFINITY, 0.0, f32::INFINITY, 2.0], None),
        ];

        let mask = line_support_mask(10, 10, &invalid);
        assert!(mask.pixels().all(|pixel| pixel.0[0] == 0));
    }

    #[test]
    fn line_support_mask_keeps_oversized_quad_out_of_english_roi() {
        let text = text_data(
            "English\n中文",
            Some(vec![
                quad(12.0, 22.0, 80.0, 38.0),
                quad(8.0, 42.0, 95.0, 65.0),
            ]),
        );
        let lines = eligible_text_lines(&transform(), &text, 100, 100).expect("eligible line");

        let mask = line_support_mask(100, 100, &lines);

        assert_eq!(mask.get_pixel(5, 50).0[0], 0);
        assert_ne!(mask.get_pixel(20, 50).0[0], 0);
        assert_eq!(mask.get_pixel(95, 50).0[0], 0);
    }

    fn node_line(node_id: NodeId, bbox: [f32; 4]) -> (NodeId, EligibleTextLine) {
        (
            node_id,
            support_line(bbox, Some(vec![quad(bbox[0], bbox[1], bbox[2], bbox[3])])),
        )
    }

    #[test]
    fn canonical_han_mask_uses_complete_components_for_ownership_but_clips_retained_ink() {
        let owner = NodeId::new();
        let source = GrayImage::from_fn(12, 6, |x, y| {
            Luma([if (1..=5).contains(&x) && y == 2 || (x, y) == (9, 2) {
                255
            } else {
                0
            }])
        });

        let (retained, allowed) =
            canonical_han_mask(&source, &[node_line(owner, [2.0, 1.0, 5.0, 4.0])], &[]).unwrap();

        assert_eq!(retained.get_pixel(1, 2).0[0], 0);
        assert_eq!(allowed.get_pixel(1, 2).0[0], 0);
        for x in 2..5 {
            assert_eq!(retained.get_pixel(x, 2).0[0], 255);
            assert_eq!(allowed.get_pixel(x, 2).0[0], 255);
        }
        assert_eq!(retained.get_pixel(5, 2).0[0], 0);
        assert_eq!(allowed.get_pixel(5, 2).0[0], 0);
        assert_eq!(retained.get_pixel(9, 2).0[0], 0);
        assert_eq!(allowed.get_pixel(9, 2).0[0], 0);
    }

    #[test]
    fn canonical_han_mask_subtracts_protected_support_before_components() {
        let owner = NodeId::new();
        let protected = NodeId::new();
        let source = GrayImage::from_fn(10, 5, |x, y| {
            Luma([if (1..=7).contains(&x) && y == 2 {
                255
            } else {
                0
            }])
        });

        let (retained, allowed) = canonical_han_mask(
            &source,
            &[node_line(owner, [1.0, 1.0, 4.0, 4.0])],
            &[node_line(protected, [4.0, 1.0, 5.0, 4.0])],
        )
        .unwrap();

        for x in 1..=3 {
            assert_eq!(retained.get_pixel(x, 2).0[0], 255);
        }
        for x in 4..=7 {
            assert_eq!(retained.get_pixel(x, 2).0[0], 0);
            assert_eq!(allowed.get_pixel(x, 2).0[0], 0);
        }
    }

    #[test]
    fn canonical_han_mask_is_node_order_independent_for_disjoint_owners() {
        let first = NodeId::new();
        let second = NodeId::new();
        let source = GrayImage::from_fn(12, 5, |x, y| {
            Luma([if matches!((x, y), (2, 2) | (3, 2) | (8, 2) | (9, 2)) {
                255
            } else {
                0
            }])
        });
        let targets = [
            node_line(first, [2.0, 1.0, 4.0, 4.0]),
            node_line(second, [8.0, 1.0, 10.0, 4.0]),
        ];
        let reversed = [targets[1].clone(), targets[0].clone()];

        let forward = canonical_han_mask(&source, &targets, &[]).unwrap();
        let backward = canonical_han_mask(&source, &reversed, &[]).unwrap();

        assert_eq!(forward.0.as_raw(), backward.0.as_raw());
        assert_eq!(forward.1.as_raw(), backward.1.as_raw());
    }

    #[test]
    fn canonical_han_mask_rejects_multi_owner_and_owner_without_ink() {
        let first = NodeId::new();
        let second = NodeId::new();
        let connected = GrayImage::from_fn(8, 4, |x, y| {
            Luma([if (1..=6).contains(&x) && y == 2 {
                255
            } else {
                0
            }])
        });
        let error = canonical_han_mask(
            &connected,
            &[
                node_line(first, [1.0, 1.0, 3.0, 4.0]),
                node_line(second, [5.0, 1.0, 7.0, 4.0]),
            ],
            &[],
        )
        .unwrap_err();
        assert!(error.to_string().contains("multiple eligible owners"));

        let error = canonical_han_mask(
            &GrayImage::new(8, 4),
            &[node_line(first, [0.0, 0.0, 2.0, 2.0])],
            &[],
        )
        .unwrap_err();
        assert!(error.to_string().contains("no allowed ink"));
    }

    #[test]
    fn canonical_han_mask_clips_page_edges_and_is_scale_stable() {
        for scale in [0.5_f32, 1.0, 2.0, 4.0] {
            let width = (12.0 * scale).round() as u32;
            let height = (8.0 * scale).round() as u32;
            let left = (2.0 * scale).round() as u32;
            let right = (6.0 * scale).round() as u32;
            let y = (3.0 * scale).round().min(height.saturating_sub(1) as f32) as u32;
            let source = GrayImage::from_fn(width, height, |x, py| {
                Luma([if (left..right).contains(&x) && py == y {
                    255
                } else {
                    0
                }])
            });
            let target = node_line(NodeId::new(), [-2.0, 0.0, 6.0 * scale, height as f32]);
            let (retained, allowed) = canonical_han_mask(&source, &[target], &[]).unwrap();
            assert_eq!(
                retained.pixels().filter(|pixel| pixel.0[0] != 0).count(),
                right.saturating_sub(left) as usize
            );
            assert!(
                retained
                    .pixels()
                    .zip(allowed.pixels())
                    .all(|(pixel, support)| pixel.0[0] == 0 || support.0[0] != 0)
            );
        }
    }

    #[test]
    fn prepare_han_mask_stops_no_target_and_unsafe_inputs_before_expansion() {
        let bubble = DynamicImage::ImageLuma8(GrayImage::from_pixel(8, 4, Luma([255])));
        let calls = Cell::new(0);
        let expand = |mask: &DynamicImage, _: &DynamicImage, _: &[TextRegion]| {
            calls.set(calls.get() + 1);
            mask.to_luma8()
        };
        let source = DynamicImage::ImageLuma8(GrayImage::from_pixel(8, 4, Luma([255])));
        assert!(matches!(
            super::prepare_inpaint_mask(
                &source,
                &bubble,
                &[],
                &[],
                &[],
                SourceTextPolicy::HanOnly,
                None,
                expand,
            )
            .unwrap(),
            PreparedInpaintMask::NoEligibleHanTargets
        ));
        assert_eq!(calls.get(), 0);

        let owner = NodeId::new();
        let empty = DynamicImage::ImageLuma8(GrayImage::new(8, 4));
        assert!(
            super::prepare_inpaint_mask(
                &empty,
                &bubble,
                &[],
                &[node_line(owner, [1.0, 1.0, 3.0, 3.0])],
                &[],
                SourceTextPolicy::HanOnly,
                None,
                expand,
            )
            .is_err()
        );
        assert_eq!(calls.get(), 0);

        let second = NodeId::new();
        assert!(
            super::prepare_inpaint_mask(
                &source,
                &bubble,
                &[],
                &[
                    node_line(owner, [1.0, 1.0, 3.0, 3.0]),
                    node_line(second, [5.0, 1.0, 7.0, 3.0]),
                ],
                &[],
                SourceTextPolicy::HanOnly,
                None,
                expand,
            )
            .is_err()
        );
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn final_inpaint_mask_limits_han_support_before_and_after_expansion() {
        let mask = DynamicImage::ImageLuma8(GrayImage::from_fn(32, 16, |x, y| {
            Luma([if (x == 5 || x == 20) && y == 8 {
                255
            } else {
                0
            }])
        }));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(32, 16));
        let eligible = vec![support_line(
            [18.0, 6.0, 23.0, 11.0],
            Some(vec![quad(18.0, 6.0, 23.0, 11.0)]),
        )];

        for expand in [
            expand_mask_for_inpainting
                as fn(&DynamicImage, &DynamicImage, &[koharu_ml::types::TextRegion]) -> GrayImage,
            expand_mask_to_bubble_region_for_inpainting,
        ] {
            let (prepared, blocks) = prepare_inpaint_mask(
                &mask,
                &bubble,
                &[],
                &eligible,
                SourceTextPolicy::HanOnly,
                None,
                expand,
            )
            .expect("Han target should produce a final mask");
            let prepared = prepared.to_luma8();

            assert_eq!(blocks.len(), 1);
            assert_eq!(prepared.get_pixel(5, 8).0[0], 0);
            assert_eq!(prepared.get_pixel(17, 8).0[0], 0);
            assert_ne!(prepared.get_pixel(20, 8).0[0], 0);
            assert_eq!(prepared.get_pixel(23, 8).0[0], 0);
        }
    }

    #[test]
    fn final_inpaint_mask_keeps_word_box_english_word_zero() {
        let text = text_data(
            "Peach\n蜜桃臀",
            Some(vec![quad(2.0, 2.0, 12.0, 8.0), quad(18.0, 2.0, 30.0, 8.0)]),
        );
        let transform = Transform {
            x: 0.0,
            y: 0.0,
            width: 32.0,
            height: 12.0,
            rotation_deg: 0.0,
        };
        let eligible = eligible_text_lines(&transform, &text, 32, 12).expect("safe word boxes");
        let mask = DynamicImage::ImageLuma8(GrayImage::from_pixel(32, 12, Luma([255])));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(32, 12));

        let owner = NodeId::new();
        let protected = support_line([2.0, 2.0, 12.0, 8.0], Some(vec![quad(2.0, 2.0, 12.0, 8.0)]));
        let PreparedInpaintMask::Prepared { mask: prepared, .. } = super::prepare_inpaint_mask(
            &mask,
            &bubble,
            &[],
            &eligible
                .into_iter()
                .map(|line| (owner, line))
                .collect::<Vec<_>>(),
            &[(owner, protected)],
            SourceTextPolicy::HanOnly,
            None,
            |mask, _, _| mask.to_luma8(),
        )
        .expect("Han target") else {
            panic!("Han target should prepare a mask");
        };
        let prepared = prepared.to_luma8();

        assert_eq!(prepared.get_pixel(6, 5).0[0], 0);
        assert_ne!(prepared.get_pixel(24, 5).0[0], 0);
    }

    #[test]
    fn final_inpaint_mask_preserves_repair_region_semantics() {
        let mask = DynamicImage::ImageLuma8(GrayImage::from_fn(16, 12, |x, y| {
            Luma([if x == 5 && y == 6 { 255 } else { 0 }])
        }));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(16, 12));
        let all_blocks = vec![support_line([0.0, 0.0, 16.0, 12.0], None).region];
        let region = Region {
            x: 3,
            y: 4,
            width: 5,
            height: 4,
        };

        let (prepared, blocks) = prepare_inpaint_mask(
            &mask,
            &bubble,
            &all_blocks,
            &[],
            SourceTextPolicy::HanOnly,
            Some(region),
            expand_mask_for_inpainting,
        )
        .expect("repair mask should not require Han lines");
        let prepared = prepared.to_luma8();

        assert_eq!(blocks.len(), 1);
        assert_ne!(prepared.get_pixel(5, 6).0[0], 0);
        assert_eq!(prepared.get_pixel(2, 6).0[0], 0);
        assert_eq!(prepared.get_pixel(8, 6).0[0], 0);
    }

    #[test]
    fn final_inpaint_mask_short_circuits_empty_results() {
        let empty = DynamicImage::ImageLuma8(GrayImage::new(8, 8));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(8, 8));
        let nonempty = DynamicImage::ImageLuma8(GrayImage::from_fn(8, 8, |x, y| {
            Luma([if x == 3 && y == 3 { 255 } else { 0 }])
        }));

        assert!(
            prepare_inpaint_mask(
                &empty,
                &bubble,
                &[],
                &[support_line([2.0, 2.0, 5.0, 5.0], None)],
                SourceTextPolicy::HanOnly,
                None,
                expand_mask_for_inpainting,
            )
            .is_none()
        );
        assert!(
            prepare_inpaint_mask(
                &nonempty,
                &bubble,
                &[],
                &[],
                SourceTextPolicy::HanOnly,
                None,
                expand_mask_for_inpainting,
            )
            .is_none()
        );
        assert!(
            prepare_inpaint_mask(
                &nonempty,
                &bubble,
                &[],
                &[support_line([2.0, 2.0, 5.0, 5.0], None)],
                SourceTextPolicy::HanOnly,
                None,
                |mask, _, _| GrayImage::new(mask.width(), mask.height()),
            )
            .is_none()
        );
    }

    #[test]
    fn erase_diagnostics_lock_inpaint_observation_without_changing_results() {
        fn returned_bytes(result: &Option<(DynamicImage, Vec<TextRegion>)>) -> Option<Vec<u8>> {
            result.as_ref().map(|(mask, _)| mask.to_luma8().into_raw())
        }
        fn signature(event: &EraseDiagnosticEvent) -> Option<(u32, u32, u64, &str)> {
            event.mask.as_ref().map(|mask| {
                (
                    mask.width,
                    mask.height,
                    mask.nonzero_pixels,
                    mask.grayscale_blake3.as_str(),
                )
            })
        }

        let mask = DynamicImage::ImageLuma8(GrayImage::from_fn(6, 4, |x, y| {
            Luma([if (x, y) == (1, 1) || (x, y) == (4, 2) {
                255
            } else {
                0
            }])
        }));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(6, 4));
        let eligible = vec![support_line(
            [3.0, 1.0, 6.0, 4.0],
            Some(vec![quad(3.0, 1.0, 6.0, 4.0)]),
        )];
        let all_blocks = vec![support_line([0.0, 0.0, 6.0, 4.0], None).region];
        let calls = Cell::new(0);
        let expand = |input: &DynamicImage, _: &DynamicImage, _: &[TextRegion]| {
            calls.set(calls.get() + 1);
            let mut expanded = input.to_luma8();
            expanded.put_pixel(0, 0, Luma([255]));
            expanded
        };

        let inactive_han = prepare_inpaint_mask(
            &mask,
            &bubble,
            &all_blocks,
            &eligible,
            SourceTextPolicy::HanOnly,
            None,
            expand,
        );
        let capture = start_erase_capture();
        let active_han = prepare_inpaint_mask(
            &mask,
            &bubble,
            &all_blocks,
            &eligible,
            SourceTextPolicy::HanOnly,
            None,
            expand,
        );
        assert_eq!(returned_bytes(&active_han), returned_bytes(&inactive_han));
        let han_events = capture.take();
        assert_eq!(
            han_events
                .iter()
                .map(|event| event.stage)
                .collect::<Vec<_>>(),
            [
                EraseDiagnosticStage::InpaintInputSegment,
                EraseDiagnosticStage::InpaintAllowedSupport,
                EraseDiagnosticStage::InpaintPreExpandFiltered,
                EraseDiagnosticStage::InpaintBackendExpanded,
                EraseDiagnosticStage::InpaintFinal,
            ]
        );
        assert!(
            han_events
                .iter()
                .all(|event| event.branch == EraseDiagnosticBranch::HanOnly)
        );
        assert_eq!(han_events[0].mask.as_ref().unwrap().nonzero_pixels, 2);
        assert_eq!(han_events[1].mask.as_ref().unwrap().nonzero_pixels, 1);
        assert_eq!(han_events[2].mask.as_ref().unwrap().nonzero_pixels, 1);
        assert_eq!(han_events[3].mask.as_ref().unwrap().nonzero_pixels, 2);
        assert_eq!(han_events[4].mask.as_ref().unwrap().nonzero_pixels, 1);
        assert_eq!(han_events[4].returns_some, Some(true));
        let final_mask = capture
            .take_inpaint_final_mask()
            .expect("final inpaint mask bytes");
        assert_eq!(final_mask.dimensions(), (6, 4));
        assert_eq!(
            Some(final_mask.as_raw().clone()),
            returned_bytes(&active_han)
        );
        assert_eq!(
            blake3::hash(final_mask.as_raw()).to_hex().to_string(),
            han_events[4].mask.as_ref().unwrap().grayscale_blake3
        );
        assert!(capture.take_inpaint_final_mask().is_none());
        assert_eq!(
            han_events.iter().map(signature).collect::<Vec<_>>(),
            [
                Some((
                    6,
                    4,
                    2,
                    "116bebacbb6de73e4a2c236fbac6d3f91f38d751381dc759e9b1e88d25846b9e"
                )),
                Some((
                    6,
                    4,
                    1,
                    "64b0950aabbf0659df8c4bafaa54c9fef7840bdeec85703fb88775872fb540f3"
                )),
                Some((
                    6,
                    4,
                    1,
                    "64b0950aabbf0659df8c4bafaa54c9fef7840bdeec85703fb88775872fb540f3"
                )),
                Some((
                    6,
                    4,
                    2,
                    "0daa796901b99a35273ecabb931b38d050ce50ce33f86c9e737e23c7e31c471e"
                )),
                Some((
                    6,
                    4,
                    1,
                    "64b0950aabbf0659df8c4bafaa54c9fef7840bdeec85703fb88775872fb540f3"
                )),
            ]
        );

        drop(capture);
        let inactive_all = prepare_inpaint_mask(
            &mask,
            &bubble,
            &all_blocks,
            &eligible,
            SourceTextPolicy::AllText,
            None,
            expand,
        );
        let capture = start_erase_capture();
        let active_all = prepare_inpaint_mask(
            &mask,
            &bubble,
            &all_blocks,
            &eligible,
            SourceTextPolicy::AllText,
            None,
            expand,
        );
        assert_eq!(returned_bytes(&active_all), returned_bytes(&inactive_all));
        let all_events = capture.take();
        assert_eq!(all_events.len(), 5);
        assert!(
            all_events
                .iter()
                .all(|event| event.branch == EraseDiagnosticBranch::AllText)
        );
        assert_eq!(
            all_events[1].stage,
            EraseDiagnosticStage::InpaintAllowedSupport
        );
        assert_eq!(all_events[1].mask, None);
        assert_eq!(all_events[4].returns_some, Some(true));
        assert_eq!(
            all_events.iter().map(signature).collect::<Vec<_>>(),
            [
                Some((
                    6,
                    4,
                    2,
                    "116bebacbb6de73e4a2c236fbac6d3f91f38d751381dc759e9b1e88d25846b9e"
                )),
                None,
                Some((
                    6,
                    4,
                    2,
                    "116bebacbb6de73e4a2c236fbac6d3f91f38d751381dc759e9b1e88d25846b9e"
                )),
                Some((
                    6,
                    4,
                    3,
                    "f6c19c1de1f9d32968d49e6c8b1b481f0be5264e126c83d572817e8fe27c2bac"
                )),
                Some((
                    6,
                    4,
                    3,
                    "f6c19c1de1f9d32968d49e6c8b1b481f0be5264e126c83d572817e8fe27c2bac"
                )),
            ]
        );

        let region = Region {
            x: 1,
            y: 1,
            width: 3,
            height: 2,
        };
        drop(capture);
        let inactive_region = prepare_inpaint_mask(
            &mask,
            &bubble,
            &all_blocks,
            &eligible,
            SourceTextPolicy::HanOnly,
            Some(region),
            expand,
        );
        let capture = start_erase_capture();
        let active_region = prepare_inpaint_mask(
            &mask,
            &bubble,
            &all_blocks,
            &eligible,
            SourceTextPolicy::HanOnly,
            Some(region),
            expand,
        );
        assert_eq!(
            returned_bytes(&active_region),
            returned_bytes(&inactive_region)
        );
        let region_events = capture.take();
        assert_eq!(region_events.len(), 5);
        assert!(
            region_events
                .iter()
                .all(|event| event.branch == EraseDiagnosticBranch::Region)
        );
        assert_eq!(region_events[1].mask, None);
        assert_eq!(region_events[2].mask.as_ref().unwrap().nonzero_pixels, 1);
        assert_eq!(region_events[3].mask.as_ref().unwrap().nonzero_pixels, 2);
        assert_eq!(region_events[4].mask.as_ref().unwrap().nonzero_pixels, 1);
        assert_eq!(
            region_events.iter().map(signature).collect::<Vec<_>>(),
            [
                Some((
                    6,
                    4,
                    2,
                    "116bebacbb6de73e4a2c236fbac6d3f91f38d751381dc759e9b1e88d25846b9e"
                )),
                None,
                Some((
                    6,
                    4,
                    1,
                    "0fc4241829876639d2362de149a9a5f1d0d3d687e0cc2f51743d4981bf7d696c"
                )),
                Some((
                    6,
                    4,
                    2,
                    "2ca0bf1a4387623866ef72bf9660f7ebf72bb21e0d3de4caa748757481d93dcf"
                )),
                Some((
                    6,
                    4,
                    1,
                    "0fc4241829876639d2362de149a9a5f1d0d3d687e0cc2f51743d4981bf7d696c"
                )),
            ]
        );

        let empty = DynamicImage::ImageLuma8(GrayImage::new(6, 4));
        drop(capture);
        let inactive_empty = prepare_inpaint_mask(
            &empty,
            &bubble,
            &all_blocks,
            &eligible,
            SourceTextPolicy::AllText,
            None,
            |input, _, _| {
                calls.set(calls.get() + 1);
                input.to_luma8()
            },
        );
        let capture = start_erase_capture();
        let active_empty = prepare_inpaint_mask(
            &empty,
            &bubble,
            &all_blocks,
            &eligible,
            SourceTextPolicy::AllText,
            None,
            |input, _, _| {
                calls.set(calls.get() + 1);
                input.to_luma8()
            },
        );
        assert_eq!(
            returned_bytes(&active_empty),
            returned_bytes(&inactive_empty)
        );
        assert_eq!(returned_bytes(&active_empty), None);
        let empty_events = capture.take();
        assert_eq!(empty_events[4].returns_some, Some(false));
        assert_eq!(calls.get(), 8);

        let serialized = serde_json::to_value(&han_events).unwrap();
        for event in serialized.as_array().unwrap() {
            let object = event.as_object().unwrap();
            assert_eq!(object.len(), 4);
            assert!(object.contains_key("stage"));
            assert!(object.contains_key("branch"));
            assert!(object.contains_key("mask"));
            assert!(object.contains_key("returns_some"));
            if let Some(mask) = object["mask"].as_object() {
                assert_eq!(mask.len(), 4);
                assert!(mask.contains_key("width"));
                assert!(mask.contains_key("height"));
                assert!(mask.contains_key("grayscale_blake3"));
                assert!(mask.contains_key("nonzero_pixels"));
            }
        }
        let serialized_text = serde_json::to_string(&serialized).unwrap();
        for forbidden in [
            "path",
            "node_id",
            "text",
            "target",
            "elapsed",
            "ocr",
            "translation",
        ] {
            assert!(!serialized_text.contains(forbidden));
        }
        let mut unknown = serialized[0].clone();
        unknown["unexpected"] = true.into();
        assert!(serde_json::from_value::<EraseDiagnosticEvent>(unknown).is_err());
        let mut unknown_mask = serialized[0].clone();
        unknown_mask["mask"]["unexpected"] = true.into();
        assert!(serde_json::from_value::<EraseDiagnosticEvent>(unknown_mask).is_err());
        let mut missing = serialized[0].clone();
        missing.as_object_mut().unwrap().remove("returns_some");
        assert!(serde_json::from_value::<EraseDiagnosticEvent>(missing).is_err());

        drop(capture);
        let reset = start_erase_capture();
        assert!(reset.take().is_empty());
    }

    #[test]
    fn erase_diagnostics_ignore_foreign_thread_events_between_owner_stages() {
        let capture = start_erase_capture();
        let owner_mask =
            GrayImage::from_fn(2, 2, |x, y| Luma([if (x, y) == (1, 1) { 255 } else { 0 }]));
        record_erase_diagnostic(
            EraseDiagnosticStage::SegmentProbability,
            EraseDiagnosticBranch::HanOnly,
            Some(&owner_mask),
            None,
        );
        assert!(capture.take_inpaint_final_mask().is_none());

        let barrier = Arc::new(Barrier::new(2));
        let foreign_barrier = barrier.clone();
        let foreign = std::thread::spawn(move || {
            let mask = DynamicImage::ImageLuma8(GrayImage::from_pixel(2, 2, Luma([255])));
            let bubble = DynamicImage::ImageLuma8(GrayImage::new(2, 2));
            foreign_barrier.wait();
            let result = prepare_inpaint_mask(
                &mask,
                &bubble,
                &[],
                &[],
                SourceTextPolicy::AllText,
                None,
                |input, _, _| input.to_luma8(),
            );
            foreign_barrier.wait();
            assert!(result.is_some());
        });

        barrier.wait();
        barrier.wait();
        record_erase_diagnostic(
            EraseDiagnosticStage::SegmentFinal,
            EraseDiagnosticBranch::HanOnly,
            Some(&owner_mask),
            None,
        );
        foreign.join().unwrap();

        assert!(capture.take_inpaint_final_mask().is_none());
        let events = capture.take();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events.iter().map(|event| event.stage).collect::<Vec<_>>(),
            [
                EraseDiagnosticStage::SegmentProbability,
                EraseDiagnosticStage::SegmentFinal,
            ]
        );
        assert!(
            events
                .iter()
                .all(|event| event.branch == EraseDiagnosticBranch::HanOnly)
        );
        assert_eq!(events[0].mask, events[1].mask);
        assert_eq!(events[0].mask.as_ref().unwrap().nonzero_pixels, 1);
    }

    #[test]
    fn erase_diagnostics_nested_start_unwind_and_poison_recover() {
        let capture = start_erase_capture();
        assert!(matches!(
            EraseDiagnosticCapture::start(),
            Err(EraseDiagnosticCaptureActive)
        ));
        drop(capture);

        let unwind = std::panic::catch_unwind(|| {
            let _capture = start_erase_capture();
            panic!("intentional erase diagnostic unwind");
        });
        assert!(unwind.is_err());

        let capture = start_erase_capture();
        let coordination = ERASE_DIAGNOSTIC_SINK.get().unwrap();
        let coordination_poison = std::thread::spawn(move || {
            let _guard = coordination
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            panic!("intentional erase diagnostic coordination poison");
        });
        assert!(coordination_poison.join().is_err());
        drop(capture);

        let capture = start_erase_capture();
        let events = capture.events.clone();
        let event_poison = std::thread::spawn(move || {
            let _guard = events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            panic!("intentional erase diagnostic event poison");
        });
        assert!(event_poison.join().is_err());
        assert!(capture.take().is_empty());
        drop(capture);

        let restarted = start_erase_capture();
        assert!(restarted.take().is_empty());
    }

    fn translation_scene(nodes: Vec<Node>) -> (Scene, PageId) {
        let mut page = Page::new("page", 100, 100);
        let page_id = page.id;
        page.nodes = nodes.into_iter().map(|node| (node.id, node)).collect();
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        (scene, page_id)
    }

    fn translated_node(id: NodeId, text: &str) -> Node {
        Node {
            id,
            transform: transform(),
            visible: true,
            kind: NodeKind::Text(TextData {
                text: Some(text.to_string()),
                translation: Some("old translation".to_string()),
                sprite: Some(BlobRef::new("old-sprite")),
                sprite_transform: Some(transform()),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn detector_cleanup_clears_empty_han_only_but_preserves_empty_all_text() {
        let id = NodeId::new();
        let (scene, page) = translation_scene(vec![translated_node(id, "旧文本")]);

        let han_only = clear_text_nodes_ops(&scene, page, 0, true);
        assert_eq!(han_only.len(), 1);
        assert!(matches!(han_only[0], Op::RemoveNode { id: removed, .. } if removed == id));

        assert!(clear_text_nodes_ops(&scene, page, 0, false).is_empty());

        let ops = clear_text_nodes_ops(&scene, page, 1, false);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], Op::RemoveNode { id: removed, .. } if removed == id));
    }

    fn target(node_id: NodeId, line_index: usize) -> (NodeId, EligibleTextLine) {
        let mut line = support_line([10.0, 10.0, 30.0, 20.0], None);
        line.line_index = line_index;
        (node_id, line)
    }

    #[test]
    fn han_translation_ops_reject_mismatched_or_duplicate_targets() {
        let id = NodeId::new();
        let (scene, page) = translation_scene(vec![translated_node(id, "中文")]);

        assert!(build_han_only_translation_ops(&scene, page, None, &[target(id, 0)], &[]).is_err());
        assert!(
            build_han_only_translation_ops(
                &scene,
                page,
                None,
                &[target(id, 0), target(id, 0)],
                &["一".to_string(), "二".to_string()],
            )
            .is_err()
        );
    }

    #[test]
    fn han_translation_ops_map_lines_atomically_and_cleanup_in_scope() {
        let eligible = NodeId::new();
        let english = NodeId::new();
        let unsupported = NodeId::new();
        let outside = NodeId::new();
        let (mut scene, page) = translation_scene(vec![
            translated_node(eligible, "English\n中文一\n中文二"),
            translated_node(english, "English"),
            translated_node(unsupported, "English\n中文"),
            translated_node(outside, "中文"),
        ]);
        let allowed = [eligible, english, unsupported];

        let mut ops = build_han_only_translation_ops(
            &scene,
            page,
            Some(&allowed),
            &[target(eligible, 2), target(eligible, 1)],
            &["译文二".to_string(), "译文一".to_string()],
        )
        .unwrap();
        for op in &mut ops {
            op.apply(&mut scene).unwrap();
        }

        let text = |id| match &scene.node(page, id).unwrap().kind {
            NodeKind::Text(text) => text,
            _ => panic!("expected text node"),
        };
        assert_eq!(
            text(eligible).translation.as_deref(),
            Some("译文一\n译文二")
        );
        assert!(text(eligible).sprite.is_none());
        assert!(text(eligible).sprite_transform.is_none());
        for id in [english, unsupported] {
            assert!(text(id).translation.is_none());
            assert!(text(id).sprite.is_none());
            assert!(text(id).sprite_transform.is_none());
        }
        assert_eq!(
            text(outside).translation.as_deref(),
            Some("old translation")
        );
        assert!(text(outside).sprite.is_some());
        assert!(text(outside).sprite_transform.is_some());
    }

    #[test]
    fn han_translation_ops_empty_targets_still_cleanup() {
        let english = NodeId::new();
        let outside = NodeId::new();
        let (mut scene, page) = translation_scene(vec![
            translated_node(english, "English"),
            translated_node(outside, "English"),
        ]);

        let mut ops =
            build_han_only_translation_ops(&scene, page, Some(&[english]), &[], &[]).unwrap();
        for op in &mut ops {
            op.apply(&mut scene).unwrap();
        }

        let NodeKind::Text(english_text) = &scene.node(page, english).unwrap().kind else {
            panic!("expected text node");
        };
        let NodeKind::Text(outside_text) = &scene.node(page, outside).unwrap().kind else {
            panic!("expected text node");
        };
        assert!(english_text.translation.is_none());
        assert_eq!(outside_text.translation.as_deref(), Some("old translation"));
    }

    #[test]
    fn inpainting_engines_share_identical_region_clipping() {
        let source = GrayImage::from_fn(4, 3, |x, y| Luma([(x + y * 4 + 1) as u8]));
        let region = Region {
            x: 1,
            y: 1,
            width: 2,
            height: 9,
        };

        let gray = clip_gray_mask_to_region(&source, &region);
        let dynamic =
            clip_mask_to_region(&DynamicImage::ImageLuma8(source.clone()), &region).to_luma8();

        assert_eq!(gray, dynamic);
        assert_eq!(gray.dimensions(), source.dimensions());
        assert_eq!(gray.get_pixel(1, 1), source.get_pixel(1, 1));
        assert_eq!(gray.get_pixel(2, 2), source.get_pixel(2, 2));
        assert_eq!(gray.get_pixel(0, 1).0, [0]);
        assert_eq!(gray.get_pixel(3, 2).0, [0]);
    }

    #[test]
    fn test_reading_order_sort() {
        // Two blocks side-by-side
        // B1: [100, 100, 200, 200] (Left)
        // B2: [300, 100, 400, 200] (Right)
        let b1 = [100.0, 100.0, 200.0, 200.0];
        let b2 = [300.0, 100.0, 400.0, 200.0];

        let mut blocks = vec![(b1, "left"), (b2, "right")];

        // RTL: Right should come first
        sort_manga_reading_order(&mut blocks, ReadingOrder::Rtl);
        assert_eq!(blocks[0].1, "right");
        assert_eq!(blocks[1].1, "left");

        // LTR: Left should come first
        sort_manga_reading_order(&mut blocks, ReadingOrder::Ltr);
        assert_eq!(blocks[0].1, "left");
        assert_eq!(blocks[1].1, "right");
    }
}
