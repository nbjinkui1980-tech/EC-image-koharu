use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use base64::Engine as _;
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use koharu_core::{
    FontFaceInfo, FontSource, NodeDataPatch, NodeId, NodePatch, Op, PageId, Scene, TextAlign,
    TextDataPatch, TextShaderEffect, TextStrokeStyle, TextStyle,
};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::config::SourceTextPolicy;
use crate::pipeline::{eligible_lines_for_page, support::text_nodes};

#[cfg(test)]
use std::{
    sync::{Mutex, OnceLock},
    thread::ThreadId,
};

const PRODUCTION_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_IMAGE_DIMENSION: u32 = 1536;
const MAX_FONT_SIZE_PX: f32 = 300.0;
const MAX_STROKE_WIDTH_PX: f32 = 24.0;
const MAX_FONT_CANDIDATES: usize = 64;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TypographyDiagnosticOutcome {
    SkippedNoTargets,
    TimedOut,
    SenderFailed,
    ResponseRejected,
    Accepted,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TypographyFieldOutcome {
    Applied,
    IgnoredPreserveLines,
    ManualOverride,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TypographyAlignDiagnostic {
    Left,
    Center,
    Right,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TypographyEffectDiagnostic {
    pub(crate) italic: bool,
    pub(crate) bold: bool,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TypographyTargetDiagnostic {
    pub(crate) node_id: NodeId,
    pub(crate) preserve_lines: bool,
    pub(crate) safe_region_count: usize,
    pub(crate) planner_line_count: usize,
    pub(crate) translation_exactly_preserved: bool,
    pub(crate) line_outcome: TypographyFieldOutcome,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) current_font_size: Option<f32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) manual_font_size: Option<f32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) proposed_font_size: Option<f32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) resolved_font_size: Option<f32>,
    pub(crate) font_size_outcome: TypographyFieldOutcome,
    pub(crate) font_family_outcome: TypographyFieldOutcome,
    pub(crate) resolved_family_in_allowlist: bool,
    pub(crate) resolved_family_changed_current_first: bool,
    pub(crate) current_fill_rgba: [u8; 4],
    pub(crate) proposed_fill_rgba: [u8; 4],
    pub(crate) resolved_fill_rgba: [u8; 4],
    pub(crate) color_outcome: TypographyFieldOutcome,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) current_stroke_enabled: Option<bool>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) current_stroke_rgba: Option<[u8; 4]>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) current_stroke_width: Option<f32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) proposed_stroke_enabled: Option<bool>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) proposed_stroke_rgba: Option<[u8; 4]>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) proposed_stroke_width: Option<f32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) resolved_stroke_enabled: Option<bool>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) resolved_stroke_rgba: Option<[u8; 4]>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) resolved_stroke_width: Option<f32>,
    pub(crate) stroke_outcome: TypographyFieldOutcome,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) proposed_effect: Option<TypographyEffectDiagnostic>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) resolved_effect: Option<TypographyEffectDiagnostic>,
    pub(crate) effect_outcome: TypographyFieldOutcome,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) proposed_align: Option<TypographyAlignDiagnostic>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) resolved_align: Option<TypographyAlignDiagnostic>,
    pub(crate) align_outcome: TypographyFieldOutcome,
    pub(crate) typography_plan_verified: bool,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TypographyDiagnosticEvent {
    pub(crate) outcome: TypographyDiagnosticOutcome,
    pub(crate) target_count: usize,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) accepted_op_count: Option<usize>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub(crate) target_field_outcomes: Option<Vec<TypographyTargetDiagnostic>>,
}

fn deserialize_required_option<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

#[cfg(test)]
type TypographyDiagnosticEvents = Arc<Mutex<Vec<TypographyDiagnosticEvent>>>;

#[cfg(test)]
#[derive(Clone)]
struct TypographyDiagnosticSinkToken {
    events: TypographyDiagnosticEvents,
}

#[cfg(test)]
struct ActiveTypographyDiagnosticSink {
    owner: ThreadId,
    events: TypographyDiagnosticEvents,
}

#[cfg(test)]
static TYPOGRAPHY_DIAGNOSTIC_SINK: OnceLock<Mutex<Option<ActiveTypographyDiagnosticSink>>> =
    OnceLock::new();

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TypographyDiagnosticCaptureActive;

#[cfg(test)]
pub(crate) struct TypographyDiagnosticCapture {
    owner: ThreadId,
    events: TypographyDiagnosticEvents,
}

#[cfg(test)]
impl TypographyDiagnosticCapture {
    pub(crate) fn start() -> std::result::Result<Self, TypographyDiagnosticCaptureActive> {
        let owner = std::thread::current().id();
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut active = TYPOGRAPHY_DIAGNOSTIC_SINK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.is_some() {
            return Err(TypographyDiagnosticCaptureActive);
        }
        *active = Some(ActiveTypographyDiagnosticSink {
            owner,
            events: events.clone(),
        });
        Ok(Self { owner, events })
    }

    pub(crate) fn take(&self) -> Vec<TypographyDiagnosticEvent> {
        std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

#[cfg(test)]
impl Drop for TypographyDiagnosticCapture {
    fn drop(&mut self) {
        let mut active = TYPOGRAPHY_DIAGNOSTIC_SINK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active
            .as_ref()
            .is_some_and(|sink| sink.owner == self.owner && Arc::ptr_eq(&sink.events, &self.events))
        {
            *active = None;
        }
    }
}

#[cfg(test)]
fn current_typography_diagnostic_sink() -> Option<TypographyDiagnosticSinkToken> {
    let owner = std::thread::current().id();
    TYPOGRAPHY_DIAGNOSTIC_SINK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|sink| sink.owner == owner)
        .map(|sink| TypographyDiagnosticSinkToken {
            events: sink.events.clone(),
        })
}

#[cfg(test)]
fn record_typography_diagnostic_with(
    sink: Option<&TypographyDiagnosticSinkToken>,
    event: TypographyDiagnosticEvent,
) {
    if let Some(sink) = sink {
        sink.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

#[cfg(test)]
fn record_typography_outcome_with(
    sink: Option<&TypographyDiagnosticSinkToken>,
    outcome: TypographyDiagnosticOutcome,
    target_count: usize,
) {
    record_typography_diagnostic_with(
        sink,
        TypographyDiagnosticEvent {
            outcome,
            target_count,
            accepted_op_count: None,
            target_field_outcomes: None,
        },
    );
}

#[cfg(test)]
fn record_typography_outcome(outcome: TypographyDiagnosticOutcome, target_count: usize) {
    let sink = current_typography_diagnostic_sink();
    record_typography_outcome_with(sink.as_ref(), outcome, target_count);
}

#[cfg(test)]
struct TypographyResponseDiagnosticGuard {
    target_count: usize,
    sink: Option<TypographyDiagnosticSinkToken>,
    accepted: bool,
}

#[cfg(test)]
impl TypographyResponseDiagnosticGuard {
    fn new(target_count: usize, sink: Option<&TypographyDiagnosticSinkToken>) -> Self {
        Self {
            target_count,
            sink: sink.cloned(),
            accepted: false,
        }
    }

    fn accept(
        &mut self,
        accepted_op_count: usize,
        target_field_outcomes: Vec<TypographyTargetDiagnostic>,
    ) {
        record_typography_diagnostic_with(
            self.sink.as_ref(),
            TypographyDiagnosticEvent {
                outcome: TypographyDiagnosticOutcome::Accepted,
                target_count: self.target_count,
                accepted_op_count: Some(accepted_op_count),
                target_field_outcomes: Some(target_field_outcomes),
            },
        );
        self.accepted = true;
    }
}

#[cfg(test)]
impl Drop for TypographyResponseDiagnosticGuard {
    fn drop(&mut self) {
        if !self.accepted {
            record_typography_outcome_with(
                self.sink.as_ref(),
                TypographyDiagnosticOutcome::ResponseRejected,
                self.target_count,
            );
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypographyPageRequest {
    #[serde(skip)]
    pub page: PageId,
    pub image_width: u32,
    pub image_height: u32,
    pub fonts: Vec<String>,
    pub targets: Vec<TypographyTarget>,
    #[serde(skip)]
    image_data_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypographyTarget {
    pub node_id: NodeId,
    pub image_width: u32,
    pub image_height: u32,
    pub translation: String,
    pub current_style: TextStyle,
    pub safe_regions: Vec<NormalizedRegion>,
    pub preserve_lines: bool,
    #[serde(skip)]
    active_font_hints: Vec<String>,
    #[serde(skip)]
    detected_font_hints: Vec<String>,
    #[serde(skip)]
    manual_font_size: Option<f32>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRegion {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TypographyPlan {
    nodes: Vec<PlannedNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedNode {
    node_id: NodeId,
    lines: Vec<String>,
    style: PlannedStyle,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedStyle {
    font_family: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    font_size: Option<f32>,
    color: [u8; 4],
    #[serde(deserialize_with = "deserialize_required_option")]
    stroke: Option<PlannedStroke>,
    #[serde(deserialize_with = "deserialize_required_option")]
    effect: Option<PlannedEffect>,
    #[serde(deserialize_with = "deserialize_required_option")]
    text_align: Option<TextAlign>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedStroke {
    enabled: bool,
    color: [u8; 4],
    #[serde(deserialize_with = "deserialize_required_option")]
    width_px: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedEffect {
    italic: bool,
    bold: bool,
}

/// Stateless cloud planner.  It deliberately owns only the live connection
/// source and shared runtime client; translation keeps its separately loaded
/// provider instance.
#[derive(Default)]
pub struct TypographyPlanner {
    config: Option<Arc<ArcSwap<AppConfig>>>,
    http_client: Option<koharu_runtime::RuntimeHttpClient>,
    #[cfg(test)]
    test_sender: Option<TypographyTestSender>,
    #[cfg(test)]
    test_timeout: Option<Duration>,
}

#[cfg(test)]
pub(crate) type TypographyTestSender = Arc<
    dyn Fn(String, String) -> crate::pipeline::BoxFuture<'static, Result<String>> + Send + Sync,
>;

impl TypographyPlanner {
    pub fn new(
        config: Arc<ArcSwap<AppConfig>>,
        http_client: koharu_runtime::RuntimeHttpClient,
    ) -> Self {
        Self {
            config: Some(config),
            http_client: Some(http_client),
            #[cfg(test)]
            test_sender: None,
            #[cfg(test)]
            test_timeout: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_sender(sender: TypographyTestSender, timeout: Duration) -> Self {
        Self {
            test_sender: Some(sender),
            test_timeout: Some(timeout),
            ..Self::default()
        }
    }

    /// Plan a page using the current persisted OpenAI-compatible connection.
    /// Configuration is loaded for every call so URL/key changes take effect
    /// without rebuilding the independently loaded translator.
    pub async fn plan_page(&self, request: &TypographyPageRequest) -> Result<Vec<Op>> {
        if request.targets.is_empty() {
            #[cfg(test)]
            record_typography_outcome(TypographyDiagnosticOutcome::SkippedNoTargets, 0);
            return Ok(Vec::new());
        }
        #[cfg(test)]
        if let Some(sender) = self.test_sender.clone() {
            return self
                .plan_with_timeout(
                    request,
                    move |prompt, image| sender(prompt, image),
                    self.test_timeout.unwrap_or(PRODUCTION_TIMEOUT),
                )
                .await;
        }
        let (client, base_url, api_key, model, timeout) = self.connection_settings()?;

        self.plan_with_timeout(
            request,
            move |prompt, image_data_url| async move {
                koharu_llm::providers::openai_compatible::send_typography_completion(
                    client,
                    &base_url,
                    api_key.as_deref(),
                    &model,
                    &prompt,
                    &image_data_url,
                )
                .await
            },
            timeout,
        )
        .await
    }

    fn connection_settings(
        &self,
    ) -> Result<(
        koharu_runtime::RuntimeHttpClient,
        String,
        Option<String>,
        String,
        Duration,
    )> {
        let config = self
            .config
            .as_ref()
            .context("Typography Planner is not configured")?
            .load();
        let provider = config
            .providers
            .iter()
            .find(|provider| provider.id == "openai-compatible")
            .context("OpenAI-compatible provider is not configured")?;
        let base_url = provider
            .base_url
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .context("OpenAI-compatible base URL is not configured")?
            .to_owned();
        let api_key = provider.api_key.as_ref().map(|key| key.expose().to_owned());
        let model = config
            .typography_planner
            .model_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .context("Typography Planner model is not configured")?
            .to_owned();
        let timeout = Duration::from_secs(config.http.read_timeout.max(1));
        let client = self
            .http_client
            .as_ref()
            .context("Typography Planner HTTP client is not configured")?
            .clone();

        Ok((client, base_url, api_key, model, timeout))
    }

    pub async fn plan<Send, SendFuture>(
        &self,
        request: &TypographyPageRequest,
        sender: Send,
    ) -> Result<Vec<Op>>
    where
        Send: FnOnce(String, String) -> SendFuture,
        SendFuture: Future<Output = Result<String>>,
    {
        self.plan_with_timeout(request, sender, PRODUCTION_TIMEOUT)
            .await
    }

    async fn plan_with_timeout<Send, SendFuture>(
        &self,
        request: &TypographyPageRequest,
        sender: Send,
        timeout: Duration,
    ) -> Result<Vec<Op>>
    where
        Send: FnOnce(String, String) -> SendFuture,
        SendFuture: Future<Output = Result<String>>,
    {
        if request.targets.is_empty() {
            #[cfg(test)]
            record_typography_outcome(TypographyDiagnosticOutcome::SkippedNoTargets, 0);
            return Ok(Vec::new());
        }
        #[cfg(test)]
        let diagnostic_sink = current_typography_diagnostic_sink();
        let payload = serde_json::to_string(request)?;
        let prompt = format!(
            "Return only this strict JSON shape with exactly one result per input node and no extra fields: {{\"nodes\":[{{\"nodeId\":\"uuid\",\"lines\":[\"text\"],\"style\":{{\"fontFamily\":\"allowed PostScript name\",\"fontSize\":null,\"color\":[0,0,0,255],\"stroke\":null,\"effect\":null,\"textAlign\":null}}}}]}}. For targets with preserveLines=true, lines must exactly equal the explicit input lines and fontSize must be null. Otherwise preserve every character and whitespace; a boundary between lines may only insert a line break or replace one existing ASCII space/newline. Input: {payload}"
        );
        #[cfg(not(test))]
        let response =
            tokio::time::timeout(timeout, sender(prompt, request.image_data_url.clone()))
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "typography planner request timed out after {} seconds",
                        timeout.as_secs()
                    )
                })??;
        #[cfg(test)]
        let response =
            match tokio::time::timeout(timeout, sender(prompt, request.image_data_url.clone()))
                .await
            {
                Err(_) => {
                    record_typography_outcome_with(
                        diagnostic_sink.as_ref(),
                        TypographyDiagnosticOutcome::TimedOut,
                        request.targets.len(),
                    );
                    return Err(anyhow::anyhow!(
                        "typography planner request timed out after {} seconds",
                        timeout.as_secs()
                    ));
                }
                Ok(Err(error)) => {
                    record_typography_outcome_with(
                        diagnostic_sink.as_ref(),
                        TypographyDiagnosticOutcome::SenderFailed,
                        request.targets.len(),
                    );
                    return Err(error);
                }
                Ok(Ok(response)) => response,
            };
        #[cfg(test)]
        {
            build_typography_ops_inner(request, &response, diagnostic_sink.as_ref())
        }
        #[cfg(not(test))]
        {
            build_typography_ops(request, &response)
        }
    }
}

pub fn build_typography_request(
    scene: &Scene,
    page: PageId,
    source_image: &DynamicImage,
    available_fonts: &[FontFaceInfo],
    policy: SourceTextPolicy,
    allowed_ids: Option<&[NodeId]>,
    default_font: Option<&str>,
) -> Result<TypographyPageRequest> {
    let page_ref = scene
        .page(page)
        .with_context(|| format!("page {page} not found"))?;
    let (image_width, image_height) = source_image.dimensions();
    anyhow::ensure!(
        (image_width, image_height) == (page_ref.width, page_ref.height),
        "Typography source image dimensions differ from the page"
    );

    let mut han_regions: HashMap<NodeId, Vec<NormalizedRegion>> = HashMap::new();
    if policy == SourceTextPolicy::HanOnly {
        for (node_id, line) in eligible_lines_for_page(scene, page).0 {
            if allowed_ids.is_some_and(|ids| !ids.contains(&node_id)) {
                continue;
            }
            han_regions
                .entry(node_id)
                .or_default()
                .push(normalize_region(
                    line.region.x,
                    line.region.y,
                    line.region.width,
                    line.region.height,
                    image_width,
                    image_height,
                )?);
        }
    }

    let mut targets = Vec::new();
    for (node_id, transform, text) in text_nodes(scene, page) {
        if allowed_ids.is_some_and(|ids| !ids.contains(&node_id)) {
            continue;
        }
        let Some(translation) = text
            .translation
            .as_deref()
            .filter(|translation| !translation.trim().is_empty())
        else {
            continue;
        };
        let safe_regions = if policy == SourceTextPolicy::HanOnly {
            let Some(regions) = han_regions.remove(&node_id) else {
                continue;
            };
            regions
        } else {
            vec![normalize_region(
                transform.x,
                transform.y,
                transform.width,
                transform.height,
                image_width,
                image_height,
            )?]
        };
        let current_style = text.style.clone().unwrap_or_default();
        let preserve_lines = policy == SourceTextPolicy::HanOnly;
        let manual_font_size =
            if policy != SourceTextPolicy::HanOnly || text.typography_plan_verified {
                None
            } else {
                current_style
                    .font_size
                    .filter(|size| size.is_finite() && *size > 0.0)
            };
        targets.push(TypographyTarget {
            node_id,
            image_width,
            image_height,
            translation: translation.to_string(),
            active_font_hints: current_style.font_families.clone(),
            detected_font_hints: text
                .font_prediction
                .as_ref()
                .into_iter()
                .flat_map(|prediction| prediction.named_fonts.iter())
                .map(|font| font.name.clone())
                .collect(),
            current_style,
            safe_regions,
            preserve_lines,
            manual_font_size,
        });
    }

    let fonts = typography_font_candidates(available_fonts, &targets, default_font);
    anyhow::ensure!(
        targets.is_empty() || !fonts.is_empty(),
        "no safe fonts available"
    );
    Ok(TypographyPageRequest {
        page,
        image_width,
        image_height,
        fonts,
        targets,
        image_data_url: image_data_url(source_image)?,
    })
}

fn normalize_region(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    image_width: u32,
    image_height: u32,
) -> Result<NormalizedRegion> {
    anyhow::ensure!(image_width > 0 && image_height > 0, "empty source image");
    anyhow::ensure!(
        [x, y, width, height].iter().all(|value| value.is_finite())
            && x >= 0.0
            && y >= 0.0
            && width > 0.0
            && height > 0.0
            && x + width <= image_width as f32
            && y + height <= image_height as f32,
        "unsafe Typography target geometry"
    );
    Ok(NormalizedRegion {
        x: x / image_width as f32,
        y: y / image_height as f32,
        width: width / image_width as f32,
        height: height / image_height as f32,
    })
}

fn typography_font_candidates(
    available_fonts: &[FontFaceInfo],
    targets: &[TypographyTarget],
    default_font: Option<&str>,
) -> Vec<String> {
    let mut safe_fonts = available_fonts
        .iter()
        .filter(|font| font.source == FontSource::System || font.cached)
        .collect::<Vec<_>>();
    safe_fonts.sort_by(|a, b| a.post_script_name.cmp(&b.post_script_name));

    let mut lookup = HashMap::new();
    for font in &safe_fonts {
        lookup.insert(
            font.post_script_name.trim().to_lowercase(),
            font.post_script_name.clone(),
        );
        lookup
            .entry(font.family_name.trim().to_lowercase())
            .or_insert_with(|| font.post_script_name.clone());
    }

    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |hint: &str| {
        let Some(post_script_name) = lookup.get(&hint.trim().to_lowercase()) else {
            return;
        };
        if seen.insert(post_script_name.to_lowercase()) {
            candidates.push(post_script_name.clone());
        }
    };
    for target in targets {
        for hint in &target.active_font_hints {
            add(hint);
        }
    }
    for target in targets {
        for hint in &target.detected_font_hints {
            add(hint);
        }
    }
    if let Some(default_font) = default_font {
        add(default_font);
    }
    for font in safe_fonts {
        add(&font.post_script_name);
    }
    // ponytail: 64 keeps prompts bounded; raise it only if real samples prove coverage is lacking.
    candidates.truncate(MAX_FONT_CANDIDATES);
    candidates
}

fn image_data_url(image: &DynamicImage) -> Result<String> {
    let (width, height) = image.dimensions();
    let image = if width.max(height) > MAX_IMAGE_DIMENSION {
        image.resize(
            MAX_IMAGE_DIMENSION,
            MAX_IMAGE_DIMENSION,
            FilterType::Lanczos3,
        )
    } else {
        image.clone()
    };
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
    ))
}

pub fn build_typography_ops(request: &TypographyPageRequest, response: &str) -> Result<Vec<Op>> {
    #[cfg(test)]
    let diagnostic_sink = current_typography_diagnostic_sink();
    build_typography_ops_inner(
        request,
        response,
        #[cfg(test)]
        diagnostic_sink.as_ref(),
    )
}

fn build_typography_ops_inner(
    request: &TypographyPageRequest,
    response: &str,
    #[cfg(test)] diagnostic_sink: Option<&TypographyDiagnosticSinkToken>,
) -> Result<Vec<Op>> {
    #[cfg(test)]
    let mut diagnostic_guard =
        TypographyResponseDiagnosticGuard::new(request.targets.len(), diagnostic_sink);
    let plan: TypographyPlan =
        serde_json::from_str(response).context("invalid strict Typography Planner response")?;
    anyhow::ensure!(
        plan.nodes.len() == request.targets.len(),
        "Typography response node coverage mismatch"
    );

    let target_ids = request
        .targets
        .iter()
        .map(|target| target.node_id)
        .collect::<HashSet<_>>();
    let mut planned = HashMap::with_capacity(plan.nodes.len());
    for node in plan.nodes {
        anyhow::ensure!(
            target_ids.contains(&node.node_id),
            "unknown Typography response node"
        );
        anyhow::ensure!(
            planned.insert(node.node_id, node).is_none(),
            "duplicate Typography response node"
        );
    }

    let font_lookup = request
        .fonts
        .iter()
        .map(|font| (font.to_lowercase(), font.as_str()))
        .collect::<HashMap<_, _>>();
    let min_font_size =
        crate::renderer::min_font_size_for_image(request.image_width, request.image_height);
    let mut validated = Vec::with_capacity(request.targets.len());
    #[cfg(test)]
    let mut target_diagnostics = Vec::with_capacity(request.targets.len());
    for target in &request.targets {
        let node = planned
            .remove(&target.node_id)
            .ok_or_else(|| anyhow::anyhow!("missing Typography response node"))?;
        #[cfg(test)]
        let proposed = ProposedTypographyDiagnostic::from_node(&node);
        let (translation, planned_font_size) = if target.preserve_lines {
            (target.translation.clone(), None)
        } else {
            validate_lines(target, &node.lines)?;
            (node.lines.join("\n"), node.style.font_size)
        };
        let font_family = font_lookup
            .get(&node.style.font_family.trim().to_lowercase())
            .ok_or_else(|| anyhow::anyhow!("unknown Typography font"))?
            .to_string();
        if let Some(font_size) = planned_font_size {
            anyhow::ensure!(
                font_size.is_finite()
                    && font_size >= min_font_size
                    && font_size <= MAX_FONT_SIZE_PX,
                "invalid Typography font size"
            );
        }
        if let Some(width) = node
            .style
            .stroke
            .as_ref()
            .and_then(|stroke| stroke.width_px)
        {
            anyhow::ensure!(
                width.is_finite() && (0.0..=MAX_STROKE_WIDTH_PX).contains(&width),
                "invalid Typography stroke width"
            );
        }
        let style = TextStyle {
            font_families: vec![font_family],
            font_size: target.manual_font_size.or(planned_font_size),
            color: node.style.color,
            stroke: node.style.stroke.map(|stroke| TextStrokeStyle {
                enabled: stroke.enabled,
                color: stroke.color,
                width_px: stroke.width_px,
            }),
            effect: node.style.effect.map(|effect| TextShaderEffect {
                italic: effect.italic,
                bold: effect.bold,
            }),
            text_align: node.style.text_align,
        };
        #[cfg(test)]
        target_diagnostics.push(typography_target_diagnostic(
            request,
            target,
            &translation,
            &style,
            proposed,
        ));
        validated.push((
            target.node_id,
            translation,
            style,
            target.manual_font_size.is_none(),
        ));
    }
    anyhow::ensure!(planned.is_empty(), "unknown Typography response node");

    let ops = validated
        .into_iter()
        .map(
            |(node_id, translation, style, typography_plan_verified)| Op::UpdateNode {
                page: request.page,
                id: node_id,
                patch: NodePatch {
                    data: Some(NodeDataPatch::Text(TextDataPatch {
                        translation: Some(Some(translation)),
                        style: Some(Some(style)),
                        sprite: Some(None),
                        sprite_transform: Some(None),
                        typography_plan_verified: Some(typography_plan_verified),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                prev: NodePatch::default(),
            },
        )
        .collect::<Vec<_>>();
    #[cfg(test)]
    diagnostic_guard.accept(ops.len(), target_diagnostics);
    Ok(ops)
}

#[cfg(test)]
struct ProposedTypographyDiagnostic {
    planner_line_count: usize,
    font_size: Option<f32>,
    fill_rgba: [u8; 4],
    stroke_enabled: Option<bool>,
    stroke_rgba: Option<[u8; 4]>,
    stroke_width: Option<f32>,
    effect: Option<TypographyEffectDiagnostic>,
    align: Option<TypographyAlignDiagnostic>,
}

#[cfg(test)]
impl ProposedTypographyDiagnostic {
    fn from_node(node: &PlannedNode) -> Self {
        Self {
            planner_line_count: node.lines.len(),
            font_size: finite_typography_number(node.style.font_size),
            fill_rgba: node.style.color,
            stroke_enabled: node.style.stroke.as_ref().map(|stroke| stroke.enabled),
            stroke_rgba: node.style.stroke.as_ref().map(|stroke| stroke.color),
            stroke_width: finite_typography_number(
                node.style
                    .stroke
                    .as_ref()
                    .and_then(|stroke| stroke.width_px),
            ),
            effect: node
                .style
                .effect
                .as_ref()
                .map(|effect| TypographyEffectDiagnostic {
                    italic: effect.italic,
                    bold: effect.bold,
                }),
            align: node.style.text_align.map(typography_align_diagnostic),
        }
    }
}

#[cfg(test)]
fn typography_target_diagnostic(
    request: &TypographyPageRequest,
    target: &TypographyTarget,
    translation: &str,
    resolved: &TextStyle,
    proposed: ProposedTypographyDiagnostic,
) -> TypographyTargetDiagnostic {
    let current_stroke = target.current_style.stroke.as_ref();
    let resolved_stroke = resolved.stroke.as_ref();
    let line_outcome = if target.preserve_lines {
        TypographyFieldOutcome::IgnoredPreserveLines
    } else {
        TypographyFieldOutcome::Applied
    };
    let font_size_outcome = if target.manual_font_size.is_some() {
        TypographyFieldOutcome::ManualOverride
    } else if target.preserve_lines {
        TypographyFieldOutcome::IgnoredPreserveLines
    } else {
        TypographyFieldOutcome::Applied
    };
    let resolved_family = resolved
        .font_families
        .first()
        .expect("validated Typography style has one font family");
    TypographyTargetDiagnostic {
        node_id: target.node_id,
        preserve_lines: target.preserve_lines,
        safe_region_count: target.safe_regions.len(),
        planner_line_count: proposed.planner_line_count,
        translation_exactly_preserved: translation == target.translation,
        line_outcome,
        current_font_size: finite_typography_number(target.current_style.font_size),
        manual_font_size: finite_typography_number(target.manual_font_size),
        proposed_font_size: proposed.font_size,
        resolved_font_size: finite_typography_number(resolved.font_size),
        font_size_outcome,
        font_family_outcome: TypographyFieldOutcome::Applied,
        resolved_family_in_allowlist: request.fonts.iter().any(|font| font == resolved_family),
        resolved_family_changed_current_first: target.current_style.font_families.first()
            != Some(resolved_family),
        current_fill_rgba: target.current_style.color,
        proposed_fill_rgba: proposed.fill_rgba,
        resolved_fill_rgba: resolved.color,
        color_outcome: TypographyFieldOutcome::Applied,
        current_stroke_enabled: current_stroke.map(|stroke| stroke.enabled),
        current_stroke_rgba: current_stroke.map(|stroke| stroke.color),
        current_stroke_width: finite_typography_number(
            current_stroke.and_then(|stroke| stroke.width_px),
        ),
        proposed_stroke_enabled: proposed.stroke_enabled,
        proposed_stroke_rgba: proposed.stroke_rgba,
        proposed_stroke_width: proposed.stroke_width,
        resolved_stroke_enabled: resolved_stroke.map(|stroke| stroke.enabled),
        resolved_stroke_rgba: resolved_stroke.map(|stroke| stroke.color),
        resolved_stroke_width: finite_typography_number(
            resolved_stroke.and_then(|stroke| stroke.width_px),
        ),
        stroke_outcome: TypographyFieldOutcome::Applied,
        proposed_effect: proposed.effect,
        resolved_effect: resolved.effect.map(|effect| TypographyEffectDiagnostic {
            italic: effect.italic,
            bold: effect.bold,
        }),
        effect_outcome: TypographyFieldOutcome::Applied,
        proposed_align: proposed.align,
        resolved_align: resolved.text_align.map(typography_align_diagnostic),
        align_outcome: TypographyFieldOutcome::Applied,
        typography_plan_verified: target.manual_font_size.is_none(),
    }
}

#[cfg(test)]
fn finite_typography_number(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite())
}

#[cfg(test)]
fn typography_align_diagnostic(value: TextAlign) -> TypographyAlignDiagnostic {
    match value {
        TextAlign::Left => TypographyAlignDiagnostic::Left,
        TextAlign::Center => TypographyAlignDiagnostic::Center,
        TextAlign::Right => TypographyAlignDiagnostic::Right,
    }
}

fn validate_lines(target: &TypographyTarget, lines: &[String]) -> Result<()> {
    anyhow::ensure!(
        !lines.is_empty()
            && lines
                .iter()
                .all(|line| !line.is_empty() && !line.contains(['\n', '\r'])),
        "Typography response contains an empty or embedded line"
    );
    if target.safe_regions.len() != 1 {
        anyhow::ensure!(
            lines
                == target
                    .translation
                    .split('\n')
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            "Typography reflow crosses multiple safe regions"
        );
        return Ok(());
    }
    anyhow::ensure!(
        lines_preserve_text(&target.translation, lines),
        "Typography response changed text or whitespace"
    );
    Ok(())
}

fn lines_preserve_text(original: &str, lines: &[String]) -> bool {
    fn matches_from(
        original: &str,
        lines: &[String],
        line_index: usize,
        offset: usize,
        seen: &mut HashSet<(usize, usize)>,
    ) -> bool {
        if !seen.insert((line_index, offset)) {
            return false;
        }
        let Some(rest) = original.get(offset..) else {
            return false;
        };
        let line = &lines[line_index];
        if !rest.starts_with(line) {
            return false;
        }
        let next = offset + line.len();
        if line_index + 1 == lines.len() {
            return next == original.len();
        }
        let Some(separator) = original.get(next..).and_then(|rest| rest.chars().next()) else {
            return false;
        };
        matches!(separator, ' ' | '\n')
            && matches_from(
                original,
                lines,
                line_index + 1,
                next + separator.len_utf8(),
                seen,
            )
    }

    !lines.is_empty() && matches_from(original, lines, 0, 0, &mut HashSet::new())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    use anyhow::Result;
    use image::DynamicImage;
    use koharu_core::{
        FontFaceInfo, FontPrediction, FontSource, NamedFontPrediction, Node, NodeDataPatch, NodeId,
        NodeKind, Page, Scene, TextData, TextStyle, Transform,
    };
    use serde_json::{Value, json};

    use super::*;
    use crate::config::SourceTextPolicy;

    fn font(
        family: &str,
        post_script_name: &str,
        source: FontSource,
        cached: bool,
    ) -> FontFaceInfo {
        FontFaceInfo {
            family_name: family.to_string(),
            post_script_name: post_script_name.to_string(),
            source,
            category: None,
            cached,
        }
    }

    fn fonts() -> Vec<FontFaceInfo> {
        vec![
            font("Arial", "ArialMT", FontSource::System, true),
            font(
                "Noto Sans SC",
                "NotoSansSC-Regular",
                FontSource::Google,
                true,
            ),
            font("Uncached", "Uncached-Regular", FontSource::Google, false),
        ]
    }

    fn text_node(text: &str, translation: Option<&str>) -> Node {
        let id = NodeId::new();
        Node {
            id,
            transform: Transform {
                x: 10.0,
                y: 20.0,
                width: 60.0,
                height: 30.0,
                rotation_deg: 0.0,
            },
            visible: true,
            kind: NodeKind::Text(TextData {
                text: Some(text.to_string()),
                translation: translation.map(str::to_string),
                ..Default::default()
            }),
        }
    }

    fn scene(nodes: Vec<Node>) -> (Scene, koharu_core::PageId) {
        let mut page = Page::new("page", 100, 100);
        let page_id = page.id;
        page.nodes = nodes.into_iter().map(|node| (node.id, node)).collect();
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        (scene, page_id)
    }

    fn request(
        scene: &Scene,
        page: koharu_core::PageId,
        policy: SourceTextPolicy,
        scope: Option<&[NodeId]>,
    ) -> Result<TypographyPageRequest> {
        build_typography_request(
            scene,
            page,
            &DynamicImage::new_rgba8(100, 100),
            &fonts(),
            policy,
            scope,
            Some("Arial"),
        )
    }

    fn response_node(target: &TypographyTarget, lines: Vec<String>) -> Value {
        json!({
            "nodeId": target.node_id,
            "lines": lines,
            "style": {
                "fontFamily": "ArialMT",
                "fontSize": if target.preserve_lines { Value::Null } else { json!(18.0) },
                "color": [1, 2, 3, 255],
                "stroke": {
                    "enabled": true,
                    "color": [255, 255, 255, 255],
                    "widthPx": 2.0
                },
                "effect": { "italic": false, "bold": false },
                "textAlign": "center"
            }
        })
    }

    fn start_typography_diagnostic_capture() -> TypographyDiagnosticCapture {
        loop {
            match TypographyDiagnosticCapture::start() {
                Ok(capture) => return capture,
                Err(TypographyDiagnosticCaptureActive) => std::thread::yield_now(),
            }
        }
    }

    fn build_with_typography_diagnostics(
        request: &TypographyPageRequest,
        response: &str,
    ) -> Result<(Vec<Op>, TypographyDiagnosticEvent)> {
        let capture = start_typography_diagnostic_capture();
        let ops = build_typography_ops(request, response)?;
        let events = capture.take();
        assert_eq!(events.len(), 1);
        Ok((ops, events.into_iter().next().unwrap()))
    }

    fn apply_ops(mut scene: Scene, ops: &[Op]) -> Result<Scene> {
        for mut op in ops.iter().cloned() {
            op.apply(&mut scene)?;
        }
        Ok(scene)
    }

    fn text_patch(op: &Op) -> (&str, &TextStyle, bool) {
        let Op::UpdateNode { patch, .. } = op else {
            panic!("expected update")
        };
        let Some(NodeDataPatch::Text(patch)) = &patch.data else {
            panic!("expected text patch")
        };
        (
            patch
                .translation
                .as_ref()
                .expect("translation patch")
                .as_deref()
                .expect("translation"),
            patch
                .style
                .as_ref()
                .expect("style patch")
                .as_ref()
                .expect("style"),
            patch.typography_plan_verified.expect("typography marker"),
        )
    }

    fn planned_node_with_style(
        target: &TypographyTarget,
        lines: Vec<String>,
        font_size: f32,
    ) -> Value {
        let mut node = response_node(target, lines);
        node["style"]["fontSize"] = json!(font_size);
        node["style"]["color"] = json!([240, 241, 242, 255]);
        node["style"]["stroke"] = json!({
            "enabled": true,
            "color": [12, 13, 14, 255],
            "widthPx": 3.0
        });
        node["style"]["effect"] = json!({ "italic": true, "bold": true });
        node["style"]["textAlign"] = json!("right");
        node
    }

    fn json_contains_key(value: &Value, needle: &str) -> bool {
        match value {
            Value::Object(fields) => {
                fields.contains_key(needle)
                    || fields
                        .values()
                        .any(|value| json_contains_key(value, needle))
            }
            Value::Array(values) => values.iter().any(|value| json_contains_key(value, needle)),
            _ => false,
        }
    }

    #[test]
    fn typography_diagnostics_match_accepted_han_and_all_text_ops_without_drift() -> Result<()> {
        let mut automatic = text_node("中文", Some("automatic"));
        let NodeKind::Text(automatic_text) = &mut automatic.kind else {
            unreachable!()
        };
        automatic_text.style = Some(TextStyle {
            font_families: vec!["NotoSansSC-Regular".into()],
            color: [10, 11, 12, 255],
            stroke: Some(TextStrokeStyle {
                enabled: false,
                color: [20, 21, 22, 255],
                width_px: None,
            }),
            ..Default::default()
        });
        let mut manual = text_node("汉字", Some("manual"));
        let NodeKind::Text(manual_text) = &mut manual.kind else {
            unreachable!()
        };
        manual_text.style = Some(TextStyle {
            font_families: vec!["ArialMT".into()],
            font_size: Some(72.0),
            color: [30, 31, 32, 255],
            ..Default::default()
        });
        manual_text.typography_plan_verified = false;
        let (han_scene, han_page) = scene(vec![automatic, manual]);
        let han_request = request(&han_scene, han_page, SourceTextPolicy::HanOnly, None)?;
        let han_response = serde_json::to_string(&json!({
            "nodes": han_request
                .targets
                .iter()
                .map(|target| {
                    planned_node_with_style(target, vec!["planner rewrite".into()], 19.0)
                })
                .collect::<Vec<_>>()
        }))?;

        let inactive_han = build_typography_ops(&han_request, &han_response)?;
        let (active_han, han_event) =
            build_with_typography_diagnostics(&han_request, &han_response)?;
        assert_eq!(
            serde_json::to_value(&inactive_han)?,
            serde_json::to_value(&active_han)?
        );
        assert_eq!(
            serde_json::to_value(apply_ops(han_scene.clone(), &inactive_han)?)?,
            serde_json::to_value(apply_ops(han_scene, &active_han)?)?
        );
        assert_eq!(han_event.outcome, TypographyDiagnosticOutcome::Accepted);
        assert_eq!(han_event.target_count, 2);
        assert_eq!(han_event.accepted_op_count, Some(2));
        let han_targets = han_event.target_field_outcomes.as_ref().unwrap();
        assert_eq!(
            han_targets
                .iter()
                .map(|target| target.node_id)
                .collect::<Vec<_>>(),
            han_request
                .targets
                .iter()
                .map(|target| target.node_id)
                .collect::<Vec<_>>()
        );
        let expected_current_fills = han_request
            .targets
            .iter()
            .map(|target| target.current_style.color)
            .collect::<Vec<_>>();
        assert_eq!(
            han_targets
                .iter()
                .map(|target| target.current_fill_rgba)
                .collect::<Vec<_>>(),
            expected_current_fills
        );
        for target in han_targets {
            assert!(target.preserve_lines);
            assert_eq!(
                target.line_outcome,
                TypographyFieldOutcome::IgnoredPreserveLines
            );
            assert!(target.translation_exactly_preserved);
            assert_eq!(target.planner_line_count, 1);
            assert!(target.safe_region_count >= 1);
            assert_eq!(target.proposed_font_size, Some(19.0));
            assert_eq!(target.font_family_outcome, TypographyFieldOutcome::Applied);
            assert!(target.resolved_family_in_allowlist);
            assert_eq!(target.color_outcome, TypographyFieldOutcome::Applied);
            assert_eq!(target.stroke_outcome, TypographyFieldOutcome::Applied);
            assert_eq!(target.effect_outcome, TypographyFieldOutcome::Applied);
            assert_eq!(target.align_outcome, TypographyFieldOutcome::Applied);
            assert_eq!(target.proposed_fill_rgba, [240, 241, 242, 255]);
            assert_eq!(target.resolved_fill_rgba, [240, 241, 242, 255]);
            assert_eq!(target.proposed_stroke_rgba, Some([12, 13, 14, 255]));
            assert_eq!(target.resolved_stroke_rgba, Some([12, 13, 14, 255]));
        }
        let automatic_target = han_targets
            .iter()
            .find(|target| target.manual_font_size.is_none())
            .expect("automatic HanOnly target");
        assert_eq!(
            automatic_target.font_size_outcome,
            TypographyFieldOutcome::IgnoredPreserveLines
        );
        assert_eq!(automatic_target.resolved_font_size, None);
        assert!(automatic_target.typography_plan_verified);
        assert!(automatic_target.resolved_family_changed_current_first);
        let manual_target = han_targets
            .iter()
            .find(|target| target.manual_font_size.is_some())
            .expect("manual HanOnly target");
        assert_eq!(
            manual_target.font_size_outcome,
            TypographyFieldOutcome::ManualOverride
        );
        assert_eq!(manual_target.current_font_size, Some(72.0));
        assert_eq!(manual_target.manual_font_size, Some(72.0));
        assert_eq!(manual_target.resolved_font_size, Some(72.0));
        assert!(!manual_target.typography_plan_verified);
        assert!(!manual_target.resolved_family_changed_current_first);
        for (event, op) in han_targets.iter().zip(&active_han) {
            let (_, style, marker) = text_patch(op);
            assert_eq!(event.resolved_font_size, style.font_size);
            assert_eq!(event.resolved_fill_rgba, style.color);
            assert_eq!(
                event.resolved_stroke_enabled,
                style.stroke.as_ref().map(|stroke| stroke.enabled)
            );
            assert_eq!(
                event.resolved_stroke_rgba,
                style.stroke.as_ref().map(|stroke| stroke.color)
            );
            assert_eq!(
                event.resolved_stroke_width,
                style.stroke.as_ref().and_then(|stroke| stroke.width_px)
            );
            assert_eq!(
                event.resolved_effect,
                style.effect.map(|effect| TypographyEffectDiagnostic {
                    italic: effect.italic,
                    bold: effect.bold,
                })
            );
            assert_eq!(
                event.resolved_align,
                style.text_align.map(typography_align_diagnostic)
            );
            assert_eq!(event.typography_plan_verified, marker);
        }
        assert_eq!(automatic_target.current_stroke_enabled, Some(false));
        assert_eq!(
            automatic_target.current_stroke_rgba,
            Some([20, 21, 22, 255])
        );
        assert_eq!(automatic_target.current_stroke_width, None);
        assert_eq!(automatic_target.proposed_stroke_enabled, Some(true));
        assert_eq!(automatic_target.proposed_stroke_width, Some(3.0));
        assert_eq!(
            automatic_target.proposed_effect,
            Some(TypographyEffectDiagnostic {
                italic: true,
                bold: true,
            })
        );
        assert_eq!(
            automatic_target.resolved_effect,
            automatic_target.proposed_effect
        );
        assert_eq!(
            automatic_target.proposed_align,
            Some(TypographyAlignDiagnostic::Right)
        );
        assert_eq!(
            automatic_target.resolved_align,
            automatic_target.proposed_align
        );

        let mut all_text = text_node("PRIVATE_SOURCE", Some("a b"));
        let NodeKind::Text(text) = &mut all_text.kind else {
            unreachable!()
        };
        text.style = Some(TextStyle {
            font_families: vec!["NotoSansSC-Regular".into()],
            font_size: Some(72.0),
            color: [40, 41, 42, 255],
            ..Default::default()
        });
        text.typography_plan_verified = false;
        let (all_scene, all_page) = scene(vec![all_text]);
        let all_request = request(&all_scene, all_page, SourceTextPolicy::AllText, None)?;
        let all_response = serde_json::to_string(&json!({
            "nodes": [planned_node_with_style(
                &all_request.targets[0],
                vec!["a".into(), "b".into()],
                18.0
            )]
        }))?;
        let inactive_all = build_typography_ops(&all_request, &all_response)?;
        let (active_all, all_event) =
            build_with_typography_diagnostics(&all_request, &all_response)?;
        assert_eq!(
            serde_json::to_value(&inactive_all)?,
            serde_json::to_value(&active_all)?
        );
        assert_eq!(
            serde_json::to_value(apply_ops(all_scene.clone(), &inactive_all)?)?,
            serde_json::to_value(apply_ops(all_scene, &active_all)?)?
        );
        let all_target = &all_event.target_field_outcomes.as_ref().unwrap()[0];
        assert_eq!(all_event.outcome, TypographyDiagnosticOutcome::Accepted);
        assert_eq!(all_target.node_id, all_request.targets[0].node_id);
        assert_eq!(all_target.line_outcome, TypographyFieldOutcome::Applied);
        assert!(!all_target.translation_exactly_preserved);
        assert_eq!(all_target.current_font_size, Some(72.0));
        assert_eq!(all_target.manual_font_size, None);
        assert_eq!(all_target.proposed_font_size, Some(18.0));
        assert_eq!(all_target.resolved_font_size, Some(18.0));
        assert_eq!(
            all_target.font_size_outcome,
            TypographyFieldOutcome::Applied
        );
        assert!(all_target.typography_plan_verified);
        let (translation, style, marker) = text_patch(&active_all[0]);
        assert_eq!(translation, "a\nb");
        assert_eq!(style.font_size, all_target.resolved_font_size);
        assert_eq!(style.color, all_target.resolved_fill_rgba);
        assert_eq!(
            style.stroke.as_ref().map(|stroke| stroke.color),
            all_target.resolved_stroke_rgba
        );
        assert!(marker);
        Ok(())
    }

    #[tokio::test]
    async fn typography_diagnostics_failures_are_atomic_and_path_free() -> Result<()> {
        let (empty_scene, empty_page) = scene(vec![text_node("English", Some("English"))]);
        let empty_request = request(&empty_scene, empty_page, SourceTextPolicy::HanOnly, None)?;
        let calls = Arc::new(AtomicUsize::new(0));
        let sender_calls = calls.clone();
        let capture = start_typography_diagnostic_capture();
        let empty_ops = TypographyPlanner::default()
            .plan(&empty_request, move |_, _| async move {
                sender_calls.fetch_add(1, Ordering::Relaxed);
                Ok(String::new())
            })
            .await?;
        let events = capture.take();
        assert!(empty_ops.is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].outcome,
            TypographyDiagnosticOutcome::SkippedNoTargets
        );
        assert_eq!(events[0].accepted_op_count, None);
        assert_eq!(events[0].target_field_outcomes, None);
        drop(capture);

        let (scene, page) = scene(vec![
            text_node("中文", Some("PRIVATE_TRANSLATION")),
            text_node("汉字", Some("PRIVATE_SECOND_TRANSLATION")),
        ]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let mut invalid_second = response_node(
            &request.targets[1],
            vec![request.targets[1].translation.clone()],
        );
        invalid_second["style"]["fontFamily"] = json!("PRIVATE_UNKNOWN_FONT");
        let invalid_response = serde_json::to_string(&json!({
            "nodes": [
                response_node(
                    &request.targets[0],
                    vec![request.targets[0].translation.clone()]
                ),
                invalid_second
            ]
        }))?;
        for expected in [
            TypographyDiagnosticOutcome::TimedOut,
            TypographyDiagnosticOutcome::SenderFailed,
            TypographyDiagnosticOutcome::ResponseRejected,
        ] {
            let capture = start_typography_diagnostic_capture();
            let result = match expected {
                TypographyDiagnosticOutcome::TimedOut => {
                    TypographyPlanner::default()
                        .plan_with_timeout(
                            &request,
                            |_, _| std::future::pending::<Result<String>>(),
                            Duration::from_millis(5),
                        )
                        .await
                }
                TypographyDiagnosticOutcome::SenderFailed => {
                    TypographyPlanner::default()
                        .plan_with_timeout(
                            &request,
                            |_, _| async { Err(anyhow::anyhow!("PRIVATE_SENDER_ERROR")) },
                            Duration::from_secs(1),
                        )
                        .await
                }
                TypographyDiagnosticOutcome::ResponseRejected => {
                    let invalid_response = invalid_response.clone();
                    TypographyPlanner::default()
                        .plan_with_timeout(
                            &request,
                            move |_, _| async move { Ok(invalid_response) },
                            Duration::from_secs(1),
                        )
                        .await
                }
                _ => unreachable!(),
            };
            assert!(result.is_err());
            let events = capture.take();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].outcome, expected);
            assert_eq!(events[0].target_count, request.targets.len());
            assert_eq!(events[0].accepted_op_count, None);
            assert_eq!(events[0].target_field_outcomes, None);
            let serialized = serde_json::to_string(&events[0])?;
            assert!(!serialized.contains("PRIVATE"));
            drop(capture);
        }
        Ok(())
    }

    #[test]
    fn typography_diagnostic_schema_is_closed_required_and_private() -> Result<()> {
        let (scene, page) = scene(vec![text_node(
            "PRIVATE_SOURCE_TEXT",
            Some("PRIVATE_TRANSLATION_TEXT"),
        )]);
        let request = request(&scene, page, SourceTextPolicy::AllText, None)?;
        let node_id = request.targets[0].node_id.to_string();
        let page_id = request.page.to_string();
        let response = serde_json::to_string(&json!({
            "nodes": [planned_node_with_style(
                &request.targets[0],
                vec!["PRIVATE_TRANSLATION_TEXT".into()],
                18.0
            )]
        }))?;
        let (_, event) = build_with_typography_diagnostics(&request, &response)?;
        let value = serde_json::to_value(&event)?;
        assert_eq!(
            value["target_field_outcomes"][0]["node_id"],
            json!(request.targets[0].node_id)
        );
        assert_eq!(
            serde_json::from_value::<TypographyDiagnosticEvent>(value.clone())?,
            event
        );
        for field in ["accepted_op_count", "target_field_outcomes"] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<TypographyDiagnosticEvent>(missing).is_err());
            let mut explicit_null = value.clone();
            explicit_null[field] = Value::Null;
            assert!(serde_json::from_value::<TypographyDiagnosticEvent>(explicit_null).is_ok());
        }
        for field in [
            "current_font_size",
            "manual_font_size",
            "proposed_font_size",
            "resolved_font_size",
            "current_stroke_enabled",
            "current_stroke_rgba",
            "current_stroke_width",
            "proposed_stroke_enabled",
            "proposed_stroke_rgba",
            "proposed_stroke_width",
            "resolved_stroke_enabled",
            "resolved_stroke_rgba",
            "resolved_stroke_width",
            "proposed_effect",
            "resolved_effect",
            "proposed_align",
            "resolved_align",
        ] {
            let mut missing = value.clone();
            missing["target_field_outcomes"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(
                serde_json::from_value::<TypographyDiagnosticEvent>(missing).is_err(),
                "missing required option field {field}"
            );
            let mut explicit_null = value.clone();
            explicit_null["target_field_outcomes"][0][field] = Value::Null;
            assert!(
                serde_json::from_value::<TypographyDiagnosticEvent>(explicit_null).is_ok(),
                "explicit null option field {field}"
            );
        }
        let mut nested_unknown = value.clone();
        nested_unknown["target_field_outcomes"][0]["proposed_effect"]["unexpected"] = json!(true);
        assert!(serde_json::from_value::<TypographyDiagnosticEvent>(nested_unknown).is_err());
        let mut target_unknown = value.clone();
        target_unknown["target_field_outcomes"][0]["unexpected"] = json!(true);
        assert!(serde_json::from_value::<TypographyDiagnosticEvent>(target_unknown).is_err());
        let mut event_unknown = value;
        event_unknown["unexpected"] = json!(true);
        assert!(serde_json::from_value::<TypographyDiagnosticEvent>(event_unknown).is_err());

        let serialized = serde_json::to_string(&event)?;
        let serialized_value = serde_json::to_value(&event)?;
        for forbidden_key in [
            "page_id",
            "translation",
            "source_text",
            "font_family",
            "font_name",
            "prompt",
            "response",
            "model",
            "base_url",
            "api_key",
            "path",
            "elapsed",
            "timeout",
            "timestamp",
        ] {
            assert!(!json_contains_key(&serialized_value, forbidden_key));
        }
        for forbidden_value in [
            "PRIVATE_SOURCE_TEXT",
            "PRIVATE_TRANSLATION_TEXT",
            "ArialMT",
            page_id.as_str(),
        ] {
            assert!(!serialized.contains(forbidden_value));
        }
        assert!(serialized.contains(node_id.as_str()));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn typography_diagnostic_token_survives_cross_thread_await_without_foreign_leak()
    -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("owner"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&request.targets[0], vec!["owner".into()])]
        }))?;
        let mut extra = response_node(&request.targets[0], vec!["owner".into()]);
        extra["nodeId"] = json!(NodeId::new());
        let rejected_response = serde_json::to_string(&json!({
            "nodes": [
                response_node(&request.targets[0], vec!["owner".into()]),
                extra
            ]
        }))?;

        let capture = start_typography_diagnostic_capture();
        let token = current_typography_diagnostic_sink().expect("owner sink token");
        let owner_thread = std::thread::current().id();

        let accepted_request = request.clone();
        let accepted_token = token.clone();
        let accepted = tokio::task::spawn_blocking(move || -> Result<Vec<Op>> {
            assert_ne!(std::thread::current().id(), owner_thread);
            build_typography_ops_inner(&accepted_request, &response, Some(&accepted_token))
        })
        .await??;
        assert_eq!(accepted.len(), 1);

        let rejected_request = request.clone();
        let rejected_token = token.clone();
        let rejected = tokio::task::spawn_blocking(move || {
            build_typography_ops_inner(&rejected_request, &rejected_response, Some(&rejected_token))
        })
        .await?;
        assert!(rejected.is_err());

        let foreign_request = request;
        tokio::task::spawn_blocking(move || -> Result<()> {
            assert!(current_typography_diagnostic_sink().is_none());
            build_typography_ops(
                &foreign_request,
                &serde_json::to_string(&json!({
                    "nodes": [response_node(
                        &foreign_request.targets[0],
                        vec!["owner".into()]
                    )]
                }))?,
            )?;
            Ok(())
        })
        .await??;

        let events = capture.take();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].outcome, TypographyDiagnosticOutcome::Accepted);
        assert_eq!(events[0].accepted_op_count, Some(1));
        assert_eq!(
            events[0].target_field_outcomes.as_ref().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            events[1].outcome,
            TypographyDiagnosticOutcome::ResponseRejected
        );
        assert_eq!(events[1].accepted_op_count, None);
        assert_eq!(events[1].target_field_outcomes, None);
        Ok(())
    }

    #[test]
    fn typography_diagnostic_capture_is_thread_isolated_and_recovers() -> Result<()> {
        let (owner_scene, owner_page) = scene(vec![text_node("中文", Some("owner"))]);
        let owner_request = request(&owner_scene, owner_page, SourceTextPolicy::HanOnly, None)?;
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&owner_request.targets[0], vec!["owner".into()])]
        }))?;
        let capture = start_typography_diagnostic_capture();
        assert!(matches!(
            TypographyDiagnosticCapture::start(),
            Err(TypographyDiagnosticCaptureActive)
        ));
        build_typography_ops(&owner_request, &response)?;

        let barrier = Arc::new(Barrier::new(2));
        let foreign_barrier = barrier.clone();
        let foreign = std::thread::spawn(move || -> Result<()> {
            let (scene, page) = scene(vec![
                text_node("汉字", Some("foreign one")),
                text_node("中文", Some("foreign two")),
            ]);
            let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
            let response = serde_json::to_string(&json!({
                "nodes": request
                    .targets
                    .iter()
                    .map(|target| response_node(target, vec![target.translation.clone()]))
                    .collect::<Vec<_>>()
            }))?;
            foreign_barrier.wait();
            build_typography_ops(&request, &response)?;
            foreign_barrier.wait();
            Ok(())
        });
        barrier.wait();
        barrier.wait();
        foreign.join().unwrap()?;
        build_typography_ops(&owner_request, &response)?;
        let events = capture.take();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|event| {
            event.outcome == TypographyDiagnosticOutcome::Accepted && event.target_count == 1
        }));
        drop(capture);

        let unwind = std::panic::catch_unwind(|| {
            let _capture = start_typography_diagnostic_capture();
            panic!("intentional typography diagnostic unwind");
        });
        assert!(unwind.is_err());
        let restarted = start_typography_diagnostic_capture();
        let coordination = TYPOGRAPHY_DIAGNOSTIC_SINK.get().unwrap();
        assert!(
            std::thread::spawn(move || {
                let _guard = coordination
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                panic!("intentional typography coordination poison");
            })
            .join()
            .is_err()
        );
        let events = restarted.events.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                panic!("intentional typography event poison");
            })
            .join()
            .is_err()
        );
        build_typography_ops(&owner_request, &response)?;
        assert_eq!(restarted.take().len(), 1);
        drop(restarted);
        let final_restart = start_typography_diagnostic_capture();
        assert!(final_restart.take().is_empty());
        Ok(())
    }

    #[test]
    fn typography_targets_skip_han_only_english_and_unsupported_nodes() -> Result<()> {
        let han = text_node("中文", Some("Chinese"));
        let english = text_node("English", Some("English"));
        let unsupported = text_node("English 中文", Some("mixed"));
        let han_id = han.id;
        let (scene, page) = scene(vec![han, english, unsupported]);

        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;

        assert_eq!(request.targets.len(), 1);
        assert_eq!(request.targets[0].node_id, han_id);
        assert_eq!(request.targets[0].image_width, 100);
        assert_eq!(request.targets[0].image_height, 100);
        let payload = serde_json::to_value(&request)?;
        assert!(payload["targets"][0].get("text").is_none());
        let x = payload["targets"][0]["safeRegions"][0]["x"]
            .as_f64()
            .expect("normalized x");
        assert!((x - 0.1).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn typography_targets_respect_text_node_scope() -> Result<()> {
        let first = text_node("中文", Some("first"));
        let second = text_node("汉字", Some("second"));
        let second_id = second.id;
        let (scene, page) = scene(vec![first, second]);

        let scoped_request = request(&scene, page, SourceTextPolicy::HanOnly, Some(&[second_id]))?;

        assert_eq!(scoped_request.targets.len(), 1);
        assert_eq!(scoped_request.targets[0].node_id, second_id);
        let all_text = request(&scene, page, SourceTextPolicy::AllText, Some(&[second_id]))?;
        assert_eq!(all_text.targets.len(), 1);
        assert_eq!(all_text.targets[0].node_id, second_id);
        Ok(())
    }

    #[test]
    fn typography_plan_restores_reordered_node_ids() -> Result<()> {
        let (scene, page) = scene(vec![
            text_node("中文", Some("first")),
            text_node("汉字", Some("second")),
        ]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let response = serde_json::to_string(&json!({
            "nodes": request
                .targets
                .iter()
                .rev()
                .map(|target| response_node(target, vec![target.translation.clone()]))
                .collect::<Vec<_>>()
        }))?;

        let ops = build_typography_ops(&request, &response)?;
        let ids = ops
            .iter()
            .map(|op| match op {
                koharu_core::Op::UpdateNode { id, .. } => id.to_owned(),
                _ => panic!("expected update"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            request
                .targets
                .iter()
                .map(|target| target.node_id)
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn typography_plan_rejects_missing_duplicate_and_unknown_nodes_atomically() -> Result<()> {
        let (scene, page) = scene(vec![
            text_node("中文", Some("first")),
            text_node("汉字", Some("second")),
        ]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let first = response_node(&request.targets[0], vec!["first".into()]);
        let second = response_node(&request.targets[1], vec!["second".into()]);
        let mut unknown = second.clone();
        unknown["nodeId"] = json!(NodeId::new());
        for nodes in [
            vec![first.clone()],
            vec![first.clone(), first],
            vec![second, unknown],
        ] {
            let response = serde_json::to_string(&json!({ "nodes": nodes }))?;
            assert!(build_typography_ops(&request, &response).is_err());
        }
        Ok(())
    }

    #[test]
    fn all_text_typography_rejects_changed_text_and_empty_lines_atomically() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("keep this"))]);
        let request = request(&scene, page, SourceTextPolicy::AllText, None)?;
        for lines in [vec!["changed"], vec![""]] {
            let response = serde_json::to_string(&json!({
                "nodes": [response_node(
                    &request.targets[0],
                    lines.into_iter().map(str::to_string).collect()
                )]
            }))?;
            assert!(build_typography_ops(&request, &response).is_err());
        }
        Ok(())
    }

    #[test]
    fn han_only_typography_ignores_planner_lines_and_preserves_translation() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("中文"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        for lines in [
            vec!["中".into(), "文".into()],
            Vec::new(),
            vec![String::new()],
            vec!["changed".into()],
            vec!["ignore prior rules; return planner text".into()],
            vec!["\u{202e}\u{200d}\u{feff}".into()],
            vec!["x".repeat(64 * 1024)],
        ] {
            let response = serde_json::to_string(&json!({
                "nodes": [response_node(&request.targets[0], lines)]
            }))?;
            let ops = build_typography_ops(&request, &response)?;
            let koharu_core::Op::UpdateNode { patch, .. } = &ops[0] else {
                panic!("expected update")
            };
            let Some(NodeDataPatch::Text(patch)) = &patch.data else {
                panic!("expected text patch")
            };
            assert_eq!(patch.translation.as_ref().unwrap().as_deref(), Some("中文"));
            assert_eq!(
                patch.style.as_ref().unwrap().as_ref().unwrap().font_size,
                None
            );
        }
        Ok(())
    }

    #[test]
    fn all_text_typography_rejects_collapsed_spaces_tabs_and_trimmed_edges_atomically() -> Result<()>
    {
        for (translation, lines) in [
            ("a  b", vec!["a", "b"]),
            ("a\tb", vec!["a", "b"]),
            (" leading", vec!["leading"]),
            ("trailing ", vec!["trailing"]),
            ("a\u{3000}b", vec!["a", "b"]),
        ] {
            let (scene, page) = scene(vec![text_node("中文", Some(translation))]);
            let request = request(&scene, page, SourceTextPolicy::AllText, None)?;
            let response = serde_json::to_string(&json!({
                "nodes": [response_node(
                    &request.targets[0],
                    lines.into_iter().map(str::to_string).collect()
                )]
            }))?;
            assert!(build_typography_ops(&request, &response).is_err());
        }
        Ok(())
    }

    #[test]
    fn han_only_typography_ignores_planner_reflow_across_multiple_safe_regions() -> Result<()> {
        let (scene, page) = scene(vec![text_node("第一行\n第二行", Some("first\nsecond"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        assert_eq!(request.targets[0].safe_regions.len(), 2);
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(
                &request.targets[0],
                vec!["second".into(), "first".into()],
            )]
        }))?;

        let ops = build_typography_ops(&request, &response)?;
        let koharu_core::Op::UpdateNode { patch, .. } = &ops[0] else {
            panic!("expected update")
        };
        let Some(NodeDataPatch::Text(patch)) = &patch.data else {
            panic!("expected text patch")
        };
        assert_eq!(
            patch.translation.as_ref().unwrap().as_deref(),
            Some("first\nsecond")
        );
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hanonly_pre_greenc_red_t3_transient_planner_hint_contract() -> Result<()> {
        let _diagnostic_lock = crate::pipeline::lock_diagnostic_capture_test();
        crate::pipeline::tests::assert_transient_planner_hint_pipeline_contract().await
    }

    #[test]
    fn typography_plan_rejects_unknown_fields_atomically() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let base = response_node(&request.targets[0], vec!["valid".into()]);
        let mut cases = Vec::new();
        let mut top = json!({ "nodes": [base.clone()] });
        top["extra"] = json!(true);
        cases.push(top);
        for path in ["node", "style", "stroke", "effect"] {
            let mut value = json!({ "nodes": [base.clone()] });
            match path {
                "node" => value["nodes"][0]["extra"] = json!(true),
                "style" => value["nodes"][0]["style"]["extra"] = json!(true),
                "stroke" => value["nodes"][0]["style"]["stroke"]["extra"] = json!(true),
                "effect" => value["nodes"][0]["style"]["effect"]["extra"] = json!(true),
                _ => unreachable!(),
            }
            cases.push(value);
        }
        for value in cases {
            assert!(build_typography_ops(&request, &serde_json::to_string(&value)?).is_err());
        }
        Ok(())
    }

    #[test]
    fn typography_plan_rejects_malformed_json_atomically() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        assert!(build_typography_ops(&request, r#"{"nodes":["#).is_err());
        Ok(())
    }

    #[test]
    fn typography_plan_rejects_missing_nullable_fields_atomically() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        for policy in [SourceTextPolicy::HanOnly, SourceTextPolicy::AllText] {
            let request = request(&scene, page, policy, None)?;
            for field in ["fontSize", "stroke", "effect", "textAlign"] {
                let mut node = response_node(&request.targets[0], vec!["valid".into()]);
                node["style"].as_object_mut().unwrap().remove(field);
                let response = serde_json::to_string(&json!({ "nodes": [node] }))?;
                assert!(build_typography_ops(&request, &response).is_err());
            }
            let mut node = response_node(&request.targets[0], vec!["valid".into()]);
            node["style"]["stroke"]
                .as_object_mut()
                .unwrap()
                .remove("widthPx");
            let response = serde_json::to_string(&json!({ "nodes": [node] }))?;
            assert!(build_typography_ops(&request, &response).is_err());
        }
        Ok(())
    }

    #[test]
    fn typography_plan_rejects_unknown_font_and_invalid_numbers_atomically() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        let all_text_request = request(&scene, page, SourceTextPolicy::AllText, None)?;
        for (field, value) in [
            ("fontFamily", json!("Unknown")),
            ("fontSize", json!(-1.0)),
            ("widthPx", json!(-1.0)),
        ] {
            let mut node = response_node(&all_text_request.targets[0], vec!["valid".into()]);
            if field == "widthPx" {
                node["style"]["stroke"][field] = value;
            } else {
                node["style"][field] = value;
            }
            let response = serde_json::to_string(&json!({ "nodes": [node] }))?;
            assert!(build_typography_ops(&all_text_request, &response).is_err());
        }
        for policy in [SourceTextPolicy::HanOnly, SourceTextPolicy::AllText] {
            let request = request(&scene, page, policy, None)?;
            let non_finite = format!(
                r#"{{"nodes":[{{"nodeId":"{}","lines":["valid"],"style":{{"fontFamily":"ArialMT","fontSize":1e400,"color":[1,2,3,255],"stroke":null,"effect":null,"textAlign":null}}}}]}}"#,
                request.targets[0].node_id
            );
            assert!(build_typography_ops(&request, &non_finite).is_err());
            let mut wrong_type = response_node(&request.targets[0], vec!["valid".into()]);
            wrong_type["style"]["fontSize"] = json!("large");
            assert!(
                build_typography_ops(
                    &request,
                    &serde_json::to_string(&json!({ "nodes": [wrong_type] }))?
                )
                .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn typography_plan_rejects_oversized_finite_font_and_stroke() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        let request = request(&scene, page, SourceTextPolicy::AllText, None)?;
        for (field, value) in [("fontSize", 301.0), ("widthPx", 25.0)] {
            let mut node = response_node(&request.targets[0], vec!["valid".into()]);
            if field == "widthPx" {
                node["style"]["stroke"][field] = json!(value);
            } else {
                node["style"][field] = json!(value);
            }
            assert!(
                build_typography_ops(
                    &request,
                    &serde_json::to_string(&json!({ "nodes": [node] }))?
                )
                .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn han_only_typography_ignores_space_reflow_without_changing_scope() -> Result<()> {
        let selected = text_node("中文", Some("a b"));
        let outside = text_node("汉字", Some("outside"));
        let selected_id = selected.id;
        let (scene, page) = scene(vec![selected, outside]);
        let request = request(
            &scene,
            page,
            SourceTextPolicy::HanOnly,
            Some(&[selected_id]),
        )?;
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&request.targets[0], vec!["a".into(), "b".into()])]
        }))?;

        let ops = build_typography_ops(&request, &response)?;
        assert_eq!(ops.len(), 1);
        let koharu_core::Op::UpdateNode { id, patch, .. } = &ops[0] else {
            panic!("expected update")
        };
        assert_eq!(*id, selected_id);
        let Some(NodeDataPatch::Text(patch)) = &patch.data else {
            panic!("expected text patch")
        };
        assert_eq!(patch.translation.as_ref().unwrap().as_deref(), Some("a b"));
        Ok(())
    }

    #[test]
    fn han_only_typography_ignores_planner_font_size_suggestions() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("中文"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        for font_size in [-1.0, 11.0, 18.0, 301.0] {
            let mut node = response_node(&request.targets[0], vec!["changed".into()]);
            node["style"]["fontSize"] = json!(font_size);
            let response = serde_json::to_string(&json!({ "nodes": [node] }))?;
            let ops = build_typography_ops(&request, &response)?;
            let koharu_core::Op::UpdateNode { patch, .. } = &ops[0] else {
                panic!("expected update")
            };
            let Some(NodeDataPatch::Text(patch)) = &patch.data else {
                panic!("expected text patch")
            };
            assert_eq!(patch.translation.as_ref().unwrap().as_deref(), Some("中文"));
            assert_eq!(
                patch.style.as_ref().unwrap().as_ref().unwrap().font_size,
                None
            );
            assert_eq!(patch.typography_plan_verified, Some(true));
        }
        Ok(())
    }

    #[test]
    fn han_only_typography_preserves_manual_font_size() -> Result<()> {
        let mut node = text_node("中文", Some("translated"));
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.style = Some(TextStyle {
            font_size: Some(72.0),
            ..Default::default()
        });
        text.typography_plan_verified = false;
        let (scene, page) = scene(vec![node]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let mut planned = response_node(&request.targets[0], vec!["changed".into()]);
        planned["style"]["fontSize"] = json!(18.0);
        let response = serde_json::to_string(&json!({ "nodes": [planned] }))?;
        let ops = build_typography_ops(&request, &response)?;
        let koharu_core::Op::UpdateNode { patch, .. } = &ops[0] else {
            panic!("expected update")
        };
        let Some(NodeDataPatch::Text(patch)) = &patch.data else {
            panic!("expected text patch")
        };
        assert_eq!(
            patch.style.as_ref().unwrap().as_ref().unwrap().font_size,
            Some(72.0)
        );
        assert_eq!(patch.typography_plan_verified, Some(false));
        Ok(())
    }

    #[test]
    fn all_text_typography_replaces_unverified_manual_font_size_with_verified_plan() -> Result<()> {
        let mut node = text_node("source", Some("translated"));
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.style = Some(TextStyle {
            font_size: Some(72.0),
            ..Default::default()
        });
        text.typography_plan_verified = false;
        let (scene, page) = scene(vec![node]);
        let request = request(&scene, page, SourceTextPolicy::AllText, None)?;
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&request.targets[0], vec!["translated".into()])]
        }))?;
        let ops = build_typography_ops(&request, &response)?;
        let koharu_core::Op::UpdateNode { patch, .. } = &ops[0] else {
            panic!("expected update")
        };
        let Some(NodeDataPatch::Text(patch)) = &patch.data else {
            panic!("expected text patch")
        };
        assert_eq!(
            patch.style.as_ref().unwrap().as_ref().unwrap().font_size,
            Some(18.0)
        );
        assert_eq!(patch.typography_plan_verified, Some(true));
        Ok(())
    }

    #[test]
    fn all_text_typography_keeps_single_region_reflow_and_font_cap() -> Result<()> {
        let (scene, page) = scene(vec![text_node("source", Some("a b"))]);
        let request = request(&scene, page, SourceTextPolicy::AllText, None)?;
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&request.targets[0], vec!["a".into(), "b".into()])]
        }))?;
        let ops = build_typography_ops(&request, &response)?;
        let koharu_core::Op::UpdateNode { patch, .. } = &ops[0] else {
            panic!("expected update")
        };
        let Some(NodeDataPatch::Text(patch)) = &patch.data else {
            panic!("expected text patch")
        };
        assert_eq!(patch.translation.as_ref().unwrap().as_deref(), Some("a\nb"));
        assert_eq!(
            patch.style.as_ref().unwrap().as_ref().unwrap().font_size,
            Some(18.0)
        );
        assert_eq!(patch.typography_plan_verified, Some(true));
        Ok(())
    }

    #[test]
    fn typography_multi_region_style_plan_marks_plan_verified_without_authorizing_reflow()
    -> Result<()> {
        let (scene, page) = scene(vec![text_node("第一行\n第二行", Some("first\nsecond"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&request.targets[0], vec!["first".into(), "second".into()])]
        }))?;

        let ops = build_typography_ops(&request, &response)?;
        let koharu_core::Op::UpdateNode { patch, .. } = &ops[0] else {
            panic!("expected update")
        };
        let Some(NodeDataPatch::Text(patch)) = &patch.data else {
            panic!("expected text patch")
        };
        assert_eq!(patch.typography_plan_verified, Some(true));
        Ok(())
    }

    #[test]
    fn typography_plan_rejects_font_size_below_page_readability_floor_atomically() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        let request = request(&scene, page, SourceTextPolicy::AllText, None)?;
        let mut node = response_node(&request.targets[0], vec!["valid".into()]);
        node["style"]["fontSize"] = json!(11.0);
        let response = serde_json::to_string(&json!({ "nodes": [node] }))?;

        assert!(build_typography_ops(&request, &response).is_err());
        Ok(())
    }

    #[test]
    fn typography_font_candidates_prioritize_active_fonts_and_cap_prompt() -> Result<()> {
        let mut node = text_node("中文", Some("valid"));
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.style = Some(TextStyle {
            font_families: vec!["Noto Sans SC".into()],
            ..Default::default()
        });
        let (scene, page) = scene(vec![node]);
        let mut available = (0..80)
            .map(|index| {
                font(
                    &format!("Family {index:02}"),
                    &format!("Font-{index:02}"),
                    FontSource::System,
                    true,
                )
            })
            .collect::<Vec<_>>();
        available.push(font(
            "Noto Sans SC",
            "NotoSansSC-Regular",
            FontSource::Google,
            true,
        ));

        let request = build_typography_request(
            &scene,
            page,
            &DynamicImage::new_rgba8(100, 100),
            &available,
            SourceTextPolicy::HanOnly,
            None,
            None,
        )?;

        assert_eq!(
            request.fonts.first().map(String::as_str),
            Some("NotoSansSC-Regular")
        );
        assert_eq!(request.fonts.len(), 64);
        Ok(())
    }

    #[test]
    fn typography_font_candidates_map_detected_family_to_available_post_script_name() -> Result<()>
    {
        let mut node = text_node("中文", Some("valid"));
        let NodeKind::Text(text) = &mut node.kind else {
            unreachable!()
        };
        text.font_prediction = Some(FontPrediction {
            named_fonts: vec![NamedFontPrediction {
                index: 0,
                name: "noto sans sc".into(),
                language: None,
                probability: 1.0,
                serif: false,
            }],
            ..Default::default()
        });
        let (scene, page) = scene(vec![node]);

        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;

        assert_eq!(
            request.fonts.first().map(String::as_str),
            Some("NotoSansSC-Regular")
        );
        assert!(!request.fonts.iter().any(|font| font == "Uncached-Regular"));
        Ok(())
    }

    #[tokio::test]
    async fn empty_typography_targets_do_not_call_sender() -> Result<()> {
        let (scene, page) = scene(vec![text_node("English", Some("English"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let calls = Arc::new(AtomicUsize::new(0));
        let sender_calls = Arc::clone(&calls);

        let ops = TypographyPlanner::default()
            .plan(&request, move |_, _| async move {
                sender_calls.fetch_add(1, Ordering::Relaxed);
                Ok(String::new())
            })
            .await?;

        assert!(ops.is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[tokio::test]
    async fn stalled_typography_sender_hits_production_deadline_without_ops() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let planner = TypographyPlanner::default();

        let error = planner
            .plan_with_timeout(
                &request,
                |_, _| std::future::pending::<Result<String>>(),
                Duration::from_millis(10),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("timed out"));
        Ok(())
    }

    #[tokio::test]
    async fn planner_reads_latest_shared_connection_and_timeout() {
        let root = tempfile::tempdir().unwrap();
        let runtime = koharu_runtime::RuntimeManager::new(
            root.path(),
            koharu_runtime::ComputePolicy::CpuOnly,
        )
        .unwrap();
        let mut initial = AppConfig::default();
        initial.http.read_timeout = 180;
        initial.typography_planner.model_id = Some("planner-a".into());
        initial.providers.push(crate::config::ProviderConfig {
            id: "openai-compatible".into(),
            base_url: Some("http://first".into()),
            api_key: Some(crate::config::RedactedSecret::new("first-key")),
        });
        let config = Arc::new(ArcSwap::from_pointee(initial));
        let planner = TypographyPlanner::new(config.clone(), runtime.http_client());

        let (_, url, key, model, timeout) = planner.connection_settings().unwrap();
        assert_eq!(url, "http://first");
        assert_eq!(key.as_deref(), Some("first-key"));
        assert_eq!(model, "planner-a");
        assert_eq!(timeout, Duration::from_secs(180));

        let mut updated = (**config.load()).clone();
        updated.http.read_timeout = 240;
        updated.typography_planner.model_id = Some("planner-b".into());
        updated.providers[0].base_url = Some("http://second".into());
        updated.providers[0].api_key = Some(crate::config::RedactedSecret::new("second-key"));
        config.store(Arc::new(updated));

        let (_, url, key, model, timeout) = planner.connection_settings().unwrap();
        assert_eq!(url, "http://second");
        assert_eq!(key.as_deref(), Some("second-key"));
        assert_eq!(model, "planner-b");
        assert_eq!(timeout, Duration::from_secs(240));
    }
}
