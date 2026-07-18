//! PaddleOCR-VL. Vision-language OCR driven by llama.cpp + mtmd.
//!
//! Each visible text node is cropped from the source image and recognised as
//! one node-level OCR block. HanOnly replaces this engine with the dedicated
//! source-language gate before the pipeline is loaded.

use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;
use koharu_core::{NodeDataPatch, NodePatch, Op, TextDataPatch};
use koharu_llm::paddleocr_vl::{PaddleOcrVl, PaddleOcrVlTask};
use koharu_ml::comic_text_detector::crop_text_block_bbox;

use crate::app::shared_llama_backend;
use crate::pipeline::artifacts::Artifact;
use crate::pipeline::engine::{Engine, EngineCtx, EngineInfo};
use crate::pipeline::engines::support::{load_source_image, text_node_to_region, text_nodes};

const MAX_NEW_TOKENS: usize = 256;

pub struct Model(Mutex<PaddleOcrVl>);

#[async_trait]
impl Engine for Model {
    async fn run(&self, ctx: EngineCtx<'_>) -> Result<Vec<Op>> {
        let texts = text_nodes(ctx.scene, ctx.page);
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let image = load_source_image(ctx.scene, ctx.page, ctx.blobs)?;
        let crops = texts
            .iter()
            .map(|(_, transform, text)| {
                crop_text_block_bbox(&image, &text_node_to_region(transform, text))
            })
            .collect::<Vec<_>>();
        let outputs = {
            let mut ocr = self
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("PaddleOCR mutex poisoned"))?;
            ocr.inference_images(&crops, PaddleOcrVlTask::Ocr, MAX_NEW_TOKENS)?
        };

        Ok(texts
            .iter()
            .zip(outputs)
            .map(|((node_id, _, _), output)| Op::UpdateNode {
                page: ctx.page,
                id: *node_id,
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(TextDataPatch {
                        text: Some(Some(output.text)),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                prev: NodePatch::default(),
            })
            .collect())
    }
}

inventory::submit! {
    EngineInfo {
        id: "paddle-ocr-vl-1.6",
        name: "PaddleOCR-VL",
        needs: &[Artifact::TextBoxes],
        produces: &[Artifact::OcrText],
        load: |runtime, cpu| Box::pin(async move {
            let backend = shared_llama_backend(runtime)?;
            let model = PaddleOcrVl::load(runtime, cpu, backend).await?;
            Ok(Box::new(Model(Mutex::new(model))) as Box<dyn Engine>)
        }),
    }
}
