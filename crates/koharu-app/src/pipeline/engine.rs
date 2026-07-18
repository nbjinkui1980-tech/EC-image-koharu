//! Engine trait + inventory-based registry + DAG resolver.
//!
//! An engine is a pluggable model that transforms one page. It declares the
//! artifacts it needs and produces; the DAG resolver derives execution order.
//!
//! **Engines emit ops, not mutations.** `run()` returns `Vec<Op>`; the driver
//! wraps them in `Op::Batch` and hands to `ProjectSession::apply`.
//!
//! ## Adding an engine
//!
//! 1. Define a struct holding your model.
//! 2. Implement `Engine` for it (returning `Vec<Op>`).
//! 3. Register via `inventory::submit! { EngineInfo { … } }` with a static
//!    async `load` function.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, bail};
use async_trait::async_trait;
use koharu_core::{NodeId, Op, PageId, ReadingOrder, Region, Scene};
use koharu_runtime::RuntimeManager;
use parking_lot::RwLock;
use petgraph::algo::toposort;
use petgraph::graph::DiGraph;
use tracing::Instrument;

use crate::blobs::BlobStore;
use crate::config::SourceTextPolicy;
use crate::llm;
use crate::pipeline::artifacts::Artifact;
use crate::renderer;
use crate::typography::TypographyPlanner;

// ---------------------------------------------------------------------------
// EngineCtx — everything an engine needs to produce ops
// ---------------------------------------------------------------------------

pub type EngineWarningSink<'a> = dyn Fn(String) + Send + Sync + 'a;

pub struct EngineCtx<'a> {
    /// A cheap clone of the target page (read-only).
    pub scene: &'a Scene,
    pub page: PageId,
    pub blobs: &'a BlobStore,
    pub runtime: &'a RuntimeManager,
    pub cancel: &'a AtomicBool,
    pub options: &'a PipelineRunOptions,
    pub llm: &'a llm::Model,
    pub renderer: &'a renderer::Renderer,
    pub typography_planner: &'a TypographyPlanner,
    pub warnings: Option<&'a EngineWarningSink<'a>>,
}

impl EngineCtx<'_> {
    pub fn warn(&self, message: impl Into<String>) {
        if let Some(sink) = self.warnings {
            sink(message.into());
        }
    }
}

/// Options threaded through a pipeline run.
#[derive(Debug, Clone, Default)]
pub struct PipelineRunOptions {
    pub source_text_policy: SourceTextPolicy,
    pub target_language: Option<String>,
    pub system_prompt: Option<String>,
    pub default_font: Option<String>,
    /// Optional text-node scope for engines that can operate on individual
    /// text blocks. Engines that render full-page artifacts ignore it.
    pub text_node_ids: Option<Vec<NodeId>>,
    /// Optional bounding-box hint. Inpainter engines (lama/aot) honor it:
    /// composite onto the existing `Image { Inpainted }` (fallback Source)
    /// and process just that one block. Other engines ignore it.
    pub region: Option<Region>,
    pub reading_order: Option<ReadingOrder>,
}

// ---------------------------------------------------------------------------
// Engine trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Engine: Send + Sync + 'static {
    /// Run the engine on one page. Return the ops to apply.
    /// Empty `Vec` = nothing changed (still a success).
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>>;
}

// ---------------------------------------------------------------------------
// EngineInfo — static descriptor + factory (registered via inventory)
// ---------------------------------------------------------------------------

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type EngineLoadFn =
    for<'a> fn(&'a RuntimeManager, bool) -> BoxFuture<'a, Result<Box<dyn Engine>>>;

pub struct EngineInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub needs: &'static [Artifact],
    pub produces: &'static [Artifact],
    pub load: EngineLoadFn,
}

inventory::collect!(EngineInfo);

// ---------------------------------------------------------------------------
// Registry — lazy load + cache engine instances
// ---------------------------------------------------------------------------

pub struct Registry {
    engines: RwLock<HashMap<&'static str, Arc<dyn Engine>>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            engines: RwLock::new(HashMap::new()),
        }
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or load an engine instance by id.
    pub async fn get(
        &self,
        id: &str,
        runtime: &RuntimeManager,
        cpu: bool,
    ) -> Result<Arc<dyn Engine>> {
        if let Some(engine) = self.engines.read().get(id).cloned() {
            return Ok(engine);
        }
        let info = Self::find(id)?;
        let loaded = async { (info.load)(runtime, cpu).await }
            .instrument(tracing::info_span!("engine_load", engine = id))
            .await?;
        let engine: Arc<dyn Engine> = Arc::from(loaded);
        self.engines.write().insert(info.id, engine.clone());
        Ok(engine)
    }

    /// Drop all cached engines (frees GPU memory).
    pub fn clear(&self) {
        self.engines.write().clear();
    }

    #[cfg(test)]
    pub(crate) fn insert_test_engine(&self, id: &str, engine: Arc<dyn Engine>) {
        let info = Self::find(id).expect("registered test engine id");
        self.engines.write().insert(info.id, engine);
    }

    /// Find engine descriptor by id.
    pub fn find(id: &str) -> Result<&'static EngineInfo> {
        Self::catalog()
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown engine: {id}"))
    }

    /// All registered engine descriptors.
    pub fn catalog() -> Vec<&'static EngineInfo> {
        inventory::iter::<EngineInfo>.into_iter().collect()
    }

    /// Engines that produce a given artifact.
    pub fn providers(artifact: Artifact) -> Vec<&'static EngineInfo> {
        Self::catalog()
            .into_iter()
            .filter(|e| e.produces.contains(&artifact))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// DAG — derive execution order from artifact dependencies
// ---------------------------------------------------------------------------

/// Build a topological execution order from a set of engine infos.
pub fn build_order(infos: &[&EngineInfo]) -> Result<Vec<usize>> {
    let mut g = DiGraph::<usize, ()>::new();
    let mut id_to_node: HashMap<&str, _> = HashMap::new();

    for (i, info) in infos.iter().enumerate() {
        let n = g.add_node(i);
        if id_to_node.insert(info.id, n).is_some() {
            bail!("duplicate engine: {}", info.id);
        }
    }

    let mut producers: HashMap<Artifact, usize> = HashMap::new();
    for (i, info) in infos.iter().enumerate() {
        for &artifact in info.produces {
            producers.insert(artifact, i);
        }
    }

    for info in infos.iter() {
        let to = id_to_node[info.id];
        for &artifact in info.needs {
            if let Some(&producer) = producers.get(&artifact) {
                g.add_edge(id_to_node[infos[producer].id], to, ());
            }
        }
    }

    let order = toposort(&g, None)
        .map_err(|c| anyhow::anyhow!("cycle at '{}'", infos[g[c.node_id()]].id))?;
    Ok(order.into_iter().map(|n| g[n]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordered_ids(ids: &[&str]) -> Vec<&'static str> {
        let infos = ids
            .iter()
            .map(|id| Registry::find(id).unwrap())
            .collect::<Vec<_>>();
        build_order(&infos)
            .unwrap()
            .into_iter()
            .map(|index| infos[index].id)
            .collect()
    }

    #[test]
    fn orders_ocr_before_segmenter() {
        let ids = ordered_ids(&["manga-ocr", "comic-text-detector-seg"]);
        assert!(
            ids.iter().position(|id| *id == "manga-ocr")
                < ids.iter().position(|id| *id == "comic-text-detector-seg")
        );
    }

    #[test]
    fn orders_translator_before_inpainters() {
        for inpainter in ["lama-manga", "aot-inpainting", "flux2-klein"] {
            let ids = ordered_ids(&["llm", inpainter]);
            assert!(
                ids.iter().position(|id| *id == "llm") < ids.iter().position(|id| *id == inpainter),
                "translator must run before {inpainter}"
            );
        }
    }

    #[test]
    fn orders_inpainters_without_translator() {
        for inpainter in ["lama-manga", "aot-inpainting", "flux2-klein"] {
            assert_eq!(ordered_ids(&[inpainter]), vec![inpainter]);
        }
    }

    #[test]
    fn orders_repair_engine_alone() {
        assert_eq!(ordered_ids(&["lama-manga"]), vec!["lama-manga"]);
    }

    #[test]
    fn orders_typography_after_translator_before_renderer() {
        let order = ordered_ids(&["llm", "cloud-typography-planner", "koharu-renderer"]);
        let at = |id| order.iter().position(|candidate| *candidate == id).unwrap();
        assert!(at("llm") < at("cloud-typography-planner"));
        assert!(at("cloud-typography-planner") < at("koharu-renderer"));
    }

    #[test]
    fn orders_font_detector_before_typography() {
        let order = ordered_ids(&["yuzumarker-font-detection", "cloud-typography-planner"]);
        let at = |id| order.iter().position(|candidate| *candidate == id).unwrap();
        assert!(at("yuzumarker-font-detection") < at("cloud-typography-planner"));
        assert_eq!(
            ordered_ids(&["cloud-typography-planner"]),
            ["cloud-typography-planner"]
        );
        assert_eq!(ordered_ids(&["koharu-renderer"]), ["koharu-renderer"]);
    }

    #[test]
    fn orders_source_gate_after_detector_and_before_every_downstream_stage() {
        let ids = ordered_ids(&[
            "pp-doclayout-v3",
            "pp-ocr-v5-source-gate",
            "yuzumarker-font-detection",
            "speech-bubble-segmentation",
            "comic-text-detector-seg",
            "llm",
            "cloud-typography-planner",
            "lama-manga",
            "koharu-renderer",
        ]);
        let position = |id| ids.iter().position(|item| *item == id).unwrap();
        assert!(position("pp-doclayout-v3") < position("pp-ocr-v5-source-gate"));
        for consumer in [
            "yuzumarker-font-detection",
            "speech-bubble-segmentation",
            "comic-text-detector-seg",
            "llm",
            "cloud-typography-planner",
            "lama-manga",
            "koharu-renderer",
        ] {
            assert!(position("pp-ocr-v5-source-gate") < position(consumer));
        }
    }
}
