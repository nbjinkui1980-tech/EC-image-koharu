//! Lama Manga inpainter. Reads source + segmentation mask from the page,
//! runs the model, writes the output as `Image { role: Inpainted }`.
//!
//! Box subdivision (the "which regions to run the model on" question) is
//! driven by the **mask itself** via `boxes_from_mask` — mirrors IOPaint's
//! `InpaintModel.__call__`. Text detections are no longer consulted; the
//! segmentation mask already encodes which pixels to remove.
//!
//! When `ctx.options.region` is set (repair-brush re-inpaint), we composite
//! onto the existing `Image { Inpainted }` if present (falling back to
//! `Source`) and zero out mask pixels outside the region before dispatch —
//! so only that region is reprocessed.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use image::DynamicImage;
use koharu_core::{ImageRole, MaskRole, NodeId, Op, Region};
use koharu_ml::inpainting::expand_mask_for_inpainting;
use koharu_ml::lama::Lama;
use koharu_ml::types::TextRegion;

use crate::config::SourceTextPolicy;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    EligibleTextLine, PreparedInpaintMask, clip_mask_to_region, eligible_lines_for_page,
    find_image_node, find_mask_node, image_dimensions, load_source_image, prepare_inpaint_mask,
    protected_source_lines_for_page, text_node_to_region, text_nodes, upsert_image_blob,
};

pub struct Model(Lama);

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        let (_, mask_ref) = find_mask_node(ctx.scene, ctx.page, MaskRole::Segment)
            .ok_or_else(|| anyhow!("no Segment mask on page"))?;
        let (_, bubble_ref) = find_mask_node(ctx.scene, ctx.page, MaskRole::Bubble)
            .ok_or_else(|| anyhow!("no Bubble mask on page"))?;
        let mask = ctx.blobs.load_image(&mask_ref)?;
        let bubble_mask = ctx.blobs.load_image(&bubble_ref)?;

        let image = match ctx.options.region {
            Some(_) => match find_image_node(ctx.scene, ctx.page, ImageRole::Inpainted) {
                Some((_, blob)) => ctx.blobs.load_image(&blob)?,
                None => load_source_image(ctx.scene, ctx.page, ctx.blobs)?,
            },
            None => load_source_image(ctx.scene, ctx.page, ctx.blobs)?,
        };

        let text_blocks = text_nodes(ctx.scene, ctx.page)
            .into_iter()
            .map(|(_, transform, text)| text_node_to_region(transform, text))
            .collect::<Vec<_>>();
        let eligible_lines = eligible_lines_for_page(ctx.scene, ctx.page).0;
        let protected_lines = protected_source_lines_for_page(ctx.scene, ctx.page);
        let result = dispatch_lama_inpaint(
            &image,
            &mask,
            &bubble_mask,
            &text_blocks,
            &eligible_lines,
            &protected_lines,
            ctx.options.source_text_policy,
            ctx.options.region,
            |image, mask, bubble_mask, blocks| {
                if blocks.is_empty() {
                    self.0.inference(image, mask, bubble_mask)
                } else {
                    self.0
                        .inference_with_blocks(image, mask, bubble_mask, blocks)
                }
            },
        )?;
        let (w, h) = image_dimensions(&result);
        let blob = ctx.blobs.put_webp(&result)?;
        Ok(vec![upsert_image_blob(
            ctx.scene,
            ctx.page,
            ImageRole::Inpainted,
            blob,
            w,
            h,
        )])
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_lama_inpaint<Inference>(
    image: &DynamicImage,
    mask: &DynamicImage,
    bubble_mask: &DynamicImage,
    all_blocks: &[TextRegion],
    eligible_lines: &[(NodeId, EligibleTextLine)],
    protected_lines: &[(NodeId, EligibleTextLine)],
    policy: SourceTextPolicy,
    region: Option<Region>,
    inference: Inference,
) -> Result<DynamicImage>
where
    Inference:
        FnOnce(&DynamicImage, &DynamicImage, &DynamicImage, &[TextRegion]) -> Result<DynamicImage>,
{
    let inference_bubble = region
        .map(|region| clip_mask_to_region(bubble_mask, &region))
        .unwrap_or_else(|| bubble_mask.clone());
    let prepared = prepare_inpaint_mask(
        mask,
        bubble_mask,
        all_blocks,
        eligible_lines,
        protected_lines,
        policy,
        region,
        expand_mask_for_inpainting,
    )?;
    match prepared {
        PreparedInpaintMask::Prepared { mask, blocks } => {
            inference(image, &mask, &inference_bubble, &blocks)
        }
        PreparedInpaintMask::NoEligibleHanTargets | PreparedInpaintMask::EmptyMask => {
            Ok(image.clone())
        }
    }
}

inventory::submit! {
    EngineInfo {
        id: "lama-manga",
        name: "Lama Manga",
        needs: &[
            Artifact::SegmentMask,
            Artifact::BubbleMask,
            Artifact::Translations,
            Artifact::SourceTextBoxes,
        ],
        produces: &[Artifact::Inpainted],
        load: |runtime, cpu| Box::pin(async move {
            let m = Lama::load(runtime, cpu).await?;
            Ok(Box::new(Model(m)) as Box<dyn Engine>)
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use image::{GrayImage, Luma, Rgb, RgbImage};
    use koharu_ml::types::TextRegion;

    use super::*;
    use crate::{config::SourceTextPolicy, pipeline::engines::support::EligibleTextLine};

    fn eligible_line() -> EligibleTextLine {
        EligibleTextLine {
            line_index: 1,
            text: "中文".to_string(),
            region: TextRegion {
                x: 18.0,
                y: 6.0,
                width: 5.0,
                height: 5.0,
                line_polygons: Some(vec![[[18.0, 6.0], [23.0, 6.0], [23.0, 11.0], [18.0, 11.0]]]),
                ..Default::default()
            },
        }
    }

    #[test]
    fn lama_inpaint_dispatch_receives_final_mask() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
        let mask = DynamicImage::ImageLuma8(GrayImage::from_fn(32, 16, |x, y| {
            Luma([if (x == 5 || x == 20) && y == 8 {
                255
            } else {
                0
            }])
        }));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(32, 16));
        let calls = Cell::new(0);

        let result = dispatch_lama_inpaint(
            &image,
            &mask,
            &bubble,
            &[],
            &[(NodeId::new(), eligible_line())],
            &[],
            SourceTextPolicy::HanOnly,
            None,
            |frame, final_mask, _, blocks| {
                calls.set(calls.get() + 1);
                let final_mask = final_mask.to_luma8();
                assert_eq!(final_mask.get_pixel(5, 8).0[0], 0);
                assert_ne!(final_mask.get_pixel(20, 8).0[0], 0);
                assert!(final_mask.enumerate_pixels().all(|(x, y, pixel)| {
                    pixel.0[0] == 0 || ((18..23).contains(&x) && (6..11).contains(&y))
                }));
                assert_eq!(blocks.len(), 1);
                Ok(frame.clone())
            },
        )
        .unwrap();

        assert_eq!(calls.get(), 1);
        assert_eq!(result.to_rgb8(), image.to_rgb8());

        let empty_calls = Cell::new(0);
        let empty = dispatch_lama_inpaint(
            &image,
            &mask,
            &bubble,
            &[],
            &[],
            &[],
            SourceTextPolicy::HanOnly,
            None,
            |frame, _, _, _| {
                empty_calls.set(empty_calls.get() + 1);
                Ok(frame.clone())
            },
        )
        .unwrap();
        assert_eq!(empty_calls.get(), 0);
        assert_eq!(empty.to_rgb8(), image.to_rgb8());
    }
}
