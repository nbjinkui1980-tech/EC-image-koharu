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

    let bind_host = cli.host.as_deref().unwrap_or("127.0.0.1");
    let bind_port = cli.port.unwrap_or(4000);
    crate::security::validate_desktop_options(
        cli.headless,
        bind_host,
        cli.auth_secret_file.as_deref(),
        &cli.allowed_host,
    )?;
    let headless = if cli.headless && !cli.download {
        Some(
            crate::security::HeadlessSecurityOptions {
                secret_from_env: std::env::var("KOHARU_AUTH_SECRET").ok(),
                secret_file: cli.auth_secret_file.clone(),
                allowed_hosts: cli.allowed_host.clone(),
                bind_host: bind_host.to_owned(),
            }
            .resolve()?,
        )
    } else {
        None
    };

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

    let mut context = tauri::generate_context!();
    let assets = crate::assets::from_context(&mut context);
    let server_state = state.clone();
    let origin_policy = OriginHostPolicy::for_listener(
        listener.local_addr()?,
        cfg!(debug_assertions),
        RemoteHostPolicy::empty(),
    );

    if let Some(headless) = headless {
        let origin_policy = OriginHostPolicy::for_listener(
            listener.local_addr()?,
            cfg!(debug_assertions),
            headless.remote_policy.clone(),
        );
        tauri::async_runtime::spawn(async move {
            server::serve_with_listener_and_assets_with_session(
                listener,
                server_state,
                headless.security,
                origin_policy,
                headless.session,
                assets,
            )
            .await
            .expect("failed to start headless server");
        });
        tracing::info!(port, host = bind_host, "headless server ready");
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

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(desktop_auth)
        .invoke_handler(tauri::generate_handler![
            crate::security::desktop_bootstrap_proof
        ])
        .setup(move |handle| {
            tauri::async_runtime::spawn(async move {
                bootstrap_app(state, config, cli.cpu)
                    .await
                    .expect("failed to bootstrap app");
            });

            let cfg = handle.config();
            let service_origin: tauri::Url = format!("http://127.0.0.1:{port}").parse()?;
            let dev_origin: Option<tauri::Url> = if cfg!(debug_assertions) {
                Some(
                    cfg.build
                        .dev_url
                        .as_ref()
                        .expect("dev_url must be set in dev mode")
                        .as_str()
                        .parse()?,
                )
            } else {
                None
            };
            let url = if cfg!(debug_assertions) {
                dev_origin.clone().expect("dev origin in dev mode")
            } else {
                service_origin.clone()
            };
            let wc = cfg
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .expect("main window config not found");
            tauri::webview::WebviewWindowBuilder::from_config(handle, wc)?
                .on_navigation(move |target| {
                    navigation_allowed(target, &service_origin, dev_origin.as_ref())
                })
                .build()?
                .navigate(url)?;

            Ok(())
        })
        .run(context)?;

    Ok(())
}

/// Decide whether the main webview may navigate to `url`: only the active
/// service origin, plus the dev-server origin in debug builds. Everything
/// else is rejected so external content can never replace the main webview.
fn navigation_allowed(
    url: &tauri::Url,
    service_origin: &tauri::Url,
    dev_origin: Option<&tauri::Url>,
) -> bool {
    fn same_origin(a: &tauri::Url, b: &tauri::Url) -> bool {
        a.scheme() == b.scheme()
            && a.host() == b.host()
            && a.port_or_known_default() == b.port_or_known_default()
    }
    if same_origin(url, service_origin) {
        return true;
    }
    cfg!(debug_assertions) && dev_origin.is_some_and(|dev| same_origin(url, dev))
}

#[cfg(test)]
mod navigation_tests {
    use super::navigation_allowed;

    fn url(input: &str) -> tauri::Url {
        tauri::Url::parse(input).unwrap()
    }

    // AR07-T02 RED: external origins must not replace the main webview; only
    // the service origin (and the dev origin in debug builds) may navigate.
    #[test]
    fn navigation_allows_service_origin() {
        let service = url("http://127.0.0.1:4000");
        assert!(navigation_allowed(
            &url("http://127.0.0.1:4000/"),
            &service,
            None
        ));
        assert!(navigation_allowed(
            &url("http://127.0.0.1:4000/api/v1/scene.json"),
            &service,
            None
        ));
    }

    #[test]
    fn navigation_rejects_external_origin() {
        let service = url("http://127.0.0.1:4000");
        for target in [
            "https://evil.example/",
            "http://127.0.0.1:9999/x",
            "http://localhost:4000/",
            "about:blank",
        ] {
            assert!(
                !navigation_allowed(&url(target), &service, None),
                "must reject {target}"
            );
        }
    }

    #[test]
    fn navigation_dev_origin_allowed_only_in_debug() {
        let service = url("http://127.0.0.1:4000");
        let dev = url("http://localhost:3000");
        let target = url("http://localhost:3000/settings");
        assert_eq!(
            navigation_allowed(&target, &service, Some(&dev)),
            cfg!(debug_assertions),
            "dev origin allowed only in debug builds"
        );
        assert!(!navigation_allowed(&target, &service, None));
    }
}
