//! Small, test-only HTTP client for the backend routes exercised here.
//!
//! This intentionally models only the request/response fields consumed by
//! the integration suite. The OpenAPI document remains the source of truth
//! for the public API; tests do not need a generated workspace crate for it.

use anyhow::{Context, Result, bail};
use reqwest::{Method, Response};
use serde::{Serialize, de::DeserializeOwned};

#[derive(Clone, Debug)]
pub struct Configuration {
    pub base_path: String,
    pub user_agent: Option<String>,
    pub client: reqwest::Client,
}

impl Configuration {
    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let request = self.client.request(
            method,
            format!("{}{path}", self.base_path.trim_end_matches('/')),
        );
        match &self.user_agent {
            Some(user_agent) => request.header(reqwest::header::USER_AGENT, user_agent),
            None => request,
        }
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        decode_json(self.request(Method::GET, path).send().await?).await
    }

    pub async fn request_json<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<T> {
        decode_json(self.request(method, path).json(body).send().await?).await
    }

    pub async fn request_empty(&self, method: Method, path: &str) -> Result<()> {
        ensure_success(self.request(method, path).send().await?).await?;
        Ok(())
    }

    pub async fn request_empty_json<B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<()> {
        ensure_success(self.request(method, path).json(body).send().await?).await?;
        Ok(())
    }
}

async fn ensure_success(response: Response) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    bail!("backend returned {status}: {body}")
}

async fn decode_json<T: DeserializeOwned>(response: Response) -> Result<T> {
    let response = ensure_success(response).await?;
    let bytes = response.bytes().await?;
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "decode backend JSON response: {}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

pub mod models {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    #[derive(Clone, Debug, Serialize)]
    pub struct CreateProjectRequest {
        pub name: String,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct OpenProjectRequest {
        pub id: String,
    }

    #[derive(Clone, Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StartPipelineRequest {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub default_font: Option<Option<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub pages: Option<Option<Vec<uuid::Uuid>>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub region: Option<Option<Box<Value>>>,
        pub steps: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub system_prompt: Option<Option<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub target_language: Option<Option<String>>,
    }

    #[derive(Clone, Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StartDownloadRequest {
        pub model_id: String,
    }

    #[derive(Clone, Copy, Debug, Default, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ExportFormat {
        #[default]
        Khr,
        Psd,
        Rendered,
        Inpainted,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct ExportProjectRequest {
        pub format: ExportFormat,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub pages: Option<Option<Vec<uuid::Uuid>>>,
    }

    #[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "lowercase")]
    pub enum LlmTargetKind {
        #[default]
        Local,
        Provider,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct LlmTarget {
        pub kind: LlmTargetKind,
        pub model_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub provider_id: Option<Option<String>>,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct LlmLoadRequest {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub options: Option<Option<Box<Value>>>,
        pub target: Box<LlmTarget>,
    }

    #[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "lowercase")]
    pub enum LlmStateStatus {
        #[default]
        Empty,
        Loading,
        Ready,
        Failed,
    }

    #[derive(Clone, Debug, Serialize)]
    pub struct ProviderSecretRequest {
        pub secret: String,
    }

    #[derive(Clone, Debug, Default, Serialize)]
    pub struct ConfigPatch {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub http: Option<Option<Box<HttpConfigPatch>>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub pipeline: Option<Option<Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub providers: Option<Option<Vec<Value>>>,
    }

    #[derive(Clone, Debug, Default, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct HttpConfigPatch {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub connect_timeout: Option<Option<u64>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub read_timeout: Option<Option<u64>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub max_retries: Option<Option<u32>>,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ProjectSummary {
        pub id: String,
        pub name: String,
        pub path: String,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct ListProjectsResponse {
        pub projects: Vec<ProjectSummary>,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct MetaInfo {
        pub ml_device: String,
        pub version: String,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct EngineCatalog {
        #[serde(default)]
        pub detectors: Vec<Value>,
        #[serde(default)]
        pub inpainters: Vec<Value>,
        #[serde(default)]
        pub ocr: Vec<Value>,
        #[serde(default)]
        pub renderers: Vec<Value>,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct AppConfig {
        #[serde(default)]
        pub http: Option<Box<HttpConfig>>,
        #[serde(default)]
        pub providers: Option<Vec<ProviderConfig>>,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct HttpConfig {
        pub connect_timeout: Option<u64>,
        pub read_timeout: Option<u64>,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct ProviderConfig {
        pub id: String,
        #[serde(default)]
        pub api_key: Option<Option<String>>,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct GoogleFontCatalog {
        pub fonts: Vec<GoogleFontEntry>,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct GoogleFontEntry {
        pub family: String,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct LlmCatalog {
        pub local_models: Vec<Value>,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct LlmState {
        pub status: LlmStateStatus,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StartPipelineResponse {
        pub operation_id: String,
    }

    #[derive(Clone, Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StartDownloadResponse {
        pub operation_id: String,
    }
}

pub mod api {
    use anyhow::Result;
    use reqwest::Method;

    use super::{Configuration, models};

    pub async fn cancel_operation(client: &Configuration, id: &str) -> Result<()> {
        client
            .request_empty(Method::DELETE, &format!("/operations/{id}"))
            .await
    }

    pub async fn clear_provider_secret(client: &Configuration, id: &str) -> Result<()> {
        client
            .request_empty(Method::DELETE, &format!("/config/providers/{id}/secret"))
            .await
    }

    pub async fn create_project(
        client: &Configuration,
        request: models::CreateProjectRequest,
    ) -> Result<models::ProjectSummary> {
        client
            .request_json(Method::POST, "/projects", &request)
            .await
    }

    pub async fn delete_current_llm(client: &Configuration) -> Result<()> {
        client.request_empty(Method::DELETE, "/llm/current").await
    }

    pub async fn delete_current_project(client: &Configuration) -> Result<()> {
        client
            .request_empty(Method::DELETE, "/projects/current")
            .await
    }

    pub async fn get_catalog(client: &Configuration) -> Result<models::LlmCatalog> {
        client.get_json("/llm/catalog").await
    }

    pub async fn get_config(client: &Configuration) -> Result<models::AppConfig> {
        client.get_json("/config").await
    }

    pub async fn get_current_llm(client: &Configuration) -> Result<models::LlmState> {
        client.get_json("/llm/current").await
    }

    pub async fn get_engine_catalog(client: &Configuration) -> Result<models::EngineCatalog> {
        client.get_json("/engines").await
    }

    pub async fn get_google_fonts_catalog(
        client: &Configuration,
    ) -> Result<models::GoogleFontCatalog> {
        client.get_json("/google-fonts").await
    }

    pub async fn get_meta(client: &Configuration) -> Result<models::MetaInfo> {
        client.get_json("/meta").await
    }

    pub async fn list_fonts(client: &Configuration) -> Result<Vec<serde_json::Value>> {
        client.get_json("/fonts").await
    }

    pub async fn list_projects(client: &Configuration) -> Result<models::ListProjectsResponse> {
        client.get_json("/projects").await
    }

    pub async fn patch_config(
        client: &Configuration,
        request: models::ConfigPatch,
    ) -> Result<models::AppConfig> {
        client
            .request_json(Method::PATCH, "/config", &request)
            .await
    }

    pub async fn put_current_llm(
        client: &Configuration,
        request: models::LlmLoadRequest,
    ) -> Result<()> {
        client
            .request_empty_json(Method::PUT, "/llm/current", &request)
            .await
    }

    pub async fn put_current_project(
        client: &Configuration,
        request: models::OpenProjectRequest,
    ) -> Result<models::ProjectSummary> {
        client
            .request_json(Method::PUT, "/projects/current", &request)
            .await
    }

    pub async fn set_provider_secret(
        client: &Configuration,
        id: &str,
        request: models::ProviderSecretRequest,
    ) -> Result<()> {
        client
            .request_empty_json(
                Method::PUT,
                &format!("/config/providers/{id}/secret"),
                &request,
            )
            .await
    }

    pub async fn start_download(
        client: &Configuration,
        request: models::StartDownloadRequest,
    ) -> Result<models::StartDownloadResponse> {
        client
            .request_json(Method::POST, "/downloads", &request)
            .await
    }

    pub async fn start_pipeline(
        client: &Configuration,
        request: models::StartPipelineRequest,
    ) -> Result<models::StartPipelineResponse> {
        client
            .request_json(Method::POST, "/pipelines", &request)
            .await
    }
}
