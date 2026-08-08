//! Koharu text renderer.
//!
//! Owns the font book, symbol fallbacks, and Google Fonts service. Exposes
//! [`Renderer::render_page`], which rasterises each text block's translation
//! into an RGBA sprite and composites them onto the inpainted plane.
//!
//! Pure output: the pipeline engine ([`crate::pipeline::engines::renderer`])
//! takes a `RenderOutput` and translates sprites + final composite into ops.

use std::{
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use image::{DynamicImage, GrayImage, RgbaImage, imageops};
use koharu_core::{
    FontFaceInfo, FontPrediction, FontSource, NodeId, TextDirection, TextShaderEffect,
    TextStrokeStyle, TextStyle, Transform,
};

use koharu_renderer::{
    TextAlign as RendererTextAlign, TextShaderEffect as RendererEffect,
    font::{FaceInfo, Font, FontBook},
    layout::{LayoutRun, TextLayout, WritingMode},
    renderer::{RasterOptions, RenderOptions, RenderStrokeOptions, TinySkiaRenderer},
    text::{
        latin::{BubbleIndex, LayoutBox},
        script::{font_families_for_text, writing_mode_for_block},
    },
    types::{RenderBlock, TextDirection as RendererTextDirection},
};

use crate::google_fonts::GoogleFontService;

#[cfg(test)]
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::{sync::OnceLock, thread::ThreadId};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RendererSourceSizeBranch {
    Detected,
    Predicted,
    GeometryFallback,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RendererLayoutBoxBranch {
    Seed,
    LockedSeed,
    UniqueBubble,
    SharedBubble,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RendererFillBranch {
    Explicit,
    Predicted,
    DefaultBlack,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RendererStrokeBranch {
    BlockDisabled,
    BlockExplicit,
    GlobalDisabled,
    GlobalExplicit,
    PredictedWidth,
    PredictedNoStroke,
    AutomaticDefault,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RendererFieldOutcome {
    ManualOverride,
    Prediction,
    TransientPlanner,
    Default,
    SourceColorContract,
    IgnoredByPolicy,
    Unsupported,
}

#[cfg(test)]
struct SourceSizeResolution {
    candidate: f32,
    valid_detected_size: Option<f32>,
    valid_predicted_size: Option<f32>,
    branch: RendererSourceSizeBranch,
}

#[cfg(test)]
type SourceSizeResolutionOutput = SourceSizeResolution;
#[cfg(not(test))]
type SourceSizeResolutionOutput = f32;

#[cfg(test)]
struct FillResolution {
    color: [u8; 4],
    branch: RendererFillBranch,
}

#[cfg(test)]
type FillResolutionOutput = FillResolution;
#[cfg(not(test))]
type FillResolutionOutput = [u8; 4];

#[cfg(test)]
struct StrokeResolution {
    stroke: Option<RenderStrokeOptions>,
    branch: RendererStrokeBranch,
}

#[cfg(test)]
type StrokeResolutionOutput = StrokeResolution;
#[cfg(not(test))]
type StrokeResolutionOutput = Option<RenderStrokeOptions>;

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RendererAlphaBbox {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RendererHalfOpenBox {
    pub(crate) left: i64,
    pub(crate) top: i64,
    pub(crate) right: i64,
    pub(crate) bottom: i64,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RendererDiagnosticEvent {
    pub(crate) node_id: NodeId,
    pub(crate) source_geometry_estimate: f32,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) valid_detected_size: Option<f32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) valid_predicted_size: Option<f32>,
    pub(crate) source_size_branch: RendererSourceSizeBranch,
    pub(crate) policy_offset: f32,
    pub(crate) candidate_size: f32,
    pub(crate) auto_max: f32,
    pub(crate) cap: f32,
    pub(crate) resolved_layout_width: f32,
    pub(crate) resolved_layout_height: f32,
    pub(crate) layout_box_branch: RendererLayoutBoxBranch,
    pub(crate) tight_layout_width: f32,
    pub(crate) tight_layout_height: f32,
    pub(crate) independent_size: f32,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) group_size: Option<f32>,
    pub(crate) final_size: f32,
    pub(crate) rotation_deg: f32,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) predicted_fill_rgb: Option<[u8; 3]>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) predicted_stroke_rgb: Option<[u8; 3]>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) predicted_stroke_width: Option<f32>,
    pub(crate) resolved_fill_rgba: [u8; 4],
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) resolved_stroke_rgba: Option<[u8; 4]>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) resolved_stroke_width: Option<f32>,
    pub(crate) fill_branch: RendererFillBranch,
    pub(crate) stroke_branch: RendererStrokeBranch,
    pub(crate) font_outcome: RendererFieldOutcome,
    pub(crate) fill_outcome: RendererFieldOutcome,
    pub(crate) stroke_outcome: RendererFieldOutcome,
    pub(crate) final_font_size_px: u32,
    pub(crate) resolver_record_ptr: usize,
    pub(crate) fit_record_ptr: usize,
    pub(crate) postvalidate_record_ptr: usize,
    pub(crate) resolver_box: RendererHalfOpenBox,
    pub(crate) fit_box: RendererHalfOpenBox,
    pub(crate) postvalidate_box: RendererHalfOpenBox,
    pub(crate) resolver_box_blake3: String,
    pub(crate) fit_box_blake3: String,
    pub(crate) postvalidate_box_blake3: String,
    pub(crate) builder_publication_count: u32,
    pub(crate) builder_raster_count: u32,
    pub(crate) renderer_rebuild_count: u32,
    pub(crate) sprite_width: u32,
    pub(crate) sprite_height: u32,
    pub(crate) sprite_rgba_blake3: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) alpha_bbox: Option<RendererAlphaBbox>,
    pub(crate) alpha_nonzero_pixels: u64,
    pub(crate) alpha_blake3: String,
}

#[cfg(test)]
fn deserialize_required_option<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

#[cfg(test)]
type RendererDiagnosticEvents = Arc<Mutex<Vec<RendererDiagnosticEvent>>>;

#[cfg(test)]
#[derive(Clone, Debug)]
struct RendererLayoutDiagnosticRecord {
    node_id: NodeId,
    resolver_record_ptr: usize,
    fit_record_ptr: Option<usize>,
    postvalidate_record_ptr: Option<usize>,
    resolver_box: RendererHalfOpenBox,
    fit_box: Option<RendererHalfOpenBox>,
    postvalidate_box: Option<RendererHalfOpenBox>,
    builder_publication_count: u32,
    builder_raster_count: u32,
    renderer_rebuild_count: u32,
}

#[cfg(test)]
type RendererLayoutDiagnosticRecords = Arc<Mutex<Vec<RendererLayoutDiagnosticRecord>>>;

#[cfg(test)]
struct ActiveRendererDiagnosticSink {
    owner: ThreadId,
    events: RendererDiagnosticEvents,
    layout_records: RendererLayoutDiagnosticRecords,
}

#[cfg(test)]
static RENDERER_DIAGNOSTIC_SINK: OnceLock<Mutex<Option<ActiveRendererDiagnosticSink>>> =
    OnceLock::new();

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererDiagnosticCaptureActive;

#[cfg(test)]
pub(crate) struct RendererDiagnosticCapture {
    owner: ThreadId,
    events: RendererDiagnosticEvents,
    layout_records: RendererLayoutDiagnosticRecords,
}

#[cfg(test)]
impl RendererDiagnosticCapture {
    pub(crate) fn start() -> std::result::Result<Self, RendererDiagnosticCaptureActive> {
        let owner = std::thread::current().id();
        let events = Arc::new(Mutex::new(Vec::new()));
        let layout_records = Arc::new(Mutex::new(Vec::new()));
        let mut active = RENDERER_DIAGNOSTIC_SINK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.is_some() {
            return Err(RendererDiagnosticCaptureActive);
        }
        *active = Some(ActiveRendererDiagnosticSink {
            owner,
            events: events.clone(),
            layout_records: layout_records.clone(),
        });
        Ok(Self {
            owner,
            events,
            layout_records,
        })
    }

    pub(crate) fn take(&self) -> Vec<RendererDiagnosticEvent> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

#[cfg(test)]
impl Drop for RendererDiagnosticCapture {
    fn drop(&mut self) {
        let mut active = RENDERER_DIAGNOSTIC_SINK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.as_ref().is_some_and(|sink| {
            sink.owner == self.owner
                && Arc::ptr_eq(&sink.events, &self.events)
                && Arc::ptr_eq(&sink.layout_records, &self.layout_records)
        }) {
            *active = None;
        }
    }
}

#[cfg(test)]
fn record_renderer_diagnostic(event: RendererDiagnosticEvent) {
    let owner = std::thread::current().id();
    let sink = RENDERER_DIAGNOSTIC_SINK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|sink| sink.owner == owner)
        .map(|sink| sink.events.clone());
    if let Some(sink) = sink {
        sink.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

#[cfg(test)]
fn active_renderer_layout_records() -> Option<RendererLayoutDiagnosticRecords> {
    let owner = std::thread::current().id();
    RENDERER_DIAGNOSTIC_SINK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|sink| sink.owner == owner)
        .map(|sink| sink.layout_records.clone())
}

#[cfg(test)]
fn renderer_half_open_box(layout_box: LayoutBox) -> RendererHalfOpenBox {
    RendererHalfOpenBox {
        left: layout_box.x.floor() as i64,
        top: layout_box.y.floor() as i64,
        right: (layout_box.x + layout_box.width).ceil() as i64,
        bottom: (layout_box.y + layout_box.height).ceil() as i64,
    }
}

#[cfg(test)]
fn renderer_box_digest(layout_box: RendererHalfOpenBox) -> String {
    let mut bytes = Vec::with_capacity(32);
    for value in [
        layout_box.left,
        layout_box.top,
        layout_box.right,
        layout_box.bottom,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    blake3::hash(&bytes).to_hex().to_string()
}

#[cfg(test)]
fn record_renderer_layout_resolver(node_id: NodeId, layout_box: &LayoutBox) {
    let Some(records) = active_renderer_layout_records() else {
        return;
    };
    records
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(RendererLayoutDiagnosticRecord {
            node_id,
            resolver_record_ptr: std::ptr::from_ref(layout_box) as usize,
            fit_record_ptr: None,
            postvalidate_record_ptr: None,
            resolver_box: renderer_half_open_box(*layout_box),
            fit_box: None,
            postvalidate_box: None,
            builder_publication_count: 0,
            builder_raster_count: 0,
            renderer_rebuild_count: 0,
        });
}

#[cfg(test)]
fn record_renderer_layout_stage(
    node_id: NodeId,
    layout_box: &LayoutBox,
    update: impl FnOnce(&mut RendererLayoutDiagnosticRecord, usize, RendererHalfOpenBox),
) {
    let Some(records) = active_renderer_layout_records() else {
        return;
    };
    let mut records = records
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(record) = records.iter_mut().find(|record| record.node_id == node_id) {
        update(
            record,
            std::ptr::from_ref(layout_box) as usize,
            renderer_half_open_box(*layout_box),
        );
    }
}

#[cfg(test)]
fn record_renderer_layout_count(
    node_id: NodeId,
    update: impl FnOnce(&mut RendererLayoutDiagnosticRecord),
) {
    let Some(records) = active_renderer_layout_records() else {
        return;
    };
    let mut records = records
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(record) = records.iter_mut().find(|record| record.node_id == node_id) {
        update(record);
    }
}

#[cfg(test)]
type RendererLayoutDiagnostic = (
    usize,
    usize,
    usize,
    RendererHalfOpenBox,
    RendererHalfOpenBox,
    RendererHalfOpenBox,
    u32,
    u32,
    u32,
);

#[cfg(test)]
fn renderer_layout_diagnostic(node_id: NodeId) -> Option<RendererLayoutDiagnostic> {
    let records = active_renderer_layout_records()?;
    let records = records
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let record = records.iter().find(|record| record.node_id == node_id)?;
    Some((
        record.resolver_record_ptr,
        record.fit_record_ptr?,
        record.postvalidate_record_ptr?,
        record.resolver_box,
        record.fit_box?,
        record.postvalidate_box?,
        record.builder_publication_count,
        record.builder_raster_count,
        record.renderer_rebuild_count,
    ))
}

// ---------------------------------------------------------------------------
// Inputs / outputs
// ---------------------------------------------------------------------------

/// Per-block input (immutable snapshot of a scene text node).
#[derive(Debug, Clone)]
pub struct RenderBlockInput {
    pub node_id: NodeId,
    /// Original Scene geometry used only for transient Source-relative grouping.
    pub source_transform: Transform,
    pub transform: Transform,
    pub translation: String,
    pub style: Option<TextStyle>,
    pub font_prediction: Option<FontPrediction>,
    pub detected_font_size_px: Option<f32>,
    pub source_direction: Option<TextDirection>,
    pub rendered_direction: Option<TextDirection>,
    pub lock_layout_box: bool,
    pub preserve_explicit_lines: bool,
    pub typography_plan_verified: bool,
}

/// Document-level render options (shared across all blocks).
#[derive(Debug, Clone, Default)]
pub struct PageRenderOptions {
    pub shader_effect: TextShaderEffect,
    pub shader_stroke: Option<TextStrokeStyle>,
    pub document_font: Option<String>,
    pub target_language: Option<String>,
    pub(crate) source_relative_font_size_policy: Option<SourceRelativeFontSizePolicy>,
    pub raster: RasterOptions,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceRelativeFontSizePolicy {
    pub offset: f32,
    pub prefer_detected: bool,
}

/// Per-block sprite output. `transform` becomes `TextData.sprite_transform`
/// when the renderer expanded the layout beyond the original bubble.
pub struct RenderedBlock {
    pub node_id: NodeId,
    pub sprite: DynamicImage,
    pub rendered_direction: TextDirection,
    pub expanded_transform: Option<Transform>,
}

/// Result of rendering a whole page.
pub struct RenderOutput {
    pub final_render: DynamicImage,
    pub blocks: Vec<RenderedBlock>,
}

// ---------------------------------------------------------------------------
// Renderer
// ---------------------------------------------------------------------------

pub struct Renderer {
    fontbook: Arc<Mutex<FontBook>>,
    renderer: TinySkiaRenderer,
    symbol_fallbacks: Vec<Font>,
    pub google_fonts: Arc<GoogleFontService>,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        let mut fontbook = FontBook::new();
        let symbol_fallbacks = load_symbol_fallbacks(&mut fontbook);
        let app_data_root = koharu_runtime::default_app_data_root();
        let google_fonts = Arc::new(
            GoogleFontService::new(&app_data_root)
                .context("failed to initialize Google Fonts service")?,
        );
        Ok(Self {
            fontbook: Arc::new(Mutex::new(fontbook)),
            renderer: TinySkiaRenderer::new()?,
            symbol_fallbacks,
            google_fonts,
        })
    }

    /// List system + cached Google Fonts for the API.
    pub fn available_fonts(&self) -> Result<Vec<FontFaceInfo>> {
        let fontbook = self
            .fontbook
            .lock()
            .map_err(|_| anyhow::anyhow!("failed to lock fontbook"))?;
        let mut fonts = fontbook
            .all_families()
            .into_iter()
            .filter(|face| !face.post_script_name.is_empty())
            .map(|face| {
                let family_name = face
                    .families
                    .first()
                    .map(|(family, _)| family.clone())
                    .unwrap_or_else(|| face.post_script_name.clone());
                FontFaceInfo {
                    family_name,
                    post_script_name: face.post_script_name,
                    source: FontSource::System,
                    category: None,
                    cached: true,
                }
            })
            .collect::<Vec<_>>();
        fonts.extend(self.google_fonts.default_faces());
        let mut seen = HashSet::new();
        fonts.retain(|font| seen.insert(font.post_script_name.clone()));
        fonts.sort();
        Ok(fonts)
    }

    /// Render every block's translation, composite onto `inpainted`, return
    /// the full page + per-block sprites. Blocks with an empty translation
    /// are skipped (they appear as holes in the composite, falling through to
    /// the inpainted plane).
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(level = "info", skip_all, fields(blocks = blocks.len()))]
    pub fn render_page(
        &self,
        inpainted: &DynamicImage,
        brush_layer: Option<&DynamicImage>,
        bubble_mask: Option<&DynamicImage>,
        image_width: u32,
        image_height: u32,
        blocks: &[RenderBlockInput],
        opts: &PageRenderOptions,
    ) -> Result<RenderOutput> {
        let min_font = min_font_size_for_image(image_width, image_height);
        // Build the bubble index once per page. The mask encodes each
        // detected bubble as a distinct grayscale ID; the index scans
        // once to record per-ID bboxes and then answers seed→bbox
        // lookups in O(seed_area).
        let bubble_index: Option<BubbleIndex> = bubble_mask.map(|m| BubbleIndex::new(m.to_luma8()));
        let layout_boxes = resolve_layout_boxes(blocks, bubble_index.as_ref());
        #[cfg(test)]
        for (block, resolved) in blocks.iter().zip(&layout_boxes) {
            record_renderer_layout_resolver(block.node_id, &resolved.layout_box);
        }
        let mut automatic = HashMap::new();
        if let Some(policy) = opts.source_relative_font_size_policy {
            for (block, resolved_box) in blocks.iter().zip(layout_boxes.iter().copied()) {
                if let Some(prepared) = self.prepare_source_relative_automatic(
                    block,
                    resolved_box,
                    opts,
                    policy,
                    min_font,
                )? {
                    #[cfg(test)]
                    record_renderer_layout_count(block.node_id, |record| {
                        record.builder_publication_count += 1;
                    });
                    automatic.insert(block.node_id, prepared);
                }
            }
        }
        let grouped_font_sizes = grouped_fitted_source_relative_font_sizes(blocks, &automatic);
        let bubble_mask = bubble_index.as_ref().map(BubbleIndex::mask);

        let mut rendered_blocks = Vec::with_capacity(blocks.len());
        for (block, layout_box) in blocks.iter().zip(layout_boxes.iter().copied()) {
            if let Some(prepared) = automatic.get(&block.node_id) {
                #[cfg(test)]
                let grouped_font_size = grouped_font_sizes.get(&block.node_id).copied();
                #[cfg(test)]
                let final_size = grouped_font_size.unwrap_or(prepared.independent_font_size);
                #[cfg(not(test))]
                let final_size = grouped_font_sizes
                    .get(&block.node_id)
                    .copied()
                    .unwrap_or(prepared.independent_font_size);
                rendered_blocks.push(self.render_prepared_source_relative(
                    block,
                    prepared,
                    final_size,
                    #[cfg(test)]
                    grouped_font_size,
                    opts.target_language.as_deref(),
                    opts.raster,
                )?);
                continue;
            }
            match self.render_one(
                block,
                layout_box,
                bubble_mask,
                &opts.shader_effect,
                &opts.shader_stroke,
                opts.document_font.as_deref(),
                opts.target_language.as_deref(),
                opts.source_relative_font_size_policy,
                grouped_font_sizes.get(&block.node_id).copied(),
                opts.raster,
                min_font,
            ) {
                Ok(Some(out)) => rendered_blocks.push(out),
                Ok(None) => {}
                Err(e) => tracing::warn!(node = %block.node_id, "render failed: {e:#}"),
            }
        }

        // Compose the final page: inpainted → brush → per-block sprites.
        let mut canvas = inpainted.to_rgba8();
        if let Some(brush) = brush_layer {
            imageops::overlay(&mut canvas, &brush.to_rgba8(), 0, 0);
        }
        for out in &rendered_blocks {
            let (x, y) = placement_origin(find_input(blocks, out.node_id), &out.expanded_transform);
            imageops::overlay(&mut canvas, &out.sprite.to_rgba8(), x as i64, y as i64);
        }
        Ok(RenderOutput {
            final_render: DynamicImage::ImageRgba8(canvas),
            blocks: rendered_blocks,
        })
    }

    fn prepare_source_relative_automatic(
        &self,
        block: &RenderBlockInput,
        resolved_box: ResolvedLayoutBox,
        opts: &PageRenderOptions,
        policy: SourceRelativeFontSizePolicy,
        min_font_size: f32,
    ) -> Result<Option<PreparedAutomaticBlock>> {
        let translation = block.translation.trim();
        if translation.is_empty() {
            return Ok(None);
        }

        let layout_box = resolved_box.layout_box;
        let (explicit_font_size, cap) =
            font_size_constraints_with_group(block, layout_box, min_font_size, Some(policy), None);
        if explicit_font_size.is_some() {
            return Ok(None);
        }

        let mut style = block.style.clone().unwrap_or_default();
        if style.font_families.is_empty()
            && let Some(font) = opts.document_font.as_deref()
        {
            style.font_families.push(font.to_string());
        }
        apply_default_font_families(&mut style.font_families, translation);
        let font = self.select_font(&style)?;
        #[cfg(test)]
        let fill_resolution = resolve_text_color_decision(
            block.style.as_ref(),
            &style,
            block.font_prediction.as_ref(),
        );
        #[cfg(test)]
        let color = fill_resolution.color;
        #[cfg(not(test))]
        let color =
            resolve_text_color(block.style.as_ref(), &style, block.font_prediction.as_ref());
        let layout_source = layout_source_from_input(block, translation);
        let writing_mode = writing_mode_for_block(&layout_source);
        let align = style
            .text_align
            .map(core_align_to_renderer)
            .unwrap_or(RendererTextAlign::Center);
        let stroke = style.stroke.clone();
        let prediction = block.font_prediction.as_ref();
        let global_stroke = opts.shader_stroke.clone();
        let block_effect = style.effect.unwrap_or(opts.shader_effect);
        let independent_font_size = {
            let layout_builder = automatic_layout_builder(
                &font,
                &self.symbol_fallbacks,
                writing_mode,
                align,
                opts.target_language.as_deref(),
            );
            let fits = |run: &LayoutRun<'_>| {
                source_relative_raster_dimensions(
                    run,
                    resolve_stroke_style(
                        prediction,
                        stroke.as_ref(),
                        global_stroke.as_ref(),
                        run.font_size,
                        color,
                    ),
                    block_effect,
                )
                .is_some_and(|(width, height, _)| {
                    width as f32 <= layout_box.width && height as f32 <= layout_box.height
                })
            };
            fit_font_size_with_predicate(
                &layout_builder,
                translation,
                layout_box,
                min_font_size.ceil() as i32..=cap.floor() as i32,
                block.preserve_explicit_lines,
                fits,
                true,
            )
            .with_context(|| format!("automatic font size does not fit node {}", block.node_id))?
            .font_size
        };

        #[cfg(test)]
        let diagnostic = {
            let source_geometry_estimate = source_geometry_font_size(block);
            let source_resolution =
                resolve_source_size_candidate(block, source_geometry_estimate, policy);
            PreparedRendererDiagnostic {
                source_geometry_estimate,
                valid_detected_size: source_resolution.valid_detected_size,
                valid_predicted_size: source_resolution.valid_predicted_size,
                source_size_branch: source_resolution.branch,
                policy_offset: policy.offset,
                candidate_size: source_resolution.candidate,
                auto_max: max_font_size_for_box(layout_box, min_font_size),
                layout_box_branch: resolved_box.diagnostic_branch,
                fill_branch: fill_resolution.branch,
            }
        };

        Ok(Some(PreparedAutomaticBlock {
            font,
            stroke,
            global_stroke,
            effect: block_effect,
            color,
            writing_mode,
            align,
            layout_box,
            cap,
            independent_font_size,
            #[cfg(test)]
            diagnostic,
        }))
    }

    fn render_prepared_source_relative(
        &self,
        block: &RenderBlockInput,
        prepared: &PreparedAutomaticBlock,
        font_size: f32,
        #[cfg(test)] grouped_font_size: Option<f32>,
        target_language: Option<&str>,
        raster: RasterOptions,
    ) -> Result<RenderedBlock> {
        anyhow::ensure!(
            font_size <= prepared.independent_font_size + FIT_EPSILON,
            "group font size exceeds independent safe size for node {}",
            block.node_id
        );
        let builder = automatic_layout_builder(
            &prepared.font,
            &self.symbol_fallbacks,
            prepared.writing_mode,
            prepared.align,
            target_language,
        );
        #[cfg(test)]
        record_renderer_layout_count(block.node_id, |record| {
            record.renderer_rebuild_count += 1;
        });
        #[cfg(test)]
        record_renderer_layout_stage(
            block.node_id,
            &prepared.layout_box,
            |record, record_ptr, value| {
                record.fit_record_ptr = Some(record_ptr);
                record.fit_box = Some(value);
            },
        );
        let layout = run_layout_at(
            &builder,
            block.translation.trim(),
            prepared.layout_box,
            font_size,
            block.preserve_explicit_lines,
        )?;
        #[cfg(test)]
        let stroke_resolution = resolve_stroke_style_decision(
            block.font_prediction.as_ref(),
            prepared.stroke.as_ref(),
            prepared.global_stroke.as_ref(),
            font_size,
            prepared.color,
        );
        #[cfg(test)]
        let resolved_stroke = stroke_resolution.stroke;
        #[cfg(not(test))]
        let resolved_stroke = resolve_stroke_style(
            block.font_prediction.as_ref(),
            prepared.stroke.as_ref(),
            prepared.global_stroke.as_ref(),
            font_size,
            prepared.color,
        );
        let (predicted_width, predicted_height, padding) =
            source_relative_raster_dimensions(&layout, resolved_stroke, prepared.effect)
                .context("automatic layout has invalid raster dimensions")?;
        anyhow::ensure!(
            predicted_width as f32 <= prepared.layout_box.width
                && predicted_height as f32 <= prepared.layout_box.height,
            "automatic font size does not fit node {}",
            block.node_id
        );
        #[cfg(test)]
        record_renderer_layout_stage(
            block.node_id,
            &prepared.layout_box,
            |record, record_ptr, value| {
                record.postvalidate_record_ptr = Some(record_ptr);
                record.postvalidate_box = Some(value);
            },
        );
        let rendered = self.render_layout(
            &layout,
            prepared.writing_mode,
            &RenderOptions {
                font_size,
                color: prepared.color,
                effect: shader_core_to_renderer(prepared.effect),
                stroke: resolved_stroke,
                padding,
                raster,
                ..Default::default()
            },
            block.node_id,
            RasterPath::SourceRelative,
        )?;
        #[cfg(test)]
        record_renderer_layout_count(block.node_id, |record| {
            record.builder_raster_count += 1;
        });
        debug_assert_eq!(rendered.width(), predicted_width);
        debug_assert_eq!(rendered.height(), predicted_height);
        #[cfg(test)]
        {
            let prediction = block.font_prediction.as_ref();
            let (alpha_bbox, alpha_nonzero_pixels, alpha_blake3) =
                renderer_alpha_summary(&rendered);
            if let Some((
                resolver_record_ptr,
                fit_record_ptr,
                postvalidate_record_ptr,
                resolver_box,
                fit_box,
                postvalidate_box,
                builder_publication_count,
                builder_raster_count,
                renderer_rebuild_count,
            )) = renderer_layout_diagnostic(block.node_id)
            {
                record_renderer_diagnostic(RendererDiagnosticEvent {
                    node_id: block.node_id,
                    source_geometry_estimate: prepared.diagnostic.source_geometry_estimate,
                    valid_detected_size: prepared.diagnostic.valid_detected_size,
                    valid_predicted_size: prepared.diagnostic.valid_predicted_size,
                    source_size_branch: prepared.diagnostic.source_size_branch,
                    policy_offset: prepared.diagnostic.policy_offset,
                    candidate_size: prepared.diagnostic.candidate_size,
                    auto_max: prepared.diagnostic.auto_max,
                    cap: prepared.cap,
                    resolved_layout_width: prepared.layout_box.width,
                    resolved_layout_height: prepared.layout_box.height,
                    layout_box_branch: prepared.diagnostic.layout_box_branch,
                    tight_layout_width: layout.width,
                    tight_layout_height: layout.height,
                    independent_size: prepared.independent_font_size,
                    group_size: grouped_font_size,
                    final_size: font_size,
                    rotation_deg: block.transform.rotation_deg,
                    predicted_fill_rgb: prediction.map(|prediction| prediction.text_color),
                    predicted_stroke_rgb: prediction.map(|prediction| prediction.stroke_color),
                    predicted_stroke_width: prediction
                        .map(|prediction| prediction.stroke_width_px)
                        .filter(|width| width.is_finite()),
                    resolved_fill_rgba: prepared.color,
                    resolved_stroke_rgba: resolved_stroke.map(|stroke| stroke.color),
                    resolved_stroke_width: resolved_stroke.map(|stroke| stroke.width_px),
                    fill_branch: prepared.diagnostic.fill_branch,
                    stroke_branch: stroke_resolution.branch,
                    font_outcome: renderer_font_outcome(block),
                    fill_outcome: renderer_fill_outcome(prepared.diagnostic.fill_branch),
                    stroke_outcome: renderer_stroke_outcome(stroke_resolution.branch),
                    final_font_size_px: renderer_final_font_size(font_size)?,
                    resolver_record_ptr,
                    fit_record_ptr,
                    postvalidate_record_ptr,
                    resolver_box,
                    fit_box,
                    postvalidate_box,
                    resolver_box_blake3: renderer_box_digest(resolver_box),
                    fit_box_blake3: renderer_box_digest(fit_box),
                    postvalidate_box_blake3: renderer_box_digest(postvalidate_box),
                    builder_publication_count,
                    builder_raster_count,
                    renderer_rebuild_count,
                    sprite_width: rendered.width(),
                    sprite_height: rendered.height(),
                    sprite_rgba_blake3: blake3::hash(rendered.as_raw()).to_hex().to_string(),
                    alpha_bbox,
                    alpha_nonzero_pixels,
                    alpha_blake3,
                });
            }
        }
        tracing::debug!(
            node = %block.node_id,
            cap = prepared.cap,
            independent_font_size = prepared.independent_font_size,
            final_font_size = font_size,
            ink_width = layout.width,
            ink_height = layout.height,
            padding,
            sprite_width = rendered.width(),
            sprite_height = rendered.height(),
            source_width = prepared.layout_box.width,
            source_height = prepared.layout_box.height,
            "source-relative automatic layout"
        );
        let transform = centred_sprite_transform(
            prepared.layout_box,
            rendered.width(),
            rendered.height(),
            block.transform.rotation_deg,
        );
        Ok(RenderedBlock {
            node_id: block.node_id,
            sprite: DynamicImage::ImageRgba8(rendered),
            rendered_direction: rendered_direction_for_writing_mode(prepared.writing_mode),
            expanded_transform: Some(transform),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render_one(
        &self,
        block: &RenderBlockInput,
        resolved_box: ResolvedLayoutBox,
        bubble_mask: Option<&GrayImage>,
        effect: &TextShaderEffect,
        global_stroke: &Option<TextStrokeStyle>,
        document_font: Option<&str>,
        target_language: Option<&str>,
        source_relative_font_size_policy: Option<SourceRelativeFontSizePolicy>,
        grouped_source_relative_font_size: Option<f32>,
        raster: RasterOptions,
        min_font_size: f32,
    ) -> Result<Option<RenderedBlock>> {
        let translation = block.translation.trim();
        if translation.is_empty() {
            return Ok(None);
        }

        let layout_source = layout_source_from_input(block, translation);

        let mut style = block.style.clone().unwrap_or_else(|| TextStyle {
            font_families: Vec::new(),
            font_size: None,
            color: [0, 0, 0, 255],
            effect: None,
            stroke: None,
            text_align: None,
        });
        if style.font_families.is_empty()
            && let Some(font) = document_font
        {
            style.font_families.push(font.to_string());
        }
        apply_default_font_families(&mut style.font_families, translation);

        let font = self.select_font(&style)?;
        let block_effect = style.effect.unwrap_or(*effect);
        let color =
            resolve_text_color(block.style.as_ref(), &style, block.font_prediction.as_ref());

        let writing_mode = writing_mode_for_block(&layout_source);
        // Translations default to centre alignment inside a bubble — each
        // line sits centred above/below the others, matching manga
        // typesetting convention. Explicit `style.text_align` wins if set.
        let align = style
            .text_align
            .map(core_align_to_renderer)
            .unwrap_or(RendererTextAlign::Center);
        let layout_box = resolved_box.layout_box;

        let mut layout_builder = TextLayout::new(&font, None)
            .with_fallback_fonts(&self.symbol_fallbacks)
            .with_writing_mode(writing_mode)
            .with_alignment(align);
        if let Some(target_language) = target_language {
            layout_builder = layout_builder.with_hyphenation_language_tag(target_language);
        }
        let (explicit_font_size, max_font) = font_size_constraints_with_group(
            block,
            layout_box,
            min_font_size,
            source_relative_font_size_policy,
            grouped_source_relative_font_size,
        );
        let sizing_font_size = explicit_font_size.unwrap_or(max_font);
        let sizing_padding = stroke_padding(resolve_stroke_style(
            block.font_prediction.as_ref(),
            style.stroke.as_ref(),
            global_stroke.as_ref(),
            sizing_font_size,
            color,
        ));
        let content_width = (layout_box.width - sizing_padding * 2.0).max(1.0);
        let content_height = (layout_box.height - sizing_padding * 2.0).max(1.0);
        let content_box = LayoutBox {
            x: layout_box.x + (layout_box.width - content_width) * 0.5,
            y: layout_box.y + (layout_box.height - content_height) * 0.5,
            width: content_width,
            height: content_height,
        };
        let mut render_candidate = |layout: &LayoutRun<'_>| -> Result<RenderedTextCandidate> {
            let resolved_stroke = resolve_stroke_style(
                block.font_prediction.as_ref(),
                style.stroke.as_ref(),
                global_stroke.as_ref(),
                layout.font_size,
                color,
            );
            let padding = stroke_padding(resolved_stroke);

            let rendered = self.render_layout(
                layout,
                writing_mode,
                &RenderOptions {
                    font_size: layout.font_size,
                    color,
                    effect: shader_core_to_renderer(block_effect),
                    stroke: resolved_stroke,
                    padding,
                    raster,
                    ..Default::default()
                },
                block.node_id,
                RasterPath::Legacy,
            )?;
            let transform = centred_sprite_transform(
                layout_box,
                rendered.width(),
                rendered.height(),
                block.transform.rotation_deg,
            );
            Ok(RenderedTextCandidate {
                image: rendered,
                transform,
            })
        };

        if let Some((mask, bubble_id)) = bubble_mask.zip(resolved_box.bubble_id) {
            let candidate = fit_rendered_with_mask_collision(
                &layout_builder,
                translation,
                content_box,
                explicit_font_size,
                min_font_size,
                max_font,
                block.preserve_explicit_lines,
                mask,
                bubble_id,
                &mut render_candidate,
            )?;
            return Ok(Some(RenderedBlock {
                node_id: block.node_id,
                sprite: DynamicImage::ImageRgba8(candidate.image),
                rendered_direction: rendered_direction_for_writing_mode(writing_mode),
                expanded_transform: Some(candidate.transform),
            }));
        }

        let layout = fit_font_size(
            &layout_builder,
            translation,
            content_box,
            explicit_font_size,
            min_font_size,
            max_font,
            block.preserve_explicit_lines,
        )?;

        let candidate = render_candidate(&layout)?;

        Ok(Some(RenderedBlock {
            node_id: block.node_id,
            sprite: DynamicImage::ImageRgba8(candidate.image),
            rendered_direction: rendered_direction_for_writing_mode(writing_mode),
            expanded_transform: Some(candidate.transform),
        }))
    }

    /// Resolve a set of font family candidates into a single PostScript name.
    pub fn resolve_post_script_name(
        &self,
        style: &TextStyle,
        text: Option<&str>,
    ) -> Result<String> {
        let fontbook = self
            .fontbook
            .lock()
            .map_err(|_| anyhow::anyhow!("failed to lock fontbook"))?;
        let faces = fontbook.all_families();

        let mut families = style.font_families.clone();
        if families.is_empty()
            && let Some(text) = text
        {
            tracing::debug!(
                "Families empty, applying script-based default font families for text: {}",
                text
            );
            apply_default_font_families(&mut families, text);
        }
        if families.is_empty() {
            families.push("ArialMT".to_string());
        }

        for candidate in &families {
            tracing::debug!("Attempting to resolve font candidate: {}", candidate);
            // 1. Exact PS name
            if let Some(face) = faces.iter().find(|f| f.post_script_name == *candidate) {
                tracing::debug!("Resolved via exact PS name: {}", face.post_script_name);
                return Ok(face.post_script_name.clone());
            }

            // 2. Google Font variant
            let (family, weight, style_str) = crate::google_fonts::parse_variant_query(candidate);
            if candidate.contains(':')
                && self
                    .google_fonts
                    .read_cached_variant(family, weight, style_str)
                    .map(|opt| opt.is_some())
                    .unwrap_or(false)
            {
                tracing::debug!("Resolved via Google Font variant: {}", candidate);
                return Ok(candidate.clone());
            }

            // 3. Fuzzy family name
            if let Some(psn) = face_post_script_name(&faces, candidate) {
                tracing::debug!("Resolved via fuzzy family name: {}", psn);
                return Ok(psn);
            }

            // 4. Base Google Font
            if self
                .google_fonts
                .read_cached_file(candidate)
                .map(|opt| opt.is_some())
                .unwrap_or(false)
            {
                tracing::debug!("Resolved via base Google Font: {}", candidate);
                return Ok(candidate.clone());
            }
        }

        tracing::warn!(?families, "font resolution failed, falling back to ArialMT");
        Ok("ArialMT".to_string())
    }

    fn select_font(&self, style: &TextStyle) -> Result<Font> {
        let mut fontbook = self
            .fontbook
            .lock()
            .map_err(|_| anyhow::anyhow!("failed to lock fontbook"))?;
        for candidate in &style.font_families {
            let faces = fontbook.all_families();

            // 1. Try exact PostScript name match first (most reliable for variants)
            if let Some(face) = faces.iter().find(|f| f.post_script_name == *candidate) {
                return fontbook.load_font(face.id);
            }

            // 2. Check if it's a Google Font variant (Family:WeightStyle)
            let (family, weight, style_str) = crate::google_fonts::parse_variant_query(candidate);
            if candidate.contains(':')
                && let Some(data) = self
                    .google_fonts
                    .read_cached_variant(family, weight, style_str)?
            {
                let mut font = fontbook.load_from_bytes(data)?;

                // Explicitly set the weight and style for variable font instancing
                font.weight = weight;
                font.style = style_str.to_string();

                return Ok(font);
            }

            // 3. Try fuzzy family name match
            if let Some(psn) = face_post_script_name(&faces, candidate) {
                return fontbook.query(&psn);
            }

            // 4. Try base Google Font file
            if let Some(data) = self.google_fonts.read_cached_file(candidate)? {
                return fontbook.load_from_bytes(data);
            }
        }
        Err(anyhow::anyhow!(
            "no font found for candidates: {:?}",
            style.font_families
        ))
    }

    fn render_layout(
        &self,
        layout: &LayoutRun<'_>,
        writing_mode: WritingMode,
        options: &RenderOptions,
        node_id: NodeId,
        path: RasterPath,
    ) -> Result<RgbaImage> {
        let rendered = self.renderer.render(layout, writing_mode, options)?;

        #[cfg(test)]
        RASTER_TRACE.with(|trace| {
            trace.borrow_mut().push(RasterTrace {
                node_id,
                path,
                font_size: layout.font_size,
            });
        });
        #[cfg(not(test))]
        let _ = (node_id, path);

        Ok(rendered)
    }
}

// ---------------------------------------------------------------------------
// Helpers: font sizing
// ---------------------------------------------------------------------------

const MASK_COLLISION_ALPHA_THRESHOLD: u8 = 8;
const FIT_EPSILON: f32 = 0.5;

#[cfg(test)]
std::thread_local! {
    static RASTER_TRACE: std::cell::RefCell<Vec<RasterTrace>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RasterPath {
    SourceRelative,
    Legacy,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
struct RasterTrace {
    node_id: NodeId,
    path: RasterPath,
    font_size: f32,
}

#[cfg(test)]
fn clear_raster_trace() {
    RASTER_TRACE.with(|trace| trace.borrow_mut().clear());
}

#[cfg(test)]
fn raster_trace() -> Vec<RasterTrace> {
    RASTER_TRACE.with(|trace| trace.borrow().clone())
}

struct RenderedTextCandidate {
    image: RgbaImage,
    transform: Transform,
}

struct PreparedAutomaticBlock {
    font: Font,
    stroke: Option<TextStrokeStyle>,
    global_stroke: Option<TextStrokeStyle>,
    effect: TextShaderEffect,
    color: [u8; 4],
    writing_mode: WritingMode,
    align: RendererTextAlign,
    layout_box: LayoutBox,
    cap: f32,
    independent_font_size: f32,
    #[cfg(test)]
    diagnostic: PreparedRendererDiagnostic,
}

#[cfg(test)]
struct PreparedRendererDiagnostic {
    source_geometry_estimate: f32,
    valid_detected_size: Option<f32>,
    valid_predicted_size: Option<f32>,
    source_size_branch: RendererSourceSizeBranch,
    policy_offset: f32,
    candidate_size: f32,
    auto_max: f32,
    layout_box_branch: RendererLayoutBoxBranch,
    fill_branch: RendererFillBranch,
}

fn automatic_layout_builder<'a>(
    font: &'a Font,
    fallbacks: &'a [Font],
    writing_mode: WritingMode,
    align: RendererTextAlign,
    target_language: Option<&str>,
) -> TextLayout<'a> {
    let mut builder = TextLayout::new(font, None)
        .with_fallback_fonts(fallbacks)
        .with_writing_mode(writing_mode)
        .with_alignment(align);
    if let Some(target_language) = target_language {
        builder = builder.with_hyphenation_language_tag(target_language);
    }
    builder
}

fn source_relative_raster_dimensions(
    layout: &LayoutRun<'_>,
    stroke: Option<RenderStrokeOptions>,
    effect: TextShaderEffect,
) -> Option<(u32, u32, f32)> {
    const BOLD_EFFECT_PADDING: f32 = 4.0;
    const ITALIC_EFFECT_PADDING: f32 = 3.0;
    const ITALIC_SLANT_FACTOR: f32 = 0.22;
    const MIN_ITALIC_SLANT: f32 = 1.0;

    let effect_padding = if effect.bold {
        BOLD_EFFECT_PADDING
    } else if effect.italic {
        ITALIC_EFFECT_PADDING
    } else {
        0.0
    };
    let italic_slant = if effect.italic {
        (layout.width.min(layout.height) * ITALIC_SLANT_FACTOR)
            .max(MIN_ITALIC_SLANT)
            .ceil()
    } else {
        0.0
    };
    let padding = stroke_padding(stroke).max(effect_padding) + italic_slant;
    let width = (layout.width + padding * 2.0).ceil();
    let height = (layout.height + padding * 2.0).ceil();
    (width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0).then_some((
        width as u32,
        height as u32,
        padding,
    ))
}

struct MaskCollisionAttempt {
    candidate: RenderedTextCandidate,
    valid: bool,
}

pub(crate) fn min_font_size_for_image(image_width: u32, image_height: u32) -> f32 {
    let max_dim = image_width.max(image_height) as f32;
    (max_dim / 90.0).clamp(12.0, 28.0)
}

#[cfg(test)]
fn font_size_constraints(
    block: &RenderBlockInput,
    layout_box: LayoutBox,
    min_size: f32,
    source_relative_offset: Option<f32>,
) -> (Option<f32>, f32) {
    font_size_constraints_with_group(
        block,
        layout_box,
        min_size,
        source_relative_offset.map(|offset| SourceRelativeFontSizePolicy {
            offset,
            prefer_detected: false,
        }),
        None,
    )
}

fn font_size_constraints_with_group(
    block: &RenderBlockInput,
    layout_box: LayoutBox,
    min_size: f32,
    source_relative_policy: Option<SourceRelativeFontSizePolicy>,
    grouped_source_relative_font_size: Option<f32>,
) -> (Option<f32>, f32) {
    let auto_max = max_font_size_for_box(layout_box, min_size);
    let scene_size = block.style.as_ref().and_then(|style| style.font_size);
    let Some(policy) = source_relative_policy else {
        return if block.typography_plan_verified {
            (None, scene_size.map_or(auto_max, |cap| auto_max.min(cap)))
        } else {
            (scene_size, auto_max)
        };
    };
    let valid_scene_size = scene_size.filter(|size| size.is_finite() && *size > 0.0);
    if !block.typography_plan_verified
        && let Some(scene_size) = valid_scene_size
    {
        return (Some(scene_size), auto_max);
    }
    let cap = grouped_source_relative_font_size.unwrap_or_else(|| {
        source_relative_font_size_candidate(block, source_geometry_font_size(block), policy)
    });
    (None, cap)
}

fn source_geometry_font_size(block: &RenderBlockInput) -> f32 {
    block.transform.width.min(block.transform.height).max(1.0)
}

fn source_relative_font_size_candidate(
    block: &RenderBlockInput,
    auto_max: f32,
    policy: SourceRelativeFontSizePolicy,
) -> f32 {
    let resolution = resolve_source_size_candidate(block, auto_max, policy);
    #[cfg(test)]
    {
        resolution.candidate
    }
    #[cfg(not(test))]
    {
        resolution
    }
}

fn resolve_source_size_candidate(
    block: &RenderBlockInput,
    fallback: f32,
    policy: SourceRelativeFontSizePolicy,
) -> SourceSizeResolutionOutput {
    let detected = block
        .detected_font_size_px
        .filter(|size| size.is_finite() && *size > 0.0);
    let prediction = block
        .font_prediction
        .as_ref()
        .map(|prediction| prediction.font_size_px)
        .filter(|size| size.is_finite() && *size > 0.0);
    if policy.prefer_detected {
        if let Some(source_size) = detected {
            return source_size_resolution(
                source_size,
                policy.offset,
                #[cfg(test)]
                detected,
                #[cfg(test)]
                prediction,
                #[cfg(test)]
                RendererSourceSizeBranch::Detected,
            );
        }
        if let Some(source_size) = prediction {
            return source_size_resolution(
                source_size,
                policy.offset,
                #[cfg(test)]
                detected,
                #[cfg(test)]
                prediction,
                #[cfg(test)]
                RendererSourceSizeBranch::Predicted,
            );
        }
    } else {
        if let Some(source_size) = prediction {
            return source_size_resolution(
                source_size,
                policy.offset,
                #[cfg(test)]
                detected,
                #[cfg(test)]
                prediction,
                #[cfg(test)]
                RendererSourceSizeBranch::Predicted,
            );
        }
        if let Some(source_size) = detected {
            return source_size_resolution(
                source_size,
                policy.offset,
                #[cfg(test)]
                detected,
                #[cfg(test)]
                prediction,
                #[cfg(test)]
                RendererSourceSizeBranch::Detected,
            );
        }
    }
    source_size_resolution(
        fallback,
        policy.offset,
        #[cfg(test)]
        detected,
        #[cfg(test)]
        prediction,
        #[cfg(test)]
        RendererSourceSizeBranch::GeometryFallback,
    )
}

fn source_size_resolution(
    source_size: f32,
    offset: f32,
    #[cfg(test)] valid_detected_size: Option<f32>,
    #[cfg(test)] valid_predicted_size: Option<f32>,
    #[cfg(test)] branch: RendererSourceSizeBranch,
) -> SourceSizeResolutionOutput {
    let candidate = (source_size + offset).max(1.0);
    #[cfg(test)]
    {
        SourceSizeResolution {
            candidate,
            valid_detected_size,
            valid_predicted_size,
            branch,
        }
    }
    #[cfg(not(test))]
    {
        candidate
    }
}

fn grouped_fitted_source_relative_font_sizes(
    blocks: &[RenderBlockInput],
    automatic: &HashMap<NodeId, PreparedAutomaticBlock>,
) -> HashMap<NodeId, f32> {
    let automatic = blocks
        .iter()
        .filter_map(|block| {
            automatic
                .get(&block.node_id)
                .filter(|prepared| prepared.writing_mode == WritingMode::Horizontal)
                .map(|prepared| {
                    (
                        block.node_id,
                        block.source_transform,
                        prepared.independent_font_size,
                    )
                })
        })
        .collect::<Vec<_>>();
    group_source_relative_font_sizes(automatic)
}

#[cfg(test)]
fn grouped_source_relative_font_sizes(
    blocks: &[RenderBlockInput],
    layout_boxes: &[ResolvedLayoutBox],
    min_size: f32,
    policy: Option<SourceRelativeFontSizePolicy>,
) -> HashMap<NodeId, f32> {
    let Some(policy) = policy else {
        return HashMap::new();
    };
    let automatic = blocks
        .iter()
        .zip(layout_boxes)
        .filter(|(block, _)| {
            !block.translation.trim().is_empty()
                && block.source_direction != Some(TextDirection::Vertical)
                && !(block
                    .style
                    .as_ref()
                    .and_then(|style| style.font_size)
                    .is_some_and(|size| size.is_finite() && size > 0.0)
                    && !block.typography_plan_verified)
        })
        .map(|(block, resolved)| {
            let auto_max = max_font_size_for_box(resolved.layout_box, min_size);
            (
                block.node_id,
                block.source_transform,
                source_relative_font_size_candidate(block, auto_max, policy),
            )
        })
        .collect::<Vec<_>>();
    group_source_relative_font_sizes(automatic)
}

fn group_source_relative_font_sizes(
    mut automatic: Vec<(NodeId, Transform, f32)>,
) -> HashMap<NodeId, f32> {
    automatic.sort_by_key(|(node_id, _, _)| *node_id);

    let mut sizes = HashMap::with_capacity(automatic.len());
    let mut visited = vec![false; automatic.len()];
    for start in 0..automatic.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![start];
        let mut component = Vec::new();
        while let Some(index) = stack.pop() {
            component.push(index);
            for other in 0..automatic.len() {
                if !visited[other] && same_source_row(automatic[index].1, automatic[other].1) {
                    visited[other] = true;
                    stack.push(other);
                }
            }
        }
        let minimum = component
            .iter()
            .map(|index| automatic[*index].2)
            .fold(f32::INFINITY, f32::min);
        for index in component {
            sizes.insert(automatic[index].0, minimum);
        }
    }
    sizes
}

fn same_source_row(left: Transform, right: Transform) -> bool {
    let values = [left.y, left.height, right.y, right.height];
    if !values.iter().all(|value| value.is_finite()) || left.height <= 0.0 || right.height <= 0.0 {
        return false;
    }
    let center_delta = ((left.y + left.height * 0.5) - (right.y + right.height * 0.5)).abs();
    center_delta <= 4.0_f32.max(left.height.min(right.height) * 0.25)
}

/// Maximum font size for the given layout box, derived from its dimensions.
/// Caps extreme cases (huge empty bubble + short text → giant glyphs).
fn max_font_size_for_box(layout_box: LayoutBox, min_size: f32) -> f32 {
    const GLOBAL_CAP_PX: f32 = 72.0;
    let by_height = layout_box.height * 0.45;
    let by_width = layout_box.width * 0.9;
    by_height.min(by_width).clamp(min_size + 1.0, GLOBAL_CAP_PX)
}

/// Binary-search the largest integer font size in `[min_size, max_size]`
/// whose shaped layout still fits inside the constraint box. An
/// `explicit_size` override (user-set per-block font size) bypasses the
/// search.
fn fit_font_size<'a>(
    layout_builder: &TextLayout<'a>,
    text: &str,
    layout_box: LayoutBox,
    explicit_size: Option<f32>,
    min_size: f32,
    max_size: f32,
    preserve_explicit_lines: bool,
) -> Result<LayoutRun<'a>> {
    let run_at = |size: f32| {
        run_layout_at(
            layout_builder,
            text,
            layout_box,
            size,
            preserve_explicit_lines,
        )
    };
    if let Some(s) = explicit_size {
        return run_at(s);
    }

    let min_size = min_size.max(1.0).round() as i32;
    let max_size = (max_size.round() as i32).max(min_size);
    fit_font_size_with_predicate(
        layout_builder,
        text,
        layout_box,
        min_size..=max_size,
        preserve_explicit_lines,
        |run| run.width <= layout_box.width && run.height <= layout_box.height,
        false,
    )
}

fn fit_font_size_with_predicate<'a, F>(
    layout_builder: &TextLayout<'a>,
    text: &str,
    layout_box: LayoutBox,
    font_size_range: RangeInclusive<i32>,
    preserve_explicit_lines: bool,
    fits: F,
    require_fit: bool,
) -> Result<LayoutRun<'a>>
where
    F: Fn(&LayoutRun<'a>) -> bool,
{
    let (min_size, max_size) = font_size_range.into_inner();
    anyhow::ensure!(
        max_size >= min_size,
        "font size cap is below the readability floor"
    );
    let run_at = |size: f32| {
        run_layout_at(
            layout_builder,
            text,
            layout_box,
            size,
            preserve_explicit_lines,
        )
    };
    let at_max = run_at(max_size as f32)?;
    if fits(&at_max) {
        return Ok(at_max);
    }
    // Binary-search [min, max) for the largest fitting size.
    let mut lo = min_size;
    let mut hi = max_size - 1;
    let mut best = run_at(min_size as f32)?;
    if !fits(&best) {
        anyhow::ensure!(!require_fit, "text does not fit at the readability floor");
        return Ok(best);
    }
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let candidate = run_at(mid as f32)?;
        if fits(&candidate) {
            best = candidate;
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    Ok(best)
}

#[allow(clippy::too_many_arguments)]
fn fit_rendered_with_mask_collision<'a, F>(
    layout_builder: &TextLayout<'a>,
    text: &str,
    layout_box: LayoutBox,
    explicit_size: Option<f32>,
    min_size: f32,
    max_size: f32,
    preserve_explicit_lines: bool,
    mask: &GrayImage,
    bubble_id: u8,
    render_candidate: &mut F,
) -> Result<RenderedTextCandidate>
where
    F: FnMut(&LayoutRun<'a>) -> Result<RenderedTextCandidate>,
{
    if let Some(size) = explicit_size {
        let attempt = render_mask_collision_attempt(
            layout_builder,
            text,
            layout_box,
            size.max(1.0),
            preserve_explicit_lines,
            mask,
            bubble_id,
            render_candidate,
        )?;
        return Ok(attempt.candidate);
    }

    let min_size = min_size.max(1.0).round() as i32;
    let max_size = (max_size.max(1.0).round() as i32).max(min_size);

    if let Some(candidate) = try_mask_collision_size(
        layout_builder,
        text,
        layout_box,
        max_size as f32,
        preserve_explicit_lines,
        mask,
        bubble_id,
        render_candidate,
    )? {
        return Ok(candidate);
    }

    let min_attempt = render_mask_collision_attempt(
        layout_builder,
        text,
        layout_box,
        min_size as f32,
        preserve_explicit_lines,
        mask,
        bubble_id,
        render_candidate,
    )?;
    if !min_attempt.valid {
        return Ok(min_attempt.candidate);
    }
    let mut best = min_attempt.candidate;

    let mut lo = min_size + 1;
    let mut hi = max_size - 1;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        if let Some(candidate) = try_mask_collision_size(
            layout_builder,
            text,
            layout_box,
            mid as f32,
            preserve_explicit_lines,
            mask,
            bubble_id,
            render_candidate,
        )? {
            best = candidate;
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }

    Ok(best)
}

#[allow(clippy::too_many_arguments)]
fn try_mask_collision_size<'a, F>(
    layout_builder: &TextLayout<'a>,
    text: &str,
    layout_box: LayoutBox,
    font_size: f32,
    preserve_explicit_lines: bool,
    mask: &GrayImage,
    bubble_id: u8,
    render_candidate: &mut F,
) -> Result<Option<RenderedTextCandidate>>
where
    F: FnMut(&LayoutRun<'a>) -> Result<RenderedTextCandidate>,
{
    let layout = run_collision_layout_at(
        layout_builder,
        text,
        layout_box,
        font_size,
        preserve_explicit_lines,
    )?;
    let fits_layout_box = layout_fits_collision_attempt(&layout, layout_box);
    if !fits_layout_box {
        return Ok(None);
    }

    let candidate = render_candidate(&layout)?;
    if sprite_collides_with_bubble_mask(&candidate.image, &candidate.transform, mask, bubble_id) {
        return Ok(None);
    }
    Ok(Some(candidate))
}

#[allow(clippy::too_many_arguments)]
fn render_mask_collision_attempt<'a, F>(
    layout_builder: &TextLayout<'a>,
    text: &str,
    layout_box: LayoutBox,
    font_size: f32,
    preserve_explicit_lines: bool,
    mask: &GrayImage,
    bubble_id: u8,
    render_candidate: &mut F,
) -> Result<MaskCollisionAttempt>
where
    F: FnMut(&LayoutRun<'a>) -> Result<RenderedTextCandidate>,
{
    let layout = run_collision_layout_at(
        layout_builder,
        text,
        layout_box,
        font_size,
        preserve_explicit_lines,
    )?;
    let fits_layout_box = layout_fits_collision_attempt(&layout, layout_box);
    let candidate = render_candidate(&layout)?;
    let valid = fits_layout_box
        && !sprite_collides_with_bubble_mask(
            &candidate.image,
            &candidate.transform,
            mask,
            bubble_id,
        );
    Ok(MaskCollisionAttempt { candidate, valid })
}

fn run_collision_layout_at<'a>(
    layout_builder: &TextLayout<'a>,
    text: &str,
    layout_box: LayoutBox,
    font_size: f32,
    preserve_explicit_lines: bool,
) -> Result<LayoutRun<'a>> {
    run_layout_at(
        layout_builder,
        text,
        layout_box,
        font_size,
        preserve_explicit_lines,
    )
}

fn run_layout_at<'a>(
    layout_builder: &TextLayout<'a>,
    text: &str,
    layout_box: LayoutBox,
    font_size: f32,
    preserve_explicit_lines: bool,
) -> Result<LayoutRun<'a>> {
    let layout = layout_builder.clone().with_font_size(font_size.max(1.0));
    if preserve_explicit_lines {
        layout.without_hyphenation().run(text)
    } else {
        layout
            .with_max_width(layout_box.width.max(1.0))
            .with_max_height(layout_box.height.max(1.0))
            .run(text)
    }
}

fn layout_fits_collision_attempt(layout: &LayoutRun<'_>, layout_box: LayoutBox) -> bool {
    layout.width <= layout_box.width + FIT_EPSILON
        && layout.height <= layout_box.height + FIT_EPSILON
}

fn sprite_collides_with_bubble_mask(
    sprite: &RgbaImage,
    transform: &Transform,
    mask: &GrayImage,
    bubble_id: u8,
) -> bool {
    let origin_x = transform.x.round() as i32;
    let origin_y = transform.y.round() as i32;
    let mask_w = mask.width() as i32;
    let mask_h = mask.height() as i32;

    for (x, y, pixel) in sprite.enumerate_pixels() {
        if pixel.0[3] <= MASK_COLLISION_ALPHA_THRESHOLD {
            continue;
        }
        let mask_x = origin_x + x as i32;
        let mask_y = origin_y + y as i32;
        if mask_x < 0 || mask_y < 0 || mask_x >= mask_w || mask_y >= mask_h {
            return true;
        }
        if mask.get_pixel(mask_x as u32, mask_y as u32).0[0] != bubble_id {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ResolvedLayoutBox {
    seed_box: LayoutBox,
    layout_box: LayoutBox,
    bubble_id: Option<u8>,
    #[cfg(test)]
    diagnostic_branch: RendererLayoutBoxBranch,
}

fn resolve_layout_boxes(
    blocks: &[RenderBlockInput],
    bubble_index: Option<&BubbleIndex>,
) -> Vec<ResolvedLayoutBox> {
    let Some(bubble_index) = bubble_index else {
        return blocks
            .iter()
            .map(|block| {
                let seed_box = seed_layout_box(block);
                ResolvedLayoutBox {
                    seed_box,
                    layout_box: seed_box,
                    bubble_id: None,
                    #[cfg(test)]
                    diagnostic_branch: if block.lock_layout_box {
                        RendererLayoutBoxBranch::LockedSeed
                    } else {
                        RendererLayoutBoxBranch::Seed
                    },
                }
            })
            .collect();
    };

    let mut counts: HashMap<u8, usize> = HashMap::new();
    let mut matches = Vec::with_capacity(blocks.len());
    #[cfg(test)]
    let mut unmatched_branches = Vec::with_capacity(blocks.len());

    for block in blocks {
        let seed_box = seed_layout_box(block);
        let translation = block.translation.trim();
        let bubble_match = if block.lock_layout_box || translation.is_empty() {
            None
        } else {
            let layout_source = layout_source_from_input(block, translation);
            let writing_mode = writing_mode_for_block(&layout_source);
            bubble_index.lookup_match(seed_box, writing_mode)
        };
        if let Some(matched) = bubble_match {
            *counts.entry(matched.id).or_insert(0) += 1;
        }
        matches.push((seed_box, bubble_match));
        #[cfg(test)]
        unmatched_branches.push(if block.lock_layout_box {
            RendererLayoutBoxBranch::LockedSeed
        } else {
            RendererLayoutBoxBranch::Seed
        });
    }

    // Pre-compute expanded layouts for shared bubbles so blocks
    // receive non-overlapping space that still contains each seed.
    // Key: bubble_id → (seed_box, expanded_box) pairs.
    let mut shared_expanded: HashMap<u8, Vec<(LayoutBox, LayoutBox)>> = HashMap::new();
    for (&bubble_id, &count) in &counts {
        if count <= 1 {
            continue;
        }
        let bubble_layout = matches
            .iter()
            .find_map(|(_, m)| {
                m.as_ref()
                    .filter(|bm| bm.id == bubble_id)
                    .map(|bm| bm.layout_box)
            })
            .unwrap();
        let seeds: Vec<LayoutBox> = matches
            .iter()
            .filter_map(|(seed, m)| {
                m.as_ref()
                    .filter(|bm| bm.id == bubble_id)
                    .map(|_| *seed)
            })
            .collect();
        let n = seeds.len();
        let mut expanded_boxes = Vec::with_capacity(n);
        let slice_w = bubble_layout.width / n as f32;
        for &seed in &seeds {
            let idx = ((seed.x - bubble_layout.x) / slice_w).floor() as usize;
            let clamped = idx.min(n - 1);
            let box_x = bubble_layout.x + slice_w * clamped as f32;
            expanded_boxes.push((seed, LayoutBox {
                x: box_x.min(seed.x),
                y: bubble_layout.y,
                width: slice_w.max(seed.width),
                height: bubble_layout.height.max(seed.y + seed.height - bubble_layout.y),
            }));
        }
        shared_expanded.insert(bubble_id, expanded_boxes);
    }

    #[cfg(test)]
    let mut unmatched_branches = unmatched_branches.into_iter();
    matches
        .into_iter()
        .map(|(seed_box, bubble_match)| {
            #[cfg(test)]
            let unmatched_branch = unmatched_branches.next().unwrap();
            match bubble_match {
                Some(matched) if counts.get(&matched.id).copied().unwrap_or(0) == 1 => {
                    ResolvedLayoutBox {
                        seed_box,
                        layout_box: matched.layout_box,
                        bubble_id: Some(matched.id),
                        #[cfg(test)]
                        diagnostic_branch: RendererLayoutBoxBranch::UniqueBubble,
                    }
                }
                Some(matched) => {
                    // Match by seed_box position to preserve node
                    // identity across input-order changes
                    let candidate = shared_expanded
                        .get(&matched.id)
                        .and_then(|pairs| {
                            pairs.iter().find_map(|(seed, expanded)| {
                                let dx = (seed.x - seed_box.x).abs();
                                let dy = (seed.y - seed_box.y).abs();
                                if dx < 0.5 && dy < 0.5 {
                                    Some(*expanded)
                                } else {
                                    None
                                }
                            })
                        })
                        .unwrap_or(seed_box);
                    let expanded = if candidate.x <= seed_box.x
                        && candidate.y <= seed_box.y
                        && candidate.x + candidate.width >= seed_box.x + seed_box.width
                        && candidate.y + candidate.height >= seed_box.y + seed_box.height
                    {
                        candidate
                    } else {
                        seed_box
                    };
                    ResolvedLayoutBox {
                        seed_box,
                        layout_box: expanded,
                        bubble_id: Some(matched.id),
                        #[cfg(test)]
                        diagnostic_branch: RendererLayoutBoxBranch::SharedBubble,
                    }
                },
                None => ResolvedLayoutBox {
                    seed_box,
                    layout_box: seed_box,
                    bubble_id: None,
                    #[cfg(test)]
                    diagnostic_branch: unmatched_branch,
                },
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers: font families, fallbacks
// ---------------------------------------------------------------------------

fn apply_default_font_families(font_families: &mut Vec<String>, text: &str) {
    if font_families.is_empty() {
        *font_families = font_families_for_text(text);
    }
}

fn load_symbol_fallbacks(fontbook: &mut FontBook) -> Vec<Font> {
    let candidates = [
        "Segoe UI Symbol",
        "Segoe UI Emoji",
        "Noto Sans Symbols",
        "Noto Sans Symbols2",
        "Noto Color Emoji",
        "Apple Color Emoji",
        "Apple Symbols",
        "Symbola",
        "Arial Unicode MS",
    ];
    let faces = fontbook.all_families();
    candidates
        .iter()
        .filter_map(|candidate| face_post_script_name(&faces, candidate))
        .filter_map(|post_script_name| fontbook.query(&post_script_name).ok())
        .collect()
}

fn face_post_script_name(faces: &[FaceInfo], candidate: &str) -> Option<String> {
    let candidate_lower = candidate.trim().to_lowercase();
    faces
        .iter()
        .find(|face| {
            face.post_script_name.to_lowercase() == candidate_lower
                || face
                    .families
                    .iter()
                    .any(|(family, _)| family.to_lowercase() == candidate_lower)
        })
        .map(|face| face.post_script_name.clone())
        .filter(|post_script_name| !post_script_name.is_empty())
}

fn layout_source_from_input(block: &RenderBlockInput, translation: &str) -> RenderBlock {
    RenderBlock {
        x: block.transform.x,
        y: block.transform.y,
        width: block.transform.width.max(1.0),
        height: block.transform.height.max(1.0),
        text: translation.to_string(),
        source_direction: block.source_direction.map(core_direction_to_renderer),
    }
}

fn seed_layout_box(block: &RenderBlockInput) -> LayoutBox {
    LayoutBox {
        x: block.transform.x,
        y: block.transform.y,
        width: block.transform.width.max(1.0),
        height: block.transform.height.max(1.0),
    }
}

// ---------------------------------------------------------------------------
// Helpers: stroke resolution
// ---------------------------------------------------------------------------

fn default_stroke_width(font_size: f32) -> f32 {
    (font_size * 0.10).clamp(1.2, 8.0)
}

fn stroke_padding(stroke: Option<RenderStrokeOptions>) -> f32 {
    stroke
        .filter(|stroke| {
            stroke.width_px.is_finite() && stroke.width_px > 0.0 && stroke.color[3] > 0
        })
        // The stroke radius needs its own space, plus three logical pixels so
        // Lanczos downsampling cannot leave a non-zero alpha tail on the edge.
        .map_or(0.0, |stroke| stroke.width_px.ceil() + 3.0)
}

fn contrasting_stroke_color(text_color: [u8; 4]) -> [u8; 4] {
    let luminance =
        0.299 * text_color[0] as f32 + 0.587 * text_color[1] as f32 + 0.114 * text_color[2] as f32;
    if luminance > 128.0 {
        [0, 0, 0, 255]
    } else {
        [255, 255, 255, 255]
    }
}

fn resolve_stroke_style(
    font_prediction: Option<&FontPrediction>,
    block_stroke: Option<&TextStrokeStyle>,
    global_stroke: Option<&TextStrokeStyle>,
    font_size: f32,
    text_color: [u8; 4],
) -> Option<RenderStrokeOptions> {
    let resolution = resolve_stroke_style_decision(
        font_prediction,
        block_stroke,
        global_stroke,
        font_size,
        text_color,
    );
    #[cfg(test)]
    {
        resolution.stroke
    }
    #[cfg(not(test))]
    {
        resolution
    }
}

fn resolve_stroke_style_decision(
    font_prediction: Option<&FontPrediction>,
    block_stroke: Option<&TextStrokeStyle>,
    global_stroke: Option<&TextStrokeStyle>,
    font_size: f32,
    text_color: [u8; 4],
) -> StrokeResolutionOutput {
    if let Some(stroke) = block_stroke {
        if !stroke.enabled {
            return stroke_resolution(
                None,
                #[cfg(test)]
                RendererStrokeBranch::BlockDisabled,
            );
        }
        return stroke_resolution(
            Some(RenderStrokeOptions {
                color: stroke.color,
                width_px: stroke
                    .width_px
                    .unwrap_or_else(|| default_stroke_width(font_size)),
            }),
            #[cfg(test)]
            RendererStrokeBranch::BlockExplicit,
        );
    }
    if let Some(stroke) = global_stroke {
        if !stroke.enabled {
            return stroke_resolution(
                None,
                #[cfg(test)]
                RendererStrokeBranch::GlobalDisabled,
            );
        }
        return stroke_resolution(
            Some(RenderStrokeOptions {
                color: stroke.color,
                width_px: stroke
                    .width_px
                    .unwrap_or_else(|| default_stroke_width(font_size)),
            }),
            #[cfg(test)]
            RendererStrokeBranch::GlobalExplicit,
        );
    }
    let auto_stroke_color = contrasting_stroke_color(text_color);
    if let Some(pred) = font_prediction {
        if pred.stroke_width_px.is_finite() && pred.stroke_width_px > 0.0 {
            return stroke_resolution(
                Some(RenderStrokeOptions {
                    color: auto_stroke_color,
                    width_px: pred.stroke_width_px,
                }),
                #[cfg(test)]
                RendererStrokeBranch::PredictedWidth,
            );
        }
        return stroke_resolution(
            None,
            #[cfg(test)]
            RendererStrokeBranch::PredictedNoStroke,
        );
    }
    stroke_resolution(
        Some(RenderStrokeOptions {
            color: auto_stroke_color,
            width_px: default_stroke_width(font_size),
        }),
        #[cfg(test)]
        RendererStrokeBranch::AutomaticDefault,
    )
}

fn stroke_resolution(
    stroke: Option<RenderStrokeOptions>,
    #[cfg(test)] branch: RendererStrokeBranch,
) -> StrokeResolutionOutput {
    #[cfg(test)]
    {
        StrokeResolution { stroke, branch }
    }
    #[cfg(not(test))]
    {
        stroke
    }
}

fn resolve_text_color(
    explicit_style: Option<&TextStyle>,
    derived_style: &TextStyle,
    font_prediction: Option<&FontPrediction>,
) -> [u8; 4] {
    let resolution = resolve_text_color_decision(explicit_style, derived_style, font_prediction);
    #[cfg(test)]
    {
        resolution.color
    }
    #[cfg(not(test))]
    {
        resolution
    }
}

fn resolve_text_color_decision(
    explicit_style: Option<&TextStyle>,
    derived_style: &TextStyle,
    font_prediction: Option<&FontPrediction>,
) -> FillResolutionOutput {
    if explicit_style.is_some() {
        return fill_resolution(
            derived_style.color,
            #[cfg(test)]
            RendererFillBranch::Explicit,
        );
    }
    if let Some(pred) = font_prediction {
        return fill_resolution(
            [
                pred.text_color[0],
                pred.text_color[1],
                pred.text_color[2],
                255,
            ],
            #[cfg(test)]
            RendererFillBranch::Predicted,
        );
    }
    fill_resolution(
        [0, 0, 0, 255],
        #[cfg(test)]
        RendererFillBranch::DefaultBlack,
    )
}

fn fill_resolution(
    color: [u8; 4],
    #[cfg(test)] branch: RendererFillBranch,
) -> FillResolutionOutput {
    #[cfg(test)]
    {
        FillResolution { color, branch }
    }
    #[cfg(not(test))]
    {
        color
    }
}

#[cfg(test)]
fn renderer_alpha_summary(image: &RgbaImage) -> (Option<RendererAlphaBbox>, u64, String) {
    let mut alpha =
        Vec::with_capacity((image.width() as usize).saturating_mul(image.height() as usize));
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut nonzero = 0_u64;
    for (x, y, pixel) in image.enumerate_pixels() {
        let value = pixel.0[3];
        alpha.push(value);
        if value != 0 {
            nonzero += 1;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    let bbox = (nonzero != 0).then_some(RendererAlphaBbox {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    });
    (bbox, nonzero, blake3::hash(&alpha).to_hex().to_string())
}

#[cfg(test)]
fn renderer_font_outcome(block: &RenderBlockInput) -> RendererFieldOutcome {
    if block.typography_plan_verified
        && block
            .style
            .as_ref()
            .and_then(|style| style.font_size)
            .is_some()
    {
        RendererFieldOutcome::IgnoredByPolicy
    } else if block.font_prediction.as_ref().is_some_and(|prediction| {
        prediction.font_size_px.is_finite() && prediction.font_size_px > 0.0
    }) {
        RendererFieldOutcome::Prediction
    } else {
        RendererFieldOutcome::Default
    }
}

#[cfg(test)]
fn renderer_fill_outcome(branch: RendererFillBranch) -> RendererFieldOutcome {
    match branch {
        RendererFillBranch::Explicit => RendererFieldOutcome::ManualOverride,
        RendererFillBranch::Predicted => RendererFieldOutcome::Prediction,
        RendererFillBranch::DefaultBlack => RendererFieldOutcome::Default,
    }
}

#[cfg(test)]
fn renderer_stroke_outcome(branch: RendererStrokeBranch) -> RendererFieldOutcome {
    match branch {
        RendererStrokeBranch::BlockExplicit | RendererStrokeBranch::BlockDisabled => {
            RendererFieldOutcome::ManualOverride
        }
        RendererStrokeBranch::PredictedWidth | RendererStrokeBranch::PredictedNoStroke => {
            RendererFieldOutcome::Prediction
        }
        _ => RendererFieldOutcome::Default,
    }
}

#[cfg(test)]
fn renderer_final_font_size(font_size: f32) -> Result<u32> {
    anyhow::ensure!(
        font_size.is_finite() && font_size >= 0.0 && font_size <= u32::MAX as f32,
        "invalid final renderer font size"
    );
    Ok((font_size + 0.5).floor() as u32)
}

// ---------------------------------------------------------------------------
// Helpers: type conversions
// ---------------------------------------------------------------------------

fn shader_core_to_renderer(e: TextShaderEffect) -> RendererEffect {
    RendererEffect {
        italic: e.italic,
        bold: e.bold,
    }
}

fn core_align_to_renderer(a: koharu_core::TextAlign) -> RendererTextAlign {
    match a {
        koharu_core::TextAlign::Left => RendererTextAlign::Left,
        koharu_core::TextAlign::Center => RendererTextAlign::Center,
        koharu_core::TextAlign::Right => RendererTextAlign::Right,
    }
}

fn core_direction_to_renderer(d: TextDirection) -> RendererTextDirection {
    match d {
        TextDirection::Horizontal => RendererTextDirection::Horizontal,
        TextDirection::Vertical => RendererTextDirection::Vertical,
    }
}

fn rendered_direction_for_writing_mode(writing_mode: WritingMode) -> TextDirection {
    match writing_mode {
        WritingMode::Horizontal => TextDirection::Horizontal,
        WritingMode::VerticalRl => TextDirection::Vertical,
    }
}

// ---------------------------------------------------------------------------
// Helpers: placement
// ---------------------------------------------------------------------------

fn centred_sprite_transform(
    anchor_box: LayoutBox,
    sprite_width: u32,
    sprite_height: u32,
    rotation_deg: f32,
) -> Transform {
    let sprite_w = sprite_width as f32;
    let sprite_h = sprite_height as f32;
    let cx = anchor_box.x + anchor_box.width * 0.5;
    let cy = anchor_box.y + anchor_box.height * 0.5;
    Transform {
        x: (cx - sprite_w * 0.5).round(),
        y: (cy - sprite_h * 0.5).round(),
        width: sprite_w,
        height: sprite_h,
        rotation_deg,
    }
}

fn find_input(blocks: &[RenderBlockInput], id: NodeId) -> &RenderBlockInput {
    blocks
        .iter()
        .find(|b| b.node_id == id)
        .expect("rendered_block must have matching input")
}

pub(crate) fn placement_origin(
    input: &RenderBlockInput,
    expanded: &Option<Transform>,
) -> (f32, f32) {
    if let Some(t) = expanded {
        (t.x.round(), t.y.round())
    } else {
        (input.transform.x, input.transform.y)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Barrier;

    use super::*;
    use image::{GenericImageView, GrayImage, Luma, Rgba, RgbaImage};
    use koharu_core::NodeId;

    fn start_renderer_diagnostic_capture() -> RendererDiagnosticCapture {
        loop {
            match crate::renderer::RendererDiagnosticCapture::start() {
                Ok(capture) => return capture,
                Err(RendererDiagnosticCaptureActive) => std::thread::yield_now(),
            }
        }
    }

    fn assert_render_outputs_equal(left: &RenderOutput, right: &RenderOutput) {
        assert_eq!(
            left.final_render.dimensions(),
            right.final_render.dimensions()
        );
        assert_eq!(
            left.final_render.to_rgba8().as_raw(),
            right.final_render.to_rgba8().as_raw()
        );
        assert_eq!(left.blocks.len(), right.blocks.len());
        for (left, right) in left.blocks.iter().zip(&right.blocks) {
            assert_eq!(left.node_id, right.node_id);
            assert_eq!(
                left.sprite.to_rgba8().as_raw(),
                right.sprite.to_rgba8().as_raw()
            );
            assert_eq!(left.rendered_direction, right.rendered_direction);
            match (&left.expanded_transform, &right.expanded_transform) {
                (Some(left), Some(right)) => {
                    assert_eq!(left.x, right.x);
                    assert_eq!(left.y, right.y);
                    assert_eq!(left.width, right.width);
                    assert_eq!(left.height, right.height);
                    assert_eq!(left.rotation_deg, right.rotation_deg);
                }
                (None, None) => {}
                _ => panic!("expanded transform presence changed"),
            }
        }
    }

    type IndependentAlphaSummary = (Option<(u32, u32, u32, u32)>, u64, String);

    fn independent_alpha_summary(image: &RgbaImage) -> IndependentAlphaSummary {
        let alpha = image.pixels().map(|pixel| pixel.0[3]).collect::<Vec<_>>();
        let nonzero = alpha.iter().filter(|value| **value != 0).count() as u64;
        let points = image
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel.0[3] != 0)
            .map(|(x, y, _)| (x, y))
            .collect::<Vec<_>>();
        let bbox = (!points.is_empty()).then(|| {
            let min_x = points.iter().map(|(x, _)| *x).min().unwrap();
            let min_y = points.iter().map(|(_, y)| *y).min().unwrap();
            let max_x = points.iter().map(|(x, _)| *x).max().unwrap();
            let max_y = points.iter().map(|(_, y)| *y).max().unwrap();
            (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
        });
        (bbox, nonzero, blake3::hash(&alpha).to_hex().to_string())
    }

    fn assert_diagnostic_matches_sprite(event: &RendererDiagnosticEvent, sprite: &DynamicImage) {
        let sprite = sprite.to_rgba8();
        let (bbox, nonzero, hash) = independent_alpha_summary(&sprite);
        assert_eq!(
            (event.sprite_width, event.sprite_height),
            sprite.dimensions()
        );
        assert_eq!(event.alpha_nonzero_pixels, nonzero);
        assert_eq!(event.alpha_blake3, hash);
        assert_eq!(
            event.sprite_rgba_blake3,
            blake3::hash(sprite.as_raw()).to_hex().to_string()
        );
        assert_eq!(
            event
                .alpha_bbox
                .as_ref()
                .map(|bbox| (bbox.x, bbox.y, bbox.width, bbox.height)),
            bbox
        );
        assert!(
            [
                event.resolver_record_ptr,
                event.fit_record_ptr,
                event.postvalidate_record_ptr,
            ]
            .into_iter()
            .all(|record_ptr| record_ptr != 0)
        );
        assert_eq!(event.resolver_box, event.fit_box);
        assert_eq!(event.fit_box, event.postvalidate_box);
        assert_eq!(
            event.resolver_box_blake3,
            renderer_box_digest(event.resolver_box)
        );
        assert_eq!(event.resolver_box_blake3, event.fit_box_blake3);
        assert_eq!(event.fit_box_blake3, event.postvalidate_box_blake3);
        assert_eq!(
            event.final_font_size_px,
            renderer_final_font_size(event.final_size).unwrap()
        );
    }

    fn render_with_diagnostics(
        renderer: &Renderer,
        blocks: &[RenderBlockInput],
        bubble_mask: Option<&DynamicImage>,
        image_width: u32,
        image_height: u32,
        options: &PageRenderOptions,
    ) -> Result<(RenderOutput, Vec<RendererDiagnosticEvent>)> {
        let inpainted = DynamicImage::new_rgba8(image_width, image_height);
        let inactive = renderer.render_page(
            &inpainted,
            None,
            bubble_mask,
            image_width,
            image_height,
            blocks,
            options,
        )?;
        let capture = start_renderer_diagnostic_capture();
        let active = renderer.render_page(
            &inpainted,
            None,
            bubble_mask,
            image_width,
            image_height,
            blocks,
            options,
        )?;
        assert_render_outputs_equal(&inactive, &active);
        let events = capture.take();
        assert_eq!(events.len(), active.blocks.len());
        for (event, block) in events.iter().zip(&active.blocks) {
            assert_diagnostic_matches_sprite(event, &block.sprite);
        }
        Ok((active, events))
    }

    #[test]
    fn renderer_diagnostics_capture_source_layout_and_sprite_without_output_drift() -> Result<()> {
        let renderer = Renderer::new()?;
        let transform = Transform {
            x: 20.0,
            y: 30.0,
            width: 240.0,
            height: 96.0,
            rotation_deg: 7.0,
        };
        let block = automatic_test_block(
            &renderer,
            transform,
            "Observed",
            40.0,
            28.0,
            TextDirection::Horizontal,
        );
        let options = PageRenderOptions {
            source_relative_font_size_policy: Some(SourceRelativeFontSizePolicy {
                offset: -5.0,
                prefer_detected: true,
            }),
            ..Default::default()
        };
        let (active, events): (_, Vec<crate::renderer::RendererDiagnosticEvent>) =
            render_with_diagnostics(
                &renderer,
                std::slice::from_ref(&block),
                None,
                320,
                200,
                &options,
            )?;
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.node_id, block.node_id);
        assert_eq!(event.source_geometry_estimate, 96.0);
        assert_eq!(event.valid_detected_size, Some(40.0));
        assert_eq!(event.valid_predicted_size, Some(28.0));
        assert_eq!(event.source_size_branch, RendererSourceSizeBranch::Detected);
        assert_eq!(event.policy_offset, -5.0);
        assert_eq!(event.candidate_size, 35.0);
        let expected_auto_max =
            max_font_size_for_box(seed_layout_box(&block), min_font_size_for_image(320, 200));
        assert!((event.auto_max - expected_auto_max).abs() <= f32::EPSILON);
        assert_eq!(event.cap, 35.0);
        assert_eq!(
            (event.resolved_layout_width, event.resolved_layout_height),
            (240.0, 96.0)
        );
        assert_eq!(event.layout_box_branch, RendererLayoutBoxBranch::LockedSeed);
        assert!(event.tight_layout_width <= event.resolved_layout_width);
        assert!(event.tight_layout_height <= event.resolved_layout_height);
        assert_eq!(event.group_size, Some(event.independent_size));
        assert_eq!(event.final_size, event.independent_size);
        assert_eq!(event.rotation_deg, 7.0);

        assert_diagnostic_matches_sprite(event, &active.blocks[0].sprite);

        let value = serde_json::to_value(event)?;
        let mut actual_fields = value
            .as_object()
            .context("renderer diagnostic object")?
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        actual_fields.sort_unstable();
        let mut expected_fields = [
            "alpha_bbox",
            "alpha_blake3",
            "alpha_nonzero_pixels",
            "auto_max",
            "builder_publication_count",
            "builder_raster_count",
            "candidate_size",
            "cap",
            "fill_branch",
            "fill_outcome",
            "final_font_size_px",
            "final_size",
            "fit_box",
            "fit_box_blake3",
            "fit_record_ptr",
            "font_outcome",
            "group_size",
            "independent_size",
            "layout_box_branch",
            "node_id",
            "policy_offset",
            "postvalidate_box",
            "postvalidate_box_blake3",
            "postvalidate_record_ptr",
            "predicted_fill_rgb",
            "predicted_stroke_rgb",
            "predicted_stroke_width",
            "renderer_rebuild_count",
            "resolved_fill_rgba",
            "resolved_layout_height",
            "resolved_layout_width",
            "resolved_stroke_rgba",
            "resolved_stroke_width",
            "resolver_box",
            "resolver_box_blake3",
            "resolver_record_ptr",
            "rotation_deg",
            "source_geometry_estimate",
            "source_size_branch",
            "sprite_height",
            "sprite_rgba_blake3",
            "sprite_width",
            "stroke_branch",
            "stroke_outcome",
            "tight_layout_height",
            "tight_layout_width",
            "valid_detected_size",
            "valid_predicted_size",
        ];
        expected_fields.sort_unstable();
        assert_eq!(actual_fields, expected_fields);
        assert_eq!(
            serde_json::from_value::<RendererDiagnosticEvent>(value.clone())?,
            *event
        );
        let serialized = serde_json::to_string(&value)?;
        for forbidden in [
            block.translation.as_str(),
            "translation",
            "text",
            "path",
            "font_path",
            "font_family",
            "target",
            "elapsed",
            "timestamp",
        ] {
            assert!(!serialized.contains(forbidden));
        }
        for field in [
            "valid_detected_size",
            "valid_predicted_size",
            "group_size",
            "predicted_fill_rgb",
            "predicted_stroke_rgb",
            "predicted_stroke_width",
            "resolved_stroke_rgba",
            "resolved_stroke_width",
            "alpha_bbox",
        ] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<RendererDiagnosticEvent>(missing).is_err(),
                "missing required option field {field}"
            );
            let mut explicit_null = value.clone();
            explicit_null[field] = serde_json::Value::Null;
            assert!(
                serde_json::from_value::<RendererDiagnosticEvent>(explicit_null).is_ok(),
                "explicit null option field {field}"
            );
        }
        let mut unknown_bbox = value.clone();
        assert!(unknown_bbox["alpha_bbox"].is_object());
        unknown_bbox["alpha_bbox"]["unexpected"] = true.into();
        assert!(serde_json::from_value::<RendererDiagnosticEvent>(unknown_bbox).is_err());
        let mut unknown = value;
        unknown["unexpected"] = true.into();
        assert!(serde_json::from_value::<RendererDiagnosticEvent>(unknown).is_err());
        Ok(())
    }

    #[test]
    fn renderer_diagnostics_color_and_stroke_branches_match_real_resolution() -> Result<()> {
        let renderer = Renderer::new()?;
        let make_transform = |y| Transform {
            x: 20.0,
            y,
            width: 220.0,
            height: 80.0,
            rotation_deg: 0.0,
        };
        let explicit = automatic_test_block(
            &renderer,
            make_transform(0.0),
            "Explicit",
            30.0,
            28.0,
            TextDirection::Horizontal,
        );
        let mut predicted = automatic_test_block(
            &renderer,
            make_transform(120.0),
            "Predicted",
            30.0,
            28.0,
            TextDirection::Horizontal,
        );
        predicted.lock_layout_box = false;
        predicted.style = None;
        predicted.detected_font_size_px = None;
        predicted.font_prediction = Some(FontPrediction {
            text_color: [240, 240, 240],
            stroke_color: [12, 34, 56],
            stroke_width_px: 3.0,
            font_size_px: 28.0,
            ..Default::default()
        });
        let mut defaulted = automatic_test_block(
            &renderer,
            make_transform(240.0),
            "Default",
            30.0,
            28.0,
            TextDirection::Horizontal,
        );
        defaulted.lock_layout_box = false;
        defaulted.style = None;
        defaulted.detected_font_size_px = None;
        defaulted.font_prediction = None;
        let blocks = [explicit, predicted.clone(), defaulted];
        let options = PageRenderOptions {
            source_relative_font_size_policy: Some(SourceRelativeFontSizePolicy {
                offset: 0.0,
                prefer_detected: true,
            }),
            ..Default::default()
        };
        let (output, events) =
            render_with_diagnostics(&renderer, &blocks, None, 320, 360, &options)?;
        assert_eq!(output.blocks.len(), 3);
        assert_eq!(events.len(), 3);

        assert_eq!(events[0].fill_branch, RendererFillBranch::Explicit);
        assert_eq!(events[0].stroke_branch, RendererStrokeBranch::BlockExplicit);
        assert_eq!(events[0].resolved_fill_rgba, [255, 255, 255, 255]);
        assert_eq!(events[0].resolved_stroke_rgba, Some([0, 0, 0, 255]));

        assert_eq!(events[1].fill_branch, RendererFillBranch::Predicted);
        assert_eq!(
            events[1].stroke_branch,
            RendererStrokeBranch::PredictedWidth
        );
        assert_eq!(
            events[1].source_size_branch,
            RendererSourceSizeBranch::Predicted
        );
        assert_eq!(events[1].layout_box_branch, RendererLayoutBoxBranch::Seed);
        assert_eq!(events[1].predicted_fill_rgb, Some([240, 240, 240]));
        assert_eq!(events[1].predicted_stroke_rgb, Some([12, 34, 56]));
        assert_eq!(events[1].predicted_stroke_width, Some(3.0));
        assert_eq!(events[1].resolved_fill_rgba, [240, 240, 240, 255]);
        assert_eq!(events[1].resolved_stroke_rgba, Some([0, 0, 0, 255]));
        assert_ne!(
            events[1].predicted_stroke_rgb.unwrap(),
            events[1].resolved_stroke_rgba.unwrap()[..3]
        );
        assert_eq!(events[1].resolved_stroke_width, Some(3.0));

        assert_eq!(events[2].fill_branch, RendererFillBranch::DefaultBlack);
        assert_eq!(
            events[2].stroke_branch,
            RendererStrokeBranch::AutomaticDefault
        );
        assert_eq!(
            events[2].source_size_branch,
            RendererSourceSizeBranch::GeometryFallback
        );
        assert_eq!(events[2].layout_box_branch, RendererLayoutBoxBranch::Seed);
        assert_eq!(events[2].resolved_fill_rgba, [0, 0, 0, 255]);
        assert_eq!(events[2].resolved_stroke_rgba, Some([255, 255, 255, 255]));

        let mut block_disabled = automatic_test_block(
            &renderer,
            make_transform(0.0),
            "Block disabled",
            30.0,
            28.0,
            TextDirection::Horizontal,
        );
        block_disabled.style.as_mut().expect("test style").stroke = Some(TextStrokeStyle {
            enabled: false,
            color: [1, 2, 3, 255],
            width_px: Some(4.0),
        });
        let (_, events) =
            render_with_diagnostics(&renderer, &[block_disabled], None, 320, 120, &options)?;
        assert_eq!(events[0].stroke_branch, RendererStrokeBranch::BlockDisabled);
        assert_eq!(events[0].resolved_stroke_rgba, None);

        let mut global_block = automatic_test_block(
            &renderer,
            make_transform(0.0),
            "Global",
            30.0,
            28.0,
            TextDirection::Horizontal,
        );
        global_block.style.as_mut().expect("test style").stroke = None;
        let global_disabled = PageRenderOptions {
            shader_stroke: Some(TextStrokeStyle {
                enabled: false,
                color: [9, 8, 7, 255],
                width_px: Some(5.0),
            }),
            ..options.clone()
        };
        let (_, events) = render_with_diagnostics(
            &renderer,
            std::slice::from_ref(&global_block),
            None,
            320,
            120,
            &global_disabled,
        )?;
        assert_eq!(
            events[0].stroke_branch,
            RendererStrokeBranch::GlobalDisabled
        );
        assert_eq!(events[0].resolved_stroke_rgba, None);

        let global_explicit = PageRenderOptions {
            shader_stroke: Some(TextStrokeStyle {
                enabled: true,
                color: [9, 8, 7, 255],
                width_px: Some(5.0),
            }),
            ..options.clone()
        };
        let (_, events) = render_with_diagnostics(
            &renderer,
            std::slice::from_ref(&global_block),
            None,
            320,
            120,
            &global_explicit,
        )?;
        assert_eq!(
            events[0].stroke_branch,
            RendererStrokeBranch::GlobalExplicit
        );
        assert_eq!(events[0].resolved_stroke_rgba, Some([9, 8, 7, 255]));
        assert_eq!(events[0].resolved_stroke_width, Some(5.0));

        for invalid_width in [0.0, -1.0, f32::NAN] {
            let mut no_stroke = predicted.clone();
            no_stroke
                .font_prediction
                .as_mut()
                .expect("test prediction")
                .stroke_width_px = invalid_width;
            let (_, events) =
                render_with_diagnostics(&renderer, &[no_stroke], None, 320, 120, &options)?;
            assert_eq!(
                events[0].stroke_branch,
                RendererStrokeBranch::PredictedNoStroke
            );
            assert_eq!(events[0].resolved_stroke_rgba, None);
            assert_eq!(events[0].resolved_stroke_width, None);
            if invalid_width.is_finite() {
                assert_eq!(events[0].predicted_stroke_width, Some(invalid_width));
            } else {
                assert_eq!(events[0].predicted_stroke_width, None);
            }
        }
        Ok(())
    }

    #[test]
    fn renderer_diagnostics_layout_grouping_and_failure_paths_are_factual() -> Result<()> {
        let renderer = Renderer::new()?;
        let options = PageRenderOptions {
            source_relative_font_size_policy: Some(SourceRelativeFontSizePolicy {
                offset: 0.0,
                prefer_detected: true,
            }),
            ..Default::default()
        };

        let mut unique = automatic_test_block(
            &renderer,
            Transform {
                x: 70.0,
                y: 70.0,
                width: 30.0,
                height: 40.0,
                rotation_deg: 1.0,
            },
            "Unique",
            28.0,
            24.0,
            TextDirection::Horizontal,
        );
        unique.lock_layout_box = false;
        let mut unique_mask = GrayImage::from_pixel(200, 200, Luma([0u8]));
        paint_rect(&mut unique_mask, 20, 20, 180, 180, 1);
        let unique_mask = DynamicImage::ImageLuma8(unique_mask);
        let (_, unique_events) = render_with_diagnostics(
            &renderer,
            std::slice::from_ref(&unique),
            Some(&unique_mask),
            200,
            200,
            &options,
        )?;
        assert_eq!(
            unique_events[0].layout_box_branch,
            RendererLayoutBoxBranch::UniqueBubble
        );

        let mut shared_left = automatic_test_block(
            &renderer,
            Transform {
                x: 30.0,
                y: 50.0,
                width: 75.0,
                height: 80.0,
                rotation_deg: 2.0,
            },
            "Left",
            28.0,
            24.0,
            TextDirection::Horizontal,
        );
        shared_left.lock_layout_box = false;
        let mut shared_right = automatic_test_block(
            &renderer,
            Transform {
                x: 150.0,
                y: 50.0,
                width: 85.0,
                height: 80.0,
                rotation_deg: 3.0,
            },
            "Right",
            28.0,
            24.0,
            TextDirection::Horizontal,
        );
        shared_right.lock_layout_box = false;
        let shared_blocks = [shared_left, shared_right];
        let mut shared_mask = GrayImage::from_pixel(280, 200, Luma([0u8]));
        paint_rect(&mut shared_mask, 10, 10, 270, 190, 1);
        let shared_mask = DynamicImage::ImageLuma8(shared_mask);
        let (_, shared_events) = render_with_diagnostics(
            &renderer,
            &shared_blocks,
            Some(&shared_mask),
            280,
            200,
            &options,
        )?;
        assert_eq!(
            shared_events
                .iter()
                .map(|event| event.layout_box_branch)
                .collect::<Vec<_>>(),
            [
                RendererLayoutBoxBranch::SharedBubble,
                RendererLayoutBoxBranch::SharedBubble
            ]
        );
        assert!(
            shared_events.iter().all(|e| e.resolved_layout_width >= 75.0),
            "shared-bubble blocks receive expanded width ≥ seed width"
        );
        assert_eq!(
            shared_events
                .iter()
                .map(|event| event.rotation_deg)
                .collect::<Vec<_>>(),
            [2.0, 3.0]
        );

        let common_source = Transform {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
            rotation_deg: 0.0,
        };
        let mut roomy = automatic_test_block(
            &renderer,
            Transform {
                x: 10.0,
                y: 10.0,
                width: 280.0,
                height: 100.0,
                rotation_deg: 4.0,
            },
            "Wide",
            40.0,
            36.0,
            TextDirection::Horizontal,
        );
        roomy.source_transform = common_source;
        let mut constrained = automatic_test_block(
            &renderer,
            Transform {
                x: 10.0,
                y: 130.0,
                width: 120.0,
                height: 60.0,
                rotation_deg: 5.0,
            },
            "Narrow words",
            40.0,
            36.0,
            TextDirection::Horizontal,
        );
        constrained.source_transform = common_source;
        let grouped_blocks = [roomy, constrained];
        let (_, grouped_events) =
            render_with_diagnostics(&renderer, &grouped_blocks, None, 320, 220, &options)?;
        let group_size = grouped_events[0]
            .group_size
            .expect("horizontal source row must be grouped");
        assert_eq!(grouped_events[1].group_size, Some(group_size));
        assert!(
            grouped_events
                .iter()
                .any(|event| event.independent_size > event.final_size)
        );
        assert!(
            grouped_events
                .iter()
                .all(|event| event.final_size == group_size)
        );
        assert_eq!(
            grouped_events
                .iter()
                .map(|event| event.rotation_deg)
                .collect::<Vec<_>>(),
            [4.0, 5.0]
        );

        let vertical = automatic_test_block(
            &renderer,
            Transform {
                x: 20.0,
                y: 20.0,
                width: 90.0,
                height: 180.0,
                rotation_deg: 6.0,
            },
            "縦書き",
            28.0,
            24.0,
            TextDirection::Vertical,
        );
        let (_, vertical_events) =
            render_with_diagnostics(&renderer, &[vertical], None, 160, 240, &options)?;
        assert_eq!(vertical_events[0].group_size, None);

        let failed = automatic_test_block(
            &renderer,
            Transform {
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 80.0,
                rotation_deg: 0.0,
            },
            "Failure",
            28.0,
            24.0,
            TextDirection::Horizontal,
        );
        let resolved = resolve_layout_boxes(std::slice::from_ref(&failed), None)[0];
        let prepared = renderer
            .prepare_source_relative_automatic(
                &failed,
                resolved,
                &options,
                options
                    .source_relative_font_size_policy
                    .expect("test policy"),
                min_font_size_for_image(240, 120),
            )?
            .expect("test block must prepare");
        let capture = start_renderer_diagnostic_capture();
        let error = renderer.render_prepared_source_relative(
            &failed,
            &prepared,
            prepared.independent_font_size + 1.0,
            Some(prepared.independent_font_size),
            None,
            RasterOptions::default(),
        );
        assert!(error.is_err());
        assert!(capture.take().is_empty());
        Ok(())
    }

    #[test]
    fn renderer_diagnostics_owner_thread_nested_and_unwind_contract() -> Result<()> {
        let renderer = Renderer::new()?;
        let block = automatic_test_block(
            &renderer,
            Transform {
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 80.0,
                rotation_deg: 0.0,
            },
            "Owner",
            28.0,
            24.0,
            TextDirection::Horizontal,
        );
        let capture = start_renderer_diagnostic_capture();
        assert!(matches!(
            RendererDiagnosticCapture::start(),
            Err(RendererDiagnosticCaptureActive)
        ));
        render_automatic_test_blocks(&renderer, std::slice::from_ref(&block), 240, 120, 0.0)?;

        let barrier = Arc::new(Barrier::new(2));
        let foreign_barrier = barrier.clone();
        let foreign = std::thread::spawn(move || -> Result<()> {
            let renderer = Renderer::new()?;
            let block = automatic_test_block(
                &renderer,
                Transform {
                    x: 0.0,
                    y: 0.0,
                    width: 180.0,
                    height: 80.0,
                    rotation_deg: 0.0,
                },
                "Foreign",
                28.0,
                24.0,
                TextDirection::Horizontal,
            );
            foreign_barrier.wait();
            render_automatic_test_blocks(&renderer, &[block], 240, 120, 0.0)?;
            foreign_barrier.wait();
            Ok(())
        });
        barrier.wait();
        barrier.wait();
        foreign.join().unwrap()?;
        render_automatic_test_blocks(&renderer, std::slice::from_ref(&block), 240, 120, 0.0)?;
        assert_eq!(capture.take().len(), 2);
        drop(capture);

        let unwind = std::panic::catch_unwind(|| {
            let _capture = start_renderer_diagnostic_capture();
            panic!("intentional renderer diagnostic unwind");
        });
        assert!(unwind.is_err());
        let restarted = start_renderer_diagnostic_capture();
        let coordination = RENDERER_DIAGNOSTIC_SINK.get().unwrap();
        let coordination_poison = std::thread::spawn(move || {
            let _guard = coordination
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            panic!("intentional renderer diagnostic coordination poison");
        });
        assert!(coordination_poison.join().is_err());

        let events = restarted.events.clone();
        let event_poison = std::thread::spawn(move || {
            let _guard = events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            panic!("intentional renderer diagnostic event poison");
        });
        assert!(event_poison.join().is_err());

        render_automatic_test_blocks(&renderer, std::slice::from_ref(&block), 240, 120, 0.0)?;
        assert_eq!(restarted.take().len(), 1);
        drop(restarted);
        let final_restart = start_renderer_diagnostic_capture();
        assert!(final_restart.take().is_empty());
        Ok(())
    }

    #[test]
    fn available_fonts_uses_default_google_face_boundary() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = camino::Utf8Path::from_path(temp.path()).expect("temp path should be UTF-8");
        let cached_path = root
            .join("fonts/google/abeezee")
            .join("ABeeZee-Regular.ttf");
        std::fs::create_dir_all(
            cached_path
                .parent()
                .expect("cached font should have a parent"),
        )?;
        std::fs::write(cached_path, b"cached")?;

        let mut fontbook = FontBook::new();
        let symbol_fallbacks = load_symbol_fallbacks(&mut fontbook);
        let renderer = Renderer {
            fontbook: Arc::new(Mutex::new(fontbook)),
            renderer: TinySkiaRenderer::new()?,
            symbol_fallbacks,
            google_fonts: Arc::new(GoogleFontService::new(root)?),
        };

        let fonts = renderer.available_fonts()?;
        assert!(fonts.iter().any(|font| {
            font.post_script_name == "Noto Sans SC:400"
                && font.source == FontSource::Google
                && !font.cached
        }));
        assert!(fonts.iter().any(|font| {
            font.post_script_name == "ABeeZee:400"
                && font.source == FontSource::Google
                && font.cached
        }));
        assert!(!fonts.iter().any(|font| font.post_script_name == "Abel:400"));
        Ok(())
    }

    #[test]
    fn default_font_families_should_fill_empty_list() {
        let mut font_families = Vec::new();
        apply_default_font_families(&mut font_families, "hello");
        assert!(!font_families.is_empty());
    }

    #[test]
    fn typography_planner_font_size_is_fit_cap_not_explicit_override() {
        let mut planned = block(0.0, 0.0, 100.0, 40.0, "planned text");
        planned.style = Some(TextStyle {
            font_size: Some(18.0),
            ..Default::default()
        });
        planned.typography_plan_verified = true;
        let layout_box = seed_layout_box(&planned);

        let (explicit, max) = font_size_constraints(&planned, layout_box, 12.0, None);
        assert_eq!(explicit, None);
        assert_eq!(max, 18.0);

        planned.typography_plan_verified = false;
        let (explicit, _) = font_size_constraints(&planned, layout_box, 12.0, None);
        assert_eq!(explicit, Some(18.0));
    }

    #[test]
    fn hanonly_pre_greenc_red_t3_planner_font_outcome_contract() -> Result<()> {
        let _diagnostic_lock = crate::pipeline::lock_diagnostic_capture_test();
        let renderer = Renderer::new()?;
        let mut planned = automatic_test_block(
            &renderer,
            Transform {
                x: 20.0,
                y: 20.0,
                width: 300.0,
                height: 200.0,
                rotation_deg: 0.0,
            },
            "planned text",
            40.0,
            36.0,
            TextDirection::Horizontal,
        );
        planned.style.as_mut().context("test style")?.font_size = Some(18.0);
        planned.typography_plan_verified = true;
        let han_only_options = PageRenderOptions {
            source_relative_font_size_policy: Some(SourceRelativeFontSizePolicy {
                offset: 0.0,
                prefer_detected: true,
            }),
            ..Default::default()
        };

        clear_raster_trace();
        let (automatic, events) = render_with_diagnostics(
            &renderer,
            std::slice::from_ref(&planned),
            None,
            340,
            240,
            &han_only_options,
        )?;
        let [event] = events.as_slice() else {
            panic!("expected one automatic diagnostic");
        };
        assert_eq!(event.node_id, planned.node_id);
        assert_eq!(event.font_outcome, RendererFieldOutcome::IgnoredByPolicy);
        assert_eq!(event.candidate_size, 40.0);
        assert_eq!(event.cap, 40.0);
        assert_eq!(event.independent_size, 40.0);
        assert_eq!(event.group_size, Some(40.0));
        assert_eq!(event.final_size, 40.0);
        assert_eq!(event.final_font_size_px, 40);
        assert_diagnostic_matches_sprite(event, &automatic.blocks[0].sprite);

        clear_raster_trace();
        let all_text = renderer.render_page(
            &DynamicImage::new_rgba8(340, 240),
            None,
            None,
            340,
            240,
            std::slice::from_ref(&planned),
            &PageRenderOptions::default(),
        )?;
        assert_eq!(
            raster_trace(),
            vec![RasterTrace {
                node_id: planned.node_id,
                path: RasterPath::Legacy,
                font_size: 18.0,
            }],
            "AllText must keep the existing verified Planner font cap"
        );

        let mut manual = planned.clone();
        manual.node_id = NodeId::new();
        manual.typography_plan_verified = false;
        clear_raster_trace();
        let manual_output = renderer.render_page(
            &DynamicImage::new_rgba8(340, 240),
            None,
            None,
            340,
            240,
            std::slice::from_ref(&manual),
            &han_only_options,
        )?;
        assert_eq!(
            raster_trace(),
            vec![RasterTrace {
                node_id: manual.node_id,
                path: RasterPath::Legacy,
                font_size: 18.0,
            }],
            "manual font size must remain authoritative and outside HanOnly automatic layout"
        );
        assert_eq!(
            all_text.blocks[0].sprite.to_rgba8().as_raw(),
            manual_output.blocks[0].sprite.to_rgba8().as_raw()
        );
        assert_eq!(event.builder_publication_count, 1);
        assert_eq!(event.builder_raster_count, 1);
        assert_eq!(event.renderer_rebuild_count, 1);
        Ok(())
    }

    #[test]
    fn typography_planner_font_cap_never_exceeds_auto_fit_normal_and_bubble() -> Result<()> {
        let font = any_system_font();
        let builder = TextLayout::new(&font, None);
        let layout_box = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 100.0,
        };
        let normal = fit_font_size(&builder, "short", layout_box, None, 12.0, 18.0, false)?;
        assert!(normal.font_size <= 18.0);

        let mask = GrayImage::from_pixel(220, 120, Luma([1]));
        let mut rendered_size = 0.0;
        let mut render_candidate = |layout: &LayoutRun<'_>| -> Result<RenderedTextCandidate> {
            rendered_size = layout.font_size;
            Ok(RenderedTextCandidate {
                image: RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255])),
                transform: Transform {
                    x: 10.0,
                    y: 10.0,
                    width: 1.0,
                    height: 1.0,
                    rotation_deg: 0.0,
                },
            })
        };
        fit_rendered_with_mask_collision(
            &builder,
            "short",
            layout_box,
            None,
            12.0,
            18.0,
            false,
            &mask,
            1,
            &mut render_candidate,
        )?;
        assert!(rendered_size <= 18.0);
        Ok(())
    }

    #[test]
    fn typography_planner_no_fit_matches_existing_readability_floor() -> Result<()> {
        let font = any_system_font();
        let builder = TextLayout::new(&font, None);
        let layout_box = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 8.0,
            height: 8.0,
        };
        let baseline = fit_font_size(
            &builder,
            "text that cannot fit",
            layout_box,
            None,
            12.0,
            72.0,
            false,
        )?;
        let planned = fit_font_size(
            &builder,
            "text that cannot fit",
            layout_box,
            None,
            12.0,
            72.0_f32.min(300.0),
            false,
        )?;
        assert_eq!(planned.font_size, baseline.font_size);
        assert_eq!(planned.width, baseline.width);
        assert_eq!(planned.height, baseline.height);
        Ok(())
    }

    #[test]
    fn manual_explicit_font_size_keeps_existing_semantics() -> Result<()> {
        let font = any_system_font();
        let builder = TextLayout::new(&font, None);
        let layout = fit_font_size(
            &builder,
            "manual",
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            Some(42.0),
            12.0,
            18.0,
            false,
        )?;
        assert_eq!(layout.font_size, 42.0);
        Ok(())
    }

    #[test]
    fn han_only_auto_size_uses_source_minus_five_for_other_targets() {
        let mut input = block(0.0, 0.0, 100.0, 50.0, "translated");
        input.font_prediction = Some(FontPrediction {
            font_size_px: 60.0,
            ..Default::default()
        });
        let (explicit, cap) =
            font_size_constraints(&input, seed_layout_box(&input), 12.0, Some(-5.0));
        assert_eq!(explicit, None);
        assert_eq!(cap, 55.0);
    }

    #[test]
    fn han_only_zero_offset_keeps_source_size() {
        let mut input = block(0.0, 0.0, 100.0, 50.0, "translated");
        input.font_prediction = Some(FontPrediction {
            font_size_px: 60.0,
            ..Default::default()
        });
        let (explicit, cap) =
            font_size_constraints(&input, seed_layout_box(&input), 12.0, Some(0.0));
        assert_eq!(explicit, None);
        assert_eq!(cap, 60.0);
    }

    #[test]
    fn han_only_manual_size_overrides_language_offset() {
        let mut input = block(0.0, 0.0, 100.0, 50.0, "translated");
        input.style = Some(TextStyle {
            font_size: Some(72.0),
            ..Default::default()
        });
        input.font_prediction = Some(FontPrediction {
            font_size_px: 60.0,
            ..Default::default()
        });
        let (exact, _) = font_size_constraints(&input, seed_layout_box(&input), 12.0, Some(-5.0));
        assert_eq!(exact, Some(72.0));
    }

    #[test]
    fn han_only_source_size_falls_back_prediction_then_detected_then_box() {
        let mut input = block(0.0, 0.0, 100.0, 50.0, "translated");
        input.detected_font_size_px = Some(44.0);
        input.font_prediction = Some(FontPrediction {
            font_size_px: 60.0,
            ..Default::default()
        });
        assert_eq!(
            font_size_constraints(&input, seed_layout_box(&input), 12.0, Some(-5.0)),
            (None, 55.0)
        );
        input.font_prediction.as_mut().unwrap().font_size_px = f32::NAN;
        assert_eq!(
            font_size_constraints(&input, seed_layout_box(&input), 12.0, Some(-5.0)),
            (None, 39.0)
        );
        input.detected_font_size_px = Some(0.0);
        assert_eq!(
            font_size_constraints(&input, seed_layout_box(&input), 12.0, Some(-5.0)),
            (None, 45.0)
        );
    }

    #[test]
    fn english_detected_size_is_a_thirty_four_pixel_cap_not_an_explicit_size() {
        let mut input = block(0.0, 0.0, 208.0, 39.0, "Full-Body Sculpt");
        input.detected_font_size_px = Some(39.0);
        input.font_prediction = Some(FontPrediction {
            font_size_px: 54.222416,
            ..Default::default()
        });
        let layout_box = seed_layout_box(&input);
        let (explicit, cap) = font_size_constraints_with_group(
            &input,
            layout_box,
            12.0,
            Some(SourceRelativeFontSizePolicy {
                offset: -5.0,
                prefer_detected: true,
            }),
            None,
        );

        assert_eq!(explicit, None);
        assert_eq!(cap, 34.0);
    }

    #[test]
    fn fractional_automatic_cap_never_rounds_up() -> Result<()> {
        let renderer = Renderer::new()?;
        let block = automatic_test_block(
            &renderer,
            Transform {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 1000.0,
                rotation_deg: 0.0,
            },
            "fit",
            34.75,
            34.75,
            TextDirection::Horizontal,
        );
        let resolved_box = resolve_layout_boxes(std::slice::from_ref(&block), None)[0];
        let prepared = renderer
            .prepare_source_relative_automatic(
                &block,
                resolved_box,
                &PageRenderOptions::default(),
                SourceRelativeFontSizePolicy {
                    offset: 0.0,
                    prefer_detected: true,
                },
                12.0,
            )?
            .expect("automatic block must prepare");

        assert_eq!(prepared.cap, 34.75);
        assert_eq!(prepared.independent_font_size, 34.0);
        Ok(())
    }

    #[test]
    fn legacy_auto_fit_rounds_a_fractional_cap_without_source_relative_policy() -> Result<()> {
        let font = any_system_font();
        let layout = fit_font_size(
            &TextLayout::new(&font, None),
            "fit",
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 1000.0,
            },
            None,
            12.0,
            22.5,
            true,
        )?;

        assert_eq!(layout.font_size, 23.0);
        Ok(())
    }

    #[test]
    fn english_auto_size_prefers_detected_before_prediction_and_subtracts_five() {
        let mut input = block(0.0, 0.0, 100.0, 50.0, "translated");
        input.font_prediction = Some(FontPrediction {
            font_size_px: 66.0,
            ..Default::default()
        });
        input.detected_font_size_px = Some(60.0);
        let node_id = input.node_id;
        let blocks = vec![input];
        let layout_boxes = resolve_layout_boxes(&blocks, None);

        let sizes = grouped_source_relative_font_sizes(
            &blocks,
            &layout_boxes,
            12.0,
            Some(SourceRelativeFontSizePolicy {
                offset: -5.0,
                prefer_detected: true,
            }),
        );

        assert_eq!(sizes.get(&node_id), Some(&55.0));
    }

    #[test]
    fn horizontal_source_group_uses_minimum_candidate_independent_of_input_order() {
        let mut left = block(0.0, 0.0, 100.0, 20.0, "left");
        left.source_transform = Transform {
            x: 0.0,
            y: 856.5,
            width: 100.0,
            height: 40.0,
            rotation_deg: 0.0,
        };
        left.detected_font_size_px = Some(37.437256);
        left.source_direction = Some(TextDirection::Horizontal);

        let mut middle = block(120.0, 200.0, 100.0, 20.0, "middle");
        middle.source_transform = Transform {
            x: 120.0,
            y: 857.5,
            width: 100.0,
            height: 40.0,
            rotation_deg: 0.0,
        };
        middle.detected_font_size_px = Some(76.79797);
        middle.source_direction = Some(TextDirection::Horizontal);

        let mut right = block(240.0, 400.0, 100.0, 20.0, "right");
        right.source_transform = Transform {
            x: 240.0,
            y: 856.75,
            width: 100.0,
            height: 40.0,
            rotation_deg: 0.0,
        };
        right.detected_font_size_px = Some(37.19812);
        right.source_direction = Some(TextDirection::Horizontal);

        let node_ids = [left.node_id, middle.node_id, right.node_id];
        let blocks = vec![middle, right, left];
        let layout_boxes = resolve_layout_boxes(&blocks, None);
        let sizes = grouped_source_relative_font_sizes(
            &blocks,
            &layout_boxes,
            12.0,
            Some(SourceRelativeFontSizePolicy {
                offset: -5.0,
                prefer_detected: true,
            }),
        );

        for node_id in node_ids {
            assert!((sizes[&node_id] - 32.19812).abs() < 0.0001);
        }
    }

    #[test]
    fn source_group_excludes_manual_vertical_and_other_rows() {
        let policy = Some(SourceRelativeFontSizePolicy {
            offset: -5.0,
            prefer_detected: true,
        });
        let mut selected = block(0.0, 0.0, 100.0, 20.0, "selected");
        selected.detected_font_size_px = Some(60.0);
        selected.source_direction = Some(TextDirection::Horizontal);

        let mut manual = block(120.0, 0.0, 100.0, 20.0, "manual");
        manual.detected_font_size_px = Some(10.0);
        manual.style = Some(TextStyle {
            font_size: Some(40.0),
            ..Default::default()
        });

        let mut vertical = block(240.0, 0.0, 100.0, 20.0, "vertical");
        vertical.detected_font_size_px = Some(8.0);
        vertical.source_direction = Some(TextDirection::Vertical);

        let mut other_row = block(360.0, 80.0, 100.0, 20.0, "other");
        other_row.detected_font_size_px = Some(12.0);

        let ids = (
            selected.node_id,
            manual.node_id,
            vertical.node_id,
            other_row.node_id,
        );
        let blocks = vec![selected, manual, vertical, other_row];
        let boxes = resolve_layout_boxes(&blocks, None);
        let sizes = grouped_source_relative_font_sizes(&blocks, &boxes, 12.0, policy);

        assert_eq!(sizes.get(&ids.0), Some(&55.0));
        assert!(!sizes.contains_key(&ids.1));
        assert!(!sizes.contains_key(&ids.2));
        assert_eq!(sizes.get(&ids.3), Some(&7.0));
        assert_eq!(
            font_size_constraints_with_group(&blocks[2], boxes[2].layout_box, 12.0, policy, None,),
            (None, 3.0)
        );
    }

    #[test]
    fn legacy_language_modes_render_identically() -> Result<()> {
        let renderer = Renderer::new()?;
        let auto = block(20.0, 20.0, 180.0, 60.0, "Legacy auto");
        let mut manual = block(240.0, 20.0, 180.0, 60.0, "Legacy manual");
        manual.style = Some(TextStyle {
            font_size: Some(18.0),
            ..Default::default()
        });
        let expected_ids = HashSet::from([auto.node_id, manual.node_id]);
        let cases = [
            ("invalid HanOnly", Some("invalid")),
            ("missing HanOnly", None),
            ("AllText", Some("en-US")),
        ];
        let mut baseline = None;

        for (name, target_language) in cases {
            clear_raster_trace();
            let output = renderer.render_page(
                &DynamicImage::new_rgba8(480, 140),
                None,
                None,
                480,
                140,
                &[auto.clone(), manual.clone()],
                &PageRenderOptions {
                    target_language: target_language.map(str::to_string),
                    source_relative_font_size_policy: None,
                    ..Default::default()
                },
            )?;
            assert_eq!(
                output
                    .blocks
                    .iter()
                    .map(|block| block.node_id)
                    .collect::<HashSet<_>>(),
                expected_ids,
                "{name}: every expected node must render"
            );
            let signature = output
                .blocks
                .iter()
                .map(|block| {
                    (
                        block.node_id,
                        block.sprite.dimensions(),
                        block.sprite.to_rgba8().into_raw(),
                        block.expanded_transform.map(|transform| {
                            (
                                transform.x,
                                transform.y,
                                transform.width,
                                transform.height,
                                transform.rotation_deg,
                            )
                        }),
                        block.rendered_direction,
                    )
                })
                .collect::<Vec<_>>();
            if let Some(baseline) = &baseline {
                assert_eq!(&signature, baseline, "{name}: legacy render drifted");
            } else {
                baseline = Some(signature);
            }

            let trace = raster_trace();
            assert_eq!(
                trace.iter().find(|entry| entry.node_id == manual.node_id),
                Some(&RasterTrace {
                    node_id: manual.node_id,
                    path: RasterPath::Legacy,
                    font_size: 18.0,
                }),
                "{name}: manual size must remain explicit"
            );
            let auto_trace = trace
                .iter()
                .find(|entry| entry.node_id == auto.node_id)
                .expect("automatic legacy node must raster");
            assert_eq!(auto_trace.path, RasterPath::Legacy);
            assert_ne!(auto_trace.font_size, 18.0);
        }
        Ok(())
    }

    #[test]
    fn all_text_nonpositive_manual_size_keeps_legacy_explicit_path_and_layout_clamp() -> Result<()>
    {
        let font = any_system_font();
        for scene_size in [0.0, -5.0] {
            let mut input = block(0.0, 0.0, 100.0, 50.0, "translated");
            input.style = Some(TextStyle {
                font_size: Some(scene_size),
                ..Default::default()
            });
            let (explicit, _) = font_size_constraints(&input, seed_layout_box(&input), 12.0, None);
            assert_eq!(explicit, Some(scene_size));
            let layout = run_layout_at(
                &TextLayout::new(&font, None),
                &input.translation,
                seed_layout_box(&input),
                explicit.unwrap(),
                false,
            )?;
            assert_eq!(layout.font_size, 1.0);
        }
        Ok(())
    }

    #[test]
    fn han_only_invalid_target_nonpositive_manual_size_keeps_legacy_explicit_path() {
        for scene_size in [0.0, -5.0] {
            let mut input = block(0.0, 0.0, 100.0, 50.0, "translated");
            input.style = Some(TextStyle {
                font_size: Some(scene_size),
                ..Default::default()
            });
            assert_eq!(
                font_size_constraints(&input, seed_layout_box(&input), 12.0, None).0,
                Some(scene_size)
            );
        }
    }

    #[test]
    fn han_only_long_spaced_text_stays_one_line_without_hyphen() -> Result<()> {
        let font = any_system_font();
        for mode in [WritingMode::Horizontal, WritingMode::VerticalRl] {
            let layout = run_layout_at(
                &TextLayout::new(&font, None).with_writing_mode(mode),
                "a very long translated product title with spaces",
                LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 20.0,
                    height: 20.0,
                },
                60.0,
                true,
            )?;
            assert_eq!(layout.lines.len(), 1);
            assert!(!has_synthetic_hyphen(&layout));
        }
        Ok(())
    }

    #[test]
    fn all_text_keeps_existing_auto_fit_and_soft_wrap() -> Result<()> {
        let font = any_system_font();
        let input = block(0.0, 0.0, 80.0, 200.0, "antidisestablishmentarianism");
        assert!(
            font_size_constraints(&input, seed_layout_box(&input), 12.0, None)
                .0
                .is_none()
        );
        let layout = run_layout_at(
            &TextLayout::new(&font, None),
            &input.translation,
            seed_layout_box(&input),
            24.0,
            false,
        )?;
        assert!(layout.lines.len() > 1);
        Ok(())
    }

    #[test]
    fn han_only_full_body_sculpt_fits_the_final_source_region() -> Result<()> {
        let renderer = Renderer::new()?;
        let source = Transform {
            x: 100.0,
            y: 200.0,
            width: 208.0,
            height: 39.0,
            rotation_deg: 0.0,
        };
        let block = automatic_test_block(
            &renderer,
            source,
            "Full-Body Sculpt",
            39.0,
            54.222416,
            TextDirection::Horizontal,
        );
        let font = test_block_font(&renderer, &block);
        let (safe_size, expected) = expected_safe_raster(
            &renderer,
            &font,
            &block,
            WritingMode::Horizontal,
            12.0,
            34.0,
        )?;
        assert!(safe_size < 34.0, "fixture must exercise down-fitting");

        let output =
            render_automatic_test_blocks(&renderer, std::slice::from_ref(&block), 790, 1023, -5.0)?;
        let rendered = output.blocks.first().expect("automatic block must render");
        assert_eq!(rendered.sprite.dimensions(), expected);
        assert!(rendered.sprite.width() <= source.width as u32);
        assert!(rendered.sprite.height() <= source.height as u32);
        Ok(())
    }

    #[test]
    fn han_only_automatic_bold_preserves_complete_effect_ink() -> Result<()> {
        assert_source_relative_effect_surface(TextShaderEffect {
            bold: true,
            italic: false,
        })
    }

    #[test]
    fn han_only_automatic_italic_preserves_complete_effect_ink() -> Result<()> {
        assert_source_relative_effect_surface(TextShaderEffect {
            bold: false,
            italic: true,
        })
    }

    #[test]
    fn han_only_automatic_bold_italic_preserves_complete_effect_ink() -> Result<()> {
        assert_source_relative_effect_surface(TextShaderEffect {
            bold: true,
            italic: true,
        })
    }

    #[test]
    fn han_only_automatic_effect_surface_matrix_preserves_complete_ink() -> Result<()> {
        let cases = [
            (
                "stroke bold",
                TextShaderEffect {
                    bold: true,
                    italic: false,
                },
                Some(3.0),
                TextDirection::Horizontal,
                "Effect",
                42.0,
            ),
            (
                "stroke italic",
                TextShaderEffect {
                    bold: false,
                    italic: true,
                },
                Some(3.0),
                TextDirection::Horizontal,
                "Effect",
                42.0,
            ),
            (
                "vertical italic",
                TextShaderEffect {
                    bold: false,
                    italic: true,
                },
                None,
                TextDirection::Vertical,
                "AB",
                42.0,
            ),
            (
                "multiline bold",
                TextShaderEffect {
                    bold: true,
                    italic: false,
                },
                None,
                TextDirection::Horizontal,
                "Line\nTwo",
                42.0,
            ),
            (
                "single glyph minimum slant",
                TextShaderEffect {
                    bold: false,
                    italic: true,
                },
                None,
                TextDirection::Horizontal,
                "I",
                12.0,
            ),
        ];

        for (name, effect, stroke_width, direction, text, font_size) in cases {
            assert_source_relative_effect_surface_case(
                name,
                effect,
                stroke_width,
                direction,
                text,
                font_size,
            )
            .with_context(|| name.to_string())?;
        }
        Ok(())
    }

    #[test]
    fn source_relative_preflight_does_not_raster() -> Result<()> {
        let renderer = Renderer::new()?;
        let block = automatic_test_block(
            &renderer,
            Transform {
                x: 0.0,
                y: 0.0,
                width: 208.0,
                height: 39.0,
                rotation_deg: 0.0,
            },
            "Full-Body Sculpt",
            39.0,
            54.222416,
            TextDirection::Horizontal,
        );
        let resolved_box = resolve_layout_boxes(std::slice::from_ref(&block), None)[0];
        clear_raster_trace();
        renderer
            .prepare_source_relative_automatic(
                &block,
                resolved_box,
                &PageRenderOptions {
                    target_language: Some("en".into()),
                    ..Default::default()
                },
                SourceRelativeFontSizePolicy {
                    offset: -5.0,
                    prefer_detected: true,
                },
                min_font_size_for_image(790, 1023),
            )?
            .expect("automatic block must prepare");

        assert!(raster_trace().is_empty());
        Ok(())
    }

    #[test]
    fn source_relative_full_render_rasters_each_successful_node_once() -> Result<()> {
        let renderer = Renderer::new()?;
        let first = automatic_test_block(
            &renderer,
            Transform {
                x: 20.0,
                y: 20.0,
                width: 208.0,
                height: 39.0,
                rotation_deg: 0.0,
            },
            "Full-Body Sculpt",
            39.0,
            54.222416,
            TextDirection::Horizontal,
        );
        let second = automatic_test_block(
            &renderer,
            Transform {
                x: 260.0,
                y: 20.0,
                width: 180.0,
                height: 39.0,
                rotation_deg: 0.0,
            },
            "Peach Hip",
            39.0,
            48.0,
            TextDirection::Horizontal,
        );

        clear_raster_trace();
        let one =
            render_automatic_test_blocks(&renderer, std::slice::from_ref(&first), 790, 1023, -5.0)?;
        assert_eq!(
            one.blocks
                .iter()
                .map(|block| block.node_id)
                .collect::<Vec<_>>(),
            vec![first.node_id]
        );
        let one_trace = raster_trace();
        assert_eq!(one_trace.len(), 1);
        assert_eq!(one_trace[0].node_id, first.node_id);
        assert_eq!(one_trace[0].path, RasterPath::SourceRelative);

        clear_raster_trace();
        let many = render_automatic_test_blocks(
            &renderer,
            &[first.clone(), second.clone()],
            790,
            1023,
            -5.0,
        )?;
        assert_eq!(
            many.blocks
                .iter()
                .map(|block| block.node_id)
                .collect::<HashSet<_>>(),
            HashSet::from([first.node_id, second.node_id])
        );
        let trace = raster_trace();
        for node_id in [first.node_id, second.node_id] {
            assert_eq!(
                trace
                    .iter()
                    .filter(|entry| {
                        entry.node_id == node_id && entry.path == RasterPath::SourceRelative
                    })
                    .count(),
                1,
                "node {node_id} must raster exactly once"
            );
        }
        assert_eq!(trace.len(), 2);
        Ok(())
    }

    #[test]
    fn failed_real_renders_are_not_recorded_as_successes() -> Result<()> {
        let renderer = Renderer::new()?;
        let font = any_system_font();
        let shaped = TextLayout::new(&font, None)
            .with_font_size(24.0)
            .run("Real glyphs")?;
        assert!(shaped.width > 0.0);
        assert!(shaped.height > 0.0);

        let mut invalid = shaped.clone();
        invalid.width = 0.0;
        assert!(invalid.height > 0.0);
        assert!(!invalid.lines.is_empty());
        assert!(invalid.lines.iter().any(|line| !line.glyphs.is_empty()));

        let cases = [
            (NodeId::new(), RasterPath::SourceRelative),
            (NodeId::new(), RasterPath::Legacy),
        ];
        clear_raster_trace();
        for (node_id, path) in cases {
            let error = renderer
                .render_layout(
                    &invalid,
                    WritingMode::Horizontal,
                    &RenderOptions {
                        font_size: invalid.font_size,
                        effect: RendererEffect::default(),
                        stroke: None,
                        padding: 0.0,
                        ..Default::default()
                    },
                    node_id,
                    path,
                )
                .expect_err("zero-width layout must reach the real renderer failure");
            let message = format!("{error:#}");
            let height = message
                .strip_prefix("invalid surface size 0x")
                .and_then(|height| height.parse::<u32>().ok())
                .expect("real renderer error must contain the exact zero-width prefix");
            assert!(height > 0);
        }

        let trace = raster_trace();
        for (node_id, path) in cases {
            assert_eq!(
                trace
                    .iter()
                    .filter(|entry| entry.node_id == node_id && entry.path == path)
                    .count(),
                0,
                "failed {path:?} raster must not be recorded as successful"
            );
        }
        Ok(())
    }

    #[test]
    fn render_page_keeps_manual_explicit_and_out_of_auto_group() -> Result<()> {
        let renderer = Renderer::new()?;
        let row = Transform {
            x: 20.0,
            y: 40.0,
            width: 180.0,
            height: 60.0,
            rotation_deg: 0.0,
        };
        let left = automatic_test_block(
            &renderer,
            row,
            "Slim",
            50.0,
            50.0,
            TextDirection::Horizontal,
        );
        let right = automatic_test_block(
            &renderer,
            Transform { x: 240.0, ..row },
            "Full-Body Sculpt Collection",
            60.0,
            60.0,
            TextDirection::Horizontal,
        );
        let mut manual = automatic_test_block(
            &renderer,
            Transform {
                x: 460.0,
                width: 80.0,
                ..row
            },
            "M",
            8.0,
            8.0,
            TextDirection::Horizontal,
        );
        manual.style.as_mut().expect("manual style").font_size = Some(8.0);
        let mut vertical = automatic_test_block(
            &renderer,
            Transform {
                x: 600.0,
                y: 30.0,
                width: 70.0,
                height: 240.0,
                rotation_deg: 0.0,
            },
            "縦書き",
            52.0,
            52.0,
            TextDirection::Vertical,
        );
        vertical.source_transform = Transform {
            x: 600.0,
            y: 40.0,
            width: 70.0,
            height: 60.0,
            rotation_deg: 0.0,
        };
        let other_row = automatic_test_block(
            &renderer,
            Transform {
                x: 20.0,
                y: 330.0,
                width: 220.0,
                height: 70.0,
                rotation_deg: 0.0,
            },
            "Other",
            42.0,
            42.0,
            TextDirection::Horizontal,
        );
        let automatic_ids = [
            left.node_id,
            right.node_id,
            vertical.node_id,
            other_row.node_id,
        ];

        clear_raster_trace();
        let with_manual = render_automatic_test_blocks(
            &renderer,
            &[
                left.clone(),
                right.clone(),
                manual.clone(),
                vertical.clone(),
                other_row.clone(),
            ],
            900,
            500,
            -5.0,
        )?;
        assert_eq!(
            with_manual
                .blocks
                .iter()
                .map(|block| block.node_id)
                .collect::<HashSet<_>>(),
            HashSet::from([
                left.node_id,
                right.node_id,
                manual.node_id,
                vertical.node_id,
                other_row.node_id,
            ])
        );
        let with_manual_trace = raster_trace();
        assert_eq!(
            with_manual_trace
                .iter()
                .find(|entry| entry.node_id == manual.node_id),
            Some(&RasterTrace {
                node_id: manual.node_id,
                path: RasterPath::Legacy,
                font_size: 8.0,
            })
        );
        let with_manual_auto = automatic_ids
            .into_iter()
            .map(|node_id| {
                let entry = with_manual_trace
                    .iter()
                    .find(|entry| {
                        entry.node_id == node_id && entry.path == RasterPath::SourceRelative
                    })
                    .expect("every automatic node must raster");
                (node_id, entry.font_size)
            })
            .collect::<HashMap<_, _>>();

        clear_raster_trace();
        let without_manual = render_automatic_test_blocks(
            &renderer,
            &[
                left.clone(),
                right.clone(),
                vertical.clone(),
                other_row.clone(),
            ],
            900,
            500,
            -5.0,
        )?;
        assert_eq!(
            without_manual
                .blocks
                .iter()
                .map(|block| block.node_id)
                .collect::<HashSet<_>>(),
            HashSet::from(automatic_ids)
        );
        let without_manual_trace = raster_trace();
        let without_manual_auto = automatic_ids
            .into_iter()
            .map(|node_id| {
                let entry = without_manual_trace
                    .iter()
                    .find(|entry| {
                        entry.node_id == node_id && entry.path == RasterPath::SourceRelative
                    })
                    .expect("every automatic node must raster");
                (node_id, entry.font_size)
            })
            .collect::<HashMap<_, _>>();

        assert_eq!(with_manual_auto, without_manual_auto);
        assert_eq!(
            with_manual_auto[&left.node_id],
            with_manual_auto[&right.node_id]
        );
        assert_ne!(
            with_manual_auto[&vertical.node_id],
            with_manual_auto[&left.node_id]
        );
        assert_ne!(
            with_manual_auto[&other_row.node_id],
            with_manual_auto[&left.node_id]
        );
        Ok(())
    }

    #[test]
    fn han_only_longer_translations_choose_nonincreasing_safe_sizes() -> Result<()> {
        let renderer = Renderer::new()?;
        let source = Transform {
            x: 0.0,
            y: 0.0,
            width: 208.0,
            height: 39.0,
            rotation_deg: 0.0,
        };
        let texts = ["Sculpt", "Body Sculpt", "Full-Body Sculpt Collection"];
        let mut selected = Vec::new();
        for text in texts {
            let block = automatic_test_block(
                &renderer,
                source,
                text,
                39.0,
                54.222416,
                TextDirection::Horizontal,
            );
            let font = test_block_font(&renderer, &block);
            let (safe_size, expected) = expected_safe_raster(
                &renderer,
                &font,
                &block,
                WritingMode::Horizontal,
                12.0,
                34.0,
            )?;
            let output = render_automatic_test_blocks(
                &renderer,
                std::slice::from_ref(&block),
                790,
                1023,
                -5.0,
            )?;
            assert_eq!(
                output.blocks[0].sprite.dimensions(),
                expected,
                "text={text}"
            );
            selected.push(safe_size);
        }
        assert!(selected.windows(2).all(|pair| pair[0] >= pair[1]));
        assert!(selected[1] > selected[2]);
        Ok(())
    }

    #[test]
    fn han_only_equivalent_two_x_geometry_scales_without_a_sample_size_constant() -> Result<()> {
        let renderer = Renderer::new()?;
        let one_x = Transform {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 40.0,
            rotation_deg: 0.0,
        };
        let two_x = Transform {
            width: 320.0,
            height: 80.0,
            ..one_x
        };
        let one = automatic_test_block(
            &renderer,
            one_x,
            "Body Sculpt",
            32.0,
            32.0,
            TextDirection::Horizontal,
        );
        let two = automatic_test_block(
            &renderer,
            two_x,
            "Body Sculpt",
            64.0,
            64.0,
            TextDirection::Horizontal,
        );
        let one_output = render_automatic_test_blocks(&renderer, &[one], 400, 200, 0.0)?;
        let two_output = render_automatic_test_blocks(&renderer, &[two], 800, 400, 0.0)?;
        let (one_w, one_h) = one_output.blocks[0].sprite.dimensions();
        let (two_w, two_h) = two_output.blocks[0].sprite.dimensions();
        let width_ratio = two_w as f32 / one_w as f32;
        let height_ratio = two_h as f32 / one_h as f32;
        assert!((1.7..=2.2).contains(&width_ratio), "ratio={width_ratio}");
        assert!((1.7..=2.2).contains(&height_ratio), "ratio={height_ratio}");
        Ok(())
    }

    #[test]
    fn han_only_vertical_fit_uses_physical_source_axes() -> Result<()> {
        let renderer = Renderer::new()?;
        let source = Transform {
            x: 0.0,
            y: 0.0,
            width: 39.0,
            height: 208.0,
            rotation_deg: 0.0,
        };
        let block = automatic_test_block(
            &renderer,
            source,
            "縦書き文字",
            39.0,
            39.0,
            TextDirection::Vertical,
        );
        let font = test_block_font(&renderer, &block);
        let (_, expected) = expected_safe_raster(
            &renderer,
            &font,
            &block,
            WritingMode::VerticalRl,
            12.0,
            39.0,
        )?;
        let output =
            render_automatic_test_blocks(&renderer, std::slice::from_ref(&block), 200, 400, 0.0)?;
        assert_eq!(output.blocks[0].sprite.dimensions(), expected);
        assert!(output.blocks[0].sprite.width() <= source.width as u32);
        assert!(output.blocks[0].sprite.height() <= source.height as u32);
        Ok(())
    }

    #[test]
    fn horizontal_group_uses_minimum_independent_safe_size_and_is_order_invariant() -> Result<()> {
        let renderer = Renderer::new()?;
        let source = Transform {
            x: 0.0,
            y: 20.0,
            width: 208.0,
            height: 39.0,
            rotation_deg: 0.0,
        };
        let left = automatic_test_block(
            &renderer,
            source,
            "Slim",
            39.0,
            39.0,
            TextDirection::Horizontal,
        );
        let right = automatic_test_block(
            &renderer,
            Transform { x: 220.0, ..source },
            "Full-Body Sculpt",
            60.0,
            60.0,
            TextDirection::Horizontal,
        );
        let left_font = test_block_font(&renderer, &left);
        let right_font = test_block_font(&renderer, &right);
        let (left_safe, _) = expected_safe_raster(
            &renderer,
            &left_font,
            &left,
            WritingMode::Horizontal,
            12.0,
            34.0,
        )?;
        let (right_safe, _) = expected_safe_raster(
            &renderer,
            &right_font,
            &right,
            WritingMode::Horizontal,
            12.0,
            55.0,
        )?;
        let group_size = left_safe.min(right_safe);
        let expected = HashMap::from([
            (
                left.node_id,
                predicted_raster_at_size(
                    &renderer,
                    &left_font,
                    &left,
                    WritingMode::Horizontal,
                    group_size,
                )?,
            ),
            (
                right.node_id,
                predicted_raster_at_size(
                    &renderer,
                    &right_font,
                    &right,
                    WritingMode::Horizontal,
                    group_size,
                )?,
            ),
        ]);

        let first = render_automatic_test_blocks(
            &renderer,
            &[left.clone(), right.clone()],
            500,
            120,
            -5.0,
        )?;
        let second = render_automatic_test_blocks(&renderer, &[right, left], 500, 120, -5.0)?;
        for output in [first, second] {
            let actual = output
                .blocks
                .into_iter()
                .map(|block| (block.node_id, block.sprite.dimensions()))
                .collect::<HashMap<_, _>>();
            assert_eq!(actual, expected);
        }
        Ok(())
    }

    #[test]
    fn han_only_cap_below_readability_floor_returns_no_fit() -> Result<()> {
        let renderer = Renderer::new()?;
        let block = automatic_test_block(
            &renderer,
            Transform {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
                rotation_deg: 0.0,
            },
            "text",
            5.0,
            5.0,
            TextDirection::Horizontal,
        );
        let error = render_automatic_test_blocks(&renderer, &[block], 100, 100, -5.0)
            .err()
            .expect("cap below the readability floor must be a direct no-fit");
        assert!(error.to_string().contains("fit"));
        Ok(())
    }

    #[test]
    fn default_stroke_color_uses_black_for_light_text() {
        let stroke = resolve_stroke_style(None, None, None, 16.0, [255, 255, 255, 255])
            .expect("default stroke should be present");
        assert_eq!(stroke.color, [0, 0, 0, 255]);
        assert_eq!(stroke.width_px, 1.6);
    }

    #[test]
    fn predicted_stroke_width_keeps_auto_black_or_white_color() {
        let prediction = FontPrediction {
            stroke_color: [12, 34, 56],
            stroke_width_px: 3.0,
            ..Default::default()
        };
        let stroke =
            resolve_stroke_style(Some(&prediction), None, None, 18.0, [255, 255, 255, 255])
                .expect("predicted stroke should be present");
        assert_eq!(stroke.color, [0, 0, 0, 255]);
        assert_eq!(stroke.width_px, 3.0);
    }

    #[test]
    fn predicted_zero_stroke_remains_unstroked() {
        let prediction = FontPrediction {
            stroke_width_px: 0.0,
            ..Default::default()
        };

        assert!(
            resolve_stroke_style(Some(&prediction), None, None, 18.0, [0, 0, 0, 255]).is_none()
        );
    }

    #[test]
    fn render_page_keeps_horizontal_stroke_inside_sprite() -> Result<()> {
        let sprite = render_stroked_test_block(TextDirection::Horizontal)?;

        assert_transparent_sprite_border(&sprite);
        Ok(())
    }

    #[test]
    fn render_page_keeps_vertical_stroke_inside_sprite() -> Result<()> {
        let sprite = render_stroked_test_block(TextDirection::Vertical)?;

        assert_transparent_sprite_border(&sprite);
        Ok(())
    }

    #[test]
    fn render_page_padding_fits_non_bubble_sprite_inside_layout_box() -> Result<()> {
        let renderer = Renderer::new()?;
        let layout_box = Transform {
            x: 40.0,
            y: 50.0,
            width: 180.0,
            height: 90.0,
            rotation_deg: 0.0,
        };
        let block = stroked_renderer_block(
            &renderer,
            TextDirection::Horizontal,
            layout_box,
            "A longer translated product title",
            None,
            4.0,
        )?;

        let rendered = render_single_test_block(&renderer, block, None)?;
        let transform = rendered
            .expanded_transform
            .expect("rendered sprite should have a transform");

        assert_transparent_sprite_border(&rendered.sprite.to_rgba8());
        assert!(transform.x >= layout_box.x - FIT_EPSILON);
        assert!(transform.y >= layout_box.y - FIT_EPSILON);
        assert!(transform.x + transform.width <= layout_box.x + layout_box.width + FIT_EPSILON);
        assert!(transform.y + transform.height <= layout_box.y + layout_box.height + FIT_EPSILON);
        Ok(())
    }

    #[test]
    fn render_page_padding_fits_bubble_sprite_inside_mask() -> Result<()> {
        let renderer = Renderer::new()?;
        let mut bubble_mask = GrayImage::from_pixel(320, 320, Luma([0u8]));
        paint_rect(&mut bubble_mask, 20, 20, 300, 300, 1);
        let block = stroked_renderer_block(
            &renderer,
            TextDirection::Horizontal,
            Transform {
                x: 100.0,
                y: 100.0,
                width: 80.0,
                height: 50.0,
                rotation_deg: 0.0,
            },
            "Bubble translated product title",
            None,
            4.0,
        )?;

        let rendered = render_single_test_block(&renderer, block, Some(&bubble_mask))?;
        let transform = rendered
            .expanded_transform
            .expect("rendered sprite should have a transform");
        let sprite = rendered.sprite.to_rgba8();

        assert_transparent_sprite_border(&sprite);
        assert!(!sprite_collides_with_bubble_mask(
            &sprite,
            &transform,
            &bubble_mask,
            1,
        ));
        Ok(())
    }

    #[test]
    fn render_page_padding_keeps_sprite_centered() -> Result<()> {
        let renderer = Renderer::new()?;
        let layout_box = Transform {
            x: 40.0,
            y: 50.0,
            width: 180.0,
            height: 100.0,
            rotation_deg: 0.0,
        };
        let block = stroked_renderer_block(
            &renderer,
            TextDirection::Horizontal,
            layout_box,
            "Centered",
            Some(36.0),
            4.0,
        )?;

        let rendered = render_single_test_block(&renderer, block, None)?;
        let transform = rendered
            .expanded_transform
            .expect("rendered sprite should have a transform");

        assert!(
            (transform.x + transform.width * 0.5 - (layout_box.x + layout_box.width * 0.5)).abs()
                <= 0.5
        );
        assert!(
            (transform.y + transform.height * 0.5 - (layout_box.y + layout_box.height * 0.5)).abs()
                <= 0.5
        );
        Ok(())
    }

    #[test]
    fn tiny_layout_box_with_stroke_does_not_panic() -> Result<()> {
        let renderer = Renderer::new()?;
        let block = stroked_renderer_block(
            &renderer,
            TextDirection::Horizontal,
            Transform {
                x: 10.0,
                y: 10.0,
                width: 4.0,
                height: 4.0,
                rotation_deg: 0.0,
            },
            "T",
            None,
            8.0,
        )?;

        let rendered = render_single_test_block(&renderer, block, None)?;

        assert!(rendered.sprite.width() > 0);
        assert!(rendered.sprite.height() > 0);
        Ok(())
    }

    #[test]
    fn stroke_padding_ignores_none_zero_transparent_and_non_finite() {
        assert_eq!(stroke_padding(None), 0.0);
        assert_eq!(
            stroke_padding(Some(RenderStrokeOptions {
                color: [0, 0, 0, 255],
                width_px: 0.0,
            })),
            0.0
        );
        assert_eq!(
            stroke_padding(Some(RenderStrokeOptions {
                color: [0, 0, 0, 0],
                width_px: 4.0,
            })),
            0.0
        );
        assert_eq!(
            stroke_padding(Some(RenderStrokeOptions {
                color: [0, 0, 0, 255],
                width_px: f32::NAN,
            })),
            0.0
        );
        assert_eq!(
            stroke_padding(Some(RenderStrokeOptions {
                color: [0, 0, 0, 255],
                width_px: 4.25,
            })),
            8.0
        );
    }

    #[test]
    fn explicit_block_stroke_color_is_preserved_even_if_it_matches_text() {
        let stroke = resolve_stroke_style(
            None,
            Some(&TextStrokeStyle {
                enabled: true,
                color: [255, 255, 255, 255],
                width_px: Some(2.0),
            }),
            None,
            18.0,
            [255, 255, 255, 255],
        )
        .expect("explicit stroke should be present");
        assert_eq!(stroke.color, [255, 255, 255, 255]);
        assert_eq!(stroke.width_px, 2.0);
    }

    #[test]
    fn predicted_text_color_wins_without_explicit_style() {
        let derived = TextStyle {
            font_families: Vec::new(),
            font_size: None,
            color: [0, 0, 0, 255],
            effect: None,
            stroke: None,
            text_align: None,
        };
        let prediction = FontPrediction {
            text_color: [12, 34, 56],
            ..Default::default()
        };
        assert_eq!(
            resolve_text_color(None, &derived, Some(&prediction)),
            [12, 34, 56, 255]
        );
    }

    #[test]
    fn explicit_text_color_wins_over_prediction() {
        let explicit = TextStyle {
            font_families: Vec::new(),
            font_size: None,
            color: [200, 100, 50, 255],
            effect: None,
            stroke: None,
            text_align: None,
        };
        let prediction = FontPrediction {
            text_color: [12, 34, 56],
            ..Default::default()
        };
        assert_eq!(
            resolve_text_color(Some(&explicit), &explicit, Some(&prediction)),
            [200, 100, 50, 255]
        );
    }

    #[test]
    #[ignore = "hanonly-pre-greenc-red"]
    fn hanonly_pre_greenc_red_t3_source_color_contract() -> Result<()> {
        let _diagnostic_lock = crate::pipeline::lock_diagnostic_capture_test();
        let renderer = Renderer::new()?;
        const SOURCE_RGBA: [u8; 4] = [12, 34, 56, 255];
        const STALE_RGBA: [u8; 4] = [1, 2, 3, 255];
        let options = PageRenderOptions {
            source_relative_font_size_policy: Some(SourceRelativeFontSizePolicy {
                offset: 0.0,
                prefer_detected: true,
            }),
            ..Default::default()
        };

        let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(280, 120, Rgba(SOURCE_RGBA)));
        let mut observations = Vec::new();
        for (name, predicted) in [
            ("absent", None),
            ("present", Some([240, 241, 242])),
            ("delayed", Some([90, 91, 92])),
            ("contradictory", Some([255, 0, 255])),
        ] {
            let mut block = automatic_test_block(
                &renderer,
                Transform {
                    x: 20.0,
                    y: 20.0,
                    width: 220.0,
                    height: 80.0,
                    rotation_deg: 0.0,
                },
                "Source color",
                30.0,
                28.0,
                TextDirection::Horizontal,
            );
            block.typography_plan_verified = true;
            let style = block.style.as_mut().context("test style")?;
            style.color = STALE_RGBA;
            style.stroke = None;
            match predicted {
                Some(color) => {
                    block
                        .font_prediction
                        .as_mut()
                        .context("prediction")?
                        .text_color = color
                }
                None => block.font_prediction = None,
            }
            let capture = start_renderer_diagnostic_capture();
            let output = renderer.render_page(&source, None, None, 280, 120, &[block], &options)?;
            let events = capture.take();
            let [event] = events.as_slice() else {
                panic!("{name}: expected one diagnostic");
            };
            assert_diagnostic_matches_sprite(event, &output.blocks[0].sprite);
            observations.push((name, event.clone(), output.blocks[0].sprite.to_rgba8()));
        }
        let digest = observations[0].1.sprite_rgba_blake3.clone();
        for (name, event, sprite) in observations {
            assert_eq!(
                event.fill_outcome,
                RendererFieldOutcome::SourceColorContract,
                "{name}"
            );
            assert_eq!(event.resolved_fill_rgba, SOURCE_RGBA, "{name}");
            assert_eq!(event.final_font_size_px, 30, "{name}");
            assert_eq!(event.sprite_rgba_blake3, digest, "{name}");
            assert!(
                sprite.pixels().any(|pixel| pixel.0 == SOURCE_RGBA),
                "{name}"
            );
            assert!(
                sprite
                    .pixels()
                    .all(|pixel| pixel.0[3] == 0 || pixel.0 != STALE_RGBA),
                "{name}"
            );
            assert_eq!(event.builder_publication_count, 1, "{name}");
            assert_eq!(event.builder_raster_count, 1, "{name}");
            assert_eq!(event.renderer_rebuild_count, 0, "{name}");
        }
        Ok(())
    }

    #[test]
    fn mask_collision_fit_renders_min_size_when_no_safe_size_exists() -> Result<()> {
        let font = any_system_font();
        let layout_builder = TextLayout::new(&font, None);
        let layout_box = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 24.0,
            height: 12.0,
        };
        let mask = GrayImage::from_pixel(64, 64, Luma([0u8]));
        let mut rendered_sizes = Vec::new();
        let mut render_candidate = |layout: &LayoutRun<'_>| -> Result<RenderedTextCandidate> {
            rendered_sizes.push(layout.font_size);
            let width = layout.width.ceil().max(1.0) as u32;
            let height = layout.height.ceil().max(1.0) as u32;
            Ok(RenderedTextCandidate {
                image: RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255])),
                transform: Transform {
                    x: 0.0,
                    y: 0.0,
                    width: width as f32,
                    height: height as f32,
                    rotation_deg: 0.0,
                },
            })
        };

        let candidate = fit_rendered_with_mask_collision(
            &layout_builder,
            "overflowing text",
            layout_box,
            None,
            12.0,
            18.0,
            false,
            &mask,
            1,
            &mut render_candidate,
        )?;

        assert_eq!(rendered_sizes.last().copied(), Some(12.0));
        assert!(candidate.image.width() >= 1);
        assert!(candidate.image.height() >= 1);
        Ok(())
    }

    #[test]
    fn shared_bubble_keeps_seed_boxes_to_avoid_overlap() {
        let mut mask = GrayImage::from_pixel(200, 200, Luma([0u8]));
        paint_rect(&mut mask, 10, 10, 190, 190, 1);
        let index = BubbleIndex::new(mask);
        let blocks = vec![
            block(30.0, 30.0, 40.0, 80.0, "hello"),
            block(120.0, 30.0, 40.0, 80.0, "world"),
        ];

        let forward = resolve_layout_boxes(&blocks, Some(&index));

        for (i, block) in blocks.iter().enumerate() {
            let seed = seed_layout_box(block);
            let resolved = forward[i].layout_box;
            assert!(
                resolved.x <= seed.x
                    && resolved.y <= seed.y
                    && resolved.x + resolved.width >= seed.x + seed.width
                    && resolved.y + resolved.height >= seed.y + seed.height,
                "every resolved box must contain its seed anchor"
            );
        }
        assert!(
            forward[0].layout_box.x + forward[0].layout_box.width <= forward[1].layout_box.x
                || forward[1].layout_box.x + forward[1].layout_box.width <= forward[0].layout_box.x,
            "shared-bubble owners must receive non-overlapping resolved boxes"
        );

        let mut reversed = blocks.clone();
        reversed.reverse();
        let reverse = resolve_layout_boxes(&reversed, Some(&index));
        let forward_map: HashMap<NodeId, LayoutBox> = blocks
            .iter()
            .zip(&forward)
            .map(|(b, r)| (b.node_id, r.layout_box))
            .collect();
        for (block, rev) in reversed.iter().zip(&reverse) {
            assert_eq!(
                forward_map[&block.node_id], rev.layout_box,
                "resolver must be order-independent: same owner, same resolved box"
            );
        }

        assert_eq!(forward[0].bubble_id, Some(1));
        assert_eq!(forward[1].bubble_id, Some(1));
        assert_eq!(
            forward[0].diagnostic_branch,
            RendererLayoutBoxBranch::SharedBubble
        );
        assert_eq!(
            forward[1].diagnostic_branch,
            RendererLayoutBoxBranch::SharedBubble
        );
    }

    #[test]
    fn hanonly_pre_b1_red_t2_dynamic_layout_contract() -> Result<()> {
        use std::io::Cursor;

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        struct Rect(u32, u32, u32, u32);

        impl Rect {
            fn contains(self, other: Self) -> bool {
                self.0 <= other.0 && self.1 <= other.1 && self.2 >= other.2 && self.3 >= other.3
            }

            fn overlaps(self, other: Self) -> bool {
                self.0 < other.2 && other.0 < self.2 && self.1 < other.3 && other.1 < self.3
            }
        }

        fn actual(layout: LayoutBox) -> Rect {
            Rect(
                layout.x.floor() as u32,
                layout.y.floor() as u32,
                (layout.x + layout.width).ceil() as u32,
                (layout.y + layout.height).ceil() as u32,
            )
        }

        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum SizeBin {
            Below720,
            From720To1439,
            From1440To2159,
            AtLeast2160,
        }

        fn size_bin(short_side: u32) -> SizeBin {
            match short_side {
                ..720 => SizeBin::Below720,
                720..1440 => SizeBin::From720To1439,
                1440..2160 => SizeBin::From1440To2159,
                _ => SizeBin::AtLeast2160,
            }
        }

        fn decoded_source(short_side: u32, marker: u8) -> Result<(DynamicImage, String)> {
            let width = short_side * 4 / 3;
            let source =
                DynamicImage::ImageLuma8(GrayImage::from_pixel(width, short_side, Luma([marker])));
            let mut encoded = Cursor::new(Vec::new());
            source.write_to(&mut encoded, image::ImageFormat::Png)?;
            let decoded = image::load_from_memory(&encoded.into_inner())?;
            let rgba = decoded.to_rgba8();
            let mut identity = blake3::Hasher::new();
            identity.update(&rgba.width().to_le_bytes());
            identity.update(&rgba.height().to_le_bytes());
            identity.update(rgba.as_raw());
            Ok((decoded, identity.finalize().to_hex().to_string()))
        }

        type ResolvedDimensions = (
            Vec<(NodeId, Rect)>,
            Vec<(NodeId, Rect)>,
            Vec<(NodeId, Rect)>,
        );

        fn resolve_real_dimensions(
            decoded: &DynamicImage,
            node_ids: &[NodeId; 2],
        ) -> ResolvedDimensions {
            let (width, height) = decoded.dimensions();
            let anchors = [
                Rect(width / 8, height / 3, width * 3 / 8, height * 2 / 3),
                Rect(width * 5 / 8, height / 3, width * 7 / 8, height * 2 / 3),
            ];
            let mut blocks = anchors
                .iter()
                .zip(["short", "three times longer"])
                .enumerate()
                .map(|(index, (anchor, text))| {
                    let mut input = block(
                        anchor.0 as f32,
                        anchor.1 as f32,
                        (anchor.2 - anchor.0) as f32,
                        (anchor.3 - anchor.1) as f32,
                        text,
                    );
                    input.node_id = node_ids[index];
                    input.source_direction = Some(TextDirection::Horizontal);
                    input
                })
                .collect::<Vec<_>>();
            let index = BubbleIndex::new(GrayImage::from_pixel(width, height, Luma([1])));
            let resolve = |inputs: &[RenderBlockInput]| {
                inputs
                    .iter()
                    .zip(resolve_layout_boxes(inputs, Some(&index)))
                    .map(|(input, resolved)| (input.node_id, actual(resolved.layout_box)))
                    .collect::<Vec<_>>()
            };
            let forward = resolve(&blocks);
            blocks.reverse();
            let reverse = resolve(&blocks);
            blocks.reverse();
            let repeated = resolve(&blocks);
            (
                anchors
                    .into_iter()
                    .zip(&blocks)
                    .map(|(anchor, input)| (input.node_id, anchor))
                    .collect(),
                forward,
                reverse.into_iter().chain(repeated).collect::<Vec<_>>(),
            )
        }

        let boundary_cases = [
            (719, SizeBin::Below720),
            (720, SizeBin::From720To1439),
            (1439, SizeBin::From720To1439),
            (1440, SizeBin::From1440To2159),
            (2159, SizeBin::From1440To2159),
            (2160, SizeBin::AtLeast2160),
        ];
        let metamorphic_cases = [(0.5, 270), (1.0, 540), (2.0, 1080), (4.0, 2160)];
        let node_ids = [NodeId::new(), NodeId::new()];
        let mut source_hashes = Vec::new();
        let mut observations = Vec::new();

        for (index, (short_side, expected_bin)) in boundary_cases.into_iter().enumerate() {
            let (decoded, source_hash) = decoded_source(short_side, index as u8 + 1)?;
            assert_eq!(decoded.dimensions().1, short_side);
            assert_eq!(size_bin(decoded.dimensions().1), expected_bin);
            source_hashes.push(source_hash);
            observations.push((
                short_side as f32 / 540.0,
                resolve_real_dimensions(&decoded, &node_ids),
            ));
        }
        let boundary_observation_count = observations.len();
        for (index, (scale, short_side)) in metamorphic_cases.into_iter().enumerate() {
            let (decoded, source_hash) = decoded_source(
                short_side,
                index as u8 + boundary_observation_count as u8 + 1,
            )?;
            assert_eq!(decoded.dimensions(), (short_side * 4 / 3, short_side));
            source_hashes.push(source_hash);
            observations.push((scale, resolve_real_dimensions(&decoded, &node_ids)));
        }

        source_hashes.sort_unstable();
        source_hashes.dedup();
        assert!(
            source_hashes.len() >= 4,
            "dynamic bins must come from distinct decoded sources"
        );

        for (_, (anchors, forward, reverse_and_repeat)) in &observations {
            let forward = forward.iter().copied().collect::<HashMap<_, _>>();
            let reverse = reverse_and_repeat[..anchors.len()]
                .iter()
                .copied()
                .collect::<HashMap<_, _>>();
            let repeated = reverse_and_repeat[anchors.len()..]
                .iter()
                .copied()
                .collect::<HashMap<_, _>>();
            assert_eq!(reverse, forward, "resolver must be order-independent");
            assert_eq!(repeated, forward, "resolver must be repeatable");
            let regions = anchors
                .iter()
                .map(|(node_id, anchor)| (*anchor, forward[node_id]))
                .collect::<Vec<_>>();
            for (anchor, region) in &regions {
                assert!(region.contains(*anchor));
                assert_ne!(
                    region, anchor,
                    "shared-bubble owners must receive dynamic space beyond their detector boxes"
                );
            }
            assert!(!regions[0].1.overlaps(regions[1].1));
        }

        let metamorphic = &observations[boundary_observation_count..];
        let reference = &metamorphic[1].1.1;
        for (scale, (_, resolved, _)) in metamorphic {
            for ((reference_id, reference), (node_id, actual)) in reference.iter().zip(resolved) {
                assert_eq!(node_id, reference_id);
                for (actual, reference) in [
                    (actual.0 as f32 / scale, reference.0 as f32),
                    (actual.1 as f32 / scale, reference.1 as f32),
                    (actual.2 as f32 / scale, reference.2 as f32),
                    (actual.3 as f32 / scale, reference.3 as f32),
                ] {
                assert!(
                    (actual - reference).abs() <= 2.0,
                    "resolver geometry must be scale-metamorphic"
                );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn single_block_can_still_expand_into_its_bubble() {
        let mut mask = GrayImage::from_pixel(200, 200, Luma([0u8]));
        paint_rect(&mut mask, 20, 20, 180, 180, 1);
        let index = BubbleIndex::new(mask);
        let blocks = vec![block(70.0, 70.0, 20.0, 30.0, "hello")];

        let layout_boxes = resolve_layout_boxes(&blocks, Some(&index));

        assert!(layout_boxes[0].layout_box.width > blocks[0].transform.width);
        assert!(layout_boxes[0].layout_box.height > blocks[0].transform.height);
        assert_eq!(layout_boxes[0].bubble_id, Some(1));
        assert_eq!(
            layout_boxes[0].diagnostic_branch,
            RendererLayoutBoxBranch::UniqueBubble
        );
    }

    #[test]
    fn locked_block_keeps_manual_layout_box_inside_bubble() {
        let mut mask = GrayImage::from_pixel(200, 200, Luma([0u8]));
        paint_rect(&mut mask, 20, 20, 180, 180, 1);
        let index = BubbleIndex::new(mask);
        let mut locked = block(70.0, 70.0, 20.0, 30.0, "hello");
        locked.lock_layout_box = true;
        let blocks = vec![locked];

        let layout_boxes = resolve_layout_boxes(&blocks, Some(&index));

        assert_eq!(layout_boxes[0].layout_box, seed_layout_box(&blocks[0]));
        assert_eq!(layout_boxes[0].bubble_id, None);
        assert_eq!(
            layout_boxes[0].diagnostic_branch,
            RendererLayoutBoxBranch::LockedSeed
        );

        let unlocked = block(70.0, 70.0, 20.0, 30.0, "hello");
        let seed = resolve_layout_boxes(std::slice::from_ref(&unlocked), None);
        assert_eq!(seed[0].diagnostic_branch, RendererLayoutBoxBranch::Seed);

        let empty = block(70.0, 70.0, 20.0, 30.0, "");
        let empty_seed = resolve_layout_boxes(std::slice::from_ref(&empty), Some(&index));
        assert_eq!(
            empty_seed[0].diagnostic_branch,
            RendererLayoutBoxBranch::Seed
        );
    }

    fn has_synthetic_hyphen(layout: &LayoutRun<'_>) -> bool {
        layout.lines.iter().any(|line| {
            line.glyphs
                .iter()
                .any(|glyph| glyph.cluster as usize == line.range.end)
        })
    }

    #[test]
    fn preserves_explicit_lines_in_horizontal_and_vertical_fit_paths() -> Result<()> {
        let font = any_system_font();
        let cases = [
            (WritingMode::Horizontal, "antidisestablishmentarianism", 1),
            (
                WritingMode::Horizontal,
                "first explicit line\nsecond explicit line",
                2,
            ),
            (WritingMode::VerticalRl, "VERTICALTEXT", 1),
            (WritingMode::VerticalRl, "FIRST\nSECOND", 2),
        ];
        for (mode, text, expected_lines) in cases {
            let builder = TextLayout::new(&font, None).with_writing_mode(mode);
            let layout_box = LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 140.0,
                height: 140.0,
            };
            let layout = fit_font_size(&builder, text, layout_box, None, 6.0, 24.0, true)?;
            assert_eq!(layout.lines.len(), expected_lines, "mode={mode:?}");
            assert!(!has_synthetic_hyphen(&layout), "mode={mode:?}");
        }
        Ok(())
    }

    #[test]
    fn preserves_explicit_lines_in_horizontal_and_vertical_collision_paths() -> Result<()> {
        let font = any_system_font();
        let cases = [
            (WritingMode::Horizontal, "antidisestablishmentarianism", 1),
            (
                WritingMode::Horizontal,
                "first explicit line\nsecond explicit line",
                2,
            ),
            (WritingMode::VerticalRl, "VERTICALTEXT", 1),
            (WritingMode::VerticalRl, "FIRST\nSECOND", 2),
        ];
        for (mode, text, expected_lines) in cases {
            let builder = TextLayout::new(&font, None).with_writing_mode(mode);
            let layout = run_collision_layout_at(
                &builder,
                text,
                LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 140.0,
                    height: 140.0,
                },
                12.0,
                true,
            )?;
            assert_eq!(layout.lines.len(), expected_lines, "mode={mode:?}");
            assert!(!has_synthetic_hyphen(&layout), "mode={mode:?}");
        }
        Ok(())
    }

    #[test]
    fn preserves_explicit_lines_when_bubble_mask_is_locked_to_han_box() -> Result<()> {
        let mut mask = GrayImage::from_pixel(200, 200, Luma([0u8]));
        paint_rect(&mut mask, 20, 20, 180, 180, 1);
        let index = BubbleIndex::new(mask);
        let mut locked = block(70.0, 70.0, 30.0, 40.0, "FIRST\nSECOND");
        locked.lock_layout_box = true;
        locked.preserve_explicit_lines = true;
        let blocks = vec![locked];

        let resolved = resolve_layout_boxes(&blocks, Some(&index));
        let font = any_system_font();
        let layout = fit_font_size(
            &TextLayout::new(&font, None),
            &blocks[0].translation,
            resolved[0].layout_box,
            None,
            6.0,
            18.0,
            blocks[0].preserve_explicit_lines,
        )?;

        assert_eq!(resolved[0].layout_box, seed_layout_box(&blocks[0]));
        assert_eq!(resolved[0].bubble_id, None);
        assert_eq!(layout.lines.len(), 2);
        Ok(())
    }

    #[test]
    fn preserves_explicit_lines_keeps_all_text_soft_wrapping() -> Result<()> {
        let font = any_system_font();
        let builder = TextLayout::new(&font, None).with_writing_mode(WritingMode::Horizontal);
        let layout = run_layout_at(
            &builder,
            "antidisestablishmentarianism",
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 200.0,
            },
            24.0,
            false,
        )?;
        assert!(layout.lines.len() > 1);
        Ok(())
    }

    #[test]
    fn mask_collision_detects_alpha_outside_matched_bubble() {
        let mut mask = GrayImage::from_pixel(10, 10, Luma([0u8]));
        paint_rect(&mut mask, 2, 2, 8, 8, 1);
        let sprite = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 255]));

        let inside = Transform {
            x: 3.0,
            y: 3.0,
            width: 4.0,
            height: 4.0,
            rotation_deg: 0.0,
        };
        assert!(!sprite_collides_with_bubble_mask(
            &sprite, &inside, &mask, 1
        ));

        let outside = Transform {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 4.0,
            rotation_deg: 0.0,
        };
        assert!(sprite_collides_with_bubble_mask(
            &sprite, &outside, &mask, 1
        ));
    }

    #[test]
    fn mask_collision_ignores_transparent_sprite_pixels() {
        let mask = GrayImage::from_pixel(4, 4, Luma([0u8]));
        let sprite = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 0]));
        let transform = Transform {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 4.0,
            rotation_deg: 0.0,
        };

        assert!(!sprite_collides_with_bubble_mask(
            &sprite, &transform, &mask, 1
        ));
    }

    fn automatic_test_block(
        renderer: &Renderer,
        transform: Transform,
        translation: &str,
        detected_font_size_px: f32,
        predicted_font_size_px: f32,
        direction: TextDirection,
    ) -> RenderBlockInput {
        let font = test_font_post_script_name(
            &renderer
                .fontbook
                .lock()
                .expect("failed to lock renderer fontbook")
                .all_families(),
        );
        RenderBlockInput {
            node_id: NodeId::new(),
            source_transform: transform,
            transform,
            translation: translation.to_string(),
            style: Some(TextStyle {
                font_families: vec![font],
                font_size: None,
                color: [255, 255, 255, 255],
                effect: None,
                stroke: Some(TextStrokeStyle {
                    enabled: true,
                    color: [0, 0, 0, 255],
                    width_px: None,
                }),
                text_align: None,
            }),
            font_prediction: Some(FontPrediction {
                font_size_px: predicted_font_size_px,
                ..Default::default()
            }),
            detected_font_size_px: Some(detected_font_size_px),
            source_direction: Some(direction),
            rendered_direction: None,
            lock_layout_box: true,
            preserve_explicit_lines: true,
            typography_plan_verified: false,
        }
    }

    fn test_block_font(renderer: &Renderer, block: &RenderBlockInput) -> Font {
        renderer
            .select_font(block.style.as_ref().expect("test style"))
            .expect("test font must resolve")
    }

    fn render_automatic_test_blocks(
        renderer: &Renderer,
        blocks: &[RenderBlockInput],
        image_width: u32,
        image_height: u32,
        offset: f32,
    ) -> Result<RenderOutput> {
        renderer.render_page(
            &DynamicImage::new_rgba8(image_width, image_height),
            None,
            None,
            image_width,
            image_height,
            blocks,
            &PageRenderOptions {
                source_relative_font_size_policy: Some(SourceRelativeFontSizePolicy {
                    offset,
                    prefer_detected: true,
                }),
                ..Default::default()
            },
        )
    }

    fn assert_source_relative_effect_surface(effect: TextShaderEffect) -> Result<()> {
        assert_source_relative_effect_surface_case(
            "horizontal effect",
            effect,
            None,
            TextDirection::Horizontal,
            "Effect",
            42.0,
        )
    }

    fn assert_source_relative_effect_surface_case(
        name: &str,
        effect: TextShaderEffect,
        stroke_width: Option<f32>,
        direction: TextDirection,
        text: &str,
        font_size: f32,
    ) -> Result<()> {
        const REFERENCE_PADDING: f32 = 16.0;
        const ALPHA_RESAMPLING_TOLERANCE_DIVISOR: u64 = 100_000;

        let renderer = Renderer::new()?;
        let source = match direction {
            TextDirection::Horizontal => Transform {
                x: 0.0,
                y: 0.0,
                width: 240.0,
                height: 96.0,
                rotation_deg: 0.0,
            },
            TextDirection::Vertical => Transform {
                x: 0.0,
                y: 0.0,
                width: 96.0,
                height: 420.0,
                rotation_deg: 0.0,
            },
        };
        let mut block =
            automatic_test_block(&renderer, source, text, font_size, font_size, direction);
        let style = block.style.as_mut().expect("test style");
        style.effect = Some(effect);
        style.stroke = stroke_width.map(|width_px| TextStrokeStyle {
            enabled: true,
            color: [0, 0, 0, 255],
            width_px: Some(width_px),
        });
        let color = style.color;
        block
            .font_prediction
            .as_mut()
            .expect("test prediction")
            .stroke_width_px = 0.0;

        let resolved_box = resolve_layout_boxes(std::slice::from_ref(&block), None)[0];
        let prepared = renderer
            .prepare_source_relative_automatic(
                &block,
                resolved_box,
                &PageRenderOptions::default(),
                SourceRelativeFontSizePolicy {
                    offset: 0.0,
                    prefer_detected: true,
                },
                12.0,
            )?
            .expect("automatic block must prepare");
        let final_size = prepared.independent_font_size;
        let layout_builder = automatic_layout_builder(
            &prepared.font,
            &renderer.symbol_fallbacks,
            prepared.writing_mode,
            prepared.align,
            None,
        );
        let layout = run_layout_at(
            &layout_builder,
            &block.translation,
            prepared.layout_box,
            final_size,
            true,
        )?;
        let resolved_stroke = resolve_stroke_style(
            block.font_prediction.as_ref(),
            block.style.as_ref().and_then(|style| style.stroke.as_ref()),
            None,
            final_size,
            color,
        );
        let predicted = source_relative_raster_dimensions(&layout, resolved_stroke, effect)
            .expect("effect surface dimensions must be valid");
        let reference = renderer.renderer.render(
            &layout,
            prepared.writing_mode,
            &RenderOptions {
                font_size: final_size,
                color,
                effect: shader_core_to_renderer(effect),
                stroke: resolved_stroke,
                padding: REFERENCE_PADDING,
                ..Default::default()
            },
        )?;
        let rendered = renderer.render_prepared_source_relative(
            &block,
            &prepared,
            final_size,
            None,
            None,
            RasterOptions::default(),
        )?;
        assert_eq!(
            rendered.sprite.dimensions(),
            (predicted.0, predicted.1),
            "{name}: preflight and final raster must use the same effect-aware surface dimensions"
        );
        assert!(rendered.sprite.width() <= source.width as u32);
        assert!(rendered.sprite.height() <= source.height as u32);
        let rendered_alpha = alpha_coverage(&rendered.sprite.to_rgba8());
        let reference_alpha = alpha_coverage(&reference);
        // Lanczos downsampling may shift a tiny amount of alpha at glyph edges.
        let resampling_tolerance = reference_alpha / ALPHA_RESAMPLING_TOLERANCE_DIVISOR + 1;
        assert!(
            rendered_alpha + resampling_tolerance >= reference_alpha,
            "{name}: predicted surface clipped effect ink: rendered alpha {rendered_alpha}, reference alpha {reference_alpha}"
        );
        Ok(())
    }

    fn alpha_coverage(image: &RgbaImage) -> u64 {
        image.pixels().map(|pixel| u64::from(pixel.0[3])).sum()
    }

    fn expected_safe_raster(
        renderer: &Renderer,
        font: &Font,
        block: &RenderBlockInput,
        writing_mode: WritingMode,
        min_size: f32,
        cap: f32,
    ) -> Result<(f32, (u32, u32))> {
        let minimum = min_size.ceil() as i32;
        let maximum = cap.floor() as i32;
        anyhow::ensure!(maximum >= minimum, "no fit below readability floor");
        for size in (minimum..=maximum).rev() {
            let dimensions =
                predicted_raster_at_size(renderer, font, block, writing_mode, size as f32)?;
            if dimensions.0 <= block.source_transform.width.floor() as u32
                && dimensions.1 <= block.source_transform.height.floor() as u32
            {
                return Ok((size as f32, dimensions));
            }
        }
        anyhow::bail!("no fit at readability floor")
    }

    fn predicted_raster_at_size(
        renderer: &Renderer,
        font: &Font,
        block: &RenderBlockInput,
        writing_mode: WritingMode,
        size: f32,
    ) -> Result<(u32, u32)> {
        let layout = run_layout_at(
            &TextLayout::new(font, None)
                .with_fallback_fonts(&renderer.symbol_fallbacks)
                .with_writing_mode(writing_mode)
                .with_alignment(RendererTextAlign::Center),
            &block.translation,
            seed_layout_box(block),
            size,
            block.preserve_explicit_lines,
        )?;
        let style = block.style.as_ref().expect("test style");
        let padding = stroke_padding(resolve_stroke_style(
            block.font_prediction.as_ref(),
            style.stroke.as_ref(),
            None,
            size,
            style.color,
        ));
        Ok((
            (layout.width + padding * 2.0).ceil() as u32,
            (layout.height + padding * 2.0).ceil() as u32,
        ))
    }

    fn block(x: f32, y: f32, width: f32, height: f32, translation: &str) -> RenderBlockInput {
        RenderBlockInput {
            node_id: NodeId::new(),
            source_transform: Transform {
                x,
                y,
                width,
                height,
                rotation_deg: 0.0,
            },
            transform: Transform {
                x,
                y,
                width,
                height,
                rotation_deg: 0.0,
            },
            translation: translation.to_string(),
            style: None,
            font_prediction: None,
            detected_font_size_px: None,
            source_direction: None,
            rendered_direction: None,
            lock_layout_box: false,
            preserve_explicit_lines: false,
            typography_plan_verified: false,
        }
    }

    fn paint_rect(img: &mut GrayImage, x0: u32, y0: u32, x1: u32, y1: u32, value: u8) {
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, Luma([value]));
            }
        }
    }

    fn any_system_font() -> Font {
        let mut book = FontBook::new();
        let post_script_name = test_font_post_script_name(&book.all_families());
        book.query(&post_script_name)
            .expect("failed to load system font")
    }

    fn render_stroked_test_block(direction: TextDirection) -> Result<RgbaImage> {
        let renderer = Renderer::new()?;
        let transform = match direction {
            TextDirection::Horizontal => Transform {
                x: 40.0,
                y: 40.0,
                width: 220.0,
                height: 100.0,
                rotation_deg: 0.0,
            },
            TextDirection::Vertical => Transform {
                x: 40.0,
                y: 40.0,
                width: 100.0,
                height: 220.0,
                rotation_deg: 0.0,
            },
        };
        let block =
            stroked_renderer_block(&renderer, direction, transform, "STROKE", Some(48.0), 4.0)?;

        Ok(render_single_test_block(&renderer, block, None)?
            .sprite
            .to_rgba8())
    }

    fn stroked_renderer_block(
        renderer: &Renderer,
        direction: TextDirection,
        transform: Transform,
        translation: &str,
        font_size: Option<f32>,
        stroke_width: f32,
    ) -> Result<RenderBlockInput> {
        let font = test_font_post_script_name(
            &renderer
                .fontbook
                .lock()
                .expect("failed to lock renderer fontbook")
                .all_families(),
        );

        Ok(RenderBlockInput {
            node_id: NodeId::new(),
            source_transform: transform,
            transform,
            translation: translation.to_string(),
            style: Some(TextStyle {
                font_families: vec![font],
                font_size,
                color: [255, 255, 255, 255],
                effect: None,
                stroke: Some(TextStrokeStyle {
                    enabled: true,
                    color: [0, 0, 0, 255],
                    width_px: Some(stroke_width),
                }),
                text_align: None,
            }),
            font_prediction: None,
            detected_font_size_px: None,
            source_direction: Some(direction),
            rendered_direction: None,
            lock_layout_box: false,
            preserve_explicit_lines: false,
            typography_plan_verified: false,
        })
    }

    fn render_single_test_block(
        renderer: &Renderer,
        block: RenderBlockInput,
        bubble_mask: Option<&GrayImage>,
    ) -> Result<RenderedBlock> {
        let bubble_mask = bubble_mask.cloned().map(DynamicImage::ImageLuma8);
        let output = renderer.render_page(
            &DynamicImage::new_rgba8(320, 320),
            None,
            bubble_mask.as_ref(),
            320,
            320,
            &[block],
            &PageRenderOptions::default(),
        )?;

        Ok(output
            .blocks
            .into_iter()
            .next()
            .expect("test block should render"))
    }

    fn test_font_post_script_name(faces: &[FaceInfo]) -> String {
        [
            "Yu Gothic",
            "MS Gothic",
            "Noto Sans CJK JP",
            "Noto Sans",
            "Arial",
            "DejaVu Sans",
            "Liberation Sans",
        ]
        .into_iter()
        .find_map(|name| face_post_script_name(faces, name))
        .or_else(|| {
            faces
                .iter()
                .find(|face| !face.post_script_name.is_empty())
                .map(|face| face.post_script_name.clone())
        })
        .expect("no system font available for tests")
    }

    fn assert_transparent_sprite_border(sprite: &RgbaImage) {
        assert!(sprite.width() > 2 && sprite.height() > 2);
        let last_x = sprite.width() - 1;
        let last_y = sprite.height() - 1;
        let top = (0..sprite.width())
            .filter(|&x| sprite.get_pixel(x, 0).0[3] != 0)
            .count();
        let bottom = (0..sprite.width())
            .filter(|&x| sprite.get_pixel(x, last_y).0[3] != 0)
            .count();
        let left = (0..sprite.height())
            .filter(|&y| sprite.get_pixel(0, y).0[3] != 0)
            .count();
        let right = (0..sprite.height())
            .filter(|&y| sprite.get_pixel(last_x, y).0[3] != 0)
            .count();
        let left_columns = (0..sprite.width().min(8))
            .map(|x| {
                (0..sprite.height())
                    .filter(|&y| sprite.get_pixel(x, y).0[3] != 0)
                    .count()
            })
            .collect::<Vec<_>>();
        let right_columns = (sprite.width().saturating_sub(8)..sprite.width())
            .map(|x| {
                (0..sprite.height())
                    .filter(|&y| sprite.get_pixel(x, y).0[3] != 0)
                    .count()
            })
            .collect::<Vec<_>>();
        let left_max_alpha = (0..sprite.height())
            .map(|y| sprite.get_pixel(0, y).0[3])
            .max()
            .unwrap_or(0);
        let right_max_alpha = (0..sprite.height())
            .map(|y| sprite.get_pixel(last_x, y).0[3])
            .max()
            .unwrap_or(0);

        assert_eq!(
            (top, bottom, left, right),
            (0, 0, 0, 0),
            "non-transparent border pixels in {}x{} sprite; left={left_columns:?} max={left_max_alpha}; right={right_columns:?} max={right_max_alpha}",
            sprite.width(),
            sprite.height()
        );
    }

    #[test]
    fn centred_sprite_transform_anchors_to_provided_box_center() {
        let anchor = LayoutBox {
            x: 100.0,
            y: 100.0,
            width: 200.0,
            height: 100.0,
        };
        let sprite_w = 100;
        let sprite_h = 50;

        let transform = centred_sprite_transform(anchor, sprite_w, sprite_h, 0.0);

        // Center of anchor is (200, 150).
        // Sprite (100x50) centered on (200, 150) starts at (150, 125).
        assert_eq!(transform.x, 150.0);
        assert_eq!(transform.y, 125.0);
    }
}
