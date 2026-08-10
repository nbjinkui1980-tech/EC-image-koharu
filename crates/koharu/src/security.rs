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

#[tauri::command]
pub fn desktop_bootstrap_proof(
    window: tauri::Window,
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
