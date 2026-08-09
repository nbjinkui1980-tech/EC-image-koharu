//! One-shot pipeline CLI.
//!
//! Runs the full engine chain (or a custom subset) on a single image and
//! dumps every intermediate artifact to an output directory. Reuses the
//! production `pipeline::run` driver — same code path the HTTP server
//! takes — so renderer / engine regressions surface identically here.
//!
//! ## Quick-start
//!
//! ```text
//! cargo run --features bin -p koharu-app --bin pipeline -- \
//!     --input sample.png \
//!     --output-dir out/
//! ```
//!
//! By default the LLM translate step is skipped (it would need a local
//! model loaded). When translate is skipped we copy OCR text into the
//! translation slot so the renderer still has something to rasterise.
//!
//! To run the translate step end-to-end, preload a local model:
//! `--with-translate --llm <modelId> --target-lang en`.
//!
//! ## Output files
//!
//! For every role-keyed image/mask that lands on the page:
//!
//! - `source.png`, `inpainted.png`, `rendered.png`
//! - `segment.png`, `bubble.png` (only if the engine produced them)
//! - `brush.png` (if the user painted anything — unusual from CLI)
//! - `scene.json` — the final Scene snapshot (useful for diffing).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow};
use camino::Utf8PathBuf;
use clap::Parser;
use image::{DynamicImage, GenericImageView};
use koharu_app::{App, AppConfig};
use koharu_core::{
    ImageData, ImageRole, MaskRole, Node, NodeId, NodeKind, Op, Page, PageId, Transform,
};
use koharu_runtime::{ComputePolicy, RuntimeHttpConfig, RuntimeManager};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser, Debug)]
#[command(version, about = "Run the Koharu pipeline against a single image")]
struct Cli {
    /// Source image (png / jpg / webp).
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Directory to write intermediate + final artifacts into. Created if missing.
    #[arg(short, long, value_name = "DIR")]
    output_dir: PathBuf,

    /// Optional TOML override for the runtime config. Defaults to the
    /// built-in `AppConfig::default()`.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Override the pipeline step list (comma-separated engine ids).
    /// When omitted we run the engines named in `config.pipeline.*`,
    /// skipping translate unless `--with-translate` is passed.
    #[arg(long, value_name = "IDS", value_delimiter = ',')]
    steps: Option<Vec<String>>,

    /// Target language for the translator engine (ignored when translate is skipped).
    #[arg(long, default_value = "en")]
    target_lang: String,

    /// Custom system prompt for the translator.
    #[arg(long)]
    system_prompt: Option<String>,

    /// Default font family to apply when a block has no detected font.
    #[arg(long)]
    default_font: Option<String>,

    /// Include the llm-translate step. Requires `--llm <id>` to pre-load a
    /// local model, or for the currently-registered translator to accept
    /// provider-backed requests.
    #[arg(long)]
    with_translate: bool,

    /// Pre-load a local LLM before the pipeline runs (e.g. `lfm2.5-1.2b-instruct`).
    #[arg(long, value_name = "MODEL_ID")]
    llm: Option<String>,

    /// Force CPU-only compute.
    #[arg(long)]
    cpu: bool,

    /// Override the data root directory (default: CARGO_MANIFEST_DIR/.cache).
    /// Use a pre-populated directory to skip model downloads.
    #[arg(long, value_name = "DIR")]
    data_root: Option<PathBuf>,
}

fn main() -> Result<()> {
    init_tracing();

    // A generous stack keeps ONNX + large image decoders happy on Windows.
    std::thread::Builder::new()
        .name("koharu-pipeline".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(run())
        })?
        .join()
        .map_err(|_| anyhow!("pipeline worker thread panicked"))?
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::Level::INFO.into())
        .from_env_lossy();
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
}

fn pipeline_options(cli: &Cli, config: &AppConfig) -> koharu_app::PipelineRunOptions {
    koharu_app::PipelineRunOptions {
        source_text_policy: config.pipeline.source_text_policy,
        target_language: Some(cli.target_lang.clone()),
        system_prompt: cli.system_prompt.clone(),
        default_font: cli.default_font.clone(),
        text_node_ids: None,
        reading_order: None,
        region: None,
    }
}

fn emit_model_inventory(data_root: &Utf8PathBuf) -> Result<()> {
    let models_dir = data_root.join("models");
    if !models_dir.exists() {
        eprintln!("model_inventory: models dir not found at {}", models_dir);
        return Ok(());
    }
    let mut entries: Vec<(String, u64, String)> = Vec::new();
    for entry in walkdir::WalkDir::new(&models_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let relative = path
            .strip_prefix(models_dir.as_std_path())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let len = bytes.len() as u64;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        entries.push((relative, len, hash));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, size, hash) in &entries {
        eprintln!("model_inventory path={path} size={size} sha256={hash}");
    }
    Ok(())
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    std::fs::create_dir_all(&cli.output_dir)
        .with_context(|| format!("create output dir {}", cli.output_dir.display()))?;

    let cfg = load_config(cli.config.as_deref())?;

    // Stage the project + runtime under a fresh tempdir so repeat runs
    // never collide. TempDir cleans up automatically when the CLI exits.
    let temp_root: Utf8PathBuf = if let Some(ref root) = cli.data_root {
        Utf8PathBuf::try_from(root.clone()).map_err(|_| anyhow!("data-root path is not UTF-8"))?
    } else {
        env!("CARGO_MANIFEST_DIR")
            .parse::<Utf8PathBuf>()
            .expect("manifest dir not UTF-8")
            .join(".cache")
    };

    let mut cfg = cfg;
    let options = pipeline_options(&cli, &cfg);
    cfg.data.path = temp_root.clone();
    std::fs::create_dir_all(cfg.data.path.as_std_path()).context("create data dir")?;

    let http = RuntimeHttpConfig {
        connect_timeout_secs: cfg.http.connect_timeout.max(1),
        read_timeout_secs: cfg.http.read_timeout.max(1),
        max_retries: cfg.http.max_retries,
    };
    let compute = if cli.cpu {
        ComputePolicy::CpuOnly
    } else {
        ComputePolicy::PreferGpu
    };
    let runtime = RuntimeManager::new_with_http(cfg.data.path.as_std_path(), compute, http)?;
    runtime
        .ensure_prepared_cached()
        .await
        .context("cached-only preflight: models must be pre-seeded (use --data-root <path>)")?;

    emit_model_inventory(&temp_root)?;

    runtime
        .prepare()
        .await
        .context("prepare runtime (downloads llama.cpp if missing)")?;

    let actual_compute = if cli.cpu { "cpu" } else { "metal" };
    eprintln!("model_instance_device engine=cli model=none instance=0 actual={actual_compute}");

    let app = Arc::new(App::new(cfg, Arc::new(runtime), cli.cpu, "cli")?);
    app.spawn_download_forwarder();
    app.spawn_llm_forwarder();

    // Optional LLM preload so the translate step can reach the model.
    if let Some(model_id) = cli.llm.as_deref() {
        eprintln!("=> loading LLM `{model_id}`");
        app.llm
            .load_from_request(
                koharu_core::LlmLoadRequest {
                    target: koharu_core::LlmTarget {
                        kind: koharu_core::LlmTargetKind::Local,
                        model_id: model_id.to_string(),
                        provider_id: None,
                    },
                    options: None,
                },
                None,
            )
            .await
            .with_context(|| format!("load local llm `{model_id}`"))?;
        // `load_local` is fire-and-forget; poll until it reports Ready.
        wait_for_llm_ready(&app).await?;
    } else if cli.with_translate {
        anyhow::bail!("--with-translate requires --llm <modelId>");
    }

    // Project session + source image.
    let project_dir = tempfile::tempdir()
        .context("create temp project dir")?
        .path()
        .to_string_lossy()
        .parse::<Utf8PathBuf>()
        .context("temp project dir not UTF-8")?;
    let session = app
        .open_project(project_dir, Some("cli".to_string()))
        .await
        .context("open cli project")?;

    let page_id = import_page(&app, &cli.input).context("import source image")?;

    // Pick the step chain.
    let steps = resolve_steps(&cli, &app.config.load())?;
    if steps.is_empty() {
        anyhow::bail!("no steps to run; check --steps or config.pipeline.*");
    }
    let (first_steps, render_steps) = split_render_phase(steps)?;

    // Progress sink — one JSON line per tick to stdout. Useful when a step
    // hangs and you want to see which one.
    let progress_sink: koharu_app::pipeline::ProgressSink =
        Arc::new(|tick: koharu_app::pipeline::ProgressTick| {
            let line = serde_json::json!({
                "step_id": tick.step_id,
                "step_index": tick.step_index,
                "total_steps": tick.total_steps,
                "page_index": tick.page_index,
                "total_pages": tick.total_pages,
                "percent": tick.overall_percent,
            });
            println!("{line}");
        });

    let warning_sink: koharu_app::pipeline::WarningSink =
        Arc::new(|tick: koharu_app::pipeline::WarningTick| {
            eprintln!(
                "warn: step '{}' failed on page {}/{}: {}",
                tick.step_id,
                tick.page_index + 1,
                tick.total_pages,
                tick.message
            );
        });

    let pipeline_result = run_pipeline_phases(
        first_steps,
        render_steps,
        |steps| {
            run_pipeline_phase(
                &app,
                session.clone(),
                page_id,
                steps,
                options.clone(),
                progress_sink.clone(),
                warning_sink.clone(),
            )
        },
        || synthesize_translations(&app, page_id),
    )
    .await;

    match &pipeline_result {
        Ok(()) => eprintln!("=> pipeline succeeded"),
        Err(error) => eprintln!("=> pipeline failed: {error:#}"),
    }

    dump_artifacts(&session, page_id, &cli.output_dir)
        .with_context(|| format!("dump artifacts to {}", cli.output_dir.display()))?;

    app.close_project().await.ok();
    pipeline_result
}

async fn run_pipeline_phases<RunPhase, RunFuture, Fallback, FallbackFuture>(
    first_steps: Vec<String>,
    render_steps: Vec<String>,
    mut run_phase: RunPhase,
    fallback: Fallback,
) -> Result<()>
where
    RunPhase: FnMut(Vec<String>) -> RunFuture,
    RunFuture: std::future::Future<Output = Result<koharu_app::pipeline::RunOutcome>>,
    Fallback: FnOnce() -> FallbackFuture,
    FallbackFuture: std::future::Future<Output = Result<()>>,
{
    if !first_steps.is_empty() {
        eprintln!("=> running first phase: {}", first_steps.join(" → "));
        require_clean_phase(&run_phase(first_steps).await?)?;
    }
    if !render_steps.is_empty() {
        fallback().await?;
        eprintln!("=> running render phase: {}", render_steps.join(" → "));
        require_clean_phase(&run_phase(render_steps).await?)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_pipeline_phase(
    app: &App,
    session: Arc<koharu_app::ProjectSession>,
    page_id: PageId,
    steps: Vec<String>,
    options: koharu_app::PipelineRunOptions,
    progress: koharu_app::pipeline::ProgressSink,
    warnings: koharu_app::pipeline::WarningSink,
) -> Result<koharu_app::pipeline::RunOutcome> {
    koharu_app::pipeline::run(
        session,
        app.registry.clone(),
        app.runtime.clone(),
        app.cpu_only(),
        app.llm.clone(),
        app.renderer.clone(),
        app.typography_planner.clone(),
        koharu_app::pipeline::PipelineSpec {
            scope: koharu_app::pipeline::Scope::Pages(vec![page_id]),
            steps,
            options,
        },
        Arc::new(AtomicBool::new(false)),
        Some(progress),
        Some(warnings),
    )
    .await
}

/// Load standard desktop config by default; custom TOML shares the same secret
/// hydration path without writing its keys back to disk.
fn load_config(path: Option<&std::path::Path>) -> Result<AppConfig> {
    load_config_with(
        path,
        koharu_app::config::load,
        koharu_app::config::hydrate_provider_secrets,
    )
}

fn load_config_with(
    path: Option<&std::path::Path>,
    standard_load: impl FnOnce() -> Result<AppConfig>,
    hydrate: impl FnOnce(&mut AppConfig) -> Result<()>,
) -> Result<AppConfig> {
    match path {
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .with_context(|| format!("read config {}", p.display()))?;
            let mut config = toml::from_str(&text)?;
            hydrate(&mut config)?;
            Ok(config)
        }
        None => standard_load(),
    }
}

/// Poll the LLM state every 200 ms until it's ready or fails. Local GGUF
/// loads are seconds to minutes depending on size — this avoids racing the
/// pipeline against a still-loading model.
async fn wait_for_llm_ready(app: &App) -> Result<()> {
    loop {
        let snap = app.llm.snapshot().await;
        match snap.status {
            koharu_core::LlmStateStatus::Ready => return Ok(()),
            koharu_core::LlmStateStatus::Failed => {
                anyhow::bail!("llm load failed");
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
        }
    }
}

/// Import the source image as a new page + `Image { Source }` node. Mirrors
/// the `POST /pages` handler, minus the multipart plumbing.
fn import_page(app: &App, input: &std::path::Path) -> Result<PageId> {
    let bytes =
        std::fs::read(input).with_context(|| format!("read input image {}", input.display()))?;
    let decoded = koharu_app::blobs::admit_source_image(&bytes)
        .with_context(|| format!("admit {}", input.display()))?;
    let (w, h) = decoded.dimensions();
    let filename = input
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("input")
        .to_string();

    let session = app
        .current_session()
        .ok_or_else(|| anyhow!("no session open"))?;
    let blob = session.blobs.put_bytes(&bytes)?;
    let mut page = Page::new(&filename, w, h);
    let page_id = page.id;
    let source_node_id = NodeId::new();
    page.nodes.insert(
        source_node_id,
        Node {
            id: source_node_id,
            transform: Transform::default(),
            visible: true,
            kind: NodeKind::Image(ImageData {
                role: ImageRole::Source,
                blob,
                opacity: 1.0,
                natural_width: w,
                natural_height: h,
                name: Some(filename),
            }),
        },
    );

    app.apply(Op::AddPage { page, at: 0 })?;
    Ok(page_id)
}

/// Compose the step list. Order preference:
/// 1. `--steps a,b,c` — literal, in user-supplied order.
/// 2. Else: engines named in `config.pipeline.*` in the canonical order,
///    with `translator` included only when `--with-translate`.
fn resolve_steps(cli: &Cli, cfg: &AppConfig) -> Result<Vec<String>> {
    if let Some(s) = cli.steps.clone() {
        return Ok(s.into_iter().filter(|s| !s.is_empty()).collect());
    }
    let p = &cfg.pipeline;
    let mut steps: Vec<String> = Vec::new();
    let push = |v: &mut Vec<String>, s: &str| {
        if !s.is_empty() {
            v.push(s.to_string());
        }
    };
    push(&mut steps, &p.detector);
    push(&mut steps, &p.segmenter);
    push(&mut steps, &p.bubble_segmenter);
    push(&mut steps, &p.font_detector);
    push(&mut steps, &p.ocr);
    if cli.with_translate {
        push(&mut steps, &p.translator);
    }
    if cfg.typography_planner.enabled {
        push(&mut steps, &p.typography_planner);
    }
    push(&mut steps, &p.inpainter);
    push(&mut steps, &p.renderer);
    Ok(steps)
}

fn split_render_phase(steps: Vec<String>) -> Result<(Vec<String>, Vec<String>)> {
    let infos = steps
        .iter()
        .map(|id| koharu_app::Registry::find(id))
        .collect::<Result<Vec<_>>>()?;
    if infos
        .iter()
        .any(|info| info.produces.contains(&koharu_app::Artifact::Translations))
    {
        return Ok((steps, Vec::new()));
    }

    let mut first = Vec::new();
    let mut render = Vec::new();
    for (id, info) in steps.into_iter().zip(infos) {
        if info
            .produces
            .contains(&koharu_app::Artifact::TypographyStyles)
            || info.produces.contains(&koharu_app::Artifact::FinalRender)
        {
            render.push(id);
        } else {
            first.push(id);
        }
    }
    Ok((first, render))
}

fn require_clean_phase(outcome: &koharu_app::pipeline::RunOutcome) -> Result<()> {
    anyhow::ensure!(outcome.warning_count == 0, "pipeline phase failed");
    Ok(())
}

fn build_translation_fallback_ops(
    scene: &koharu_core::Scene,
    page: PageId,
    policy: koharu_app::config::SourceTextPolicy,
) -> Result<Vec<Op>> {
    if policy == koharu_app::config::SourceTextPolicy::HanOnly {
        let targets = koharu_app::pipeline::eligible_lines_for_page(scene, page).0;
        let translations = targets
            .iter()
            .map(|(_, line)| line.text.clone())
            .collect::<Vec<_>>();
        return koharu_app::pipeline::build_han_only_translation_ops(
            scene,
            page,
            None,
            &targets,
            &translations,
        );
    }

    let Some(page_data) = scene.pages.get(&page) else {
        return Ok(Vec::new());
    };
    let mut ops = Vec::new();
    for (id, node) in &page_data.nodes {
        if let NodeKind::Text(text) = &node.kind
            && text.translation.is_none()
            && let Some(raw) = text.text.as_ref().filter(|source| !source.is_empty())
        {
            ops.push(Op::UpdateNode {
                page,
                id: *id,
                patch: koharu_core::NodePatch {
                    data: Some(koharu_core::NodeDataPatch::Text(
                        koharu_core::TextDataPatch {
                            translation: Some(Some(raw.clone())),
                            ..Default::default()
                        },
                    )),
                    transform: None,
                    visible: None,
                },
                prev: koharu_core::NodePatch::default(),
            });
        }
    }
    Ok(ops)
}

/// Populate renderer input when no translator engine was selected. HanOnly
/// copies only eligible Han lines and clears stale English/unsupported output;
/// AllText preserves the existing node-level OCR fallback.
async fn synthesize_translations(app: &App, page: PageId) -> Result<()> {
    let session = app
        .current_session()
        .ok_or_else(|| anyhow!("no session open"))?;
    let ops = build_translation_fallback_ops(
        &session.scene.read(),
        page,
        app.config.load().pipeline.source_text_policy,
    )?;
    if ops.is_empty() {
        return Ok(());
    }
    app.apply(Op::Batch {
        ops,
        label: "synthesize translations".into(),
    })?;
    Ok(())
}

/// Walk the final scene and dump every role-keyed image/mask to disk.
fn dump_artifacts(
    session: &koharu_app::ProjectSession,
    page: PageId,
    out_dir: &std::path::Path,
) -> Result<()> {
    let scene = session.scene.read();
    let page_data = scene
        .pages
        .get(&page)
        .ok_or_else(|| anyhow!("page disappeared from scene"))?;

    for node in page_data.nodes.values() {
        match &node.kind {
            NodeKind::Image(img) => {
                let name = match img.role {
                    ImageRole::Source => "source.png",
                    ImageRole::Inpainted => "inpainted.png",
                    ImageRole::Rendered => "rendered.png",
                    ImageRole::Custom => continue,
                };
                save_blob_image(session, &img.blob, &out_dir.join(name))?;
            }
            NodeKind::Mask(m) => {
                let name = match m.role {
                    MaskRole::Segment => "segment.png",
                    MaskRole::Bubble => "bubble.png",
                    MaskRole::BrushInpaint => "brush.png",
                };
                save_blob_image(session, &m.blob, &out_dir.join(name))?;
            }
            NodeKind::Text(_) => {}
        }
    }

    // Dump the full scene JSON for diffing / inspection.
    let scene_json = serde_json::to_string_pretty(&*scene)?;
    std::fs::write(out_dir.join("scene.json"), scene_json)?;

    eprintln!("=> wrote artifacts to {}", out_dir.display());
    Ok(())
}

fn save_blob_image(
    session: &koharu_app::ProjectSession,
    blob: &koharu_core::BlobRef,
    path: &std::path::Path,
) -> Result<()> {
    let img: DynamicImage = session.blobs.load_image(blob)?;
    img.save(path)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use koharu_app::config::SourceTextPolicy;
    use koharu_core::{BlobRef, Scene, TextData, TextDirection};

    #[test]
    fn cli_options_inherits_source_text_policy() {
        let mut config = AppConfig::default();
        config.pipeline.source_text_policy = SourceTextPolicy::AllText;
        let cli = Cli {
            input: PathBuf::from("input.png"),
            output_dir: PathBuf::from("output"),
            config: None,
            steps: Some(vec!["llm".to_string()]),
            target_lang: "fr".to_string(),
            system_prompt: Some("system".to_string()),
            default_font: Some("Noto Sans".to_string()),
            with_translate: false,
            llm: None,
            cpu: true,
            data_root: None,
        };

        let options = pipeline_options(&cli, &config);

        assert_eq!(options.source_text_policy, SourceTextPolicy::AllText);
        assert_eq!(options.target_language.as_deref(), Some("fr"));
        assert_eq!(options.system_prompt.as_deref(), Some("system"));
        assert_eq!(options.default_font.as_deref(), Some("Noto Sans"));
        assert_eq!(cli.steps.as_deref(), Some(["llm".to_string()].as_slice()));
    }

    #[test]
    fn cli_fallback_splits_renderer_without_translator() {
        let (first, render) = split_render_phase(vec![
            "koharu-renderer".to_string(),
            "comic-text-detector".to_string(),
        ])
        .unwrap();
        assert_eq!(first, vec!["comic-text-detector"]);
        assert_eq!(render, vec!["koharu-renderer"]);

        let (first, render) = split_render_phase(vec!["koharu-renderer".to_string()]).unwrap();
        assert!(first.is_empty());
        assert_eq!(render, vec!["koharu-renderer"]);
    }

    #[test]
    fn cli_fallback_keeps_explicit_translator_chain() {
        let steps = vec!["llm".to_string(), "koharu-renderer".to_string()];
        let (first, render) = split_render_phase(steps.clone()).unwrap();
        assert_eq!(first, steps);
        assert!(render.is_empty());
    }

    #[test]
    fn cli_fallback_requires_clean_phase() {
        assert!(
            require_clean_phase(&koharu_app::pipeline::RunOutcome { warning_count: 0 }).is_ok()
        );
        assert!(
            require_clean_phase(&koharu_app::pipeline::RunOutcome { warning_count: 1 }).is_err()
        );
    }

    fn fallback_scene() -> (Scene, PageId, NodeId, NodeId) {
        let mut page = Page::new("page", 100, 100);
        let page_id = page.id;
        let mixed = NodeId::new();
        let english = NodeId::new();
        let node = |id, text: &str, polygons| Node {
            id,
            transform: Transform {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 50.0,
                rotation_deg: 0.0,
            },
            visible: true,
            kind: NodeKind::Text(TextData {
                text: Some(text.to_string()),
                translation: Some("old".to_string()),
                sprite: Some(
                    BlobRef::parse(
                        "4a046e33ecf7aced9bfd000747bb1fda7836c8ceeff662af33c2a2c288b4e78c",
                    )
                    .unwrap(),
                ),
                sprite_transform: Some(Transform::default()),
                line_polygons: polygons,
                source_direction: Some(TextDirection::Horizontal),
                ..Default::default()
            }),
        };
        page.nodes.insert(
            mixed,
            node(
                mixed,
                "English\n中文",
                Some(vec![
                    [[10.0, 10.0], [90.0, 10.0], [90.0, 30.0], [10.0, 30.0]],
                    [[10.0, 35.0], [90.0, 35.0], [90.0, 55.0], [10.0, 55.0]],
                ]),
            ),
        );
        page.nodes.insert(english, node(english, "English", None));
        let mut scene = Scene::default();
        scene.pages.insert(page_id, page);
        (scene, page_id, mixed, english)
    }

    fn text(scene: &Scene, page: PageId, id: NodeId) -> &TextData {
        match &scene.node(page, id).unwrap().kind {
            NodeKind::Text(text) => text,
            _ => panic!("expected text node"),
        }
    }

    #[test]
    fn cli_fallback_han_only_and_all_text_policy() {
        let (mut han_scene, page, mixed, english) = fallback_scene();
        let mut ops =
            build_translation_fallback_ops(&han_scene, page, SourceTextPolicy::HanOnly).unwrap();
        for op in &mut ops {
            op.apply(&mut han_scene).unwrap();
        }
        assert_eq!(
            text(&han_scene, page, mixed).translation.as_deref(),
            Some("中文")
        );
        assert!(text(&han_scene, page, english).translation.is_none());
        assert!(text(&han_scene, page, english).sprite.is_none());

        let (mut all_scene, page, mixed, english) = fallback_scene();
        for id in [mixed, english] {
            let NodeKind::Text(text) = &mut all_scene.node_mut(page, id).unwrap().kind else {
                panic!("expected text node");
            };
            text.translation = None;
        }
        let mut ops =
            build_translation_fallback_ops(&all_scene, page, SourceTextPolicy::AllText).unwrap();
        for op in &mut ops {
            op.apply(&mut all_scene).unwrap();
        }
        assert_eq!(
            text(&all_scene, page, mixed).translation.as_deref(),
            Some("English\n中文")
        );
        assert_eq!(
            text(&all_scene, page, english).translation.as_deref(),
            Some("English")
        );
    }

    #[tokio::test]
    async fn typography_planner_cli_runs_fallback_before_planner_and_renderer() {
        let mut config = AppConfig::default();
        config.typography_planner.enabled = true;
        let cli = Cli {
            input: PathBuf::from("input"),
            output_dir: PathBuf::from("output"),
            config: None,
            steps: None,
            target_lang: "en".into(),
            system_prompt: None,
            default_font: None,
            with_translate: false,
            llm: None,
            cpu: true,
            data_root: None,
        };
        let steps = resolve_steps(&cli, &config).unwrap();
        let (first, render) = split_render_phase(steps).unwrap();
        let blocked_fallback = Arc::new(AtomicBool::new(false));
        let blocked_flag = blocked_fallback.clone();
        let error = run_pipeline_phases(
            vec!["first".into()],
            vec!["cloud-typography-planner".into()],
            |_| async { Ok(koharu_app::pipeline::RunOutcome { warning_count: 1 }) },
            move || async move {
                blocked_flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .expect_err("warning in the first phase must block fallback");
        assert!(error.to_string().contains("pipeline phase failed"));
        assert!(!blocked_fallback.load(Ordering::SeqCst));

        let events = Arc::new(Mutex::new(Vec::new()));
        let first_clean = Arc::new(AtomicBool::new(false));
        let fallback_done = Arc::new(AtomicBool::new(false));
        let phase_events = events.clone();
        let fallback_events = events.clone();
        let phase_clean = first_clean.clone();
        let phase_fallback = fallback_done.clone();

        run_pipeline_phases(
            first,
            render,
            move |steps| {
                let events = phase_events.clone();
                let clean = phase_clean.clone();
                let fallback_done = phase_fallback.clone();
                async move {
                    if steps.iter().any(|step| step == "cloud-typography-planner") {
                        assert!(fallback_done.load(Ordering::SeqCst));
                        events
                            .lock()
                            .unwrap()
                            .extend(steps.into_iter().filter(|step| {
                                step == "cloud-typography-planner" || step == "koharu-renderer"
                            }));
                    } else {
                        events.lock().unwrap().push("first-phase".into());
                        clean.store(true, Ordering::SeqCst);
                    }
                    Ok(koharu_app::pipeline::RunOutcome { warning_count: 0 })
                }
            },
            move || {
                let events = fallback_events.clone();
                async move {
                    assert!(first_clean.load(Ordering::SeqCst));
                    events.lock().unwrap().push("fallback".into());
                    fallback_done.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(
            *events.lock().unwrap(),
            [
                "first-phase",
                "fallback",
                "cloud-typography-planner",
                "koharu-renderer",
            ]
        );
    }

    #[tokio::test]
    async fn typography_planner_cli_translator_steps_skip_fallback() {
        let steps = vec![
            "llm".to_string(),
            "cloud-typography-planner".to_string(),
            "koharu-renderer".to_string(),
        ];
        let (first, render) = split_render_phase(steps.clone()).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let phase_events = events.clone();
        let fallback_called = Arc::new(AtomicBool::new(false));
        let fallback_flag = fallback_called.clone();

        run_pipeline_phases(
            first,
            render,
            move |steps| {
                let events = phase_events.clone();
                async move {
                    events.lock().unwrap().extend(steps);
                    Ok(koharu_app::pipeline::RunOutcome { warning_count: 0 })
                }
            },
            move || async move {
                fallback_flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(*events.lock().unwrap(), steps);
        assert!(!fallback_called.load(Ordering::SeqCst));
    }

    #[test]
    fn typography_planner_cli_default_config_uses_standard_loader() {
        let loaded = load_config_with(
            None,
            || {
                let mut config = AppConfig::default();
                config.providers.push(koharu_app::config::ProviderConfig {
                    id: "openai-compatible".into(),
                    base_url: Some("http://saved".into()),
                    api_key: Some(koharu_app::config::RedactedSecret::new("secret")),
                });
                Ok(config)
            },
            |_| panic!("standard loader already hydrates secrets"),
        )
        .unwrap();
        assert_eq!(
            loaded.providers[0].base_url.as_deref(),
            Some("http://saved")
        );
        assert_eq!(
            loaded.providers[0].api_key.as_ref().unwrap().expose(),
            "secret"
        );
    }

    #[test]
    fn typography_planner_cli_custom_config_hydrates_provider_secret() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "[[providers]]\nid = 'openai-compatible'\nbase_url = 'http://custom'\n",
        )
        .unwrap();
        let config = load_config_with(
            Some(file.path()),
            || panic!("custom config must not use standard loader"),
            |config| {
                config.providers[0].api_key = Some(koharu_app::config::RedactedSecret::new(
                    "custom-config-secret",
                ));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            config.providers[0].api_key.as_ref().unwrap().expose(),
            "custom-config-secret"
        );
        let serialized = toml::to_string(&config).unwrap();
        assert!(serialized.contains("[REDACTED]"));
        assert!(!serialized.contains("custom-config-secret"));
    }

    #[test]
    fn typography_planner_cli_custom_config_propagates_secret_store_errors() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "[[providers]]\nid = 'openai-compatible'\n").unwrap();

        let error = load_config_with(
            Some(file.path()),
            || panic!("custom config must not use standard loader"),
            |_| anyhow::bail!("credential store unavailable"),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "credential store unavailable");
    }
}
