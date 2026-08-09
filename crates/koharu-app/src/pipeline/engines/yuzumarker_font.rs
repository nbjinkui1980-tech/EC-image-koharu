//! YuzuMarker font detection. Takes each text node's bbox on the source
//! image, runs the ML model, attaches a `FontPrediction` to the node.

use anyhow::Result;
use async_trait::async_trait;
use image::DynamicImage;
use koharu_core::{FontPrediction, NodeDataPatch, NodePatch, Op, TextDataPatch};
use koharu_ml::font_detector::FontDetector;

use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{load_source_image, text_nodes};

pub struct Model(FontDetector);

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        crate::pipeline::engine::emit_engine_device(
            "yuzumarker-font-detection",
            "yuzumarker-font-detection",
            0,
        );
        let texts = text_nodes(ctx.scene, ctx.page);
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let image = load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
        let crops = text_crops(&image, &texts);

        let mut preds = self.0.inference(&crops, 1)?;
        for p in &mut preds {
            normalize_font_prediction(p);
        }

        let mut ops = Vec::with_capacity(texts.len());
        for ((node_id, _, _), pred) in texts.iter().zip(preds) {
            ops.push(Op::UpdateNode {
                page: ctx.page,
                id: *node_id,
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(TextDataPatch {
                        font_prediction: Some(Some(ml_prediction_to_core(pred))),
                        // Clear any previous style so the renderer re-derives.
                        style: Some(None),
                        ..Default::default()
                    })),
                    transform: None,
                    visible: None,
                },
                prev: NodePatch::default(),
            });
        }
        Ok(ops)
    }
}

fn text_crops(
    image: &DynamicImage,
    texts: &[(
        koharu_core::NodeId,
        &koharu_core::Transform,
        &koharu_core::TextData,
    )],
) -> Vec<DynamicImage> {
    if image.width() == 0 || image.height() == 0 {
        return vec![DynamicImage::new_rgba8(1, 1); texts.len()];
    }
    texts
        .iter()
        .map(|(_, transform, _)| {
            let left = transform.x.floor().max(0.0) as u32;
            let top = transform.y.floor().max(0.0) as u32;
            let left = left.min(image.width() - 1);
            let top = top.min(image.height() - 1);
            let right = (transform.x + transform.width)
                .ceil()
                .max(left as f32 + 1.0) as u32;
            let bottom = (transform.y + transform.height)
                .ceil()
                .max(top as f32 + 1.0) as u32;
            image.crop_imm(
                left,
                top,
                right.min(image.width()) - left,
                bottom.min(image.height()) - top,
            )
        })
        .collect()
}

inventory::submit! {
    EngineInfo {
        id: "yuzumarker-font-detection",
        name: "YuzuMarker Font Detection",
        needs: &[Artifact::TextBoxes, Artifact::SourceTextBoxes],
        produces: &[Artifact::FontPredictions],
        load: |runtime, cpu| Box::pin(async move {
            let m = FontDetector::load(runtime, cpu).await?;
            Ok(Box::new(Model(m)) as Box<dyn Engine>)
        }),
    }
}

// ---------------------------------------------------------------------------
// Translate ml FontPrediction → scene FontPrediction
// ---------------------------------------------------------------------------

fn ml_prediction_to_core(p: koharu_ml::types::FontPrediction) -> FontPrediction {
    FontPrediction {
        top_fonts: p
            .top_fonts
            .into_iter()
            .map(|tf| koharu_core::TopFont {
                index: tf.index,
                score: tf.score,
            })
            .collect(),
        named_fonts: p
            .named_fonts
            .into_iter()
            .map(|nf| koharu_core::NamedFontPrediction {
                index: nf.index,
                name: nf.name,
                language: nf.language,
                probability: nf.probability,
                serif: nf.serif,
            })
            .collect(),
        direction: match p.direction {
            koharu_ml::types::TextDirection::Horizontal => koharu_core::TextDirection::Horizontal,
            koharu_ml::types::TextDirection::Vertical => koharu_core::TextDirection::Vertical,
        },
        text_color: p.text_color,
        stroke_color: p.stroke_color,
        font_size_px: p.font_size_px,
        stroke_width_px: p.stroke_width_px,
        line_height: p.line_height,
        angle_deg: p.angle_deg,
    }
}

// ---------------------------------------------------------------------------
// Color normalization (ported from legacy engine.rs)
// ---------------------------------------------------------------------------

fn normalize_font_prediction(p: &mut koharu_ml::types::FontPrediction) {
    p.text_color = clamp_white(clamp_black(p.text_color));
    p.stroke_color = clamp_white(clamp_black(p.stroke_color));
    if p.stroke_width_px > 0.0 && colors_similar(p.text_color, p.stroke_color) {
        p.stroke_width_px = 0.0;
        p.stroke_color = p.text_color;
    }
}

fn clamp_black(c: [u8; 3]) -> [u8; 3] {
    let t = if gray(c) { 60 } else { 12 };
    if c[0] <= t && c[1] <= t && c[2] <= t {
        [0, 0, 0]
    } else {
        c
    }
}

fn clamp_white(c: [u8; 3]) -> [u8; 3] {
    let t = 255 - if gray(c) { 60 } else { 12 };
    if c[0] >= t && c[1] >= t && c[2] >= t {
        [255, 255, 255]
    } else {
        c
    }
}

fn gray(c: [u8; 3]) -> bool {
    c.iter().max().unwrap().abs_diff(*c.iter().min().unwrap()) <= 10
}

fn colors_similar(a: [u8; 3], b: [u8; 3]) -> bool {
    (0..3).all(|i| a[i].abs_diff(b[i]) <= 16)
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgb, RgbImage};
    use koharu_core::{Node, NodeId, NodeKind, Page, Scene, TextData, Transform};

    use super::*;

    #[test]
    fn font_crops_exclude_english_between_han_targets() {
        let mut pixels = RgbImage::from_pixel(100, 60, Rgb([0, 0, 0]));
        for y in 20..40 {
            for x in 0..100 {
                pixels.put_pixel(x, y, Rgb([255, 0, 0]));
            }
        }
        let image = DynamicImage::ImageRgb8(pixels);
        let nodes = [
            ([10.0, 2.0, 50.0, 18.0], "中文一"),
            ([10.0, 42.0, 50.0, 58.0], "中文二"),
        ]
        .into_iter()
        .map(|(bbox, value)| {
            let id = NodeId::new();
            Node {
                id,
                transform: Transform {
                    x: bbox[0],
                    y: bbox[1],
                    width: bbox[2] - bbox[0],
                    height: bbox[3] - bbox[1],
                    rotation_deg: 0.0,
                },
                visible: true,
                kind: NodeKind::Text(TextData {
                    text: Some(value.into()),
                    ..Default::default()
                }),
            }
        })
        .collect::<Vec<_>>();
        let mut page = Page::new("page", 100, 60);
        let page_id = page.id;
        page.nodes = nodes.into_iter().map(|node| (node.id, node)).collect();
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        let texts = text_nodes(&scene, page_id);

        let crops = text_crops(&image, &texts);

        assert_eq!(crops.len(), 2);
        assert!(
            crops
                .iter()
                .all(|crop| { crop.to_rgb8().pixels().all(|pixel| pixel.0 != [255, 0, 0]) })
        );
    }
}
