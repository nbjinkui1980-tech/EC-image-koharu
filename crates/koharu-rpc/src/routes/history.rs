//! Scene mutation routes — the only way a client changes the scene.
//!
//! - `POST /history/apply` — apply an `Op` (including `Op::Batch`)
//! - `POST /history/undo`  — revert the last applied op
//! - `POST /history/redo`  — re-apply the last undone op
//!
//! Three distinct sub-resource actions under `/history` (Stripe-style
//! named-action URLs). Each returns `{ epoch }` — populated if the action
//! advanced the scene, `None` for a no-op boundary.

use axum::Json;
use axum::extract::State;
use koharu_core::Op;
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::AppState;
use crate::error::{ApiError, ApiResult};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .routes(routes!(apply_command))
        .routes(routes!(undo))
        .routes(routes!(redo))
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HistoryResult {
    /// New epoch. `None` only for a no-op undo/redo at the stack boundary.
    pub epoch: Option<u64>,
}

#[utoipa::path(
    post,
    path = "/history/apply",
    request_body = Op,
    responses((status = 200, body = HistoryResult))
)]
async fn apply_command(
    State(app): State<AppState>,
    Json(op): Json<Op>,
) -> ApiResult<Json<HistoryResult>> {
    validate_external_op(&op).map_err(ApiError::bad_request)?;
    let epoch = app.apply(op).map_err(ApiError::internal)?;
    Ok(Json(HistoryResult { epoch: Some(epoch) }))
}

pub(crate) fn validate_external_op(op: &Op) -> Result<(), &'static str> {
    let forged = match op {
        Op::AddPage { page, .. } => page.nodes.values().any(|node| {
            matches!(&node.kind, koharu_core::NodeKind::Text(text) if text.typography_plan_verified)
        }),
        Op::AddNode { node, .. } => {
            matches!(&node.kind, koharu_core::NodeKind::Text(text) if text.typography_plan_verified)
        }
        Op::UpdateNode { patch, .. } => matches!(
            &patch.data,
            Some(koharu_core::NodeDataPatch::Text(text))
                if text.typography_plan_verified == Some(true)
        ),
        Op::Batch { ops, .. } => {
            for child in ops {
                validate_external_op(child)?;
            }
            false
        }
        _ => false,
    };
    if forged {
        Err("typographyPlanVerified is internal and cannot be set by external operations")
    } else {
        Ok(())
    }
}

#[utoipa::path(post, path = "/history/undo", responses((status = 200, body = HistoryResult)))]
async fn undo(State(app): State<AppState>) -> ApiResult<Json<HistoryResult>> {
    let epoch = app.undo().map_err(ApiError::internal)?;
    Ok(Json(HistoryResult { epoch }))
}

#[utoipa::path(post, path = "/history/redo", responses((status = 200, body = HistoryResult)))]
async fn redo(State(app): State<AppState>) -> ApiResult<Json<HistoryResult>> {
    let epoch = app.redo().map_err(ApiError::internal)?;
    Ok(Json(HistoryResult { epoch }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_core::{
        Node, NodeDataPatch, NodeId, NodeKind, NodePatch, Page, PageId, TextData, TextDataPatch,
        Transform,
    };

    fn text_node(verified: bool) -> Node {
        Node {
            id: NodeId::new(),
            transform: Transform::default(),
            visible: true,
            kind: NodeKind::Text(TextData {
                typography_plan_verified: verified,
                ..Default::default()
            }),
        }
    }

    #[test]
    fn http_apply_rejects_forged_typography_plan_marker() {
        let page_id = PageId::new();
        let node_id = NodeId::new();
        let mut page = Page::new("forged", 10, 10);
        page.nodes.insert(node_id, text_node(true));
        let forged = [
            Op::AddPage { page, at: 0 },
            Op::AddNode {
                page: page_id,
                node: text_node(true),
                at: 0,
            },
            Op::UpdateNode {
                page: page_id,
                id: node_id,
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(TextDataPatch {
                        typography_plan_verified: Some(true),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                prev: NodePatch::default(),
            },
            Op::Batch {
                ops: vec![Op::Batch {
                    ops: vec![Op::AddNode {
                        page: page_id,
                        node: text_node(true),
                        at: 0,
                    }],
                    label: "inner".into(),
                }],
                label: "outer".into(),
            },
        ];
        for op in forged {
            assert!(validate_external_op(&op).is_err());
        }

        let allowed = Op::UpdateNode {
            page: page_id,
            id: node_id,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    typography_plan_verified: Some(false),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        };
        assert!(validate_external_op(&allowed).is_ok());
    }
}
