//! Comic Text Detector (segmentation-only). Needs text boxes from another
//! detector; produces a refined `Mask { Segment }` layer.

use anyhow::{Result, bail};
use async_trait::async_trait;
use image::{DynamicImage, GrayImage};
use koharu_core::{MaskRole, NodeId, Op, PageId, Scene};
use koharu_ml::comic_text_detector::{
    ComicTextDetector, refine_segmentation_candidate_mask, refine_segmentation_mask,
};
use koharu_ml::types::TextRegion;

use crate::config::SourceTextPolicy;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    EligibleTextLine, canonical_han_mask, eligible_lines_for_page, forbidden_han_lines_for_page,
    load_source_image, text_node_to_region, text_nodes, upsert_mask_blob,
};
#[cfg(test)]
use crate::pipeline::engines::support::{
    EraseDiagnosticBranch, EraseDiagnosticStage, record_erase_diagnostic,
};

pub struct Model(ComicTextDetector);

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        crate::pipeline::engine::emit_engine_device("comic-text-detector-seg", "comic-text-detector-seg", 0);
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
) -> Result<(
    Vec<TextRegion>,
    Vec<(NodeId, EligibleTextLine)>,
    Vec<(NodeId, EligibleTextLine)>,
)> {
    let nodes = text_nodes(scene, page);
    if nodes.is_empty() {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    }
    if nodes.iter().any(|(_, _, text)| {
        text.text
            .as_deref()
            .is_none_or(|content| content.trim().is_empty())
    }) {
        bail!("ocr text required before segmentation");
    }

    if policy == SourceTextPolicy::AllText {
        let regions = nodes
            .into_iter()
            .map(|(_, transform, text)| text_node_to_region(transform, text))
            .collect();
        return Ok((regions, Vec::new(), Vec::new()));
    }

    let eligible_lines = eligible_lines_for_page(scene, page).0;
    let regions = eligible_lines
        .iter()
        .map(|(_, line)| line.region.clone())
        .collect();
    let protected_lines = forbidden_han_lines_for_page(scene, page);
    Ok((regions, eligible_lines, protected_lines))
}

fn finalize_segment_mask(
    image: &DynamicImage,
    probability: &GrayImage,
    regions: &[TextRegion],
    eligible_lines: &[(NodeId, EligibleTextLine)],
    protected_lines: &[(NodeId, EligibleTextLine)],
    policy: SourceTextPolicy,
) -> Result<GrayImage> {
    #[cfg(test)]
    let diagnostic_branch = if policy == SourceTextPolicy::HanOnly {
        EraseDiagnosticBranch::HanOnly
    } else {
        EraseDiagnosticBranch::AllText
    };
    #[cfg(test)]
    record_erase_diagnostic(
        EraseDiagnosticStage::SegmentProbability,
        diagnostic_branch,
        Some(probability),
        None,
    );
    let refined = if policy == SourceTextPolicy::HanOnly {
        refine_segmentation_candidate_mask(probability, regions)
    } else {
        refine_segmentation_mask(image, probability, regions)
    };
    #[cfg(test)]
    record_erase_diagnostic(
        EraseDiagnosticStage::SegmentRefined,
        diagnostic_branch,
        Some(&refined),
        None,
    );
    let final_mask = if policy == SourceTextPolicy::HanOnly {
        let (retained, allowed) = canonical_han_mask(&refined, eligible_lines, protected_lines)?;
        #[cfg(not(test))]
        let _ = allowed;
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::SegmentAllowedSupport,
            diagnostic_branch,
            Some(&allowed),
            None,
        );
        retained
    } else {
        #[cfg(test)]
        record_erase_diagnostic(
            EraseDiagnosticStage::SegmentAllowedSupport,
            diagnostic_branch,
            None,
            None,
        );
        refined
    };
    #[cfg(test)]
    record_erase_diagnostic(
        EraseDiagnosticStage::SegmentFinal,
        diagnostic_branch,
        Some(&final_mask),
        None,
    );
    Ok(final_mask)
}

pub(in crate::pipeline) fn dispatch_segment<Inference>(
    image: &DynamicImage,
    scene: &Scene,
    page: PageId,
    policy: SourceTextPolicy,
    inference: Inference,
) -> Result<GrayImage>
where
    Inference: FnOnce(&DynamicImage) -> Result<GrayImage>,
{
    let (regions, eligible_lines, protected_lines) = segment_regions(scene, page, policy)?;
    if regions.is_empty() {
        return Ok(GrayImage::new(image.width(), image.height()));
    }
    let probability = inference(image)?;
    finalize_segment_mask(
        image,
        &probability,
        &regions,
        &eligible_lines,
        &protected_lines,
        policy,
    )
}

inventory::submit! {
    EngineInfo {
        id: "comic-text-detector-seg",
        name: "Comic Text Detector (Segmentation)",
        needs: &[Artifact::OcrText, Artifact::SourceTextBoxes],
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
    use crate::pipeline::engines::support::{EraseDiagnosticCapture, EraseDiagnosticCaptureActive};

    fn start_erase_capture() -> EraseDiagnosticCapture {
        loop {
            match EraseDiagnosticCapture::start() {
                Ok(capture) => return capture,
                Err(EraseDiagnosticCaptureActive) => std::thread::yield_now(),
            }
        }
    }

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

    fn owned_line(node_id: NodeId, bounds: [f32; 4]) -> (NodeId, EligibleTextLine) {
        (
            node_id,
            EligibleTextLine {
                line_index: 0,
                text: "中文".into(),
                region: TextRegion {
                    x: bounds[0],
                    y: bounds[1],
                    width: bounds[2] - bounds[0],
                    height: bounds[3] - bounds[1],
                    detected_font_size_px: Some(bounds[3] - bounds[1]),
                    ..Default::default()
                },
            },
        )
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
        assert!(error.to_string().contains("ocr text required"));
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
    fn segment_dispatch_word_box_inline_mixed_keeps_english_roi_zero() {
        let quad = |x1, y1, x2, y2| [[x1, y1], [x2, y1], [x2, y2], [x1, y2]];
        let (scene, page) = scene_with_texts(vec![(
            transform(),
            text(
                Some("Peach\n蜜桃臀"),
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
        assert_ne!(mask.get_pixel(31, 11).0[0], 0);
    }

    #[test]
    fn han_refinement_keeps_complete_owned_component_outside_region_without_filling_box() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
        let owner = owned_line(NodeId::new(), [10.0, 6.0, 14.0, 10.0]);
        let probability = GrayImage::from_fn(32, 16, |x, y| {
            Luma([if y == 8 && (7..18).contains(&x) {
                255
            } else {
                0
            }])
        });

        let mask = finalize_segment_mask(
            &image,
            &probability,
            std::slice::from_ref(&owner.1.region),
            std::slice::from_ref(&owner),
            &[],
            SourceTextPolicy::HanOnly,
        )
        .unwrap();

        assert_ne!(mask.get_pixel(5, 8).0[0], 0);
        assert_ne!(mask.get_pixel(19, 8).0[0], 0);
        assert_eq!(mask.get_pixel(10, 5).0[0], 0);
    }

    #[test]
    fn han_refinement_drops_unowned_noise_and_rejects_multi_owner_components() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
        let first = owned_line(NodeId::new(), [10.0, 6.0, 13.0, 10.0]);
        let second = owned_line(NodeId::new(), [16.0, 6.0, 19.0, 10.0]);
        let unowned = GrayImage::from_fn(32, 16, |x, y| {
            Luma([u8::from(y == 8 && (1..4).contains(&x)) * 255])
        });
        assert!(
            finalize_segment_mask(
                &image,
                &unowned,
                std::slice::from_ref(&first.1.region),
                std::slice::from_ref(&first),
                &[],
                SourceTextPolicy::HanOnly,
            )
            .unwrap_err()
            .to_string()
            .contains("eligible target has no allowed ink")
        );

        let bridged = GrayImage::from_fn(32, 16, |x, y| {
            Luma([u8::from(y == 8 && (11..18).contains(&x)) * 255])
        });
        assert!(
            finalize_segment_mask(
                &image,
                &bridged,
                &[first.1.region.clone(), second.1.region.clone()],
                &[first, second],
                &[],
                SourceTextPolicy::HanOnly,
            )
            .unwrap_err()
            .to_string()
            .contains("multiple eligible owners")
        );
    }

    #[test]
    fn ctd_segment_erase_diagnostics_lock_order_and_owned_component_support() {
        fn signature(
            event: &crate::pipeline::engines::support::EraseDiagnosticEvent,
        ) -> Option<(u32, u32, u64, &str)> {
            event.mask.as_ref().map(|mask| {
                (
                    mask.width,
                    mask.height,
                    mask.nonzero_pixels,
                    mask.grayscale_blake3.as_str(),
                )
            })
        }

        let quad = |x1, y1, x2, y2| [[x1, y1], [x2, y1], [x2, y2], [x1, y2]];
        let (scene, page) = scene_with_texts(vec![(
            transform(),
            text(
                Some("Peach\n蜜桃臀"),
                Some(vec![quad(2.0, 1.0, 30.0, 7.0), quad(2.0, 9.0, 30.0, 15.0)]),
            ),
        )]);
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 16, Rgb([1, 2, 3])));
        let probability = GrayImage::from_pixel(32, 16, Luma([255]));
        let (han_regions, eligible, protected) =
            segment_regions(&scene, page, SourceTextPolicy::HanOnly).unwrap();
        let (all_regions, _, _) = segment_regions(&scene, page, SourceTextPolicy::AllText).unwrap();

        let inactive_han = finalize_segment_mask(
            &image,
            &probability,
            &han_regions,
            &eligible,
            &protected,
            SourceTextPolicy::HanOnly,
        )
        .unwrap();
        let inactive_all = finalize_segment_mask(
            &image,
            &probability,
            &all_regions,
            &[],
            &[],
            SourceTextPolicy::AllText,
        )
        .unwrap();

        let capture = start_erase_capture();
        let active_han = finalize_segment_mask(
            &image,
            &probability,
            &han_regions,
            &eligible,
            &protected,
            SourceTextPolicy::HanOnly,
        )
        .unwrap();
        let han_events = capture.take();
        assert_eq!(active_han.as_raw(), inactive_han.as_raw());
        assert_eq!(
            han_events
                .iter()
                .map(|event| event.stage)
                .collect::<Vec<_>>(),
            [
                EraseDiagnosticStage::SegmentProbability,
                EraseDiagnosticStage::SegmentRefined,
                EraseDiagnosticStage::SegmentAllowedSupport,
                EraseDiagnosticStage::SegmentFinal,
            ]
        );
        assert!(
            han_events
                .iter()
                .all(|event| event.branch == EraseDiagnosticBranch::HanOnly)
        );
        assert_eq!(han_events[0].mask.as_ref().unwrap().nonzero_pixels, 512);
        assert_eq!(
            han_events.iter().map(signature).collect::<Vec<_>>(),
            [
                Some((
                    32,
                    16,
                    512,
                    "6a76f663f0a95a2f733ab79724659a1661ccd223c309cc27f30a56c94f860845"
                )),
                Some((
                    32,
                    16,
                    512,
                    "6a76f663f0a95a2f733ab79724659a1661ccd223c309cc27f30a56c94f860845"
                )),
                Some((
                    32,
                    16,
                    344,
                    "212c771e619d14c0765dfec3793dc16c1f9342c705507adb2b5f30b8787bbd7b"
                )),
                Some((
                    32,
                    16,
                    344,
                    "212c771e619d14c0765dfec3793dc16c1f9342c705507adb2b5f30b8787bbd7b"
                )),
            ]
        );

        let active_all = finalize_segment_mask(
            &image,
            &probability,
            &all_regions,
            &[],
            &[],
            SourceTextPolicy::AllText,
        )
        .unwrap();
        let all_events = capture.take();
        assert_eq!(active_all.as_raw(), inactive_all.as_raw());
        assert_ne!(active_han.as_raw(), active_all.as_raw());
        assert_eq!(all_events.len(), 4);
        assert!(
            all_events
                .iter()
                .all(|event| event.branch == EraseDiagnosticBranch::AllText)
        );
        assert_eq!(
            all_events[2].stage,
            EraseDiagnosticStage::SegmentAllowedSupport
        );
        assert_eq!(all_events[2].mask, None);
        assert_eq!(
            all_events.iter().map(signature).collect::<Vec<_>>(),
            [
                Some((
                    32,
                    16,
                    512,
                    "6a76f663f0a95a2f733ab79724659a1661ccd223c309cc27f30a56c94f860845"
                )),
                Some((
                    32,
                    16,
                    512,
                    "6a76f663f0a95a2f733ab79724659a1661ccd223c309cc27f30a56c94f860845"
                )),
                None,
                Some((
                    32,
                    16,
                    512,
                    "6a76f663f0a95a2f733ab79724659a1661ccd223c309cc27f30a56c94f860845"
                )),
            ]
        );
    }
}
