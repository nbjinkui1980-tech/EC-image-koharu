//! Project lifecycle routes. Every project lives under the managed
//! `{data.path}/projects/` directory; clients never supply filesystem
//! paths. A project's `id` is the `.khrproj/` directory basename.
//!
//! - `GET    /projects` — list managed projects
//! - `POST   /projects` — create a new project (`{name}`), server allocates path
//! - `POST   /projects/import` — extract a `.khr` archive into a fresh dir + open
//! - `PUT    /projects/current` — open a managed project by `id`
//! - `DELETE /projects/current` — close current session
//! - `POST   /projects/current/export` — export current; returns bytes

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use koharu_app::projects as project_dirs;
use koharu_core::{ImageRole, PageId, ProjectSummary};
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::AppState;
use crate::error::{ApiError, ApiResult};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .routes(routes!(list_projects))
        .routes(routes!(create_project))
        .routes(routes!(import_project))
        .routes(routes!(put_current_project))
        .routes(routes!(delete_current_project))
        .routes(routes!(delete_project))
        .routes(routes!(export_current_project))
}

// ---------------------------------------------------------------------------
// GET /projects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListProjectsResponse {
    pub projects: Vec<ProjectSummary>,
}

#[utoipa::path(
    get,
    path = "/projects",
    responses((status = 200, body = ListProjectsResponse))
)]
async fn list_projects(State(app): State<AppState>) -> ApiResult<Json<ListProjectsResponse>> {
    let config = (**app.config.load()).clone();
    let projects = project_dirs::list_projects(&config).map_err(ApiError::internal)?;
    Ok(Json(ListProjectsResponse { projects }))
}

// ---------------------------------------------------------------------------
// POST /projects — create a new project from a display name
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    pub name: String,
}

#[utoipa::path(
    post,
    path = "/projects",
    request_body = CreateProjectRequest,
    responses((status = 200, body = ProjectSummary))
)]
async fn create_project(
    State(app): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> ApiResult<Json<ProjectSummary>> {
    let trimmed = req.name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }
    let config = (**app.config.load()).clone();
    let path = project_dirs::allocate_named(&config, trimmed).map_err(ApiError::internal)?;
    // `allocate_named` atomically created the directory so concurrent
    // callers can't collide. Session::create wants an empty-or-missing dir
    // and writes the scaffold — remove so it can populate.
    std::fs::remove_dir(path.as_std_path())
        .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?;
    let session = app
        .open_project(path, Some(trimmed.to_string()))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(koharu_app::app::project_summary(&session)))
}

// ---------------------------------------------------------------------------
// PUT /projects/current — open a managed project by id
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpenProjectRequest {
    /// `.khrproj/` directory basename (no extension). Must exist under the
    /// managed projects directory.
    pub id: String,
}

#[utoipa::path(
    put,
    path = "/projects/current",
    request_body = OpenProjectRequest,
    responses((status = 200, body = ProjectSummary))
)]
async fn put_current_project(
    State(app): State<AppState>,
    Json(req): Json<OpenProjectRequest>,
) -> ApiResult<Json<ProjectSummary>> {
    let config = (**app.config.load()).clone();
    let path = project_dirs::project_path(&config, &req.id)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    if !path.exists() {
        return Err(ApiError::not_found(format!("project {}", req.id)));
    }
    let session = app
        .open_project(path, None)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(koharu_app::app::project_summary(&session)))
}

#[utoipa::path(delete, path = "/projects/current", responses((status = 204)))]
async fn delete_current_project(State(app): State<AppState>) -> ApiResult<axum::http::StatusCode> {
    app.close_project().await.map_err(ApiError::internal)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// DELETE /projects/{id} — delete a managed project recursively
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/projects/{id}",
    params(
        ("id" = String, Path, description = "Project ID to delete")
    ),
    responses(
        (status = 204, description = "Project successfully deleted"),
        (status = 400, description = "Invalid project ID"),
        (status = 404, description = "Project not found"),
        (status = 500, description = "Internal filesystem error")
    )
)]
async fn delete_project(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    let config = (**app.config.load()).clone();
    let path = project_dirs::project_path(&config, &id)
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;

    if !path.exists() {
        return Err(ApiError::not_found(format!("project {}", id)));
    }

    // If the active session is the project we are deleting, close it first to release lock files
    if app
        .current_session()
        .is_some_and(|session| session.dir == path)
    {
        app.close_project().await.map_err(ApiError::internal)?;
    }

    // Recursively delete the project directory from disk
    tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(path.as_std_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
    .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// POST /projects/import — extract an archive into a fresh allocated dir
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/projects/import",
    request_body(content_type = "application/zip"),
    responses((status = 200, body = ProjectSummary))
)]
async fn import_project(
    State(app): State<AppState>,
    body: Bytes,
) -> ApiResult<Json<ProjectSummary>> {
    if body.is_empty() {
        return Err(ApiError::bad_request("empty archive body"));
    }
    let config = (**app.config.load()).clone();
    let body_vec = body.to_vec();
    let published =
        tokio::task::spawn_blocking(move || sanitize_and_publish_import(&config, &body_vec))
            .await
            .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
            .map_err(ApiError::internal)?;

    let session = app
        .open_project(published, None)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(koharu_app::app::project_summary(&session)))
}

fn sanitize_and_publish_import(
    config: &koharu_app::AppConfig,
    bytes: &[u8],
) -> anyhow::Result<camino::Utf8PathBuf> {
    let (staging, final_path) = project_dirs::allocate_imported(config, Some("imported"))?;
    let result = (|| {
        koharu_app::archive::import_khr_bytes_into_empty_staging(bytes, &staging)?;
        let session = koharu_app::ProjectSession::open_untrusted(&staging)?;
        drop(session);
        std::fs::rename(staging.as_std_path(), final_path.as_std_path())?;
        Ok(final_path.clone())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging.as_std_path());
    }
    result
}

// ---------------------------------------------------------------------------
// Export — returns bytes (zip when the format produces >1 file)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportProjectRequest {
    pub format: ExportFormat,
    /// Optional subset of pages; defaults to every page.
    #[serde(default)]
    pub pages: Option<Vec<PageId>>,
    /// Optional global font override (from UI preferences).
    #[serde(default)]
    pub default_font: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Whole project as a `.khr` archive (always a single zip).
    Khr,
    /// One `.psd` per page.
    Psd,
    /// One `.png` per page (the Rendered layer).
    Rendered,
    /// One `.png` per page (the Inpainted layer).
    Inpainted,
}

#[utoipa::path(
    post,
    path = "/projects/current/export",
    request_body = ExportProjectRequest,
    responses((
        status = 200,
        content_type = "application/octet-stream",
        description = "Export bytes. Content-Type is `application/zip` when the format produces multiple files."
    ))
)]
async fn export_current_project(
    State(app): State<AppState>,
    Json(req): Json<ExportProjectRequest>,
) -> ApiResult<Response> {
    let session = app
        .current_session()
        .ok_or_else(|| ApiError::bad_request("no project open"))?;

    let s_for_compact = session.clone();
    tokio::task::spawn_blocking(move || s_for_compact.compact())
        .await
        .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
        .map_err(ApiError::internal)?;

    let project_name = session.scene.read().project.name.clone();

    match req.format {
        ExportFormat::Khr => {
            let src = session.dir.clone();
            let bytes =
                tokio::task::spawn_blocking(move || koharu_app::archive::export_khr_bytes(&src))
                    .await
                    .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
                    .map_err(ApiError::internal)?;
            Ok(bytes_response(
                bytes,
                &sanitize(&project_name, "project"),
                "khr",
                "application/octet-stream",
            ))
        }
        ExportFormat::Psd => {
            let page_ids = resolve_page_ids(&session, req.pages.as_deref())?;
            if page_ids.is_empty() {
                return Err(ApiError::bad_request("no pages in selection"));
            }
            let session_c = session.clone();
            let page_ids_c = page_ids.clone();
            let renderer_c = app.renderer.clone();
            let default_font_c = req.default_font.clone();
            let files = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let mut out = Vec::with_capacity(page_ids_c.len());
                for (i, id) in page_ids_c.iter().enumerate() {
                    let bytes = crate::psd_export::psd_bytes_for_page(
                        &session_c,
                        &renderer_c,
                        default_font_c.clone(),
                        *id,
                    )?;
                    out.push((format!("page-{:03}-{id}.psd", i + 1), bytes));
                }
                Ok(out)
            })
            .await
            .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
            .map_err(ApiError::internal)?;
            Ok(files_to_response(files, &project_name, "psd")?)
        }
        ExportFormat::Rendered => {
            export_image_role(
                &session,
                req.pages.as_deref(),
                ImageRole::Rendered,
                &project_name,
            )
            .await
        }
        ExportFormat::Inpainted => {
            export_image_role(
                &session,
                req.pages.as_deref(),
                ImageRole::Inpainted,
                &project_name,
            )
            .await
        }
    }
}

async fn export_image_role(
    session: &std::sync::Arc<koharu_app::ProjectSession>,
    pages: Option<&[PageId]>,
    role: ImageRole,
    project_name: &str,
) -> ApiResult<Response> {
    let page_ids = resolve_page_ids(session, pages)?;
    if page_ids.is_empty() {
        return Err(ApiError::bad_request("no pages in selection"));
    }
    let session_c = session.clone();
    let page_ids_c = page_ids.clone();
    let files = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let mut out: Vec<(String, Vec<u8>)> = Vec::new();
        for (i, id) in page_ids_c.iter().enumerate() {
            if let Some(bytes) = crate::psd_export::png_bytes_for_page(&session_c, *id, role)? {
                out.push((format!("page-{:03}-{id}.png", i + 1), bytes));
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::internal(anyhow::Error::new(e)))?
    .map_err(ApiError::internal)?;

    if files.is_empty() {
        return Err(ApiError::bad_request(
            "no pages have the requested layer populated",
        ));
    }
    files_to_response(files, project_name, role_ext(role))
}

fn resolve_page_ids(
    session: &koharu_app::ProjectSession,
    requested: Option<&[PageId]>,
) -> ApiResult<Vec<PageId>> {
    let scene = session.scene.read();
    match requested {
        None => Ok(scene.pages.keys().copied().collect()),
        Some(ids) => {
            for id in ids {
                if !scene.pages.contains_key(id) {
                    return Err(ApiError::not_found(format!("page {id}")));
                }
            }
            Ok(ids.to_vec())
        }
    }
}

fn role_ext(role: ImageRole) -> &'static str {
    match role {
        ImageRole::Rendered => "png",
        ImageRole::Inpainted => "png",
        ImageRole::Source => "png",
        ImageRole::Custom => "png",
    }
}

fn files_to_response(
    mut files: Vec<(String, Vec<u8>)>,
    project_name: &str,
    ext: &str,
) -> ApiResult<Response> {
    if files.len() == 1 {
        let (fname, bytes) = files.remove(0);
        let content_type = match ext {
            "psd" => "image/vnd.adobe.photoshop",
            "png" => "image/png",
            "khr" => "application/octet-stream",
            _ => "application/octet-stream",
        };
        return Ok(bytes_response_with_filename(bytes, &fname, content_type));
    }
    let zip_bytes = koharu_app::archive::zip_files_to_bytes(&files).map_err(ApiError::internal)?;
    let base = sanitize(project_name, "export");
    let filename = format!("{base}-{ext}.zip");
    Ok(bytes_response_with_filename(
        zip_bytes,
        &filename,
        "application/zip",
    ))
}

fn bytes_response(bytes: Vec<u8>, base: &str, ext: &str, content_type: &str) -> Response {
    let filename = format!("{base}.{ext}");
    bytes_response_with_filename(bytes, &filename, content_type)
}

fn bytes_response_with_filename(bytes: Vec<u8>, filename: &str, content_type: &str) -> Response {
    let cd = format!("attachment; filename=\"{filename}\"");
    let mut resp = Response::new(Body::from(bytes));
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    if let Ok(v) = HeaderValue::from_str(&cd) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    resp.into_response()
}

fn sanitize(name: &str, fallback: &str) -> String {
    let s: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use koharu_app::{AppConfig, ProjectSession};
    use koharu_core::{
        Node, NodeDataPatch, NodeId, NodeKind, NodePatch, Op, Page, TextData, TextDataPatch,
        Transform,
    };

    struct TempRoot(Utf8PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("koharu-import-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(Utf8PathBuf::from_path_buf(path).unwrap())
        }

        fn config(&self) -> AppConfig {
            let mut config = AppConfig::default();
            config.data.path = self.0.clone();
            config
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.as_std_path());
        }
    }

    fn verified_archive(root: &Utf8PathBuf) -> Vec<u8> {
        let path = root.join("source.khrproj");
        let session = ProjectSession::create(&path, "import source").unwrap();
        let mut page = Page::new("p1", 100, 100);
        let page_id = page.id;
        let id = NodeId::new();
        page.nodes.insert(
            id,
            Node {
                id,
                transform: Transform::default(),
                visible: true,
                kind: NodeKind::Text(TextData {
                    text: Some("source".into()),
                    translation: Some("planned".into()),
                    typography_plan_verified: true,
                    ..Default::default()
                }),
            },
        );
        session.apply(Op::AddPage { page, at: 0 }).unwrap();
        session.compact().unwrap();
        session
            .apply(Op::UpdateNode {
                page: page_id,
                id,
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(TextDataPatch {
                        translation: Some(Some("planned from history".into())),
                        typography_plan_verified: Some(true),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                prev: NodePatch::default(),
            })
            .unwrap();
        drop(session);
        let bytes = koharu_app::archive::export_khr_bytes(&path).unwrap();
        std::fs::remove_dir_all(path.as_std_path()).unwrap();
        bytes
    }

    fn only_text_marker(session: &ProjectSession) -> bool {
        session
            .scene
            .read()
            .pages
            .values()
            .flat_map(|page| page.nodes.values())
            .find_map(|node| match &node.kind {
                NodeKind::Text(text) => Some(text.typography_plan_verified),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn http_import_clears_forged_typography_marker_before_activation() {
        let root = TempRoot::new();
        let config = root.config();
        let bytes = verified_archive(&root.0);

        let published = sanitize_and_publish_import(&config, &bytes).unwrap();
        let session = ProjectSession::open(&published).unwrap();
        assert!(!only_text_marker(&session));
        assert_eq!(
            std::fs::metadata(published.join("history.log"))
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn failed_import_sanitization_never_publishes_project() {
        let root = TempRoot::new();
        let config = root.config();

        assert!(sanitize_and_publish_import(&config, b"not a zip").is_err());
        let corrupt_project = koharu_app::archive::zip_files_to_bytes(&[
            ("project.toml".into(), b"name = \"corrupt\"\n".to_vec()),
            ("scene.bin".into(), b"KHARSCN\x02not-postcard".to_vec()),
        ])
        .unwrap();
        assert!(sanitize_and_publish_import(&config, &corrupt_project).is_err());
        let truncated_history = koharu_app::archive::zip_files_to_bytes(&[
            (
                "scene.bin".into(),
                include_bytes!("../../../koharu-app/tests/fixtures/persistence-v1/scene.bin")
                    .to_vec(),
            ),
            ("history.log".into(), vec![1]),
        ])
        .unwrap();
        assert!(sanitize_and_publish_import(&config, &truncated_history).is_err());
        assert!(project_dirs::list_projects(&config).unwrap().is_empty());
        let projects = project_dirs::projects_dir(&config).unwrap();
        assert!(
            std::fs::read_dir(projects)
                .unwrap()
                .flatten()
                .all(|entry| !entry.file_name().to_string_lossy().ends_with(".khrproj"))
        );
    }

    #[test]
    fn successful_import_publishes_after_sanitize_and_atomic_rename() {
        let root = TempRoot::new();
        let config = root.config();
        let bytes = verified_archive(&root.0);

        assert!(project_dirs::list_projects(&config).unwrap().is_empty());
        let published = sanitize_and_publish_import(&config, &bytes).unwrap();
        assert!(published.exists());
        assert_eq!(project_dirs::list_projects(&config).unwrap().len(), 1);
        let session = ProjectSession::open(&published).unwrap();
        assert!(!only_text_marker(&session));
        assert_eq!(
            std::fs::metadata(published.join("history.log"))
                .unwrap()
                .len(),
            0
        );
    }
}
