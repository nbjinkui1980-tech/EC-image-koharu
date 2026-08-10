//! MCP server exposing Koharu operations as tools.
//!
//! Built on rmcp 1.5's `#[tool_router]` + streamable HTTP transport. Mount
//! via [`mount`] onto an existing axum `Router`; sessions and routing are
//! handled by `StreamableHttpService`.
//!
//! **Tools exposed:**
//!   - `koharu.apply` — apply an `Op` to the active scene
//!   - `koharu.undo` / `koharu.redo`
//!   - `koharu.open_project` / `koharu.close_project`
//!   - `koharu.start_pipeline` — kick off a pipeline run
//!
//! More tools can be added by extending the `#[tool_router]` impl.

use std::sync::Arc;

use dashmap::DashMap;

use camino::Utf8PathBuf;
use koharu_app::{
    App, AppConfig,
    pipeline::{PipelineRunOptions, PipelineSpec, Scope},
};
use koharu_core::{JobSummary, NodeId, Op, PageId, ReadingOrder};
use rmcp::handler::server::wrapper::{Json as JsonOutput, Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::security::SecurityContext;

/// Server state handed to each tool call. Carries the shared `App`.
#[derive(Clone)]
pub struct KoharuServer {
    state: AppState,
}

impl KoharuServer {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    fn app(&self) -> Result<Arc<App>, rmcp::ErrorData> {
        self.state
            .app()
            .ok_or_else(|| rmcp::ErrorData::internal_error("app is still bootstrapping", None))
    }
}

fn get_job_from_registry(jobs: &DashMap<String, JobSummary>, id: &str) -> Option<JobSummary> {
    jobs.get(id).map(|entry| entry.value().clone())
}

// ---------------------------------------------------------------------------
// Tool I/O schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplyInput {
    /// The `Op` value to apply.
    pub op: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutput {
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UndoOutput {
    pub epoch: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenProjectInput {
    pub path: String,
    /// If set, create the project with this name instead of opening an existing one.
    pub create_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenProjectOutput {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct StartPipelineInput {
    pub steps: Vec<String>,
    pub pages: Option<Vec<PageId>>,
    pub text_node_ids: Option<Vec<NodeId>>,
    pub target_language: Option<String>,
    pub system_prompt: Option<String>,
    pub default_font: Option<String>,
    pub reading_order: Option<ReadingOrder>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartPipelineOutput {
    pub job_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetJobInput {
    pub job_id: String,
}

fn options_from_input(input: &StartPipelineInput, config: &AppConfig) -> PipelineRunOptions {
    PipelineRunOptions {
        source_text_policy: config.pipeline.source_text_policy,
        target_language: input.target_language.clone(),
        system_prompt: input.system_prompt.clone(),
        default_font: input.default_font.clone(),
        text_node_ids: input.text_node_ids.clone(),
        reading_order: input.reading_order,
        region: None,
    }
}

// ---------------------------------------------------------------------------
// Tool router
// ---------------------------------------------------------------------------

#[tool_router]
impl KoharuServer {
    #[tool(name = "koharu.apply", description = "Apply an Op to the active scene")]
    async fn apply(
        &self,
        Parameters(input): Parameters<ApplyInput>,
    ) -> Result<JsonOutput<ApplyOutput>, rmcp::ErrorData> {
        let app = self.app()?;
        let op: Op = serde_json::from_value(input.op).map_err(err)?;
        crate::routes::history::validate_external_op(&op).map_err(err)?;
        let epoch = app.apply(op).map_err(err)?;
        Ok(JsonOutput(ApplyOutput { epoch }))
    }

    #[tool(name = "koharu.undo", description = "Undo the most recent op")]
    async fn undo(&self) -> Result<JsonOutput<UndoOutput>, rmcp::ErrorData> {
        let app = self.app()?;
        let epoch = app.undo().map_err(err)?;
        Ok(JsonOutput(UndoOutput { epoch }))
    }

    #[tool(name = "koharu.redo", description = "Redo the most recent undo")]
    async fn redo(&self) -> Result<JsonOutput<UndoOutput>, rmcp::ErrorData> {
        let app = self.app()?;
        let epoch = app.redo().map_err(err)?;
        Ok(JsonOutput(UndoOutput { epoch }))
    }

    #[tool(
        name = "koharu.open_project",
        description = "Open or create a Koharu project directory"
    )]
    async fn open_project(
        &self,
        Parameters(input): Parameters<OpenProjectInput>,
    ) -> Result<JsonOutput<OpenProjectOutput>, rmcp::ErrorData> {
        let app = self.app()?;
        let path = Utf8PathBuf::from(input.path);
        let session = match input.create_name {
            Some(name) => app.open_project(path, Some(name)).await,
            None => app.open_untrusted_project(path).await,
        }
        .map_err(err)?;
        Ok(JsonOutput(OpenProjectOutput {
            name: session.scene.read().project.name.clone(),
            path: session.dir.to_string(),
        }))
    }

    #[tool(
        name = "koharu.close_project",
        description = "Close the active project"
    )]
    async fn close_project(&self) -> Result<JsonOutput<serde_json::Value>, rmcp::ErrorData> {
        let app = self.app()?;
        app.close_project().await.map_err(err)?;
        Ok(JsonOutput(serde_json::Value::Null))
    }

    #[tool(
        name = "koharu.start_pipeline",
        description = "Kick off a pipeline run; returns a job id"
    )]
    async fn start_pipeline(
        &self,
        Parameters(input): Parameters<StartPipelineInput>,
    ) -> Result<JsonOutput<StartPipelineOutput>, rmcp::ErrorData> {
        let app = self.app()?;
        let session = app
            .current_session()
            .ok_or_else(|| rmcp::ErrorData::invalid_request("no project open", None))?;
        let options = options_from_input(&input, &app.config.load());
        let spec = PipelineSpec {
            scope: match input.pages {
                Some(pages) => Scope::Pages(pages),
                None => Scope::WholeProject,
            },
            steps: input.steps,
            options,
        };
        let job_id = crate::routes::pipelines::spawn_pipeline_job(app.as_ref(), session, spec)
            .map_err(|error| rmcp::ErrorData::invalid_request(format!("{error:#}"), None))?;
        Ok(JsonOutput(StartPipelineOutput { job_id }))
    }

    #[tool(name = "koharu.get_job", description = "Read an existing pipeline job")]
    async fn get_job(
        &self,
        Parameters(input): Parameters<GetJobInput>,
    ) -> Result<JsonOutput<JobSummary>, rmcp::ErrorData> {
        let app = self.app()?;
        let job = get_job_from_registry(&app.jobs, &input.job_id)
            .ok_or_else(|| rmcp::ErrorData::invalid_request("unknown job id", None))?;
        Ok(JsonOutput(job))
    }
}

fn err(e: impl std::fmt::Display) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(e.to_string(), None)
}

#[tool_handler]
impl ServerHandler for KoharuServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        let mut implementation = rmcp::model::Implementation::default();
        implementation.name = "koharu".into();
        implementation.version = env!("CARGO_PKG_VERSION").into();
        info.server_info = implementation;
        info
    }
}

// ---------------------------------------------------------------------------
// Axum mount
// ---------------------------------------------------------------------------

/// Mount the MCP endpoint at `/mcp` on `router`.
pub fn mount(router: axum::Router, state: AppState, security: SecurityContext) -> axum::Router {
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::middleware;
    use axum::response::IntoResponse;

    let manager = Arc::new(LocalSessionManager::default());
    let factory = {
        let state = state.clone();
        move || -> Result<KoharuServer, std::io::Error> { Ok(KoharuServer::new(state.clone())) }
    };
    let service =
        StreamableHttpService::new(factory, manager, StreamableHttpServerConfig::default());

    let mcp_auth = middleware::from_fn(move |request: Request, next: middleware::Next| {
        let security = security.clone();
        async move {
            if security.authorizes_bearer(request.headers()) {
                return next.run(request).await;
            }
            StatusCode::UNAUTHORIZED.into_response()
        }
    });

    router.nest_service("/mcp", service).layer(mcp_auth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use camino::Utf8PathBuf;
    use koharu_app::{AppConfig, ProjectSession, config::SourceTextPolicy};
    use koharu_core::{
        ImageData, ImageRole, Node, NodeDataPatch, NodePatch, Page, TextData, TextDataPatch,
        Transform,
    };
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use uuid::Uuid;

    fn in_memory_app() -> Arc<App> {
        let runtime = RuntimeManager::new(
            koharu_runtime::default_app_data_root().into_std_path_buf(),
            ComputePolicy::CpuOnly,
        )
        .expect("create runtime");
        Arc::new(App::new(AppConfig::default(), Arc::new(runtime), true, "test").expect("app"))
    }

    fn typography_session() -> (Utf8PathBuf, Arc<ProjectSession>, PageId) {
        let root = std::env::temp_dir().join(format!("koharu-mcp-pipeline-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).expect("create test root");
        let root = Utf8PathBuf::from_path_buf(root).expect("UTF-8 test root");
        let path = root.join("pipeline.khrproj");
        let session = ProjectSession::create(&path, "pipeline").expect("create session");
        let source = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            16,
            16,
            image::Rgba([255, 255, 255, 255]),
        ));
        let blob = session.blobs.put_raw(&source).expect("store source image");
        let mut page = Page::new("page", 16, 16);
        let page_id = page.id;
        let source_id = NodeId::new();
        page.nodes.insert(
            source_id,
            Node {
                id: source_id,
                transform: Transform::default(),
                visible: true,
                kind: koharu_core::NodeKind::Image(ImageData {
                    role: ImageRole::Source,
                    blob,
                    opacity: 1.0,
                    natural_width: 16,
                    natural_height: 16,
                    name: Some("source".into()),
                }),
            },
        );
        let text_id = NodeId::new();
        page.nodes.insert(
            text_id,
            Node {
                id: text_id,
                transform: Transform {
                    x: 1.0,
                    y: 1.0,
                    width: 14.0,
                    height: 14.0,
                    rotation_deg: 0.0,
                },
                visible: true,
                kind: koharu_core::NodeKind::Text(TextData {
                    text: Some("source".into()),
                    translation: Some("translation".into()),
                    ..Default::default()
                }),
            },
        );
        session
            .apply(Op::AddPage { page, at: 0 })
            .expect("add source page");
        (root, session, page_id)
    }

    #[test]
    fn mcp_options_inherits_source_text_policy() {
        let mut config = AppConfig::default();
        config.pipeline.source_text_policy = SourceTextPolicy::AllText;
        let page = PageId::new();
        let input = StartPipelineInput {
            steps: vec!["llm".to_string()],
            pages: Some(vec![page]),
            text_node_ids: None,
            target_language: Some("ko".to_string()),
            system_prompt: Some("system".to_string()),
            default_font: Some("Noto Sans".to_string()),
            reading_order: Some(ReadingOrder::Ltr),
        };

        let options = options_from_input(&input, &config);

        assert_eq!(options.source_text_policy, SourceTextPolicy::AllText);
        assert_eq!(options.target_language.as_deref(), Some("ko"));
        assert_eq!(input.steps, ["llm"]);
        assert_eq!(input.pages.as_deref(), Some([page].as_slice()));
    }

    #[test]
    fn mcp_apply_rejects_forged_typography_plan_marker() {
        let op = Op::Batch {
            ops: vec![Op::UpdateNode {
                page: PageId::new(),
                id: NodeId::new(),
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(TextDataPatch {
                        typography_plan_verified: Some(true),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                prev: NodePatch::default(),
            }],
            label: "nested".into(),
        };

        assert!(crate::routes::history::validate_external_op(&op).is_err());
    }

    #[tokio::test]

    async fn hanonly_pre_greenc_red_t3_mcp_marker_rejection_contract() {
        let app = in_memory_app();
        let (root, session, page_id) = typography_session();
        let node_id = session
            .scene
            .read()
            .pages
            .get(&page_id)
            .unwrap()
            .nodes
            .values()
            .find(|node| matches!(node.kind, koharu_core::NodeKind::Text(_)))
            .unwrap()
            .id;
        let page = session.scene.read().pages.get(&page_id).unwrap().clone();
        app.session.store(Some(session.clone()));
        let state = crate::BootstrapManager::new(app.runtime.clone());
        assert!(state.set_app(app).is_ok(), "set app");
        let server = KoharuServer::new(state);

        for case in crate::routes::history::tests::t3_marker_cases(&page, node_id) {
            let before = crate::routes::history::tests::mutation_state(&session);
            let result = server.apply(Parameters(ApplyInput { op: case.raw })).await;
            if case.reject {
                assert!(
                    result.is_err(),
                    "{case_name}: expected error",
                    case_name = case.name
                );
                assert_eq!(
                    crate::routes::history::tests::mutation_state(&session),
                    before,
                    "{}",
                    case.name
                );
            } else {
                if result.is_ok() {
                    assert_eq!(session.epoch(), before.1 + 1, "{}", case.name);
                    assert!(
                        !crate::routes::history::tests::has_verified_marker(&session),
                        "{}",
                        case.name
                    );
                } else {
                    assert_eq!(
                        crate::routes::history::tests::mutation_state(&session),
                        before,
                        "{}",
                        case.name
                    );
                }
            }
        }
        drop(server);
        drop(session);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mcp_existing_path_open_clears_forged_typography_marker_before_activation() {
        let root = std::env::temp_dir().join(format!("koharu-mcp-open-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = Utf8PathBuf::from_path_buf(root).unwrap();
        let existing = root.join("existing.khrproj");
        let session = ProjectSession::create(&existing, "existing").unwrap();
        let mut page = Page::new("p1", 10, 10);
        let id = NodeId::new();
        page.nodes.insert(
            id,
            Node {
                id,
                transform: Transform::default(),
                visible: true,
                kind: koharu_core::NodeKind::Text(TextData {
                    typography_plan_verified: true,
                    ..Default::default()
                }),
            },
        );
        session.apply(Op::AddPage { page, at: 0 }).unwrap();
        session.compact().unwrap();
        drop(session);

        let session = ProjectSession::open_untrusted(&existing).unwrap();
        let marker = session
            .scene
            .read()
            .pages
            .values()
            .flat_map(|page| page.nodes.values())
            .find_map(|node| match &node.kind {
                koharu_core::NodeKind::Text(text) => Some(text.typography_plan_verified),
                _ => None,
            })
            .unwrap();
        assert!(!marker);
        drop(session);

        let created = root.join("created.khrproj");
        let created = ProjectSession::create(&created, "created").unwrap();
        assert!(created.scene.read().pages.is_empty());
        drop(created);
        std::fs::remove_dir_all(root.as_std_path()).unwrap();
    }

    #[tokio::test]
    async fn mcp_typography_pipeline_job_uses_shared_registry_and_emits_planner_warning() {
        let app = in_memory_app();
        let (root, session, page) = typography_session();
        let mut events = app.bus.subscribe();
        let id = crate::routes::pipelines::spawn_pipeline_job(
            app.as_ref(),
            session.clone(),
            PipelineSpec {
                scope: Scope::Pages(vec![page]),
                steps: vec!["cloud-typography-planner".into()],
                options: PipelineRunOptions {
                    source_text_policy: SourceTextPolicy::AllText,
                    ..Default::default()
                },
            },
        )
        .expect("start registered Typography Planner job");
        assert!(app.jobs.contains_key(&id));

        let mut started = false;
        let mut warnings = Vec::new();
        let finished = loop {
            let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .expect("job event timed out")
                .expect("job event channel closed");
            match event.event {
                koharu_core::AppEvent::JobStarted { id: event_id, .. } if event_id == id => {
                    started = true;
                }
                koharu_core::AppEvent::JobWarning(warning) if warning.job_id == id => {
                    warnings.push(warning);
                }
                koharu_core::AppEvent::JobFinished(finished) if finished.id == id => {
                    break finished;
                }
                _ => {}
            }
        };

        assert!(started, "shared job id must emit JobStarted");
        assert_eq!(warnings.len(), 1, "planner emits one soft warning");
        assert_eq!(warnings[0].step_id, "cloud-typography-planner");
        assert!(
            warnings[0]
                .message
                .starts_with("Typography Planner fallback:")
        );
        let state = crate::BootstrapManager::new(app.runtime.clone());
        assert!(state.set_app(app.clone()).is_ok(), "set app");
        let summary = KoharuServer::new(state)
            .get_job(Parameters(GetJobInput { job_id: id.clone() }))
            .await
            .expect("MCP get_job lookup")
            .0;
        assert_eq!(summary.status, koharu_core::JobStatus::CompletedWithErrors);
        assert_eq!(summary.error.as_deref(), Some(warnings[0].message.as_str()));
        assert_eq!(finished.status, koharu_core::JobStatus::CompletedWithErrors);
        assert_eq!(finished.error, summary.error);

        drop(session);
        std::fs::remove_dir_all(root.as_std_path()).expect("remove test root");
    }

    #[tokio::test]
    async fn mcp_typography_get_job_rejects_unknown_id() {
        let app = in_memory_app();
        let state = crate::BootstrapManager::new(app.runtime.clone());
        assert!(state.set_app(app.clone()).is_ok(), "set app");
        let server = KoharuServer::new(state);

        let error = match server
            .get_job(Parameters(GetJobInput {
                job_id: "missing".into(),
            }))
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("unknown job id must be an invalid MCP request"),
        };

        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
        assert_eq!(error.message, "unknown job id");
        assert!(app.jobs.is_empty(), "unknown lookup must not create a job");
    }
}
