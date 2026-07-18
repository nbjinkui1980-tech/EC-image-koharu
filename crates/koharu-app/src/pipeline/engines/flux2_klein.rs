//! Flux.2 Klein inpainter. Uses the CTD segment mask to build a looser
//! text-region mask, then runs Flux.2 inpainting on the resulting crop.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use image::DynamicImage;
use koharu_core::{ImageRole, MaskRole, Op, Region};
use koharu_ml::flux2_klein::{Flux2InpaintOptions, Flux2Klein};
use koharu_ml::inpainting::mask::expand_mask_to_bubble_region_for_inpainting;
use koharu_ml::types::TextRegion;

use crate::config::SourceTextPolicy;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    EligibleTextLine, clip_mask_to_region, eligible_lines_for_page, find_image_node,
    find_mask_node, image_dimensions, load_source_image, prepare_inpaint_mask, text_node_to_region,
    text_nodes, upsert_image_blob,
};

pub struct Model(Flux2Klein);

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
        let eligible_lines = eligible_lines_for_page(ctx.scene, ctx.page)
            .0
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>();
        let result = dispatch_flux2_inpaint(
            &image,
            &mask,
            &bubble_mask,
            &text_blocks,
            &eligible_lines,
            ctx.options.source_text_policy,
            ctx.options.region,
            |image, mask, _| {
                let options = Flux2InpaintOptions {
                    mask_padding: 0,
                    ..Default::default()
                };
                self.0.inpaint_with_reference(image, mask, None, &options)
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
fn dispatch_flux2_inpaint<Inference>(
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
        expand_mask_to_bubble_region_for_inpainting,
    ) else {
        return Ok(image.clone());
    };
    inference(image, &final_mask, &inference_bubble)
}

inventory::submit! {
    EngineInfo {
        id: "flux2-klein",
        name: "Flux.2 Klein",
        needs: &[
            Artifact::SegmentMask,
            Artifact::BubbleMask,
            Artifact::Translations,
            Artifact::SourceTextBoxes,
        ],
        produces: &[Artifact::Inpainted],
        load: |runtime, _cpu| Box::pin(async move {
            let m = Flux2Klein::load(runtime).await?;
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

    #[test]
    fn flux2_inpaint_dispatch_receives_final_mask() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
        let mask = DynamicImage::ImageLuma8(GrayImage::from_fn(32, 16, |x, y| {
            Luma([if (x == 5 || x == 20) && y == 8 {
                255
            } else {
                0
            }])
        }));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(32, 16));
        let eligible = EligibleTextLine {
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
        };
        let calls = Cell::new(0);

        let result = dispatch_flux2_inpaint(
            &image,
            &mask,
            &bubble,
            &[],
            &[eligible],
            SourceTextPolicy::HanOnly,
            None,
            |frame, final_mask, _| {
                calls.set(calls.get() + 1);
                let final_mask = final_mask.to_luma8();
                assert_eq!(final_mask.get_pixel(5, 8).0[0], 0);
                assert_ne!(final_mask.get_pixel(20, 8).0[0], 0);
                Ok(frame.clone())
            },
        )
        .unwrap();

        assert_eq!(calls.get(), 1);
        assert_eq!(result.to_rgb8(), image.to_rgb8());
    }

    #[test]
    fn flux2_inpaint_dispatch_skips_empty_han_targets() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(16, 12, Rgb([1, 2, 3])));
        let mask = DynamicImage::ImageLuma8(GrayImage::from_pixel(16, 12, Luma([255])));
        let bubble = DynamicImage::ImageLuma8(GrayImage::new(16, 12));
        let calls = Cell::new(0);

        let output = dispatch_flux2_inpaint(
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
