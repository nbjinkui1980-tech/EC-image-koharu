//! Thin pipeline adapter for the cloud Typography Planner.

use anyhow::Result;
use async_trait::async_trait;

use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::load_source_image;
use crate::typography::build_typography_request;

pub struct Model;

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<koharu_core::Op>> {
        // Malformed scene/input state remains a hard pipeline error. Provider,
        // deadline and strict-response failures below are recoverable.
        let image = load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
        let fonts = ctx.renderer.available_fonts()?;
        let request = build_typography_request(
            ctx.scene,
            ctx.page,
            &image,
            &fonts,
            ctx.options.source_text_policy,
            ctx.options.text_node_ids.as_deref(),
            ctx.options.default_font.as_deref(),
        )?;
        match ctx.typography_planner.plan_page(&request).await {
            Ok(ops) => Ok(ops),
            Err(error) => {
                ctx.warn(format!("Typography Planner fallback: {error:#}"));
                Ok(Vec::new())
            }
        }
    }
}

inventory::submit! {
    EngineInfo {
        id: "cloud-typography-planner",
        name: "Cloud Typography Planner",
        needs: &[
            Artifact::Translations,
            Artifact::FontPredictions,
            Artifact::SourceTextBoxes,
        ],
        produces: &[Artifact::TypographyStyles],
        load: |_runtime, _cpu| Box::pin(async move { Ok(Box::new(Model) as Box<dyn Engine>) }),
    }
}
