//! AOT inpainting. Direct source + segment → result. Subdivision is handled
//! by [`koharu_ml::inpainting::run_inpaint`] (shared with Lama) — this engine
//! only wires up the scene I/O.
//!
//! For repair-brush (`ctx.options.region`), composite onto the existing
//! `Image { Inpainted }` if present (fallback Source) and zero out mask
//! pixels outside the region so only that area is reprocessed.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use image::DynamicImage;
use koharu_core::{ImageRole, MaskRole, Op, Region};
use koharu_ml::aot_inpainting::AotInpainting;
use koharu_ml::inpainting::expand_mask_for_inpainting;
use koharu_ml::types::TextRegion;

use crate::config::SourceTextPolicy;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    EligibleTextLine, clip_mask_to_region, eligible_lines_for_page, find_image_node,
    find_mask_node, image_dimensions, load_source_image, prepare_inpaint_mask, text_node_to_region,
    text_nodes, upsert_image_blob,
};

pub struct Model(AotInpainting);

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
        let text_blocks: Vec<TextRegion> = text_nodes(ctx.scene, ctx.page)
            .into_iter()
            .map(|(_, transform, text)| text_node_to_region(transform, text))
            .collect();
        let eligible_lines = eligible_lines_for_page(ctx.scene, ctx.page)
            .0
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>();
        let result = dispatch_aot_inpaint(
            &image,
            &mask,
            &bubble_mask,
            &text_blocks,
            &eligible_lines,
            ctx.options.source_text_policy,
            ctx.options.region,
            |image, mask, bubble_mask| self.0.inference(image, mask, bubble_mask),
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
fn dispatch_aot_inpaint<Inference>(
    image: &DynamicImage,
    mask: &DynamicImage,
    bubble_mask: &DynamicImage,
    all_blocks: &[TextRegion],
    eligible_lines: &[EligibleTextLine],
    policy: SourceTextPolicy,
    region: Option<Region>,
    inference: Inference,
) -> Result<DynamicImage>
where
    Inference: FnOnce(&DynamicImage, &DynamicImage, &DynamicImage) -> Result<DynamicImage>,
{
    let inference_bubble = region
        .map(|region| clip_mask_to_region(bubble_mask, &region))
        .unwrap_or_else(|| bubble_mask.clone());
    let Some((final_mask, _)) = prepare_inpaint_mask(
        mask,
        bubble_mask,
        all_blocks,
        eligible_lines,
        policy,
        region,
        expand_mask_for_inpainting,
    ) else {
        return Ok(image.clone());
    };
    inference(image, &final_mask, &inference_bubble)
}

inventory::submit! {
    EngineInfo {
        id: "aot-inpainting",
        name: "AOT Inpainting",
        needs: &[
            Artifact::SegmentMask,
            Artifact::BubbleMask,
            Artifact::Translations,
            Artifact::SourceTextBoxes,
        ],
        produces: &[Artifact::Inpainted],
        load: |runtime, cpu| Box::pin(async move {
            let m = AotInpainting::load(runtime, cpu).await?;
            Ok(Box::new(Model(m)) as Box<dyn Engine>)
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use image::{GrayImage, Luma, Rgb, RgbImage};
    use koharu_core::Region;
    use koharu_ml::inpainting::{HdStrategy, HdStrategyConfig, InpaintForward, run_inpaint};

    use super::*;
    use crate::config::SourceTextPolicy;

    struct PaintForward;

    impl InpaintForward for PaintForward {
        fn forward(
            &self,
            image: &RgbImage,
            _mask: &GrayImage,
            _bubble_mask: Option<&GrayImage>,
        ) -> Result<RgbImage> {
            Ok(RgbImage::from_pixel(
                image.width(),
                image.height(),
                Rgb([240, 8, 16]),
            ))
        }
    }

    #[test]
    fn aot_inpaint_dispatch_receives_final_mask_and_preserves_repair_region() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(16, 12, Rgb([1, 2, 3])));
        let mask = DynamicImage::ImageLuma8(GrayImage::from_fn(16, 12, |x, y| {
            Luma([if x == 5 && y == 6 { 255 } else { 0 }])
        }));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(16, 12));
        let region = Region {
            x: 3,
            y: 4,
            width: 5,
            height: 4,
        };
        let calls = Cell::new(0);

        let result = dispatch_aot_inpaint(
            &image,
            &mask,
            &bubble,
            &[TextRegion {
                x: 0.0,
                y: 0.0,
                width: 16.0,
                height: 12.0,
                ..Default::default()
            }],
            &[],
            SourceTextPolicy::HanOnly,
            Some(region),
            |frame, final_mask, final_bubble| {
                calls.set(calls.get() + 1);
                let final_mask = final_mask.to_luma8();
                assert_eq!(final_mask.get_pixel(2, 6).0[0], 0);
                assert_ne!(final_mask.get_pixel(5, 6).0[0], 0);
                let cfg = HdStrategyConfig {
                    strategy: HdStrategy::Original,
                    ..HdStrategyConfig::lama_default()
                };
                let output = run_inpaint(
                    &PaintForward,
                    &frame.to_rgb8(),
                    &final_mask,
                    Some(&final_bubble.to_luma8()),
                    &cfg,
                )?;
                Ok(DynamicImage::ImageRgb8(output))
            },
        )
        .unwrap();

        assert_eq!(calls.get(), 1);
        assert_eq!(result.to_rgb8().get_pixel(0, 0).0, [1, 2, 3]);
        assert_eq!(result.to_rgb8().get_pixel(5, 6).0, [240, 8, 16]);
        assert_eq!(result.to_rgb8().get_pixel(8, 6).0, [1, 2, 3]);
    }

    #[test]
    fn aot_inpaint_dispatch_skips_empty_han_targets() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(16, 12, Rgb([1, 2, 3])));
        let mask = DynamicImage::ImageLuma8(GrayImage::from_pixel(16, 12, Luma([255])));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(16, 12));
        let calls = Cell::new(0);

        let output = dispatch_aot_inpaint(
            &image,
            &mask,
            &bubble,
            &[],
            &[],
            SourceTextPolicy::HanOnly,
            None,
            |frame, _, _| {
                calls.set(calls.get() + 1);
                Ok(frame.clone())
            },
        )
        .unwrap();

        assert_eq!(calls.get(), 0);
        assert_eq!(output.to_rgb8(), image.to_rgb8());
    }
}
