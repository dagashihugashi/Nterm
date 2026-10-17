//! AI 機能（feature "ai" のみ）。設計は AI.md を参照（12 章：Claude Code 連携）。
//!
//! AI なし版のビルドにはこのモジュールを含めない。

pub mod chat;
mod claude_cli;
mod mcp_server;
mod model_settings;
mod project_instructions;
mod session;
mod sessions;
mod terminal_buffer;
mod tools;

pub use session::{AiSessionRequest, ReadUser};
pub use terminal_buffer::{TerminalBuffer, MAX_LINES as TERMINAL_BUFFER_LINES};

use serde::Serialize;

/// AI あり版のビルドであることを示す目印。AI なし版の exe にこの文字列がないことを
/// scripts/check-no-ai.mjs で確認する（コマンド名は exe 内で分割されて格納されるため目印に使えない）
const BUILD_MARKER: &str = "nterm-ai-build";
/// 既定の作業フォルダの名前（設定フォルダの下）
const WORKSPACE_DIR_NAME: &str = "ai-workspace";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// Claude Code を使える状態か
    pub ready: bool,
    /// Claude Code のバージョン（使える場合）または使えない理由
    pub message: String,
    /// 作業フォルダの既定値（Orca のフォルダ）
    pub default_cwd: Option<String>,
    pub build: &'static str,
}

/// AI 機能の状態（チャットパネルの表示用）
#[tauri::command]
pub fn ai_status() -> AiStatus {
    let (ready, message) = match claude_cli::find_claude().and_then(|path| claude_cli::version(&path)) {
        Ok(version) => (true, format!("Claude Code {}", version.trim_end_matches(" (Claude Code)"))),
        Err(message) => (false, message),
    };
    let default_cwd = default_working_folder().map(|dir| dir.display().to_string());
    AiStatus { ready, message, default_cwd, build: BUILD_MARKER }
}

/// 既定の作業フォルダ（exe の隣の空のフォルダ）。作業フォルダの中は Claude Code が承認なしで読めるため、
/// servers.json 等のある Orca のフォルダや、鍵・認証情報のあるホームのフォルダにはしない
fn default_working_folder() -> Option<std::path::PathBuf> {
    let dir = crate::config::config_dir().ok()?.join(WORKSPACE_DIR_NAME);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}
