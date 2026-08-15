use reqwest_middleware::ClientWithMiddleware;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::Language;

use super::authority::provider_authority;
use super::chat_completions::{ChatCompletionsAuth, ChatCompletionsRequest, send_chat_completion};
use super::{AnyProvider, ensure_provider_success, resolve_system_prompt};

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    pub http_client: Arc<ClientWithMiddleware>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

fn normalized_base_url(base_url: &str) -> anyhow::Result<String> {
    let normalized = base_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        anyhow::bail!("OpenAI-compatible base URL is required");
    }
    provider_authority(&normalized)?;
    Ok(normalized)
}

#[derive(Serialize)]
struct TypographyImageUrl<'a> {
    url: &'a str,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TypographyContent<'a> {
    Text { text: &'a str },
    ImageUrl { image_url: TypographyImageUrl<'a> },
}

#[derive(Serialize)]
struct TypographyMessage<'a> {
    role: &'static str,
    content: Vec<TypographyContent<'a>>,
}

#[derive(Serialize)]
struct TypographyResponseFormat {
    r#type: &'static str,
}

#[derive(Serialize)]
struct TypographyChatRequest<'a> {
    model: &'a str,
    messages: Vec<TypographyMessage<'a>>,
    response_format: TypographyResponseFormat,
}

fn typography_request_body<'a>(
    model: &'a str,
    prompt: &'a str,
    image_data_url: &'a str,
) -> TypographyChatRequest<'a> {
    TypographyChatRequest {
        model,
        messages: vec![TypographyMessage {
            role: "user",
            content: vec![
                TypographyContent::Text { text: prompt },
                TypographyContent::ImageUrl {
                    image_url: TypographyImageUrl {
                        url: image_data_url,
                    },
                },
            ],
        }],
        response_format: TypographyResponseFormat {
            r#type: "json_object",
        },
    }
}

#[derive(Deserialize)]
struct TypographyChatResponse {
    choices: Vec<TypographyChoice>,
}

#[derive(Deserialize)]
struct TypographyChoice {
    message: TypographyResponseMessage,
}

#[derive(Deserialize)]
struct TypographyResponseMessage {
    content: String,
}

pub async fn send_typography_completion(
    http_client: Arc<ClientWithMiddleware>,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    prompt: &str,
    image_data_url: &str,
) -> anyhow::Result<String> {
    let endpoint = format!("{}/chat/completions", normalized_base_url(base_url)?);
    let mut request = http_client
        .post(endpoint)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&typography_request_body(
            model,
            prompt,
            image_data_url,
        ))?);
    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(api_key);
    }

    let response: TypographyChatResponse =
        ensure_provider_success("openai-compatible", request.send().await?)
            .await?
            .json()
            .await?;
    response
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .ok_or_else(|| anyhow::anyhow!("openai-compatible returned no content"))
}

pub async fn list_models(
    http_client: Arc<ClientWithMiddleware>,
    base_url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let endpoint = format!("{}/models", normalized_base_url(base_url)?);
    let mut request = http_client.get(endpoint);

    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(api_key);
    }

    let response = request.send().await?;
    let models: ModelsResponse = ensure_provider_success("openai-compatible", response)
        .await?
        .json()
        .await?;

    let mut ids = models
        .data
        .into_iter()
        .map(|model| model.id)
        .filter(|id| !id.trim().is_empty())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

impl AnyProvider for OpenAiCompatibleProvider {
    fn translate<'a>(
        &'a self,
        source: &'a str,
        target_language: Language,
        model: &'a str,
        custom_system_prompt: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let prompt = resolve_system_prompt(custom_system_prompt, target_language);
            send_chat_completion(
                Arc::clone(&self.http_client),
                ChatCompletionsRequest {
                    provider: "openai-compatible",
                    endpoint: format!("{}/chat/completions", normalized_base_url(&self.base_url)?),
                    auth: self
                        .api_key
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .map(|key| ChatCompletionsAuth::Bearer(key.to_string()))
                        .unwrap_or(ChatCompletionsAuth::None),
                    model: model.to_string(),
                    system_prompt: prompt,
                    user_prompt: source.to_string(),
                    temperature: self.temperature,
                    max_tokens: self.max_tokens,
                },
            )
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{normalized_base_url, typography_request_body};

    #[test]
    fn trims_trailing_slashes() {
        let normalized = normalized_base_url(" http://127.0.0.1:1234/v1/ ").unwrap();
        assert_eq!(normalized, "http://127.0.0.1:1234/v1");
    }

    #[test]
    fn openai_compatible_typography_request_contains_image_safe_regions_fonts_and_json_mode() {
        let prompt = r#"{"imageWidth":100,"imageHeight":200,"fonts":["ArialMT"],"nodes":[{"nodeId":"018f","safeRegions":[{"x":0.1,"y":0.2,"width":0.3,"height":0.4}]}]}"#;

        let body = typography_request_body("vision-model", prompt, "data:image/png;base64,cG5n");
        let value = serde_json::to_value(body).expect("serialize request");

        assert_eq!(value["model"], "vision-model");
        assert_eq!(value["response_format"]["type"], "json_object");
        assert_eq!(value["messages"][0]["content"][0]["type"], "text");
        assert_eq!(value["messages"][0]["content"][0]["text"], prompt);
        assert_eq!(value["messages"][0]["content"][1]["type"], "image_url");
        assert_eq!(
            value["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,cG5n"
        );
    }
}
