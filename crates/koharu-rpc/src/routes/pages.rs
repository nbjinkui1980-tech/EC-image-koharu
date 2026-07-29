//! Page + page-subresource byte-ingress routes.
//!
//! - `POST /pages`                           — multipart: create pages from N image files
//! - `PUT  /pages/{id}/masks/{role}`         — raw PNG body: upsert a mask node
//!   (role ∈ `segment`, `brushInpaint`)
//!
//! These ingress routes do the same server-side dance: read bytes → `blobs.put_bytes`
//! → emit an `Op` on the session history.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Multipart, Path, Query, State};
use image::GenericImageView;
use koharu_app::AppConfig;
use koharu_app::pipeline::{self, EngineCtx, PipelineRunOptions};
use koharu_core::{
    BlobRef, ImageData, ImageRole, MaskRole, Node, NodeDataPatch, NodeId, NodeKind, Op, Page,
    PageId, ReadingOrder, Region, Transform,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::AppState;
use crate::error::{ApiError, ApiResult};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct PutMaskParams {
    /// Optional pipeline engine to run after the mask is updated.
    pub pipeline: Option<String>,
    /// Bounding box for the pipeline run.
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .routes(routes!(create_pages))
        .routes(routes!(create_pages_from_paths))
        .routes(routes!(put_mask))
        .routes(routes!(reorder_text_nodes))
}

fn repair_options(region: Region, config: &AppConfig) -> PipelineRunOptions {
    PipelineRunOptions {
        source_text_policy: config.pipeline.source_text_policy,
        region: Some(region),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// POST /pages  — create pages from uploaded image files
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatePagesResponse {
    pub pages: Vec<PageId>,
}

#[utoipa::path(
    post,
    path = "/pages",
    request_body(content_type = "multipart/form-data"),
    responses((status = 200, body = CreatePagesResponse))
)]
async fn create_pages(
    State(app): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<Json<CreatePagesResponse>> {
    let session = app
        .current_session()
        .ok_or_else(|| ApiError::bad_request("no project open"))?;

    // Collect (filename, bytes) pairs first so we can sort naturally.
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut replace = false;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("multipart: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "replace" {
            let text = field
                .text()
                .await
                .map_err(|e| ApiError::bad_request(format!("{e}")))?;
            replace = text == "true" || text == "1";
            continue;
        }
        let filename = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("page-{}.bin", files.len() + 1));
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(format!("read file: {e}")))?;
        files.push((filename, bytes.to_vec()));
    }

    files.sort_by(|a, b| natord::compare(&a.0, &b.0));

    // Optionally clear the project first. Emitted as a batch so it's one undo step.
    let starting_index = if replace {
        let scene = session.scene.read();
        let remove_ops: Vec<Op> = scene
            .pages
            .keys()
            .copied()
            .map(|id| Op::RemovePage {
                id,
                prev_page: scene.pages[&id].clone(),
                prev_index: scene.pages.get_index_of(&id).unwrap_or(0),
            })
            .collect();
        drop(scene);
        if !remove_ops.is_empty() {
            app.apply(Op::Batch {
                ops: remove_ops,
                label: "Replace pages (clear)".into(),
            })
            .map_err(ApiError::internal)?;
        }
        0
    } else {
        session.scene.read().pages.len()
    };

    // Decode + hash + write each file in parallel. Image decode is the
    // dominant cost per page (~10–50ms for a typical JPEG/PNG), so a
    // 200-page folder benefits almost linearly from multi-core. The output
    // vector preserves the pre-sorted order because rayon's `par_iter`
    // keeps indices through `collect::<Result<Vec<_>>>()`.
    //
    // `BlobStore::put_bytes` is Send + Sync (stateless beyond disk + blake3),
    // so it's safe to call from the rayon pool.
    //
    // Run the rayon section on a blocking thread so we don't stall the
    // tokio runtime while decoding.
    let blobs = session.blobs.clone();
    let decoded: Vec<(String, u32, u32, BlobRef)> = tokio::task::spawn_blocking(move || {
        files
            .into_par_iter()
            .map(
                |(filename, bytes)| -> ApiResult<(String, u32, u32, BlobRef)> {
                    let img = image::load_from_memory(&bytes)
                        .map_err(|e| ApiError::bad_request(format!("decode `{filename}`: {e}")))?;
                    let (w, h) = img.dimensions();
                    let blob = blobs.put_bytes(&bytes).map_err(ApiError::internal)?;
                    Ok((filename, w, h, blob))
                },
            )
            .collect::<ApiResult<Vec<_>>>()
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::anyhow!("import task panicked: {e}")))??;

    // Build one AddPage batch for the whole import.
    let mut ops = Vec::with_capacity(decoded.len());
    let mut created_ids = Vec::with_capacity(decoded.len());
    for (i, (filename, w, h, blob)) in decoded.into_iter().enumerate() {
        let mut page = Page::new(&filename, w, h);
        let page_id = page.id;
        let source_node_id = NodeId::new();
        page.nodes.insert(
            source_node_id,
            Node {
                id: source_node_id,
                transform: Transform::default(),
                visible: true,
                kind: NodeKind::Image(ImageData {
                    role: ImageRole::Source,
                    blob,
                    opacity: 1.0,
                    natural_width: w,
                    natural_height: h,
                    name: Some(filename),
                }),
            },
        );
        created_ids.push(page_id);
        ops.push(Op::AddPage {
            page,
            at: starting_index + i,
        });
    }

    app.apply(Op::Batch {
        ops,
        label: "Import pages".into(),
    })
    .map_err(ApiError::internal)?;

    Ok(Json(CreatePagesResponse { pages: created_ids }))
}

// ---------------------------------------------------------------------------
// POST /pages/from-paths — Tauri fast-path: import by reading files directly
// from disk, skipping multipart upload entirely
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreatePagesFromPathsRequest {
    pub paths: Vec<String>,
    #[serde(default)]
    pub replace: bool,
}

/// Create pages by reading image files from absolute paths on the server's
/// filesystem. This is the Tauri desktop import path — the webview picker
/// returns paths, and the backend reads + decodes + hashes them in parallel
/// without a round-trip through JS memory or a multipart upload body.
///
/// Web clients should keep using `POST /pages` with multipart.
#[utoipa::path(
    post,
    path = "/pages/from-paths",
    request_body = CreatePagesFromPathsRequest,
    responses((status = 200, body = CreatePagesResponse))
)]
async fn create_pages_from_paths(
    State(app): State<AppState>,
    Json(req): Json<CreatePagesFromPathsRequest>,
) -> ApiResult<Json<CreatePagesResponse>> {
    let session = app
        .current_session()
        .ok_or_else(|| ApiError::bad_request("no project open"))?;

    // Natural-order sort by filename component so `page-2.png` < `page-10.png`.
    let mut paths = req.paths;
    paths.sort_by(|a, b| {
        let af = std::path::Path::new(a)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(a);
        let bf = std::path::Path::new(b)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(b);
        natord::compare(af, bf)
    });

    let starting_index = if req.replace {
        let scene = session.scene.read();
        let remove_ops: Vec<Op> = scene
            .pages
            .keys()
            .copied()
            .map(|id| Op::RemovePage {
                id,
                prev_page: scene.pages[&id].clone(),
                prev_index: scene.pages.get_index_of(&id).unwrap_or(0),
            })
            .collect();
        drop(scene);
        if !remove_ops.is_empty() {
            app.apply(Op::Batch {
                ops: remove_ops,
                label: "Replace pages (clear)".into(),
            })
            .map_err(ApiError::internal)?;
        }
        0
    } else {
        session.scene.read().pages.len()
    };

    let blobs = session.blobs.clone();
    let decoded: Vec<(String, u32, u32, BlobRef)> = tokio::task::spawn_blocking(move || {
        paths
            .into_par_iter()
            .map(|path| -> ApiResult<(String, u32, u32, BlobRef)> {
                let filename = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "page.bin".to_string());
                let bytes = std::fs::read(&path)
                    .map_err(|e| ApiError::bad_request(format!("read `{filename}`: {e}")))?;
                let img = image::load_from_memory(&bytes)
                    .map_err(|e| ApiError::bad_request(format!("decode `{filename}`: {e}")))?;
                let (w, h) = img.dimensions();
                let blob = blobs.put_bytes(&bytes).map_err(ApiError::internal)?;
                Ok((filename, w, h, blob))
            })
            .collect::<ApiResult<Vec<_>>>()
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::anyhow!("import task panicked: {e}")))??;

    let mut ops = Vec::with_capacity(decoded.len());
    let mut created_ids = Vec::with_capacity(decoded.len());
    for (i, (filename, w, h, blob)) in decoded.into_iter().enumerate() {
        let mut page = Page::new(&filename, w, h);
        let page_id = page.id;
        let source_node_id = NodeId::new();
        page.nodes.insert(
            source_node_id,
            Node {
                id: source_node_id,
                transform: Transform::default(),
                visible: true,
                kind: NodeKind::Image(ImageData {
                    role: ImageRole::Source,
                    blob,
                    opacity: 1.0,
                    natural_width: w,
                    natural_height: h,
                    name: Some(filename),
                }),
            },
        );
        created_ids.push(page_id);
        ops.push(Op::AddPage {
            page,
            at: starting_index + i,
        });
    }

    app.apply(Op::Batch {
        ops,
        label: "Import pages".into(),
    })
    .map_err(ApiError::internal)?;

    Ok(Json(CreatePagesResponse { pages: created_ids }))
}

// ---------------------------------------------------------------------------
// PUT /pages/{id}/masks/{role}
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PutMaskResponse {
    pub node: NodeId,
    pub blob: BlobRef,
}

/// Upsert the `Mask { role }` node on a page with the raw image bytes in
/// the body. Emits `Op::UpdateNode` if a mask of that role exists, else
/// `Op::AddNode`. Used by the repair-brush / segment-edit flow; the
/// follow-up localized inpaint is a separate `POST /pipelines` call.
#[utoipa::path(
    put,
    path = "/pages/{id}/masks/{role}",
    params(
        ("id"   = PageId,   Path, description = "Page id"),
        ("role" = MaskRole, Path, description = "Mask role (segment|brushInpaint)"),
        PutMaskParams,
    ),
    request_body(content_type = "image/png"),
    responses((status = 200, body = PutMaskResponse))
)]
async fn put_mask(
    State(app): State<AppState>,
    Path((page_id, role)): Path<(PageId, MaskRole)>,
    Query(params): Query<PutMaskParams>,
    body: Bytes,
) -> ApiResult<Json<PutMaskResponse>> {
    let session = app
        .current_session()
        .ok_or_else(|| ApiError::bad_request("no project open"))?;
    if body.is_empty() {
        return Err(ApiError::bad_request("empty body"));
    }
    // Validate it actually decodes so we don't persist garbage.
    image::load_from_memory(&body)
        .map_err(|e| ApiError::bad_request(format!("decode mask: {e}")))?;

    let blob = session.blobs.put_bytes(&body).map_err(ApiError::internal)?;

    // Find existing mask node of this role, or plan an AddNode.
    let (mut mask_op, node_id) = {
        let scene = session.scene.read();
        let existing = scene
            .page(page_id)
            .ok_or_else(|| ApiError::not_found(format!("page {page_id}")))?
            .nodes
            .iter()
            .find_map(|(id, node)| match &node.kind {
                NodeKind::Mask(m) if m.role == role => Some(*id),
                _ => None,
            });
        match existing {
            Some(id) => {
                let op = Op::UpdateNode {
                    page: page_id,
                    id,
                    patch: koharu_core::NodePatch {
                        data: Some(NodeDataPatch::Mask(koharu_core::MaskDataPatch {
                            blob: Some(blob.clone()),
                        })),
                        transform: None,
                        visible: None,
                    },
                    prev: koharu_core::NodePatch::default(),
                };
                (op, id)
            }
            None => {
                let node_id = NodeId::new();
                let at = scene.page(page_id).map(|p| p.nodes.len()).unwrap_or(0);
                let node = Node {
                    id: node_id,
                    transform: Transform::default(),
                    visible: matches!(role, MaskRole::BrushInpaint),
                    kind: NodeKind::Mask(koharu_core::MaskData {
                        role,
                        blob: blob.clone(),
                    }),
                };
                (
                    Op::AddNode {
                        page: page_id,
                        node,
                        at,
                    },
                    node_id,
                )
            }
        }
    };

    if let Some(engine_id) = params.pipeline.as_ref() {
        // Atomic Batch: Mask Update + Pipeline Run
        let mut ops = vec![mask_op.clone()];

        // 1. Simulate the mask update in a cloned scene so the engine sees it.
        let mut scene = session.scene_snapshot();
        mask_op
            .apply(&mut scene)
            .map_err(|e| ApiError::internal(e.into()))?;

        // 2. Prepare EngineCtx
        let region = Region {
            x: params.x.unwrap_or(0.0) as u32,
            y: params.y.unwrap_or(0.0) as u32,
            width: params.width.unwrap_or(0.0) as u32,
            height: params.height.unwrap_or(0.0) as u32,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let options = repair_options(region, &app.config.load());
        let ctx = EngineCtx {
            scene: &scene,
            page: page_id,
            blobs: &session.blobs,
            runtime: &app.runtime,
            cancel: &cancel,
            options: &options,
            llm: &app.llm,
            renderer: &app.renderer,
            typography_planner: &app.typography_planner,
            warnings: None,
        };

        // 3. Run Engine (Synchronously for this request)
        let engine_info = pipeline::Registry::find(engine_id)
            .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
        let engine = app
            .registry
            .get(engine_info.id, &app.runtime, app.cpu_only())
            .await
            .map_err(|e| ApiError::internal(anyhow::anyhow!("load engine: {e:#}")))?;

        let engine_ops = engine
            .run(ctx)
            .await
            .map_err(|e| ApiError::internal(anyhow::anyhow!("run engine: {e:#}")))?;

        ops.extend(engine_ops);

        let batch = Op::Batch {
            ops,
            label: format!("Repair Brush ({})", engine_id),
        };
        app.apply(batch).map_err(ApiError::internal)?;
    } else {
        app.apply(mask_op).map_err(ApiError::internal)?;
    }

    Ok(Json(PutMaskResponse {
        node: node_id,
        blob,
    }))
}

// ---------------------------------------------------------------------------
// POST /pages/{page_id}/reorder-text-nodes  — re-sort existing text blocks
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/pages/{page_id}/reorder-text-nodes",
    params(("page_id" = PageId, Path, description = "Page id")),
    request_body = ReadingOrder,
    responses((status = 200))
)]
async fn reorder_text_nodes(
    State(app): State<AppState>,
    Path(page_id): Path<PageId>,
    Json(order): Json<ReadingOrder>,
) -> ApiResult<axum::http::StatusCode> {
    if order == ReadingOrder::Custom {
        return Ok(axum::http::StatusCode::OK);
    }

    tracing::debug!(
        "Reordering text nodes for page {} with order {:?}",
        page_id,
        order
    );
    let new_order_opt = {
        let session = app
            .current_session()
            .ok_or_else(|| ApiError::bad_request("no project open"))?;
        let scene = session.scene_snapshot();
        let page = scene
            .page(page_id)
            .ok_or_else(|| ApiError::not_found("page not found"))?;

        // 1. Collect all text nodes and their bboxes
        let mut text_nodes: Vec<([f32; 4], NodeId)> = page
            .nodes
            .iter()
            .filter_map(|(id, node)| {
                if let NodeKind::Text(_) = &node.kind {
                    let b = &node.transform;
                    Some(([b.x, b.y, b.x + b.width, b.y + b.height], *id))
                } else {
                    None
                }
            })
            .collect();

        tracing::debug!(
            "Found {} text nodes. Current order: {:?}",
            text_nodes.len(),
            text_nodes.iter().map(|(_, id)| id).collect::<Vec<_>>()
        );

        if text_nodes.len() <= 1 {
            return Ok(axum::http::StatusCode::OK);
        }

        // 2. Sort them
        koharu_app::pipeline::support::sort_manga_reading_order(&mut text_nodes, order);

        // 3. Construct the full node order
        let mut new_order = Vec::with_capacity(page.nodes.len());
        let mut sorted_text_iter = text_nodes.into_iter().map(|(_, id)| id);

        for (id, node) in page.nodes.iter() {
            if let NodeKind::Text(_) = &node.kind {
                let sorted_id = sorted_text_iter.next().ok_or_else(|| {
                    ApiError::internal(anyhow::anyhow!("text node count mismatch during reorder"))
                })?;
                new_order.push(sorted_id);
            } else {
                new_order.push(*id);
            }
        }

        // Only return new order if it actually changed
        let changed = page
            .nodes
            .keys()
            .zip(new_order.iter())
            .any(|(old, new)| old != new);

        Ok::<_, ApiError>(if changed { Some(new_order) } else { None })
    }?;

    if let Some(new_order) = new_order_opt {
        tracing::debug!("Applying new order: {:?}", new_order);
        let op = Op::ReorderNodes {
            page: page_id,
            order: new_order,
            prev_order: Vec::new(),
        };
        app.apply(op).map_err(ApiError::internal)?;
    } else {
        tracing::debug!("Order unchanged, skipping Op::ReorderNodes");
    }

    Ok(axum::http::StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use koharu_app::{App, AppConfig, ProjectSession, config::SourceTextPolicy};
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use uuid::Uuid;

    struct TestDir(std::path::PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn repair_options_inherits_source_text_policy() {
        let mut config = AppConfig::default();
        config.pipeline.source_text_policy = SourceTextPolicy::AllText;
        let region = Region {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };

        let options = repair_options(region, &config);

        assert_eq!(options.source_text_policy, SourceTextPolicy::AllText);
        let actual = options.region.expect("repair region");
        assert_eq!(
            (actual.x, actual.y, actual.width, actual.height),
            (region.x, region.y, region.width, region.height)
        );
    }

    #[test]
    fn repair_brush_engine_ctx_keeps_single_engine_path() {
        let options = repair_options(
            Region {
                x: 4,
                y: 5,
                width: 6,
                height: 7,
            },
            &AppConfig::default(),
        );
        assert_eq!(options.region.unwrap().width, 6);
        assert!(pipeline::Registry::find("lama-manga").is_ok());
        assert!(pipeline::Registry::find("cloud-typography-planner").is_ok());
    }

    #[tokio::test]
    #[ignore = "hanonly-pre-b1-red"]
    async fn hanonly_pre_b1_red_t2_replace_import_atomicity_contract() {
        let root = TestDir(
            std::env::temp_dir().join(format!("koharu-replace-atomicity-{}", Uuid::new_v4())),
        );
        std::fs::create_dir_all(&root.0).expect("create test root");
        let project_dir =
            Utf8PathBuf::from_path_buf(root.0.join("project.khrproj")).expect("UTF-8 project path");
        let session = ProjectSession::create(&project_dir, "atomicity").expect("create session");
        let old_page = Page::new("old.png", 4, 4);
        let old_page_id = old_page.id;
        session
            .apply(Op::AddPage {
                page: old_page,
                at: 0,
            })
            .expect("seed old page");

        let runtime = RuntimeManager::new(root.0.join("runtime"), ComputePolicy::CpuOnly)
            .expect("create runtime");
        runtime.prepare().await.expect("prepare runtime");
        let runtime = Arc::new(runtime);
        let app = Arc::new(
            App::new(AppConfig::default(), runtime.clone(), true, "test").expect("create app"),
        );
        app.session.store(Some(session.clone()));
        let state = crate::BootstrapManager::new(runtime);
        assert!(state.set_app(app).is_ok(), "set test app");

        let bad_image = root.0.join("broken.png");
        std::fs::write(&bad_image, b"not an image").expect("write invalid image");
        let scene_before = postcard::to_allocvec(&*session.scene.read()).expect("encode scene");
        let epoch_before = session.epoch();
        let history_before =
            std::fs::read(project_dir.join("history.log")).expect("read history log");

        let result = create_pages_from_paths(
            State(state),
            Json(CreatePagesFromPathsRequest {
                paths: vec![bad_image.to_string_lossy().into_owned()],
                replace: true,
            }),
        )
        .await;

        assert!(
            result.is_err(),
            "invalid replacement image must be rejected"
        );
        assert_eq!(
            postcard::to_allocvec(&*session.scene.read()).expect("encode scene after rejection"),
            scene_before,
            "failed replacement must preserve the complete Scene"
        );
        assert_eq!(
            session.epoch(),
            epoch_before,
            "failed replacement must preserve the History epoch"
        );
        assert_eq!(
            std::fs::read(project_dir.join("history.log"))
                .expect("read history log after rejection"),
            history_before,
            "failed replacement must preserve canonical History bytes"
        );
        assert!(
            session.scene.read().pages.contains_key(&old_page_id),
            "failed replacement must preserve the original page"
        );
    }
}
