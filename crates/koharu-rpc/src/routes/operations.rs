//! Operations registry endpoints.
//!
//! - `GET /operations` — snapshot of every in-flight + recently-completed
//!   pipeline job, including the latest progress tick. Clients poll this
//!   endpoint while running jobs are expected; React Query drives the
//!   cadence on the UI side.
//! - `DELETE /operations/{id}` — unified cancel. Pipeline cancellation
//!   flips the cancel flag registered at start time; download cancellation
//!   is best-effort (HF hub transfers don't expose mid-stream cancel) and
//!   just evicts the row so the UI clears it.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use koharu_core::JobSummary;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use dashmap::DashMap;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::AppState;
use crate::error::ApiResult;

static CANCELS: OnceLock<DashMap<String, Arc<AtomicBool>>> = OnceLock::new();
fn cancels() -> &'static DashMap<String, Arc<AtomicBool>> {
    CANCELS.get_or_init(DashMap::new)
}

/// Register a cancel flag for an operation id. Called by `pipelines::start_pipeline`.
pub fn register_cancel(id: String, flag: Arc<AtomicBool>) {
    cancels().insert(id, flag);
}

/// Drop a cancel flag once the operation has finished.
pub fn unregister_cancel(id: &str) {
    cancels().remove(id);
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .routes(routes!(list_operations))
        .routes(routes!(cancel_operation))
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListOperationsResponse {
    pub operations: Vec<JobSummary>,
}

#[utoipa::path(
    get,
    path = "/operations",
    responses((status = 200, body = ListOperationsResponse))
)]
async fn list_operations(State(app): State<AppState>) -> ApiResult<Json<ListOperationsResponse>> {
    let jobs = app.jobs();
    let operations = jobs.iter().map(|e| e.value().clone()).collect();
    Ok(Json(ListOperationsResponse { operations }))
}

#[utoipa::path(
    delete,
    path = "/operations/{id}",
    params(("id" = String, Path, description = "Operation id")),
    responses((status = 204))
)]
async fn cancel_operation(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    if let Some(flag) = cancels().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
    // Best-effort download cancel: drop the registry row.
    app.downloads().remove(&id);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod job_registry_tests {
    use super::*;
    use koharu_app::{App, AppConfig};
    use koharu_core::JobStatus;
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use std::collections::BTreeSet;

    fn test_state() -> AppState {
        let runtime = RuntimeManager::new(
            koharu_runtime::default_app_data_root().into_std_path_buf(),
            ComputePolicy::CpuOnly,
        )
        .expect("create runtime");
        let runtime = Arc::new(runtime);
        let app =
            Arc::new(App::new(AppConfig::default(), runtime.clone(), true, "test").expect("app"));
        let state = crate::BootstrapManager::new(runtime);
        assert!(state.set_app(app).is_ok(), "set test app");
        state
    }

    fn insert_completed(state: &AppState, count: usize) {
        let jobs = state.jobs();
        for i in 0..count {
            let id = format!("done-{i}");
            jobs.insert(
                id.clone(),
                JobSummary {
                    id,
                    kind: "test".into(),
                    status: JobStatus::Completed,
                    error: None,
                },
            );
        }
    }

    fn id_set(jobs: &[JobSummary]) -> BTreeSet<String> {
        jobs.iter().map(|job| job.id.clone()).collect()
    }

    // AR06-T02 lock: SSE snapshot, operations list, and MCP lookup all read
    // the same bounded registry — identical id sets at the eviction boundary.
    #[tokio::test]
    async fn job_registry_three_entries_consistent_at_eviction_boundary() {
        let state = test_state();
        insert_completed(&state, 260);

        let sse_snapshot = crate::events::snapshot_from(&state);
        let sse_ids = id_set(&sse_snapshot.jobs);
        let operations = list_operations(State(state.clone()))
            .await
            .expect("list operations")
            .0
            .operations;
        let ops_ids = id_set(&operations);
        let registry_ids: BTreeSet<String> = state
            .jobs()
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        assert_eq!(
            sse_ids, registry_ids,
            "SSE snapshot must mirror the registry"
        );
        assert_eq!(
            ops_ids, registry_ids,
            "operations list must mirror the registry"
        );
        assert_eq!(registry_ids.len(), 256, "registry must be bounded");
        assert!(!registry_ids.contains("done-0"), "oldest completed evicted");
        assert!(registry_ids.contains("done-259"));

        // MCP lookup surface: every visible id resolves, evicted id does not.
        let jobs = state.jobs();
        assert!(crate::mcp::get_job_from_registry(&jobs, "done-259").is_some());
        assert!(crate::mcp::get_job_from_registry(&jobs, "done-0").is_none());
    }

    // Lock: unknown lookup is an error and never creates a registry entry.
    #[tokio::test]
    async fn job_registry_unknown_lookup_creates_nothing() {
        let state = test_state();
        let jobs = state.jobs();
        assert!(crate::mcp::get_job_from_registry(&jobs, "missing").is_none());
        assert!(jobs.is_empty(), "unknown lookup must not create a job");
    }
}
