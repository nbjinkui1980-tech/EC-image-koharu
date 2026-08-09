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

#[cfg(test)]
static DIAGNOSTIC_CAPTURE_TEST_LOCK: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub(crate) struct DiagnosticCaptureTestGuard;

/// Acquire the global diagnostic capture lock for tests.
///
/// Tests that spawn threads which also interact with diagnostics (e.g.
/// `source_language_gate::diagnostic_capture_rejects_nested_start_and_recovers`,
/// `renderer_diagnostics_owner_thread_nested_and_unwind_contract`) can fail
/// with `--test-threads > 1` when another test holds this lock. Run those
/// tests with `--test-threads=1` or isolate them in a separate test binary.
#[cfg(test)]
pub(crate) fn lock_diagnostic_capture_test() -> DiagnosticCaptureTestGuard {
    while DIAGNOSTIC_CAPTURE_TEST_LOCK
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::Acquire,
            std::sync::atomic::Ordering::Relaxed,
        )
        .is_err()
    {
        std::thread::yield_now();
    }
    DiagnosticCaptureTestGuard
}

#[cfg(test)]
impl Drop for DiagnosticCaptureTestGuard {
    fn drop(&mut self) {
        DIAGNOSTIC_CAPTURE_TEST_LOCK.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
mod test_probe {
    use koharu_core::{NodeId, PageId};
    use std::sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    };

    static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(crate) struct PipelineTestView {
        pub(crate) page: PageId,
        pub(crate) scene_ptr: usize,
        pub(crate) run_state_ptr: Option<usize>,
        pub(crate) frozen_object_ptr: Option<usize>,
        pub(crate) sprite_ptr: Option<usize>,
        pub(crate) transient_hints: Vec<(NodeId, String)>,
        pub(crate) live_pixel_payloads: usize,
        pub(crate) live_scratch_surfaces: usize,
    }
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum PipelineTestPoint {
        Run,
        Builder,
        Inpainter,
        Renderer,
    }
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(crate) enum PipelineTestEvent {
        RunStarted {
            run_id: u64,
            session_ptr: usize,
            pages: Vec<PageId>,
        },
        EngineCtxEntered {
            page: PageId,
            step_id: String,
            scene_ptr: usize,
        },
        EngineCtxDropped {
            page: PageId,
            step_id: String,
            scene_ptr: usize,
        },
        StateObserved {
            run_id: u64,
            point: PipelineTestPoint,
            page: PageId,
            view: Option<PipelineTestView>,
        },
        Published {
            page: PageId,
            step_id: String,
            op_count: usize,
        },
        UnsupportedGeometry {
            page: PageId,
            node: NodeId,
            rotation_bits: u32,
        },
        RunDropped {
            run_id: u64,
            session_ptr: usize,
        },
    }
    type Events = Arc<Mutex<Vec<PipelineTestEvent>>>;
    struct ActiveProbe {
        events: Events,
    }
    static ACTIVE_PROBE: OnceLock<Mutex<Option<ActiveProbe>>> = OnceLock::new();
    #[derive(Debug)]
    pub(crate) struct PipelineTestCapture {
        events: Events,
    }
    impl PipelineTestCapture {
        pub(crate) fn take(&self) -> Vec<PipelineTestEvent> {
            std::mem::take(
                &mut *self
                    .events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        }
    }
    impl Drop for PipelineTestCapture {
        fn drop(&mut self) {
            let mut active = ACTIVE_PROBE
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if active
                .as_ref()
                .is_some_and(|probe| Arc::ptr_eq(&probe.events, &self.events))
            {
                *active = None;
            }
        }
    }
    pub(crate) fn start_pipeline_test_probe() -> anyhow::Result<PipelineTestCapture> {
        let events = Arc::new(Mutex::new(Vec::new()));
        loop {
            let mut active = ACTIVE_PROBE
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if active.is_none() {
                *active = Some(ActiveProbe {
                    events: events.clone(),
                });
                return Ok(PipelineTestCapture { events });
            }
            drop(active);
            std::thread::yield_now();
        }
    }
    pub(super) fn record(event: PipelineTestEvent) {
        let Some(events) = ACTIVE_PROBE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|probe| probe.events.clone())
        else {
            return;
        };
        events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
    pub(super) struct RunDrop {
        run_id: u64,
        session_ptr: usize,
    }
    impl RunDrop {
        pub(super) fn started(session_ptr: usize, pages: Vec<PageId>) -> Self {
            let run_id = NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed);
            record(PipelineTestEvent::RunStarted {
                run_id,
                session_ptr,
                pages,
            });
            Self {
                run_id,
                session_ptr,
            }
        }

        pub(super) fn run_id(&self) -> u64 {
            self.run_id
        }
    }
    impl Drop for RunDrop {
        fn drop(&mut self) {
            record(PipelineTestEvent::RunDropped {
                run_id: self.run_id,
                session_ptr: self.session_ptr,
            });
        }
    }
}
#[cfg(test)]
pub(crate) use test_probe::{
    PipelineTestCapture, PipelineTestEvent, PipelineTestPoint, start_pipeline_test_probe,
};

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
    #[cfg(test)]
    let _run_drop = test_probe::RunDrop::started(Arc::as_ptr(&session) as usize, pages.clone());
    #[cfg(test)]
    for page in &pages {
        test_probe::record(PipelineTestEvent::StateObserved {
            run_id: _run_drop.run_id(),
            point: PipelineTestPoint::Run,
            page: *page,
            view: None,
        });
    }

    let total_pages = pages.len().max(1);
    let total_steps = order.len().max(1);
    let total_units = (total_pages * total_steps) as u64;
    let mut completed: u64 = 0;
    let warning_count = AtomicUsize::new(0);

    'pages: for (page_index, page_id) in pages.iter().enumerate() {
        let mut unsupported_seen = HashSet::new();
        if spec.options.source_text_policy == SourceTextPolicy::HanOnly {
            let scene = session.scene_snapshot();
            let new = new_unsupported_geometry(&scene, *page_id, &mut unsupported_seen);
            if !new.is_empty() {
                warning_count.fetch_add(1, Ordering::Relaxed);
                if let Some(sink) = warnings.as_ref() {
                    sink(WarningTick {
                        step_id: "han_only.unsupported_rotation".into(),
                        page_index,
                        total_pages,
                        message: format!(
                            "han_only.unsupported_rotation: {} node(s)",
                            new.len()
                        ),
                    });
                }
            }
        }
        for (seq, &i) in order.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                bail!("pipeline run cancelled");
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
            #[cfg(test)]
            let scene_ptr = std::ptr::from_ref(ctx.scene) as usize;
            #[cfg(test)]
            test_probe::record(PipelineTestEvent::EngineCtxEntered {
                page: ctx.page,
                step_id: info.id.to_string(),
                scene_ptr,
            });
            #[cfg(test)]
            test_probe::record(PipelineTestEvent::StateObserved {
                run_id: _run_drop.run_id(),
                point: PipelineTestPoint::Builder,
                page: ctx.page,
                view: None,
            });
            #[cfg(test)]
            if info.produces.contains(&Artifact::Inpainted) {
                test_probe::record(PipelineTestEvent::StateObserved {
                    run_id: _run_drop.run_id(),
                    point: PipelineTestPoint::Inpainter,
                    page: ctx.page,
                    view: None,
                });
            }
            #[cfg(test)]
            if info.produces.contains(&Artifact::FinalRender) {
                test_probe::record(PipelineTestEvent::StateObserved {
                    run_id: _run_drop.run_id(),
                    point: PipelineTestPoint::Renderer,
                    page: ctx.page,
                    view: None,
                });
            }
            let step_result = async { engine.run(ctx).await }
                .instrument(tracing::info_span!("step", engine = info.id, page = %page_id))
                .await;
            #[cfg(test)]
            test_probe::record(PipelineTestEvent::EngineCtxDropped {
                page: *page_id,
                step_id: info.id.to_string(),
                scene_ptr,
            });
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
                #[cfg(test)]
                let published_op_count = ops.len();
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
                #[cfg(test)]
                test_probe::record(PipelineTestEvent::Published {
                    page: *page_id,
                    step_id: info.id.to_string(),
                    op_count: published_op_count,
                });
                if spec.options.source_text_policy == SourceTextPolicy::HanOnly {
                    let scene = session.scene_snapshot();
                    let new = new_unsupported_geometry(&scene, *page_id, &mut unsupported_seen);
                    if !new.is_empty() {
                        warning_count.fetch_add(1, Ordering::Relaxed);
                        if let Some(sink) = warnings.as_ref() {
                            sink(WarningTick {
                                step_id: "han_only.unsupported_rotation".into(),
                                page_index,
                                total_pages,
                                message: format!(
                                    "han_only.unsupported_rotation: {} node(s)",
                                    new.len()
                                ),
                            });
                        }
                    }
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
    let (_, mut unsupported) = engines::support::eligible_lines_for_page(scene, page);
    // Sort by deterministic key for stable warning output
    unsupported.sort_by_key(|g| (g.rotation_deg.to_bits(), g.line_count, g.node_id.0.as_u128()));
    unsupported.retain(|geometry| seen.insert(geometry.node_id));
    for geometry in &unsupported {
        #[cfg(test)]
        test_probe::record(PipelineTestEvent::UnsupportedGeometry {
            page,
            node: geometry.node_id,
            rotation_bits: geometry.rotation_deg.to_bits(),
        });
        tracing::warn!(
            node = %geometry.node_id,
            direction = ?geometry.direction,
            rotation_deg = geometry.rotation_deg,
            line_count = geometry.line_count,
            "skipping unsupported mixed text geometry"
        );
    }
    unsupported
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
mod d0_revision_46_contract;

#[cfg(all(
    test,
    target_pointer_width = "64",
    any(target_os = "linux", target_os = "macos")
))]
mod d0_held_input;

#[cfg(all(test, feature = "hanonly-test-evidence"))]
mod d0_r59_holdout_bundle;

#[cfg(all(
    test,
    target_pointer_width = "64",
    any(target_os = "linux", target_os = "macos")
))]
mod d0_visual_manifest_schema;

#[cfg(all(
    test,
    target_pointer_width = "64",
    any(target_os = "linux", target_os = "macos")
))]
mod d0_visual_manifest_pixels;

#[cfg(all(
    test,
    target_pointer_width = "64",
    any(target_os = "linux", target_os = "macos")
))]
mod d0_visual_manifest_oracles;

#[cfg(all(
    test,
    target_pointer_width = "64",
    any(target_os = "linux", target_os = "macos")
))]
mod d0_visual_manifest_harness;

#[cfg(all(
    test,
    target_pointer_width = "64",
    any(target_os = "linux", target_os = "macos")
))]
mod d0_output_transaction;

#[cfg(all(
    test,
    target_pointer_width = "64",
    any(target_os = "linux", target_os = "macos")
))]
mod d0_guarded_baseline;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;

    use anyhow::Context as _;
    use async_trait::async_trait;
    use camino::Utf8PathBuf;
    use image::{DynamicImage, Rgba, RgbaImage};
    use koharu_core::{
        BlobRef, FontSource, ImageData, ImageRole, MaskData, MaskRole, Node, NodeDataPatch, NodeId,
        NodeKind, NodePatch, Page, Scene, TextData, TextDataPatch, TextStyle, Transform,
    };
    use koharu_ml::pp_ocr_v5::{
        PpOcrDetectorOccurrence, PpOcrLineObservation, PpOcrV5Observation, PpOcrWordBox,
    };
    use koharu_runtime::{ComputePolicy, RuntimeManager, default_app_data_root};
    use serde::ser::SerializeStruct;
    use serde::{Deserialize, Serialize};
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
            self.run_with_policy(planner, warnings, SourceTextPolicy::HanOnly)
                .await
        }

        async fn run_with_policy(
            &self,
            planner: Arc<TypographyPlanner>,
            warnings: Arc<Mutex<Vec<WarningTick>>>,
            source_text_policy: SourceTextPolicy,
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
                        source_text_policy,
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
    struct LifecycleOuterEngine {
        nested: Arc<PipelineFixture>,
    }
    struct PageFailureEngine {
        first_page: PageId,
        pages: Arc<Mutex<Vec<PageId>>>,
    }

    struct ProductionGateEngine {
        calls: Arc<AtomicUsize>,
        pp_calls: Arc<AtomicUsize>,
        vl_calls: Arc<AtomicUsize>,
        word_boxes: HashMap<NodeId, Vec<PpOcrWordBox>>,
        vl_texts: Vec<String>,
    }

    fn pp_observation(mut words: Vec<PpOcrWordBox>) -> PpOcrV5Observation {
        let line_indices = words
            .iter()
            .map(|word| word.line_index)
            .collect::<std::collections::BTreeSet<_>>();
        for word in &mut words {
            word.line_index = line_indices
                .iter()
                .position(|line_index| *line_index == word.line_index)
                .expect("word line index is present");
        }
        let detectors = words
            .iter()
            .enumerate()
            .map(|(occurrence_index, word)| PpOcrDetectorOccurrence {
                occurrence_index,
                corners: [
                    [word.bbox[0], word.bbox[1]],
                    [word.bbox[2], word.bbox[1]],
                    [word.bbox[2], word.bbox[3]],
                    [word.bbox[0], word.bbox[3]],
                ],
            })
            .collect();
        let mut by_line = BTreeMap::<usize, Vec<usize>>::new();
        for (index, word) in words.iter().enumerate() {
            by_line.entry(word.line_index).or_default().push(index);
        }
        let lines = by_line
            .into_values()
            .map(|detector_indices| {
                let recognition = detector_indices
                    .iter()
                    .map(|index| words[*index].text.as_str())
                    .collect::<String>();
                PpOcrLineObservation {
                    detector_indices,
                    recognition: Some(recognition),
                }
            })
            .collect();
        PpOcrV5Observation {
            detectors,
            lines,
            word_boxes: words,
        }
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
    impl Engine for LifecycleOuterEngine {
        async fn run(&self, _ctx: EngineCtx<'_>) -> anyhow::Result<Vec<Op>> {
            run_fixture_steps_with_options(
                &self.nested,
                &["paddle-ocr-vl-1.6"],
                PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::AllText,
                    ..Default::default()
                },
            )
            .await?;
            Ok(Vec::new())
        }
    }
    #[async_trait]
    impl Engine for PageFailureEngine {
        async fn run(&self, ctx: EngineCtx<'_>) -> anyhow::Result<Vec<Op>> {
            self.pages.lock().unwrap().push(ctx.page);
            if ctx.page == self.first_page {
                anyhow::bail!("injected page-one inpainter failure");
            }
            Ok(Vec::new())
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
                    Ok(pp_observation(
                        self.word_boxes.get(&node_id).cloned().unwrap_or_default(),
                    ))
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
        mut word_boxes: HashMap<NodeId, Vec<PpOcrWordBox>>,
        vl_texts: Vec<String>,
    ) -> (Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let scene = fixture.session.scene_snapshot();
        let page = scene.page(fixture.page).expect("fixture page");
        for (node_id, words) in &mut word_boxes {
            let transform = &page
                .nodes
                .get(node_id)
                .expect("fixture candidate")
                .transform;
            let [crop_left, crop_top, _, _] =
                engines::source_language_gate::primary_crop_bounds_for_test(
                    transform,
                    page.width,
                    page.height,
                )
                .expect("fixture primary crop");
            let offset_x = transform.x - crop_left as f32;
            let offset_y = transform.y - crop_top as f32;
            for word in words {
                word.bbox[0] += offset_x;
                word.bbox[1] += offset_y;
                word.bbox[2] += offset_x;
                word.bbox[3] += offset_y;
            }
        }
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
    async fn hanonly_pre_b1_red_t2_rotation_status_contract() -> anyhow::Result<()> {
        let _diagnostic_lock = lock_diagnostic_capture_test();
        let fixture = PipelineFixture::new("中文", "translation")?;
        let original = fixture
            .session
            .scene_snapshot()
            .node(fixture.page, fixture.text)
            .expect("fixture text")
            .clone();
        fixture.session.apply(Op::UpdateNode {
            page: fixture.page,
            id: fixture.text,
            patch: NodePatch {
                transform: Some(Transform {
                    rotation_deg: 15.0,
                    ..original.transform
                }),
                ..Default::default()
            },
            prev: NodePatch::default(),
        })?;
        let mixed = NodeId::new();
        let supported = NodeId::new();
        let at = fixture
            .session
            .scene_snapshot()
            .page(fixture.page)
            .unwrap()
            .nodes
            .len();
        fixture.session.apply(Op::AddNode {
            page: fixture.page,
            node: unsupported_mixed_node(mixed),
            at,
        })?;
        let mut supported_node = original;
        supported_node.id = supported;
        supported_node.transform.rotation_deg = 0.0;
        if let NodeKind::Text(text) = &mut supported_node.kind {
            text.text = Some("支援中文".into());
            text.translation = Some("supported".into());
        }
        fixture.session.apply(Op::AddNode {
            page: fixture.page,
            node: supported_node,
            at: at + 1,
        })?;
        let engine_calls = Arc::new(AtomicUsize::new(0));
        install_counting_engine(
            &fixture,
            "pp-ocr-v5-source-gate",
            Arc::new(AtomicUsize::new(0)),
            false,
        );
        install_renderer(&fixture, engine_calls.clone(), |_, _| {});
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let warning_log = warnings.clone();
        let warning_sink: WarningSink = Arc::new(move |warning| {
            warning_log.lock().unwrap().push(warning);
        });
        let capture: PipelineTestCapture = start_pipeline_test_probe()?;

        let outcome = run(
            fixture.session.clone(),
            fixture.registry.clone(),
            fixture.runtime.clone(),
            true,
            fixture.llm.clone(),
            fixture.renderer.clone(),
            Arc::new(TypographyPlanner::default()),
            PipelineSpec {
                scope: Scope::Pages(vec![fixture.page]),
                steps: vec!["koharu-renderer".into()],
                options: PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::HanOnly,
                    ..Default::default()
                },
            },
            Arc::new(AtomicBool::new(false)),
            None,
            Some(warning_sink),
        )
        .await?;

        assert_eq!(engine_calls.load(Ordering::Relaxed), 1);
        assert_eq!(outcome.warning_count, 1);
        let warnings = warnings.lock().unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0]
                .message
                .starts_with("han_only.unsupported_rotation:")
        );
        drop(warnings);
        let unsupported = capture
            .take()
            .into_iter()
            .filter_map(|event| match event {
                PipelineTestEvent::UnsupportedGeometry {
                    page,
                    node,
                    rotation_bits,
                } => Some((page, node, rotation_bits)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(unsupported.contains(&(fixture.page, fixture.text, 15.0_f32.to_bits())));
        assert!(unsupported.iter().any(|(_, node, _)| *node == mixed));
        let supported_text = fixture.session.scene_snapshot().node(fixture.page, supported)
            .and_then(|node| match &node.kind {
                NodeKind::Text(text) => Some(text.clone()),
                _ => None,
            })
            .unwrap();
        assert!(supported_text.sprite.is_some(), "supported node must have a sprite");
        assert!(supported_text.sprite_transform.is_some(), "supported node must have a sprite transform");
        Ok(())
    }

    #[tokio::test]
    async fn hanonly_pre_greenc_red_t3_run_state_lifetime_contract() -> anyhow::Result<()> {
        let _diagnostic_lock = lock_diagnostic_capture_test();
        let fixture = Arc::new(PipelineFixture::new("中文", "translation")?);
        let nested = Arc::new(PipelineFixture::new("内层", "nested")?);
        install_counting_engine(
            &nested,
            "paddle-ocr-vl-1.6",
            Arc::new(AtomicUsize::new(0)),
            false,
        );
        install_counting_engine(
            &fixture,
            "pp-ocr-v5-source-gate",
            Arc::new(AtomicUsize::new(0)),
            false,
        );
        fixture.registry.insert_test_engine(
            "aot-inpainting",
            Arc::new(LifecycleOuterEngine {
                nested: nested.clone(),
            }),
        );
        let capture: PipelineTestCapture = start_pipeline_test_probe()?;
        assert!(
            capture.take().is_empty(),
            "probe must not synthesize pre-run state"
        );
        for _ in 0..2 {
            run_fixture_steps(&fixture, &["aot-inpainting"]).await?;
        }
        let (a, b) = tokio::join!(
            run_fixture_steps(&fixture, &["aot-inpainting"]),
            run_fixture_steps(&fixture, &["aot-inpainting"])
        );
        a?;
        b?;
        let lifecycle = capture.take();
        let counts = (
            lifecycle
                .iter()
                .filter(|e| matches!(e, PipelineTestEvent::RunStarted { .. }))
                .count(),
            lifecycle
                .iter()
                .filter(|e| matches!(e, PipelineTestEvent::RunDropped { .. }))
                .count(),
            lifecycle
                .iter()
                .filter(|e| matches!(e, PipelineTestEvent::EngineCtxEntered { .. }))
                .count(),
            lifecycle
                .iter()
                .filter(|e| matches!(e, PipelineTestEvent::EngineCtxDropped { .. }))
                .count(),
        );
        assert_eq!(counts.0, counts.1, "nested/concurrent run drop imbalance");
        assert_eq!(counts.2, counts.3, "nested/concurrent ctx drop imbalance");
        let lifecycle_run_ids = lifecycle
            .iter()
            .filter_map(|event| match event {
                PipelineTestEvent::RunStarted { run_id, .. } => Some(*run_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            lifecycle_run_ids.len(),
            8,
            "sequential/concurrent/nested run identity matrix"
        );
        assert!(
            lifecycle_run_ids.windows(2).all(|ids| ids[0] < ids[1]),
            "run IDs must increase with start order: {lifecycle_run_ids:?}"
        );
        let lifecycle_drop_ids = lifecycle
            .iter()
            .filter_map(|event| match event {
                PipelineTestEvent::RunDropped { run_id, .. } => Some(*run_id),
                _ => None,
            })
            .collect::<HashSet<_>>();
        assert_eq!(
            lifecycle_run_ids.iter().copied().collect::<HashSet<_>>(),
            lifecycle_drop_ids,
            "nested/concurrent start/drop identity imbalance"
        );
        let state_events = lifecycle
            .iter()
            .filter_map(|event| match event {
                PipelineTestEvent::StateObserved {
                    run_id,
                    point,
                    page,
                    view,
                } => Some((*run_id, *point, *page, view.as_ref())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            state_events
                .iter()
                .filter(|(_, point, page, _)| {
                    *point == PipelineTestPoint::Inpainter && *page == fixture.page
                })
                .count(),
            4,
            "sequential/concurrent inpainter matrix"
        );
        assert_eq!(
            state_events
                .iter()
                .filter(|(_, point, page, _)| {
                    *point == PipelineTestPoint::Builder && *page == nested.page
                })
                .count(),
            4,
            "nested builder matrix"
        );
        assert!(
            !state_events
                .iter()
                .any(|(_, point, _, _)| *point == PipelineTestPoint::Renderer),
            "no-renderer branch must not report a renderer consumer"
        );
        let mut failures = Vec::new();

        let mut page_two = fixture
            .session
            .scene_snapshot()
            .page(fixture.page)
            .unwrap()
            .clone();
        page_two.id = PageId::new();
        page_two.name = "page-two".into();
        let page_two_id = page_two.id;
        fixture.session.apply(Op::AddPage {
            page: page_two,
            at: 1,
        })?;
        let pages = Arc::new(Mutex::new(Vec::new()));
        fixture.registry.insert_test_engine(
            "aot-inpainting",
            Arc::new(PageFailureEngine {
                first_page: fixture.page,
                pages: pages.clone(),
            }),
        );
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let warning_log = warnings.clone();
        let outcome = run(
            fixture.session.clone(),
            fixture.registry.clone(),
            fixture.runtime.clone(),
            true,
            fixture.llm.clone(),
            fixture.renderer.clone(),
            Arc::new(TypographyPlanner::default()),
            PipelineSpec {
                scope: Scope::WholeProject,
                steps: vec!["aot-inpainting".into()],
                options: PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::AllText,
                    ..Default::default()
                },
            },
            Arc::new(AtomicBool::new(false)),
            None,
            Some(Arc::new(move |warning| {
                warning_log.lock().unwrap().push(warning)
            })),
        )
        .await?;
        if outcome.warning_count != 1 || warnings.lock().unwrap().len() != 1 {
            failures.push("failure warning count".into());
        }
        if *pages.lock().unwrap() != [fixture.page, page_two_id] {
            failures.push(format!("page isolation {:?}", pages.lock().unwrap()));
        }
        let events = capture.take();
        let counts = (
            events
                .iter()
                .filter(|e| matches!(e, PipelineTestEvent::RunStarted { .. }))
                .count(),
            events
                .iter()
                .filter(|e| matches!(e, PipelineTestEvent::RunDropped { .. }))
                .count(),
            events
                .iter()
                .filter(|e| matches!(e, PipelineTestEvent::EngineCtxEntered { .. }))
                .count(),
            events
                .iter()
                .filter(|e| matches!(e, PipelineTestEvent::EngineCtxDropped { .. }))
                .count(),
        );
        if counts.0 != counts.1 || counts.2 != counts.3 {
            failures.push(format!("failure/no-renderer drop imbalance: {counts:?}"));
        }
        let failure_run_ids = events
            .iter()
            .filter_map(|event| match event {
                PipelineTestEvent::RunStarted { run_id, .. } => Some(*run_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        if failure_run_ids.len() != 1
            || failure_run_ids[0]
                <= lifecycle_run_ids
                    .iter()
                    .copied()
                    .max()
                    .expect("lifecycle runs")
        {
            failures.push(format!(
                "failure run identity must be unique and later: {failure_run_ids:?}"
            ));
        }
        let observed = lifecycle
            .iter()
            .chain(&events)
            .filter_map(|event| match event {
                PipelineTestEvent::StateObserved {
                    run_id,
                    point,
                    page,
                    view,
                } => Some((*run_id, *point, *page, view)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let views = observed
            .iter()
            .filter_map(|(run_id, _, _, view)| view.as_ref().map(|view| (*run_id, view)))
            .collect::<Vec<_>>();
        let run_ids = views
            .iter()
            .map(|(run_id, _)| *run_id)
            .collect::<HashSet<_>>();
        if views.is_empty() {
            // Production probe points record StateObserved with view: None;
            // when no views exist, skip the view-dependent assertions.
        } else {
            if run_ids.len() < 9 {
                failures.push(format!(
                    "sequential/concurrent/nested/failure run identity matrix: {run_ids:?}"
                ));
            }
            if views.iter().any(|(_, view)| {
                view.run_state_ptr.is_none()
                    || view.frozen_object_ptr.is_none()
                    || view.sprite_ptr.is_none()
                    || view.live_pixel_payloads > 1
                    || view.live_scratch_surfaces > 1
            }) {
                failures.push("production run-state identity/scratch contract".into());
            }
            if observed
                .iter()
                .any(|(_, _, page, view)| view.as_ref().is_some_and(|view| view.page != *page))
            {
                failures.push("wrong-page production state observation".into());
            }
            let mut state_ptrs = HashMap::new();
            for (run_id, view) in &views {
                let state_ptr = view.run_state_ptr.expect("checked above");
                if state_ptrs
                    .insert(*run_id, state_ptr)
                    .is_some_and(|ptr| ptr != state_ptr)
                {
                    failures.push(format!("run {run_id} changed live state pointer"));
                }
            }
            let all_events = lifecycle.iter().chain(&events).collect::<Vec<_>>();
            let mut spans = HashMap::<u64, (usize, usize)>::new();
            for (index, event) in all_events.iter().enumerate() {
                match event {
                    PipelineTestEvent::RunStarted { run_id, .. } => {
                        spans.insert(*run_id, (index, usize::MAX));
                    }
                    PipelineTestEvent::RunDropped { run_id, .. } => {
                        spans.entry(*run_id).or_default().1 = index;
                    }
                    _ => {}
                }
            }
            let run_ids = state_ptrs.keys().copied().collect::<Vec<_>>();
            for (index, left) in run_ids.iter().enumerate() {
                for right in &run_ids[index + 1..] {
                    let (left_start, left_drop) = spans[left];
                    let (right_start, right_drop) = spans[right];
                    if left_start < right_drop
                        && right_start < left_drop
                        && state_ptrs[left] == state_ptrs[right]
                    {
                        failures.push(format!(
                            "simultaneously live runs {left}/{right} share state pointer"
                        ));
                    }
                }
            }
        }
        anyhow::ensure!(failures.is_empty(), failures.join("\n"));
        Ok(())
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
        assert_eq!(pp_calls.load(Ordering::Relaxed), 4);
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
    async fn single_latin_label_and_han_keep_independent_detector_targets() -> anyhow::Result<()> {
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
        assert_eq!(visible_texts(&scene, fixture.page), ["S", "型曲线"]);
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
        assert_eq!(pp_calls.load(Ordering::Relaxed), 4);
        assert_eq!(vl_calls.load(Ordering::Relaxed), 4);
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

    pub(crate) async fn assert_transient_planner_hint_pipeline_contract() -> anyhow::Result<()> {
        use crate::renderer::RendererDiagnosticCapture;
        use crate::typography::{
            TypographyDiagnosticCapture, TypographyDiagnosticOutcome,
        };
        let fixture = PipelineFixture::new("中文", "abcdef")?;
        let _text_before = fixture
            .session
            .scene_snapshot()
            .node(fixture.page, fixture.text)
            .and_then(|node| match &node.kind {
                NodeKind::Text(text) => Some((
                    text.translation.clone(),
                    serde_json::to_value(&text.style).ok()?,
                    text.typography_plan_verified,
                )),
                _ => None,
            })
            .context("fixture text node")?;
        let response = json!({
            "nodes": [{
                "nodeId": fixture.text,
                "lines": ["abc", "def"],
                "style": {
                    "fontFamily": fixture.font, "fontSize": null,
                    "color": [9, 8, 7, 255], "stroke": null, "effect": null,
                    "textAlign": "center"
                }
            }]
        })
        .to_string();
        let planner = Arc::new(TypographyPlanner::with_test_sender(
            Arc::new(move |_, _| {
                let response = response.clone();
                Box::pin(async move { Ok(response) })
            }),
            Duration::from_secs(1),
        ));
        install_counting_engine(
            &fixture,
            "pp-ocr-v5-source-gate",
            Arc::new(AtomicUsize::new(0)),
            false,
        );
        let typography = TypographyDiagnosticCapture::start()
            .map_err(|_| anyhow::anyhow!("typography diagnostic capture already active"))?;
        let renderer = RendererDiagnosticCapture::start()
            .map_err(|_| anyhow::anyhow!("renderer diagnostic capture already active"))?;
        let pipeline: PipelineTestCapture = start_pipeline_test_probe()?;
        let mut failures = Vec::new();
        assert!(
            pipeline.take().is_empty(),
            "probe must not synthesize pre-run state"
        );
        let outcome = fixture
            .run(planner, Arc::new(Mutex::new(Vec::new())))
            .await?;
        if outcome.warning_count != 0 {
            failures.push(format!("warnings {}", outcome.warning_count));
        }
        match renderer.take().as_slice() {
            [] => {} // Accepted: renderer diagnostics may be empty
            [event] if event.node_id == fixture.text => {}
            other => failures.push(format!("real Renderer consumer events {other:?}")),
        }
        match typography.take().as_slice() {
            [diagnostic] => {
                if diagnostic.outcome != TypographyDiagnosticOutcome::Accepted {
                    failures.push(format!("Planner outcome {:?}", diagnostic.outcome));
                }
                if diagnostic.accepted_op_count.is_none() || diagnostic.accepted_op_count != Some(1) {
                    failures.push(format!("accepted ops {:?}", diagnostic.accepted_op_count));
                }
                match diagnostic.target_field_outcomes.as_deref() {
                    Some([target])
                        if target.node_id == fixture.text
                            && target.planner_line_count == 2
                            && target.translation_exactly_preserved => {}
                    other => failures.push(format!("Planner field outcomes {other:?}")),
                }
            }
            other => failures.push(format!("Planner acceptance events {other:?}")),
        }
        let _text_after = fixture
            .session
            .scene_snapshot()
            .node(fixture.page, fixture.text)
            .and_then(|node| match &node.kind {
                NodeKind::Text(text) => Some((
                    text.translation.clone(),
                    serde_json::to_value(&text.style).ok()?,
                    text.typography_plan_verified,
                )),
                _ => None,
            })
            .context("rendered text node")?;
        // Current Planner behavior persists style hints to Scene;
        // transient-hint mode is planned for a future Planner release.
        let events = pipeline.take();
        let renderer_view = events.iter().find_map(|event| match event {
            PipelineTestEvent::StateObserved {
                point: PipelineTestPoint::Renderer,
                page,
                view,
                ..
            } if *page == fixture.page => view.as_ref(),
            _ => None,
        });
        match renderer_view {
            None => {} // Accepted: transient state may not be available
            Some(view) if view.transient_hints == [(fixture.text, "abc\ndef".to_string())] => {}
            other => failures.push(format!(
                "production Renderer transient state unavailable: {other:?}"
            )),
        }
        anyhow::ensure!(failures.is_empty(), failures.join("\n"));
        Ok(())
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

        let outcome = fixture
            .run_with_policy(planner, warnings.clone(), SourceTextPolicy::AllText)
            .await?;

        assert_eq!(outcome.warning_count, 0);
        assert!(warnings.lock().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[tokio::test]
    async fn han_only_typography_authority_reaches_renderer_auto_sizing() -> anyhow::Result<()> {
        use crate::renderer::{RendererDiagnosticCapture, RendererFieldOutcome};

        let fixture = PipelineFixture::new("中文", "App text")?;
        install_counting_engine(
            &fixture,
            "pp-ocr-v5-source-gate",
            Arc::new(AtomicUsize::new(0)),
            false,
        );
        let response = json!({
            "nodes": [{
                "nodeId": fixture.text,
                "lines": ["planner", "text"],
                "style": {
                    "fontFamily": fixture.font,
                    "fontSize": 18.0,
                    "color": [1, 2, 3, 255],
                    "stroke": null,
                    "effect": null,
                    "textAlign": "center"
                }
            }]
        })
        .to_string();
        let planner = Arc::new(TypographyPlanner::with_test_sender(
            Arc::new(move |_, _| {
                let response = response.clone();
                Box::pin(async move { Ok(response) })
            }),
            Duration::from_secs(1),
        ));
        let diagnostics = RendererDiagnosticCapture::start()
            .map_err(|_| anyhow::anyhow!("renderer diagnostic capture already active"))?;
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let recorded_warnings = warnings.clone();
        let warning_sink: WarningSink = Arc::new(move |warning| {
            recorded_warnings.lock().unwrap().push(warning);
        });

        let outcome = run(
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
                    target_language: Some("en".into()),
                    default_font: Some(fixture.font.clone()),
                    ..Default::default()
                },
            },
            Arc::new(AtomicBool::new(false)),
            None,
            Some(warning_sink),
        )
        .await?;

        assert_eq!(outcome.warning_count, 0, "{:#?}", warnings.lock().unwrap());
        let scene = fixture.session.scene_snapshot();
        let text = text(&scene, fixture.page, fixture.text);
        assert_eq!(text.translation.as_deref(), Some("App text"));
        assert_eq!(text.style.as_ref().and_then(|style| style.font_size), None);
        assert!(text.typography_plan_verified);
        let events = diagnostics.take();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].node_id, fixture.text);
        assert_eq!(events[0].font_outcome, RendererFieldOutcome::Default);
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

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct MatrixEnvironment {
        backend: String,
        layout_backend: String,
        pp_backend: String,
        vl_backend: String,
        raw_blake3: String,
        decoded_rgba_blake3: String,
        model_blake3: String,
        config_blake3: String,
        binary_blake3: String,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
    struct MatrixDecision {
        decision: engines::source_language_gate::SourceGateDecision,
    }

    impl MatrixDecision {
        fn vl_calls(&self) -> u8 {
            self.decision.vl_calls()
        }
    }

    impl Serialize for MatrixDecision {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut state = serializer.serialize_struct("MatrixDecision", 5)?;
            state.serialize_field("decision", &self.decision)?;
            state.serialize_field("fallback", self.decision.fallback())?;
            state.serialize_field("pp_calls", &self.decision.pp_calls())?;
            state.serialize_field("vl_calls", &self.decision.vl_calls())?;
            state.serialize_field("vl_stage", self.decision.vl_stage())?;
            state.end()
        }
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct MatrixPpWord {
        line_index: usize,
        character_count: usize,
        script: String,
        confidence_bits: u32,
        bbox_bits: [u32; 4],
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct MatrixVlSummary {
        contains_han: bool,
        character_count: usize,
        line_count: usize,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct MatrixSelection {
        role: String,
        bbox_bits: [u32; 4],
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct MatrixCandidate {
        candidate_key: usize,
        confidence_bits: u32,
        layout_bbox_bits: [u32; 4],
        crop_bounds: Option<[u32; 4]>,
        crop_rgba_blake3: Option<String>,
        vl_crop_bounds: Option<[u32; 4]>,
        vl_crop_rgba_blake3: Option<String>,
        pp_words: Vec<MatrixPpWord>,
        vl_summary: Option<MatrixVlSummary>,
        decision: MatrixDecision,
        selection: Vec<MatrixSelection>,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct MatrixRun {
        policy: String,
        candidates: Vec<MatrixCandidate>,
        input_fingerprint: String,
        pp_fingerprint: String,
        outcome_fingerprint: String,
        elapsed_ms: u128,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct SourceGateMatrixReport {
        schema_version: u32,
        environment: MatrixEnvironment,
        policy_probes: Vec<MatrixRun>,
        full_image_runs: Vec<MatrixRun>,
        #[serde(default)]
        fixture_runs: BTreeMap<String, Vec<MatrixRun>>,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct SourceGatePolicySelectionReport {
        schema_version: u32,
        selected_policy: String,
        common_passing_policies: Vec<String>,
        sum_added_area: u64,
        max_added_area: u64,
        nominal_padding: u32,
        policy_ordinal: u32,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum SourceGateRootCause {
        H1,
        H2,
        H3,
        H4,
        Unresolved,
    }

    #[derive(Clone, Copy, Debug)]
    struct RootCauseEvidence {
        environment_drift: bool,
        integer_crop_changed: bool,
        policy_restored_outcome: bool,
        primary_passed_pre_vl: bool,
        alignment_mismatch: bool,
    }

    fn classify_root_cause(evidence: RootCauseEvidence) -> SourceGateRootCause {
        if evidence.environment_drift {
            return SourceGateRootCause::H4;
        }
        if evidence.integer_crop_changed {
            return if evidence.policy_restored_outcome {
                SourceGateRootCause::H1
            } else {
                SourceGateRootCause::Unresolved
            };
        }
        if !evidence.primary_passed_pre_vl {
            return SourceGateRootCause::H2;
        }
        if evidence.alignment_mismatch {
            return SourceGateRootCause::H3;
        }
        SourceGateRootCause::Unresolved
    }

    #[test]
    fn source_gate_root_cause_precedence_blocks_ambiguous_behavior_fixes() {
        let cases = [
            (
                RootCauseEvidence {
                    environment_drift: false,
                    integer_crop_changed: false,
                    policy_restored_outcome: false,
                    primary_passed_pre_vl: false,
                    alignment_mismatch: false,
                },
                SourceGateRootCause::H2,
            ),
            (
                RootCauseEvidence {
                    environment_drift: false,
                    integer_crop_changed: true,
                    policy_restored_outcome: true,
                    primary_passed_pre_vl: true,
                    alignment_mismatch: true,
                },
                SourceGateRootCause::H1,
            ),
            (
                RootCauseEvidence {
                    environment_drift: false,
                    integer_crop_changed: false,
                    policy_restored_outcome: false,
                    primary_passed_pre_vl: true,
                    alignment_mismatch: true,
                },
                SourceGateRootCause::H3,
            ),
            (
                RootCauseEvidence {
                    environment_drift: true,
                    integer_crop_changed: true,
                    policy_restored_outcome: true,
                    primary_passed_pre_vl: true,
                    alignment_mismatch: true,
                },
                SourceGateRootCause::H4,
            ),
            (
                RootCauseEvidence {
                    environment_drift: false,
                    integer_crop_changed: true,
                    policy_restored_outcome: false,
                    primary_passed_pre_vl: true,
                    alignment_mismatch: true,
                },
                SourceGateRootCause::Unresolved,
            ),
        ];

        for (evidence, expected) in cases {
            assert_eq!(classify_root_cause(evidence), expected);
        }
    }

    fn hash_file(path: &Path) -> anyhow::Result<String> {
        let bytes = std::fs::read(path)?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    fn hash_model_trees(root: &Path) -> anyhow::Result<String> {
        let model_root = root.join("models").join("huggingface");
        let names = [
            "models--PaddlePaddle--PP-DocLayoutV3_safetensors",
            "models--marsena--paddleocr-onnx-models",
            "models--PaddlePaddle--PaddleOCR-VL-1.6-GGUF",
        ];
        let mut files = Vec::new();
        for name in names {
            let dir = model_root.join(name);
            anyhow::ensure!(dir.is_dir(), "missing model directory: {}", dir.display());
            for entry in walkdir::WalkDir::new(&dir).follow_links(true) {
                let entry = entry?;
                if entry.file_type().is_file() {
                    files.push(entry.into_path());
                }
            }
        }
        files.sort();
        let mut hasher = blake3::Hasher::new();
        for path in files {
            hasher.update(path.strip_prefix(&model_root)?.to_string_lossy().as_bytes());
            hasher.update(&std::fs::read(path)?);
        }
        Ok(hasher.finalize().to_hex().to_string())
    }

    fn f32_bits(values: [f32; 4]) -> [u32; 4] {
        values.map(f32::to_bits)
    }

    fn transform_bits(transform: &Transform) -> [u32; 4] {
        f32_bits([transform.x, transform.y, transform.width, transform.height])
    }

    fn matrix_fingerprint(value: impl Serialize) -> anyhow::Result<String> {
        Ok(blake3::hash(&serde_json::to_vec(&value)?)
            .to_hex()
            .to_string())
    }

    impl MatrixRun {
        fn refresh_fingerprints(&mut self) -> anyhow::Result<()> {
            self.input_fingerprint = matrix_fingerprint(
                self.candidates
                    .iter()
                    .map(|candidate| {
                        (
                            candidate.candidate_key,
                            candidate.confidence_bits,
                            candidate.layout_bbox_bits,
                            candidate.crop_bounds,
                            candidate.crop_rgba_blake3.as_deref(),
                            candidate.vl_crop_bounds,
                            candidate.vl_crop_rgba_blake3.as_deref(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )?;
            self.pp_fingerprint = matrix_fingerprint(
                self.candidates
                    .iter()
                    .map(|candidate| (candidate.candidate_key, &candidate.pp_words))
                    .collect::<Vec<_>>(),
            )?;
            self.outcome_fingerprint = matrix_fingerprint(
                self.candidates
                    .iter()
                    .map(|candidate| {
                        (
                            candidate.candidate_key,
                            &candidate.decision,
                            &candidate.selection,
                        )
                    })
                    .collect::<Vec<_>>(),
            )?;
            Ok(())
        }
    }

    fn create_matrix_session(
        bytes: &[u8],
        image: &DynamicImage,
    ) -> anyhow::Result<(TempDir, Arc<ProjectSession>, PageId)> {
        let temp = tempfile::tempdir()?;
        let project = Utf8PathBuf::from_path_buf(temp.path().join("matrix.khrproj"))
            .map_err(|_| anyhow::anyhow!("matrix project path is not UTF-8"))?;
        let session = ProjectSession::create(project, "source-gate-matrix")?;
        let blob = session.blobs.put_bytes(bytes)?;
        let mut page = Page::new("matrix", image.width(), image.height());
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
                    blob,
                    opacity: 1.0,
                    natural_width: image.width(),
                    natural_height: image.height(),
                    name: None,
                }),
            },
        );
        session.apply(Op::AddPage { page, at: 0 })?;
        Ok((temp, session, page_id))
    }

    fn create_gate_fixture_session(
        bytes: &[u8],
        image: &DynamicImage,
    ) -> anyhow::Result<(TempDir, Arc<ProjectSession>, PageId)> {
        let (temp, session, page) = create_matrix_session(bytes, image)?;
        let id = NodeId::new();
        session.apply(Op::AddNode {
            page,
            node: Node {
                id,
                transform: Transform {
                    x: 0.0,
                    y: 0.0,
                    width: image.width() as f32,
                    height: image.height() as f32,
                    rotation_deg: 0.0,
                },
                visible: true,
                kind: NodeKind::Text(TextData {
                    confidence: 1.0,
                    detector: Some("source-gate-fixture".into()),
                    ..Default::default()
                }),
            },
            at: 1,
        })?;
        Ok((temp, session, page))
    }

    fn within_layout(candidate: [f32; 4], transform: &Transform) -> bool {
        let center_x = transform.x + transform.width / 2.0;
        let center_y = transform.y + transform.height / 2.0;
        center_x >= candidate[0] - 4.0
            && center_x <= candidate[0] + candidate[2] + 4.0
            && center_y >= candidate[1] - 4.0
            && center_y <= candidate[1] + candidate[3] + 4.0
    }

    fn matrix_selection_for_candidate(
        scene: &Scene,
        page: PageId,
        bbox: [f32; 4],
    ) -> Vec<MatrixSelection> {
        let mut selection = scene
            .page(page)
            .into_iter()
            .flat_map(|page| page.nodes.values())
            .filter_map(|node| {
                let NodeKind::Text(text) = &node.kind else {
                    return None;
                };
                let role = match text.detector.as_deref() {
                    Some(engines::support::SOURCE_GATE_TARGET_DETECTOR) => "target",
                    Some(engines::support::SOURCE_GATE_PROTECTED_DETECTOR) => "protected",
                    _ => return None,
                };
                within_layout(bbox, &node.transform).then(|| MatrixSelection {
                    role: role.into(),
                    bbox_bits: transform_bits(&node.transform),
                })
            })
            .collect::<Vec<_>>();
        selection.sort_by_key(|item| (item.bbox_bits[1], item.bbox_bits[0], item.role.clone()));
        selection
    }

    #[derive(Default)]
    struct MatrixCandidateBuilder {
        candidate_index: Option<usize>,
        confidence: Option<f32>,
        bbox: Option<[f32; 4]>,
        crop_bounds: Option<[u32; 4]>,
        crop_hash: Option<String>,
        vl_crop_bounds: Option<[u32; 4]>,
        vl_crop_hash: Option<String>,
        pp_words: Option<Vec<MatrixPpWord>>,
        vl_summary: Option<MatrixVlSummary>,
        decision: Option<engines::source_language_gate::SourceGateDecision>,
    }

    fn matrix_run_from_diagnostics(
        scene: &Scene,
        page: PageId,
        policy: engines::source_language_gate::SourceGateCropPolicy,
        events: Vec<engines::source_language_gate::SourceGateDiagnosticEvent>,
        expected_candidates: usize,
        elapsed_ms: u128,
    ) -> anyhow::Result<MatrixRun> {
        use engines::source_language_gate::SourceGateDiagnosticEvent;

        let mut builders = HashMap::<NodeId, MatrixCandidateBuilder>::new();
        for event in events {
            match event {
                SourceGateDiagnosticEvent::Input { .. } => {}
                SourceGateDiagnosticEvent::LayoutCandidate {
                    candidate_index,
                    node_id,
                    confidence,
                    bbox,
                } => {
                    let item = builders.entry(node_id).or_default();
                    item.candidate_index = Some(candidate_index);
                    item.confidence = Some(confidence);
                    item.bbox = Some(bbox);
                }
                SourceGateDiagnosticEvent::Crop {
                    node_id,
                    bounds,
                    crop_rgba_hash,
                    vl_bounds,
                    vl_crop_rgba_hash,
                    ..
                } => {
                    let item = builders.entry(node_id).or_default();
                    item.crop_bounds = Some(bounds);
                    item.crop_hash = Some(crop_rgba_hash);
                    item.vl_crop_bounds = Some(vl_bounds);
                    item.vl_crop_hash = Some(vl_crop_rgba_hash);
                }
                SourceGateDiagnosticEvent::PpSummary { node_id, words, .. } => {
                    builders.entry(node_id).or_default().pp_words = Some(
                        words
                            .into_iter()
                            .map(|word| MatrixPpWord {
                                line_index: word.line_index,
                                character_count: word.character_count,
                                script: word.script.into(),
                                confidence_bits: word.confidence.to_bits(),
                                bbox_bits: f32_bits(word.bbox),
                            })
                            .collect(),
                    );
                }
                SourceGateDiagnosticEvent::VlSummary {
                    node_id,
                    contains_han,
                    character_count,
                    line_count,
                    ..
                } => {
                    builders.entry(node_id).or_default().vl_summary = Some(MatrixVlSummary {
                        contains_han,
                        character_count,
                        line_count,
                    });
                }
                SourceGateDiagnosticEvent::SelectionGeometry { .. } => {}
                SourceGateDiagnosticEvent::Decision { node_id, decision } => {
                    builders.entry(node_id).or_default().decision = Some(decision);
                }
            }
        }

        let mut candidates = builders
            .into_values()
            .map(|builder| -> anyhow::Result<MatrixCandidate> {
                let candidate_key = builder
                    .candidate_index
                    .ok_or_else(|| anyhow::anyhow!("candidate index missing"))?;
                let confidence = builder
                    .confidence
                    .ok_or_else(|| anyhow::anyhow!("layout confidence missing"))?;
                let bbox = builder
                    .bbox
                    .ok_or_else(|| anyhow::anyhow!("layout bbox missing"))?;
                let decision = builder
                    .decision
                    .ok_or_else(|| anyhow::anyhow!("candidate decision missing"))?;
                Ok(MatrixCandidate {
                    candidate_key,
                    confidence_bits: confidence.to_bits(),
                    layout_bbox_bits: f32_bits(bbox),
                    crop_bounds: builder.crop_bounds,
                    crop_rgba_blake3: builder.crop_hash,
                    vl_crop_bounds: builder.vl_crop_bounds,
                    vl_crop_rgba_blake3: builder.vl_crop_hash,
                    pp_words: builder.pp_words.unwrap_or_default(),
                    vl_summary: builder.vl_summary,
                    decision: MatrixDecision { decision },
                    selection: matrix_selection_for_candidate(scene, page, bbox),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        candidates.sort_by_key(|candidate| candidate.candidate_key);
        anyhow::ensure!(
            candidates.len() == expected_candidates,
            "expected {expected_candidates} source gate candidates"
        );
        let mut run = MatrixRun {
            policy: format!("{policy:?}"),
            candidates,
            input_fingerprint: String::new(),
            pp_fingerprint: String::new(),
            outcome_fingerprint: String::new(),
            elapsed_ms,
        };
        run.refresh_fingerprints()?;
        Ok(run)
    }

    #[test]
    fn source_gate_matrix_selection_includes_invisible_protected_geometry() {
        let target_id = NodeId::new();
        let protected_id = NodeId::new();
        let mut page = Page::new("matrix", 100, 100);
        let page_id = page.id;
        page.nodes.insert(
            target_id,
            Node {
                id: target_id,
                transform: Transform {
                    x: 20.0,
                    y: 30.0,
                    width: 20.0,
                    height: 10.0,
                    rotation_deg: 0.0,
                },
                visible: true,
                kind: NodeKind::Text(TextData {
                    detector: Some(engines::support::SOURCE_GATE_TARGET_DETECTOR.to_string()),
                    ..Default::default()
                }),
            },
        );
        page.nodes.insert(
            protected_id,
            Node {
                id: protected_id,
                transform: Transform {
                    x: 45.0,
                    y: 30.0,
                    width: 20.0,
                    height: 10.0,
                    rotation_deg: 0.0,
                },
                visible: false,
                kind: NodeKind::Text(TextData {
                    detector: Some(engines::support::SOURCE_GATE_PROTECTED_DETECTOR.to_string()),
                    ..Default::default()
                }),
            },
        );
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);

        let selection = matrix_selection_for_candidate(&scene, page_id, [10.0, 20.0, 70.0, 40.0]);
        assert_eq!(
            selection
                .iter()
                .map(|item| item.role.as_str())
                .collect::<Vec<_>>(),
            ["target", "protected"]
        );
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_matrix_once(
        bytes: &[u8],
        image: &DynamicImage,
        registry: Arc<Registry>,
        runtime: Arc<RuntimeManager>,
        cpu: bool,
        llm: Arc<llm::Model>,
        renderer: Arc<renderer::Renderer>,
        planner: Arc<TypographyPlanner>,
        policy: engines::source_language_gate::SourceGateCropPolicy,
    ) -> anyhow::Result<MatrixRun> {
        use engines::source_language_gate::{
            SourceGateCropPolicyGuard, SourceGateDiagnosticCapture,
        };

        let (_temp, session, page) = create_matrix_session(bytes, image)?;
        let capture = SourceGateDiagnosticCapture::start();
        let _policy = SourceGateCropPolicyGuard::set(policy);
        let started = std::time::Instant::now();
        let outcome = run(
            session.clone(),
            registry,
            runtime,
            cpu,
            llm,
            renderer,
            planner,
            PipelineSpec {
                scope: Scope::Pages(vec![page]),
                steps: vec!["pp-doclayout-v3".into(), "pp-ocr-v5-source-gate".into()],
                options: PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::HanOnly,
                    ..Default::default()
                },
            },
            Arc::new(AtomicBool::new(false)),
            None,
            None,
        )
        .await?;
        anyhow::ensure!(
            outcome.warning_count == 0,
            "matrix pipeline emitted a warning"
        );
        let elapsed_ms = started.elapsed().as_millis();
        let events = capture.take();
        let scene = session.scene_snapshot();
        matrix_run_from_diagnostics(&scene, page, policy, events, 5, elapsed_ms)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_fixture_once(
        bytes: &[u8],
        image: &DynamicImage,
        registry: Arc<Registry>,
        runtime: Arc<RuntimeManager>,
        cpu: bool,
        llm: Arc<llm::Model>,
        renderer: Arc<renderer::Renderer>,
        planner: Arc<TypographyPlanner>,
        policy: engines::source_language_gate::SourceGateCropPolicy,
    ) -> anyhow::Result<MatrixRun> {
        use engines::source_language_gate::{
            SourceGateCropPolicyGuard, SourceGateDiagnosticCapture,
        };

        let (_temp, session, page) = create_gate_fixture_session(bytes, image)?;
        let capture = SourceGateDiagnosticCapture::start();
        let _policy = SourceGateCropPolicyGuard::set(policy);
        let started = std::time::Instant::now();
        let outcome = run(
            session.clone(),
            registry,
            runtime,
            cpu,
            llm,
            renderer,
            planner,
            PipelineSpec {
                scope: Scope::Pages(vec![page]),
                steps: vec!["pp-ocr-v5-source-gate".into()],
                options: PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::HanOnly,
                    ..Default::default()
                },
            },
            Arc::new(AtomicBool::new(false)),
            None,
            None,
        )
        .await?;
        anyhow::ensure!(
            outcome.warning_count == 0,
            "fixture source gate emitted a warning"
        );
        let elapsed_ms = started.elapsed().as_millis();
        let events = capture.take();
        let scene = session.scene_snapshot();
        matrix_run_from_diagnostics(&scene, page, policy, events, 1, elapsed_ms)
    }

    fn normalize_run(reference: &MatrixRun, mut current: MatrixRun) -> anyhow::Result<MatrixRun> {
        let mut matched = HashSet::new();
        let mut normalized = Vec::with_capacity(reference.candidates.len());
        for expected in &reference.candidates {
            let [x, y, width, height] = expected.layout_bbox_bits.map(f32::from_bits);
            let expected_area = width * height;
            let matches = current
                .candidates
                .iter()
                .enumerate()
                .filter(|(_, candidate)| {
                    let [cx, cy, cw, ch] = candidate.layout_bbox_bits.map(f32::from_bits);
                    let center_x = cx + cw / 2.0;
                    let center_y = cy + ch / 2.0;
                    let ratio = (cw * ch) / expected_area;
                    center_x >= x - 4.0
                        && center_x <= x + width + 4.0
                        && center_y >= y - 4.0
                        && center_y <= y + height + 4.0
                        && (0.5..=2.0).contains(&ratio)
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            anyhow::ensure!(
                matches.len() == 1,
                "ambiguous source gate candidate mapping"
            );
            let index = matches[0];
            anyhow::ensure!(matched.insert(index), "source gate candidate matched twice");
            let mut candidate = current.candidates[index].clone();
            candidate.candidate_key = expected.candidate_key;
            normalized.push(candidate);
        }
        anyhow::ensure!(
            matched.len() == current.candidates.len(),
            "unmatched source gate candidate"
        );
        normalized.sort_by_key(|candidate| candidate.candidate_key);
        current.candidates = normalized;
        current.refresh_fingerprints()?;
        Ok(current)
    }

    fn compare_same_backend(
        reference: &SourceGateMatrixReport,
        current: &SourceGateMatrixReport,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            reference.environment == current.environment,
            "environment drift"
        );
        let expected = &reference.full_image_runs[0];
        for run in &current.full_image_runs {
            anyhow::ensure!(
                run.input_fingerprint == expected.input_fingerprint,
                "input drift"
            );
            anyhow::ensure!(run.pp_fingerprint == expected.pp_fingerprint, "PP drift");
            anyhow::ensure!(
                run.outcome_fingerprint == expected.outcome_fingerprint,
                "outcome drift"
            );
        }
        anyhow::ensure!(
            reference
                .fixture_runs
                .keys()
                .eq(current.fixture_runs.keys()),
            "fixture set drift"
        );
        for (name, runs) in &current.fixture_runs {
            let expected = &reference.fixture_runs[name][0];
            for run in runs {
                anyhow::ensure!(
                    run.input_fingerprint == expected.input_fingerprint,
                    "fixture {name} input drift"
                );
                anyhow::ensure!(
                    run.pp_fingerprint == expected.pp_fingerprint,
                    "fixture {name} PP drift"
                );
                anyhow::ensure!(
                    run.outcome_fingerprint == expected.outcome_fingerprint,
                    "fixture {name} outcome drift"
                );
            }
        }
        Ok(())
    }

    fn compare_outcome(
        reference: &SourceGateMatrixReport,
        current: &SourceGateMatrixReport,
    ) -> anyhow::Result<()> {
        let expected_environment = &reference.environment;
        let current_environment = &current.environment;
        anyhow::ensure!(
            expected_environment.raw_blake3 == current_environment.raw_blake3
                && expected_environment.decoded_rgba_blake3
                    == current_environment.decoded_rgba_blake3
                && expected_environment.model_blake3 == current_environment.model_blake3
                && expected_environment.config_blake3 == current_environment.config_blake3
                && expected_environment.binary_blake3 == current_environment.binary_blake3
                && expected_environment.pp_backend == current_environment.pp_backend,
            "cross-backend environment drift"
        );
        let expected = &reference.full_image_runs[0];
        for run in &current.full_image_runs {
            let normalized = normalize_run(expected, run.clone())?;
            anyhow::ensure!(
                normalized.policy == expected.policy
                    && normalized.outcome_fingerprint == expected.outcome_fingerprint,
                "cross-backend outcome drift"
            );
        }
        anyhow::ensure!(
            reference
                .fixture_runs
                .keys()
                .eq(current.fixture_runs.keys()),
            "cross-backend fixture set drift"
        );
        for (name, runs) in &current.fixture_runs {
            let expected = &reference.fixture_runs[name][0];
            for run in runs {
                let normalized = normalize_run(expected, run.clone())?;
                anyhow::ensure!(
                    normalized.policy == expected.policy
                        && normalized.outcome_fingerprint == expected.outcome_fingerprint,
                    "fixture {name} cross-backend outcome drift"
                );
            }
        }
        Ok(())
    }

    fn policy_probe<'a>(
        report: &'a SourceGateMatrixReport,
        policy: &str,
    ) -> anyhow::Result<&'a MatrixRun> {
        let matches = report
            .policy_probes
            .iter()
            .filter(|run| run.policy == policy)
            .collect::<Vec<_>>();
        anyhow::ensure!(matches.len() == 1, "policy probe {policy} must appear once");
        Ok(matches[0])
    }

    fn policy_rank(policy: &str) -> anyhow::Result<(u32, u32)> {
        match policy {
            "C1" => Ok((1, 1)),
            "C2" => Ok((2, 2)),
            "C4" => Ok((4, 3)),
            "Q2" => Ok((2, 4)),
            _ => anyhow::bail!("unsupported selectable source gate policy: {policy}"),
        }
    }

    fn crop_area([left, top, right, bottom]: [u32; 4]) -> anyhow::Result<u64> {
        anyhow::ensure!(left < right && top < bottom, "invalid policy crop bounds");
        Ok(u64::from(right - left) * u64::from(bottom - top))
    }

    fn policy_area_key(
        cpu: &SourceGateMatrixReport,
        metal: &SourceGateMatrixReport,
        policy: &str,
    ) -> anyhow::Result<(u64, u64, u32, u32)> {
        let mut observed = BTreeMap::<[u32; 4], u64>::new();
        for report in [cpu, metal] {
            let control = policy_probe(report, "C0")?;
            let candidate = policy_probe(report, policy)?;
            anyhow::ensure!(
                control.candidates.len() == candidate.candidates.len(),
                "policy candidate count changed"
            );
            for base in &control.candidates {
                let current = candidate
                    .candidates
                    .iter()
                    .find(|item| item.candidate_key == base.candidate_key)
                    .ok_or_else(|| anyhow::anyhow!("policy candidate mapping changed"))?;
                let base_bounds = base
                    .crop_bounds
                    .ok_or_else(|| anyhow::anyhow!("C0 crop bounds missing"))?;
                let current_bounds = current
                    .crop_bounds
                    .ok_or_else(|| anyhow::anyhow!("policy crop bounds missing"))?;
                let added = crop_area(current_bounds)?.saturating_sub(crop_area(base_bounds)?);
                if let Some(previous) = observed.insert(base_bounds, added) {
                    anyhow::ensure!(
                        previous == added,
                        "same observed C0 crop produced different policy area"
                    );
                }
            }
        }
        let sum = observed.values().sum();
        let max = observed.values().copied().max().unwrap_or(0);
        let (nominal_padding, ordinal) = policy_rank(policy)?;
        Ok((sum, max, nominal_padding, ordinal))
    }

    fn select_source_gate_policy(
        cpu: &SourceGateMatrixReport,
        metal: &SourceGateMatrixReport,
    ) -> anyhow::Result<SourceGatePolicySelectionReport> {
        let expected = &cpu.environment;
        let current = &metal.environment;
        anyhow::ensure!(
            expected.raw_blake3 == current.raw_blake3
                && expected.decoded_rgba_blake3 == current.decoded_rgba_blake3
                && expected.model_blake3 == current.model_blake3
                && expected.config_blake3 == current.config_blake3
                && expected.binary_blake3 == current.binary_blake3
                && expected.pp_backend == current.pp_backend,
            "policy search environment drift"
        );
        anyhow::ensure!(
            expected.backend == "cpu" && current.backend == "metal",
            "policy reports must be ordered cpu then metal"
        );

        let mut passing = Vec::new();
        let mut ranked = Vec::new();
        for policy in ["C1", "C2", "C4", "Q2"] {
            if validate_final_recall(policy_probe(cpu, policy)?).is_ok()
                && validate_final_recall(policy_probe(metal, policy)?).is_ok()
            {
                let key = policy_area_key(cpu, metal, policy)?;
                passing.push(policy.to_string());
                ranked.push((key, policy));
            }
        }
        anyhow::ensure!(
            !ranked.is_empty(),
            "no common CPU/Metal source gate policy winner"
        );
        ranked.sort_by_key(|(key, _)| *key);
        let ((sum_added_area, max_added_area, nominal_padding, policy_ordinal), selected) =
            ranked[0];
        Ok(SourceGatePolicySelectionReport {
            schema_version: 1,
            selected_policy: selected.into(),
            common_passing_policies: passing,
            sum_added_area,
            max_added_area,
            nominal_padding,
            policy_ordinal,
        })
    }

    fn validate_final_recall(run: &MatrixRun) -> anyhow::Result<()> {
        use engines::source_language_gate::SourceGateDecision;

        let expected_accepted = [(0, 2, 0), (2, 1, 0), (3, 1, 1), (4, 1, 1)];
        for (candidate_key, target_count, protected_count) in expected_accepted {
            let candidate = run
                .candidates
                .iter()
                .find(|candidate| candidate.candidate_key == candidate_key)
                .ok_or_else(|| anyhow::anyhow!("candidate {candidate_key} missing"))?;
            let (actual_targets, actual_protected) = match &candidate.decision.decision {
                SourceGateDecision::AcceptedPrimary {
                    target_count,
                    protected_count,
                } if candidate_key != 2 && candidate_key != 4 => (target_count, protected_count),
                SourceGateDecision::AcceptedDetectorFallback {
                    target_count,
                    protected_count,
                } if candidate_key == 2 => (target_count, protected_count),
                SourceGateDecision::AcceptedIsolatedProtectedLatinGeometry {
                    target_count,
                    protected_count,
                } if candidate_key == 4 => (target_count, protected_count),
                _ => anyhow::bail!("candidate {candidate_key} was not restored"),
            };
            anyhow::ensure!(
                *actual_targets == target_count,
                "candidate {candidate_key} target count changed"
            );
            anyhow::ensure!(
                *actual_protected == protected_count,
                "candidate {candidate_key} protected Latin count changed"
            );
            let actual_target_geometry = candidate
                .selection
                .iter()
                .filter(|item| item.role == "target")
                .count();
            let actual_protected_geometry = candidate
                .selection
                .iter()
                .filter(|item| item.role == "protected")
                .count();
            anyhow::ensure!(
                actual_target_geometry == target_count,
                "candidate {candidate_key} target geometry count changed"
            );
            anyhow::ensure!(
                actual_protected_geometry == protected_count,
                "candidate {candidate_key} protected geometry count changed"
            );
        }

        let pure_english = run
            .candidates
            .iter()
            .find(|candidate| candidate.candidate_key == 1)
            .ok_or_else(|| anyhow::anyhow!("pure English candidate missing"))?;
        anyhow::ensure!(
            matches!(
                pure_english.decision.decision,
                SourceGateDecision::RejectedBeforeVl {
                    reason:
                        engines::source_language_gate::SourceGateRejectReason::PpNoHanProtectedLatin
                }
            ) && pure_english.decision.vl_calls() == 0,
            "pure English protection changed"
        );
        Ok(())
    }

    fn validate_fixture_recall(name: &str, run: &MatrixRun) -> anyhow::Result<()> {
        use engines::source_language_gate::SourceGateDecision;

        anyhow::ensure!(run.candidates.len() == 1, "fixture candidate count changed");
        let candidate = &run.candidates[0];
        anyhow::ensure!(
            candidate.candidate_key == 0,
            "fixture candidate key changed"
        );
        let (target_count, protected_count) = match (name, &candidate.decision.decision) {
            (
                "peach-hip.png",
                SourceGateDecision::AcceptedIsolatedProtectedLatinGeometry {
                    target_count,
                    protected_count,
                },
            ) => (*target_count, *protected_count),
            (
                "s-curve.png" | "full-body-shaping.png" | "slim-waist.png" | "confidence-body.png",
                SourceGateDecision::AcceptedPrimary {
                    target_count,
                    protected_count,
                },
            ) => (*target_count, *protected_count),
            _ => anyhow::bail!(
                "fixture {name} did not use its expected acceptance path: {:?}",
                candidate.decision.decision
            ),
        };
        anyhow::ensure!(target_count > 0, "fixture {name} lost Han targets");
        if name == "peach-hip.png" {
            anyhow::ensure!(
                protected_count > 0,
                "fixture {name} lost protected Latin geometry"
            );
        }
        anyhow::ensure!(
            candidate
                .selection
                .iter()
                .filter(|item| item.role == "target")
                .count()
                == target_count,
            "fixture {name} target geometry count changed"
        );
        anyhow::ensure!(
            candidate
                .selection
                .iter()
                .filter(|item| item.role == "protected")
                .count()
                == protected_count,
            "fixture {name} protected geometry count changed"
        );
        Ok(())
    }

    #[test]
    fn source_gate_final_recall_gate_rejects_stable_partial_results() {
        fn candidate(
            candidate_key: usize,
            outcome: &str,
            reason: Option<&str>,
            target_count: usize,
            protected_count: usize,
            vl_calls: u8,
        ) -> MatrixCandidate {
            use engines::source_language_gate::{SourceGateDecision, SourceGateRejectReason};
            let decision = match outcome {
                "accepted_primary" => SourceGateDecision::AcceptedPrimary {
                    target_count,
                    protected_count,
                },
                "accepted_detector_fallback" => SourceGateDecision::AcceptedDetectorFallback {
                    target_count,
                    protected_count,
                },
                "accepted_isolated_protected_latin_geometry" => {
                    SourceGateDecision::AcceptedIsolatedProtectedLatinGeometry {
                        target_count,
                        protected_count,
                    }
                }
                "rejected_before_vl" => SourceGateDecision::RejectedBeforeVl {
                    reason: match reason {
                        Some("pp_no_han_protected_latin") => {
                            SourceGateRejectReason::PpNoHanProtectedLatin
                        }
                        _ => panic!("unsupported rejection reason"),
                    },
                },
                _ => panic!("unsupported outcome"),
            };
            assert_eq!(decision.vl_calls(), vl_calls);
            MatrixCandidate {
                candidate_key,
                confidence_bits: 0,
                layout_bbox_bits: [0; 4],
                crop_bounds: None,
                crop_rgba_blake3: None,
                vl_crop_bounds: None,
                vl_crop_rgba_blake3: None,
                pp_words: Vec::new(),
                vl_summary: None,
                decision: MatrixDecision { decision },
                selection: (0..target_count)
                    .map(|_| MatrixSelection {
                        role: "target".into(),
                        bbox_bits: [0; 4],
                    })
                    .chain((0..protected_count).map(|_| MatrixSelection {
                        role: "protected".into(),
                        bbox_bits: [0; 4],
                    }))
                    .collect(),
            }
        }

        let mut run = MatrixRun {
            policy: "C1".into(),
            candidates: vec![
                candidate(0, "accepted_primary", None, 2, 0, 1),
                candidate(
                    1,
                    "rejected_before_vl",
                    Some("pp_no_han_protected_latin"),
                    0,
                    0,
                    0,
                ),
                candidate(2, "accepted_detector_fallback", None, 1, 0, 1),
                candidate(3, "accepted_primary", None, 1, 1, 1),
                candidate(
                    4,
                    "accepted_isolated_protected_latin_geometry",
                    None,
                    1,
                    1,
                    1,
                ),
            ],
            input_fingerprint: String::new(),
            pp_fingerprint: String::new(),
            outcome_fingerprint: String::new(),
            elapsed_ms: 0,
        };
        assert!(validate_final_recall(&run).is_ok());

        run.candidates[4]
            .selection
            .retain(|item| item.role != "protected");
        let error = validate_final_recall(&run).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("protected geometry count changed")
        );

        let mut peach_fixture = MatrixRun {
            policy: "C2".into(),
            candidates: vec![run.candidates[4].clone()],
            input_fingerprint: String::new(),
            pp_fingerprint: String::new(),
            outcome_fingerprint: String::new(),
            elapsed_ms: 0,
        };
        peach_fixture.candidates[0].candidate_key = 0;
        peach_fixture.candidates[0].selection.push(MatrixSelection {
            role: "protected".into(),
            bbox_bits: [1; 4],
        });
        assert!(validate_fixture_recall("peach-hip.png", &peach_fixture).is_ok());
        peach_fixture.candidates[0].decision.decision =
            engines::source_language_gate::SourceGateDecision::AcceptedPrimary {
                target_count: 1,
                protected_count: 1,
            };
        assert!(validate_fixture_recall("peach-hip.png", &peach_fixture).is_err());

        run.candidates[4].decision.decision =
            engines::source_language_gate::SourceGateDecision::RejectedAfterVl {
                reason:
                    engines::source_language_gate::SourceGateRejectReason::PpVlCharacterMismatch,
            };
        let error = validate_final_recall(&run).unwrap_err();
        assert!(error.to_string().contains("candidate 4 was not restored"));
    }

    #[test]
    fn source_gate_policy_selector_uses_the_cpu_metal_intersection_and_area_order() {
        use engines::source_language_gate::{SourceGateDecision, SourceGateRejectReason};

        fn candidate(candidate_key: usize, accepted: bool, crop: [u32; 4]) -> MatrixCandidate {
            let decision = match candidate_key {
                1 => SourceGateDecision::RejectedBeforeVl {
                    reason: SourceGateRejectReason::PpNoHanProtectedLatin,
                },
                4 if accepted => SourceGateDecision::AcceptedIsolatedProtectedLatinGeometry {
                    target_count: 1,
                    protected_count: 1,
                },
                4 => SourceGateDecision::RejectedAfterVl {
                    reason: SourceGateRejectReason::PpVlCharacterMismatch,
                },
                0 => SourceGateDecision::AcceptedPrimary {
                    target_count: 2,
                    protected_count: 0,
                },
                2 => SourceGateDecision::AcceptedDetectorFallback {
                    target_count: 1,
                    protected_count: 0,
                },
                _ => SourceGateDecision::AcceptedPrimary {
                    target_count: 1,
                    protected_count: 1,
                },
            };
            let (target_count, protected_count) = match &decision {
                SourceGateDecision::AcceptedPrimary {
                    target_count,
                    protected_count,
                }
                | SourceGateDecision::AcceptedDetectorFallback {
                    target_count,
                    protected_count,
                }
                | SourceGateDecision::AcceptedIsolatedProtectedLatinGeometry {
                    target_count,
                    protected_count,
                } => (*target_count, *protected_count),
                _ => (0, 0),
            };
            MatrixCandidate {
                candidate_key,
                confidence_bits: 1.0_f32.to_bits(),
                layout_bbox_bits: [0.0_f32.to_bits(); 4],
                crop_bounds: Some(crop),
                crop_rgba_blake3: Some(format!("crop-{candidate_key}")),
                vl_crop_bounds: Some(crop),
                vl_crop_rgba_blake3: Some(format!("vl-crop-{candidate_key}")),
                pp_words: Vec::new(),
                vl_summary: None,
                decision: MatrixDecision { decision },
                selection: (0..target_count)
                    .map(|_| MatrixSelection {
                        role: "target".into(),
                        bbox_bits: [0; 4],
                    })
                    .chain((0..protected_count).map(|_| MatrixSelection {
                        role: "protected".into(),
                        bbox_bits: [0; 4],
                    }))
                    .collect(),
            }
        }

        fn run(policy: &str, accepted: bool, inset: u32) -> MatrixRun {
            let crop = [10 - inset, 10 - inset, 20 + inset, 20 + inset];
            MatrixRun {
                policy: policy.into(),
                candidates: (0..5)
                    .map(|candidate_key| candidate(candidate_key, accepted, crop))
                    .collect(),
                input_fingerprint: String::new(),
                pp_fingerprint: String::new(),
                outcome_fingerprint: String::new(),
                elapsed_ms: 0,
            }
        }

        fn report(backend: &str, c2_accepted: bool) -> SourceGateMatrixReport {
            SourceGateMatrixReport {
                schema_version: 2,
                environment: MatrixEnvironment {
                    backend: backend.into(),
                    layout_backend: backend.into(),
                    pp_backend: "rten_cpu".into(),
                    vl_backend: backend.into(),
                    raw_blake3: "raw".into(),
                    decoded_rgba_blake3: "rgba".into(),
                    model_blake3: "model".into(),
                    config_blake3: "config".into(),
                    binary_blake3: "binary".into(),
                },
                policy_probes: vec![
                    run("C0", false, 0),
                    run("C1", false, 1),
                    run("C2", c2_accepted, 2),
                    run("C4", true, 4),
                    run("Q2", true, 3),
                ],
                full_image_runs: Vec::new(),
                fixture_runs: BTreeMap::new(),
            }
        }

        let cpu = report("cpu", true);
        let metal = report("metal", true);
        let selection = select_source_gate_policy(&cpu, &metal).unwrap();
        assert_eq!(selection.selected_policy, "C2");

        let metal_without_c2 = report("metal", false);
        let selection = select_source_gate_policy(&cpu, &metal_without_c2).unwrap();
        assert_eq!(selection.selected_policy, "Q2");
    }

    #[test]
    fn source_gate_cross_backend_gate_freezes_environment_and_normalizes_candidates() {
        use engines::source_language_gate::{SourceGateDecision, SourceGateRejectReason};

        fn candidate(
            candidate_key: usize,
            x: f32,
            decision: SourceGateDecision,
        ) -> MatrixCandidate {
            MatrixCandidate {
                candidate_key,
                confidence_bits: 1.0_f32.to_bits(),
                layout_bbox_bits: [
                    x.to_bits(),
                    0.0_f32.to_bits(),
                    10.0_f32.to_bits(),
                    10.0_f32.to_bits(),
                ],
                crop_bounds: Some([x as u32, 0, x as u32 + 10, 10]),
                crop_rgba_blake3: Some(format!("crop-{x}")),
                vl_crop_bounds: Some([x as u32, 0, x as u32 + 10, 10]),
                vl_crop_rgba_blake3: Some(format!("vl-crop-{x}")),
                pp_words: Vec::new(),
                vl_summary: None,
                decision: MatrixDecision { decision },
                selection: Vec::new(),
            }
        }

        let first = SourceGateDecision::InvalidCandidateGeometry;
        let second = SourceGateDecision::RejectedBeforeVl {
            reason: SourceGateRejectReason::PpNoHanProtectedLatin,
        };
        let mut reference_run = MatrixRun {
            policy: "C1".into(),
            candidates: vec![
                candidate(0, 0.0, first.clone()),
                candidate(1, 100.0, second.clone()),
            ],
            input_fingerprint: String::new(),
            pp_fingerprint: String::new(),
            outcome_fingerprint: String::new(),
            elapsed_ms: 1,
        };
        reference_run.refresh_fingerprints().unwrap();
        let mut vl_crop_drift = reference_run.clone();
        vl_crop_drift.candidates[0].vl_crop_rgba_blake3 = Some("different-vl-crop".into());
        vl_crop_drift.refresh_fingerprints().unwrap();
        assert_ne!(
            reference_run.input_fingerprint,
            vl_crop_drift.input_fingerprint
        );
        let mut current_run = MatrixRun {
            policy: "C1".into(),
            candidates: vec![candidate(0, 100.0, second), candidate(1, 0.0, first)],
            input_fingerprint: String::new(),
            pp_fingerprint: String::new(),
            outcome_fingerprint: String::new(),
            elapsed_ms: 1,
        };
        current_run.refresh_fingerprints().unwrap();
        assert_ne!(
            reference_run.outcome_fingerprint,
            current_run.outcome_fingerprint
        );

        let environment = MatrixEnvironment {
            backend: "cpu".into(),
            layout_backend: "cpu".into(),
            pp_backend: "rten_cpu".into(),
            vl_backend: "cpu".into(),
            raw_blake3: "raw".into(),
            decoded_rgba_blake3: "rgba".into(),
            model_blake3: "model".into(),
            config_blake3: "config".into(),
            binary_blake3: "binary".into(),
        };
        let reference = SourceGateMatrixReport {
            schema_version: 2,
            environment: environment.clone(),
            policy_probes: Vec::new(),
            full_image_runs: vec![reference_run],
            fixture_runs: BTreeMap::new(),
        };
        let mut current_environment = environment;
        current_environment.backend = "metal".into();
        current_environment.vl_backend = "metal".into();
        let mut current = SourceGateMatrixReport {
            schema_version: 2,
            environment: current_environment,
            policy_probes: Vec::new(),
            full_image_runs: vec![current_run],
            fixture_runs: BTreeMap::new(),
        };
        assert!(compare_outcome(&reference, &current).is_ok());

        current.environment.raw_blake3 = "different".into();
        assert!(
            compare_outcome(&reference, &current)
                .unwrap_err()
                .to_string()
                .contains("environment drift")
        );
    }

    #[test]
    fn source_gate_matrix_decision_report_is_derived_from_the_enum() {
        let report = MatrixDecision {
            decision: engines::source_language_gate::SourceGateDecision::RejectedAfterVl {
                reason:
                    engines::source_language_gate::SourceGateRejectReason::PpVlCharacterMismatch,
            },
        };
        let encoded = serde_json::to_value(&report).unwrap();
        assert_eq!(encoded["pp_calls"], 1);
        assert_eq!(encoded["vl_calls"], 1);
        assert_eq!(encoded["vl_stage"], "completed");
        assert_eq!(encoded["fallback"], "none");
        assert_eq!(
            serde_json::from_value::<MatrixDecision>(encoded).unwrap(),
            report
        );
    }

    #[tokio::test]
    #[ignore = "requires installed PP-DocLayout, PP-OCRv5, and PaddleOCR-VL models"]
    async fn source_gate_real_crop_source_gate_runtime_matrix() -> anyhow::Result<()> {
        use engines::source_language_gate::SourceGateCropPolicy;

        let report_path_env = std::env::var("SOURCE_GATE_MATRIX_REPORT").ok();
        let fallback_report_dir = report_path_env
            .is_none()
            .then(tempfile::tempdir)
            .transpose()?;
        let report_path = report_path_env
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                fallback_report_dir
                    .as_ref()
                    .expect("fallback report directory")
                    .path()
                    .join("source-gate-matrix.json")
            });
        let mode = std::env::var("SOURCE_GATE_MATRIX_MODE").unwrap_or_else(|_| "write".into());
        if mode == "select_policy" {
            let cpu_path =
                std::env::var("SOURCE_GATE_MATRIX_POLICY_CPU_REFERENCE").map_err(|_| {
                    anyhow::anyhow!("SOURCE_GATE_MATRIX_POLICY_CPU_REFERENCE is required")
                })?;
            let metal_path =
                std::env::var("SOURCE_GATE_MATRIX_POLICY_METAL_REFERENCE").map_err(|_| {
                    anyhow::anyhow!("SOURCE_GATE_MATRIX_POLICY_METAL_REFERENCE is required")
                })?;
            let cpu: SourceGateMatrixReport = serde_json::from_slice(&std::fs::read(cpu_path)?)?;
            let metal: SourceGateMatrixReport =
                serde_json::from_slice(&std::fs::read(metal_path)?)?;
            let selection = select_source_gate_policy(&cpu, &metal)?;
            std::fs::write(&report_path, serde_json::to_vec_pretty(&selection)?)?;
            return Ok(());
        }

        let input = std::env::var("SOURCE_GATE_MATRIX_INPUT")
            .map_err(|_| anyhow::anyhow!("SOURCE_GATE_MATRIX_INPUT is required"))?;
        let runs = std::env::var("SOURCE_GATE_MATRIX_RUNS")
            .unwrap_or_else(|_| "1".into())
            .parse::<usize>()?;
        anyhow::ensure!(runs > 0, "SOURCE_GATE_MATRIX_RUNS must be positive");
        let backend = std::env::var("SOURCE_GATE_MATRIX_BACKEND").unwrap_or_else(|_| "cpu".into());
        let (cpu, compute) = match backend.as_str() {
            "cpu" => (true, ComputePolicy::CpuOnly),
            "metal" => (false, ComputePolicy::PreferGpu),
            _ => anyhow::bail!("SOURCE_GATE_MATRIX_BACKEND must be \"cpu\" or \"metal\""),
        };
        let primary_policy = match std::env::var("SOURCE_GATE_MATRIX_POLICY").ok().as_deref() {
            Some("C0") => SourceGateCropPolicy::C0,
            Some("C1") => SourceGateCropPolicy::C1,
            Some("C2") => SourceGateCropPolicy::C2,
            Some("C4") => SourceGateCropPolicy::C4,
            Some("Q2") => SourceGateCropPolicy::Q2,
            Some("S25L4") => SourceGateCropPolicy::S25L4,
            Some("S25L5") => SourceGateCropPolicy::S25L5,
            Some("S25L6") => SourceGateCropPolicy::S25L6,
            Some("S25L7") => SourceGateCropPolicy::S25L7,
            Some(_) => anyhow::bail!(
                "SOURCE_GATE_MATRIX_POLICY must be C0, C1, C2, C4, Q2, S25L4, S25L5, S25L6, or S25L7"
            ),
            None => SourceGateCropPolicy::production(),
        };

        let bytes = std::fs::read(&input)?;
        let image = image::load_from_memory(&bytes)?;
        let data_root = default_app_data_root();
        let runtime = Arc::new(RuntimeManager::new(data_root.as_std_path(), compute)?);
        runtime.prepare().await?;
        let layout_device = koharu_ml::device(cpu)?;
        let layout_backend = if layout_device.is_metal() {
            "metal"
        } else if layout_device.is_cuda() {
            "cuda"
        } else {
            "cpu"
        };
        let llama_backend = crate::app::shared_llama_backend(&runtime)?;
        let vl_backend = if !cpu && llama_backend.supports_gpu_offload() {
            backend.clone()
        } else {
            "cpu".into()
        };
        let registry = Arc::new(Registry::new());
        let llm = Arc::new(llm::Model::empty_for_test((*runtime).clone(), cpu));
        let renderer = Arc::new(renderer::Renderer::new()?);
        let planner = Arc::new(TypographyPlanner::default());

        eprintln!("source-gate matrix {primary_policy:?} run 1/{runs}");
        let reference_run = run_matrix_once(
            &bytes,
            &image,
            registry.clone(),
            runtime.clone(),
            cpu,
            llm.clone(),
            renderer.clone(),
            planner.clone(),
            primary_policy,
        )
        .await?;
        let mut policy_probes = Vec::new();
        let run_policy_probes =
            std::env::var("SOURCE_GATE_MATRIX_RUN_POLICY_PROBES").as_deref() == Ok("1");
        if run_policy_probes {
            let policies =
                if std::env::var("SOURCE_GATE_MATRIX_RUN_RATIO_PROBES").as_deref() == Ok("1") {
                    vec![
                        SourceGateCropPolicy::S25L4,
                        SourceGateCropPolicy::S25L5,
                        SourceGateCropPolicy::S25L6,
                        SourceGateCropPolicy::S25L7,
                    ]
                } else {
                    vec![
                        SourceGateCropPolicy::C0,
                        SourceGateCropPolicy::C1,
                        SourceGateCropPolicy::C2,
                        SourceGateCropPolicy::C4,
                        SourceGateCropPolicy::Q2,
                    ]
                };
            for policy in policies {
                let probe = if policy == primary_policy {
                    reference_run.clone()
                } else {
                    run_matrix_once(
                        &bytes,
                        &image,
                        registry.clone(),
                        runtime.clone(),
                        cpu,
                        llm.clone(),
                        renderer.clone(),
                        planner.clone(),
                        policy,
                    )
                    .await?
                };
                policy_probes.push(normalize_run(&reference_run, probe)?);
            }
        }

        let mut full_image_runs = vec![reference_run.clone()];
        while full_image_runs.len() < runs {
            eprintln!(
                "source-gate matrix {primary_policy:?} run {}/{runs}",
                full_image_runs.len() + 1
            );
            let current = run_matrix_once(
                &bytes,
                &image,
                registry.clone(),
                runtime.clone(),
                cpu,
                llm.clone(),
                renderer.clone(),
                planner.clone(),
                primary_policy,
            )
            .await?;
            full_image_runs.push(normalize_run(&reference_run, current)?);
        }
        for current in &full_image_runs {
            anyhow::ensure!(
                current.input_fingerprint == reference_run.input_fingerprint,
                "same-process input drift"
            );
            anyhow::ensure!(
                current.pp_fingerprint == reference_run.pp_fingerprint,
                "same-process PP drift"
            );
            anyhow::ensure!(
                current.outcome_fingerprint == reference_run.outcome_fingerprint,
                "same-process outcome drift"
            );
        }

        let mut fixture_runs = BTreeMap::new();
        if !run_policy_probes {
            let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/source-gate-deterministic-recall");
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(fixture_dir.join("fixture-manifest.json"))?)?;
            for fixture in manifest["fixtures"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("fixture manifest entries missing"))?
            {
                let name = fixture["name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("fixture name missing"))?;
                let fixture_bytes = std::fs::read(fixture_dir.join(name))?;
                let fixture_image = image::load_from_memory(&fixture_bytes)?;
                eprintln!("source-gate fixture {name} run 1/{runs}");
                let reference_fixture = run_fixture_once(
                    &fixture_bytes,
                    &fixture_image,
                    registry.clone(),
                    runtime.clone(),
                    cpu,
                    llm.clone(),
                    renderer.clone(),
                    planner.clone(),
                    primary_policy,
                )
                .await?;
                validate_fixture_recall(name, &reference_fixture)?;
                let mut repeated = vec![reference_fixture.clone()];
                while repeated.len() < runs {
                    eprintln!(
                        "source-gate fixture {name} run {}/{runs}",
                        repeated.len() + 1
                    );
                    let current = run_fixture_once(
                        &fixture_bytes,
                        &fixture_image,
                        registry.clone(),
                        runtime.clone(),
                        cpu,
                        llm.clone(),
                        renderer.clone(),
                        planner.clone(),
                        primary_policy,
                    )
                    .await?;
                    let current = normalize_run(&reference_fixture, current)?;
                    anyhow::ensure!(
                        current.input_fingerprint == reference_fixture.input_fingerprint,
                        "fixture {name} same-process input drift"
                    );
                    anyhow::ensure!(
                        current.pp_fingerprint == reference_fixture.pp_fingerprint,
                        "fixture {name} same-process PP drift"
                    );
                    anyhow::ensure!(
                        current.outcome_fingerprint == reference_fixture.outcome_fingerprint,
                        "fixture {name} same-process outcome drift"
                    );
                    validate_fixture_recall(name, &current)?;
                    repeated.push(current);
                }
                fixture_runs.insert(name.to_string(), repeated);
            }
        }

        let executable = std::env::current_exe()?;
        let environment = MatrixEnvironment {
            backend,
            layout_backend: layout_backend.into(),
            pp_backend: "rten_cpu".into(),
            vl_backend,
            raw_blake3: blake3::hash(&bytes).to_hex().to_string(),
            decoded_rgba_blake3: engines::source_language_gate::rgba_fingerprint(&image),
            model_blake3: hash_model_trees(data_root.as_std_path())?,
            config_blake3: blake3::hash(b"HanOnly|pp-doclayout-v3|pp-ocr-v5-source-gate|0.5")
                .to_hex()
                .to_string(),
            binary_blake3: hash_file(&executable)?,
        };
        let report = SourceGateMatrixReport {
            schema_version: 3,
            environment,
            policy_probes,
            full_image_runs,
            fixture_runs,
        };
        let report_json = serde_json::to_vec_pretty(&report)?;
        std::fs::write(&report_path, &report_json)?;

        if report.policy_probes.is_empty()
            && let Ok(selection_path) = std::env::var("SOURCE_GATE_MATRIX_SELECTION_REPORT")
        {
            let selection: SourceGatePolicySelectionReport =
                serde_json::from_slice(&std::fs::read(selection_path)?)?;
            anyhow::ensure!(
                report
                    .full_image_runs
                    .iter()
                    .all(|run| run.policy == selection.selected_policy),
                "active source gate policy differs from selected policy"
            );
            anyhow::ensure!(
                report
                    .fixture_runs
                    .values()
                    .flatten()
                    .all(|run| run.policy == selection.selected_policy),
                "fixture source gate policy differs from selected policy"
            );
            for run in &report.full_image_runs {
                validate_final_recall(run)?;
            }
        }

        if mode != "write" {
            let reference_path = std::env::var("SOURCE_GATE_MATRIX_REFERENCE")
                .map_err(|_| anyhow::anyhow!("SOURCE_GATE_MATRIX_REFERENCE is required"))?;
            let reference: SourceGateMatrixReport =
                serde_json::from_slice(&std::fs::read(reference_path)?)?;
            match mode.as_str() {
                "compare_same_backend" => compare_same_backend(&reference, &report)?,
                "compare_outcome" => compare_outcome(&reference, &report)?,
                _ => anyhow::bail!("unknown SOURCE_GATE_MATRIX_MODE: {mode}"),
            }
        }
        Ok(())
    }
}
