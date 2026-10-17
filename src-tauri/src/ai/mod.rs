//! AI 機能（feature "ai" のみ）。設計は AI.md を参照。
//!
//! AI なし版のビルドにはこのモジュールを含めない。

mod agent;
mod audit;
pub mod chat;
mod config;
mod provider;
mod session;
mod terminal_buffer;
mod tools;

pub use session::{AiSessionRequest, ReadUser};
pub use terminal_buffer::{TerminalBuffer, MAX_LINES as TERMINAL_BUFFER_LINES};

use std::collections::HashMap;

use serde::Serialize;

use crate::credentials;
use config::Provider;

/// AI あり版のビルドであることを示す目印。AI なし版の exe にこの文字列がないことを
/// scripts/check-no-ai.ts で確認する（コマンド名は exe 内で分割されて格納されるため目印に使えない）
const BUILD_MARKER: &str = "nterm-ai-build";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// AI を使える状態か（接続先の設定が見つかったか）
    pub ready: bool,
    /// 接続先の説明（使える場合）または使えない理由
    pub message: String,
    pub build: &'static str,
}

/// AI 機能の状態（チャットパネルの表示用）
#[tauri::command]
pub fn ai_status() -> AiStatus {
    let (ready, message) = match load_provider() {
        Ok(provider) => (true, provider.describe()),
        Err(message) => (false, message),
    };
    AiStatus { ready, message, build: BUILD_MARKER }
}

/// 接続先を決める（Claude Code の settings.json・環境変数・保存した API キー）
fn load_provider() -> Result<Provider, String> {
    let settings_env = match config::claude_settings_path() {
        Some(path) => config::read_settings_env(&path)?,
        None => HashMap::new(),
    };
    let saved_key = credentials::load_secret(&credentials::credentials_path()?, config::ANTHROPIC_KEY_ID)?;
    config::resolve(&|key| std::env::var(key).ok(), &settings_env, saved_key)
}
