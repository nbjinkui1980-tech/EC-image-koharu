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
use koharu_core::{ImageRole, MaskRole, Op};
use koharu_ml::inpainting::expand_mask_for_inpainting;
use koharu_ml::lama::Lama;

use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    clip_mask_to_region, find_image_node, find_mask_node, image_dimensions, load_source_image,
    text_node_to_region, text_nodes, upsert_image_blob,
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

        let (image, mask, bubble_mask) = match ctx.options.region {
            Some(r) => {
                let base = match find_image_node(ctx.scene, ctx.page, ImageRole::Inpainted) {
                    Some((_, blob)) => ctx.blobs.load_image(&blob)?,
                    None => load_source_image(ctx.scene, ctx.page, ctx.blobs)?,
                };
                let clipped_mask = clip_mask_to_region(&mask, &r);
                let clipped_bubble = clip_mask_to_region(&bubble_mask, &r);
                (base, clipped_mask, clipped_bubble)
            }
            None => {
                let image = load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
                (image, mask, bubble_mask)
            }
        };

        let text_blocks = text_nodes(ctx.scene, ctx.page)
            .into_iter()
            .map(|(_, transform, text)| text_node_to_region(transform, text))
            .collect::<Vec<_>>();
        let expanded = expand_mask_for_inpainting(&mask, &bubble_mask, &text_blocks);
        let mask = match ctx.options.region {
            Some(r) => clip_mask_to_region(&DynamicImage::ImageLuma8(expanded), &r),
            None => DynamicImage::ImageLuma8(expanded),
        };

        let result = if text_blocks.is_empty() {
            self.0.inference(&image, &mask, &bubble_mask)?
        } else {
            self.0
                .inference_with_blocks(&image, &mask, &bubble_mask, &text_blocks)?
        };
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

inventory::submit! {
    EngineInfo {
        id: "lama-manga",
        name: "Lama Manga",
        needs: &[Artifact::SegmentMask, Artifact::BubbleMask],
        produces: &[Artifact::Inpainted],
        load: |runtime, cpu| Box::pin(async move {
            let m = Lama::load(runtime, cpu).await?;
            Ok(Box::new(Model(m)) as Box<dyn Engine>)
        }),
    }
}
