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

// --- Batch import budget (AMEND-02) -----------------------------------------

const MAX_IMPORT_FILES: usize = 256;
const MAX_IMPORT_ENCODED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_IMPORT_DECODED_RGBA_BYTES: u64 = 1024 * 1024 * 1024;
const DECODE_CONCURRENCY: usize = 2;

struct ImportBudget {
    max_files: usize,
    max_encoded_bytes: u64,
    max_decoded_rgba_bytes: u64,
}

fn import_budget() -> ImportBudget {
    #[cfg(test)]
    if let Some(over) = IMPORT_BUDGET_OVERRIDE.with(|cell| cell.get()) {
        return ImportBudget {
            max_files: over.max_files,
            max_encoded_bytes: over.max_encoded_bytes,
            max_decoded_rgba_bytes: over.max_decoded_rgba_bytes,
        };
    }
    ImportBudget {
        max_files: MAX_IMPORT_FILES,
        max_encoded_bytes: MAX_IMPORT_ENCODED_BYTES,
        max_decoded_rgba_bytes: MAX_IMPORT_DECODED_RGBA_BYTES,
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct ImportBudgetOverride {
    max_files: usize,
    max_encoded_bytes: u64,
    max_decoded_rgba_bytes: u64,
}

#[cfg(test)]
impl ImportBudgetOverride {
    const DEFAULTS: Self = Self {
        max_files: MAX_IMPORT_FILES,
        max_encoded_bytes: MAX_IMPORT_ENCODED_BYTES,
        max_decoded_rgba_bytes: MAX_IMPORT_DECODED_RGBA_BYTES,
    };
}

#[cfg(test)]
thread_local! {
    static IMPORT_BUDGET_OVERRIDE: std::cell::Cell<Option<ImportBudgetOverride>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn import_budget_override(over: Option<ImportBudgetOverride>) {
    IMPORT_BUDGET_OVERRIDE.with(|cell| cell.set(over));
}

// Decode-concurrency gauge: observability only. RED measures the unbounded
// rayon admission; GREEN pins it at DECODE_CONCURRENCY via a dedicated pool.
#[cfg(test)]
static DECODE_IN_FLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static DECODE_IN_FLIGHT_MAX: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn reset_decode_gauge() {
    DECODE_IN_FLIGHT_MAX.store(0, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
fn decode_in_flight_max() -> usize {
    DECODE_IN_FLIGHT_MAX.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
struct DecodeGauge;

#[cfg(test)]
impl DecodeGauge {
    fn enter() -> Self {
        use std::sync::atomic::Ordering::SeqCst;
        let now = DECODE_IN_FLIGHT.fetch_add(1, SeqCst) + 1;
        DECODE_IN_FLIGHT_MAX.fetch_max(now, SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for DecodeGauge {
    fn drop(&mut self) {
        DECODE_IN_FLIGHT.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn decode_pool() -> &'static rayon::ThreadPool {
    static DECODE_POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    DECODE_POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(DECODE_CONCURRENCY)
            .build()
            .expect("decode pool")
    })
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

    // Collect (filename, bytes) pairs first so we can sort naturally. The
    // AMEND-02 batch budget is enforced while collecting — before any decode,
    // blob write, or scene mutation.
    let budget = import_budget();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut replace = false;
    let mut encoded_total: u64 = 0;
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
        encoded_total += bytes.len() as u64;
        if files.len() + 1 > budget.max_files {
            return Err(ApiError::new(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "import budget exceeded: {} files > {}",
                    files.len() + 1,
                    budget.max_files
                ),
            ));
        }
        if encoded_total > budget.max_encoded_bytes {
            return Err(ApiError::new(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "import budget exceeded: {encoded_total} encoded bytes > {}",
                    budget.max_encoded_bytes
                ),
            ));
        }
        files.push((filename, bytes.to_vec()));
    }

    files.sort_by(|a, b| natord::compare(&a.0, &b.0));

    // Admit every source image before any mutation. A rejected
    // format leaves the project unchanged. Bytes are preserved
    // for blob storage after admission passes. Decoding runs on a
    // dedicated pool so at most DECODE_CONCURRENCY images decode at once.
    if files.is_empty() {
        return Err(ApiError::bad_request("no files in request"));
    }
    let admitted: Vec<(String, u32, u32, Vec<u8>)> = tokio::task::spawn_blocking(move || {
        decode_pool().install(|| {
            files
                .into_par_iter()
                .map(
                    |(filename, bytes)| -> ApiResult<(String, u32, u32, Vec<u8>)> {
                        #[cfg(test)]
                        let _gauge = DecodeGauge::enter();
                        let img = koharu_app::blobs::admit_source_image(&bytes).map_err(|e| {
                            ApiError::bad_request(format!("admit `{filename}`: {e}"))
                        })?;
                        let (w, h) = img.dimensions();
                        Ok((filename, w, h, bytes))
                    },
                )
                .collect::<ApiResult<Vec<_>>>()
        })
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::anyhow!("import task panicked: {e}")))??;

    let decoded_total: u64 = admitted
        .iter()
        .map(|(_, w, h, _)| u64::from(*w) * u64::from(*h) * 4)
        .sum();
    if decoded_total > budget.max_decoded_rgba_bytes {
        return Err(ApiError::new(
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "import budget exceeded: {decoded_total} decoded RGBA bytes > {}",
                budget.max_decoded_rgba_bytes
            ),
        ));
    }

    // All files admitted — now safe to mutate the project.
    let mut ops: Vec<Op> = Vec::new();
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
        ops.extend(remove_ops);
        0
    } else {
        session.scene.read().pages.len()
    };

    // Store blobs from admitted bytes, then build pages.
    let blobs2 = session.blobs.clone();
    let decoded: Vec<(String, u32, u32, BlobRef)> = tokio::task::spawn_blocking(move || {
        admitted
            .into_par_iter()
            .map(
                |(filename, w, h, bytes)| -> ApiResult<(String, u32, u32, BlobRef)> {
                    let blob = blobs2.put_bytes(&bytes).map_err(ApiError::internal)?;
                    Ok((filename, w, h, blob))
                },
            )
            .collect::<ApiResult<Vec<_>>>()
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::anyhow!("import task panicked: {e}")))??;

    // Build AddPage ops into the combined batch.
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

    if paths.is_empty() && req.replace {
        // Replace with empty is just a clear — no admission needed.
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
        return Ok(Json(CreatePagesResponse { pages: Vec::new() }));
    }
    if paths.is_empty() {
        return Err(ApiError::bad_request("no paths provided"));
    }

    // Admit every source image before any mutation.
    let admitted: Vec<(String, u32, u32, Vec<u8>)> = tokio::task::spawn_blocking(move || {
        paths
            .into_par_iter()
            .map(|path| -> ApiResult<(String, u32, u32, Vec<u8>)> {
                let filename = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "page.bin".to_string());
                let bytes = std::fs::read(&path)
                    .map_err(|e| ApiError::bad_request(format!("read `{filename}`: {e}")))?;
                let img = koharu_app::blobs::admit_source_image(&bytes)
                    .map_err(|e| ApiError::bad_request(format!("admit `{filename}`: {e}")))?;
                let (w, h) = img.dimensions();
                Ok((filename, w, h, bytes))
            })
            .collect::<ApiResult<Vec<_>>>()
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::anyhow!("import task panicked: {e}")))??;

    // All files admitted — now safe to mutate the project.
    let mut ops: Vec<Op> = Vec::new();
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
        ops.extend(remove_ops);
        0
    } else {
        session.scene.read().pages.len()
    };

    // Store blobs from admitted bytes.
    let blobs = session.blobs.clone();
    let decoded: Vec<(String, u32, u32, BlobRef)> = tokio::task::spawn_blocking(move || {
        admitted
            .into_par_iter()
            .map(
                |(filename, w, h, bytes)| -> ApiResult<(String, u32, u32, BlobRef)> {
                    let blob = blobs.put_bytes(&bytes).map_err(ApiError::internal)?;
                    Ok((filename, w, h, blob))
                },
            )
            .collect::<ApiResult<Vec<_>>>()
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::anyhow!("import task panicked: {e}")))??;

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
    use axum::body::Body;
    use axum::extract::FromRequest;
    use axum::http::{Request, header::CONTENT_TYPE};
    use camino::Utf8PathBuf;
    use koharu_app::{App, AppConfig, ProjectSession, config::SourceTextPolicy};
    use koharu_core::Scene;
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use uuid::Uuid;

    struct TestDir(std::path::PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Clone)]
    struct ImportFile {
        name: &'static str,
        bytes: Option<Vec<u8>>,
    }

    #[derive(Clone)]
    enum ImportIngress {
        Paths,
        Multipart { malformed: bool },
    }

    struct ImportCase {
        name: &'static str,
        ingress: ImportIngress,
        files: Vec<ImportFile>,
        succeeds: bool,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ImportSnapshot {
        scene: Vec<u8>,
        page_order: Vec<PageId>,
        epoch: u64,
        history: Vec<u8>,
        scene_blob_refs: Vec<String>,
        blob_files: Vec<String>,
    }

    fn encoded_image(format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 1)
            .write_to(&mut bytes, format)
            .unwrap();
        bytes.into_inner()
    }

    fn overbudget_png() -> Vec<u8> {
        let mut bytes = encoded_image(image::ImageFormat::Png);
        bytes[16..20].copy_from_slice(&20_000_u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&20_000_u32.to_be_bytes());
        let mut crc = 0xffff_ffff_u32;
        for byte in &bytes[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
        bytes
    }

    fn multipart_body(files: &[ImportFile], malformed: bool) -> (String, Vec<u8>) {
        let boundary = "koharu-g002-boundary";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"replace\"\r\n\r\ntrue\r\n"
        )
        .into_bytes();
        for file in files {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
                    file.name
                )
                .as_bytes(),
            );
            if let Some(bytes) = &file.bytes {
                body.extend_from_slice(bytes);
            }
            body.extend_from_slice(b"\r\n");
        }
        if !malformed {
            body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        }
        (boundary.into(), body)
    }

    fn scene_blob_refs(scene: &Scene) -> Vec<String> {
        let mut refs = scene
            .pages
            .values()
            .flat_map(|page| page.nodes.values())
            .filter_map(|node| match &node.kind {
                NodeKind::Image(image) => Some(image.blob.hash().to_string()),
                NodeKind::Mask(mask) => Some(mask.blob.hash().to_string()),
                NodeKind::Text(_) => None,
            })
            .collect::<Vec<_>>();
        refs.sort();
        refs
    }

    fn blob_files(root: &std::path::Path) -> Vec<String> {
        fn visit(root: &std::path::Path, path: &std::path::Path, files: &mut Vec<String>) {
            let Ok(entries) = std::fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else if let Ok(relative) = path.strip_prefix(root) {
                    files.push(relative.to_string_lossy().into_owned());
                }
            }
        }
        let mut files = Vec::new();
        visit(root, root, &mut files);
        files.sort();
        files
    }

    fn snapshot(session: &ProjectSession, project_dir: &camino::Utf8Path) -> ImportSnapshot {
        let scene = session.scene.read();
        ImportSnapshot {
            scene: postcard::to_allocvec(&*scene).unwrap(),
            page_order: scene.pages.keys().copied().collect(),
            epoch: session.epoch(),
            history: std::fs::read(project_dir.join("history.log")).unwrap(),
            scene_blob_refs: scene_blob_refs(&scene),
            blob_files: blob_files(project_dir.join("blobs").as_std_path()),
        }
    }

    fn seeded_session(
        root: &std::path::Path,
        name: &str,
        source_bytes: &[u8],
    ) -> (Utf8PathBuf, Arc<ProjectSession>) {
        let project_dir = Utf8PathBuf::from_path_buf(root.join(format!("{name}.khrproj"))).unwrap();
        let session = ProjectSession::create(&project_dir, name).unwrap();
        let blob = session.blobs.put_bytes(source_bytes).unwrap();
        let mut page = Page::new("old.png", 2, 1);
        let id = NodeId::new();
        page.nodes.insert(
            id,
            Node {
                id,
                transform: Transform::default(),
                visible: true,
                kind: NodeKind::Image(ImageData {
                    role: ImageRole::Source,
                    blob,
                    opacity: 1.0,
                    natural_width: 2,
                    natural_height: 1,
                    name: Some("old.png".into()),
                }),
            },
        );
        session.apply(Op::AddPage { page, at: 0 }).unwrap();
        (project_dir, session)
    }

    async fn run_import(
        state: AppState,
        project_dir: &camino::Utf8Path,
        case: &ImportCase,
    ) -> Result<CreatePagesResponse, String> {
        match &case.ingress {
            ImportIngress::Paths => {
                let input_dir = project_dir.join("inputs");
                std::fs::create_dir_all(&input_dir).unwrap();
                let mut paths = Vec::new();
                for file in &case.files {
                    let path = input_dir.join(file.name);
                    if let Some(bytes) = &file.bytes {
                        std::fs::write(&path, bytes).unwrap();
                    }
                    paths.push(path.into_string());
                }
                create_pages_from_paths(
                    State(state),
                    Json(CreatePagesFromPathsRequest {
                        paths,
                        replace: true,
                    }),
                )
                .await
                .map(|Json(response)| response)
                .map_err(|error| error.message)
            }
            ImportIngress::Multipart { malformed } => {
                let (boundary, body) = multipart_body(&case.files, *malformed);
                let request = Request::builder()
                    .header(
                        CONTENT_TYPE,
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap();
                let multipart = Multipart::from_request(request, &state)
                    .await
                    .map_err(|error| error.to_string())?;
                create_pages(State(state), multipart)
                    .await
                    .map(|Json(response)| response)
                    .map_err(|error| error.message)
            }
        }
    }

    async fn budget_test_state(root: &std::path::Path) -> AppState {
        let runtime = RuntimeManager::new(root.join("runtime"), ComputePolicy::CpuOnly)
            .expect("create runtime");
        runtime.prepare().await.expect("prepare runtime");
        let runtime = Arc::new(runtime);
        let app = Arc::new(
            App::new(AppConfig::default(), runtime.clone(), true, "test").expect("create app"),
        );
        let state = crate::BootstrapManager::new(runtime);
        assert!(state.set_app(app).is_ok(), "set test app");
        state
    }

    fn png_files(names: &[&'static str]) -> Vec<ImportFile> {
        names
            .iter()
            .map(|name| ImportFile {
                name,
                bytes: Some(encoded_image(image::ImageFormat::Png)),
            })
            .collect()
    }

    async fn budget_import(
        case_name: &'static str,
        files: Vec<ImportFile>,
    ) -> (
        Result<CreatePagesResponse, String>,
        ImportSnapshot,
        ImportSnapshot,
    ) {
        let root =
            TestDir(std::env::temp_dir().join(format!("koharu-import-budget-{}", Uuid::new_v4())));
        std::fs::create_dir_all(&root.0).expect("create test root");
        let state = budget_test_state(&root.0).await;
        let png = encoded_image(image::ImageFormat::Png);
        let (project_dir, session) = seeded_session(&root.0, case_name, &png);
        state.app().unwrap().session.store(Some(session.clone()));
        let case = ImportCase {
            name: case_name,
            ingress: ImportIngress::Multipart { malformed: false },
            files,
            succeeds: true,
        };
        let before = snapshot(&session, &project_dir);
        let result = run_import(state.clone(), &project_dir, &case).await;
        let after = snapshot(&session, &project_dir);
        (result, before, after)
    }

    // AR05-T06 RED: batch import budget (AMEND-02) — file count, total encoded
    // bytes, and total decoded RGBA bytes are rejected before any mutation;
    // decode concurrency is pinned at 2. Every test clears this thread's
    // budget override first (pooled test threads can inherit one).
    #[tokio::test]
    async fn page_import_budget_rejects_over_file_count() {
        import_budget_override(None);
        import_budget_override(Some(ImportBudgetOverride {
            max_files: 2,
            ..ImportBudgetOverride::DEFAULTS
        }));
        let (result, before, after) =
            budget_import("over-file-count", png_files(&["a.png", "b.png", "c.png"])).await;
        import_budget_override(None);
        let error = result.expect_err("3 files over a 2-file budget must be rejected");
        assert!(error.contains("import budget"), "unexpected error: {error}");
        assert_eq!(
            after, before,
            "rejected import must leave zero side effects"
        );
    }

    #[tokio::test]
    async fn page_import_budget_rejects_over_encoded_bytes() {
        import_budget_override(None);
        let single = encoded_image(image::ImageFormat::Png).len() as u64;
        import_budget_override(Some(ImportBudgetOverride {
            max_encoded_bytes: single,
            ..ImportBudgetOverride::DEFAULTS
        }));
        let (result, before, after) =
            budget_import("over-encoded", png_files(&["a.png", "b.png"])).await;
        import_budget_override(None);
        let error = result.expect_err("encoded total over budget must be rejected");
        assert!(error.contains("import budget"), "unexpected error: {error}");
        assert_eq!(
            after, before,
            "rejected import must leave zero side effects"
        );
    }

    #[tokio::test]
    async fn page_import_budget_rejects_over_decoded_rgba() {
        import_budget_override(None);
        // The fixture PNG is 2x1 RGBA = 8 bytes decoded; two images = 16 > 8.
        import_budget_override(Some(ImportBudgetOverride {
            max_decoded_rgba_bytes: 8,
            ..ImportBudgetOverride::DEFAULTS
        }));
        let (result, before, after) =
            budget_import("over-decoded", png_files(&["a.png", "b.png"])).await;
        import_budget_override(None);
        let error = result.expect_err("decoded RGBA total over budget must be rejected");
        assert!(error.contains("import budget"), "unexpected error: {error}");
        assert_eq!(
            after, before,
            "rejected import must leave zero side effects"
        );
    }

    #[tokio::test]
    async fn page_import_budget_decode_concurrency_two() {
        import_budget_override(None);
        reset_decode_gauge();
        let names: Vec<&'static str> = (0..8).map(|_| "p.png").collect();
        let (result, _, _) = budget_import("decode-concurrency", png_files(&names)).await;
        result.expect("8 valid images must import");
        assert!(
            decode_in_flight_max() <= 2,
            "decode concurrency must stay within 2, observed {}",
            decode_in_flight_max()
        );
    }

    // Lock: a corrupt image in the batch fails the import with zero side
    // effects (pre-existing admission-before-mutation behavior).
    #[tokio::test]
    async fn page_import_budget_corrupt_image_zero_side_effects() {
        import_budget_override(None);
        let mut files = png_files(&["a.png"]);
        files.push(ImportFile {
            name: "bad.png",
            bytes: Some(b"not an image".to_vec()),
        });
        let (result, before, after) = budget_import("corrupt-mixed", files).await;
        assert!(result.is_err(), "corrupt image must fail the import");
        assert_eq!(after, before, "failed import must leave zero side effects");
    }

    // Lock: a small import under the default budgets succeeds.
    #[tokio::test]
    async fn page_import_budget_default_small_import_ok() {
        import_budget_override(None);
        let (result, _, _) = budget_import("small-ok", png_files(&["a.png", "b.png"])).await;
        let response = result.expect("small import must succeed");
        assert_eq!(response.pages.len(), 2);
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
    async fn hanonly_pre_b1_red_t2_replace_import_atomicity_contract() {
        let root = TestDir(
            std::env::temp_dir().join(format!("koharu-replace-atomicity-{}", Uuid::new_v4())),
        );
        std::fs::create_dir_all(&root.0).expect("create test root");
        let runtime = RuntimeManager::new(root.0.join("runtime"), ComputePolicy::CpuOnly)
            .expect("create runtime");
        runtime.prepare().await.expect("prepare runtime");
        let runtime = Arc::new(runtime);
        let app = Arc::new(
            App::new(AppConfig::default(), runtime.clone(), true, "test").expect("create app"),
        );
        let state = crate::BootstrapManager::new(runtime);
        assert!(state.set_app(app).is_ok(), "set test app");
        let png = encoded_image(image::ImageFormat::Png);
        let sorted = || {
            ["page-10.png", "page-2.png", "page-1.png"]
                .map(|name| ImportFile {
                    name,
                    bytes: Some(png.clone()),
                })
                .into()
        };
        let cases = [
            ImportCase {
                name: "path-valid-corrupt",
                ingress: ImportIngress::Paths,
                files: vec![
                    ImportFile {
                        name: "page-1.png",
                        bytes: Some(png.clone()),
                    },
                    ImportFile {
                        name: "page-2.png",
                        bytes: Some(b"not an image".to_vec()),
                    },
                ],
                succeeds: false,
            },
            ImportCase {
                name: "multipart-valid-corrupt",
                ingress: ImportIngress::Multipart { malformed: false },
                files: vec![
                    ImportFile {
                        name: "page-1.png",
                        bytes: Some(png.clone()),
                    },
                    ImportFile {
                        name: "page-2.png",
                        bytes: Some(b"not an image".to_vec()),
                    },
                ],
                succeeds: false,
            },
            ImportCase {
                name: "path-unsupported-gif",
                ingress: ImportIngress::Paths,
                files: vec![ImportFile {
                    name: "page.gif",
                    bytes: Some(encoded_image(image::ImageFormat::Gif)),
                }],
                succeeds: false,
            },
            ImportCase {
                name: "multipart-unsupported-bmp",
                ingress: ImportIngress::Multipart { malformed: false },
                files: vec![ImportFile {
                    name: "page.bmp",
                    bytes: Some(encoded_image(image::ImageFormat::Bmp)),
                }],
                succeeds: false,
            },
            ImportCase {
                name: "path-overbudget",
                ingress: ImportIngress::Paths,
                files: vec![ImportFile {
                    name: "huge.png",
                    bytes: Some(overbudget_png()),
                }],
                succeeds: false,
            },
            ImportCase {
                name: "multipart-overbudget",
                ingress: ImportIngress::Multipart { malformed: false },
                files: vec![ImportFile {
                    name: "huge.png",
                    bytes: Some(overbudget_png()),
                }],
                succeeds: false,
            },
            ImportCase {
                name: "path-unreadable",
                ingress: ImportIngress::Paths,
                files: vec![ImportFile {
                    name: "missing.png",
                    bytes: None,
                }],
                succeeds: false,
            },
            ImportCase {
                name: "multipart-unreadable",
                ingress: ImportIngress::Multipart { malformed: true },
                files: vec![ImportFile {
                    name: "truncated.png",
                    bytes: Some(png.clone()),
                }],
                succeeds: false,
            },
            ImportCase {
                name: "path-success-sort",
                ingress: ImportIngress::Paths,
                files: sorted(),
                succeeds: true,
            },
            ImportCase {
                name: "multipart-success-sort",
                ingress: ImportIngress::Multipart { malformed: false },
                files: sorted(),
                succeeds: true,
            },
        ];
        let mut violations = Vec::new();

        for case in &cases {
            let (project_dir, session) = seeded_session(&root.0, case.name, &png);
            state.app().unwrap().session.store(Some(session.clone()));
            let before = snapshot(&session, &project_dir);
            let result = run_import(state.clone(), &project_dir, case).await;
            let after = snapshot(&session, &project_dir);
            if !case.succeeds {
                if result.is_ok() {
                    violations.push(format!("{}: invalid replacement succeeded", case.name));
                }
                if after != before {
                    violations.push(format!(
                        "{}: failed replacement changed Scene/order/epoch/history/blob refs",
                        case.name
                    ));
                }
                if !matches!(session.undo(), Ok(Some(_))) || !session.scene.read().pages.is_empty()
                {
                    violations.push(format!(
                        "{}: first undo did not target the pre-existing seed op",
                        case.name
                    ));
                }
                if !matches!(session.redo(), Ok(Some(_)))
                    || postcard::to_allocvec(&*session.scene.read()).unwrap() != before.scene
                {
                    violations.push(format!(
                        "{}: redo did not restore the exact pre-import Scene",
                        case.name
                    ));
                }
                if session.redo().unwrap().is_some() {
                    violations.push(format!("{}: hidden redo item remained", case.name));
                }
                continue;
            }

            let response = match result {
                Ok(response) => response,
                Err(error) => {
                    violations.push(format!("{}: valid replacement failed: {error}", case.name));
                    continue;
                }
            };
            let names = session
                .scene
                .read()
                .pages
                .values()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>();
            if names != ["page-1.png", "page-2.png", "page-10.png"] {
                violations.push(format!("{}: natural sort was {names:?}", case.name));
            }
            if response.pages.len() != 3 {
                violations.push(format!(
                    "{}: response returned {} page IDs",
                    case.name,
                    response.pages.len()
                ));
            }
            if after.epoch != before.epoch + 1 {
                violations.push(format!(
                    "{}: replacement must be one Batch epoch, {} -> {}",
                    case.name, before.epoch, after.epoch
                ));
            }
            let successful_scene = after.scene.clone();
            if !matches!(session.undo(), Ok(Some(_)))
                || postcard::to_allocvec(&*session.scene.read()).unwrap() != before.scene
            {
                violations.push(format!(
                    "{}: one undo did not restore the exact pre-import Scene",
                    case.name
                ));
            }
            if !matches!(session.redo(), Ok(Some(_)))
                || postcard::to_allocvec(&*session.scene.read()).unwrap() != successful_scene
            {
                violations.push(format!(
                    "{}: one redo did not restore the replacement Scene",
                    case.name
                ));
            }
            if session.redo().unwrap().is_some() {
                violations.push(format!("{}: hidden redo item remained", case.name));
            }
        }

        assert!(
            violations.is_empty(),
            "G002 replace contract violations:\n{}",
            violations.join("\n")
        );
    }
}
