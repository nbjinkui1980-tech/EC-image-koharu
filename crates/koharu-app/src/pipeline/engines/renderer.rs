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
use image::{DynamicImage, GenericImageView, imageops};
use koharu_core::{
    ImageRole, MaskRole, NodeDataPatch, NodeId, NodePatch, Op, PageId, Scene, TextDataPatch,
    TextStyle, Transform,
};
use koharu_llm::Language;

use crate::blobs::BlobStore;
use crate::config::SourceTextPolicy;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo, PipelineRunOptions};
use crate::pipeline::engines::support::{
    EligibleTextLine, eligible_lines_for_page, find_image_node, find_mask_node, image_dimensions,
    line_support_mask, load_source_image, protected_source_lines_for_page, text_nodes,
    upsert_image_blob,
};
use crate::renderer::{PageRenderOptions, RenderBlockInput, RenderOutput, RenderedBlock};

pub struct Model;

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        run_renderer_page(
            ctx.scene,
            ctx.page,
            ctx.blobs,
            ctx.options,
            |base, brush, bubble, w, h, inputs, page_opts| {
                ctx.renderer
                    .render_page(base, brush, bubble, w, h, inputs, page_opts)
            },
        )
    }
}

fn run_renderer_page(
    scene: &Scene,
    page: PageId,
    blobs: &BlobStore,
    options: &PipelineRunOptions,
    render: impl FnOnce(
        &DynamicImage,
        Option<&DynamicImage>,
        Option<&DynamicImage>,
        u32,
        u32,
        &[RenderBlockInput],
        &PageRenderOptions,
    ) -> Result<RenderOutput>,
) -> Result<Vec<Op>> {
    let source = load_source_image(scene, page, blobs)?;
    // Find the target surface: prefer inpainted, fall back to source.
    let base = match find_image_node(scene, page, ImageRole::Inpainted) {
        Some((_, blob)) => blobs.load_image(&blob)?,
        None => source.clone(),
    };
    let (w, h) = image_dimensions(&base);
    anyhow::ensure!(
        source.dimensions() == base.dimensions(),
        "Source and render base dimensions differ"
    );

    // Brush layer (optional): overlay before text sprites.
    let brush = match find_mask_node(scene, page, MaskRole::BrushInpaint) {
        Some((_, blob)) => Some(blobs.load_image(&blob)?),
        None => None,
    };

    // Bubble-interior mask (optional): grows latin layout boxes so text
    // wraps inside the available bubble space.
    let bubble = match find_mask_node(scene, page, MaskRole::Bubble) {
        Some((_, blob)) => Some(blobs.load_image(&blob)?),
        None => None,
    };

    let has_text_nodes = !text_nodes(scene, page).is_empty();
    let (inputs, mutable_ids, mut ops, eligible_lines) = build_render_inputs(
        scene,
        page,
        options.source_text_policy,
        options.text_node_ids.as_deref(),
    )?;
    let protected_source_lines = if options.source_text_policy == SourceTextPolicy::HanOnly {
        protected_source_lines_for_page(scene, page)
    } else {
        Vec::new()
    };

    let page_opts = PageRenderOptions {
        shader_effect: Default::default(),
        shader_stroke: None,
        document_font: options.default_font.clone(),
        target_language: options
            .target_language
            .as_deref()
            .map(render_target_language_tag),
        raster: Default::default(),
    };

    // `render_page` is synchronous and CPU-bound. It runs inline on the
    // current tokio worker; for multi-page jobs the driver parallelises
    // across pages via separate `run()` calls.
    let output = dispatch_render_page(
        options.source_text_policy,
        has_text_nodes,
        &source,
        &base,
        brush.as_ref(),
        &inputs,
        &eligible_lines,
        &protected_source_lines,
        || {
            render(
                &base,
                brush.as_ref(),
                bubble.as_ref(),
                w,
                h,
                &inputs,
                &page_opts,
            )
        },
    )?;

    // Upload sprites + compose ops.
    let mut blocks = output.blocks;
    retain_mutable_blocks(&mut blocks, &mutable_ids);
    ops.reserve(blocks.len() + 1);
    for block_out in blocks {
        let sprite_ref = blobs.put_raw(&block_out.sprite)?;
        let existing_style = inputs
            .iter()
            .find(|i| i.node_id == block_out.node_id)
            .and_then(|i| i.style.clone());
        ops.push(Op::UpdateNode {
            page,
            id: block_out.node_id,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    sprite: Some(Some(sprite_ref)),
                    sprite_transform: Some(block_out.expanded_transform.map(normalize_transform)),
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
    let final_blob = blobs.put_webp(&output.final_render)?;
    ops.push(upsert_image_blob(
        scene,
        page,
        ImageRole::Rendered,
        final_blob,
        w,
        h,
    ));
    Ok(ops)
}

#[allow(clippy::too_many_arguments)]
fn dispatch_render_page(
    policy: SourceTextPolicy,
    has_text_nodes: bool,
    source: &DynamicImage,
    base: &DynamicImage,
    brush: Option<&DynamicImage>,
    inputs: &[RenderBlockInput],
    eligible_lines: &[NodeEligibleLine],
    protected_source_lines: &[NodeEligibleLine],
    render: impl FnOnce() -> Result<RenderOutput>,
) -> Result<RenderOutput> {
    if policy == SourceTextPolicy::HanOnly && has_text_nodes && inputs.is_empty() {
        let mut canvas = base.to_rgba8();
        restore_protected_source_pixels(&mut canvas, source, protected_source_lines)?;
        if let Some(brush) = brush {
            imageops::overlay(&mut canvas, &brush.to_rgba8(), 0, 0);
        }
        return Ok(RenderOutput {
            final_render: DynamicImage::ImageRgba8(canvas),
            blocks: Vec::new(),
        });
    }
    let output = render()?;
    if policy == SourceTextPolicy::HanOnly {
        clip_han_render_output(
            source,
            base,
            brush,
            inputs,
            eligible_lines,
            protected_source_lines,
            output,
        )
    } else {
        Ok(output)
    }
}

fn clip_han_render_output(
    source: &DynamicImage,
    base: &DynamicImage,
    brush: Option<&DynamicImage>,
    inputs: &[RenderBlockInput],
    eligible_lines: &[NodeEligibleLine],
    protected_source_lines: &[NodeEligibleLine],
    mut output: RenderOutput,
) -> Result<RenderOutput> {
    for block in &mut output.blocks {
        let input = inputs
            .iter()
            .find(|input| input.node_id == block.node_id)
            .ok_or_else(|| anyhow::anyhow!("rendered block has no matching input"))?;
        let block_lines = eligible_lines
            .iter()
            .filter(|(node_id, _)| *node_id == block.node_id)
            .map(|(_, line)| line.clone())
            .collect::<Vec<_>>();
        let allowed = line_support_mask(base.width(), base.height(), &block_lines);
        let (origin_x, origin_y) = render_origin(input, &block.expanded_transform);
        let mut sprite = block.sprite.to_rgba8();
        for (x, y, pixel) in sprite.enumerate_pixels_mut() {
            let page_x = origin_x + i64::from(x);
            let page_y = origin_y + i64::from(y);
            let inside = page_x >= 0
                && page_y >= 0
                && page_x < i64::from(allowed.width())
                && page_y < i64::from(allowed.height())
                && allowed.get_pixel(page_x as u32, page_y as u32).0[0] != 0;
            if !inside {
                pixel.0[3] = 0;
            }
        }
        block.sprite = DynamicImage::ImageRgba8(sprite);
    }

    let mut canvas = base.to_rgba8();
    restore_protected_source_pixels(&mut canvas, source, protected_source_lines)?;
    if let Some(brush) = brush {
        imageops::overlay(&mut canvas, &brush.to_rgba8(), 0, 0);
    }
    for block in &output.blocks {
        let input = inputs
            .iter()
            .find(|input| input.node_id == block.node_id)
            .ok_or_else(|| anyhow::anyhow!("rendered block has no matching input"))?;
        let (x, y) = render_origin(input, &block.expanded_transform);
        imageops::overlay(&mut canvas, &block.sprite.to_rgba8(), x, y);
    }
    output.final_render = DynamicImage::ImageRgba8(canvas);
    Ok(output)
}

fn restore_protected_source_pixels(
    canvas: &mut image::RgbaImage,
    source: &DynamicImage,
    protected_source_lines: &[NodeEligibleLine],
) -> Result<()> {
    anyhow::ensure!(
        canvas.dimensions() == source.dimensions(),
        "Source and render canvas dimensions differ"
    );
    let lines = protected_source_lines
        .iter()
        .map(|(_, line)| line.clone())
        .collect::<Vec<_>>();
    let mask = line_support_mask(canvas.width(), canvas.height(), &lines);
    let source = source.to_rgba8();
    for (x, y, pixel) in canvas.enumerate_pixels_mut() {
        if mask.get_pixel(x, y).0[0] != 0 {
            *pixel = *source.get_pixel(x, y);
        }
    }
    Ok(())
}

fn render_origin(input: &RenderBlockInput, expanded: &Option<Transform>) -> (i64, i64) {
    let (x, y) = crate::renderer::placement_origin(input, expanded);
    (x as i64, y as i64)
}

fn retain_mutable_blocks(blocks: &mut Vec<RenderedBlock>, mutable_ids: &[NodeId]) {
    blocks.retain(|block| mutable_ids.contains(&block.node_id));
}

type RenderInputPlan = (
    Vec<RenderBlockInput>,
    Vec<NodeId>,
    Vec<Op>,
    Vec<NodeEligibleLine>,
);

type NodeEligibleLine = (NodeId, EligibleTextLine);

fn build_render_inputs(
    scene: &Scene,
    page: PageId,
    policy: SourceTextPolicy,
    allowed_ids: Option<&[NodeId]>,
) -> Result<RenderInputPlan> {
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
            .collect::<Vec<_>>();
        let mutable_ids = inputs.iter().map(|input| input.node_id).collect();
        return Ok((inputs, mutable_ids, Vec::new(), Vec::new()));
    }

    let mut lines_by_node: HashMap<NodeId, Vec<EligibleTextLine>> = HashMap::new();
    for (node_id, line) in eligible_lines_for_page(scene, page).0 {
        lines_by_node.entry(node_id).or_default().push(line);
    }

    let mut inputs = Vec::new();
    let mut mutable_ids = Vec::new();
    let mut cleanup = Vec::new();
    let mut render_lines = Vec::new();
    for (node_id, _, text) in text_nodes(scene, page) {
        let in_scope = allowed_ids.is_none_or(|ids| ids.contains(&node_id));
        let mut lines = lines_by_node.remove(&node_id).unwrap_or_default();
        if lines.is_empty() {
            if in_scope {
                cleanup.push(render_cleanup_op(page, node_id));
            }
            continue;
        }
        lines.sort_by_key(|line| line.line_index);

        let Some(translation) = text.translation.as_deref() else {
            if in_scope {
                cleanup.push(render_cleanup_op(page, node_id));
            }
            continue;
        };
        let translated_lines = translation
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if translated_lines.len() != lines.len() {
            if in_scope {
                cleanup.push(render_cleanup_op(page, node_id));
            }
            continue;
        }

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
            lock_layout_box: text.lock_layout_box
                || lines.len() == 1
                || lines.len() < source_line_count,
            preserve_explicit_lines: true,
        });
        render_lines.extend(lines.iter().cloned().map(|line| (node_id, line)));
        if in_scope {
            mutable_ids.push(node_id);
            cleanup.push(render_cleanup_op(page, node_id));
        }
    }
    Ok((inputs, mutable_ids, cleanup, render_lines))
}

fn render_cleanup_op(page: PageId, node_id: NodeId) -> Op {
    Op::UpdateNode {
        page,
        id: node_id,
        patch: NodePatch {
            data: Some(NodeDataPatch::Text(TextDataPatch {
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
    use std::cell::Cell;

    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};
    use koharu_core::{
        BlobRef, ImageData, ImageRole, Node, NodeId, NodeKind, Page, Scene, TextData, TextDirection,
    };

    use crate::blobs::BlobStore;
    use crate::config::SourceTextPolicy;
    use crate::pipeline::engine::PipelineRunOptions;

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

    fn image_node(id: NodeId, role: ImageRole, blob: BlobRef) -> Node {
        Node {
            id,
            transform: Transform::default(),
            visible: true,
            kind: NodeKind::Image(ImageData {
                role,
                blob,
                opacity: 1.0,
                natural_width: 100,
                natural_height: 100,
                name: None,
            }),
        }
    }

    fn renderer_scene_with_images(
        blobs: &BlobStore,
        text_node: Node,
    ) -> Result<(Scene, koharu_core::PageId)> {
        let source =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([10, 20, 30, 255])));
        let inpainted =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([200, 200, 200, 255])));
        let source_node = image_node(NodeId::new(), ImageRole::Source, blobs.put_webp(&source)?);
        let inpainted_node = image_node(
            NodeId::new(),
            ImageRole::Inpainted,
            blobs.put_webp(&inpainted)?,
        );
        Ok(renderer_scene(vec![source_node, inpainted_node, text_node]))
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

        let (inputs, mutable_ids, mut cleanup, _) =
            build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, Some(&allowed)).unwrap();

        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].node_id, mixed);
        assert_eq!(inputs[1].node_id, outside);
        assert_eq!(mutable_ids, vec![mixed]);
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
            assert_eq!(text(&scene, page, id).translation.as_deref(), Some("old"));
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
    fn han_only_renderer_skips_mismatched_translation_and_cleans_scope() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let mixed = NodeId::new();
        let node = renderer_node(
            mixed,
            "English\n中文一\n中文二",
            Some("only one"),
            Some(vec![
                quad(10.0, 10.0, 90.0, 20.0),
                quad(10.0, 25.0, 90.0, 35.0),
                quad(10.0, 40.0, 90.0, 50.0),
            ]),
        );
        let (mut scene, page) = renderer_scene_with_images(&blobs, node)?;
        let options = PipelineRunOptions {
            source_text_policy: SourceTextPolicy::HanOnly,
            text_node_ids: Some(vec![mixed]),
            ..Default::default()
        };
        let calls = Cell::new(0);

        let mut ops = run_renderer_page(&scene, page, &blobs, &options, |_, _, _, _, _, _, _| {
            calls.set(calls.get() + 1);
            unreachable!("mismatched Han translation must not call the renderer")
        })?;

        assert_eq!(calls.get(), 0);
        assert_eq!(ops.len(), 2);
        for op in &mut ops {
            op.apply(&mut scene)?;
        }
        assert_eq!(
            text(&scene, page, mixed).translation.as_deref(),
            Some("only one")
        );
        assert!(text(&scene, page, mixed).sprite.is_none());
        assert!(text(&scene, page, mixed).sprite_transform.is_none());
        let (_, rendered_blob) = find_image_node(&scene, page, ImageRole::Rendered)
            .expect("Rendered upsert must be emitted");
        let rendered = blobs.load_image(&rendered_blob)?.to_rgba8();
        assert_eq!(rendered.get_pixel(20, 40).0, [10, 20, 30, 255]);
        assert_eq!(rendered.get_pixel(5, 5).0, [200, 200, 200, 255]);
        Ok(())
    }

    #[test]
    fn han_only_renderer_skips_untranslated_han_and_restores_source() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let id = NodeId::new();
        let node = renderer_node(id, "蜜桃臀", None, None);
        let (mut scene, page) = renderer_scene_with_images(&blobs, node)?;
        let options = PipelineRunOptions {
            source_text_policy: SourceTextPolicy::HanOnly,
            text_node_ids: Some(vec![id]),
            ..Default::default()
        };
        let calls = Cell::new(0);

        let mut ops = run_renderer_page(&scene, page, &blobs, &options, |_, _, _, _, _, _, _| {
            calls.set(calls.get() + 1);
            unreachable!("untranslated Han must not call the renderer")
        })?;

        assert_eq!(calls.get(), 0);
        assert_eq!(ops.len(), 2);
        for op in &mut ops {
            op.apply(&mut scene)?;
        }
        assert!(text(&scene, page, id).translation.is_none());
        assert!(text(&scene, page, id).sprite.is_none());
        assert!(text(&scene, page, id).sprite_transform.is_none());
        let (_, rendered_blob) = find_image_node(&scene, page, ImageRole::Rendered)
            .expect("Rendered upsert must be emitted");
        let rendered = blobs.load_image(&rendered_blob)?.to_rgba8();
        assert_eq!(rendered.get_pixel(20, 20).0, [10, 20, 30, 255]);
        assert_eq!(rendered.get_pixel(5, 5).0, [200, 200, 200, 255]);
        Ok(())
    }

    #[test]
    fn han_only_renderer_restores_rotated_unsupported_from_source() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let id = NodeId::new();
        let mut node = renderer_node(id, "Peach\n蜜桃臀", None, None);
        node.transform = Transform {
            x: 40.0,
            y: 40.0,
            width: 20.0,
            height: 10.0,
            rotation_deg: 45.0,
        };
        let (mut scene, page) = renderer_scene_with_images(&blobs, node)?;
        let options = PipelineRunOptions {
            source_text_policy: SourceTextPolicy::HanOnly,
            text_node_ids: Some(vec![id]),
            ..Default::default()
        };
        let calls = Cell::new(0);

        let mut ops = run_renderer_page(&scene, page, &blobs, &options, |_, _, _, _, _, _, _| {
            calls.set(calls.get() + 1);
            unreachable!("unsupported rotated node must not call the renderer")
        })?;

        assert_eq!(calls.get(), 0);
        for op in &mut ops {
            op.apply(&mut scene)?;
        }
        let (_, rendered_blob) = find_image_node(&scene, page, ImageRole::Rendered)
            .expect("Rendered upsert must be emitted");
        let rendered = blobs.load_image(&rendered_blob)?.to_rgba8();
        assert_eq!(rendered.get_pixel(40, 35).0, [10, 20, 30, 255]);
        assert_eq!(rendered.get_pixel(20, 20).0, [200, 200, 200, 255]);
        Ok(())
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

        let (inputs, mutable_ids, cleanup, _) =
            build_render_inputs(&scene, page, SourceTextPolicy::AllText, Some(&[])).unwrap();

        assert_eq!(inputs.len(), 1);
        assert_eq!(mutable_ids, vec![id]);
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

    #[test]
    fn han_only_renderer_scoped_composite_keeps_outside_translations() -> Result<()> {
        let selected = NodeId::new();
        let outside = NodeId::new();
        let selected_node = renderer_node(selected, "中文一", Some("One"), None);
        let mut outside_node = renderer_node(outside, "中文二", Some("Two"), None);
        outside_node.transform.x = 60.0;
        outside_node.transform.y = 60.0;
        let (scene, page) = renderer_scene(vec![selected_node, outside_node]);

        let (inputs, mutable_ids, cleanup, eligible_lines) =
            build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, Some(&[selected]))?;
        assert_eq!(
            inputs.iter().map(|input| input.node_id).collect::<Vec<_>>(),
            vec![selected, outside]
        );
        assert_eq!(mutable_ids, vec![selected]);
        assert_eq!(cleanup.len(), 1);

        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255])));
        let calls = Cell::new(0);
        let output = dispatch_render_page(
            SourceTextPolicy::HanOnly,
            true,
            &base,
            &base,
            None,
            &inputs,
            &eligible_lines,
            &[],
            || {
                calls.set(calls.get() + 1);
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: inputs
                        .iter()
                        .enumerate()
                        .map(|(index, input)| RenderedBlock {
                            node_id: input.node_id,
                            sprite: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                                1,
                                1,
                                if index == 0 {
                                    Rgba([255, 0, 0, 255])
                                } else {
                                    Rgba([0, 0, 255, 255])
                                },
                            )),
                            rendered_direction: TextDirection::Horizontal,
                            expanded_transform: None,
                        })
                        .collect(),
                })
            },
        )?;

        assert_eq!(calls.get(), 1);
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(10, 10).0,
            [255, 0, 0, 255]
        );
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(60, 60).0,
            [0, 0, 255, 255]
        );
        let mut blocks = output.blocks;
        retain_mutable_blocks(&mut blocks, &mutable_ids);
        let updated = blocks.iter().map(|block| block.node_id).collect::<Vec<_>>();
        assert_eq!(updated, vec![selected]);
        Ok(())
    }

    #[test]
    fn han_only_renderer_empty_text_targets_skip_backend_but_textless_renders() -> Result<()> {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 255])));

        for node in [
            renderer_node(NodeId::new(), "English", Some("old"), None),
            renderer_node(NodeId::new(), "English\n中文", Some("old"), None),
        ] {
            let (scene, page) = renderer_scene(vec![node]);
            let (inputs, mutable_ids, cleanup, eligible_lines) =
                build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;
            assert!(inputs.is_empty());
            assert!(mutable_ids.is_empty());
            assert_eq!(cleanup.len(), 1);

            let calls = Cell::new(0);
            let output = dispatch_render_page(
                SourceTextPolicy::HanOnly,
                true,
                &base,
                &base,
                None,
                &inputs,
                &eligible_lines,
                &[],
                || {
                    calls.set(calls.get() + 1);
                    unreachable!("empty Han-only targets must skip the renderer")
                },
            )?;
            assert_eq!(calls.get(), 0);
            assert_eq!(output.final_render.to_rgba8(), base.to_rgba8());
        }

        let calls = Cell::new(0);
        let output = dispatch_render_page(
            SourceTextPolicy::HanOnly,
            false,
            &base,
            &base,
            None,
            &[],
            &[],
            &[],
            || {
                calls.set(calls.get() + 1);
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: Vec::new(),
                })
            },
        )?;
        assert_eq!(calls.get(), 1);
        assert_eq!(output.final_render.to_rgba8(), base.to_rgba8());
        Ok(())
    }

    #[test]
    fn han_only_renderer_clips_word_box_inline_mixed_to_han_mask() -> Result<()> {
        let node_id = NodeId::new();
        let input = RenderBlockInput {
            node_id,
            transform: Transform {
                x: 55.0,
                y: 10.0,
                width: 35.0,
                height: 20.0,
                rotation_deg: 0.0,
            },
            translation: "Translated".to_string(),
            style: None,
            font_prediction: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
        };
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 40, Rgba([10, 20, 30, 255])));
        let mut brush = RgbaImage::new(100, 40);
        brush.put_pixel(20, 15, Rgba([30, 200, 40, 255]));
        let brush = DynamicImage::ImageRgba8(brush);
        let eligible_lines = vec![(
            node_id,
            EligibleTextLine {
                line_index: 1,
                text: "蜜桃臀".to_string(),
                region: koharu_ml::types::TextRegion {
                    x: 55.0,
                    y: 10.0,
                    width: 35.0,
                    height: 20.0,
                    line_polygons: Some(vec![quad(55.0, 10.0, 90.0, 30.0)]),
                    ..Default::default()
                },
            },
        )];
        let fake_output = || {
            Ok(RenderOutput {
                final_render: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                    100,
                    40,
                    Rgba([250, 0, 0, 255]),
                )),
                blocks: vec![RenderedBlock {
                    node_id,
                    sprite: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                        80,
                        20,
                        Rgba([250, 0, 0, 255]),
                    )),
                    rendered_direction: TextDirection::Horizontal,
                    expanded_transform: Some(Transform {
                        x: 10.0,
                        y: 10.0,
                        width: 80.0,
                        height: 20.0,
                        rotation_deg: 0.0,
                    }),
                }],
            })
        };

        let output = dispatch_render_page(
            SourceTextPolicy::HanOnly,
            true,
            &base,
            &base,
            Some(&brush),
            std::slice::from_ref(&input),
            &eligible_lines,
            &[],
            fake_output,
        )?;

        assert_eq!(
            output.final_render.to_rgba8().get_pixel(20, 15).0,
            [30, 200, 40, 255]
        );
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(60, 15).0,
            [250, 0, 0, 255]
        );
        let sprite = output.blocks[0].sprite.to_rgba8();
        assert_eq!(sprite.get_pixel(10, 5).0[3], 0);
        assert_eq!(sprite.get_pixel(50, 5).0[3], 255);

        let all_text = dispatch_render_page(
            SourceTextPolicy::AllText,
            true,
            &base,
            &base,
            Some(&brush),
            std::slice::from_ref(&input),
            &[],
            &[],
            fake_output,
        )?;
        assert_eq!(
            all_text.blocks[0].sprite.to_rgba8().get_pixel(10, 5).0[3],
            255
        );
        assert_eq!(
            all_text.final_render.to_rgba8().get_pixel(20, 15).0,
            [250, 0, 0, 255]
        );
        Ok(())
    }

    #[test]
    fn han_only_renderer_restores_validated_english_pixels_from_source() -> Result<()> {
        let node_id = NodeId::new();
        let input = RenderBlockInput {
            node_id,
            transform: Transform {
                x: 50.0,
                y: 0.0,
                width: 50.0,
                height: 20.0,
                rotation_deg: 0.0,
            },
            translation: "Translated".to_string(),
            style: None,
            font_prediction: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
        };
        let source =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 20, Rgba([10, 20, 30, 255])));
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 20, Rgba([200, 200, 200, 255])));
        let english = EligibleTextLine {
            line_index: 0,
            text: "Peach".to_string(),
            region: koharu_ml::types::TextRegion {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 20.0,
                line_polygons: Some(vec![quad(0.0, 0.0, 40.0, 20.0)]),
                ..Default::default()
            },
        };
        let han = EligibleTextLine {
            line_index: 1,
            text: "蜜桃臀".to_string(),
            region: koharu_ml::types::TextRegion {
                x: 50.0,
                y: 0.0,
                width: 50.0,
                height: 20.0,
                line_polygons: Some(vec![quad(50.0, 0.0, 100.0, 20.0)]),
                ..Default::default()
            },
        };

        let output = dispatch_render_page(
            SourceTextPolicy::HanOnly,
            true,
            &source,
            &base,
            None,
            std::slice::from_ref(&input),
            &[(node_id, han)],
            &[(node_id, english)],
            || {
                Ok(RenderOutput {
                    final_render: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                        100,
                        20,
                        Rgba([250, 0, 0, 255]),
                    )),
                    blocks: vec![RenderedBlock {
                        node_id,
                        sprite: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                            50,
                            20,
                            Rgba([250, 0, 0, 255]),
                        )),
                        rendered_direction: TextDirection::Horizontal,
                        expanded_transform: None,
                    }],
                })
            },
        )?;

        assert_eq!(
            output.final_render.to_rgba8().get_pixel(20, 10).0,
            [10, 20, 30, 255]
        );
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(75, 10).0,
            [250, 0, 0, 255]
        );
        Ok(())
    }

    #[test]
    fn han_only_renderer_restores_skipped_text_nodes_from_source() -> Result<()> {
        let english_id = NodeId::new();
        let unsupported_id = NodeId::new();
        let han_id = NodeId::new();
        let outside_han_id = NodeId::new();
        let mut english = renderer_node(english_id, "SLENDER WAIST", None, None);
        english.transform = Transform {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
            rotation_deg: 0.0,
        };
        let mut unsupported = renderer_node(unsupported_id, "Peach蜜桃臀", None, None);
        unsupported.transform = Transform {
            x: 20.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
            rotation_deg: 0.0,
        };
        let mut han = renderer_node(han_id, "蜜桃臀", Some("Sweet butt"), None);
        han.transform = Transform {
            x: 40.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
            rotation_deg: 0.0,
        };
        let mut outside_han = renderer_node(outside_han_id, "中文", None, None);
        outside_han.transform = Transform {
            x: 60.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
            rotation_deg: 0.0,
        };
        let (scene, page) = renderer_scene(vec![english, unsupported, han, outside_han]);
        let (inputs, _, _, eligible_lines) =
            build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, Some(&[han_id]))?;
        assert!(inputs[0].lock_layout_box);
        let protected = protected_source_lines_for_page(&scene, page);
        let source =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([10, 20, 30, 255])));
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 100, Rgba([200, 200, 200, 255])));

        let output = dispatch_render_page(
            SourceTextPolicy::HanOnly,
            true,
            &source,
            &base,
            None,
            &inputs,
            &eligible_lines,
            &protected,
            || {
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![RenderedBlock {
                        node_id: han_id,
                        sprite: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                            20,
                            20,
                            Rgba([250, 0, 0, 255]),
                        )),
                        rendered_direction: TextDirection::Horizontal,
                        expanded_transform: None,
                    }],
                })
            },
        )?;

        let final_render = output.final_render.to_rgba8();
        assert_eq!(final_render.get_pixel(10, 10).0, [10, 20, 30, 255]);
        assert_eq!(final_render.get_pixel(30, 10).0, [10, 20, 30, 255]);
        assert_eq!(final_render.get_pixel(50, 10).0, [250, 0, 0, 255]);
        assert_eq!(final_render.get_pixel(70, 10).0, [10, 20, 30, 255]);
        Ok(())
    }

    #[test]
    fn han_only_renderer_does_not_allow_one_node_sprite_into_another_node_mask() -> Result<()> {
        let first_id = NodeId::new();
        let second_id = NodeId::new();
        let input = |node_id, x| RenderBlockInput {
            node_id,
            transform: Transform {
                x,
                y: 5.0,
                width: 20.0,
                height: 20.0,
                rotation_deg: 0.0,
            },
            translation: "Translated".to_string(),
            style: None,
            font_prediction: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
        };
        let inputs = vec![input(first_id, 10.0), input(second_id, 80.0)];
        let line = |x| EligibleTextLine {
            line_index: 0,
            text: "中文".to_string(),
            region: koharu_ml::types::TextRegion {
                x,
                y: 5.0,
                width: 20.0,
                height: 20.0,
                line_polygons: Some(vec![quad(x, 5.0, x + 20.0, 25.0)]),
                ..Default::default()
            },
        };
        let eligible_lines = vec![(first_id, line(10.0)), (second_id, line(80.0))];
        let base =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(120, 30, Rgba([10, 20, 30, 255])));

        let output = dispatch_render_page(
            SourceTextPolicy::HanOnly,
            true,
            &base,
            &base,
            None,
            &inputs,
            &eligible_lines,
            &[],
            || {
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![RenderedBlock {
                        node_id: first_id,
                        sprite: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                            90,
                            20,
                            Rgba([250, 0, 0, 255]),
                        )),
                        rendered_direction: TextDirection::Horizontal,
                        expanded_transform: Some(Transform {
                            x: 10.0,
                            y: 5.0,
                            width: 90.0,
                            height: 20.0,
                            rotation_deg: 0.0,
                        }),
                    }],
                })
            },
        )?;

        assert_eq!(output.blocks[0].sprite.to_rgba8().get_pixel(75, 10).0[3], 0);
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(85, 15).0,
            [10, 20, 30, 255]
        );
        Ok(())
    }

    #[test]
    fn han_only_renderer_uses_the_same_fractional_origin_for_clip_and_composite() -> Result<()> {
        let node_id = NodeId::new();
        let input = RenderBlockInput {
            node_id,
            transform: Transform {
                x: 10.6,
                y: 5.0,
                width: 1.0,
                height: 1.0,
                rotation_deg: 0.0,
            },
            translation: "Translated".to_string(),
            style: None,
            font_prediction: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
        };
        let eligible_lines = vec![(
            node_id,
            EligibleTextLine {
                line_index: 0,
                text: "中".to_string(),
                region: koharu_ml::types::TextRegion {
                    x: 11.0,
                    y: 5.0,
                    width: 1.0,
                    height: 1.0,
                    line_polygons: Some(vec![quad(11.0, 5.0, 12.0, 6.0)]),
                    ..Default::default()
                },
            },
        )];
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(20, 10, Rgba([10, 20, 30, 255])));

        let output = dispatch_render_page(
            SourceTextPolicy::HanOnly,
            true,
            &base,
            &base,
            None,
            std::slice::from_ref(&input),
            &eligible_lines,
            &[],
            || {
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![RenderedBlock {
                        node_id,
                        sprite: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                            1,
                            1,
                            Rgba([250, 0, 0, 255]),
                        )),
                        rendered_direction: TextDirection::Horizontal,
                        expanded_transform: None,
                    }],
                })
            },
        )?;

        assert_eq!(
            output.final_render.to_rgba8().get_pixel(10, 5).0,
            [10, 20, 30, 255]
        );
        Ok(())
    }

    #[test]
    fn legacy_brush_layer_still_composites_without_editor_ui() -> Result<()> {
        let base = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 255])));
        let mut brush = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 0]));
        brush.put_pixel(1, 1, Rgba([200, 10, 20, 255]));
        let brush = DynamicImage::ImageRgba8(brush);
        let calls = Cell::new(0);

        let output = dispatch_render_page(
            SourceTextPolicy::HanOnly,
            true,
            &base,
            &base,
            Some(&brush),
            &[],
            &[],
            &[],
            || {
                calls.set(calls.get() + 1);
                unreachable!("empty Han targets must not call the renderer")
            },
        )?;

        assert_eq!(calls.get(), 0);
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(0, 0).0,
            [10, 20, 30, 255]
        );
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(1, 1).0,
            [200, 10, 20, 255]
        );
        Ok(())
    }
}
