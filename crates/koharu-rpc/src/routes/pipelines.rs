//! `POST /pipelines` — start a pipeline run as a long-running operation.
//!
//! Returns an `operationId`. Progress + completion flow through SSE
//! (`JobStarted` / `JobProgress` / `JobFinished`). Cancellation goes to
//! `DELETE /operations/{id}`.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use koharu_app::AppConfig;
use koharu_app::pipeline::{
    self, PipelineRunOptions, PipelineSpec, ProgressTick, Scope, WarningTick,
};
use koharu_core::{
    AppEvent, JobFinishedEvent, JobStatus, JobSummary, JobWarningEvent, NodeId, PageId,
    PipelineProgress, PipelineStatus, ReadingOrder, Region,
};
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use crate::routes::operations::{register_cancel, unregister_cancel};

/// Try to take the pipeline admission slot for the session's project.
/// One slot per project; the returned permit releases on drop (RAII),
/// including when the owning task panics.
pub(crate) fn try_acquire_pipeline_slot(
    app: &koharu_app::App,
    session: &koharu_app::ProjectSession,
) -> anyhow::Result<tokio::sync::OwnedSemaphorePermit> {
    let slot = app
        .pipeline_slots
        .entry(session.dir.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone();
    slot.try_acquire_owned()
        .map_err(|_| anyhow::anyhow!("pipeline slot busy for project {}", session.dir))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default().routes(routes!(start_pipeline))
}

fn options_from_request(req: &StartPipelineRequest, config: &AppConfig) -> PipelineRunOptions {
    PipelineRunOptions {
        source_text_policy: config.pipeline.source_text_policy,
        target_language: req.target_language.clone(),
        system_prompt: req.system_prompt.clone(),
        default_font: req.default_font.clone(),
        text_node_ids: req.text_node_ids.clone(),
        region: req.region,
        reading_order: req.reading_order,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartPipelineRequest {
    /// Engine ids (`inventory::submit!` ids) to run in order.
    pub steps: Vec<String>,
    /// `None` → whole project, `Some(pages)` → just those pages.
    #[serde(default)]
    pub pages: Option<Vec<PageId>>,
    /// Optional bounding-box hint for inpainter engines (repair-brush).
    #[serde(default)]
    pub region: Option<Region>,
    /// Optional text-node ids for engines that can operate on individual blocks.
    #[serde(default)]
    pub text_node_ids: Option<Vec<NodeId>>,
    #[serde(default)]
    pub target_language: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub default_font: Option<String>,
    #[serde(default)]
    pub reading_order: Option<ReadingOrder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartPipelineResponse {
    pub operation_id: String,
}

#[utoipa::path(
    post,
    path = "/pipelines",
    request_body = StartPipelineRequest,
    responses(
        (status = 200, body = StartPipelineResponse),
        (status = 429, description = "a pipeline is already running for this project")
    )
)]
async fn start_pipeline(
    State(app): State<AppState>,
    Json(req): Json<StartPipelineRequest>,
) -> ApiResult<axum::response::Response> {
    let session = app
        .current_session()
        .ok_or_else(|| ApiError::bad_request("no project open"))?;
    let options = options_from_request(&req, &app.config.load());
    let spec = PipelineSpec {
        scope: match req.pages {
            Some(pages) => Scope::Pages(pages),
            None => Scope::WholeProject,
        },
        steps: req.steps,
        options,
    };

    let operation_id = match spawn_pipeline_job(app.as_ref(), session, spec) {
        Ok(id) => id,
        Err(error) => {
            let message = format!("{error:#}");
            if message.contains("pipeline slot busy") {
                return Ok((
                    StatusCode::TOO_MANY_REQUESTS,
                    [(axum::http::header::RETRY_AFTER, "1")],
                    Json(crate::ApiError::new(StatusCode::TOO_MANY_REQUESTS, message)),
                )
                    .into_response());
            }
            return Err(ApiError::bad_request(message));
        }
    };
    Ok(Json(StartPipelineResponse { operation_id }).into_response())
}

/// Start a fully registered pipeline job. HTTP and MCP intentionally share
/// this lifecycle so both expose the same id, warnings, cancellation and
/// completion records.
pub(crate) fn spawn_pipeline_job(
    app: &koharu_app::App,
    session: Arc<koharu_app::ProjectSession>,
    spec: PipelineSpec,
) -> anyhow::Result<String> {
    for id in &spec.steps {
        pipeline::Registry::find(id)?;
    }

    // Admission precedes any registry/event side effect; the permit is moved
    // into the job task and released when it ends, including on panic.
    let permit = try_acquire_pipeline_slot(app, &session)?;

    let operation_id = Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    register_cancel(operation_id.clone(), cancel.clone());
    app.jobs.insert(
        operation_id.clone(),
        JobSummary {
            id: operation_id.clone(),
            kind: "pipeline".to_string(),
            status: JobStatus::Running,
            error: None,
        },
    );
    app.bus.publish(AppEvent::JobStarted {
        id: operation_id.clone(),
        kind: "pipeline".to_string(),
    });

    // Detach the pipeline. Progress writes directly into the jobs registry;
    // clients observe via SSE.
    let session_c = session.clone();
    let op_id_c = operation_id.clone();
    let registry_c = app.registry.clone();
    let runtime_c = app.runtime.clone();
    let llm_c = app.llm.clone();
    let renderer_c = app.renderer.clone();
    let typography_planner_c = app.typography_planner.clone();
    let jobs_c = app.jobs.clone();
    let finished_bus = app.bus.clone();
    let cpu = app.cpu_only();
    let progress_bus = app.bus.clone();
    let progress_op_id = operation_id.clone();
    let progress_sink: pipeline::ProgressSink = Arc::new(move |tick: ProgressTick| {
        progress_bus.publish(AppEvent::JobProgress(PipelineProgress {
            job_id: progress_op_id.clone(),
            status: PipelineStatus::Running,
            step: tick.step,
            current_page: tick.page_index,
            total_pages: tick.total_pages,
            current_step_index: tick.step_index,
            total_steps: tick.total_steps,
            overall_percent: tick.overall_percent,
        }));
    });
    let warning_bus = app.bus.clone();
    let warning_jobs = app.jobs.clone();
    let warning_op_id = operation_id.clone();
    let warning_sink: pipeline::WarningSink = Arc::new(move |tick: WarningTick| {
        let message = tick.message.clone();
        if let Some(mut job) = warning_jobs.get_mut(&warning_op_id) {
            job.error = Some(message);
        }
        warning_bus.publish(AppEvent::JobWarning(JobWarningEvent {
            job_id: warning_op_id.clone(),
            page_index: tick.page_index,
            total_pages: tick.total_pages,
            step_id: tick.step_id,
            message: tick.message,
        }));
    });
    tokio::spawn(async move {
        let _permit = permit;
        let result = pipeline::run(
            session_c,
            registry_c,
            runtime_c,
            cpu,
            llm_c,
            renderer_c,
            typography_planner_c,
            spec,
            cancel,
            Some(progress_sink),
            Some(warning_sink),
        )
        .await;
        let (status, error) = match &result {
            Ok(outcome) if outcome.warning_count == 0 => (JobStatus::Completed, None),
            Ok(_) => (
                JobStatus::CompletedWithErrors,
                jobs_c.get(&op_id_c).and_then(|job| job.error.clone()),
            ),
            Err(e) if e.to_string().contains("cancelled") => (JobStatus::Cancelled, None),
            Err(e) => {
                tracing::warn!(operation_id = %op_id_c, "pipeline run failed: {e:#}");
                (JobStatus::Failed, Some(format!("{e:#}")))
            }
        };
        jobs_c.insert(
            op_id_c.clone(),
            JobSummary {
                id: op_id_c.clone(),
                kind: "pipeline".to_string(),
                status,
                error: error.clone(),
            },
        );
        finished_bus.publish(AppEvent::JobFinished(JobFinishedEvent {
            id: op_id_c.clone(),
            status,
            error,
        }));
        unregister_cancel(&op_id_c);
    });

    Ok(operation_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use koharu_app::{AppConfig, config::SourceTextPolicy};

    #[test]
    fn http_options_inherits_source_text_policy() {
        let mut config = AppConfig::default();
        config.pipeline.source_text_policy = SourceTextPolicy::AllText;
        let page = PageId::new();
        let req = StartPipelineRequest {
            steps: vec!["llm".to_string()],
            pages: Some(vec![page]),
            region: None,
            text_node_ids: None,
            target_language: Some("ja".to_string()),
            system_prompt: Some("system".to_string()),
            default_font: Some("Noto Sans".to_string()),
            reading_order: Some(ReadingOrder::Ltr),
        };

        let options = options_from_request(&req, &config);

        assert_eq!(options.source_text_policy, SourceTextPolicy::AllText);
        assert_eq!(options.target_language.as_deref(), Some("ja"));
        assert_eq!(req.steps, ["llm"]);
        assert_eq!(req.pages.as_deref(), Some([page].as_slice()));
    }
}

#[cfg(test)]
mod pipeline_admission_tests {
    use super::*;
    use koharu_app::{App, AppConfig, ProjectSession};
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use uuid::Uuid;

    async fn test_state() -> (AppState, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("koharu-pipeline-admission-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create test root");
        let runtime = RuntimeManager::new(root.join("runtime"), ComputePolicy::CpuOnly)
            .expect("create runtime");
        runtime.prepare().await.expect("prepare runtime");
        let runtime = Arc::new(runtime);
        let app = Arc::new(
            App::new(AppConfig::default(), runtime.clone(), true, "test").expect("create app"),
        );
        let state = crate::BootstrapManager::new(runtime);
        assert!(state.set_app(app).is_ok(), "set test app");
        (state, root)
    }

    fn open_session(state: &AppState, root: &std::path::Path) -> Arc<ProjectSession> {
        let dir = camino::Utf8PathBuf::from_path_buf(root.join("proj.khrproj")).unwrap();
        let session = ProjectSession::create(&dir, "admission").expect("create session");
        state.app().unwrap().session.store(Some(session.clone()));
        session
    }

    fn pipeline_request() -> StartPipelineRequest {
        StartPipelineRequest {
            steps: vec!["lama-manga".to_string()],
            pages: None,
            region: None,
            text_node_ids: None,
            target_language: None,
            system_prompt: None,
            default_font: None,
            reading_order: None,
        }
    }

    // AR06-T03 RED: a second concurrent pipeline on the same project must be
    // rejected 429 + Retry-After while the first holds the slot.
    #[tokio::test]
    async fn pipeline_admission_second_concurrent_gets_429() {
        let (state, root) = test_state().await;
        let session = open_session(&state, &root);
        let _first = try_acquire_pipeline_slot(state.app().unwrap().as_ref(), &session)
            .expect("first slot acquires");

        let response = start_pipeline(State(state.clone()), Json(pipeline_request()))
            .await
            .expect("handler responds");

        let status = response.status();
        let retry_after = response
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        drop(session);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(status, 429, "second concurrent pipeline must be rejected");
        assert_eq!(retry_after.as_deref(), Some("1"));
    }

    // Lock: the slot frees when the permit drops.
    #[tokio::test]
    async fn pipeline_admission_slot_released_on_permit_drop() {
        let (state, root) = test_state().await;
        let session = open_session(&state, &root);
        let app = state.app().unwrap();
        let permit = try_acquire_pipeline_slot(app.as_ref(), &session).expect("first acquire");
        assert!(
            try_acquire_pipeline_slot(app.as_ref(), &session).is_err(),
            "second acquire while held must fail"
        );
        drop(permit);
        assert!(
            try_acquire_pipeline_slot(app.as_ref(), &session).is_ok(),
            "acquire after release must succeed"
        );
        drop(session);
        let _ = std::fs::remove_dir_all(&root);
    }
}
