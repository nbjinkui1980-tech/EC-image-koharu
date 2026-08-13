//! Config routes. Apply via `koharu_app::config::apply_patch`, then persist
//! (config.toml) and broadcast `ConfigChanged`. Provider secrets sync to the
//! keyring via `sync_secrets`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use koharu_app::AppConfig;
use koharu_app::config;
use koharu_core::ConfigPatch;
use serde::{Deserialize, Serialize};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::AppState;
use crate::error::{ApiError, ApiResult};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::default()
        .routes(routes!(get_config))
        .routes(routes!(patch_config))
        .routes(routes!(set_provider_secret))
        .routes(routes!(clear_provider_secret))
}

#[utoipa::path(get, path = "/config", responses((status = 200, body = AppConfig)))]
async fn get_config(State(app): State<AppState>) -> ApiResult<Json<AppConfig>> {
    Ok(Json((**app.config.load()).clone()))
}

#[utoipa::path(
    patch,
    path = "/config",
    request_body = ConfigPatch,
    responses(
        (status = 200, body = AppConfig),
        (status = 409, body = ApiError, description = "Provider base URL authority changed without a new secret")
    )
)]
async fn patch_config(
    State(app): State<AppState>,
    Json(patch): Json<ConfigPatch>,
) -> ApiResult<Json<AppConfig>> {
    let current = (**app.config.load()).clone();
    let conflicts = config::provider_authority_conflicts(&current, &patch);
    if !conflicts.is_empty() {
        let message = format!(
            "provider base URL authority changed without a new secret: {}",
            conflicts
                .iter()
                .map(|conflict| conflict.provider_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let message: String = message.chars().take(256).collect();
        return Err(ApiError::new(StatusCode::CONFLICT, message));
    }
    let mut next = current;
    config::apply_patch(&mut next, patch);
    config::sync_secrets(&next).map_err(ApiError::internal)?;
    config::save(&next).map_err(ApiError::internal)?;
    app.config.store(Arc::new(next.clone()));
    Ok(Json(next))
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSecretRequest {
    pub secret: String,
}

/// Save (or overwrite) the keyring secret for a provider. Creates the
/// provider entry in `config.providers` if it didn't exist. `PUT` because
/// setting the secret is idempotent for the same body.
#[utoipa::path(
    put,
    path = "/config/providers/{id}/secret",
    params(("id" = String, Path, description = "Provider id")),
    request_body = ProviderSecretRequest,
    responses((status = 204))
)]
async fn set_provider_secret(
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ProviderSecretRequest>,
) -> ApiResult<StatusCode> {
    let mut next = (**app.config.load()).clone();
    upsert_provider_secret(&mut next, &id, Some(&req.secret));
    config::sync_secrets(&next).map_err(ApiError::internal)?;
    config::save(&next).map_err(ApiError::internal)?;
    app.config.store(Arc::new(next));
    Ok(StatusCode::NO_CONTENT)
}

/// Clear a provider's keyring secret. The provider entry itself is kept.
#[utoipa::path(
    delete,
    path = "/config/providers/{id}/secret",
    params(("id" = String, Path, description = "Provider id")),
    responses((status = 204))
)]
async fn clear_provider_secret(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let mut next = (**app.config.load()).clone();
    upsert_provider_secret(&mut next, &id, None);
    config::sync_secrets(&next).map_err(ApiError::internal)?;
    config::save(&next).map_err(ApiError::internal)?;
    app.config.store(Arc::new(next));
    Ok(StatusCode::NO_CONTENT)
}

fn upsert_provider_secret(config: &mut AppConfig, id: &str, secret: Option<&str>) {
    let redacted = secret.map(config::RedactedSecret::new);
    if let Some(existing) = config.providers.iter_mut().find(|p| p.id == id) {
        existing.api_key = redacted;
    } else {
        config.providers.push(config::ProviderConfig {
            id: id.to_string(),
            base_url: None,
            api_key: redacted,
        });
    }
}

#[cfg(test)]
mod config_conflict_tests {
    use super::*;
    use koharu_app::App;
    use koharu_app::config::{ProviderConfig, RedactedSecret};
    use koharu_runtime::{ComputePolicy, RuntimeManager};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    const TEST_PROVIDER: &str = "ar03-test-provider";
    const MASTER: &str = "Bearer KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio";

    static CONFIG_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn config_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
        CONFIG_TEST_LOCK.lock().await
    }

    struct ConfigFileGuard {
        path: std::path::PathBuf,
        original: Option<Vec<u8>>,
    }

    impl ConfigFileGuard {
        fn capture() -> Self {
            let path = config::config_path()
                .expect("config path")
                .into_std_path_buf();
            let original = std::fs::read(&path).ok();
            Self { path, original }
        }
    }

    impl Drop for ConfigFileGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(bytes) => {
                    let _ = std::fs::write(&self.path, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(&self.path);
                }
            }
        }
    }

    fn seeded_state(base_url: &str) -> crate::AppState {
        let runtime = RuntimeManager::new(
            koharu_runtime::default_app_data_root().into_std_path_buf(),
            ComputePolicy::CpuOnly,
        )
        .expect("create runtime");
        let mut app_config = AppConfig::default();
        app_config.providers.push(ProviderConfig {
            id: TEST_PROVIDER.into(),
            base_url: Some(base_url.into()),
            api_key: Some(RedactedSecret::new("stored-secret")),
        });
        let app = Arc::new(App::new(app_config, Arc::new(runtime), true, "test").expect("app"));
        let state = crate::BootstrapManager::new(app.runtime.clone());
        assert!(state.set_app(app).is_ok(), "set app");
        state
    }

    async fn spawn_config_server(
        state: crate::AppState,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let security = crate::security::SecurityContext::from_secret([0x2A; 32]);
        let policy = crate::security::OriginHostPolicy::for_listener(
            addr,
            false,
            crate::security::RemoteHostPolicy::empty(),
        );
        let task = tokio::spawn(async move {
            let _ = crate::server::serve_with_listener(listener, state, security, policy).await;
        });
        (addr, task)
    }

    async fn patch_raw(
        addr: std::net::SocketAddr,
        raw: &serde_json::Value,
    ) -> (u16, serde_json::Value) {
        let body = serde_json::to_vec(raw).unwrap();
        let request = format!(
            "PATCH /api/v1/config HTTP/1.1\r\nHost: {addr}\r\nAuthorization: {MASTER}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
        let head = String::from_utf8_lossy(&response[..split]).into_owned();
        let status = head
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let body =
            serde_json::from_slice(&response[split + 4..]).unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    fn cleanup_secret() {
        let mut app_config = AppConfig::default();
        app_config.providers.push(ProviderConfig {
            id: TEST_PROVIDER.into(),
            base_url: None,
            api_key: None,
        });
        let _ = config::sync_secrets(&app_config);
    }

    #[tokio::test]
    async fn rpc_patch_config_returns_409() {
        let _lock = config_test_lock().await;
        let _guard = ConfigFileGuard::capture();
        let state = seeded_state("http://h:8080/v1");
        let (addr, server) = spawn_config_server(state.clone()).await;

        let (status, body) = patch_raw(
            addr,
            &serde_json::json!({
                "providers": [{"id": TEST_PROVIDER, "baseUrl": "http://h:9090/v1"}]
            }),
        )
        .await;

        server.abort();
        cleanup_secret();

        assert_eq!(status, 409, "body: {body}");
        assert_eq!(body["status"], 409);
        let message = body["message"].as_str().unwrap_or_default();
        assert!(message.contains(TEST_PROVIDER), "message: {message}");
        assert!(
            !message.contains("stored-secret"),
            "message leaks secret: {message}"
        );
        let current = (**state.config.load()).clone();
        let provider = current
            .providers
            .iter()
            .find(|p| p.id == TEST_PROVIDER)
            .unwrap();
        assert_eq!(provider.base_url.as_deref(), Some("http://h:8080/v1"));
    }

    #[tokio::test]
    async fn rpc_set_secret_then_authority_change_commits() {
        let _lock = config_test_lock().await;
        let _guard = ConfigFileGuard::capture();
        let state = seeded_state("http://h:8080/v1");
        let (addr, server) = spawn_config_server(state.clone()).await;

        let (status, body) = patch_raw(
            addr,
            &serde_json::json!({
                "providers": [{"id": TEST_PROVIDER, "baseUrl": "http://h:9090/v1", "apiKey": "new-secret"}]
            }),
        )
        .await;

        server.abort();
        cleanup_secret();

        assert_eq!(status, 200, "body: {body}");
        let provider = body["providers"]
            .as_array()
            .and_then(|providers| providers.iter().find(|p| p["id"] == TEST_PROVIDER))
            .expect("provider in response");
        assert_eq!(provider["base_url"], "http://h:9090/v1");
        assert_eq!(provider["api_key"], "[REDACTED]");
        let current = (**state.config.load()).clone();
        let stored = current
            .providers
            .iter()
            .find(|p| p.id == TEST_PROVIDER)
            .unwrap();
        assert_eq!(stored.base_url.as_deref(), Some("http://h:9090/v1"));
        assert_eq!(
            stored.api_key.as_ref().map(RedactedSecret::expose),
            Some("new-secret")
        );
    }
}
