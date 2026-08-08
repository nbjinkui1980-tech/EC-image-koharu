//! Speech-bubble segmentation. Produces a `Mask { Bubble }` layer where each
//! detected balloon contour is encoded as a distinct non-zero grayscale ID and
//! everything else is `0`. The renderer consumes this ID mask for safe-area
//! layout, placement, and glyph collision checks against the real bubble shape.

use anyhow::Result;
use async_trait::async_trait;
use image::{DynamicImage, GrayImage, Luma};
use koharu_core::{MaskRole, Op};
use koharu_ml::speech_bubble_segmentation::{
    SpeechBubbleRegion, SpeechBubbleSegmentation, SpeechBubbleSegmentationResult,
};

use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{load_source_image, upsert_mask_blob};

pub struct Model(SpeechBubbleSegmentation);

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        crate::pipeline::engine::emit_engine_device("speech-bubble-segmentation", "speech-bubble-segmentation", 0);
        let image = load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
        let result = self.0.inference(&image)?;
        let mask = bubble_mask_from_result(&result);

        let blob = ctx.blobs.put_webp(&DynamicImage::ImageLuma8(mask))?;
        Ok(vec![upsert_mask_blob(
            ctx.scene,
            ctx.page,
            MaskRole::Bubble,
            blob,
        )])
    }
}

pub(in crate::pipeline) fn bubble_mask_from_result(
    result: &SpeechBubbleSegmentationResult,
) -> GrayImage {
    let mut regions: Vec<_> = result.regions.iter().collect();
    regions.sort_by_key(|region| std::cmp::Reverse(region.area));
    paint_bubble_id_mask(result.image_width, result.image_height, &regions)
}

fn paint_bubble_id_mask(width: u32, height: u32, regions: &[&SpeechBubbleRegion]) -> GrayImage {
    let mut mask = GrayImage::from_pixel(width, height, Luma([0u8]));

    // Cap at 255 IDs; typical manga pages have well under 20 bubbles.
    // Larger bubbles are painted first so smaller overlapping contours keep
    // their own ID when painted later.
    for (i, region) in regions.iter().take(255).enumerate() {
        if region.mask.is_empty() {
            continue;
        }
        let id = (i + 1) as u8;
        let src_width = region.mask.width as usize;
        let max_x = region.mask.width.min(width.saturating_sub(region.mask.x));
        let max_y = region.mask.height.min(height.saturating_sub(region.mask.y));
        for local_y in 0..max_y {
            let src_row = local_y as usize * src_width;
            let y = region.mask.y + local_y;
            for local_x in 0..max_x {
                if region.mask.pixels[src_row + local_x as usize] == 0 {
                    continue;
                }
                mask.put_pixel(region.mask.x + local_x, y, Luma([id]));
            }
        }
    }

    mask
}

inventory::submit! {
    EngineInfo {
        id: "speech-bubble-segmentation",
        name: "Speech Bubble Segmentation",
        needs: &[Artifact::TextBoxes, Artifact::SourceTextBoxes],
        produces: &[Artifact::BubbleMask],
        load: |runtime, cpu| Box::pin(async move {
            let m = SpeechBubbleSegmentation::load(runtime, cpu).await?;
            Ok(Box::new(Model(m)) as Box<dyn Engine>)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_ml::probability_map::ProbabilityMap;
    use koharu_ml::speech_bubble_segmentation::{SpeechBubbleRegion, SpeechBubbleRegionMask};

    #[test]
    fn id_mask_paints_region_contours_not_bboxes() {
        let region = SpeechBubbleRegion {
            label_id: 0,
            label: "bubble".to_string(),
            score: 0.9,
            bbox: [1.0, 1.0, 4.0, 4.0],
            area: 3,
            mask: SpeechBubbleRegionMask {
                x: 1,
                y: 1,
                width: 3,
                height: 3,
                pixels: vec![0, 255, 0, 255, 255, 0, 0, 0, 0],
            },
        };

        let mask = paint_bubble_id_mask(6, 6, &[&region]);

        assert_eq!(mask.get_pixel(1, 1).0[0], 0);
        assert_eq!(mask.get_pixel(2, 1).0[0], 1);
        assert_eq!(mask.get_pixel(1, 2).0[0], 1);
        assert_eq!(mask.get_pixel(2, 2).0[0], 1);
        assert_eq!(mask.get_pixel(3, 2).0[0], 0);
    }

    #[test]
    fn result_mask_uses_production_area_order() {
        let region = |area| SpeechBubbleRegion {
            label_id: 0,
            label: "bubble".to_string(),
            score: 0.9,
            bbox: [1.0, 1.0, 3.0, 3.0],
            area,
            mask: SpeechBubbleRegionMask {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
                pixels: vec![255; 4],
            },
        };
        let result = SpeechBubbleSegmentationResult {
            image_width: 4,
            image_height: 4,
            regions: vec![region(1), region(4)],
            probability_map: ProbabilityMap::zeros(4, 4),
        };

        let mask = bubble_mask_from_result(&result);

        assert_eq!(mask.get_pixel(1, 1).0[0], 2);
    }
}
