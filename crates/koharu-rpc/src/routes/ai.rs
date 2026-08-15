//! AI workflow routes. These are separate from `/llm/*` because Codex image
//! generation is not a translation model lifecycle concern.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use futures::FutureExt;
use koharu_app::ai::{CodexAuthStatus, CodexDeviceLogin, CodexImageGenerationOptions};
use koharu_core::{AppEvent, JobFinishedEvent, JobStatus, JobSummary};
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use crate::routes::operations::{register_cancel, unregister_cancel};

/// Try to take one of the two global AI image-generation slots.
/// The returned permit releases on drop (RAII), including task panic.
pub(crate) fn try_acquire_ai_slot(
    app: &koharu_app::App,
) -> Result<tokio::sync::OwnedSemaphorePermit, ApiError> {
    app.ai_slots.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "ai slot busy: two image generations already in flight",
        )
    })
}

/// Terminal cleanup for an AI image-generation job: record the outcome in
/// the bounded registry, publish JobFinished, and unregister cancellation.
/// `Panic` exists so a panicking task still ends as Failed instead of
/// leaking a forever-Running job.
pub(crate) fn finish_ai_job(app: &koharu_app::App, operation_id: &str, outcome: AiJobOutcome) {
    let (status, error) = match outcome {
        AiJobOutcome::Completed => (JobStatus::Completed, None),
        AiJobOutcome::Cancelled => (JobStatus::Cancelled, None),
        AiJobOutcome::Failed(message) => (JobStatus::Failed, Some(message)),
        AiJobOutcome::Panic => (JobStatus::Failed, Some("ai task panicked".to_string())),
    };
    app.jobs.insert(
        operation_id.to_string(),
        JobSummary {
            id: operation_id.to_string(),
            kind: "ai".to_string(),
            status,
            error: error.clone(),
        },
    );
    app.bus.publish(AppEvent::JobFinished(JobFinishedEvent {
        id: operation_id.to_string(),
        status,
        error,
    }));
    unregister_cancel(operation_id);
}

#[derive(Debug)]
pub(crate) enum AiJobOutcome {
    Completed,
    Cancelled,
    Failed(String),
    Panic,
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .routes(routes!(get_codex_auth_status))
        .routes(routes!(start_codex_device_login))
        .routes(routes!(delete_codex_session))
        .routes(routes!(start_codex_image_generation))
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexImageGenerationResponse {
    pub operation_id: String,
}

#[utoipa::path(
    get,
    path = "/ai/codex/auth/status",
    responses((status = 200, body = CodexAuthStatus))
)]
async fn get_codex_auth_status(State(app): State<AppState>) -> ApiResult<Json<CodexAuthStatus>> {
    app.ai
        .codex_auth_status()
        .map(Json)
        .map_err(ApiError::internal)
}

#[utoipa::path(
    post,
    path = "/ai/codex/auth/device-code",
    responses((status = 200, body = CodexDeviceLogin))
)]
async fn start_codex_device_login(
    State(app): State<AppState>,
) -> ApiResult<Json<CodexDeviceLogin>> {
    app.ai
        .start_codex_device_login()
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

#[utoipa::path(delete, path = "/ai/codex/auth/session", responses((status = 204)))]
async fn delete_codex_session(State(app): State<AppState>) -> ApiResult<StatusCode> {
    app.ai.logout_codex().map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/ai/codex/images",
    request_body = CodexImageGenerationOptions,
    responses((status = 200, body = CodexImageGenerationResponse))
)]
async fn start_codex_image_generation(
    State(app): State<AppState>,
    Json(req): Json<CodexImageGenerationOptions>,
) -> ApiResult<Json<CodexImageGenerationResponse>> {
    let session = app
        .current_session()
        .ok_or_else(|| ApiError::bad_request("no project open"))?;
    // Admission before any registry/event side effect; the permit is moved
    // into the job task and released on completion, cancel, error, or panic.
    let permit = try_acquire_ai_slot(app.as_ref())?;

    let operation_id = Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    register_cancel(operation_id.clone(), cancel.clone());

    app.jobs.insert(
        operation_id.clone(),
        JobSummary {
            id: operation_id.clone(),
            kind: "ai".to_string(),
            status: JobStatus::Running,
            error: None,
        },
    );
    app.bus.publish(AppEvent::JobStarted {
        id: operation_id.clone(),
        kind: "ai".to_string(),
    });

    let app_c = app.clone();
    let session_c = session.clone();
    let op_id_c = operation_id.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let outcome = match std::panic::AssertUnwindSafe(
            app_c.ai.generate_codex_page_image(session_c, req, cancel),
        )
        .catch_unwind()
        .await
        {
            Ok(Ok(())) => AiJobOutcome::Completed,
            Ok(Err(e)) if e.to_string().contains("cancelled") => AiJobOutcome::Cancelled,
            Ok(Err(e)) => {
                tracing::warn!(operation_id = %op_id_c, "Codex image generation failed: {e:#}");
                AiJobOutcome::Failed(format!("{e:#}"))
            }
            Err(_panic) => AiJobOutcome::Panic,
        };
        finish_ai_job(app_c.as_ref(), &op_id_c, outcome);
    });

    Ok(Json(CodexImageGenerationResponse { operation_id }))
}

#[cfg(test)]
mod ai_admission_tests {
    use super::*;
    use koharu_app::{App, AppConfig, ProjectSession};
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use uuid::Uuid;

    async fn test_state() -> (AppState, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("koharu-ai-admission-{}", Uuid::new_v4()));
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
        let session = ProjectSession::create(&dir, "ai-admission").expect("create session");
        state.app().unwrap().session.store(Some(session.clone()));
        session
    }

    // AR06-T04 RED: with both AI slots held, a third image-generation
    // request must be rejected (429) instead of entering a third pending job.
    #[tokio::test]
    async fn ai_admission_third_concurrent_gets_429() {
        let (state, root) = test_state().await;
        let session = open_session(&state, &root);
        let app = state.app().unwrap();
        let _first = try_acquire_ai_slot(app.as_ref()).expect("slot 1");
        let _second = try_acquire_ai_slot(app.as_ref()).expect("slot 2");

        let response = start_codex_image_generation(
            State(state.clone()),
            Json(CodexImageGenerationOptions {
                page_id: koharu_core::PageId::new(),
                prompt: "test".into(),
                model: None,
                instructions: None,
                quality: None,
                size: None,
            }),
        )
        .await;

        let status = match &response {
            Ok(_) => 200, // entered a third pending job — pre-GREEN behavior
            Err(error) => error.status,
        };
        drop(session);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(status, 429, "third concurrent AI task must be rejected");
    }

    // Lock: panic cleanup lands the job as Failed and frees the slot.
    #[tokio::test]
    async fn ai_admission_panic_cleanup_marks_failed_and_releases() {
        let (state, root) = test_state().await;
        let _session = open_session(&state, &root);
        let app = state.app().unwrap();
        let permit = try_acquire_ai_slot(app.as_ref()).expect("slot");
        register_cancel("op-panic".into(), Arc::new(AtomicBool::new(false)));
        app.jobs.insert(
            "op-panic".into(),
            JobSummary {
                id: "op-panic".into(),
                kind: "ai".into(),
                status: JobStatus::Running,
                error: None,
            },
        );

        finish_ai_job(app.as_ref(), "op-panic", AiJobOutcome::Panic);
        drop(permit);

        let job = app.jobs.get("op-panic").expect("job recorded");
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("ai task panicked"));
        drop(job);
        assert!(
            try_acquire_ai_slot(app.as_ref()).is_ok(),
            "slot must be re-acquirable after cleanup"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
