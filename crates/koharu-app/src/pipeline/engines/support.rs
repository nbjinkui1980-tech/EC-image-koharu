//! Shared helpers used by multiple engine implementations.
//!
//! The patterns here map `koharu-ml` / `koharu-llm` outputs (plain
//! `TextRegion`s, `DynamicImage`s) into `Op` sequences that mutate the scene.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, GrayImage, Luma};
use imageproc::{drawing::draw_polygon_mut, point::Point};
use koharu_core::{
    BlobRef, ImageData, ImageRole, MaskData, MaskRole, Node, NodeDataPatch, NodeId, NodeKind,
    NodePatch, Op, PageId, ReadingOrder, Region, Scene, TextData, TextDataPatch, Transform,
};
use koharu_ml::types::TextRegion;

use crate::{blobs::BlobStore, config::SourceTextPolicy};

pub const SOURCE_GATE_TARGET_DETECTOR: &str = "pp-ocr-v5-source-gate";
pub const SOURCE_GATE_PROTECTED_DETECTOR: &str = "pp-ocr-v5-source-gate-protected";

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
        let verified_single_region_reflow = text.typography_plan_verified
            && text
                .translation
                .as_deref()
                .is_some_and(|translation| !translation.trim().is_empty())
            && eligible.len() == 1;
        if translated_line_count != eligible.len() && !verified_single_region_reflow {
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
        let bbox = match line.region.line_polygons.as_deref() {
            Some([quad]) if quad_is_axis_aligned(quad) => {
                if quad.iter().flatten().any(|value| !value.is_finite()) {
                    continue;
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
                    continue;
                }
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
            }
            None => [
                line.region.x,
                line.region.y,
                line.region.x + line.region.width,
                line.region.y + line.region.height,
            ],
            _ => continue,
        };
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

pub fn prepare_inpaint_mask<Expand>(
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
    let inference_blocks = if region.is_none() && policy == SourceTextPolicy::HanOnly {
        eligible_lines
            .iter()
            .map(|line| line.region.clone())
            .collect::<Vec<_>>()
    } else {
        all_blocks.to_vec()
    };

    let final_mask = if let Some(region) = region {
        let clipped_mask = clip_mask_to_region(mask, &region);
        let clipped_bubble = clip_mask_to_region(bubble_mask, &region);
        let expanded = expand(&clipped_mask, &clipped_bubble, &inference_blocks);
        DynamicImage::ImageLuma8(clip_gray_mask_to_region(&expanded, &region))
    } else if policy == SourceTextPolicy::HanOnly {
        let allowed = line_support_mask(mask.width(), mask.height(), eligible_lines);
        let filtered = DynamicImage::ImageLuma8(intersect_gray_masks(&mask.to_luma8(), &allowed));
        let expanded = expand(&filtered, bubble_mask, &inference_blocks);
        DynamicImage::ImageLuma8(intersect_gray_masks(&expanded, &allowed))
    } else {
        DynamicImage::ImageLuma8(expand(mask, bubble_mask, &inference_blocks))
    };

    final_mask
        .to_luma8()
        .pixels()
        .any(|pixel| pixel.0[0] != 0)
        .then_some((final_mask, inference_blocks))
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

/// Sort `(bbox, data)` pairs in a reading order (RTL, LTR, or Custom).
pub fn sort_manga_reading_order<T>(blocks: &mut [([f32; 4], T)], order: ReadingOrder) {
    #[derive(Debug, PartialEq, Clone, Copy)]
    enum Axis {
        X,
        Y,
    }

    if order == ReadingOrder::Custom {
        return;
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
                        _ => Ordering::Equal,
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
                    _ => false,
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
                        _ => true,
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
                _ => Ordering::Equal,
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
    use super::*;
    use koharu_core::{BlobRef, Page, ReadingOrder, Region, TextDirection};
    use koharu_ml::inpainting::{
        expand_mask_for_inpainting, mask::expand_mask_to_bubble_region_for_inpainting,
    };

    use crate::config::SourceTextPolicy;

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

        let (prepared, _) = prepare_inpaint_mask(
            &mask,
            &bubble,
            &[],
            &eligible,
            SourceTextPolicy::HanOnly,
            None,
            |mask, _, _| mask.to_luma8(),
        )
        .expect("Han target");
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
