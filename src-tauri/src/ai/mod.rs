//! AI 機能（feature "ai" のみ）。設計は AI.md を参照。
//!
//! AI なし版のビルドにはこのモジュールを含めない。

mod terminal_buffer;

use serde::Serialize;

/// AI あり版のビルドであることを示す目印。AI なし版の exe にこの文字列がないことを
/// scripts/check-no-ai.ts で確認する（コマンド名は exe 内で分割されて格納されるため目印に使えない）
const BUILD_MARKER: &str = "nterm-ai-build";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// AI を使える状態か（接続先の設定が見つかったか）
    pub ready: bool,
    pub message: String,
    pub build: &'static str,
}

/// AI 機能の状態（チャットパネルの表示用）
#[tauri::command]
pub fn ai_status() -> AiStatus {
    AiStatus {
        ready: false,
        message: "AI features are not implemented yet.".to_string(),
        build: BUILD_MARKER,
    }
}
