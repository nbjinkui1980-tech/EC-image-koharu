use std::{collections::HashSet, sync::Mutex};

use anyhow::{Result, ensure};
use async_trait::async_trait;
use image::DynamicImage;
use koharu_core::{
    ImageRole, MaskRole, Node, NodeDataPatch, NodeId, NodeKind, NodePatch, Op, PageId, Scene,
    TextData, TextDataPatch, Transform,
};
use koharu_llm::paddleocr_vl::{PaddleOcrVl, PaddleOcrVlTask};
use koharu_ml::pp_ocr_v5::{PpOcrV5, PpOcrWordBox};

use crate::app::shared_llama_backend;
use crate::pipeline::engine::{Engine, EngineCtx};
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

pub(super) fn pp_may_contain_han(words: &[PpOcrWordBox]) -> bool {
    words.iter().any(|word| contains_han(&word.text))
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

pub(super) fn validate_pp_vl_alignment(
    vl_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> Option<Vec<ValidatedWord>> {
    let [crop_left, crop_top, crop_right, crop_bottom] = crop_bounds;
    if crop_left >= crop_right
        || crop_top >= crop_bottom
        || crop_right > image_width
        || crop_bottom > image_height
        || words.is_empty()
    {
        return None;
    }

    let vl_chars = vl_text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<Vec<_>>();
    let crop_width = (crop_right - crop_left) as f32;
    let crop_height = (crop_bottom - crop_top) as f32;
    let mut validated = Vec::with_capacity(words.len());
    let mut vl_offset = 0_usize;
    let mut previous_line = None;
    let mut previous_right = 0.0;

    for word in words {
        let pp_chars = word
            .text
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<Vec<_>>();
        let end = vl_offset.checked_add(pp_chars.len())?;
        let authoritative = vl_chars.get(vl_offset..end)?;
        if pp_chars.is_empty()
            || !word.confidence.is_finite()
            || word.confidence < MIN_WORD_CONFIDENCE
            || !pp_chars.iter().zip(authoritative).all(|(pp, vl)| {
                pp == vl || (contains_han(&pp.to_string()) && contains_han(&vl.to_string()))
            })
        {
            return None;
        }

        let [left, top, right, bottom] = word.bbox;
        if word.bbox.iter().any(|value| !value.is_finite())
            || left < 0.0
            || top < 0.0
            || right > crop_width
            || bottom > crop_height
            || left >= right
            || top >= bottom
        {
            return None;
        }
        if let Some(line_index) = previous_line
            && (word.line_index < line_index
                || (word.line_index == line_index && left < previous_right))
        {
            return None;
        }
        previous_line = Some(word.line_index);
        previous_right = right;
        vl_offset = end;

        let text = authoritative.iter().collect::<String>();
        let protected = contains_protected_latin_word(&text);
        if protected && contains_han(&text) {
            return None;
        }
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

    (vl_offset == vl_chars.len()).then_some(validated)
}

pub(super) fn select_chinese_target(
    vl_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> Option<SourceSelection> {
    let validated =
        validate_pp_vl_alignment(vl_text, words, crop_bounds, image_width, image_height)?;
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
                    return None;
                }
                han_runs.pop()?
            };
            let bbox = bbox_union(&validated, &target_indices)?;
            let text = target_indices
                .iter()
                .map(|index| validated[*index].text.as_str())
                .collect::<String>();
            if text.is_empty() || !contains_han(&text) {
                return None;
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
        return None;
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
        return None;
    }

    indexed_targets.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    Some(SourceSelection {
        targets: indexed_targets
            .into_iter()
            .map(|(_, _, target)| target)
            .collect(),
        protected_lines,
    })
}

struct SourceGateCandidate {
    node_id: NodeId,
    crop: DynamicImage,
    crop_bounds: [u32; 4],
}

fn is_gate_marker(text: &TextData) -> bool {
    matches!(
        text.detector.as_deref(),
        Some(SOURCE_GATE_TARGET_DETECTOR | SOURCE_GATE_PROTECTED_DETECTOR)
    )
}

fn safe_crop_bounds(
    transform: &Transform,
    image_width: u32,
    image_height: u32,
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
    let left = transform.x.floor().max(0.0).min(image_width as f32) as u32;
    let top = transform.y.floor().max(0.0).min(image_height as f32) as u32;
    let right = (transform.x + transform.width)
        .ceil()
        .max(0.0)
        .min(image_width as f32) as u32;
    let bottom = (transform.y + transform.height)
        .ceil()
        .max(0.0)
        .min(image_height as f32) as u32;
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
    for (node_id, node) in &page_ref.nodes {
        let NodeKind::Text(text) = &node.kind else {
            continue;
        };
        if is_gate_marker(text) {
            continue;
        }
        let Some([left, top, right, bottom]) =
            safe_crop_bounds(&node.transform, image.width(), image.height())
        else {
            invalid.push(*node_id);
            continue;
        };
        candidates.push(SourceGateCandidate {
            node_id: *node_id,
            crop: image.crop_imm(left, top, right - left, bottom - top),
            crop_bounds: [left, top, right, bottom],
        });
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

fn target_text_data(target: SourceTarget, detector: &str) -> TextData {
    TextData {
        source_lang: (detector == SOURCE_GATE_TARGET_DETECTOR).then(|| "zh".into()),
        rotation_deg: Some(0.0),
        detector: Some(detector.into()),
        text: Some(target.text),
        line_polygons: Some(vec![target.line_polygon]),
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
            kind: NodeKind::Text(target_text_data(target, SOURCE_GATE_TARGET_DETECTOR)),
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
        let node = Node {
            id: NodeId::new(),
            transform: target_transform(bbox),
            visible: false,
            kind: NodeKind::Text(target_text_data(target, SOURCE_GATE_PROTECTED_DETECTOR)),
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

pub(super) async fn dispatch_source_gate<WordBoxes, Validate, Fut>(
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
    let mut rejected = invalid;
    let mut pending = Vec::new();
    let mut accepted = scene
        .page(page)
        .into_iter()
        .flat_map(|page| page.nodes.values())
        .filter(|node| {
            node.visible
                && matches!(&node.kind, NodeKind::Text(text) if text.detector.as_deref() == Some(SOURCE_GATE_TARGET_DETECTOR))
        })
        .count();

    for candidate in candidates {
        let words = word_boxes(candidate.node_id, &candidate.crop)?;
        if pp_may_contain_han(&words) {
            pending.push((candidate, words));
        } else {
            rejected.push(candidate.node_id);
        }
    }

    let vl_texts = if pending.is_empty() {
        Vec::new()
    } else {
        validate(
            pending
                .iter()
                .map(|(candidate, _)| candidate.crop.clone())
                .collect(),
        )
        .await?
    };
    ensure!(
        vl_texts.len() == pending.len(),
        "source gate OCR count mismatch"
    );

    let mut next_at = scene.page(page).map(|page| page.nodes.len()).unwrap_or(0);
    let mut mutations = Vec::new();
    for ((candidate, words), vl_text) in pending.into_iter().zip(vl_texts) {
        match select_chinese_target(
            &vl_text,
            &words,
            candidate.crop_bounds,
            image.width(),
            image.height(),
        ) {
            Some(selection) => {
                accepted += selection.targets.len();
                mutations.extend(update_target_ops(
                    page,
                    candidate.node_id,
                    selection,
                    &mut next_at,
                )?);
            }
            None => rejected.push(candidate.node_id),
        }
    }

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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use image::{DynamicImage, RgbImage};
    use koharu_core::{
        BlobRef, ImageData, MaskData, Node, NodeId, NodeKind, Page, Scene, TextData, Transform,
    };
    use koharu_ml::pp_ocr_v5::PpOcrWordBox;

    use super::*;

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
        assert!(!pp_may_contain_han(&words));
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
        assert!(
            select_chinese_target(
                "Peach蜜桃臀",
                &[
                    word("Beach", 0, 0.0, 0.0, 40.0, 20.0),
                    word("蜜桃臀", 0, 45.0, 0.0, 100.0, 20.0),
                ],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
        assert!(
            select_chinese_target(
                "AI智能塑形",
                &[word("AI智能塑形", 0, 0.0, 0.0, 100.0, 20.0)],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
        assert!(
            select_chinese_target(
                "中文",
                &[word("中文", 0, f32::NAN, 0.0, 40.0, 20.0)],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
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
            crate::pipeline::engines::support::protected_source_lines_for_page(&scene, page);
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
}
