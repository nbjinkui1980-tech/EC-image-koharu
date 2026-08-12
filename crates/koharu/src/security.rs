use std::sync::{Arc, Mutex};

use koharu_rpc::security::{BrowserSessionState, SecurityContext};

pub struct DesktopAuth {
    security: SecurityContext,
    session: BrowserSessionState,
    startup_proof: Arc<Mutex<Option<[u8; 32]>>>,
}

impl DesktopAuth {
    pub fn generate() -> anyhow::Result<Self> {
        let master = koharu_rpc::security::generate_token();
        let proof = koharu_rpc::security::generate_token();
        let session = koharu_rpc::security::generate_token();
        Ok(Self {
            security: SecurityContext::from_secret(master),
            session: BrowserSessionState::new(Some(proof), session),
            startup_proof: Arc::new(Mutex::new(Some(proof))),
        })
    }

    pub fn security_context(&self) -> SecurityContext {
        self.security.clone()
    }

    pub fn browser_session_state(&self) -> BrowserSessionState {
        self.session.clone()
    }

    fn take_startup_proof(&self) -> Option<[u8; 32]> {
        self.startup_proof.lock().unwrap().take()
    }
}

pub struct HeadlessSecurityOptions {
    pub secret_from_env: Option<String>,
    pub secret_file: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub bind_host: String,
}

impl HeadlessSecurityOptions {
    pub fn resolve(self) -> anyhow::Result<ResolvedHeadlessSecurity> {
        if !is_loopback_host(&self.bind_host) && self.allowed_hosts.is_empty() {
            anyhow::bail!("non-loopback headless binding requires at least one --allowed-host")
        }
        let secret = match (self.secret_from_env, self.secret_file) {
            (Some(_), Some(_)) => {
                anyhow::bail!("KOHARU_AUTH_SECRET and --auth-secret-file are mutually exclusive")
            }
            (Some(encoded), None) => decode_headless_secret(&encoded)?,
            (None, Some(path)) => {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("cannot read auth secret file {path}: {e}"))?;
                decode_headless_secret(content.trim())?
            }
            (None, None) => {
                anyhow::bail!("headless mode requires KOHARU_AUTH_SECRET or --auth-secret-file")
            }
        };
        let remote_policy = koharu_rpc::security::RemoteHostPolicy::parse(&self.allowed_hosts)?;
        Ok(ResolvedHeadlessSecurity {
            security: SecurityContext::from_secret(secret),
            session: BrowserSessionState::new(None, koharu_rpc::security::generate_token()),
            remote_policy,
        })
    }
}

pub(crate) fn validate_desktop_options(
    headless: bool,
    bind_host: &str,
    secret_file: Option<&str>,
    allowed_hosts: &[String],
) -> anyhow::Result<()> {
    if headless {
        return Ok(());
    }
    if secret_file.is_some() {
        anyhow::bail!("--auth-secret-file is only valid with --headless");
    }
    if !allowed_hosts.is_empty() {
        anyhow::bail!("--allowed-host is only valid with --headless");
    }
    if !is_loopback_host(bind_host) {
        anyhow::bail!(
            "Desktop mode only supports loopback binding. Use --headless for remote exposure with --allowed-host."
        );
    }
    Ok(())
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn decode_headless_secret(encoded: &str) -> anyhow::Result<[u8; 32]> {
    use base64::Engine;

    if encoded.len() != 43 {
        anyhow::bail!("auth secret must be 43 characters (32 bytes URL-safe no-padding base64)");
    }
    let mut buf = [0u8; 32];
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode_slice(encoded, &mut buf)
        .map_err(|_| anyhow::anyhow!("invalid auth secret encoding"))?;
    Ok(buf)
}

pub struct ResolvedHeadlessSecurity {
    pub security: SecurityContext,
    pub session: BrowserSessionState,
    pub remote_policy: koharu_rpc::security::RemoteHostPolicy,
}

#[tauri::command]
pub fn desktop_bootstrap_proof(
    window: tauri::Window,
    state: tauri::State<'_, DesktopAuth>,
) -> Result<String, String> {
    if window.label() != "main" {
        return Err("unauthorized window".into());
    }
    let proof = state
        .take_startup_proof()
        .ok_or_else(|| "startup proof already consumed".to_owned())?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        proof,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SECRET: &str = "KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio";

    #[test]
    fn remote_headless_binding_requires_an_explicit_allowed_host() {
        let result = HeadlessSecurityOptions {
            secret_from_env: Some(TEST_SECRET.into()),
            secret_file: None,
            allowed_hosts: Vec::new(),
            bind_host: "0.0.0.0".into(),
        }
        .resolve();
        assert!(result.is_err());
    }

    #[test]
    fn headless_rejects_malformed_conflicting_and_wildcard_credentials() {
        for options in [
            HeadlessSecurityOptions {
                secret_from_env: Some("malformed".into()),
                secret_file: None,
                allowed_hosts: Vec::new(),
                bind_host: "127.0.0.1".into(),
            },
            HeadlessSecurityOptions {
                secret_from_env: Some(TEST_SECRET.into()),
                secret_file: Some("unused".into()),
                allowed_hosts: Vec::new(),
                bind_host: "127.0.0.1".into(),
            },
            HeadlessSecurityOptions {
                secret_from_env: Some(TEST_SECRET.into()),
                secret_file: None,
                allowed_hosts: vec!["*.example.com".into()],
                bind_host: "127.0.0.1".into(),
            },
        ] {
            assert!(options.resolve().is_err());
        }
    }

    #[test]
    fn desktop_startup_proof_is_fixed_and_returned_once() {
        let auth = DesktopAuth::generate().unwrap();
        let proof = auth.take_startup_proof().expect("startup proof");

        assert!(auth.take_startup_proof().is_none());
        assert!(auth.browser_session_state().consume_proof(&proof));
        assert!(!auth.browser_session_state().consume_proof(&proof));
    }

    #[test]
    fn desktop_options_fail_closed_before_bind() {
        assert!(validate_desktop_options(false, "0.0.0.0", None, &[]).is_err());
        assert!(validate_desktop_options(false, "127.0.0.1", Some("secret"), &[]).is_err());
        assert!(
            validate_desktop_options(false, "127.0.0.1", None, &["example.com".into()]).is_err()
        );

        for host in ["127.0.0.1", "::1", "localhost", "LOCALHOST"] {
            assert!(validate_desktop_options(false, host, None, &[]).is_ok());
        }
    }
}
