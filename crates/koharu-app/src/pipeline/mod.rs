//! Pipeline: runs an ordered set of engines across one or more pages and
//! wraps each engine's output in one `Op::Batch` before applying via the
//! session's history.
//!
//! **Engines don't mutate the scene.** They return `Vec<Op>`; this driver
//! applies them transactionally (per-engine) against the active session.

pub mod artifacts;
pub mod engine;
mod engines;

pub use engines::support::{
    EligibleTextLine, build_han_only_translation_ops, eligible_lines_for_page,
};

pub use artifacts::Artifact;
pub use engine::{
    BoxFuture, Engine, EngineCtx, EngineInfo, EngineLoadFn, EngineWarningSink, PipelineRunOptions,
    Registry, build_order,
};
pub use engines::support;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::{Result, bail};
use koharu_core::{Op, PageId, PipelineStep};
use koharu_runtime::RuntimeManager;
use tracing::Instrument;

/// Observer for pipeline progress. `step_id` is the engine id of the step
/// about to run (or just finished); step_index / page_index are 0-based.
pub type ProgressSink = Arc<dyn Fn(ProgressTick) + Send + Sync>;

/// Observer for non-fatal step failures. Called once per failed step; the
/// pipeline skips the rest of that page's steps and moves on to the next
/// page.
pub type WarningSink = Arc<dyn Fn(WarningTick) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ProgressTick {
    /// Coarse UI-facing step tag derived from the engine's primary
    /// produced artifact. `None` for the final 100% tick where no engine
    /// is running.
    pub step: Option<PipelineStep>,
    /// Engine id (e.g. `"paddle-ocr-vl-1.6"`) for diagnostics + logs.
    pub step_id: String,
    pub step_index: usize,
    pub total_steps: usize,
    pub page_index: usize,
    pub total_pages: usize,
    pub overall_percent: u8,
}

#[derive(Debug, Clone)]
pub struct WarningTick {
    pub step_id: String,
    pub page_index: usize,
    pub total_pages: usize,
    pub message: String,
}

/// Returned by [`run`]. `warning_count == 0` means the run finished cleanly.
#[derive(Debug, Clone, Default)]
pub struct RunOutcome {
    pub warning_count: usize,
}

/// Map an engine's produced artifact to its UI step category. Stays
/// co-located with the engine metadata so adding a new engine can't
/// silently bypass the toolbar spinner — only the registered artifact
/// matters, not the engine's string id.
fn step_for(info: &EngineInfo) -> Option<PipelineStep> {
    info.produces.iter().find_map(|a| match a {
        Artifact::TextBoxes
        | Artifact::SegmentMask
        | Artifact::FontPredictions
        | Artifact::BubbleMask => Some(PipelineStep::Detect),
        Artifact::OcrText | Artifact::SourceTextBoxes => Some(PipelineStep::Ocr),
        Artifact::Translations => Some(PipelineStep::LlmGenerate),
        Artifact::TypographyStyles => Some(PipelineStep::Typography),
        Artifact::Inpainted => Some(PipelineStep::Inpaint),
        Artifact::FinalRender => Some(PipelineStep::Render),
        // Non-UI-facing artifacts (inputs, intermediate sprites) — no
        // toolbar step tag.
        _ => None,
    })
}

use crate::config::SourceTextPolicy;
use crate::llm;
use crate::renderer;
use crate::session::ProjectSession;
use crate::typography::TypographyPlanner;

// ---------------------------------------------------------------------------
// Spec + scope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PipelineSpec {
    pub scope: Scope,
    pub steps: Vec<String>,
    pub options: PipelineRunOptions,
}

#[derive(Debug, Clone)]
pub enum Scope {
    WholeProject,
    Pages(Vec<PageId>),
}

struct ResolvedInfos {
    infos: Vec<&'static EngineInfo>,
    detector_selected: bool,
}

fn touches_text_pipeline(info: &EngineInfo) -> bool {
    const TEXT_ARTIFACTS: &[Artifact] = &[
        Artifact::TextBoxes,
        Artifact::OcrText,
        Artifact::SourceTextBoxes,
        Artifact::FontPredictions,
        Artifact::SegmentMask,
        Artifact::BubbleMask,
        Artifact::Translations,
        Artifact::TypographyStyles,
        Artifact::Inpainted,
        Artifact::RenderedSprites,
        Artifact::FinalRender,
    ];
    info.needs
        .iter()
        .chain(info.produces.iter())
        .any(|artifact| TEXT_ARTIFACTS.contains(artifact))
}

fn infos_for_spec(spec: &PipelineSpec) -> Result<ResolvedInfos> {
    let mut infos = spec
        .steps
        .iter()
        .map(|id| Registry::find(id))
        .collect::<Result<Vec<_>>>()?;
    if spec.options.source_text_policy == SourceTextPolicy::HanOnly
        && spec.options.region.is_none()
        && infos.iter().any(|info| info.id == "comic-text-detector")
    {
        bail!(
            "comic-text-detector also runs segmentation and is unavailable in HanOnly; use pp-doclayout-v3, anime-text, or comic-text-bubble-detector"
        );
    }
    let detector_selected = infos
        .iter()
        .any(|info| info.produces.contains(&Artifact::TextBoxes));
    if spec.options.source_text_policy == SourceTextPolicy::HanOnly
        && spec.options.region.is_none()
        && infos.iter().any(|info| touches_text_pipeline(info))
    {
        infos.retain(|info| !info.produces.contains(&Artifact::OcrText));
        if !infos.iter().any(|info| info.id == "pp-ocr-v5-source-gate") {
            infos.push(Registry::find("pp-ocr-v5-source-gate")?);
        }
    }
    Ok(ResolvedInfos {
        infos,
        detector_selected,
    })
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Execute `spec` against `session`. Each engine step becomes one `Op::Batch`
/// applied via the session's history (one undo step per step per page).
///
/// A failed step on a given page is non-fatal: the rest of that page's steps
/// are skipped (they typically depend on the failed step's output), one
/// [`WarningTick`] is emitted via `warnings`, and the driver moves on to the
/// next page. The function returns the total number of per-step warnings
/// that fired, letting callers flag the run as `CompletedWithErrors`.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(level = "info", skip_all)]
pub async fn run(
    session: Arc<ProjectSession>,
    registry: Arc<Registry>,
    runtime: Arc<RuntimeManager>,
    cpu: bool,
    llm: Arc<llm::Model>,
    renderer: Arc<renderer::Renderer>,
    typography_planner: Arc<TypographyPlanner>,
    spec: PipelineSpec,
    cancel: Arc<AtomicBool>,
    progress: Option<ProgressSink>,
    warnings: Option<WarningSink>,
) -> Result<RunOutcome> {
    let resolved = infos_for_spec(&spec)?;
    let order = build_order(&resolved.infos)?;

    let pages = match &spec.scope {
        Scope::WholeProject => session
            .scene
            .read()
            .pages
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        Scope::Pages(ids) => ids.clone(),
    };

    let total_pages = pages.len().max(1);
    let total_steps = order.len().max(1);
    let total_units = (total_pages * total_steps) as u64;
    let mut completed: u64 = 0;
    let warning_count = AtomicUsize::new(0);

    'pages: for (page_index, page_id) in pages.iter().enumerate() {
        let mut unsupported_seen = HashSet::new();
        if spec.options.source_text_policy == SourceTextPolicy::HanOnly {
            let scene = session.scene_snapshot();
            new_unsupported_geometry(&scene, *page_id, &mut unsupported_seen);
        }
        for (seq, &i) in order.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                bail!("cancelled");
            }
            let info = resolved.infos[i];

            if let Some(sink) = progress.as_ref() {
                let percent = ((completed * 100) / total_units).min(100) as u8;
                sink(ProgressTick {
                    step: step_for(info),
                    step_id: info.id.to_string(),
                    step_index: seq,
                    total_steps,
                    page_index,
                    total_pages,
                    overall_percent: percent,
                });
            }

            // The page must still exist (user may have deleted it mid-run).
            if !session.scene.read().pages.contains_key(page_id) {
                // Skip the remaining steps for a deleted page and credit all
                // of them against total_units so progress still reaches 100%.
                completed += (total_steps - seq) as u64;
                continue 'pages;
            }

            if info.id == "pp-ocr-v5-source-gate" {
                let scene = session.scene_snapshot();
                let has_candidates =
                    engines::source_language_gate::has_gate_candidates(&scene, *page_id);
                if !has_candidates && !resolved.detector_selected {
                    completed += 1;
                    continue;
                }
            }

            let engine = match registry.get(info.id, &runtime, cpu).await {
                Ok(e) => e,
                Err(err) => {
                    // Engine *load* failure: same recovery as a run failure.
                    report_step_failure(
                        info.id,
                        page_id,
                        seq,
                        page_index,
                        total_pages,
                        total_steps,
                        &err,
                        &warning_count,
                        warnings.as_ref(),
                    );
                    completed += (total_steps - seq) as u64;
                    continue 'pages;
                }
            };
            let (scene_epoch, scene_snap) = session.scene_snapshot_with_epoch();
            let engine_warnings: &EngineWarningSink<'_> = &|message| {
                warning_count.fetch_add(1, Ordering::Relaxed);
                if let Some(sink) = warnings.as_ref() {
                    sink(WarningTick {
                        step_id: info.id.to_string(),
                        page_index,
                        total_pages,
                        message,
                    });
                }
            };
            let ctx = EngineCtx {
                scene: &scene_snap,
                page: *page_id,
                blobs: &session.blobs,
                runtime: &runtime,
                cancel: &cancel,
                options: &spec.options,
                llm: &llm,
                renderer: &renderer,
                typography_planner: &typography_planner,
                warnings: Some(engine_warnings),
            };
            let step_result = async { engine.run(ctx).await }
                .instrument(tracing::info_span!("step", engine = info.id, page = %page_id))
                .await;
            let ops = match step_result {
                Ok(ops) => ops,
                Err(err) => {
                    report_step_failure(
                        info.id,
                        page_id,
                        seq,
                        page_index,
                        total_pages,
                        total_steps,
                        &err,
                        &warning_count,
                        warnings.as_ref(),
                    );
                    // Subsequent steps on this page almost always consume the
                    // failed step's artifact; skip the rest and move on.
                    completed += (total_steps - seq) as u64;
                    continue 'pages;
                }
            };
            completed += 1;
            if !ops.is_empty() {
                let batch = Op::Batch {
                    ops,
                    label: format!("{}: page {}", info.id, page_id),
                };
                let apply = if info.produces.contains(&Artifact::TypographyStyles) {
                    match session.apply_if_epoch(scene_epoch, batch)? {
                        Some(_) => Ok(()),
                        None => {
                            warning_count.fetch_add(1, Ordering::Relaxed);
                            if let Some(sink) = warnings.as_ref() {
                                sink(WarningTick {
                                    step_id: info.id.to_string(),
                                    page_index,
                                    total_pages,
                                    message: "Typography Planner result discarded because the scene changed"
                                        .into(),
                                });
                            }
                            continue;
                        }
                    }
                } else {
                    session.apply(batch).map(|_| ())
                };
                if let Err(err) = apply {
                    report_step_failure(
                        info.id,
                        page_id,
                        seq,
                        page_index,
                        total_pages,
                        total_steps,
                        &err,
                        &warning_count,
                        warnings.as_ref(),
                    );
                    continue 'pages;
                }
                if spec.options.source_text_policy == SourceTextPolicy::HanOnly {
                    let scene = session.scene_snapshot();
                    new_unsupported_geometry(&scene, *page_id, &mut unsupported_seen);
                }
            }

            if info.id == "pp-ocr-v5-source-gate"
                && engines::support::text_nodes(&session.scene_snapshot(), *page_id).is_empty()
            {
                completed += (total_steps - seq - 1) as u64;
                continue 'pages;
            }
        }
    }

    if let Some(sink) = progress.as_ref() {
        sink(ProgressTick {
            step: None,
            step_id: String::new(),
            step_index: total_steps.saturating_sub(1),
            total_steps,
            page_index: total_pages.saturating_sub(1),
            total_pages,
            overall_percent: 100,
        });
    }
    Ok(RunOutcome {
        warning_count: warning_count.load(Ordering::Relaxed),
    })
}

fn new_unsupported_geometry(
    scene: &koharu_core::Scene,
    page: PageId,
    seen: &mut HashSet<koharu_core::NodeId>,
) -> Vec<engines::support::UnsupportedTextGeometry> {
    let (_, unsupported) = engines::support::eligible_lines_for_page(scene, page);
    let new = unsupported
        .into_iter()
        .filter(|geometry| seen.insert(geometry.node_id))
        .collect::<Vec<_>>();
    for geometry in &new {
        tracing::warn!(
            node = %geometry.node_id,
            direction = ?geometry.direction,
            rotation_deg = geometry.rotation_deg,
            line_count = geometry.line_count,
            "skipping unsupported mixed text geometry"
        );
    }
    new
}

#[allow(clippy::too_many_arguments)]
fn report_step_failure(
    engine_id: &str,
    page_id: &PageId,
    step_index: usize,
    page_index: usize,
    total_pages: usize,
    total_steps: usize,
    err: &anyhow::Error,
    warning_count: &AtomicUsize,
    sink: Option<&WarningSink>,
) {
    let _ = total_steps;
    tracing::warn!(
        engine = engine_id,
        page = %page_id,
        step_index,
        "pipeline step failed: {err:#}"
    );
    warning_count.fetch_add(1, Ordering::Relaxed);
    if let Some(sink) = sink {
        sink(WarningTick {
            step_id: engine_id.to_string(),
            page_index,
            total_pages,
            message: format!("{err:#}"),
        });
    }
}

// ---------------------------------------------------------------------------
// Engine catalog building (API surface)
// ---------------------------------------------------------------------------

use koharu_core::{EngineCatalog, EngineCatalogEntry};

/// Build the engine catalog DTO for the API.
pub fn catalog() -> EngineCatalog {
    let entry = |info: &&EngineInfo| EngineCatalogEntry {
        id: info.id.to_string(),
        name: info.name.to_string(),
        produces: info.produces.iter().map(|a| format!("{a:?}")).collect(),
    };
    EngineCatalog {
        detectors: Registry::providers(Artifact::TextBoxes)
            .iter()
            .map(entry)
            .collect(),
        font_detectors: Registry::providers(Artifact::FontPredictions)
            .iter()
            .map(entry)
            .collect(),
        segmenters: Registry::providers(Artifact::SegmentMask)
            .iter()
            .map(entry)
            .collect(),
        bubble_segmenters: Registry::providers(Artifact::BubbleMask)
            .iter()
            .map(entry)
            .collect(),
        ocr: Registry::providers(Artifact::OcrText)
            .iter()
            .map(entry)
            .collect(),
        translators: Registry::providers(Artifact::Translations)
            .iter()
            .map(entry)
            .collect(),
        inpainters: Registry::providers(Artifact::Inpainted)
            .iter()
            .map(entry)
            .collect(),
        renderers: Registry::providers(Artifact::FinalRender)
            .iter()
            .map(entry)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;
    use std::time::Duration;

    use async_trait::async_trait;
    use camino::Utf8PathBuf;
    use image::{DynamicImage, Rgba, RgbaImage};
    use koharu_core::{
        BlobRef, FontSource, ImageData, ImageRole, MaskData, MaskRole, Node, NodeDataPatch, NodeId,
        NodeKind, NodePatch, Page, Scene, TextData, TextDataPatch, TextStyle, Transform,
    };
    use koharu_ml::pp_ocr_v5::PpOcrWordBox;
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::sync::Notify;

    fn unsupported_mixed_node(id: NodeId) -> Node {
        Node {
            id,
            transform: Transform {
                x: 10.0,
                y: 10.0,
                width: 50.0,
                height: 30.0,
                rotation_deg: 0.0,
            },
            visible: true,
            kind: NodeKind::Text(TextData {
                text: Some("English\n中文".to_string()),
                line_polygons: None,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn unsupported_geometry_warning_is_emitted_once() {
        let mut page = Page::new("page", 100, 100);
        let page_id = page.id;
        let first_id = NodeId::new();
        page.nodes
            .insert(first_id, unsupported_mixed_node(first_id));
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        let mut seen = HashSet::new();

        let first = new_unsupported_geometry(&scene, page_id, &mut seen);
        let duplicate = new_unsupported_geometry(&scene, page_id, &mut seen);

        assert_eq!(first.len(), 1);
        assert_eq!(first[0].node_id, first_id);
        assert!(duplicate.is_empty());
        let diagnostic = format!("{first:?}");
        assert!(!diagnostic.contains("English"));
        assert!(!diagnostic.contains("中文"));

        let second_id = NodeId::new();
        scene
            .pages
            .get_mut(&page_id)
            .expect("page")
            .nodes
            .insert(second_id, unsupported_mixed_node(second_id));
        let second = new_unsupported_geometry(&scene, page_id, &mut seen);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].node_id, second_id);
    }

    #[test]
    fn catalog_includes_anime_text_detector() {
        let catalog = catalog();

        assert!(catalog.detectors.iter().any(|engine| {
            engine.id == "anime-text"
                && engine.name == "Anime Text YOLO (N)"
                && engine.produces.iter().map(String::as_str).eq(["TextBoxes"])
        }));
    }

    #[test]
    fn registry_retains_all_production_engine_ids() {
        for id in [
            "anime-text",
            "aot-inpainting",
            "speech-bubble-segmentation",
            "comic-text-bubble-detector",
            "comic-text-detector",
            "comic-text-detector-seg",
            "flux2-klein",
            "lama-manga",
            "llm",
            "cloud-typography-planner",
            "manga-ocr",
            "mit48px-ocr",
            "paddle-ocr-vl-1.6",
            "pp-doclayout-v3",
            "koharu-renderer",
            "yuzumarker-font-detection",
        ] {
            assert!(Registry::find(id).is_ok(), "missing production engine {id}");
        }
    }

    fn resolved_ids(
        policy: SourceTextPolicy,
        region: Option<koharu_core::Region>,
    ) -> Vec<&'static str> {
        let resolved = infos_for_spec(&PipelineSpec {
            scope: Scope::WholeProject,
            steps: vec!["paddle-ocr-vl-1.6".into(), "koharu-renderer".into()],
            options: PipelineRunOptions {
                source_text_policy: policy,
                region,
                ..Default::default()
            },
        })
        .unwrap();
        resolved.infos.into_iter().map(|info| info.id).collect()
    }

    #[test]
    fn han_only_replaces_selected_ocr_with_gate() {
        let ids = resolved_ids(SourceTextPolicy::HanOnly, None);
        assert!(!ids.contains(&"paddle-ocr-vl-1.6"));
        assert!(ids.contains(&"pp-ocr-v5-source-gate"));
        assert!(ids.contains(&"koharu-renderer"));
    }

    #[test]
    fn all_text_keeps_selected_ocr_and_never_injects_gate() {
        let ids = resolved_ids(SourceTextPolicy::AllText, None);
        assert!(ids.contains(&"paddle-ocr-vl-1.6"));
        assert!(!ids.contains(&"pp-ocr-v5-source-gate"));
    }

    #[test]
    fn repair_region_never_injects_source_gate() {
        let ids = resolved_ids(
            SourceTextPolicy::HanOnly,
            Some(koharu_core::Region {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            }),
        );
        assert!(ids.contains(&"paddle-ocr-vl-1.6"));
        assert!(!ids.contains(&"pp-ocr-v5-source-gate"));
    }

    #[test]
    fn han_only_rejects_ctd_full_before_registry_load() {
        let Err(error) = infos_for_spec(&PipelineSpec {
            scope: Scope::WholeProject,
            steps: vec!["comic-text-detector".into()],
            options: PipelineRunOptions {
                source_text_policy: SourceTextPolicy::HanOnly,
                ..Default::default()
            },
        }) else {
            panic!("HanOnly must reject the combined detector")
        };
        let error = error.to_string();
        assert!(error.contains("comic-text-detector"));
        assert!(error.contains("pp-doclayout-v3"));
        assert!(error.contains("anime-text"));
        assert!(error.contains("comic-text-bubble-detector"));
    }

    struct PipelineFixture {
        _dir: TempDir,
        runtime: Arc<RuntimeManager>,
        registry: Arc<Registry>,
        llm: Arc<llm::Model>,
        renderer: Arc<renderer::Renderer>,
        session: Arc<ProjectSession>,
        page: PageId,
        text: NodeId,
        font: String,
    }

    impl PipelineFixture {
        fn new(source_text: &str, translation: &str) -> anyhow::Result<Self> {
            let dir = tempfile::tempdir()?;
            let runtime = Arc::new(RuntimeManager::new(dir.path(), ComputePolicy::CpuOnly)?);
            let registry = Arc::new(Registry::new());
            let llm = Arc::new(llm::Model::empty_for_test((*runtime).clone(), true));
            let renderer = Arc::new(renderer::Renderer::new()?);
            let font = renderer
                .available_fonts()?
                .into_iter()
                .find(|font| font.source == FontSource::System || font.cached)
                .ok_or_else(|| anyhow::anyhow!("no safe system font available for test"))?
                .post_script_name;
            let path = Utf8PathBuf::from_path_buf(dir.path().join("pipeline.khrproj"))
                .map_err(|_| anyhow::anyhow!("temporary project path is not UTF-8"))?;
            let session = ProjectSession::create(&path, "pipeline")?;
            let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                100,
                100,
                Rgba([240, 240, 240, 255]),
            ));
            let source_blob = session.blobs.put_webp(&source)?;
            let text = NodeId::new();
            let mut page = Page::new("page", 100, 100);
            let page_id = page.id;
            let source_id = NodeId::new();
            page.nodes.insert(
                source_id,
                Node {
                    id: source_id,
                    transform: Transform::default(),
                    visible: true,
                    kind: NodeKind::Image(ImageData {
                        role: ImageRole::Source,
                        blob: source_blob,
                        opacity: 1.0,
                        natural_width: 100,
                        natural_height: 100,
                        name: None,
                    }),
                },
            );
            page.nodes.insert(
                text,
                Node {
                    id: text,
                    transform: Transform {
                        x: 10.0,
                        y: 20.0,
                        width: 80.0,
                        height: 40.0,
                        rotation_deg: 0.0,
                    },
                    visible: true,
                    kind: NodeKind::Text(TextData {
                        detector: Some(engines::support::SOURCE_GATE_TARGET_DETECTOR.to_string()),
                        text: Some(source_text.to_string()),
                        translation: Some(translation.to_string()),
                        sprite: Some(BlobRef::new("old-sprite")),
                        sprite_transform: Some(Transform {
                            x: 1.0,
                            y: 2.0,
                            width: 3.0,
                            height: 4.0,
                            rotation_deg: 0.0,
                        }),
                        ..Default::default()
                    }),
                },
            );
            session.apply(Op::AddPage { page, at: 0 })?;
            Ok(Self {
                _dir: dir,
                runtime,
                registry,
                llm,
                renderer,
                session,
                page: page_id,
                text,
                font,
            })
        }

        async fn run(
            &self,
            planner: Arc<TypographyPlanner>,
            warnings: Arc<Mutex<Vec<WarningTick>>>,
        ) -> anyhow::Result<RunOutcome> {
            let warning_sink: WarningSink = Arc::new(move |warning| {
                warnings.lock().unwrap().push(warning);
            });
            run(
                self.session.clone(),
                self.registry.clone(),
                self.runtime.clone(),
                true,
                self.llm.clone(),
                self.renderer.clone(),
                planner,
                PipelineSpec {
                    scope: Scope::Pages(vec![self.page]),
                    steps: vec!["cloud-typography-planner".into(), "koharu-renderer".into()],
                    options: PipelineRunOptions {
                        source_text_policy: SourceTextPolicy::HanOnly,
                        default_font: Some(self.font.clone()),
                        ..Default::default()
                    },
                },
                Arc::new(AtomicBool::new(false)),
                None,
                Some(warning_sink),
            )
            .await
        }
    }

    type RendererInspect = Arc<dyn Fn(&Scene, PageId) + Send + Sync>;

    struct InspectRenderer {
        calls: Arc<AtomicUsize>,
        inspect: RendererInspect,
    }

    struct CountingEngine {
        calls: Arc<AtomicUsize>,
        remove_text: bool,
    }

    struct ProductionGateEngine {
        calls: Arc<AtomicUsize>,
        pp_calls: Arc<AtomicUsize>,
        vl_calls: Arc<AtomicUsize>,
        word_boxes: HashMap<NodeId, Vec<PpOcrWordBox>>,
        vl_texts: Vec<String>,
    }

    #[async_trait]
    impl Engine for CountingEngine {
        async fn run(&self, ctx: EngineCtx<'_>) -> anyhow::Result<Vec<Op>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if !self.remove_text {
                return Ok(Vec::new());
            }
            let page = ctx
                .scene
                .page(ctx.page)
                .ok_or_else(|| anyhow::anyhow!("page not found"))?;
            Ok(page
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, (_, node))| matches!(node.kind, NodeKind::Text(_)))
                .map(|(prev_index, (id, node))| Op::RemoveNode {
                    page: ctx.page,
                    id: *id,
                    prev_node: node.clone(),
                    prev_index,
                })
                .collect())
        }
    }

    #[async_trait]
    impl Engine for ProductionGateEngine {
        async fn run(&self, ctx: EngineCtx<'_>) -> anyhow::Result<Vec<Op>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let image = engines::support::load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
            engines::source_language_gate::dispatch_source_gate(
                &image,
                ctx.scene,
                ctx.page,
                |node_id, _| {
                    self.pp_calls.fetch_add(1, Ordering::Relaxed);
                    Ok(self.word_boxes.get(&node_id).cloned().unwrap_or_default())
                },
                |crops| {
                    self.vl_calls.fetch_add(crops.len(), Ordering::Relaxed);
                    std::future::ready(Ok(self.vl_texts.clone()))
                },
            )
            .await
        }
    }

    #[async_trait]
    impl Engine for InspectRenderer {
        async fn run(&self, ctx: EngineCtx<'_>) -> anyhow::Result<Vec<Op>> {
            (self.inspect)(ctx.scene, ctx.page);
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        }
    }

    fn install_renderer(
        fixture: &PipelineFixture,
        calls: Arc<AtomicUsize>,
        inspect: impl Fn(&Scene, PageId) + Send + Sync + 'static,
    ) {
        fixture.registry.insert_test_engine(
            "koharu-renderer",
            Arc::new(InspectRenderer {
                calls,
                inspect: Arc::new(inspect),
            }),
        );
    }

    fn install_counting_engine(
        fixture: &PipelineFixture,
        id: &str,
        calls: Arc<AtomicUsize>,
        remove_text: bool,
    ) {
        fixture
            .registry
            .insert_test_engine(id, Arc::new(CountingEngine { calls, remove_text }));
    }

    fn install_production_gate(
        fixture: &PipelineFixture,
        word_boxes: HashMap<NodeId, Vec<PpOcrWordBox>>,
        vl_texts: Vec<String>,
    ) -> (Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let pp_calls = Arc::new(AtomicUsize::new(0));
        let vl_calls = Arc::new(AtomicUsize::new(0));
        fixture.registry.insert_test_engine(
            "pp-ocr-v5-source-gate",
            Arc::new(ProductionGateEngine {
                calls: calls.clone(),
                pp_calls: pp_calls.clone(),
                vl_calls: vl_calls.clone(),
                word_boxes,
                vl_texts,
            }),
        );
        (calls, pp_calls, vl_calls)
    }

    fn pp_word(
        text: &str,
        line_index: usize,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    ) -> PpOcrWordBox {
        PpOcrWordBox {
            line_index,
            text: text.into(),
            bbox: [left, top, right, bottom],
            confidence: 0.9,
        }
    }

    fn make_legacy_candidate(fixture: &PipelineFixture, text: &str) -> anyhow::Result<()> {
        fixture.session.apply(Op::UpdateNode {
            page: fixture.page,
            id: fixture.text,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    detector: Some(None),
                    text: Some(Some(text.into())),
                    translation: Some(None),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        })?;
        Ok(())
    }

    fn visible_texts(scene: &Scene, page: PageId) -> Vec<String> {
        engines::support::text_nodes(scene, page)
            .into_iter()
            .filter_map(|(_, _, text)| text.text.clone())
            .collect()
    }

    fn protected_texts(scene: &Scene, page: PageId) -> Vec<String> {
        scene
            .page(page)
            .into_iter()
            .flat_map(|page| page.nodes.values())
            .filter_map(|node| match &node.kind {
                NodeKind::Text(text)
                    if text.detector.as_deref()
                        == Some(engines::support::SOURCE_GATE_PROTECTED_DETECTOR) =>
                {
                    text.text.clone()
                }
                _ => None,
            })
            .collect()
    }

    async fn run_fixture_steps(
        fixture: &PipelineFixture,
        steps: &[&str],
    ) -> anyhow::Result<RunOutcome> {
        run_fixture_steps_with_options(
            fixture,
            steps,
            PipelineRunOptions {
                source_text_policy: SourceTextPolicy::HanOnly,
                ..Default::default()
            },
        )
        .await
    }

    async fn run_fixture_steps_with_options(
        fixture: &PipelineFixture,
        steps: &[&str],
        options: PipelineRunOptions,
    ) -> anyhow::Result<RunOutcome> {
        run(
            fixture.session.clone(),
            fixture.registry.clone(),
            fixture.runtime.clone(),
            true,
            fixture.llm.clone(),
            fixture.renderer.clone(),
            Arc::new(TypographyPlanner::default()),
            PipelineSpec {
                scope: Scope::Pages(vec![fixture.page]),
                steps: steps.iter().map(|step| (*step).into()).collect(),
                options,
            },
            Arc::new(AtomicBool::new(false)),
            None,
            None,
        )
        .await
    }

    #[tokio::test]
    async fn han_only_downstream_only_existing_english_runs_gate_and_skips_renderer()
    -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("English", "English")?;
        fixture.session.apply(Op::UpdateNode {
            page: fixture.page,
            id: fixture.text,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    detector: Some(None),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        })?;
        let gate_calls = Arc::new(AtomicUsize::new(0));
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_counting_engine(&fixture, "pp-ocr-v5-source-gate", gate_calls.clone(), true);
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        let outcome = run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        assert_eq!(outcome.warning_count, 0);
        assert_eq!(gate_calls.load(Ordering::Relaxed), 1);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 0);
        assert!(
            fixture
                .session
                .scene_snapshot()
                .node(fixture.page, fixture.text)
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn han_only_zero_text_standalone_renderer_keeps_existing_behavior_without_loading_gate()
    -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("English", "English")?;
        let node = fixture
            .session
            .scene_snapshot()
            .node(fixture.page, fixture.text)
            .unwrap()
            .clone();
        fixture.session.apply(Op::RemoveNode {
            page: fixture.page,
            id: fixture.text,
            prev_node: node,
            prev_index: 1,
        })?;
        let gate_calls = Arc::new(AtomicUsize::new(0));
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_counting_engine(&fixture, "pp-ocr-v5-source-gate", gate_calls.clone(), false);
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        let outcome = run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        assert_eq!(outcome.warning_count, 0);
        assert_eq!(gate_calls.load(Ordering::Relaxed), 0);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn han_only_detector_zero_candidates_still_runs_gate_then_stops_renderer()
    -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("stale", "stale")?;
        let detector_calls = Arc::new(AtomicUsize::new(0));
        let gate_calls = Arc::new(AtomicUsize::new(0));
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_counting_engine(&fixture, "pp-doclayout-v3", detector_calls.clone(), true);
        install_counting_engine(&fixture, "pp-ocr-v5-source-gate", gate_calls.clone(), false);
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        let outcome = run_fixture_steps(&fixture, &["pp-doclayout-v3", "koharu-renderer"]).await?;

        assert_eq!(outcome.warning_count, 0);
        assert_eq!(detector_calls.load(Ordering::Relaxed), 1);
        assert_eq!(gate_calls.load(Ordering::Relaxed), 1);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn han_only_empty_source_gate_stops_every_downstream_engine() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("English", "English")?;
        fixture.session.apply(Op::UpdateNode {
            page: fixture.page,
            id: fixture.text,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    detector: Some(None),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        })?;
        let detector_calls = Arc::new(AtomicUsize::new(0));
        install_counting_engine(&fixture, "pp-doclayout-v3", detector_calls.clone(), false);
        let gate_calls = Arc::new(AtomicUsize::new(0));
        install_counting_engine(&fixture, "pp-ocr-v5-source-gate", gate_calls.clone(), true);
        let downstream = [
            "yuzumarker-font-detection",
            "speech-bubble-segmentation",
            "comic-text-detector-seg",
            "llm",
            "cloud-typography-planner",
            "lama-manga",
            "koharu-renderer",
        ];
        let downstream_calls = downstream
            .iter()
            .map(|id| {
                let calls = Arc::new(AtomicUsize::new(0));
                install_counting_engine(&fixture, id, calls.clone(), false);
                calls
            })
            .collect::<Vec<_>>();

        let mut steps = vec!["pp-doclayout-v3"];
        steps.extend(downstream);
        let outcome = run_fixture_steps(&fixture, &steps).await?;

        assert_eq!(outcome.warning_count, 0);
        assert_eq!(detector_calls.load(Ordering::Relaxed), 1);
        assert_eq!(gate_calls.load(Ordering::Relaxed), 1);
        for (id, calls) in downstream.iter().zip(downstream_calls) {
            assert_eq!(calls.load(Ordering::Relaxed), 0, "unexpected call: {id}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn han_only_detect_then_ocr_reuses_accepted_targets_without_rerunning_gate_models()
    -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("中文", "translated")?;
        let gate_calls = Arc::new(AtomicUsize::new(0));
        let segment_calls = Arc::new(AtomicUsize::new(0));
        install_counting_engine(&fixture, "pp-ocr-v5-source-gate", gate_calls.clone(), false);
        install_counting_engine(
            &fixture,
            "comic-text-detector-seg",
            segment_calls.clone(),
            false,
        );

        let outcome =
            run_fixture_steps(&fixture, &["paddle-ocr-vl-1.6", "comic-text-detector-seg"]).await?;

        assert_eq!(outcome.warning_count, 0);
        assert_eq!(gate_calls.load(Ordering::Relaxed), 0);
        assert_eq!(segment_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn pure_english_has_no_visible_text_nodes_and_source_pixels_are_unchanged()
    -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("English only", "unused")?;
        make_legacy_candidate(&fixture, "English only")?;
        let before = {
            let scene = fixture.session.scene_snapshot();
            engines::support::source_node(&scene, fixture.page)?
                .1
                .blob
                .clone()
        };
        let (_, pp_calls, vl_calls) = install_production_gate(
            &fixture,
            HashMap::from([(
                fixture.text,
                vec![pp_word("English", 0, 0.0, 0.0, 60.0, 20.0)],
            )]),
            Vec::new(),
        );
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        let scene = fixture.session.scene_snapshot();
        assert!(visible_texts(&scene, fixture.page).is_empty());
        assert_eq!(pp_calls.load(Ordering::Relaxed), 1);
        assert_eq!(vl_calls.load(Ordering::Relaxed), 0);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            engines::support::source_node(&scene, fixture.page)?.1.blob,
            before
        );
        Ok(())
    }

    #[tokio::test]
    async fn complete_english_word_is_preserved_while_adjacent_han_runs_downstream()
    -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("Peach蜜桃臀", "unused")?;
        make_legacy_candidate(&fixture, "Peach蜜桃臀")?;
        install_production_gate(
            &fixture,
            HashMap::from([(
                fixture.text,
                vec![
                    pp_word("Peach", 0, 0.0, 0.0, 30.0, 20.0),
                    pp_word("蜜桃臀", 0, 35.0, 0.0, 70.0, 20.0),
                ],
            )]),
            vec!["Peach蜜桃臀".into()],
        );
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        let scene = fixture.session.scene_snapshot();
        assert_eq!(visible_texts(&scene, fixture.page), ["蜜桃臀"]);
        assert_eq!(protected_texts(&scene, fixture.page), ["Peach"]);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn english_on_both_sides_keeps_two_disjoint_source_regions() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("Slim中文Fit", "unused")?;
        make_legacy_candidate(&fixture, "Slim中文Fit")?;
        install_production_gate(
            &fixture,
            HashMap::from([(
                fixture.text,
                vec![
                    pp_word("Slim", 0, 0.0, 0.0, 20.0, 20.0),
                    pp_word("中文", 0, 25.0, 0.0, 45.0, 20.0),
                    pp_word("Fit", 0, 50.0, 0.0, 70.0, 20.0),
                ],
            )]),
            vec!["Slim中文Fit".into()],
        );
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        let scene = fixture.session.scene_snapshot();
        assert_eq!(visible_texts(&scene, fixture.page), ["中文"]);
        assert_eq!(protected_texts(&scene, fixture.page), ["Slim", "Fit"]);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn single_latin_label_with_han_is_one_translation_target() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("S型曲线", "unused")?;
        make_legacy_candidate(&fixture, "S型曲线")?;
        install_production_gate(
            &fixture,
            HashMap::from([(
                fixture.text,
                vec![
                    pp_word("S", 0, 0.0, 0.0, 10.0, 20.0),
                    pp_word("型曲线", 0, 10.0, 0.0, 50.0, 20.0),
                ],
            )]),
            vec!["S型曲线".into()],
        );
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        let scene = fixture.session.scene_snapshot();
        assert_eq!(visible_texts(&scene, fixture.page), ["S型曲线"]);
        assert!(protected_texts(&scene, fixture.page).is_empty());
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn single_latin_on_another_line_is_protected_not_translated() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("S\n中文", "unused")?;
        make_legacy_candidate(&fixture, "S\n中文")?;
        install_production_gate(
            &fixture,
            HashMap::from([(
                fixture.text,
                vec![
                    pp_word("S", 0, 0.0, 0.0, 10.0, 10.0),
                    pp_word("中文", 1, 0.0, 20.0, 30.0, 35.0),
                ],
            )]),
            vec!["S\n中文".into()],
        );
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        let scene = fixture.session.scene_snapshot();
        assert_eq!(visible_texts(&scene, fixture.page), ["中文"]);
        assert_eq!(protected_texts(&scene, fixture.page), ["S"]);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn han_lines_around_english_create_two_tight_downstream_nodes() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("中文一\nEnglish\n中文二", "unused")?;
        make_legacy_candidate(&fixture, "中文一\nEnglish\n中文二")?;
        install_production_gate(
            &fixture,
            HashMap::from([(
                fixture.text,
                vec![
                    pp_word("中文一", 0, 0.0, 0.0, 30.0, 10.0),
                    pp_word("English", 1, 0.0, 12.0, 45.0, 22.0),
                    pp_word("中文二", 2, 0.0, 25.0, 30.0, 35.0),
                ],
            )]),
            vec!["中文一\nEnglish\n中文二".into()],
        );
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        let scene = fixture.session.scene_snapshot();
        assert_eq!(visible_texts(&scene, fixture.page), ["中文一", "中文二"]);
        assert_eq!(protected_texts(&scene, fixture.page), ["English"]);
        let transforms = engines::support::text_nodes(&scene, fixture.page)
            .into_iter()
            .map(|(_, transform, _)| *transform)
            .collect::<Vec<_>>();
        assert!(transforms[0].y + transforms[0].height <= transforms[1].y);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn pp_false_han_requires_vl_confirmation_before_downstream() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("误报", "unused")?;
        make_legacy_candidate(&fixture, "误报")?;
        let (_, pp_calls, vl_calls) = install_production_gate(
            &fixture,
            HashMap::from([(fixture.text, vec![pp_word("误报", 0, 0.0, 0.0, 30.0, 20.0)])]),
            vec!["English".into()],
        );
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        let scene = fixture.session.scene_snapshot();
        assert!(visible_texts(&scene, fixture.page).is_empty());
        assert_eq!(pp_calls.load(Ordering::Relaxed), 1);
        assert_eq!(vl_calls.load(Ordering::Relaxed), 1);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn separable_ai_han_keeps_ai_and_translates_han() -> anyhow::Result<()> {
        let separated = PipelineFixture::new("AI智能塑形", "unused")?;
        make_legacy_candidate(&separated, "AI智能塑形")?;
        install_production_gate(
            &separated,
            HashMap::from([(
                separated.text,
                vec![
                    pp_word("AI", 0, 0.0, 0.0, 20.0, 20.0),
                    pp_word("智能塑形", 0, 25.0, 0.0, 70.0, 20.0),
                ],
            )]),
            vec!["AI智能塑形".into()],
        );
        let separated_renderer = Arc::new(AtomicUsize::new(0));
        install_renderer(&separated, separated_renderer.clone(), |_, _| {});
        run_fixture_steps(&separated, &["koharu-renderer"]).await?;
        let scene = separated.session.scene_snapshot();
        assert_eq!(visible_texts(&scene, separated.page), ["智能塑形"]);
        assert_eq!(protected_texts(&scene, separated.page), ["AI"]);
        assert_eq!(separated_renderer.load(Ordering::Relaxed), 1);

        let unseparated = PipelineFixture::new("AI智能塑形", "unused")?;
        make_legacy_candidate(&unseparated, "AI智能塑形")?;
        install_production_gate(
            &unseparated,
            HashMap::from([(
                unseparated.text,
                vec![pp_word("AI智能塑形", 0, 0.0, 0.0, 70.0, 20.0)],
            )]),
            vec!["AI智能塑形".into()],
        );
        let unseparated_renderer = Arc::new(AtomicUsize::new(0));
        install_renderer(&unseparated, unseparated_renderer.clone(), |_, _| {});
        run_fixture_steps(&unseparated, &["koharu-renderer"]).await?;
        let scene = unseparated.session.scene_snapshot();
        assert!(visible_texts(&scene, unseparated.page).is_empty());
        assert_eq!(unseparated_renderer.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn unsafe_mixed_geometry_is_removed_and_never_reaches_downstream() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("English中文", "unused")?;
        make_legacy_candidate(&fixture, "English中文")?;
        fixture.session.apply(Op::UpdateNode {
            page: fixture.page,
            id: fixture.text,
            patch: NodePatch {
                transform: Some(Transform {
                    rotation_deg: 30.0,
                    ..fixture
                        .session
                        .scene_snapshot()
                        .node(fixture.page, fixture.text)
                        .unwrap()
                        .transform
                }),
                ..Default::default()
            },
            prev: NodePatch::default(),
        })?;
        let (_, pp_calls, vl_calls) = install_production_gate(&fixture, HashMap::new(), Vec::new());
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        assert!(visible_texts(&fixture.session.scene_snapshot(), fixture.page).is_empty());
        assert_eq!(pp_calls.load(Ordering::Relaxed), 0);
        assert_eq!(vl_calls.load(Ordering::Relaxed), 0);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn empty_targets_keep_repair_brush_inpainted_pixels() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("English", "unused")?;
        make_legacy_candidate(&fixture, "English")?;
        let source_blob = {
            let scene = fixture.session.scene_snapshot();
            engines::support::source_node(&scene, fixture.page)?
                .1
                .blob
                .clone()
        };
        let mut at = fixture
            .session
            .scene_snapshot()
            .page(fixture.page)
            .unwrap()
            .nodes
            .len();
        for role in [ImageRole::Inpainted, ImageRole::Rendered] {
            let id = NodeId::new();
            fixture.session.apply(Op::AddNode {
                page: fixture.page,
                node: Node {
                    id,
                    transform: Transform::default(),
                    visible: true,
                    kind: NodeKind::Image(ImageData {
                        role,
                        blob: source_blob.clone(),
                        opacity: 1.0,
                        natural_width: 100,
                        natural_height: 100,
                        name: None,
                    }),
                },
                at,
            })?;
            at += 1;
        }
        for role in [MaskRole::BrushInpaint, MaskRole::Segment, MaskRole::Bubble] {
            let id = NodeId::new();
            fixture.session.apply(Op::AddNode {
                page: fixture.page,
                node: Node {
                    id,
                    transform: Transform::default(),
                    visible: true,
                    kind: NodeKind::Mask(MaskData {
                        role,
                        blob: source_blob.clone(),
                    }),
                },
                at,
            })?;
            at += 1;
        }
        install_production_gate(
            &fixture,
            HashMap::from([(
                fixture.text,
                vec![pp_word("English", 0, 0.0, 0.0, 50.0, 20.0)],
            )]),
            Vec::new(),
        );
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});

        run_fixture_steps(&fixture, &["koharu-renderer"]).await?;

        let scene = fixture.session.scene_snapshot();
        assert!(
            engines::support::find_image_node(&scene, fixture.page, ImageRole::Inpainted).is_some()
        );
        assert!(
            engines::support::find_mask_node(&scene, fixture.page, MaskRole::BrushInpaint)
                .is_some()
        );
        assert!(
            engines::support::find_image_node(&scene, fixture.page, ImageRole::Rendered).is_none()
        );
        assert!(
            engines::support::find_mask_node(&scene, fixture.page, MaskRole::Segment).is_none()
        );
        assert!(engines::support::find_mask_node(&scene, fixture.page, MaskRole::Bubble).is_none());
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn all_text_keeps_existing_nodes_and_runs_existing_ocr_path() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("English", "unused")?;
        let second = NodeId::new();
        fixture.session.apply(Op::AddNode {
            page: fixture.page,
            node: Node {
                id: second,
                transform: Transform {
                    x: 10.0,
                    y: 70.0,
                    width: 40.0,
                    height: 20.0,
                    rotation_deg: 0.0,
                },
                visible: true,
                kind: NodeKind::Text(TextData {
                    text: Some("中文".into()),
                    ..Default::default()
                }),
            },
            at: 2,
        })?;
        let gate_calls = Arc::new(AtomicUsize::new(0));
        let ocr_calls = Arc::new(AtomicUsize::new(0));
        install_counting_engine(&fixture, "pp-ocr-v5-source-gate", gate_calls.clone(), false);
        install_counting_engine(&fixture, "paddle-ocr-vl-1.6", ocr_calls.clone(), false);

        run_fixture_steps_with_options(
            &fixture,
            &["paddle-ocr-vl-1.6"],
            PipelineRunOptions {
                source_text_policy: SourceTextPolicy::AllText,
                ..Default::default()
            },
        )
        .await?;

        let scene = fixture.session.scene_snapshot();
        assert_eq!(visible_texts(&scene, fixture.page).len(), 2);
        assert_eq!(gate_calls.load(Ordering::Relaxed), 0);
        assert_eq!(ocr_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    fn text(scene: &Scene, page: PageId, node: NodeId) -> &TextData {
        match &scene.node(page, node).unwrap().kind {
            NodeKind::Text(text) => text,
            _ => panic!("expected text node"),
        }
    }

    fn valid_response(node: NodeId, font: &str) -> String {
        json!({
            "nodes": [{
                "nodeId": node,
                "lines": ["Translated", "text"],
                "style": {
                    "fontFamily": font,
                    "fontSize": 18.0,
                    "color": [1, 2, 3, 255],
                    "stroke": null,
                    "effect": null,
                    "textAlign": "center"
                }
            }]
        })
        .to_string()
    }

    #[tokio::test]
    async fn typography_planner_soft_warning_still_runs_renderer() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("中文", "Translated text")?;
        let calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, calls.clone(), |_, _| {});
        let planner = Arc::new(TypographyPlanner::with_test_sender(
            Arc::new(|_, _| Box::pin(async { Ok("not json".into()) })),
            Duration::from_secs(1),
        ));
        let warnings = Arc::new(Mutex::new(Vec::new()));

        let outcome = fixture.run(planner, warnings.clone()).await?;

        assert_eq!(outcome.warning_count, 1);
        assert_eq!(warnings.lock().unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn stalled_typography_request_warns_once_without_ops_and_continues_renderer()
    -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("中文", "Translated text")?;
        let calls = Arc::new(AtomicUsize::new(0));
        let text_id = fixture.text;
        install_renderer(&fixture, calls.clone(), move |scene, page| {
            let text = text(scene, page, text_id);
            assert_eq!(text.translation.as_deref(), Some("Translated text"));
            assert!(text.style.is_none());
            assert!(!text.typography_plan_verified);
            assert!(text.sprite.is_some());
            assert!(text.sprite_transform.is_some());
        });
        let planner = Arc::new(TypographyPlanner::with_test_sender(
            Arc::new(|_, _| Box::pin(std::future::pending())),
            Duration::from_millis(10),
        ));
        let warnings = Arc::new(Mutex::new(Vec::new()));

        let outcome = fixture.run(planner, warnings.clone()).await?;

        assert_eq!(outcome.warning_count, 1);
        assert_eq!(warnings.lock().unwrap().len(), 1);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let scene = fixture.session.scene_snapshot();
        let text = text(&scene, fixture.page, fixture.text);
        assert_eq!(text.translation.as_deref(), Some("Translated text"));
        assert!(text.style.is_none());
        assert!(!text.typography_plan_verified);
        assert!(text.sprite.is_some());
        assert!(text.sprite_transform.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn typography_planner_applies_valid_plan_before_renderer() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("中文", "Translated text")?;
        let calls = Arc::new(AtomicUsize::new(0));
        let text_id = fixture.text;
        install_renderer(&fixture, calls.clone(), move |scene, page| {
            let text = text(scene, page, text_id);
            assert_eq!(text.translation.as_deref(), Some("Translated\ntext"));
            assert_eq!(text.style.as_ref().unwrap().color, [1, 2, 3, 255]);
            assert!(text.typography_plan_verified);
            assert!(text.sprite.is_none());
            assert!(text.sprite_transform.is_none());
        });
        let response = valid_response(fixture.text, &fixture.font);
        let planner = Arc::new(TypographyPlanner::with_test_sender(
            Arc::new(move |_, _| {
                let response = response.clone();
                Box::pin(async move { Ok(response) })
            }),
            Duration::from_secs(1),
        ));
        let warnings = Arc::new(Mutex::new(Vec::new()));

        let outcome = fixture.run(planner, warnings.clone()).await?;

        assert_eq!(outcome.warning_count, 0);
        assert!(warnings.lock().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn typography_planner_empty_targets_skip_sender_and_run_renderer() -> anyhow::Result<()> {
        let fixture = PipelineFixture::new("English", "English")?;
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        install_renderer(&fixture, renderer_calls.clone(), |_, _| {});
        let sender_calls = Arc::new(AtomicUsize::new(0));
        let counted = sender_calls.clone();
        let planner = Arc::new(TypographyPlanner::with_test_sender(
            Arc::new(move |_, _| {
                counted.fetch_add(1, Ordering::Relaxed);
                Box::pin(async { Ok(String::new()) })
            }),
            Duration::from_secs(1),
        ));
        let warnings = Arc::new(Mutex::new(Vec::new()));

        let outcome = fixture.run(planner, warnings.clone()).await?;

        assert_eq!(outcome.warning_count, 0);
        assert_eq!(sender_calls.load(Ordering::Relaxed), 0);
        assert_eq!(renderer_calls.load(Ordering::Relaxed), 1);
        assert!(warnings.lock().unwrap().is_empty());
        Ok(())
    }

    #[test]
    fn typography_planner_styles_artifact_is_dag_only_ready_token() {
        let page = Page::new("page", 1, 1);
        assert!(Artifact::TypographyStyles.ready(&page));
    }

    #[derive(Clone, Copy)]
    enum ConflictEdit {
        Translation,
        Style,
        Transform,
    }

    async fn run_epoch_conflict(edit: ConflictEdit) -> anyhow::Result<(RunOutcome, usize)> {
        let fixture = PipelineFixture::new("中文", "Translated text")?;
        let renderer_calls = Arc::new(AtomicUsize::new(0));
        let text_id = fixture.text;
        install_renderer(&fixture, renderer_calls.clone(), move |scene, page| {
            let node = scene.node(page, text_id).unwrap();
            let text = text(scene, page, text_id);
            match edit {
                ConflictEdit::Translation => {
                    assert_eq!(text.translation.as_deref(), Some("user translation"));
                }
                ConflictEdit::Style => {
                    assert_eq!(text.style.as_ref().unwrap().color, [9, 8, 7, 255]);
                }
                ConflictEdit::Transform => assert_eq!(node.transform.x, 42.0),
            }
            assert!(!text.typography_plan_verified);
            assert!(text.sprite.is_some());
            assert!(text.sprite_transform.is_some());
        });
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let response = valid_response(fixture.text, &fixture.font);
        let sender_started = started.clone();
        let sender_release = release.clone();
        let planner = Arc::new(TypographyPlanner::with_test_sender(
            Arc::new(move |_, _| {
                let response = response.clone();
                let started = sender_started.clone();
                let release = sender_release.clone();
                Box::pin(async move {
                    started.notify_one();
                    release.notified().await;
                    Ok(response)
                })
            }),
            Duration::from_secs(1),
        ));
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let warning_sink: WarningSink = {
            let warnings = warnings.clone();
            Arc::new(move |warning| warnings.lock().unwrap().push(warning))
        };
        let task = tokio::spawn(run(
            fixture.session.clone(),
            fixture.registry.clone(),
            fixture.runtime.clone(),
            true,
            fixture.llm.clone(),
            fixture.renderer.clone(),
            planner,
            PipelineSpec {
                scope: Scope::Pages(vec![fixture.page]),
                steps: vec!["cloud-typography-planner".into(), "koharu-renderer".into()],
                options: PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::HanOnly,
                    default_font: Some(fixture.font.clone()),
                    ..Default::default()
                },
            },
            Arc::new(AtomicBool::new(false)),
            None,
            Some(warning_sink),
        ));
        started.notified().await;
        let patch = match edit {
            ConflictEdit::Translation => NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    translation: Some(Some("user translation".into())),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ConflictEdit::Style => NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    style: Some(Some(TextStyle {
                        color: [9, 8, 7, 255],
                        ..Default::default()
                    })),
                    ..Default::default()
                })),
                ..Default::default()
            },
            ConflictEdit::Transform => NodePatch {
                transform: Some(Transform {
                    x: 42.0,
                    y: 20.0,
                    width: 80.0,
                    height: 40.0,
                    rotation_deg: 0.0,
                }),
                ..Default::default()
            },
        };
        fixture.session.apply(Op::UpdateNode {
            page: fixture.page,
            id: fixture.text,
            patch,
            prev: NodePatch::default(),
        })?;
        release.notify_one();
        let outcome = task.await??;
        assert_eq!(warnings.lock().unwrap().len(), 1);
        Ok((outcome, renderer_calls.load(Ordering::Relaxed)))
    }

    #[tokio::test]
    async fn typography_epoch_conflict_discards_translation_style_and_transform_changes_atomically()
    -> anyhow::Result<()> {
        for edit in [ConflictEdit::Style, ConflictEdit::Transform] {
            let (outcome, renderer_calls) = run_epoch_conflict(edit).await?;
            assert_eq!(outcome.warning_count, 1);
            assert_eq!(renderer_calls, 1);
        }
        Ok(())
    }

    #[tokio::test]
    async fn typography_epoch_conflict_warns_once_and_continues_renderer() -> anyhow::Result<()> {
        let (outcome, renderer_calls) = run_epoch_conflict(ConflictEdit::Translation).await?;
        assert_eq!(outcome.warning_count, 1);
        assert_eq!(renderer_calls, 1);
        Ok(())
    }
}
