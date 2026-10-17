//! AI チャットの Tauri コマンドと、エージェントのループとアプリ本体のつなぎ込み（AI あり版のみ）。
//!
//! - `ai_send`：質問 1 回分を別スレッドでエージェントのループに通し、進行状況をイベントで送る
//! - `ai_approve` / `ai_cancel` / `ai_reset`：承認・中止・新しい会話
//! - `ai_save_api_key` / `ai_delete_api_key`：Anthropic の API キーの保存・削除（DPAPI）
//!
//! ツールの実行はセッションスレッド経由で行い、承認はフロントエンドの回答を待つ。
//! 質問・承認/拒否・ツールの結果・回答・エラーは監査ログに記録する。

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use super::agent::{self, AgentEvent, Approval, Approver, Context, Conversation, EventSink, ToolExecutor, TurnEnd};
use super::audit::AuditLog;
use super::config as ai_config;
use super::provider::HttpModelClient;
use super::session::{AiSessionRequest, ReadUser};
use super::tools::{ToolCall, MAX_OUTPUT_BYTES};
use crate::credentials;
use crate::ssh_client::SessionCommand;

/// 読み取りツールの結果を待つ上限（隠しシェルのタイムアウト＋余裕）
const TOOL_REPLY_TIMEOUT: Duration = Duration::from_secs(40);
/// セッションスレッドに状態（読み取りユーザ・ターミナルの出力）を問い合わせる上限
const STATE_REPLY_TIMEOUT: Duration = Duration::from_secs(3);
/// 承認を待つ間に中止を確認する間隔
const APPROVAL_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// フロントエンドから渡される開いているタブの情報（list_sessions と承認ダイアログの表示用）
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    pub session_id: String,
    pub title: String,
    pub server_name: String,
    pub host: String,
    pub env: Option<String>,
    pub login_user: String,
}

/// フロントエンドへ送る進行状況
#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum ChatEvent {
    #[serde(rename_all = "camelCase")]
    Text { text: String },
    #[serde(rename_all = "camelCase")]
    ApprovalRequested {
        id: String,
        tool: String,
        summary: String,
        tab: Option<TabInfo>,
        read_user: Option<ReadUser>,
    },
    #[serde(rename_all = "camelCase")]
    ToolStarted { id: String, name: String, summary: String, session: Option<String> },
    #[serde(rename_all = "camelCase")]
    ToolFinished { id: String, output: String, is_error: bool },
    /// 質問の処理が終わった：completed / cancelled / iterationLimit
    #[serde(rename_all = "camelCase")]
    Finished { reason: String },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
    /// チャットは続けられる問題（監査ログの書き込み失敗など）
    #[serde(rename_all = "camelCase")]
    Warning { message: String },
}

type PendingApprovals = Arc<Mutex<HashMap<String, Sender<Approval>>>>;

/// チャットの状態（ウィンドウに 1 つ）
pub struct AiState {
    /// 会話の履歴。質問の処理中は取り出している（None）
    conversation: Mutex<Option<Conversation>>,
    /// 処理中の質問の中止フラグ
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    approvals: PendingApprovals,
}

impl Default for AiState {
    fn default() -> Self {
        Self {
            conversation: Mutex::new(Some(Conversation::default())),
            cancel: Mutex::new(None),
            approvals: Arc::default(),
        }
    }
}

impl AiState {
    /// 質問の処理を始める。履歴を取り出し、中止フラグを用意する（処理中なら Err）
    fn begin(&self) -> Result<(Conversation, Arc<AtomicBool>), String> {
        let mut conversation = self.conversation.lock().map_err(|_| "the AI chat is unavailable")?;
        let taken = conversation.take().ok_or("the AI is still answering the previous question")?;
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.lock().map_err(|_| "the AI chat is unavailable")? = Some(cancel.clone());
        Ok((taken, cancel))
    }

    fn finish(&self, conversation: Conversation) {
        if let Ok(mut slot) = self.conversation.lock() {
            *slot = Some(conversation);
        }
        if let Ok(mut cancel) = self.cancel.lock() {
            *cancel = None;
        }
    }
}

/// 質問する（結果は `on_event` へイベントで届く）
#[tauri::command]
pub fn ai_send(
    app: AppHandle,
    state: State<'_, AiState>,
    text: String,
    tabs: Vec<TabInfo>,
    on_event: Channel<ChatEvent>,
) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("the question is empty".to_string());
    }
    let provider = super::load_provider()?;
    let audit = AuditLog::new(&crate::config::config_dir()?);
    let (conversation, cancel) = state.begin()?;
    let approvals = state.approvals.clone();
    thread::spawn(move || {
        let mut conversation = conversation;
        let turn = Turn { app: &app, tabs: &tabs, channel: &on_event, audit: &audit, approvals, cancel: &cancel };
        turn.run(&mut conversation, &text, HttpModelClient::new(provider));
        app.state::<AiState>().finish(conversation);
    });
    Ok(())
}

/// 承認の要求に回答する
#[tauri::command]
pub fn ai_approve(state: State<'_, AiState>, id: String, allow: bool) -> Result<(), String> {
    let sender = state
        .approvals
        .lock()
        .map_err(|_| "the AI chat is unavailable")?
        .remove(&id)
        .ok_or_else(|| format!("no pending approval: {id}"))?;
    // 中止された場合はループが待つのをやめている
    let _ = sender.send(if allow { Approval::Allow } else { Approval::Deny });
    Ok(())
}

/// 処理中の質問を中止する（承認待ちのものは拒否として扱う）
#[tauri::command]
pub fn ai_cancel(state: State<'_, AiState>) -> Result<(), String> {
    if let Some(cancel) = state.cancel.lock().map_err(|_| "the AI chat is unavailable")?.as_ref() {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// 新しい会話を始める（履歴を破棄する）
#[tauri::command]
pub fn ai_reset(state: State<'_, AiState>) -> Result<(), String> {
    let mut conversation = state.conversation.lock().map_err(|_| "the AI chat is unavailable")?;
    match conversation.as_mut() {
        Some(conversation) => {
            *conversation = Conversation::default();
            Ok(())
        }
        None => Err("stop the current answer before starting a new chat".to_string()),
    }
}

/// Anthropic の API キーを保存する（DPAPI で暗号化して credentials.json へ）
#[tauri::command]
pub fn ai_save_api_key(key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() || key.chars().any(char::is_control) {
        return Err("the API key is invalid".to_string());
    }
    credentials::save_secret(&credentials::credentials_path()?, ai_config::ANTHROPIC_KEY_ID, key)
}

#[tauri::command]
pub fn ai_delete_api_key() -> Result<(), String> {
    credentials::delete_secret(&credentials::credentials_path()?, ai_config::ANTHROPIC_KEY_ID)
}

/// 質問 1 回分の処理に必要なもの
struct Turn<'a> {
    app: &'a AppHandle,
    tabs: &'a [TabInfo],
    channel: &'a Channel<ChatEvent>,
    audit: &'a AuditLog,
    approvals: PendingApprovals,
    cancel: &'a AtomicBool,
}

impl Turn<'_> {
    fn run(&self, conversation: &mut Conversation, text: &str, client: HttpModelClient) {
        let sink = ChatSink { channel: self.channel, audit: self.audit, audit_failed: Cell::new(false) };
        sink.audit("question", json!({ "text": text }));
        let tools = SessionTools { app: self.app, tabs: self.tabs };
        let approver = UiApprover { tools: &tools, sink: &sink, approvals: &self.approvals, cancel: self.cancel };
        let context = Context { client: &client, tools: &tools, approver: &approver, events: &sink, cancel: self.cancel };
        match agent::run_turn(&context, conversation, text) {
            Ok(end) => {
                let reason = match end {
                    TurnEnd::Completed => "completed",
                    TurnEnd::Cancelled => "cancelled",
                    TurnEnd::IterationLimit => "iterationLimit",
                };
                sink.send(ChatEvent::Finished { reason: reason.to_string() });
            }
            Err(message) => {
                sink.audit("error", json!({ "message": message }));
                sink.send(ChatEvent::Error { message });
            }
        }
    }
}

/// フロントエンドへイベントを送り、監査ログに記録する
struct ChatSink<'a> {
    channel: &'a Channel<ChatEvent>,
    audit: &'a AuditLog,
    /// 監査ログの書き込み失敗は 1 回だけ知らせる
    audit_failed: Cell<bool>,
}

impl ChatSink<'_> {
    fn send(&self, event: ChatEvent) {
        // パネルが既に閉じられている場合は届かなくてよい
        let _ = self.channel.send(event);
    }

    fn audit(&self, kind: &str, fields: Value) {
        if let Err(message) = self.audit.record(kind, fields) {
            if !self.audit_failed.replace(true) {
                self.send(ChatEvent::Warning { message: format!("the AI audit log could not be written: {message}") });
            }
        }
    }
}

impl EventSink for ChatSink<'_> {
    fn emit(&self, event: AgentEvent) {
        let event = match event {
            AgentEvent::Text { text } => {
                self.audit("answer", json!({ "text": text }));
                ChatEvent::Text { text }
            }
            AgentEvent::ToolStarted { id, name, summary, session } => {
                ChatEvent::ToolStarted { id, name, summary, session }
            }
            AgentEvent::ToolFinished { id, output, is_error } => {
                self.audit("tool_result", json!({ "id": id, "isError": is_error, "output": output }));
                ChatEvent::ToolFinished { id, output, is_error }
            }
        };
        self.send(event);
    }
}

/// 読み取りツールをセッションスレッド経由で実行する
struct SessionTools<'a> {
    app: &'a AppHandle,
    tabs: &'a [TabInfo],
}

impl SessionTools<'_> {
    fn tab(&self, session: &str) -> Result<&TabInfo, String> {
        self.tabs
            .iter()
            .find(|tab| tab.session_id == session)
            .ok_or_else(|| format!("unknown tab id: {session} (use list_sessions)"))
    }

    /// セッションスレッドに要求を送り、応答を待つ
    fn ask<T: Send + 'static>(
        &self,
        session: &str,
        timeout: Duration,
        make_request: impl FnOnce(Sender<T>) -> AiSessionRequest,
    ) -> Result<T, String> {
        let (reply, response) = mpsc::channel();
        crate::send_session_command(self.app, session, SessionCommand::Ai(make_request(reply)))?;
        response.recv_timeout(timeout).map_err(|_| format!("the tab {session} did not respond"))
    }

    fn read_user(&self, session: &str) -> Option<ReadUser> {
        self.ask(session, STATE_REPLY_TIMEOUT, |reply| AiSessionRequest::ReadUser { reply }).ok()
    }

    fn list_sessions(&self) -> String {
        let sessions: Vec<Value> = self
            .tabs
            .iter()
            .map(|tab| {
                let read_user = self.read_user(&tab.session_id);
                json!({
                    "tabId": tab.session_id,
                    "title": tab.title,
                    "server": tab.server_name,
                    "host": tab.host,
                    "environment": tab.env,
                    "loginUser": tab.login_user,
                    "connected": read_user.is_some(),
                    "readsAs": read_user,
                })
            })
            .collect();
        serde_json::to_string_pretty(&sessions).unwrap_or_else(|_| "[]".to_string())
    }
}

impl ToolExecutor for SessionTools<'_> {
    fn execute(&self, call: &ToolCall) -> Result<String, String> {
        if let ToolCall::ListSessions = call {
            return Ok(self.list_sessions());
        }
        let session = call.session().ok_or("the tool has no tab id")?;
        self.tab(session)?;
        if let ToolCall::ReadTerminal { lines, .. } = call {
            let lines = usize::try_from(*lines).unwrap_or(usize::MAX);
            let text = self.ask(session, STATE_REPLY_TIMEOUT, |reply| AiSessionRequest::ReadTerminal { lines, reply })?;
            return Ok(text.join("\n"));
        }
        let command = call.remote_command().ok_or("the tool does not run on the server")?;
        let output = self.ask(session, TOOL_REPLY_TIMEOUT, |reply| AiSessionRequest::Run {
            command,
            max_bytes: MAX_OUTPUT_BYTES,
            reply,
        })??;
        Ok(format_run_output(&output.text, output.status, output.truncated))
    }
}

/// 読み取りツールの結果として AI に渡すテキスト
fn format_run_output(text: &str, status: i32, truncated: bool) -> String {
    let mut result = text.to_string();
    if truncated {
        result.push_str(&format!(
            "\n\n[Output truncated at {MAX_OUTPUT_BYTES} bytes. Read a smaller range.]"
        ));
    }
    if status != 0 {
        result.push_str(&format!("\n(exit status {status})"));
    }
    result
}

/// フロントエンドに承認を求め、回答を待つ（中止された場合は拒否として扱う）
struct UiApprover<'a> {
    tools: &'a SessionTools<'a>,
    sink: &'a ChatSink<'a>,
    approvals: &'a PendingApprovals,
    cancel: &'a AtomicBool,
}

impl UiApprover<'_> {
    fn wait(&self, id: &str) -> Approval {
        let (sender, answer) = mpsc::channel();
        match self.approvals.lock() {
            Ok(mut approvals) => approvals.insert(id.to_string(), sender),
            Err(_) => return Approval::Deny,
        };
        let decision = loop {
            if self.cancel.load(Ordering::Relaxed) {
                break Approval::Deny;
            }
            match answer.recv_timeout(APPROVAL_POLL_INTERVAL) {
                Ok(decision) => break decision,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break Approval::Deny,
            }
        };
        if let Ok(mut approvals) = self.approvals.lock() {
            approvals.remove(id);
        }
        decision
    }
}

impl Approver for UiApprover<'_> {
    fn approve(&self, id: &str, call: &ToolCall) -> Approval {
        let tab = call.session().and_then(|session| self.tools.tab(session).ok()).cloned();
        let read_user = call.session().and_then(|session| self.tools.read_user(session));
        self.sink.send(ChatEvent::ApprovalRequested {
            id: id.to_string(),
            tool: call.name().to_string(),
            summary: call.summary(),
            tab: tab.clone(),
            read_user: read_user.clone(),
        });
        let decision = self.wait(id);
        self.sink.audit(
            "approval",
            json!({
                "id": id,
                "tool": call.name(),
                "summary": call.summary(),
                "server": tab.as_ref().map(|tab| &tab.server_name),
                "host": tab.as_ref().map(|tab| &tab.host),
                "environment": tab.as_ref().and_then(|tab| tab.env.as_ref()),
                "readsAs": read_user,
                "decision": if decision == Approval::Allow { "allow" } else { "deny" },
            }),
        );
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_run_output_with_truncation_and_status() {
        assert_eq!(format_run_output("abc", 0, false), "abc");
        assert!(format_run_output("abc", 0, true).contains("[Output truncated"));
        assert_eq!(format_run_output("", 2, false), "\n(exit status 2)");
    }

    #[test]
    fn only_one_question_runs_at_a_time() {
        let state = AiState::default();
        let (conversation, _cancel) = state.begin().unwrap();
        assert!(state.begin().is_err());
        state.finish(conversation);
        assert!(state.begin().is_ok());
    }

    #[test]
    fn serializes_events_for_the_frontend() {
        let event = ChatEvent::ToolFinished { id: "a".into(), output: "x".into(), is_error: false };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({ "event": "toolFinished", "data": { "id": "a", "output": "x", "isError": false } })
        );
        let read_user = serde_json::to_value(ReadUser::Switched("root".into())).unwrap();
        assert_eq!(read_user, json!({ "kind": "switched", "user": "root" }));
    }
}
