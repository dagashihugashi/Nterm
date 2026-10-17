//! セッションスレッドへの AI の要求（AI あり版のみ。ssh_client.rs の `SessionCommand::Ai`）。

use std::sync::mpsc::Sender;

use serde::Serialize;

use crate::reader_shell::RunReply;

pub enum AiSessionRequest {
    /// 組み立て済みの読み取りコマンドを隠しシェルで実行する
    Run { command: String, max_bytes: usize, reply: RunReply },
    /// ターミナルの直近の出力
    ReadTerminal { lines: usize, reply: Sender<Vec<String>> },
    /// AI がどのユーザの権限で読むか
    ReadUser { reply: Sender<ReadUser> },
}

/// AI がサーバを読むときのユーザ
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "user", rename_all = "camelCase")]
pub enum ReadUser {
    /// ログインユーザ（ユーザ切り替えの自動化を設定していない）
    Login,
    /// 自動化で切り替えた後のユーザ（root 等）
    Switched(String),
    /// 切り替えを設定しているが、まだ完了していない（または失敗した）
    Pending,
}
