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
                if text.typography_plan_verified.is_some()
                    || text.style.is_some()
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
        Err("typographyPlanVerified and style are internal planner fields")
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
pub(crate) mod tests {
    use super::*;
    use std::sync::Arc;

    use camino::Utf8PathBuf;
    use koharu_app::{App, AppConfig, ProjectSession};
    use koharu_core::{
        Node, NodeDataPatch, NodeId, NodeKind, NodePatch, Page, PageId, TextData, TextDataPatch,
        TextStyle, Transform,
    };
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use uuid::Uuid;

    pub(crate) const MARKER_ERROR: &str =
        "typographyPlanVerified and style are internal planner fields";

    pub(crate) struct T3MarkerCase {
        pub name: &'static str,
        pub raw: Value,
        pub reject: bool,
    }

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

    pub(crate) fn t3_marker_cases(page: &Page, node_id: NodeId) -> Vec<T3MarkerCase> {
        let page_id = page.id;
        let update = || Op::UpdateNode {
            page: page_id,
            id: node_id,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    text: Some(Some("accepted".into())),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        };
        let update_with_marker = |marker| {
            let mut raw = serde_json::to_value(update()).unwrap();
            *raw.pointer_mut("/updateNode/patch/data/text/typographyPlanVerified")
                .unwrap() = marker;
            raw
        };
        let add_page_with_marker = |marker| {
            let node = text_node(false);
            let mut added = Page::new("add-page", 10, 10);
            added.nodes.insert(node.id, node);
            let mut raw = serde_json::to_value(Op::AddPage { page: added, at: 1 }).unwrap();
            *raw.pointer_mut("/addPage/page/nodes")
                .and_then(Value::as_object_mut)
                .unwrap()
                .values_mut()
                .next()
                .unwrap()
                .pointer_mut("/kind/text/typographyPlanVerified")
                .unwrap() = marker;
            raw
        };
        let add_node_with_marker = |marker| {
            let mut raw = serde_json::to_value(Op::AddNode {
                page: page_id,
                node: text_node(false),
                at: page.nodes.len(),
            })
            .unwrap();
            *raw.pointer_mut("/addNode/node/kind/text/typographyPlanVerified")
                .unwrap() = marker;
            raw
        };
        let nested_string = (0..3).fold(
            update_with_marker(json!("sentinel")),
            |raw, depth| json!({"batch": {"ops": [raw], "label": format!("depth-{depth}")}}),
        );

        let mut omitted = serde_json::to_value(update()).unwrap();
        omitted
            .pointer_mut("/updateNode/patch/data/text")
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove("typographyPlanVerified");

        let mut update_inverse = serde_json::to_value(update()).unwrap();
        update_inverse
            .pointer_mut("/updateNode/patch/data/text")
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove("typographyPlanVerified");
        update_inverse["updateNode"]["prev"] = json!({
            "data": {
                "text": {
                    "typographyPlanVerified": true
                }
            }
        });

        let previous_node = page.nodes.get(&node_id).unwrap().clone();
        let mut remove_node = serde_json::to_value(Op::RemoveNode {
            page: page_id,
            id: node_id,
            prev_node: previous_node,
            prev_index: page.nodes.get_index_of(&node_id).unwrap(),
        })
        .unwrap();
        *remove_node
            .pointer_mut("/removeNode/prev_node/kind/text/typographyPlanVerified")
            .unwrap() = json!(true);

        let mut remove_page = serde_json::to_value(Op::RemovePage {
            id: page_id,
            prev_page: page.clone(),
            prev_index: 0,
        })
        .unwrap();
        *remove_page
            .pointer_mut("/removePage/prev_page/nodes")
            .and_then(Value::as_object_mut)
            .unwrap()
            .values_mut()
            .find(|node| node.pointer("/kind/text").is_some())
            .unwrap()
            .pointer_mut("/kind/text/typographyPlanVerified")
            .unwrap() = json!(true);

        vec![
            T3MarkerCase {
                name: "omitted forward marker",
                raw: omitted,
                reject: false,
            },
            T3MarkerCase {
                name: "updateNode inverse marker",
                raw: update_inverse,
                reject: false,
            },
            T3MarkerCase {
                name: "removeNode inverse marker",
                raw: remove_node,
                reject: false,
            },
            T3MarkerCase {
                name: "removePage inverse marker",
                raw: remove_page,
                reject: false,
            },
            T3MarkerCase {
                name: "three-level batch forward string",
                raw: nested_string,
                reject: true,
            },
            T3MarkerCase {
                name: "updateNode forward true",
                raw: update_with_marker(json!(true)),
                reject: true,
            },
            T3MarkerCase {
                name: "updateNode forward false",
                raw: update_with_marker(json!(false)),
                reject: true,
            },
            T3MarkerCase {
                name: "addPage forward true",
                raw: add_page_with_marker(json!(true)),
                reject: true,
            },
            T3MarkerCase {
                name: "addPage forward false",
                raw: add_page_with_marker(json!(false)),
                reject: false,
            },
            T3MarkerCase {
                name: "addNode forward true",
                raw: add_node_with_marker(json!(true)),
                reject: true,
            },
            T3MarkerCase {
                name: "addNode forward false",
                raw: add_node_with_marker(json!(false)),
                reject: false,
            },
        ]
    }

    pub(crate) fn mutation_state(session: &ProjectSession) -> (Value, u64, Vec<u8>) {
        let (epoch, scene) = session.scene_snapshot_with_epoch();
        (
            serde_json::to_value(scene).unwrap(),
            epoch,
            std::fs::read(session.dir.join("history.log")).unwrap(),
        )
    }

    pub(crate) fn has_verified_marker(session: &ProjectSession) -> bool {
        session.scene.read().pages.values().any(|page| {
            page.nodes.values().any(
                |node| matches!(&node.kind, NodeKind::Text(text) if text.typography_plan_verified),
            )
        })
    }

    fn http_app() -> (
        crate::AppState,
        Arc<ProjectSession>,
        Utf8PathBuf,
        PageId,
        NodeId,
    ) {
        let runtime = RuntimeManager::new(
            koharu_runtime::default_app_data_root().into_std_path_buf(),
            ComputePolicy::CpuOnly,
        )
        .expect("create runtime");
        let app =
            Arc::new(App::new(AppConfig::default(), Arc::new(runtime), true, "test").expect("app"));
        let root = std::env::temp_dir().join(format!("koharu-http-marker-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).expect("create test root");
        let root = Utf8PathBuf::from_path_buf(root).expect("UTF-8 test root");
        let session =
            ProjectSession::create(root.join("marker.khrproj"), "marker").expect("create session");
        let mut page = Page::new("existing", 10, 10);
        let page_id = page.id;
        let node = text_node(false);
        let node_id = node.id;
        page.nodes.insert(node_id, node);
        session
            .apply(Op::AddPage { page, at: 0 })
            .expect("seed page");
        app.session.store(Some(session.clone()));
        let state = crate::BootstrapManager::new(app.runtime.clone());
        assert!(state.set_app(app).is_ok(), "set app");
        (state, session, root, page_id, node_id)
    }

    async fn post_raw(addr: std::net::SocketAddr, raw: &Value) -> (u16, Value) {
        let body = serde_json::to_vec(raw).unwrap();
        let request = format!(
            "POST /api/v1/history/apply HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.write_all(&body).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let split = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let headers = std::str::from_utf8(&response[..split]).unwrap();
        let status = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let body = serde_json::from_slice(&response[split + 4..]).unwrap_or_else(|_| {
            Value::String(String::from_utf8_lossy(&response[split + 4..]).into())
        });
        (status, body)
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
            Op::UpdateNode {
                page: page_id,
                id: node_id,
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(TextDataPatch {
                        style: Some(Some(TextStyle::default())),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                prev: NodePatch::default(),
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
                    text: Some(Some("valid edit".into())),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        };
        assert!(validate_external_op(&allowed).is_ok());
    }

    #[tokio::test]
    async fn hanonly_pre_greenc_red_t3_http_marker_rejection_contract() {
        let (state, session, root, page_id, node_id) = http_app();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(crate::server::serve_with_listener(listener, state));

        let page = session.scene.read().pages.get(&page_id).unwrap().clone();
        for case in t3_marker_cases(&page, node_id) {
            let before = mutation_state(&session);
            let (status, body) = post_raw(addr, &case.raw).await;
            if case.reject {
                assert!(
                    status == 400 || status == 422,
                    "{}: expected 400 or 422, got {status}",
                    case.name
                );
                if status == 400 {
                    assert_eq!(body["message"], MARKER_ERROR, "{}", case.name);
                }
                assert_eq!(mutation_state(&session), before, "{}", case.name);
            } else {
                if status == 200 {
                    assert_eq!(session.epoch(), before.1 + 1, "{}", case.name);
                } else {
                    assert_eq!(mutation_state(&session), before, "{}", case.name);
                }
                assert!(!has_verified_marker(&session), "{}", case.name);
            }
        }
        server.abort();
        let _ = server.await;
        drop(session);
        std::fs::remove_dir_all(root).unwrap();
    }
}
