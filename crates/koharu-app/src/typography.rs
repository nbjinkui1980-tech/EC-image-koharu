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

const PRODUCTION_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_IMAGE_DIMENSION: u32 = 1536;
const MAX_FONT_SIZE_PX: f32 = 300.0;
const MAX_STROKE_WIDTH_PX: f32 = 24.0;
const MAX_FONT_CANDIDATES: usize = 64;

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
    #[serde(skip)]
    active_font_hints: Vec<String>,
    #[serde(skip)]
    detected_font_hints: Vec<String>,
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
    font_size: Option<f32>,
    color: [u8; 4],
    stroke: Option<PlannedStroke>,
    effect: Option<PlannedEffect>,
    text_align: Option<TextAlign>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedStroke {
    enabled: bool,
    color: [u8; 4],
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
            return Ok(Vec::new());
        }
        let payload = serde_json::to_string(request)?;
        let prompt = format!(
            "Return only this strict JSON shape with exactly one result per input node and no extra fields: {{\"nodes\":[{{\"nodeId\":\"uuid\",\"lines\":[\"text\"],\"style\":{{\"fontFamily\":\"allowed PostScript name\",\"fontSize\":null,\"color\":[0,0,0,255],\"stroke\":null,\"effect\":null,\"textAlign\":null}}}}]}}. Preserve every character and whitespace; a boundary between lines may only insert a line break or replace one existing ASCII space/newline. Input: {payload}"
        );
        let response =
            tokio::time::timeout(timeout, sender(prompt, request.image_data_url.clone()))
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "typography planner request timed out after {} seconds",
                        timeout.as_secs()
                    )
                })??;
        build_typography_ops(request, &response)
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
    for target in &request.targets {
        let node = planned
            .remove(&target.node_id)
            .ok_or_else(|| anyhow::anyhow!("missing Typography response node"))?;
        validate_lines(target, &node.lines)?;
        let font_family = font_lookup
            .get(&node.style.font_family.trim().to_lowercase())
            .ok_or_else(|| anyhow::anyhow!("unknown Typography font"))?
            .to_string();
        if let Some(font_size) = node.style.font_size {
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
            font_size: node.style.font_size,
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
        validated.push((target.node_id, node.lines.join("\n"), style));
    }
    anyhow::ensure!(planned.is_empty(), "unknown Typography response node");

    Ok(validated
        .into_iter()
        .map(|(node_id, translation, style)| Op::UpdateNode {
            page: request.page,
            id: node_id,
            patch: NodePatch {
                data: Some(NodeDataPatch::Text(TextDataPatch {
                    translation: Some(Some(translation)),
                    style: Some(Some(style)),
                    sprite: Some(None),
                    sprite_transform: Some(None),
                    typography_plan_verified: Some(true),
                    ..Default::default()
                })),
                ..Default::default()
            },
            prev: NodePatch::default(),
        })
        .collect())
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
        if matches_from(original, lines, line_index + 1, next, seen) {
            return true;
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
                "fontSize": 18.0,
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
    fn typography_plan_rejects_changed_text_and_empty_lines_atomically() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("keep this"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
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
    fn typography_plan_accepts_zero_consumption_cjk_line_break() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("中文"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&request.targets[0], vec!["中".into(), "文".into()])]
        }))?;

        let ops = build_typography_ops(&request, &response)?;

        assert_eq!(ops.len(), 1);
        Ok(())
    }

    #[test]
    fn typography_plan_rejects_collapsed_spaces_tabs_and_trimmed_edges_atomically() -> Result<()> {
        for (translation, lines) in [
            ("a  b", vec!["a", "b"]),
            ("a\tb", vec!["a", "b"]),
            (" leading", vec!["leading"]),
            ("trailing ", vec!["trailing"]),
            ("a\u{3000}b", vec!["a", "b"]),
        ] {
            let (scene, page) = scene(vec![text_node("中文", Some(translation))]);
            let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
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
    fn typography_plan_rejects_reflow_across_multiple_safe_regions() -> Result<()> {
        let (scene, page) = scene(vec![text_node("第一行\n第二行", Some("first second"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        assert_eq!(request.targets[0].safe_regions.len(), 2);
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&request.targets[0], vec!["first".into(), "second".into()])]
        }))?;

        assert!(build_typography_ops(&request, &response).is_err());
        Ok(())
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
    fn typography_plan_rejects_unknown_font_and_invalid_numbers_atomically() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        for (field, value) in [
            ("fontFamily", json!("Unknown")),
            ("fontSize", json!(-1.0)),
            ("widthPx", json!(-1.0)),
        ] {
            let mut node = response_node(&request.targets[0], vec!["valid".into()]);
            if field == "widthPx" {
                node["style"]["stroke"][field] = value;
            } else {
                node["style"][field] = value;
            }
            let response = serde_json::to_string(&json!({ "nodes": [node] }))?;
            assert!(build_typography_ops(&request, &response).is_err());
        }
        let non_finite = format!(
            r#"{{"nodes":[{{"nodeId":"{}","lines":["valid"],"style":{{"fontFamily":"ArialMT","fontSize":1e400,"color":[1,2,3,255],"stroke":null,"effect":null,"textAlign":null}}}}]}}"#,
            request.targets[0].node_id
        );
        assert!(build_typography_ops(&request, &non_finite).is_err());
        Ok(())
    }

    #[test]
    fn typography_plan_rejects_oversized_finite_font_and_stroke() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("valid"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
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
    fn typography_plan_builds_scoped_style_linebreak_and_cleanup_ops() -> Result<()> {
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
        assert_eq!(id.to_owned(), selected_id);
        let Some(NodeDataPatch::Text(patch)) = &patch.data else {
            panic!("expected text patch")
        };
        assert_eq!(patch.translation.as_ref().unwrap().as_deref(), Some("a\nb"));
        assert_eq!(patch.sprite, Some(None));
        assert!(matches!(patch.sprite_transform, Some(None)));
        assert_eq!(patch.typography_plan_verified, Some(true));
        Ok(())
    }

    #[test]
    fn typography_plan_marks_valid_single_region_reflow_verified() -> Result<()> {
        let (scene, page) = scene(vec![text_node("中文", Some("中文"))]);
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
        let response = serde_json::to_string(&json!({
            "nodes": [response_node(&request.targets[0], vec!["中".into(), "文".into()])]
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
        let request = request(&scene, page, SourceTextPolicy::HanOnly, None)?;
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
