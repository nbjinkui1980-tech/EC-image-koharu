//! Koharu renderer engine. Rasterises each text node's translation into an
//! RGBA sprite, composites them onto the inpainted plane, and writes back:
//!
//! - per-block `UpdateNode { TextDataPatch { sprite, sprite_transform,
//!   rendered_direction, style } }` (sprite blob stored as raw RGBA)
//! - one `upsert Image { role: Rendered }` for the final composite (webp)
//!
//! Requires an `Image { role: Inpainted }` node on the page.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use koharu_core::{
    ImageRole, MaskRole, NodeDataPatch, NodeId, NodePatch, Op, PageId, Scene, TextDataPatch,
    TextStyle, Transform,
};
use koharu_llm::Language;

use crate::config::SourceTextPolicy;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{
    EligibleTextLine, eligible_lines_for_page, find_image_node, find_mask_node, image_dimensions,
    load_source_image, text_nodes, upsert_image_blob,
};
use crate::renderer::{PageRenderOptions, RenderBlockInput};

pub struct Model;

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        // Find the target surface: prefer inpainted, fall back to source.
        let base = match find_image_node(ctx.scene, ctx.page, ImageRole::Inpainted) {
            Some((_, blob)) => ctx.blobs.load_image(&blob)?,
            None => load_source_image(ctx.scene, ctx.page, ctx.blobs)?,
        };
        let (w, h) = image_dimensions(&base);

        // Brush layer (optional): overlay before text sprites.
        let brush = match find_mask_node(ctx.scene, ctx.page, MaskRole::BrushInpaint) {
            Some((_, blob)) => Some(ctx.blobs.load_image(&blob)?),
            None => None,
        };

        // Bubble-interior mask (optional): grows latin layout boxes so text
        // wraps inside the available bubble space.
        let bubble = match find_mask_node(ctx.scene, ctx.page, MaskRole::Bubble) {
            Some((_, blob)) => Some(ctx.blobs.load_image(&blob)?),
            None => None,
        };

        let (inputs, mut ops) = build_render_inputs(
            ctx.scene,
            ctx.page,
            ctx.options.source_text_policy,
            ctx.options.text_node_ids.as_deref(),
        )?;

        let page_opts = PageRenderOptions {
            shader_effect: Default::default(),
            shader_stroke: None,
            document_font: ctx.options.default_font.clone(),
            target_language: ctx
                .options
                .target_language
                .as_deref()
                .map(render_target_language_tag),
            raster: Default::default(),
        };

        // `render_page` is synchronous and CPU-bound. It runs inline on the
        // current tokio worker; for multi-page jobs the driver parallelises
        // across pages via separate `run()` calls.
        let output = ctx.renderer.render_page(
            &base,
            brush.as_ref(),
            bubble.as_ref(),
            w,
            h,
            &inputs,
            &page_opts,
        )?;

        // Upload sprites + compose ops.
        ops.reserve(output.blocks.len() + 1);
        for block_out in output.blocks {
            let sprite_ref = ctx.blobs.put_raw(&block_out.sprite)?;
            let existing_style = inputs
                .iter()
                .find(|i| i.node_id == block_out.node_id)
                .and_then(|i| i.style.clone());
            ops.push(Op::UpdateNode {
                page: ctx.page,
                id: block_out.node_id,
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(TextDataPatch {
                        sprite: Some(Some(sprite_ref)),
                        sprite_transform: Some(
                            block_out.expanded_transform.map(normalize_transform),
                        ),
                        rendered_direction: Some(Some(block_out.rendered_direction)),
                        // Only persist explicit user style overrides. Writing
                        // a synthetic default style back into the scene makes
                        // later renders treat implicit predicted colors as
                        // explicit black overrides.
                        style: preserve_existing_style(existing_style),
                        ..Default::default()
                    })),
                    transform: None,
                    visible: None,
                },
                prev: NodePatch::default(),
            });
        }

        // Final composite → Image { Rendered } upsert.
        let final_blob = ctx.blobs.put_webp(&output.final_render)?;
        ops.push(upsert_image_blob(
            ctx.scene,
            ctx.page,
            ImageRole::Rendered,
            final_blob,
            w,
            h,
        ));
        Ok(ops)
    }
}

fn build_render_inputs(
    scene: &Scene,
    page: PageId,
    policy: SourceTextPolicy,
    allowed_ids: Option<&[NodeId]>,
) -> Result<(Vec<RenderBlockInput>, Vec<Op>)> {
    if policy == SourceTextPolicy::AllText {
        let inputs = text_nodes(scene, page)
            .into_iter()
            .filter_map(|(node_id, transform, text)| {
                let translation = text.translation.as_deref()?.trim();
                if translation.is_empty() {
                    return None;
                }
                Some(RenderBlockInput {
                    node_id,
                    transform: *transform,
                    translation: translation.to_string(),
                    style: text.style.clone(),
                    font_prediction: text.font_prediction.clone(),
                    source_direction: text.source_direction,
                    rendered_direction: text.rendered_direction,
                    lock_layout_box: text.lock_layout_box,
                    preserve_explicit_lines: false,
                })
            })
            .collect();
        return Ok((inputs, Vec::new()));
    }

    let mut lines_by_node: HashMap<NodeId, Vec<EligibleTextLine>> = HashMap::new();
    for (node_id, line) in eligible_lines_for_page(scene, page).0 {
        lines_by_node.entry(node_id).or_default().push(line);
    }

    let mut inputs = Vec::new();
    let mut cleanup = Vec::new();
    for (node_id, _, text) in text_nodes(scene, page) {
        if allowed_ids.is_some_and(|ids| !ids.contains(&node_id)) {
            continue;
        }
        let mut lines = lines_by_node.remove(&node_id).unwrap_or_default();
        if lines.is_empty() {
            cleanup.push(render_cleanup_op(page, node_id, true));
            continue;
        }
        lines.sort_by_key(|line| line.line_index);

        let translation = text
            .translation
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Han translation missing for node {node_id}"))?;
        let translated_lines = translation
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        anyhow::ensure!(
            translated_lines.len() == lines.len(),
            "Han translation line count mismatch for node {node_id}"
        );

        let mut left = f32::INFINITY;
        let mut top = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        for line in &lines {
            let region = &line.region;
            let values = [region.x, region.y, region.width, region.height];
            anyhow::ensure!(
                values.iter().all(|value| value.is_finite())
                    && region.width > 0.0
                    && region.height > 0.0,
                "invalid Han renderer geometry for node {node_id}"
            );
            left = left.min(region.x);
            top = top.min(region.y);
            right = right.max(region.x + region.width);
            bottom = bottom.max(region.y + region.height);
        }

        let source_line_count = text
            .text
            .as_deref()
            .map(|source| {
                source
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count()
            })
            .unwrap_or(0);
        inputs.push(RenderBlockInput {
            node_id,
            transform: Transform {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
                rotation_deg: 0.0,
            },
            translation: translated_lines.join("\n"),
            style: text.style.clone(),
            font_prediction: text.font_prediction.clone(),
            source_direction: text.source_direction,
            rendered_direction: text.rendered_direction,
            lock_layout_box: text.lock_layout_box || lines.len() < source_line_count,
            preserve_explicit_lines: true,
        });
        cleanup.push(render_cleanup_op(page, node_id, false));
    }
    Ok((inputs, cleanup))
}

fn render_cleanup_op(page: PageId, node_id: NodeId, clear_translation: bool) -> Op {
    Op::UpdateNode {
        page,
        id: node_id,
        patch: NodePatch {
            data: Some(NodeDataPatch::Text(TextDataPatch {
                translation: clear_translation.then_some(None),
                sprite: Some(None),
                sprite_transform: Some(None),
                ..Default::default()
            })),
            transform: None,
            visible: None,
        },
        prev: NodePatch::default(),
    }
}

inventory::submit! {
    EngineInfo {
        id: "koharu-renderer",
        name: "Koharu Renderer",
        needs: &[
            Artifact::Inpainted,
            Artifact::Translations,
            Artifact::FontPredictions,
        ],
        produces: &[Artifact::FinalRender, Artifact::RenderedSprites],
        load: |_runtime, _cpu| Box::pin(async move {
            Ok(Box::new(Model) as Box<dyn Engine>)
        }),
    }
}

fn normalize_transform(t: Transform) -> Transform {
    Transform {
        x: t.x.round(),
        y: t.y.round(),
        width: t.width.round(),
        height: t.height.round(),
        rotation_deg: t.rotation_deg,
    }
}

fn preserve_existing_style(existing: Option<TextStyle>) -> Option<Option<TextStyle>> {
    existing.map(Some)
}

fn render_target_language_tag(value: &str) -> String {
    Language::parse(value)
        .map(|language| language.tag().to_string())
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_core::{BlobRef, Node, NodeId, NodeKind, Page, Scene, TextData, TextDirection};

    use crate::config::SourceTextPolicy;

    #[test]
    fn omits_style_patch_when_block_has_no_explicit_style() {
        assert!(preserve_existing_style(None).is_none());
    }

    #[test]
    fn preserves_existing_explicit_style() {
        let style = TextStyle {
            font_families: vec!["Arial".to_string()],
            font_size: Some(18.0),
            color: [12, 34, 56, 255],
            effect: None,
            stroke: None,
            text_align: None,
        };
        let preserved = preserve_existing_style(Some(style));
        let Some(Some(preserved)) = preserved else {
            panic!("expected explicit style patch");
        };
        assert_eq!(preserved.font_families, vec!["Arial".to_string()]);
        assert_eq!(preserved.font_size, Some(18.0));
        assert_eq!(preserved.color, [12, 34, 56, 255]);
        assert!(preserved.effect.is_none());
        assert!(preserved.stroke.is_none());
        assert!(preserved.text_align.is_none());
    }

    #[test]
    fn render_target_language_normalizes_language_names() {
        assert_eq!(render_target_language_tag("German"), "de-DE");
        assert_eq!(render_target_language_tag("pt-BR"), "pt-BR");
        assert_eq!(
            render_target_language_tag("not-a-language"),
            "not-a-language"
        );
    }

    fn quad(x1: f32, y1: f32, x2: f32, y2: f32) -> [[f32; 2]; 4] {
        [[x1, y1], [x2, y1], [x2, y2], [x1, y2]]
    }

    fn renderer_node(
        id: NodeId,
        text: &str,
        translation: Option<&str>,
        polygons: Option<Vec<[[f32; 2]; 4]>>,
    ) -> Node {
        Node {
            id,
            transform: Transform {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 50.0,
                rotation_deg: 0.0,
            },
            visible: true,
            kind: NodeKind::Text(TextData {
                text: Some(text.to_string()),
                translation: translation.map(str::to_string),
                line_polygons: polygons,
                source_direction: Some(TextDirection::Horizontal),
                sprite: Some(BlobRef::new("old-sprite")),
                sprite_transform: Some(Transform::default()),
                ..Default::default()
            }),
        }
    }

    fn renderer_scene(nodes: Vec<Node>) -> (Scene, koharu_core::PageId) {
        let mut page = Page::new("page", 100, 100);
        let page_id = page.id;
        page.nodes = nodes.into_iter().map(|node| (node.id, node)).collect();
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        (scene, page_id)
    }

    fn text(scene: &Scene, page: koharu_core::PageId, id: NodeId) -> &TextData {
        match &scene.node(page, id).unwrap().kind {
            NodeKind::Text(text) => text,
            _ => panic!("expected text node"),
        }
    }

    #[test]
    fn han_only_renderer_uses_han_geometry_and_cleans_in_scope_nodes() {
        let mixed = NodeId::new();
        let english = NodeId::new();
        let unsupported = NodeId::new();
        let outside = NodeId::new();
        let (mut scene, page) = renderer_scene(vec![
            renderer_node(
                mixed,
                "English\n中文",
                Some("Translated"),
                Some(vec![
                    quad(12.0, 12.0, 88.0, 28.0),
                    quad(20.0, 35.0, 70.0, 52.0),
                ]),
            ),
            renderer_node(english, "English", Some("old"), None),
            renderer_node(unsupported, "English\n中文", Some("old"), None),
            renderer_node(outside, "中文", Some("outside"), None),
        ]);
        let allowed = [mixed, english, unsupported];

        let (inputs, mut cleanup) =
            build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, Some(&allowed)).unwrap();

        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].node_id, mixed);
        assert_eq!(
            (
                inputs[0].transform.x,
                inputs[0].transform.y,
                inputs[0].transform.width,
                inputs[0].transform.height,
                inputs[0].transform.rotation_deg,
            ),
            (20.0, 35.0, 50.0, 17.0, 0.0)
        );
        assert!(inputs[0].lock_layout_box);
        assert!(inputs[0].preserve_explicit_lines);

        for op in &mut cleanup {
            op.apply(&mut scene).unwrap();
        }
        assert_eq!(
            text(&scene, page, mixed).translation.as_deref(),
            Some("Translated")
        );
        assert!(text(&scene, page, mixed).sprite.is_none());
        for id in [english, unsupported] {
            assert!(text(&scene, page, id).translation.is_none());
            assert!(text(&scene, page, id).sprite.is_none());
            assert!(text(&scene, page, id).sprite_transform.is_none());
        }
        assert_eq!(
            text(&scene, page, outside).translation.as_deref(),
            Some("outside")
        );
        assert!(text(&scene, page, outside).sprite.is_some());
    }

    #[test]
    fn han_only_renderer_rejects_translation_line_count_mismatch() {
        let mixed = NodeId::new();
        let (scene, page) = renderer_scene(vec![renderer_node(
            mixed,
            "English\n中文一\n中文二",
            Some("only one"),
            Some(vec![
                quad(10.0, 10.0, 90.0, 20.0),
                quad(10.0, 25.0, 90.0, 35.0),
                quad(10.0, 40.0, 90.0, 50.0),
            ]),
        )]);

        assert!(build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None).is_err());
    }

    #[test]
    fn han_only_renderer_keeps_all_text_transform_and_layout_behavior() {
        let id = NodeId::new();
        let mut node = renderer_node(id, "English\n中文", Some("A translated paragraph"), None);
        let original = node.transform;
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.lock_layout_box = true;
        let (scene, page) = renderer_scene(vec![node]);

        let (inputs, cleanup) =
            build_render_inputs(&scene, page, SourceTextPolicy::AllText, Some(&[])).unwrap();

        assert_eq!(inputs.len(), 1);
        assert_eq!(
            (
                inputs[0].transform.x,
                inputs[0].transform.y,
                inputs[0].transform.width,
                inputs[0].transform.height,
                inputs[0].transform.rotation_deg,
            ),
            (
                original.x,
                original.y,
                original.width,
                original.height,
                original.rotation_deg,
            )
        );
        assert!(inputs[0].lock_layout_box);
        assert!(!inputs[0].preserve_explicit_lines);
        assert!(cleanup.is_empty());
    }
}
