use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context;
use reqwest_middleware::ClientWithMiddleware;

use crate::prompt::{BLOCK_TAG_INSTRUCTIONS, system_prompt};
use crate::{Language, language::tags as language_tags, supported_locales};

/// Resolve the effective system prompt: custom (with block instructions appended) or default.
pub(crate) fn resolve_system_prompt(custom: Option<&str>, target_language: Language) -> String {
    match custom {
        Some(p) if !p.trim().is_empty() => format!("{p} {BLOCK_TAG_INSTRUCTIONS}"),
        _ => system_prompt(target_language),
    }
}

pub mod authority;
pub mod caiyun;
mod chat_completions;
pub mod claude;
pub mod deepl;
pub mod deepseek;
pub mod gemini;
pub mod google_translate;
pub mod openai;
pub mod openai_compatible;

#[derive(Debug, Clone, Copy)]
pub struct ProviderModelDescriptor {
    pub id: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Clone)]
pub struct DiscoveredProviderModel {
    pub id: String,
    pub name: String,
}

pub type ProviderDiscoveryFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<Vec<DiscoveredProviderModel>>> + Send>>;

pub enum ProviderCatalogModels {
    Static(&'static [ProviderModelDescriptor]),
    Dynamic(fn(ProviderConfig) -> ProviderDiscoveryFuture),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSupportedLanguages {
    All,
    Limited(&'static [Language]),
}

impl ProviderSupportedLanguages {
    pub fn tags(self) -> Vec<String> {
        match self {
            Self::All => supported_locales(),
            Self::Limited(languages) => language_tags(languages),
        }
    }
}

pub struct ProviderDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub requires_api_key: bool,
    pub requires_base_url: bool,
    pub supported_languages: ProviderSupportedLanguages,
    pub models: ProviderCatalogModels,
    pub build: fn(ProviderConfig) -> anyhow::Result<Box<dyn AnyProvider>>,
}

pub async fn ensure_provider_success(
    provider: &str,
    response: reqwest::Response,
) -> anyhow::Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let body = response
        .text()
        .await
        .with_context(|| format!("Failed to read {provider} error response body"))?;
    let body_lower = body.to_ascii_lowercase();
    let quota_exceeded = status.as_u16() == 429
        || body_lower.contains("insufficient_quota")
        || body_lower.contains("quota")
        || body_lower.contains("resource_exhausted")
        || body_lower.contains("rate limit exceeded")
        || body_lower.contains("credit balance is too low");

    if quota_exceeded {
        anyhow::bail!("provider_quota_exceeded:{provider}");
    }

    anyhow::bail!(
        "{provider} API request failed ({status}): {}",
        summarize_provider_error_body(&body)
    );
}

const PROVIDER_ERROR_BODY_MAX_CHARS: usize = 160;

// Error bodies can echo the request (including credentials) back to us, so the
// summary redacts secret-looking tokens first and only then truncates; doing it
// in this order prevents a truncated boundary from leaking a partial secret.
fn summarize_provider_error_body(body: &str) -> String {
    let redacted = redact_secret_like_tokens(body);
    let mut summary: String = redacted
        .chars()
        .take(PROVIDER_ERROR_BODY_MAX_CHARS)
        .collect();
    if redacted.chars().count() > PROVIDER_ERROR_BODY_MAX_CHARS {
        summary.push('…');
    }
    summary
}

fn redact_secret_like_tokens(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let core_len = word
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .count();
            let has_marker = word.contains('-') || word.contains('_');
            let has_alpha = word.chars().any(|c| c.is_ascii_alphabetic());
            let has_digit = word.chars().any(|c| c.is_ascii_digit());
            let looks_secret =
                (core_len >= 12 && has_marker) || (core_len >= 24 && has_alpha && has_digit);
            if looks_secret { "[redacted]" } else { word }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub trait AnyProvider: Send + Sync {
    fn translate<'a>(
        &'a self,
        source: &'a str,
        target_language: Language,
        model: &'a str,
        custom_system_prompt: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct ProviderConfig {
    pub http_client: Arc<ClientWithMiddleware>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
}

const OPENAI_MODELS: &[ProviderModelDescriptor] = &[
    ProviderModelDescriptor {
        id: "gpt-5.5",
        name: "GPT-5.5",
    },
    ProviderModelDescriptor {
        id: "gpt-5.4",
        name: "GPT-5.4",
    },
    ProviderModelDescriptor {
        id: "gpt-5.4-mini",
        name: "GPT-5.4 mini",
    },
    ProviderModelDescriptor {
        id: "gpt-5.4-nano",
        name: "GPT-5.4 nano",
    },
    ProviderModelDescriptor {
        id: "gpt-5.2",
        name: "GPT-5.2",
    },
    ProviderModelDescriptor {
        id: "gpt-5.1",
        name: "GPT-5.1",
    },
    ProviderModelDescriptor {
        id: "gpt-5",
        name: "GPT-5",
    },
    ProviderModelDescriptor {
        id: "gpt-5-mini",
        name: "GPT-5 mini",
    },
    ProviderModelDescriptor {
        id: "gpt-5-nano",
        name: "GPT-5 nano",
    },
    ProviderModelDescriptor {
        id: "gpt-5-chat-latest",
        name: "GPT-5 Chat latest",
    },
    ProviderModelDescriptor {
        id: "gpt-4.1",
        name: "GPT-4.1",
    },
    ProviderModelDescriptor {
        id: "gpt-4.1-mini",
        name: "GPT-4.1 mini",
    },
    ProviderModelDescriptor {
        id: "gpt-4.1-nano",
        name: "GPT-4.1 nano",
    },
    ProviderModelDescriptor {
        id: "o3",
        name: "o3",
    },
    ProviderModelDescriptor {
        id: "o4-mini",
        name: "o4-mini",
    },
    ProviderModelDescriptor {
        id: "o3-mini",
        name: "o3-mini",
    },
    ProviderModelDescriptor {
        id: "o1",
        name: "o1",
    },
    ProviderModelDescriptor {
        id: "o1-mini",
        name: "o1-mini",
    },
    ProviderModelDescriptor {
        id: "o1-preview",
        name: "o1 preview",
    },
    ProviderModelDescriptor {
        id: "gpt-4o",
        name: "GPT-4o",
    },
    ProviderModelDescriptor {
        id: "gpt-4o-mini",
        name: "GPT-4o mini",
    },
    ProviderModelDescriptor {
        id: "gpt-4-turbo",
        name: "GPT-4 Turbo",
    },
    ProviderModelDescriptor {
        id: "gpt-4",
        name: "GPT-4",
    },
    ProviderModelDescriptor {
        id: "gpt-3.5-turbo",
        name: "GPT-3.5 Turbo",
    },
];

const GEMINI_MODELS: &[ProviderModelDescriptor] = &[
    ProviderModelDescriptor {
        id: "gemini-flash-lite-latest",
        name: "Gemini Flash-Lite Latest",
    },
    ProviderModelDescriptor {
        id: "gemini-flash-latest",
        name: "Gemini Flash Latest",
    },
    ProviderModelDescriptor {
        id: "gemini-pro-latest",
        name: "Gemini Pro Latest",
    },
    ProviderModelDescriptor {
        id: "gemini-3.5-flash",
        name: "Gemini 3.5 Flash",
    },
    ProviderModelDescriptor {
        id: "gemini-3.1-pro-preview",
        name: "Gemini 3.1 Pro Preview",
    },
    ProviderModelDescriptor {
        id: "gemini-3.1-pro-preview-customtools",
        name: "Gemini 3.1 Pro Preview Custom Tools",
    },
    ProviderModelDescriptor {
        id: "gemini-3.1-flash-lite",
        name: "Gemini 3.1 Flash-Lite",
    },
    ProviderModelDescriptor {
        id: "gemini-3-flash-preview",
        name: "Gemini 3 Flash Preview",
    },
    ProviderModelDescriptor {
        id: "gemini-2.5-pro",
        name: "Gemini 2.5 Pro",
    },
    ProviderModelDescriptor {
        id: "gemini-2.5-flash",
        name: "Gemini 2.5 Flash",
    },
    ProviderModelDescriptor {
        id: "gemini-2.5-flash-lite",
        name: "Gemini 2.5 Flash-Lite",
    },
    ProviderModelDescriptor {
        id: "gemini-2.0-flash",
        name: "Gemini 2.0 Flash",
    },
    ProviderModelDescriptor {
        id: "gemini-2.0-flash-001",
        name: "Gemini 2.0 Flash 001",
    },
    ProviderModelDescriptor {
        id: "gemini-2.0-flash-lite",
        name: "Gemini 2.0 Flash-Lite",
    },
    ProviderModelDescriptor {
        id: "gemini-2.0-flash-lite-001",
        name: "Gemini 2.0 Flash-Lite 001",
    },
    ProviderModelDescriptor {
        id: "gemma-4-31b-it",
        name: "Gemma 4 31B",
    },
    ProviderModelDescriptor {
        id: "gemma-4-26b-a4b-it",
        name: "Gemma 4 26B",
    },
];

const CLAUDE_MODELS: &[ProviderModelDescriptor] = &[
    ProviderModelDescriptor {
        id: "claude-opus-4-7",
        name: "Claude Opus 4.7",
    },
    ProviderModelDescriptor {
        id: "claude-sonnet-4-6",
        name: "Claude Sonnet 4.6",
    },
    ProviderModelDescriptor {
        id: "claude-haiku-4-5",
        name: "Claude Haiku 4.5",
    },
    ProviderModelDescriptor {
        id: "claude-opus-4-6",
        name: "Claude Opus 4.6",
    },
    ProviderModelDescriptor {
        id: "claude-opus-4-5-20251101",
        name: "Claude Opus 4.5",
    },
    ProviderModelDescriptor {
        id: "claude-opus-4-1-20250805",
        name: "Claude Opus 4.1",
    },
    ProviderModelDescriptor {
        id: "claude-sonnet-4-5-20250929",
        name: "Claude Sonnet 4.5",
    },
    ProviderModelDescriptor {
        id: "claude-haiku-4-5-20251001",
        name: "Claude Haiku 4.5 snapshot",
    },
    ProviderModelDescriptor {
        id: "claude-opus-4-20250514",
        name: "Claude Opus 4 (deprecated)",
    },
    ProviderModelDescriptor {
        id: "claude-sonnet-4-20250514",
        name: "Claude Sonnet 4 (deprecated)",
    },
];

const DEEPSEEK_MODELS: &[ProviderModelDescriptor] = &[
    ProviderModelDescriptor {
        id: "deepseek-v4-flash",
        name: "DeepSeek V4 Flash",
    },
    ProviderModelDescriptor {
        id: "deepseek-v4-pro",
        name: "DeepSeek V4 Pro",
    },
    ProviderModelDescriptor {
        id: "deepseek-chat",
        name: "DeepSeek Chat",
    },
    ProviderModelDescriptor {
        id: "deepseek-reasoner",
        name: "DeepSeek Reasoner",
    },
];

const MT_MODELS: &[ProviderModelDescriptor] = &[ProviderModelDescriptor {
    id: "mt",
    name: "Machine Translation",
}];

const PROVIDERS: &[ProviderDescriptor] = &[
    ProviderDescriptor {
        id: "openai",
        name: "OpenAI",
        requires_api_key: true,
        requires_base_url: false,
        supported_languages: ProviderSupportedLanguages::All,
        models: ProviderCatalogModels::Static(OPENAI_MODELS),
        build: build_openai_provider,
    },
    ProviderDescriptor {
        id: "gemini",
        name: "Gemini",
        requires_api_key: true,
        requires_base_url: false,
        supported_languages: ProviderSupportedLanguages::All,
        models: ProviderCatalogModels::Static(GEMINI_MODELS),
        build: build_gemini_provider,
    },
    ProviderDescriptor {
        id: "claude",
        name: "Claude",
        requires_api_key: true,
        requires_base_url: false,
        supported_languages: ProviderSupportedLanguages::All,
        models: ProviderCatalogModels::Static(CLAUDE_MODELS),
        build: build_claude_provider,
    },
    ProviderDescriptor {
        id: "deepseek",
        name: "DeepSeek",
        requires_api_key: true,
        requires_base_url: false,
        supported_languages: ProviderSupportedLanguages::All,
        models: ProviderCatalogModels::Static(DEEPSEEK_MODELS),
        build: build_deepseek_provider,
    },
    ProviderDescriptor {
        id: "deepl",
        name: "DeepL",
        requires_api_key: true,
        requires_base_url: false,
        supported_languages: ProviderSupportedLanguages::All,
        models: ProviderCatalogModels::Static(MT_MODELS),
        build: build_deepl_mt_provider,
    },
    ProviderDescriptor {
        id: "google-translate",
        name: "Google Cloud Translation",
        requires_api_key: true,
        requires_base_url: false,
        supported_languages: ProviderSupportedLanguages::All,
        models: ProviderCatalogModels::Static(MT_MODELS),
        build: build_google_translate_mt_provider,
    },
    ProviderDescriptor {
        id: "caiyun",
        name: "Caiyun",
        requires_api_key: true,
        requires_base_url: false,
        supported_languages: ProviderSupportedLanguages::Limited(
            caiyun::SUPPORTED_TARGET_LANGUAGES,
        ),
        models: ProviderCatalogModels::Static(MT_MODELS),
        build: build_caiyun_mt_provider,
    },
    ProviderDescriptor {
        id: "openai-compatible",
        name: "OpenAI-compatible",
        requires_api_key: false,
        requires_base_url: true,
        supported_languages: ProviderSupportedLanguages::All,
        models: ProviderCatalogModels::Dynamic(discover_openai_compatible_models),
        build: build_openai_compatible_provider,
    },
];

pub fn all_provider_descriptors() -> &'static [ProviderDescriptor] {
    PROVIDERS
}

pub fn find_provider_descriptor(provider_id: &str) -> Option<&'static ProviderDescriptor> {
    PROVIDERS
        .iter()
        .find(|descriptor| descriptor.id == provider_id)
}

pub fn discover_models(
    provider_id: &str,
    config: ProviderConfig,
) -> anyhow::Result<ProviderDiscoveryFuture> {
    let descriptor = find_provider_descriptor(provider_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown API provider: {provider_id}"))?;
    Ok(match descriptor.models {
        ProviderCatalogModels::Static(models) => {
            let models = models
                .iter()
                .map(|model| DiscoveredProviderModel {
                    id: model.id.to_string(),
                    name: model.name.to_string(),
                })
                .collect::<Vec<_>>();
            Box::pin(async move { Ok(models) })
        }
        ProviderCatalogModels::Dynamic(discover) => discover(config),
    })
}

pub fn build_provider(
    provider_id: &str,
    config: ProviderConfig,
) -> anyhow::Result<Box<dyn AnyProvider>> {
    let descriptor = find_provider_descriptor(provider_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown API provider: {provider_id}"))?;

    if descriptor.requires_api_key
        && config
            .api_key
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        anyhow::bail!("api_key is required for {}", descriptor.id);
    }

    if descriptor.requires_base_url
        && config
            .base_url
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        anyhow::bail!("base_url is required for {}", descriptor.id);
    }

    (descriptor.build)(config)
}

fn required_api_key(config: &ProviderConfig, provider_id: &str) -> anyhow::Result<String> {
    config
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("api_key is required for {provider_id}"))
}

fn required_base_url(config: &ProviderConfig, provider_id: &str) -> anyhow::Result<String> {
    config
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("base_url is required for {provider_id}"))
}

fn build_openai_provider(config: ProviderConfig) -> anyhow::Result<Box<dyn AnyProvider>> {
    Ok(Box::new(openai::OpenAiProvider {
        http_client: Arc::clone(&config.http_client),
        api_key: required_api_key(&config, "openai")?,
    }))
}

fn build_gemini_provider(config: ProviderConfig) -> anyhow::Result<Box<dyn AnyProvider>> {
    Ok(Box::new(gemini::GeminiProvider {
        http_client: Arc::clone(&config.http_client),
        api_key: required_api_key(&config, "gemini")?,
    }))
}

fn build_claude_provider(config: ProviderConfig) -> anyhow::Result<Box<dyn AnyProvider>> {
    Ok(Box::new(claude::ClaudeProvider {
        http_client: Arc::clone(&config.http_client),
        api_key: required_api_key(&config, "claude")?,
    }))
}

fn build_deepseek_provider(config: ProviderConfig) -> anyhow::Result<Box<dyn AnyProvider>> {
    Ok(Box::new(deepseek::DeepSeekProvider {
        http_client: Arc::clone(&config.http_client),
        api_key: required_api_key(&config, "deepseek")?,
    }))
}

fn build_openai_compatible_provider(
    config: ProviderConfig,
) -> anyhow::Result<Box<dyn AnyProvider>> {
    Ok(Box::new(openai_compatible::OpenAiCompatibleProvider {
        http_client: Arc::clone(&config.http_client),
        base_url: required_base_url(&config, "openai-compatible")?,
        api_key: config.api_key,
        temperature: config.temperature,
        max_tokens: config.max_tokens,
    }))
}

fn build_deepl_mt_provider(config: ProviderConfig) -> anyhow::Result<Box<dyn AnyProvider>> {
    Ok(Box::new(deepl::DeeplMtProvider {
        http_client: Arc::clone(&config.http_client),
        api_key: required_api_key(&config, "deepl")?,
        base_url: config.base_url,
    }))
}

fn build_google_translate_mt_provider(
    config: ProviderConfig,
) -> anyhow::Result<Box<dyn AnyProvider>> {
    Ok(Box::new(google_translate::GoogleTranslateMtProvider {
        http_client: Arc::clone(&config.http_client),
        api_key: required_api_key(&config, "google-translate")?,
    }))
}

fn build_caiyun_mt_provider(config: ProviderConfig) -> anyhow::Result<Box<dyn AnyProvider>> {
    Ok(Box::new(caiyun::CaiyunMtProvider {
        http_client: Arc::clone(&config.http_client),
        api_key: required_api_key(&config, "caiyun")?,
    }))
}

fn discover_openai_compatible_models(config: ProviderConfig) -> ProviderDiscoveryFuture {
    Box::pin(async move {
        let base_url = required_base_url(&config, "openai-compatible")?;
        let models = openai_compatible::list_models(
            config.http_client,
            &base_url,
            config.api_key.as_deref(),
        )
        .await?;
        Ok(models
            .into_iter()
            .map(|id| DiscoveredProviderModel {
                name: id.clone(),
                id,
            })
            .collect())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(models: &[ProviderModelDescriptor]) -> Vec<&'static str> {
        models.iter().map(|model| model.id).collect()
    }

    fn assert_contains_all(provider: &str, models: &[ProviderModelDescriptor], expected: &[&str]) {
        let ids = ids(models);
        for expected_id in expected {
            assert!(
                ids.contains(expected_id),
                "{provider} model catalog should include {expected_id}"
            );
        }
    }

    #[test]
    fn static_llm_provider_catalogs_cover_current_model_families() {
        assert_contains_all(
            "openai",
            OPENAI_MODELS,
            &[
                "gpt-5.5",
                "gpt-5.4-mini",
                "gpt-5-mini",
                "gpt-4.1",
                "gpt-4o",
                "o3",
            ],
        );
        assert_contains_all(
            "gemini",
            GEMINI_MODELS,
            &[
                "gemini-3.1-pro-preview",
                "gemini-3.1-flash-lite",
                "gemini-3.5-flash",
                "gemma-4-26b-a4b-it",
            ],
        );
        assert_contains_all(
            "claude",
            CLAUDE_MODELS,
            &["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5"],
        );
        assert_contains_all(
            "deepseek",
            DEEPSEEK_MODELS,
            &[
                "deepseek-v4-flash",
                "deepseek-v4-pro",
                "deepseek-chat",
                "deepseek-reasoner",
            ],
        );
    }
}

#[cfg(test)]
mod redirect_tests {
    use super::ensure_provider_success;
    use reqwest_middleware::ClientWithMiddleware;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    type Seen = Arc<Mutex<Vec<String>>>;

    const OK_RESPONSE: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

    // RED: reqwest's default redirect policy (current production wiring shares
    // it), proxy-free to keep the measurement about redirect header handling.
    // GREEN swaps this helper to the provider-specific client if RED-0 shows
    // the default is insufficient.
    fn test_client() -> ClientWithMiddleware {
        ClientWithMiddleware::from(
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("test client"),
        )
    }

    fn new_seen() -> Seen {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn recorded(store: &Seen) -> Vec<String> {
        store.lock().expect("seen lock").clone()
    }

    async fn read_request(stream: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read =
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut chunk))
                    .await
                    .expect("request read timed out")
                    .expect("read request");
            if read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..read]);
            if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    async fn serve_once(listener: TcpListener, store: Seen, response: String) {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let request = read_request(&mut stream).await;
        store.lock().expect("seen lock").push(request);
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    }

    fn redirect_response(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn header_value(request: &str, name: &str) -> Option<String> {
        request
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim().to_string())
    }

    async fn join(task: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("server task timed out")
            .expect("server task");
    }

    #[tokio::test]
    async fn redirect_cross_port_strips_authorization() {
        let a_seen = new_seen();
        let b_seen = new_seen();
        let a_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let b_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let a_port = a_listener.local_addr().unwrap().port();
        let b_port = b_listener.local_addr().unwrap().port();

        let location = format!("http://127.0.0.1:{b_port}/target");
        let a_task = tokio::spawn(serve_once(
            a_listener,
            a_seen.clone(),
            redirect_response(&location),
        ));
        let b_task = tokio::spawn(serve_once(
            b_listener,
            b_seen.clone(),
            OK_RESPONSE.to_string(),
        ));

        let response = test_client()
            .get(format!("http://127.0.0.1:{a_port}/start"))
            .header(reqwest::header::AUTHORIZATION, "Bearer sk-test-token")
            .header(reqwest::header::COOKIE, "session=abc123")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 200);
        join(a_task).await;
        join(b_task).await;

        let b_requests = recorded(&b_seen);
        assert_eq!(b_requests.len(), 1, "B should receive the redirect");
        assert_eq!(
            header_value(&b_requests[0], "authorization"),
            None,
            "Authorization must not leak to a different port: {}",
            b_requests[0]
        );
        assert_eq!(
            header_value(&b_requests[0], "cookie"),
            None,
            "Cookie must not leak to a different port: {}",
            b_requests[0]
        );
    }

    #[tokio::test]
    async fn redirect_same_authority_keeps_authorization() {
        let store = new_seen();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let location = format!("http://127.0.0.1:{port}/final");

        let server_seen = store.clone();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.expect("accept first");
            let request = read_request(&mut first).await;
            server_seen.lock().expect("seen lock").push(request);
            first
                .write_all(redirect_response(&location).as_bytes())
                .await
                .expect("write redirect");
            drop(first);

            let (mut second, _) = listener.accept().await.expect("accept second");
            let request = read_request(&mut second).await;
            server_seen.lock().expect("seen lock").push(request);
            second
                .write_all(OK_RESPONSE.as_bytes())
                .await
                .expect("write ok");
        });

        let response = test_client()
            .get(format!("http://127.0.0.1:{port}/start"))
            .header(reqwest::header::AUTHORIZATION, "Bearer sk-test-token")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 200);
        join(server).await;

        let requests = recorded(&store);
        assert_eq!(
            requests.len(),
            2,
            "redirect should hit the same server twice"
        );
        assert!(
            requests[1].starts_with("GET /final "),
            "second request should target /final: {}",
            requests[1]
        );
        assert_eq!(
            header_value(&requests[1], "authorization"),
            Some("Bearer sk-test-token".to_string()),
            "same-authority redirect must keep Authorization"
        );
    }

    #[tokio::test]
    async fn redirect_cross_host_strips_authorization() {
        let b_listener = match TcpListener::bind("[::1]:0").await {
            Ok(listener) => listener,
            Err(_) => return, // no IPv6 loopback on this host
        };
        let a_seen = new_seen();
        let b_seen = new_seen();
        let a_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let a_port = a_listener.local_addr().unwrap().port();
        let b_port = b_listener.local_addr().unwrap().port();

        let location = format!("http://[::1]:{b_port}/target");
        let a_task = tokio::spawn(serve_once(
            a_listener,
            a_seen.clone(),
            redirect_response(&location),
        ));
        let b_task = tokio::spawn(serve_once(
            b_listener,
            b_seen.clone(),
            OK_RESPONSE.to_string(),
        ));

        let response = test_client()
            .get(format!("http://127.0.0.1:{a_port}/start"))
            .header(reqwest::header::AUTHORIZATION, "Bearer sk-test-token")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 200);
        join(a_task).await;
        join(b_task).await;

        let b_requests = recorded(&b_seen);
        assert_eq!(b_requests.len(), 1, "B should receive the redirect");
        assert_eq!(
            header_value(&b_requests[0], "authorization"),
            None,
            "Authorization must not cross hosts: {}",
            b_requests[0]
        );
    }

    #[tokio::test]
    async fn provider_error_bounded_and_redacted() {
        let secret = "sk-live-secret";
        let body = format!(
            "failure reason: invalid key {secret} provided. {}",
            "y".repeat(5000)
        );
        let response_text = format!(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let store = new_seen();
        let server = tokio::spawn(serve_once(listener, store, response_text));

        let response = test_client()
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 500);
        let error = ensure_provider_success("test-provider", response)
            .await
            .expect_err("500 must be an error");
        join(server).await;

        let message = error.to_string();
        assert!(
            message.len() <= 256,
            "error message must be bounded, got {} chars: {message}",
            message.len()
        );
        assert!(
            !message.contains(secret),
            "error message must not leak the secret: {message}"
        );
        assert!(
            message.contains("failure reason"),
            "error message should keep a useful summary: {message}"
        );
    }
}
