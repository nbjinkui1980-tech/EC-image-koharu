//! PaddleOCR-VL. Vision-language OCR driven by llama.cpp + mtmd.
//!
//! Each text node on the page is cropped out of the source image, passed
//! through the multimodal model, and the recognised text is written back
//! via `UpdateNode { TextDataPatch { text } }`.

use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use image::DynamicImage;
use koharu_core::{NodeDataPatch, NodePatch, Op, TextDataPatch};
use koharu_llm::paddleocr_vl::{PaddleOcrVl, PaddleOcrVlOutput, PaddleOcrVlTask};
use koharu_ml::comic_text_detector::{crop_text_block_bbox, expanded_text_block_crop_bounds};
use koharu_ml::pp_ocr_v5::{PpOcrV5, PpOcrWordBox};
use tokio::sync::Mutex as AsyncMutex;

use crate::app::shared_llama_backend;
use crate::config::SourceTextPolicy;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    contains_han, contains_protected_latin_word, load_source_image, text_node_to_region, text_nodes,
};

const MAX_NEW_TOKENS: usize = 256;
const MIN_WORD_CONFIDENCE: f32 = 0.5;

fn needs_inline_word_boxes(policy: SourceTextPolicy, text: &str) -> bool {
    policy == SourceTextPolicy::HanOnly
        && text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .any(|line| contains_han(line) && contains_protected_latin_word(line))
}

type WordBoxUpdate = (String, Vec<[[f32; 2]; 4]>);

#[derive(Clone)]
struct ValidatedWord {
    line_index: usize,
    text: String,
    bbox: [f32; 4],
    protected: bool,
}

fn build_pp_ocr_word_box_update(
    first_pass_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> Option<WordBoxUpdate> {
    let [crop_left, crop_top, crop_right, crop_bottom] = crop_bounds;
    if crop_left >= crop_right
        || crop_top >= crop_bottom
        || crop_right > image_width
        || crop_bottom > image_height
        || words.is_empty()
        || words
            .windows(2)
            .any(|pair| pair[0].line_index > pair[1].line_index)
    {
        return None;
    }

    let vl_chars = first_pass_text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<Vec<_>>();
    let mut vl_offset = 0_usize;
    let mut validated = Vec::with_capacity(words.len());
    let crop_width = (crop_right - crop_left) as f32;
    let crop_height = (crop_bottom - crop_top) as f32;
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
        vl_offset = end;

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
        let text = authoritative.iter().collect::<String>();
        let has_han = contains_han(&text);
        let protected = contains_protected_latin_word(&text);
        if has_han && protected {
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
    if vl_offset != vl_chars.len() {
        return None;
    }

    let han_indices = validated
        .iter()
        .enumerate()
        .filter_map(|(index, word)| contains_han(&word.text).then_some(index))
        .collect::<Vec<_>>();
    if han_indices.len() != 1 {
        return None;
    }
    let han_index = han_indices[0];
    let han_line = validated[han_index].line_index;
    if !validated
        .iter()
        .any(|word| word.line_index == han_line && word.protected)
    {
        return None;
    }

    let target_indices = validated
        .iter()
        .enumerate()
        .filter_map(|(index, word)| {
            (word.line_index == han_line && !word.protected).then_some(index)
        })
        .collect::<Vec<_>>();
    let first_target = *target_indices.first()?;
    let last_target = *target_indices.last()?;
    if !target_indices.contains(&han_index)
        || last_target - first_target + 1 != target_indices.len()
        || validated
            .iter()
            .any(|word| !word.protected && word.line_index != han_line)
    {
        return None;
    }

    let target_text = target_indices
        .iter()
        .map(|index| validated[*index].text.as_str())
        .collect::<String>();
    let target_bbox = target_indices.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |mut bbox, index| {
            let item = &validated[*index].bbox;
            bbox[0] = bbox[0].min(item[0]);
            bbox[1] = bbox[1].min(item[1]);
            bbox[2] = bbox[2].max(item[2]);
            bbox[3] = bbox[3].max(item[3]);
            bbox
        },
    );

    let mut logical = Vec::with_capacity(validated.len() - target_indices.len() + 1);
    for (index, word) in validated.into_iter().enumerate() {
        if index == first_target {
            logical.push(ValidatedWord {
                line_index: han_line,
                text: target_text.clone(),
                bbox: target_bbox,
                protected: false,
            });
        } else if !target_indices.contains(&index) {
            logical.push(word);
        }
    }
    for protected in logical.iter().filter(|word| word.protected) {
        if protected.bbox[0].max(target_bbox[0]) < protected.bbox[2].min(target_bbox[2])
            && protected.bbox[1].max(target_bbox[1]) < protected.bbox[3].min(target_bbox[3])
        {
            return None;
        }
    }

    let text = logical
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let polygons = logical
        .into_iter()
        .map(|word| {
            let [left, top, right, bottom] = word.bbox;
            [[left, top], [right, top], [right, bottom], [left, bottom]]
        })
        .collect();
    Some((text, polygons))
}

fn dispatch_inline_word_boxes<Infer>(
    policy: SourceTextPolicy,
    first_pass: &[PaddleOcrVlOutput],
    crops: &[DynamicImage],
    crop_bounds: &[[u32; 4]],
    image_width: u32,
    image_height: u32,
    mut infer: Infer,
) -> Vec<(bool, Option<WordBoxUpdate>)>
where
    Infer: FnMut(usize, &DynamicImage) -> anyhow::Result<Vec<PpOcrWordBox>>,
{
    let mut updates = first_pass
        .iter()
        .map(|output| {
            (
                needs_inline_word_boxes(policy, &output.text),
                None::<WordBoxUpdate>,
            )
        })
        .collect::<Vec<_>>();
    for (index, output) in first_pass.iter().enumerate() {
        if !updates[index].0 {
            continue;
        }
        let Some(crop) = crops.get(index) else {
            tracing::warn!(
                node_index = index,
                "PP-OCRv5 skipped because the crop is missing"
            );
            continue;
        };
        let Some(bounds) = crop_bounds.get(index).copied() else {
            tracing::warn!(
                node_index = index,
                "PP-OCRv5 skipped because crop bounds are missing"
            );
            continue;
        };
        let update = infer(index, crop).ok().and_then(|words| {
            build_pp_ocr_word_box_update(&output.text, &words, bounds, image_width, image_height)
        });
        if update.is_none() {
            tracing::warn!(
                node_index = index,
                "PP-OCRv5 word boxes failed PaddleOCR-VL validation and were skipped"
            );
        }
        updates[index].1 = update;
    }
    updates
}

fn build_ocr_text_patch(
    first_pass_text: String,
    word_boxes_attempted: bool,
    word_box_update: Option<WordBoxUpdate>,
) -> TextDataPatch {
    let (text, line_polygons) = match word_box_update {
        Some((text, polygons)) => (text, Some(Some(polygons))),
        None if word_boxes_attempted => (first_pass_text, Some(None)),
        None => (first_pass_text, None),
    };
    TextDataPatch {
        text: Some(Some(text)),
        line_polygons,
        ..Default::default()
    }
}

pub struct Model {
    vl: Mutex<PaddleOcrVl>,
    word_boxes: AsyncMutex<Option<PpOcrV5>>,
}

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        let texts = text_nodes(ctx.scene, ctx.page);
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let image = load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
        let text_regions: Vec<_> = texts
            .iter()
            .map(|(_, transform, text)| text_node_to_region(transform, text))
            .collect();
        let crop_bounds = text_regions
            .iter()
            .map(|region| expanded_text_block_crop_bounds(image.width(), image.height(), region))
            .collect::<Vec<_>>();
        let regions: Vec<_> = text_regions
            .iter()
            .map(|region| crop_text_block_bbox(&image, region))
            .collect();

        let outputs = {
            let mut ocr = self
                .vl
                .lock()
                .map_err(|_| anyhow::anyhow!("PaddleOCR mutex poisoned"))?;
            ocr.inference_images(&regions, PaddleOcrVlTask::Ocr, MAX_NEW_TOKENS)?
        };
        let has_candidates = outputs
            .iter()
            .any(|output| needs_inline_word_boxes(ctx.options.source_text_policy, &output.text));
        let word_box_updates = if has_candidates {
            let mut model = self.word_boxes.lock().await;
            if model.is_none() {
                match PpOcrV5::load(ctx.runtime).await {
                    Ok(loaded) => *model = Some(loaded),
                    Err(error) => tracing::warn!(
                        error = %error,
                        "PP-OCRv5 word geometry could not be loaded; mixed text remains unchanged"
                    ),
                }
            }
            dispatch_inline_word_boxes(
                ctx.options.source_text_policy,
                &outputs,
                &regions,
                &crop_bounds,
                image.width(),
                image.height(),
                |_, crop| match model.as_ref() {
                    Some(model) => model.word_boxes(crop),
                    None => Err(anyhow::anyhow!("PP-OCRv5 word geometry is unavailable")),
                },
            )
        } else {
            vec![(false, None); outputs.len()]
        };

        let mut ops = Vec::with_capacity(texts.len());
        for (((node_id, _, _), out), (word_boxes_attempted, word_box_update)) in
            texts.iter().zip(outputs).zip(word_box_updates)
        {
            ops.push(Op::UpdateNode {
                page: ctx.page,
                id: *node_id,
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(build_ocr_text_patch(
                        out.text,
                        word_boxes_attempted,
                        word_box_update,
                    ))),
                    transform: None,
                    visible: None,
                },
                prev: NodePatch::default(),
            });
        }
        Ok(ops)
    }
}

inventory::submit! {
    EngineInfo {
        id: "paddle-ocr-vl-1.6",
        name: "PaddleOCR-VL",
        needs: &[Artifact::TextBoxes],
        produces: &[Artifact::OcrText],
        load: |runtime, cpu| Box::pin(async move {
            let backend = shared_llama_backend(runtime)?;
            let m = PaddleOcrVl::load(runtime, cpu, backend).await?;
            Ok(Box::new(Model {
                vl: Mutex::new(m),
                word_boxes: AsyncMutex::new(None),
            }) as Box<dyn Engine>)
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use image::{DynamicImage, RgbImage};
    use koharu_llm::paddleocr_vl::PaddleOcrVlOutput;
    use koharu_ml::pp_ocr_v5::PpOcrWordBox;

    use super::*;
    use crate::config::SourceTextPolicy;

    fn output(text: &str) -> PaddleOcrVlOutput {
        PaddleOcrVlOutput {
            task: PaddleOcrVlTask::Ocr,
            text: text.to_string(),
            token_ids: Vec::new(),
            original_width: 20,
            original_height: 10,
            processed_width: 20,
            processed_height: 10,
            grid_thw: [1, 1, 1],
            num_image_tokens: 0,
        }
    }

    #[test]
    fn pp_ocr_word_boxes_use_vl_text_and_preserve_the_english_span() {
        let update = build_pp_ocr_word_box_update(
            "Peach蜜桃臀",
            &[
                PpOcrWordBox {
                    line_index: 0,
                    text: "Peach".to_string(),
                    bbox: [0.0, 0.0, 40.0, 20.0],
                    confidence: 0.9,
                },
                PpOcrWordBox {
                    line_index: 0,
                    text: "蜜桃餐".to_string(),
                    bbox: [45.0, 0.0, 100.0, 20.0],
                    confidence: 0.9,
                },
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .expect("safe PP-OCRv5 geometry");

        assert_eq!(update.0, "Peach\n蜜桃臀");
        assert_eq!(
            update.1,
            vec![
                [[10.0, 20.0], [50.0, 20.0], [50.0, 40.0], [10.0, 40.0]],
                [[55.0, 20.0], [110.0, 20.0], [110.0, 40.0], [55.0, 40.0]],
            ]
        );
    }

    fn word(text: &str, left: f32, right: f32) -> PpOcrWordBox {
        PpOcrWordBox {
            line_index: 0,
            text: text.to_string(),
            bbox: [left, 0.0, right, 10.0],
            confidence: 0.9,
        }
    }

    fn valid_word_boxes(english: &str, han: &str) -> Vec<PpOcrWordBox> {
        vec![word(english, 0.0, 8.0), word(han, 12.0, 20.0)]
    }

    #[test]
    fn requests_word_boxes_only_for_inline_han_with_a_complete_latin_word() {
        assert!(needs_inline_word_boxes(
            SourceTextPolicy::HanOnly,
            "Peach蜜桃臀"
        ));
        assert!(needs_inline_word_boxes(
            SourceTextPolicy::HanOnly,
            "AI智能塑形"
        ));
        for text in ["S型曲线", "English only", "Peach\n蜜桃臀", "纯中文"] {
            assert!(!needs_inline_word_boxes(SourceTextPolicy::HanOnly, text));
        }
    }

    #[test]
    fn all_text_never_requests_word_boxes_or_rewrites_ocr_text() {
        let first_pass = vec![output("Peach蜜桃臀")];
        let crops = vec![DynamicImage::ImageRgb8(RgbImage::new(20, 10))];
        let calls = Cell::new(0);

        let updates = dispatch_inline_word_boxes(
            SourceTextPolicy::AllText,
            &first_pass,
            &crops,
            &[[0, 0, 20, 10]],
            20,
            10,
            |_, _| {
                calls.set(calls.get() + 1);
                Ok(valid_word_boxes("Peach", "蜜桃臀"))
            },
        );

        assert_eq!(calls.get(), 0);
        assert_eq!(updates, vec![(false, None)]);
        assert_eq!(first_pass[0].text, "Peach蜜桃臀");
    }

    #[test]
    fn production_word_box_dispatch_calls_inference_only_for_candidates() {
        let first_pass = vec![
            output("English only"),
            output("S型曲线"),
            output("Peach蜜桃臀"),
            output("纯中文"),
        ];
        let crops = vec![DynamicImage::ImageRgb8(RgbImage::new(20, 10)); first_pass.len()];
        let bounds = vec![[0, 0, 20, 10]; first_pass.len()];
        let calls = Cell::new(0);

        let updates = dispatch_inline_word_boxes(
            SourceTextPolicy::HanOnly,
            &first_pass,
            &crops,
            &bounds,
            20,
            10,
            |index, _| {
                calls.set(calls.get() + 1);
                assert_eq!(index, 2);
                Ok(valid_word_boxes("Peach", "蜜桃臀"))
            },
        );

        assert_eq!(calls.get(), 1);
        assert_eq!(updates[0], (false, None));
        assert_eq!(updates[1], (false, None));
        assert!(updates[2].0);
        assert!(updates[2].1.is_some());
        assert_eq!(updates[3], (false, None));
    }

    #[test]
    fn production_word_box_dispatch_marks_failed_candidates_for_geometry_clear() {
        let first_pass = vec![output("Peach蜜桃臀")];
        let crops = vec![DynamicImage::ImageRgb8(RgbImage::new(20, 10))];

        let updates = dispatch_inline_word_boxes(
            SourceTextPolicy::HanOnly,
            &first_pass,
            &crops,
            &[[0, 0, 20, 10]],
            20,
            10,
            |_, _| anyhow::bail!("synthetic inference failure"),
        );

        let (attempted, update) = &updates[0];
        assert!(*attempted);
        assert!(update.is_none());
        assert_eq!(first_pass[0].text, "Peach蜜桃臀");
    }

    #[test]
    fn failed_word_box_candidate_clears_stale_line_polygons() {
        let patch = build_ocr_text_patch("Peach蜜桃臀".to_string(), true, None);

        assert!(matches!(patch.line_polygons, Some(None)));
        assert_eq!(
            patch.text.as_ref().and_then(|text| text.as_deref()),
            Some("Peach蜜桃臀")
        );
    }

    #[test]
    fn production_word_box_dispatch_keeps_original_index_order() {
        let first_pass = vec![
            output("Peach蜜桃臀"),
            output("English"),
            output("AI智能塑形"),
        ];
        let crops = vec![DynamicImage::ImageRgb8(RgbImage::new(20, 10)); first_pass.len()];
        let bounds = vec![[0, 0, 20, 10]; first_pass.len()];

        let updates = dispatch_inline_word_boxes(
            SourceTextPolicy::HanOnly,
            &first_pass,
            &crops,
            &bounds,
            20,
            10,
            |index, _| match index {
                0 => Ok(valid_word_boxes("Peach", "蜜桃臀")),
                2 => Ok(valid_word_boxes("AI", "智能塑形")),
                _ => unreachable!(),
            },
        );

        assert_eq!(updates[0].1.as_ref().unwrap().0, "Peach\n蜜桃臀");
        assert_eq!(updates[1], (false, None));
        assert_eq!(updates[2].1.as_ref().unwrap().0, "AI\n智能塑形");
    }

    #[test]
    fn maps_valid_word_box_units_back_to_absolute_page_polygons() {
        let update = build_pp_ocr_word_box_update(
            "Peach蜜桃臀",
            &[word("Peach", 0.0, 40.0), word("蜜桃臀", 60.0, 100.0)],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();

        assert_eq!(update.0, "Peach\n蜜桃臀");
        assert_eq!(
            update.1,
            vec![
                [[10.0, 20.0], [50.0, 20.0], [50.0, 30.0], [10.0, 30.0]],
                [[70.0, 20.0], [110.0, 20.0], [110.0, 30.0], [70.0, 30.0]],
            ]
        );
    }

    #[test]
    fn rejects_word_boxes_that_does_not_cover_the_first_ocr_text() {
        assert!(
            build_pp_ocr_word_box_update(
                "Peach蜜桃臀!",
                &[word("Peach", 0.0, 40.0), word("蜜桃臀", 60.0, 100.0)],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_word_boxes_that_leaves_english_and_han_in_one_unit() {
        assert!(
            build_pp_ocr_word_box_update(
                "Peach蜜桃臀",
                &[word("Peach蜜桃臀", 0.0, 100.0)],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_word_boxes_with_multiple_han_units_for_one_node_sprite() {
        assert!(
            build_pp_ocr_word_box_update(
                "Peach蜜桃美臀",
                &[
                    word("Peach", 0.0, 30.0),
                    word("蜜桃", 40.0, 60.0),
                    word("美臀", 70.0, 100.0),
                ],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_word_boxes_outside_the_original_crop() {
        assert!(
            build_pp_ocr_word_box_update(
                "Peach蜜桃臀",
                &[word("Peach", -1.0, 40.0), word("蜜桃臀", 60.0, 100.0)],
                [10, 20, 110, 70],
                200,
                100,
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_word_boxes_when_the_english_word_disagrees_with_vl() {
        assert!(
            build_pp_ocr_word_box_update(
                "Peach蜜桃臀",
                &[word("Beach", 0.0, 40.0), word("蜜桃臀", 60.0, 100.0)],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
    }

    #[test]
    fn merges_a_single_latin_label_into_the_han_target() {
        let update = build_pp_ocr_word_box_update(
            "PeachS型曲线",
            &[
                word("Peach", 0.0, 30.0),
                word("S", 35.0, 45.0),
                word("型曲线", 45.0, 80.0),
            ],
            [0, 0, 100, 50],
            100,
            50,
        )
        .unwrap();

        assert_eq!(update.0, "Peach\nS型曲线");
        assert_eq!(
            update.1[1],
            [[35.0, 0.0], [80.0, 0.0], [80.0, 10.0], [35.0, 10.0]]
        );
    }

    #[test]
    fn rejects_overlapping_or_low_confidence_word_boxes() {
        let mut low = word("蜜桃臀", 60.0, 100.0);
        low.confidence = 0.2;
        assert!(
            build_pp_ocr_word_box_update(
                "Peach蜜桃臀",
                &[word("Peach", 0.0, 70.0), word("蜜桃臀", 60.0, 100.0)],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
        assert!(
            build_pp_ocr_word_box_update(
                "Peach蜜桃臀",
                &[word("Peach", 0.0, 40.0), low],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
    }
}
