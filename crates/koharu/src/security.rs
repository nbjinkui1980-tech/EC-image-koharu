use std::sync::{Arc, Mutex};

use koharu_rpc::security::{BrowserSessionState, SecurityContext};

pub struct DesktopAuth {
    security: SecurityContext,
    session: BrowserSessionState,
    proof: Arc<Mutex<Option<[u8; 32]>>>,
}

impl DesktopAuth {
    pub fn generate() -> anyhow::Result<Self> {
        let master = koharu_rpc::security::generate_token();
        let proof = koharu_rpc::security::generate_token();
        let session = koharu_rpc::security::generate_token();
        Ok(Self {
            security: SecurityContext::from_secret(master),
            session: BrowserSessionState::new(Some(proof), session),
            proof: Arc::new(Mutex::new(Some(proof))),
        })
    }

    pub fn security_context(&self) -> SecurityContext {
        self.security.clone()
    }

    pub fn browser_session_state(&self) -> BrowserSessionState {
        self.session.clone()
    }

    pub fn take_proof(&self) -> Option<[u8; 32]> {
        self.proof.lock().unwrap().take()
    }
}

pub struct HeadlessSecurityOptions {
    pub secret_from_env: Option<String>,
    pub secret_file: Option<String>,
    pub allowed_hosts: Vec<String>,
}

impl HeadlessSecurityOptions {
    pub fn resolve(self) -> anyhow::Result<ResolvedHeadlessSecurity> {
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
            session: {
                let token = koharu_rpc::security::generate_token();
                BrowserSessionState::new(None, token)
            },
            remote_policy,
        })
    }
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
    window: tauri::Window<impl tauri::Runtime>,
    state: tauri::State<'_, DesktopAuth>,
) -> Result<String, String> {
    if window.label() != "main" {
        return Err("unauthorized window".into());
    }
    let proof = state.take_proof().ok_or("proof already consumed")?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        proof,
    ))
}
