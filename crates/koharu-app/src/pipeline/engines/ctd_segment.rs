//! Comic Text Detector (segmentation-only). Needs text boxes from another
//! detector; produces a refined `Mask { Segment }` layer.

use anyhow::{Result, bail};
use async_trait::async_trait;
use image::{DynamicImage, GrayImage};
use koharu_core::{MaskRole, Op, PageId, Scene};
use koharu_ml::comic_text_detector::{ComicTextDetector, refine_segmentation_mask};
use koharu_ml::types::TextRegion;

use crate::config::SourceTextPolicy;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    EligibleTextLine, eligible_lines_for_page, intersect_gray_masks, line_support_mask,
    load_source_image, text_node_to_region, text_nodes, upsert_mask_blob,
};

pub struct Model(ComicTextDetector);

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        let image = load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
        let mask = dispatch_segment(
            &image,
            ctx.scene,
            ctx.page,
            ctx.options.source_text_policy,
            |image| self.0.inference_segmentation(image),
        )?;
        let mask_blob = ctx.blobs.put_webp(&DynamicImage::ImageLuma8(mask))?;

        Ok(vec![upsert_mask_blob(
            ctx.scene,
            ctx.page,
            MaskRole::Segment,
            mask_blob,
        )])
    }
}

fn segment_regions(
    scene: &Scene,
    page: PageId,
    policy: SourceTextPolicy,
) -> Result<(Vec<TextRegion>, Vec<EligibleTextLine>)> {
    let nodes = text_nodes(scene, page);
    if nodes.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    if nodes.iter().any(|(_, _, text)| {
        text.text
            .as_deref()
            .is_none_or(|content| content.trim().is_empty())
    }) {
        bail!("OCR text required before segmentation");
    }

    if policy == SourceTextPolicy::AllText {
        let regions = nodes
            .into_iter()
            .map(|(_, transform, text)| text_node_to_region(transform, text))
            .collect();
        return Ok((regions, Vec::new()));
    }

    let eligible_lines = eligible_lines_for_page(scene, page)
        .0
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>();
    let regions = eligible_lines
        .iter()
        .map(|line| line.region.clone())
        .collect();
    Ok((regions, eligible_lines))
}

fn finalize_segment_mask(
    image: &DynamicImage,
    probability: &GrayImage,
    regions: &[TextRegion],
    eligible_lines: &[EligibleTextLine],
    policy: SourceTextPolicy,
) -> GrayImage {
    let refined = refine_segmentation_mask(image, probability, regions);
    if policy == SourceTextPolicy::HanOnly {
        let allowed = line_support_mask(refined.width(), refined.height(), eligible_lines);
        intersect_gray_masks(&refined, &allowed)
    } else {
        refined
    }
}

fn dispatch_segment<Inference>(
    image: &DynamicImage,
    scene: &Scene,
    page: PageId,
    policy: SourceTextPolicy,
    inference: Inference,
) -> Result<GrayImage>
where
    Inference: FnOnce(&DynamicImage) -> Result<GrayImage>,
{
    let (regions, eligible_lines) = segment_regions(scene, page, policy)?;
    if regions.is_empty() {
        return Ok(GrayImage::new(image.width(), image.height()));
    }
    let probability = inference(image)?;
    Ok(finalize_segment_mask(
        image,
        &probability,
        &regions,
        &eligible_lines,
        policy,
    ))
}

inventory::submit! {
    EngineInfo {
        id: "comic-text-detector-seg",
        name: "Comic Text Detector (Segmentation)",
        needs: &[Artifact::OcrText],
        produces: &[Artifact::SegmentMask],
        load: |runtime, cpu| Box::pin(async move {
            let m = ComicTextDetector::load_segmentation_only(runtime, cpu).await?;
            Ok(Box::new(Model(m)) as Box<dyn Engine>)
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use image::{GrayImage, Luma, Rgb, RgbImage};
    use koharu_core::{Node, NodeKind, Page, PageId, Scene, TextData, TextDirection, Transform};

    use super::*;
    use crate::config::SourceTextPolicy;

    fn page_id() -> PageId {
        PageId::new()
    }

    fn scene_with_texts(texts: Vec<(Transform, TextData)>) -> (Scene, PageId) {
        let page_id = page_id();
        let mut page = Page::new("page", 32, 16);
        page.id = page_id;
        for (transform, text) in texts {
            let node = Node {
                id: koharu_core::NodeId::new(),
                transform,
                visible: true,
                kind: NodeKind::Text(text),
            };
            page.nodes.insert(node.id, node);
        }
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        (scene, page_id)
    }

    fn transform() -> Transform {
        Transform {
            x: 0.0,
            y: 0.0,
            width: 32.0,
            height: 16.0,
            rotation_deg: 0.0,
        }
    }

    fn text(text: Option<&str>, polygons: Option<Vec<[[f32; 2]; 4]>>) -> TextData {
        TextData {
            text: text.map(str::to_string),
            line_polygons: polygons,
            source_direction: Some(TextDirection::Horizontal),
            ..Default::default()
        }
    }

    #[test]
    fn segment_dispatch_zero_text_is_noop_before_inference() {
        let (scene, page) = scene_with_texts(vec![]);
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
        let calls = Cell::new(0);

        let mask = dispatch_segment(&image, &scene, page, SourceTextPolicy::HanOnly, |_| {
            calls.set(calls.get() + 1);
            Ok(GrayImage::from_pixel(32, 16, Luma([255])))
        })
        .unwrap();

        assert_eq!(calls.get(), 0);
        assert!(mask.pixels().all(|pixel| pixel.0[0] == 0));
    }

    #[test]
    fn segment_dispatch_requires_ocr_before_inference() {
        let (scene, page) = scene_with_texts(vec![(transform(), text(None, None))]);
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
        let calls = Cell::new(0);

        let error = dispatch_segment(&image, &scene, page, SourceTextPolicy::HanOnly, |_| {
            calls.set(calls.get() + 1);
            Ok(GrayImage::new(32, 16))
        })
        .unwrap_err();

        assert_eq!(calls.get(), 0);
        assert!(error.to_string().contains("OCR text required"));
    }

    #[test]
    fn segment_dispatch_skips_english_and_unsupported_before_inference() {
        let cases = [
            text(Some("English"), None),
            text(Some("English\n中文"), None),
        ];
        for text in cases {
            let (scene, page) = scene_with_texts(vec![(transform(), text)]);
            let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
            let calls = Cell::new(0);

            let mask = dispatch_segment(&image, &scene, page, SourceTextPolicy::HanOnly, |_| {
                calls.set(calls.get() + 1);
                Ok(GrayImage::from_pixel(32, 16, Luma([255])))
            })
            .unwrap();

            assert_eq!(calls.get(), 0);
            assert!(mask.pixels().all(|pixel| pixel.0[0] == 0));
        }
    }

    #[test]
    fn segment_dispatch_finalizes_to_han_line_support() {
        let quad = |x1, y1, x2, y2| [[x1, y1], [x2, y1], [x2, y2], [x1, y2]];
        let (scene, page) = scene_with_texts(vec![(
            transform(),
            text(
                Some("English\n中文"),
                Some(vec![quad(2.0, 1.0, 30.0, 7.0), quad(2.0, 9.0, 30.0, 15.0)]),
            ),
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
        let calls = Cell::new(0);

        let mask = dispatch_segment(&image, &scene, page, SourceTextPolicy::HanOnly, |_| {
            calls.set(calls.get() + 1);
            Ok(GrayImage::from_pixel(32, 16, Luma([255])))
        })
        .unwrap();

        assert_eq!(calls.get(), 1);
        assert_eq!(mask.get_pixel(10, 4).0[0], 0);
        assert_ne!(mask.get_pixel(10, 11).0[0], 0);
        assert_eq!(mask.get_pixel(31, 11).0[0], 0);
    }
}
