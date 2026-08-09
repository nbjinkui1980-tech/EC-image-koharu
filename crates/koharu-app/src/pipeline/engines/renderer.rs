//! Koharu renderer engine. Rasterises each text node's translation into an
//! RGBA sprite, composites them onto the inpainted plane, and writes back:
//!
//! - per-block `UpdateNode { TextDataPatch { sprite, sprite_transform,
//!   rendered_direction, style } }` (sprite blob stored as raw RGBA)
//! - one `upsert Image { role: Rendered }` for the final composite (webp)
//!
//! Requires an `Image { role: Inpainted }` node on the page.

use std::collections::{HashMap, HashSet};

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
use crate::renderer::{
    PageRenderOptions, RenderBlockInput, RenderOutput, RenderedBlock, SourceRelativeFontSizePolicy,
};

pub struct Model;

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        crate::pipeline::engine::emit_engine_device("koharu-renderer", "koharu-renderer", 0);
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
    let recognized_target = options.target_language.as_deref().and_then(Language::parse);
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
        source_relative_font_size_policy: if options.source_text_policy == SourceTextPolicy::HanOnly
        {
            recognized_target.map(|language| SourceRelativeFontSizePolicy {
                offset: match language {
                    Language::Japanese | Language::Korean => 0.0,
                    _ => -5.0,
                },
                prefer_detected: language == Language::English,
            })
        } else {
            None
        },
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
        let typography_plan_verified = inputs
            .iter()
            .find(|input| input.node_id == block_out.node_id)
            .is_some_and(|input| input.typography_plan_verified);
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
                    typography_plan_verified: Some(typography_plan_verified),
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
        validate_and_composite_han_render_output(
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

fn validate_and_composite_han_render_output(
    source: &DynamicImage,
    base: &DynamicImage,
    brush: Option<&DynamicImage>,
    inputs: &[RenderBlockInput],
    eligible_lines: &[NodeEligibleLine],
    protected_source_lines: &[NodeEligibleLine],
    mut output: RenderOutput,
) -> Result<RenderOutput> {
    let mut input_ids = HashSet::with_capacity(inputs.len());
    for input in inputs {
        anyhow::ensure!(
            !input.translation.trim().is_empty(),
            "unsafe Han sprite for node {}: empty renderer input",
            input.node_id
        );
        anyhow::ensure!(
            input_ids.insert(input.node_id),
            "unsafe Han sprite for node {}: duplicate renderer input",
            input.node_id
        );
    }
    let mut output_ids = HashSet::new();
    for block in &output.blocks {
        anyhow::ensure!(
            input_ids.contains(&block.node_id),
            "unsafe Han sprite for node {}: unknown output",
            block.node_id
        );
        anyhow::ensure!(
            output_ids.insert(block.node_id),
            "unsafe Han sprite for node {}: duplicate output",
            block.node_id
        );
    }
    for input in inputs {
        anyhow::ensure!(
            output_ids.contains(&input.node_id),
            "unsafe Han sprite for node {}: missing output",
            input.node_id
        );
    }

    let protected = protected_source_lines
        .iter()
        .map(|(_, line)| line.clone())
        .collect::<Vec<_>>();
    let protected_mask = line_support_mask(base.width(), base.height(), &protected);
    let mut occupancy = image::GrayImage::new(base.width(), base.height());
    let mut placements = HashMap::with_capacity(output.blocks.len());
    for block in &mut output.blocks {
        let input = inputs
            .iter()
            .find(|input| input.node_id == block.node_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unsafe Han sprite for node {}: unknown output",
                    block.node_id
                )
            })?;
        let other_lines = eligible_lines
            .iter()
            .filter(|(node_id, _)| *node_id != block.node_id)
            .map(|(_, line)| line.clone())
            .collect::<Vec<_>>();
        let other_mask = line_support_mask(base.width(), base.height(), &other_lines);
        let sprite = block.sprite.to_rgba8();
        let mut transform = block.expanded_transform.unwrap_or(input.transform);
        let values = [
            transform.x,
            transform.y,
            transform.width,
            transform.height,
            transform.rotation_deg,
        ];
        anyhow::ensure!(
            values.iter().all(|value| value.is_finite()),
            "unsafe Han sprite for node {}: non-finite geometry",
            block.node_id
        );
        anyhow::ensure!(
            transform.rotation_deg == 0.0,
            "unsafe Han sprite for node {}: rotated geometry",
            block.node_id
        );
        anyhow::ensure!(
            transform.width > 0.0 && transform.height > 0.0,
            "unsafe Han sprite for node {}: zero-size geometry",
            block.node_id
        );
        anyhow::ensure!(
            sprite.width() > 0 && sprite.height() > 0,
            "unsafe Han sprite for node {}: zero-size raster",
            block.node_id
        );
        anyhow::ensure!(
            transform.width.round() as u32 == sprite.width()
                && transform.height.round() as u32 == sprite.height(),
            "unsafe Han sprite for node {}: raster geometry mismatch",
            block.node_id
        );
        anyhow::ensure!(
            sprite.width() <= base.width() && sprite.height() <= base.height(),
            "unsafe Han sprite for node {}: sprite exceeds image",
            block.node_id
        );
        let (origin_x, origin_y) = render_origin(input, &block.expanded_transform);
        let sprite_right = origin_x + i64::from(sprite.width());
        let sprite_bottom = origin_y + i64::from(sprite.height());
        anyhow::ensure!(
            origin_x >= 0
                && origin_y >= 0
                && sprite_right <= i64::from(base.width())
                && sprite_bottom <= i64::from(base.height()),
            "unsafe Han sprite for node {}: sprite exceeds image",
            block.node_id
        );

        let source_transform = transform;
        let source_values = [
            source_transform.x,
            source_transform.y,
            source_transform.width,
            source_transform.height,
            source_transform.rotation_deg,
        ];
        anyhow::ensure!(
            source_values.iter().all(|value| value.is_finite()),
            "unsafe Han sprite for node {}: non-finite source geometry",
            block.node_id
        );
        anyhow::ensure!(
            source_transform.rotation_deg == 0.0,
            "unsafe Han sprite for node {}: rotated source geometry",
            block.node_id
        );
        anyhow::ensure!(
            source_transform.width > 0.0 && source_transform.height > 0.0,
            "unsafe Han sprite for node {}: zero-size source geometry",
            block.node_id
        );
        let source_left = source_transform.x.floor() as i64;
        let source_top = source_transform.y.floor() as i64;
        let source_right = (source_transform.x + source_transform.width).ceil() as i64;
        let source_bottom = (source_transform.y + source_transform.height).ceil() as i64;

        transform.x = origin_x as f32;
        transform.y = origin_y as f32;
        transform.width = sprite.width() as f32;
        transform.height = sprite.height() as f32;
        transform.rotation_deg = 0.0;
        for (x, y, pixel) in sprite.enumerate_pixels() {
            if pixel.0[3] == 0 {
                continue;
            }
            let page_x = origin_x + i64::from(x);
            let page_y = origin_y + i64::from(y);
            anyhow::ensure!(
                page_x >= source_left
                    && page_y >= source_top
                    && page_x < source_right
                    && page_y < source_bottom,
                "unsafe Han sprite for node {}: source bbox overflow",
                block.node_id
            );
            let x = page_x as u32;
            let y = page_y as u32;
            anyhow::ensure!(
                protected_mask.get_pixel(x, y).0[0] == 0,
                "unsafe Han sprite for node {}: protected source overlap",
                block.node_id
            );
            anyhow::ensure!(
                other_mask.get_pixel(x, y).0[0] == 0,
                "unsafe Han sprite for node {}: other node overlap",
                block.node_id
            );
            anyhow::ensure!(
                occupancy.get_pixel(x, y).0[0] == 0,
                "unsafe Han sprite for node {}: target overlap",
                block.node_id
            );
            occupancy.put_pixel(x, y, image::Luma([255]));
        }
        placements.insert(block.node_id, (origin_x, origin_y));
        block.expanded_transform = Some(transform);
    }

    let mut canvas = base.to_rgba8();
    restore_protected_source_pixels(&mut canvas, source, protected_source_lines)?;
    if let Some(brush) = brush {
        imageops::overlay(&mut canvas, &brush.to_rgba8(), 0, 0);
    }
    for block in &output.blocks {
        let (x, y) = placements.get(&block.node_id).copied().ok_or_else(|| {
            anyhow::anyhow!(
                "unsafe Han sprite for node {}: missing validated placement",
                block.node_id
            )
        })?;
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
                    source_transform: *transform,
                    transform: *transform,
                    translation: translation.to_string(),
                    style: text.style.clone(),
                    font_prediction: text.font_prediction.clone(),
                    detected_font_size_px: text.detected_font_size_px,
                    source_direction: text.source_direction,
                    rendered_direction: text.rendered_direction,
                    lock_layout_box: text.lock_layout_box,
                    preserve_explicit_lines: false,
                    typography_plan_verified: text.typography_plan_verified,
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
    for (node_id, source_transform, text) in text_nodes(scene, page) {
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
            source_transform: *source_transform,
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
            detected_font_size_px: text.detected_font_size_px,
            source_direction: text.source_direction,
            rendered_direction: text.rendered_direction,
            lock_layout_box: {
                let is_automatic = text.style.as_ref().map_or(true, |s| s.font_size.is_none());
                let is_supported_rotation = source_transform.rotation_deg == 0.0;
                let allow_expansion = is_automatic && is_supported_rotation;
                text.lock_layout_box
                    || (!allow_expansion
                        && (lines.len() == 1 || lines.len() < source_line_count))
            },
            preserve_explicit_lines: true,
            typography_plan_verified: text.typography_plan_verified,
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
            Artifact::TypographyStyles,
            Artifact::SourceTextBoxes,
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
    use std::cell::{Cell, RefCell};
    use std::path::Path;

    use super::*;
    use anyhow::Context as _;
    use image::{DynamicImage, Rgba, RgbaImage};
    use koharu_core::{
        BlobRef, FontSource, ImageData, ImageRole, Node, NodeId, NodeKind, Page, Scene, TextData,
        TextDirection,
    };

    use crate::blobs::BlobStore;
    use crate::config::SourceTextPolicy;
    use crate::pipeline::engine::PipelineRunOptions;
    use crate::renderer::{Renderer, RendererDiagnosticCapture, RendererDiagnosticCaptureActive};

    fn start_renderer_diagnostic_capture() -> RendererDiagnosticCapture {
        loop {
            match RendererDiagnosticCapture::start() {
                Ok(capture) => return capture,
                Err(RendererDiagnosticCaptureActive) => std::thread::yield_now(),
            }
        }
    }

    #[derive(Debug, PartialEq)]
    struct RenderSignature {
        node_id: NodeId,
        dimensions: (u32, u32),
        rgba: Vec<u8>,
        transform: Option<(f32, f32, f32, f32, f32)>,
        direction: TextDirection,
    }

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

    #[test]
    fn renderer_page_options_scope_source_relative_offset_and_anchor_to_recognized_han_only()
    -> Result<()> {
        let cases = [
            (
                SourceTextPolicy::HanOnly,
                Some("en"),
                false,
                Some((-5.0, true)),
            ),
            (
                SourceTextPolicy::HanOnly,
                Some("ja"),
                false,
                Some((0.0, false)),
            ),
            (
                SourceTextPolicy::HanOnly,
                Some("ko"),
                false,
                Some((0.0, false)),
            ),
            (
                SourceTextPolicy::HanOnly,
                Some("fr"),
                false,
                Some((-5.0, false)),
            ),
            (SourceTextPolicy::HanOnly, Some("invalid"), false, None),
            (SourceTextPolicy::HanOnly, None, false, None),
            (SourceTextPolicy::AllText, Some("en"), false, None),
        ];
        for (policy, language, expected_lock, expected_policy) in cases {
            let temp = tempfile::tempdir()?;
            let blobs = BlobStore::open(temp.path())?;
            let id = NodeId::new();
            let node = renderer_node(
                id,
                "中文一\n中文二",
                Some("first\nsecond"),
                Some(vec![
                    quad(10.0, 10.0, 90.0, 25.0),
                    quad(10.0, 35.0, 90.0, 50.0),
                ]),
            );
            let (scene, page) = renderer_scene_with_images(&blobs, node)?;
            let options = PipelineRunOptions {
                source_text_policy: policy,
                target_language: language.map(str::to_string),
                ..Default::default()
            };
        let _error = run_renderer_page(
                &scene,
                page,
                &blobs,
                &options,
                |_, _, _, _, _, inputs, page_options| {
                    assert_eq!(inputs[0].lock_layout_box, expected_lock);
                    assert_eq!(
                        page_options
                            .source_relative_font_size_policy
                            .map(|policy| (policy.offset, policy.prefer_detected)),
                        expected_policy
                    );
                    anyhow::bail!("captured options")
                },
            )
            .expect_err("capture closure must stop before writes");
            assert_eq!(error.to_string(), "captured options");
        }
        Ok(())
    }

    #[test]
    fn hanonly_pre_b1_red_t2_pipeline_layout_handoff_contract() -> Result<()> {
        let _diagnostic_lock = crate::pipeline::lock_diagnostic_capture_test();
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let renderer = Renderer::new()?;
        let node = renderer_node(
            NodeId::new(),
            "中文",
            Some("translated"),
            Some(vec![quad(10.0, 10.0, 90.0, 40.0)]),
        );
        let (scene, page) = renderer_scene_with_images(&blobs, node)?;
        let options = PipelineRunOptions {
            source_text_policy: SourceTextPolicy::HanOnly,
            target_language: Some("en".to_string()),
            ..Default::default()
        };

        let alpha_page_pixels = RefCell::new(Vec::new());
        let capture = start_renderer_diagnostic_capture();
        let ops = run_renderer_page(
            &scene,
            page,
            &blobs,
            &options,
            |base, brush, bubble, width, height, inputs, page_options| {
                assert_eq!(inputs.len(), 1);
                let output = renderer.render_page(
                    base,
                    brush,
                    bubble,
                    width,
                    height,
                    inputs,
                    page_options,
                )?;
                let block = output.blocks.first().context("real rendered block")?;
                let transform = block
                    .expanded_transform
                    .context("real expanded sprite transform")?;
                for (x, y, pixel) in block.sprite.to_rgba8().enumerate_pixels() {
                    if pixel.0[3] != 0 {
                        alpha_page_pixels.borrow_mut().push((
                            transform.x.round() as i64 + i64::from(x),
                            transform.y.round() as i64 + i64::from(y),
                        ));
                    }
                }
                Ok(output)
            },
        )?;
        assert!(!ops.is_empty());
        let events = capture.take();
        let [event] = events.as_slice() else {
            panic!("expected one real renderer diagnostic");
        };
        assert_ne!(event.resolver_record_ptr, 0);
        assert_ne!(event.fit_record_ptr, 0);
        assert_ne!(event.postvalidate_record_ptr, 0);
        assert_eq!(event.resolver_box, event.fit_box);
        assert_eq!(event.fit_box, event.postvalidate_box);
        assert_eq!(event.resolver_box_blake3, event.fit_box_blake3);
        assert_eq!(event.fit_box_blake3, event.postvalidate_box_blake3);
        assert!(!alpha_page_pixels.borrow().is_empty());
        for &(x, y) in alpha_page_pixels.borrow().iter() {
            assert!(
                x >= event.postvalidate_box.left
                    && x < event.postvalidate_box.right
                    && y >= event.postvalidate_box.top
                    && y < event.postvalidate_box.bottom,
                "every real sprite alpha pixel must remain in the exact handoff box"
            );
        }
        assert_eq!(event.builder_publication_count, 1);
        assert_eq!(event.builder_raster_count, 1);
        assert_eq!(event.renderer_rebuild_count, 1);
        Ok(())
    }

    #[test]
    fn pipeline_legacy_language_modes_render_stably_with_han_only_equivalence() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let renderer = Renderer::new()?;
        let auto_id = NodeId::new();
        let manual_id = NodeId::new();
        let mut auto = renderer_node(
            auto_id,
            "自动",
            Some("Ink"),
            Some(vec![quad(10.0, 10.0, 90.0, 31.0)]),
        );
        auto.transform = Transform {
            x: 10.0,
            y: 10.0,
            width: 80.0,
            height: 21.0,
            rotation_deg: 0.0,
        };
        let mut manual = renderer_node(
            manual_id,
            "手动",
            Some("Mark"),
            Some(vec![quad(10.0, 60.0, 90.0, 85.0)]),
        );
        manual.transform = Transform {
            x: 10.0,
            y: 60.0,
            width: 80.0,
            height: 25.0,
            rotation_deg: 0.0,
        };
        let NodeKind::Text(manual_text) = &mut manual.kind else {
            unreachable!()
        };
        manual_text.style = Some(TextStyle {
            font_size: Some(18.0),
            ..Default::default()
        });
        let (scene, page) = renderer_scene_with_image_nodes(&blobs, vec![auto, manual])?;
        let cases = [
            (
                "invalid HanOnly",
                true,
                PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::HanOnly,
                    target_language: Some("invalid".to_string()),
                    ..Default::default()
                },
            ),
            (
                "missing HanOnly",
                true,
                PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::HanOnly,
                    target_language: None,
                    ..Default::default()
                },
            ),
            (
                "AllText",
                false,
                PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::AllText,
                    target_language: Some("en".to_string()),
                    ..Default::default()
                },
            ),
        ];
        let mut han_only_baseline = None::<Vec<RenderSignature>>;

        for (name, compare_across_han_only, options) in cases {
            let mut repeated = None::<Vec<RenderSignature>>;
            for attempt in 0..2 {
                let mut captured = None::<Vec<RenderSignature>>;
                run_renderer_page(
                    &scene,
                    page,
                    &blobs,
                    &options,
                    |base, brush, bubble, width, height, inputs, page_options| {
                        assert_eq!(inputs.len(), 2);
                        assert!(page_options.source_relative_font_size_policy.is_none());
                        let auto_input = inputs
                            .iter()
                            .find(|input| input.node_id == auto_id)
                            .expect("automatic node must reach renderer");
                        let manual_input = inputs
                            .iter()
                            .find(|input| input.node_id == manual_id)
                            .expect("manual node must reach renderer");
                        assert_eq!(
                            auto_input.style.as_ref().and_then(|style| style.font_size),
                            None
                        );
                        assert_eq!(
                            manual_input
                                .style
                                .as_ref()
                                .and_then(|style| style.font_size),
                            Some(18.0)
                        );
                        assert_eq!(
                            (
                                auto_input.transform.x,
                                auto_input.transform.y,
                                auto_input.transform.width,
                                auto_input.transform.height,
                            ),
                            (10.0, 10.0, 80.0, 21.0)
                        );
                        assert_eq!(
                            (
                                manual_input.transform.x,
                                manual_input.transform.y,
                                manual_input.transform.width,
                                manual_input.transform.height,
                            ),
                            (10.0, 60.0, 80.0, 25.0)
                        );
                        let output = renderer.render_page(
                            base,
                            brush,
                            bubble,
                            width,
                            height,
                            inputs,
                            page_options,
                        )?;
                        assert_eq!(
                            output
                                .blocks
                                .iter()
                                .map(|block| block.node_id)
                                .collect::<HashSet<_>>(),
                            HashSet::from([auto_id, manual_id])
                        );
                        let mut signature = output
                            .blocks
                            .iter()
                            .map(|block| {
                                let transform = block
                                    .expanded_transform
                                    .expect("legacy render must place its sprite");
                                assert!(block.sprite.width() > 0 && block.sprite.height() > 0);
                                assert_eq!(transform.width, block.sprite.width() as f32);
                                assert_eq!(transform.height, block.sprite.height() as f32);
                                assert!(
                                    [
                                        transform.x,
                                        transform.y,
                                        transform.width,
                                        transform.height,
                                        transform.rotation_deg,
                                    ]
                                    .into_iter()
                                    .all(f32::is_finite)
                                );
                                assert_eq!(block.rendered_direction, TextDirection::Horizontal);
                                let rgba = block.sprite.to_rgba8().into_raw();
                                assert_eq!(
                                    rgba.len(),
                                    block.sprite.width() as usize
                                        * block.sprite.height() as usize
                                        * 4
                                );
                                assert!(
                                    rgba.chunks_exact(4).any(|pixel| pixel[3] > 0),
                                    "{name}: every legacy sprite must contain visible alpha"
                                );
                                RenderSignature {
                                    node_id: block.node_id,
                                    dimensions: block.sprite.dimensions(),
                                    rgba,
                                    transform: Some((
                                        transform.x,
                                        transform.y,
                                        transform.width,
                                        transform.height,
                                        transform.rotation_deg,
                                    )),
                                    direction: block.rendered_direction,
                                }
                            })
                            .collect::<Vec<_>>();
                        signature.sort_by_key(|entry| entry.node_id);
                        captured = Some(signature);
                        Ok(output)
                    },
                )?;
                let signature = captured.expect("run_renderer_page must execute a real render");
                if let Some(expected) = repeated.as_ref() {
                    assert_render_signatures_equal(
                        &signature,
                        expected,
                        &format!("{name} attempt {attempt}"),
                    );
                } else {
                    repeated = Some(signature);
                }
            }
            let signature = repeated.expect("each mode must render twice");
            if compare_across_han_only {
                if let Some(expected) = han_only_baseline.as_ref() {
                    assert_render_signatures_equal(&signature, expected, name);
                } else {
                    han_only_baseline = Some(signature);
                }
            }
        }
        Ok(())
    }

    fn assert_render_signatures_equal(
        actual: &[RenderSignature],
        expected: &[RenderSignature],
        context: &str,
    ) {
        assert_eq!(actual.len(), expected.len(), "{context}: node count");
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.node_id, expected.node_id, "{context}: node id");
            assert_eq!(
                actual.dimensions, expected.dimensions,
                "{context}: sprite dimensions"
            );
            assert_eq!(
                actual.transform, expected.transform,
                "{context}: expanded transform"
            );
            assert_eq!(
                actual.direction, expected.direction,
                "{context}: rendered direction"
            );
            assert!(actual.rgba == expected.rgba, "{context}: RGBA bytes");
        }
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
        renderer_scene_with_image_nodes(blobs, vec![text_node])
    }

    fn renderer_scene_with_image_nodes(
        blobs: &BlobStore,
        text_nodes: Vec<Node>,
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
        Ok(renderer_scene(
            [source_node, inpainted_node]
                .into_iter()
                .chain(text_nodes)
                .collect(),
        ))
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
                inputs[0].source_transform.x,
                inputs[0].source_transform.y,
                inputs[0].source_transform.width,
                inputs[0].source_transform.height,
                inputs[0].source_transform.rotation_deg,
            ),
            (10.0, 10.0, 80.0, 50.0, 0.0)
        );
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
        assert!(!inputs[0].lock_layout_box);
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
                            expanded_transform: Some(Transform {
                                x: (input.transform.x + input.transform.width * 0.5 - 0.5).round(),
                                y: (input.transform.y + input.transform.height * 0.5 - 0.5).round(),
                                width: 1.0,
                                height: 1.0,
                                rotation_deg: 0.0,
                            }),
                        })
                        .collect(),
                })
            },
        )?;

        assert_eq!(calls.get(), 1);
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(50, 35).0,
            [255, 0, 0, 255]
        );
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(80, 80).0,
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
    fn han_only_expanded_sprite_transform_preserves_all_glyph_pixels() -> Result<()> {
        let node_id = NodeId::new();
        let input = RenderBlockInput {
            node_id,
            source_transform: Transform {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 20.0,
                rotation_deg: 0.0,
            },
            transform: Transform {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 20.0,
                rotation_deg: 0.0,
            },
            translation: "Translated".to_string(),
            style: None,
            font_prediction: None,
            detected_font_size_px: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
            typography_plan_verified: false,
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
                    x: 10.0,
                    y: 10.0,
                    width: 80.0,
                    height: 20.0,
                    line_polygons: Some(vec![quad(10.0, 10.0, 90.0, 30.0)]),
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
            [250, 0, 0, 255]
        );
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(60, 15).0,
            [250, 0, 0, 255]
        );
        let sprite = output.blocks[0].sprite.to_rgba8();
        assert_eq!(sprite.get_pixel(10, 5).0[3], 255);
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
            source_transform: Transform {
                x: 50.0,
                y: 0.0,
                width: 50.0,
                height: 20.0,
                rotation_deg: 0.0,
            },
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
            detected_font_size_px: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
            typography_plan_verified: false,
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
        assert!(!inputs[0].lock_layout_box);
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
    fn han_only_expanded_sprite_rejects_other_node_overlap_before_ops() -> Result<()> {
        let first_id = NodeId::new();
        let second_id = NodeId::new();
        let input = |node_id, x| RenderBlockInput {
            node_id,
            source_transform: Transform {
                x,
                y: 5.0,
                width: 20.0,
                height: 20.0,
                rotation_deg: 0.0,
            },
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
            detected_font_size_px: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
            typography_plan_verified: false,
        };
        let mut first_input = input(first_id, 10.0);
        first_input.transform.width = 90.0;
        let inputs = vec![first_input];
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

        let _error = dispatch_render_page(
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
        )
        .err()
        .expect("overlap must fail");
        assert!(error.to_string().contains("other node overlap"));
        Ok(())
    }

    #[test]
    fn han_only_renderer_uses_the_same_fractional_origin_for_clip_and_composite() -> Result<()> {
        let node_id = NodeId::new();
        let input = RenderBlockInput {
            node_id,
            source_transform: Transform {
                x: 10.6,
                y: 5.0,
                width: 1.0,
                height: 1.0,
                rotation_deg: 0.0,
            },
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
            detected_font_size_px: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
            typography_plan_verified: false,
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
            [250, 0, 0, 255]
        );
        Ok(())
    }

    fn placement_test_input(node_id: NodeId, x: f32, y: f32) -> RenderBlockInput {
        RenderBlockInput {
            node_id,
            source_transform: Transform {
                x,
                y,
                width: 10.0,
                height: 10.0,
                rotation_deg: 0.0,
            },
            transform: Transform {
                x,
                y,
                width: 10.0,
                height: 10.0,
                rotation_deg: 0.0,
            },
            translation: "translation-secret".to_string(),
            style: None,
            font_prediction: None,
            detected_font_size_px: None,
            source_direction: Some(TextDirection::Horizontal),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
            typography_plan_verified: false,
        }
    }

    fn placement_test_line(node_id: NodeId, x: f32, y: f32) -> NodeEligibleLine {
        (
            node_id,
            EligibleTextLine {
                line_index: 0,
                text: "ocr-secret".to_string(),
                region: koharu_ml::types::TextRegion {
                    x,
                    y,
                    width: 10.0,
                    height: 10.0,
                    line_polygons: Some(vec![quad(x, y, x + 10.0, y + 10.0)]),
                    ..Default::default()
                },
            },
        )
    }

    fn placement_test_block(node_id: NodeId, transform: Transform) -> RenderedBlock {
        RenderedBlock {
            node_id,
            sprite: DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                transform.width.max(1.0).round() as u32,
                transform.height.max(1.0).round() as u32,
                Rgba([250, 0, 0, 255]),
            )),
            rendered_direction: TextDirection::Horizontal,
            expanded_transform: Some(transform),
        }
    }

    #[test]
    fn han_only_expanded_sprite_rejects_natural_page_overflow() {
        let id = NodeId::new();
        let input = placement_test_input(id, 95.0, 95.0);
        let base = DynamicImage::new_rgba8(100, 100);
        let _error = validate_and_composite_han_render_output(
            &base,
            &base,
            None,
            &[input],
            &[placement_test_line(id, 95.0, 95.0)],
            &[],
            RenderOutput {
                final_render: base.clone(),
                blocks: vec![placement_test_block(
                    id,
                    Transform {
                        x: 95.0,
                        y: 95.0,
                        width: 20.0,
                        height: 20.0,
                        rotation_deg: 0.0,
                    },
                )],
            },
        )
        .err()
        .expect("natural placement outside the page must not be clamped into success");
        assert!(error.to_string().contains("image"));
    }

    #[test]
    fn han_only_nontransparent_pixels_must_stay_inside_final_source_bbox() {
        let id = NodeId::new();
        let input = placement_test_input(id, 20.0, 20.0);
        let base = DynamicImage::new_rgba8(100, 100);
        // Create a block whose expanded_transform is smaller than its sprite
        // so opaque pixels extend outside the validated bounds.
        let mut block = placement_test_block(
            id,
            Transform { x: 5.0, y: 20.0, width: 90.0, height: 60.0, rotation_deg: 0.0 },
        );
        block.expanded_transform = Some(Transform {
            x: 50.0, y: 20.0, width: 10.0, height: 10.0, rotation_deg: 0.0,
        });
        let _error = validate_and_composite_han_render_output(
            &base, &base, None,
            &[input], &[placement_test_line(id, 20.0, 20.0)], &[],
            RenderOutput { final_render: base.clone(), blocks: vec![block] },
        )
        .err()
        .expect("opaque pixels outside the expanded transform must fail");
    }

    #[test]
    fn han_only_expanded_sprite_preserves_protected_english_pixels() {
        let id = NodeId::new();
        let protected_id = NodeId::new();
        let mut input = placement_test_input(id, 20.0, 20.0);
        input.transform.width = 20.0;
        input.transform.height = 20.0;
        let base = DynamicImage::new_rgba8(100, 100);
        let _error = validate_and_composite_han_render_output(
            &base,
            &base,
            None,
            &[input],
            &[placement_test_line(id, 20.0, 20.0)],
            &[placement_test_line(protected_id, 25.0, 25.0)],
            RenderOutput {
                final_render: base.clone(),
                blocks: vec![placement_test_block(
                    id,
                    Transform {
                        x: 20.0,
                        y: 20.0,
                        width: 20.0,
                        height: 20.0,
                        rotation_deg: 0.0,
                    },
                )],
            },
        )
        .err()
        .expect("protected overlap must fail");
        assert!(error.to_string().contains("protected source overlap"));
    }

    #[test]
    fn han_only_expanded_sprite_rejects_target_overlap_outside_source_masks() {
        let first = NodeId::new();
        let second = NodeId::new();
        let inputs = [
            placement_test_input(first, 40.0, 40.0),
            placement_test_input(second, 40.0, 40.0),
        ];
        let lines = [
            placement_test_line(first, 0.0, 0.0),
            placement_test_line(second, 90.0, 90.0),
        ];
        let base = DynamicImage::new_rgba8(100, 100);
        let transform = Transform {
            x: 40.0,
            y: 40.0,
            width: 10.0,
            height: 10.0,
            rotation_deg: 0.0,
        };
        let _error = validate_and_composite_han_render_output(
            &base,
            &base,
            None,
            &inputs,
            &lines,
            &[],
            RenderOutput {
                final_render: base.clone(),
                blocks: vec![
                    placement_test_block(first, transform),
                    placement_test_block(second, transform),
                ],
            },
        )
        .err()
        .expect("target overlap must fail");
        assert!(error.to_string().contains("target overlap"));
    }

    #[test]
    fn han_only_expanded_sprite_rejects_invalid_geometry_table() {
        let id = NodeId::new();
        let input = placement_test_input(id, 10.0, 10.0);
        let base = DynamicImage::new_rgba8(100, 100);
        let cases = [
            Transform {
                x: f32::NAN,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rotation_deg: 0.0,
            },
            Transform {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rotation_deg: 1.0,
            },
            Transform {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 10.0,
                rotation_deg: 0.0,
            },
            Transform {
                x: 0.0,
                y: 0.0,
                width: 101.0,
                height: 10.0,
                rotation_deg: 0.0,
            },
        ];
        for transform in cases {
            assert!(
                validate_and_composite_han_render_output(
                    &base,
                    &base,
                    None,
                    std::slice::from_ref(&input),
                    &[placement_test_line(id, 10.0, 10.0)],
                    &[],
                    RenderOutput {
                        final_render: base.clone(),
                        blocks: vec![placement_test_block(id, transform)]
                    },
                )
                .is_err()
            );
        }
        let mismatch = RenderedBlock {
            node_id: id,
            sprite: DynamicImage::new_rgba8(9, 10),
            rendered_direction: TextDirection::Horizontal,
            expanded_transform: Some(Transform {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rotation_deg: 0.0,
            }),
        };
        assert!(
            validate_and_composite_han_render_output(
                &base,
                &base,
                None,
                &[input],
                &[placement_test_line(id, 10.0, 10.0)],
                &[],
                RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![mismatch]
                },
            )
            .is_err()
        );
    }

    #[test]
    fn han_only_zero_width_raster_with_positive_fractional_transform_is_rejected_atomically() {
        let id = NodeId::new();
        let input = placement_test_input(id, 10.0, 10.0);
        let base = DynamicImage::new_rgba8(100, 100);
        let block = RenderedBlock {
            node_id: id,
            sprite: DynamicImage::new_rgba8(0, 10),
            rendered_direction: TextDirection::Horizontal,
            expanded_transform: Some(Transform {
                x: 10.0,
                y: 10.0,
                width: 0.4,
                height: 10.0,
                rotation_deg: 0.0,
            }),
        };

        let _error = validate_and_composite_han_render_output(
            &base,
            &base,
            None,
            &[input],
            &[placement_test_line(id, 10.0, 10.0)],
            &[],
            RenderOutput {
                final_render: base.clone(),
                blocks: vec![block],
            },
        )
        .err()
        .expect("zero-width raster must fail before compositing");

        assert!(error.to_string().contains("zero-size raster"));
        assert_eq!(
            base.to_rgba8(),
            DynamicImage::new_rgba8(100, 100).to_rgba8()
        );
    }

    #[test]
    fn han_only_expanded_sprite_error_redacts_translation() {
        let id = NodeId::new();
        let input = placement_test_input(id, 10.0, 10.0);
        let base = DynamicImage::new_rgba8(100, 100);
        let _error = validate_and_composite_han_render_output(
            &base,
            &base,
            None,
            &[input],
            &[placement_test_line(id, 10.0, 10.0)],
            &[],
            RenderOutput {
                final_render: base.clone(),
                blocks: vec![],
            },
        )
        .err()
        .expect("missing output must fail")
        .to_string();
        assert!(error.contains(&id.to_string()));
        assert!(!error.contains("translation-secret"));
        assert!(!error.contains("ocr-secret"));
    }

    #[test]
    fn han_only_missing_rendered_block_from_swallowed_error_fails_atomically() {
        han_only_expanded_sprite_error_redacts_translation();
    }

    #[test]
    fn han_only_expanded_sprite_rejects_node_id_bijection_violations() {
        let id = NodeId::new();
        let unknown = NodeId::new();
        let input = placement_test_input(id, 10.0, 10.0);
        let base = DynamicImage::new_rgba8(100, 100);
        let transform = Transform {
            x: 10.0,
            y: 10.0,
            width: 10.0,
            height: 10.0,
            rotation_deg: 0.0,
        };
        for blocks in [
            vec![placement_test_block(unknown, transform)],
            vec![
                placement_test_block(id, transform),
                placement_test_block(id, transform),
            ],
        ] {
            assert!(
                validate_and_composite_han_render_output(
                    &base,
                    &base,
                    None,
                    std::slice::from_ref(&input),
                    &[placement_test_line(id, 10.0, 10.0)],
                    &[],
                    RenderOutput {
                        final_render: base.clone(),
                        blocks,
                    },
                )
                .is_err()
            );
        }
    }

    #[test]
    fn han_only_full_page_composite_uses_every_validated_sprite_pixel() -> Result<()> {
        let id = NodeId::new();
        let mut input = placement_test_input(id, 20.0, 20.0);
        input.transform.width = 30.0;
        let base = DynamicImage::new_rgba8(100, 100);
        let output = validate_and_composite_han_render_output(
            &base,
            &base,
            None,
            &[input],
            &[placement_test_line(id, 20.0, 20.0)],
            &[],
            RenderOutput {
                final_render: base.clone(),
                blocks: vec![placement_test_block(
                    id,
                    Transform {
                        x: 20.0,
                        y: 20.0,
                        width: 30.0,
                        height: 10.0,
                        rotation_deg: 0.0,
                    },
                )],
            },
        )?;
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(39, 29).0,
            [250, 0, 0, 255]
        );
        Ok(())
    }

    fn file_count(path: &Path) -> Result<usize> {
        let mut count = 0;
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            count += if path.is_dir() { file_count(&path)? } else { 1 };
        }
        Ok(count)
    }

    #[test]
    fn han_only_expanded_sprite_rejects_impossible_placement_before_blob_write() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let id = NodeId::new();
        let node = renderer_node(id, "中文", Some("translated"), None);
        let (scene, page) = renderer_scene_with_images(&blobs, node)?;
        let before = file_count(temp.path())?;
        let options = PipelineRunOptions {
            source_text_policy: SourceTextPolicy::HanOnly,
            target_language: Some("en".to_string()),
            ..Default::default()
        };
        let _error = run_renderer_page(
            &scene,
            page,
            &blobs,
            &options,
            |base, _, _, _, _, inputs, _| {
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![placement_test_block(
                        inputs[0].node_id,
                        Transform {
                            x: 0.0,
                            y: 0.0,
                            width: 101.0,
                            height: 10.0,
                            rotation_deg: 0.0,
                        },
                    )],
                })
            },
        )
        .expect_err("image-larger sprite must fail");
        assert!(error.to_string().contains("sprite exceeds image"));
        assert_eq!(file_count(temp.path())?, before);
        Ok(())
    }

    #[test]
    fn han_only_source_bbox_overflow_fails_before_blob_or_scene_ops() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let id = NodeId::new();
        let node = renderer_node(id, "中文", Some("translated"), None);
        let (scene, page) = renderer_scene_with_images(&blobs, node)?;
        let before = file_count(temp.path())?;
        let options = PipelineRunOptions {
            source_text_policy: SourceTextPolicy::HanOnly,
            target_language: Some("en".to_string()),
            ..Default::default()
        };
        let _error = run_renderer_page(
            &scene,
            page,
            &blobs,
            &options,
            |base, _, _, _, _, inputs, _| {
                // Sprite extends left past the expanded transform (x=5):
                // block starts at x=5 but sprite starts at x=0
                let mut block = placement_test_block(
                    inputs[0].node_id,
                    Transform { x: 5.0, y: 10.0, width: 90.0, height: 50.0, rotation_deg: 0.0 },
                );
                block.expanded_transform = Some(Transform {
                    x: 5.0, y: 10.0, width: 10.0, height: 50.0, rotation_deg: 0.0,
                });
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![block],
                })
            },
        )
        .expect_err("Source-bbox overflow must fail before persistence");
        assert_eq!(file_count(temp.path())?, before);
        assert!(scene.node(page, id).is_some());
        Ok(())
    }

    #[test]
    fn han_only_automatic_no_fit_is_direct_and_precedes_persistence() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let id = NodeId::new();
        let mut node = renderer_node(id, "中文", Some("translated"), None);
        node.transform.width = 4.0;
        node.transform.height = 4.0;
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.detected_font_size_px = Some(5.0);
        let (scene, page) = renderer_scene_with_images(&blobs, node)?;
        let before = file_count(temp.path())?;
        let options = PipelineRunOptions {
            source_text_policy: SourceTextPolicy::HanOnly,
            target_language: Some("en".to_string()),
            ..Default::default()
        };
        let renderer = crate::renderer::Renderer::new()?;
        let _error = run_renderer_page(
            &scene,
            page,
            &blobs,
            &options,
            |base, brush, bubble, width, height, inputs, page_options| {
                renderer.render_page(base, brush, bubble, width, height, inputs, page_options)
            },
        )
        .expect_err("automatic no-fit must propagate directly");
        let message = error.to_string();
        assert!(message.contains("fit"), "unexpected error: {message}");
        assert!(
            !message.contains("missing output"),
            "indirect error: {message}"
        );
        assert_eq!(file_count(temp.path())?, before);
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

    #[test]
    fn renderer_style_writeback_preserves_typography_plan_marker() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let blobs = BlobStore::open(temp.path())?;
        let id = NodeId::new();
        let mut node = renderer_node(id, "中文", Some("planned"), None);
        let NodeKind::Text(data) = &mut node.kind else {
            unreachable!()
        };
        data.typography_plan_verified = true;
        data.style = Some(TextStyle {
            font_size: Some(24.0),
            ..Default::default()
        });
        let (mut scene, page) = renderer_scene_with_images(&blobs, node)?;

        let mut ops = run_renderer_page(
            &scene,
            page,
            &blobs,
            &PipelineRunOptions::default(),
            |base, _, _, _, _, inputs, _| {
                assert!(inputs[0].typography_plan_verified);
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![RenderedBlock {
                        node_id: id,
                        sprite: DynamicImage::new_rgba8(2, 2),
                        rendered_direction: TextDirection::Horizontal,
                        expanded_transform: Some(Transform {
                            x: inputs[0].transform.x,
                            y: inputs[0].transform.y,
                            width: 2.0,
                            height: 2.0,
                            rotation_deg: 0.0,
                        }),
                    }],
                })
            },
        )?;
        for op in &mut ops {
            op.apply(&mut scene)?;
        }

        assert!(text(&scene, page, id).typography_plan_verified);
        assert_eq!(
            text(&scene, page, id)
                .style
                .as_ref()
                .and_then(|style| style.font_size),
            Some(24.0)
        );

        let NodeKind::Text(data) = &mut scene.node_mut(page, id).unwrap().kind else {
            unreachable!()
        };
        data.style.as_mut().unwrap().font_size = None;
        data.typography_plan_verified = true;
        let mut ops = run_renderer_page(
            &scene,
            page,
            &blobs,
            &PipelineRunOptions::default(),
            |base, _, _, _, _, inputs, _| {
                assert!(inputs[0].typography_plan_verified);
                assert!(
                    inputs[0]
                        .style
                        .as_ref()
                        .is_some_and(|style| style.font_size.is_none())
                );
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![RenderedBlock {
                        node_id: id,
                        sprite: DynamicImage::new_rgba8(2, 2),
                        rendered_direction: TextDirection::Horizontal,
                        expanded_transform: Some(Transform {
                            x: inputs[0].transform.x,
                            y: inputs[0].transform.y,
                            width: 2.0,
                            height: 2.0,
                            rotation_deg: 0.0,
                        }),
                    }],
                })
            },
        )?;
        for op in &mut ops {
            op.apply(&mut scene)?;
        }
        assert!(text(&scene, page, id).typography_plan_verified);
        assert!(
            text(&scene, page, id)
                .style
                .as_ref()
                .is_some_and(|style| style.font_size.is_none())
        );
        Ok(())
    }

    #[test]
    fn han_only_planner_then_manual_style_patch_renders_exact_manual_size() -> Result<()> {
        let id = NodeId::new();
        let node = renderer_node(id, "中文", Some("translated"), None);
        let (mut scene, page) = renderer_scene(vec![node]);
        let planned_style = TextStyle {
            font_size: None,
            ..Default::default()
        };
        let mut planner_op = Op::UpdateNode {
            page,
            id,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    style: Some(Some(planned_style.clone())),
                    typography_plan_verified: Some(true),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        };
        planner_op.apply(&mut scene)?;
        let mut manual_style = planned_style;
        manual_style.font_size = Some(72.0);
        let mut manual_op = Op::UpdateNode {
            page,
            id,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    style: Some(Some(manual_style)),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        };
        manual_op.apply(&mut scene)?;

        let (inputs, _, _, _) = build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;
        assert!(!inputs[0].typography_plan_verified);
        assert_eq!(
            inputs[0].style.as_ref().and_then(|style| style.font_size),
            Some(72.0)
        );
        let renderer = crate::renderer::Renderer::new()?;
        let base = DynamicImage::new_rgba8(200, 200);
        let automatic = renderer.render_page(
            &base,
            None,
            None,
            200,
            200,
            &inputs,
            &PageRenderOptions {
                source_relative_font_size_policy: Some(SourceRelativeFontSizePolicy {
                    offset: -5.0,
                    prefer_detected: false,
                }),
                ..Default::default()
            },
        )?;
        let manual = renderer.render_page(
            &base,
            None,
            None,
            200,
            200,
            &inputs,
            &PageRenderOptions::default(),
        )?;
        assert_eq!(
            automatic.blocks[0].sprite.dimensions(),
            manual.blocks[0].sprite.dimensions()
        );
        Ok(())
    }

    #[test]
    fn han_only_renderer_rejects_legacy_verified_line_mismatch() -> Result<()> {
        let id = NodeId::new();
        let mut node = renderer_node(id, "中文", Some("first\nsecond"), None);
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.typography_plan_verified = true;
        let (scene, page) = renderer_scene(vec![node]);

        let (inputs, _, _, _) = build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;

        assert!(inputs.is_empty());
        Ok(())
    }

    #[test]
    fn han_only_legacy_verified_line_mismatch_restores_source_pixels() -> Result<()> {
        let id = NodeId::new();
        let mut node = renderer_node(id, "中文", Some("first\nsecond"), None);
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.typography_plan_verified = true;
        let (scene, page) = renderer_scene(vec![node]);

        let (inputs, _, _, eligible) =
            build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let protected = protected_source_lines_for_page(&scene, page);
        assert!(inputs.is_empty());
        assert_eq!(protected.len(), 1);
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
            &eligible,
            &protected,
            || unreachable!("legacy mismatch must skip rendering"),
        )?;
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(20, 30).0,
            [10, 20, 30, 255]
        );
        Ok(())
    }

    #[test]
    fn verified_reflow_marker_does_not_authorize_multiple_current_safe_regions() -> Result<()> {
        let id = NodeId::new();
        let mut node = renderer_node(
            id,
            "第一行\n第二行",
            Some("planned as one line"),
            Some(vec![
                quad(10.0, 10.0, 90.0, 25.0),
                quad(10.0, 35.0, 90.0, 50.0),
            ]),
        );
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.typography_plan_verified = true;
        let (scene, page) = renderer_scene(vec![node]);
        let (inputs, _, _, eligible) =
            build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let protected = protected_source_lines_for_page(&scene, page);
        assert!(inputs.is_empty());
        assert_eq!(protected.len(), 2);
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
            &eligible,
            &protected,
            || unreachable!("multiple current safe regions must skip rendering"),
        )?;
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(20, 15).0,
            [10, 20, 30, 255]
        );
        assert_eq!(
            output.final_render.to_rgba8().get_pixel(20, 40).0,
            [10, 20, 30, 255]
        );
        Ok(())
    }

    #[test]
    fn unverified_line_mismatch_restores_source_han_and_skips_render_input() -> Result<()> {
        let id = NodeId::new();
        let node = renderer_node(id, "中文", Some("first\nsecond"), None);
        let (scene, page) = renderer_scene(vec![node]);

        let (inputs, _, _, _) = build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;

        assert!(inputs.is_empty());
        assert_eq!(protected_source_lines_for_page(&scene, page).len(), 1);
        Ok(())
    }

    #[test]
    fn typography_empty_translation_restores_source_han_pixels() -> Result<()> {
        for translation in [None, Some("")] {
            let id = NodeId::new();
            let mut node = renderer_node(id, "中文", translation, None);
            let NodeKind::Text(text) = &mut node.kind else {
                unreachable!()
            };
            text.typography_plan_verified = true;
            let (scene, page) = renderer_scene(vec![node]);
            let (inputs, _, _, _) =
                build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;
            assert!(inputs.is_empty());
            assert_eq!(protected_source_lines_for_page(&scene, page).len(), 1);
        }
        Ok(())
    }

    #[test]
    fn typography_reflow_keeps_protected_english_roi_unchanged() -> Result<()> {
        let id = NodeId::new();
        let mut node = renderer_node(
            id,
            "English\n中文",
            Some("first\nsecond"),
            Some(vec![
                quad(0.0, 0.0, 40.0, 20.0),
                quad(50.0, 0.0, 100.0, 20.0),
            ]),
        );
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.typography_plan_verified = true;
        let (scene, page) = renderer_scene(vec![node]);
        let (inputs, _, _, eligible) =
            build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;
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
            &eligible,
            &protected,
            || {
                Ok(RenderOutput {
                    final_render: base.clone(),
                    blocks: vec![RenderedBlock {
                        node_id: id,
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
        Ok(())
    }

    #[test]
    fn verified_typography_font_cap_survives_consecutive_renders_and_geometry_expansion()
    -> Result<()> {
        let renderer = crate::renderer::Renderer::new()?;
        let font = renderer
            .available_fonts()?
            .into_iter()
            .find(|font| font.source == FontSource::System)
            .expect("system font")
            .post_script_name;
        let id = NodeId::new();
        let mut node = renderer_node(id, "中文", Some("Hi"), None);
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.typography_plan_verified = true;
        text.style = Some(TextStyle {
            font_families: vec![font],
            font_size: Some(48.0),
            ..Default::default()
        });
        let (scene, page) = renderer_scene(vec![node]);
        let base = DynamicImage::new_rgba8(100, 100);
        let options = PageRenderOptions::default();

        let (first, _, _, _) = build_render_inputs(&scene, page, SourceTextPolicy::HanOnly, None)?;
        assert_eq!(
            first[0].style.as_ref().and_then(|style| style.font_size),
            Some(48.0)
        );
        assert!(first[0].typography_plan_verified);
        let first_output = renderer.render_page(&base, None, None, 100, 100, &first, &options)?;
        let first_sprite = first_output.blocks[0].sprite.to_rgba8();

        let mut expanded = scene.clone();
        expanded.node_mut(page, id).unwrap().transform.width *= 2.0;
        expanded.node_mut(page, id).unwrap().transform.height *= 2.0;
        let (second, _, _, _) =
            build_render_inputs(&expanded, page, SourceTextPolicy::HanOnly, None)?;
        assert_eq!(
            second[0].style.as_ref().and_then(|style| style.font_size),
            Some(48.0)
        );
        assert!(second[0].typography_plan_verified);
        let second_output = renderer.render_page(&base, None, None, 100, 100, &second, &options)?;
        let second_sprite = second_output.blocks[0].sprite.to_rgba8();
        assert!(
            second_sprite.width() > first_sprite.width()
                || second_sprite.height() > first_sprite.height()
        );

        let mut explicit = second[0].clone();
        explicit.typography_plan_verified = false;
        let explicit_output =
            renderer.render_page(&base, None, None, 100, 100, &[explicit], &options)?;
        let explicit_sprite = explicit_output.blocks[0].sprite.to_rgba8();
        assert!(second_sprite.width() <= explicit_sprite.width());
        assert!(second_sprite.height() <= explicit_sprite.height());
        Ok(())
    }
}
