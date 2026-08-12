//! Binary entry point. Wires `koharu-app::App` to the axum router plus
//! (optionally) Tauri.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use koharu_app::{App, AppConfig, config as app_config};
use koharu_rpc::{
    BootstrapManager, security::OriginHostPolicy, security::RemoteHostPolicy, server,
};
use koharu_runtime::{ComputePolicy, RuntimeHttpConfig, RuntimeManager};
use tokio::net::TcpListener;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::cli::Cli;

fn desktop_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

fn desktop_builder<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    desktop_auth: crate::security::DesktopAuth,
) -> tauri::Builder<R> {
    builder
        .manage(desktop_auth)
        .invoke_handler(tauri::generate_handler![
            crate::security::desktop_bootstrap_proof
        ])
}

async fn bootstrap_app(
    state: Arc<BootstrapManager>,
    config: AppConfig,
    cpu_only: bool,
) -> Result<()> {
    let runtime = state.runtime();
    runtime
        .prepare()
        .await
        .context("failed to prepare runtime")?;

    let app = Arc::new(App::new_with_shared_state(
        config,
        runtime,
        cpu_only,
        state.shared_state(),
        crate::version::current(),
    )?);
    koharu_llm::suppress_native_logs();
    app.spawn_llm_forwarder();
    state
        .set_app(app)
        .map_err(|_| anyhow::anyhow!("app already initialized"))?;
    Ok(())
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    #[cfg(target_os = "windows")]
    {
        let attached = crate::windows::attach_parent_console();
        if !attached && (cli.headless || cli.debug) {
            crate::windows::create_console_window();
        }
        crate::windows::enable_ansi_support().ok();
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::filter::EnvFilter::builder()
                .with_default_directive(tracing::Level::INFO.into())
                .from_env_lossy(),
        )
        .with(crate::sentry::tracing_layer())
        .with(crate::tracing::TimingLayer::new())
        .init();

    let config: AppConfig = app_config::load()?;
    let http = RuntimeHttpConfig {
        connect_timeout_secs: config.http.connect_timeout.max(1),
        read_timeout_secs: config.http.read_timeout.max(1),
        max_retries: config.http.max_retries,
    };
    let compute = if cli.cpu {
        ComputePolicy::CpuOnly
    } else {
        ComputePolicy::PreferGpu
    };

    if cli.download {
        return RuntimeManager::new_with_http(config.data.path.as_std_path(), compute, http)?
            .prepare()
            .await
            .context("failed to download runtime packages");
    }

    let state = BootstrapManager::new(Arc::new(RuntimeManager::new_with_http(
        config.data.path.as_std_path(),
        compute,
        http,
    )?));
    state.spawn_download_forwarder();

    #[cfg(target_os = "windows")]
    crate::windows::register_khr().ok();

    let bind_host = cli.host.as_deref().unwrap_or("127.0.0.1");
    let bind_port = cli.port.unwrap_or(4000);

    validate_pre_bind(&PreBindInput {
        headless: cli.headless,
        host: bind_host.to_string(),
        allowed_hosts: cli.allowed_host.clone(),
        auth_secret_file: cli.auth_secret_file.clone(),
        has_env_secret: std::env::var("KOHARU_AUTH_SECRET").is_ok(),
    })?;

    let headless_security = if cli.headless {
        Some(
            crate::security::HeadlessSecurityOptions {
                secret_from_env: std::env::var("KOHARU_AUTH_SECRET").ok(),
                secret_file: cli.auth_secret_file.clone(),
                allowed_hosts: cli.allowed_host.clone(),
            }
            .resolve()?,
        )
    } else {
        None
    };

    let listener: TcpListener = if cfg!(debug_assertions) || cli.port.is_some() {
        TcpListener::bind((bind_host, bind_port)).await?
    } else {
        let mut port = bind_port;
        loop {
            match TcpListener::bind((bind_host, port)).await {
                Ok(listener) => break listener,
                Err(err) if err.kind() == std::io::ErrorKind::AddrInUse && port < u16::MAX => {
                    port += 1;
                }
                Err(err) => return Err(err.into()),
            }
        }
    };
    let port = listener.local_addr()?.port();
    tracing::info!(port, "starting server");

    let mut context = desktop_context();
    let assets = crate::assets::from_context(&mut context);
    let server_state = state.clone();
    let origin_policy = OriginHostPolicy::for_listener(
        listener.local_addr()?,
        cfg!(debug_assertions),
        RemoteHostPolicy::empty(),
    );

    if let Some(headless) = headless_security {
        let origin_policy = OriginHostPolicy::for_listener(
            listener.local_addr()?,
            cfg!(debug_assertions),
            headless.remote_policy.clone(),
        );
        tauri::async_runtime::spawn(async move {
            server::serve_with_listener_with_session(
                listener,
                server_state.clone(),
                headless.security,
                origin_policy,
                headless.session,
            )
            .await
            .expect("failed to start headless server");
        });
        tracing::info!(port, "headless: open http://127.0.0.1:{port}/ in a browser");
        bootstrap_app(state, config, cli.cpu).await?;
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }

    let desktop_auth = crate::security::DesktopAuth::generate()?;
    let auth_for_server = desktop_auth.security_context();
    let desktop_session = desktop_auth.browser_session_state();
    tauri::async_runtime::spawn(async move {
        server::serve_with_listener_and_assets_with_session(
            listener,
            server_state,
            auth_for_server,
            origin_policy,
            desktop_session,
            assets,
        )
        .await
        .expect("failed to start server");
    });

    desktop_builder(tauri::Builder::default(), desktop_auth)
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(move |handle| {
            tauri::async_runtime::spawn(async move {
                bootstrap_app(state, config, cli.cpu)
                    .await
                    .expect("failed to bootstrap app");
            });

            let cfg = handle.config();
            let url: tauri::Url = if cfg!(debug_assertions) {
                cfg.build
                    .dev_url
                    .as_ref()
                    .expect("dev_url must be set in dev mode")
                    .as_str()
                    .parse()?
            } else {
                format!("http://127.0.0.1:{port}").parse()?
            };
            let wc = cfg
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .expect("main window config not found");
            tauri::webview::WebviewWindowBuilder::from_config(handle, wc)?
                .build()?
                .navigate(url)?;

            Ok(())
        })
        .run(context)?;

    Ok(())
}

#[derive(Default)]
struct PreBindInput {
    headless: bool,
    host: String,
    allowed_hosts: Vec<String>,
    auth_secret_file: Option<String>,
    has_env_secret: bool,
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "::1" | "localhost")
}

fn validate_pre_bind(input: &PreBindInput) -> anyhow::Result<()> {
    if !input.headless {
        if input.auth_secret_file.is_some() {
            anyhow::bail!("--auth-secret-file is only valid with --headless");
        }
        if !input.allowed_hosts.is_empty() {
            anyhow::bail!("--allowed-host is only valid with --headless");
        }
    }

    if !input.headless && !is_loopback(&input.host) {
        anyhow::bail!(
            "Desktop mode only supports loopback binding. \
             Use --headless for remote exposure with --allowed-host."
        );
    }

    if input.headless {
        let has_file = input.auth_secret_file.is_some();
        if !input.has_env_secret && !has_file {
            anyhow::bail!("headless mode requires KOHARU_AUTH_SECRET or --auth-secret-file");
        }
        if input.has_env_secret && has_file {
            anyhow::bail!("KOHARU_AUTH_SECRET and --auth-secret-file are mutually exclusive");
        }

        if !is_loopback(&input.host) && input.allowed_hosts.is_empty() {
            anyhow::bail!(
                "Non-loopback headless binding requires --allowed-host. \
                 Specify at least one remote host that is allowed to connect."
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::{desktop_builder, desktop_context};

    fn invoke(window: &tauri::WebviewWindow<tauri::test::MockRuntime>) -> Result<String, String> {
        tauri::test::get_ipc_response(
            window,
            tauri::webview::InvokeRequest {
                cmd: "desktop_bootstrap_proof".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://127.0.0.1:4000".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.into(),
            },
        )
        .map(|body| body.deserialize::<String>().unwrap())
        .map_err(|value| value.as_str().unwrap().to_owned())
    }

    #[test]
    fn desktop_auth_command_uses_managed_one_time_proof() {
        let desktop_auth = crate::security::DesktopAuth::generate().unwrap();
        let session = desktop_auth.browser_session_state();
        let app = desktop_builder(tauri::test::mock_builder(), desktop_auth)
            .build(desktop_context())
            .unwrap();

        let other = tauri::WebviewWindowBuilder::new(&app, "other", Default::default())
            .build()
            .unwrap();
        assert!(invoke(&other).is_err());

        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let encoded = invoke(&main).unwrap();
        assert_eq!(encoded.len(), 43);
        let decoded: [u8; 32] = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .unwrap()
            .try_into()
            .unwrap();
        assert!(session.consume_proof(&decoded));
        assert_eq!(invoke(&main), Err("proof already consumed".into()));
    }
}

#[cfg(test)]
mod pre_bind_tests {
    use super::*;

    #[test]
    fn headless_without_secret_fails_before_bind() {
        let input = PreBindInput {
            headless: true,
            host: "127.0.0.1".into(),
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn headless_with_non_loopback_host_and_empty_allowed_fails() {
        let input = PreBindInput {
            headless: true,
            host: "0.0.0.0".into(),
            has_env_secret: true,
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn desktop_with_non_loopback_host_fails_before_bind() {
        let input = PreBindInput {
            host: "0.0.0.0".into(),
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn desktop_with_auth_secret_file_fails() {
        let input = PreBindInput {
            auth_secret_file: Some("/tmp/secret".into()),
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn desktop_with_allowed_hosts_fails() {
        let input = PreBindInput {
            allowed_hosts: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_err());
    }

    #[test]
    fn headless_with_secret_and_loopback_passes() {
        let input = PreBindInput {
            headless: true,
            host: "127.0.0.1".into(),
            has_env_secret: true,
            allowed_hosts: vec!["example.com:443".into()],
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_ok());
    }

    #[test]
    fn headless_with_secret_allowed_hosts_and_wildcard_host_passes() {
        let input = PreBindInput {
            headless: true,
            host: "0.0.0.0".into(),
            has_env_secret: true,
            allowed_hosts: vec!["example.com".into()],
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_ok());
    }

    #[test]
    fn desktop_with_defaults_passes() {
        let input = PreBindInput {
            host: "127.0.0.1".into(),
            ..Default::default()
        };
        assert!(validate_pre_bind(&input).is_ok());
    }
}
