//! AI チャット（Claude Code 連携）の Tauri コマンドと、Orca の道具の実装（AI あり版のみ。AI.md 12 章）。
//!
//! - `ai_send`：Claude Code（claude CLI）に質問を送る。進行状況は Claude Code の出力をイベントに変換して送る
//! - `ai_approve` / `ai_cancel` / `ai_reset`：承認・中止・新しい会話
//! - `ai_list_sessions` / `ai_resume`：作業フォルダのセッションの一覧・続きから再開
//!
//! Claude Code は Orca の MCP サーバ（`mcp_server.rs`）を通して、サーバの読み取りの道具と承認（`approve`）を使う。
//! 道具の実行はセッションスレッド経由で行い、承認はフロントエンドの回答を待つ。会話は Claude Code が保存する。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::claude_cli::{self, ClaudeProcess, CliEvent, LaunchOptions, Phase, APPROVE_TOOL, MCP_SERVER_NAME};
use super::mcp_server::{McpServer, ToolHandler, ToolOutcome};
use super::model_settings::{self, ModelOptions};
use super::project_instructions;
use super::session::{AiSessionRequest, ReadUser};
use super::sessions::{self, HistoryEntry, SessionSummary};
use super::tools::{self, ToolCall, MAX_OUTPUT_BYTES};
use crate::ssh_client::SessionCommand;

/// 読み取りの道具の結果を待つ上限（隠しシェルのタイムアウト＋余裕）
const TOOL_REPLY_TIMEOUT: Duration = Duration::from_secs(40);
/// セッションスレッドに状態（読み取りユーザ・ターミナルの出力）を問い合わせる上限
const STATE_REPLY_TIMEOUT: Duration = Duration::from_secs(3);
/// 承認を待つ間に中止を確認する間隔
const APPROVAL_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// 承認カード・道具の表示に出す引数の最大文字数（組み込みの道具の引数を JSON で見せる場合）
const MAX_SUMMARY_CHARS: usize = 300;
/// 進み具合の通知の最小間隔（同じ状態のまま届く途中経過で画面を更新しすぎない）
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// 出力トークン数の見積もりに使う 1 トークンあたりの文字数（正確な値は応答の終わりに届く）
const CHARS_PER_TOKEN: usize = 4;
/// 覚えておく許可の数（許可したまま実行されなかったものが溜まらないようにする）
const MAX_GRANTS: usize = 16;

/// フロントエンドから渡される開いているタブの情報（list_sessions と承認カードの表示用）
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
    /// Claude Code のセッションと使うモデルが決まった（続きから再開するときに使う）
    #[serde(rename_all = "camelCase")]
    Session { session_id: String, model: Option<String> },
    #[serde(rename_all = "camelCase")]
    Text { text: String },
    #[serde(rename_all = "camelCase")]
    ApprovalRequested {
        id: String,
        tool: String,
        summary: String,
        /// サーバで実際に実行するコマンド（サーバで実行しない道具は None）
        command: Option<String>,
        tab: Option<TabInfo>,
        read_user: Option<ReadUser>,
        /// Claude Code の組み込みの道具（Read 等。作業フォルダの外を読むときに確認を求められる）。
        /// 「このチャットでは許可」を出さない（フォルダの外を毎回確かめる）
        builtin: bool,
    },
    #[serde(rename_all = "camelCase")]
    ToolStarted { id: String, name: String, summary: String, command: Option<String>, session: Option<String> },
    /// 回答中の進み具合（thinking / writing / tool と、出力トークン数）
    #[serde(rename_all = "camelCase")]
    Progress { phase: String, output_tokens: u64 },
    #[serde(rename_all = "camelCase")]
    ToolFinished { id: String, output: String, is_error: bool },
    /// 質問の処理が終わった：completed / cancelled
    #[serde(rename_all = "camelCase")]
    Finished { reason: String },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Approval {
    Allow,
    Deny,
}

/// 承認の待ち合わせ（Tauri の型を含まないため、テストで直接動かせる）
#[derive(Default)]
struct Approvals {
    pending: Mutex<HashMap<String, Sender<Approval>>>,
    /// 中止された（承認待ちを拒否として終わらせる）
    cancelled: AtomicBool,
    /// 許可した Orca の道具の呼び出し（道具の名前と引数）。実行のときに 1 回だけ使う。
    /// Claude Code の設定で確認が飛ばされても、承認していない呼び出しは MCP サーバで断る
    granted: Mutex<Vec<(String, Value)>>,
}

impl Approvals {
    /// 承認待ちとして登録する（同じ ID が待っている場合は None。別の承認の回答を横取りさせない）
    fn register(&self, id: &str) -> Option<mpsc::Receiver<Approval>> {
        let mut pending = lock(&self.pending);
        if pending.contains_key(id) {
            return None;
        }
        let (sender, answer) = mpsc::channel();
        pending.insert(id.to_string(), sender);
        Some(answer)
    }

    /// 回答を待つ（中止された場合・同じ ID が待っている場合は拒否）
    #[cfg(test)]
    fn wait(&self, id: &str) -> Approval {
        match self.register(id) {
            Some(answer) => self.wait_registered(id, &answer),
            None => Approval::Deny,
        }
    }

    /// 登録した承認の回答を待つ（中止された場合は拒否）
    fn wait_registered(&self, id: &str, answer: &mpsc::Receiver<Approval>) -> Approval {
        let decision = loop {
            if self.cancelled.load(Ordering::Relaxed) {
                break Approval::Deny;
            }
            match answer.recv_timeout(APPROVAL_POLL_INTERVAL) {
                Ok(decision) => break decision,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break Approval::Deny,
            }
        };
        lock(&self.pending).remove(id);
        decision
    }

    fn answer(&self, id: &str, decision: Approval) -> Result<(), String> {
        let sender = lock(&self.pending).remove(id).ok_or_else(|| format!("no pending approval: {id}"))?;
        // 中止された場合は待つのをやめている
        let _ = sender.send(decision);
        Ok(())
    }

    /// 承認の回答を記録する（Orca の道具を許可した場合だけ、実行の許可として覚える）
    fn record(&self, tool_name: &str, input: &Value, decision: Approval) {
        let prefix = format!("mcp__{MCP_SERVER_NAME}__");
        let orca_tool = tool_name.strip_prefix(&prefix).filter(|tool| *tool != APPROVE_TOOL);
        if let (Approval::Allow, Some(tool)) = (decision, orca_tool) {
            self.grant(tool, input);
        }
    }

    /// 許可した呼び出しを覚える（実行されないまま溜まらないよう、古いものから捨てる）
    fn grant(&self, tool: &str, input: &Value) {
        let mut granted = lock(&self.granted);
        if granted.len() >= MAX_GRANTS {
            granted.remove(0);
        }
        granted.push((tool.to_string(), input.clone()));
    }

    /// 許可した呼び出しか（許可は 1 回の実行で使い切る）
    fn take_grant(&self, tool: &str, input: &Value) -> bool {
        let mut granted = lock(&self.granted);
        match granted.iter().position(|(name, args)| name == tool && args == input) {
            Some(index) => {
                granted.remove(index);
                true
            }
            None => false,
        }
    }

    fn clear_grants(&self) {
        lock(&self.granted).clear();
    }
}

/// Claude Code に返す許可の回答（`--permission-prompt-tool` の形式）
fn approval_response(decision: Approval, input: Value) -> Value {
    match decision {
        Approval::Allow => json!({ "behavior": "allow", "updatedInput": input }),
        Approval::Deny => json!({ "behavior": "deny", "message": "The user denied this tool call." }),
    }
}

/// Claude Code の出力のスレッド・MCP サーバのスレッド・Tauri のコマンドで共有するもの
#[derive(Default)]
struct Shared {
    app: OnceLock<AppHandle>,
    /// 処理中の質問のイベントの送り先
    channel: Mutex<Option<Channel<ChatEvent>>>,
    /// 最後の質問のときに開いていたタブ
    tabs: Mutex<Vec<TabInfo>>,
    approvals: Approvals,
}

impl Shared {
    fn send(&self, event: ChatEvent) {
        if let Some(channel) = lock(&self.channel).as_ref() {
            // パネルが既に閉じられている場合は届かなくてよい
            let _ = channel.send(event);
        }
    }
}

/// 回答中の進み具合（1 回の質問の出力トークン数と状態）。Tauri の型を含まないため、テストで直接動かせる
#[derive(Default)]
struct Progress {
    /// 終わった応答（API の呼び出し）の出力トークン数の合計
    finished_tokens: u64,
    /// 回答中の応答で届いた文字数（正確な値が届くまでの見積もりに使う）
    chars: usize,
    /// 回答中の応答の正確な出力トークン数（応答の終わりに届く）
    exact_tokens: Option<u64>,
    phase: Option<Phase>,
    last_sent: Option<Instant>,
}

impl Progress {
    fn output_tokens(&self) -> u64 {
        let estimate = u64::try_from(self.chars.div_ceil(CHARS_PER_TOKEN)).unwrap_or(u64::MAX);
        self.finished_tokens.saturating_add(self.exact_tokens.unwrap_or(estimate))
    }

    /// 出来事を反映し、画面へ知らせるべきなら（状態が変わった・前回から間隔が空いた）通知を返す
    fn update(&mut self, event: &CliEvent, now: Instant) -> Option<ChatEvent> {
        let previous_phase = self.phase;
        match event {
            CliEvent::MessageStart => {
                self.finished_tokens = self.output_tokens();
                self.chars = 0;
                self.exact_tokens = None;
                self.phase = Some(Phase::Thinking);
            }
            CliEvent::Delta { phase, chars } => {
                self.chars += chars;
                self.phase = Some(*phase);
            }
            CliEvent::MessageUsage { output_tokens } => self.exact_tokens = Some(*output_tokens),
            CliEvent::ToolUse { .. } => self.phase = Some(Phase::Tool),
            _ => return None,
        }
        let interval_passed = self.last_sent.is_none_or(|sent| now.duration_since(sent) >= PROGRESS_INTERVAL);
        if previous_phase == self.phase && !interval_passed {
            return None;
        }
        self.last_sent = Some(now);
        let phase = match self.phase.unwrap_or(Phase::Thinking) {
            Phase::Thinking => "thinking",
            Phase::Writing => "writing",
            Phase::Tool => "tool",
        };
        Some(ChatEvent::Progress { phase: phase.to_string(), output_tokens: self.output_tokens() })
    }
}

/// チャット（ウィンドウに 1 つ）
#[derive(Default)]
struct Chat {
    process: Option<ClaudeProcess>,
    mcp: Option<McpServer>,
    session_id: Option<String>,
    cwd: Option<PathBuf>,
    /// 選んだモデル（None なら Claude Code の設定のまま）。変えたら起動し直して同じセッションを続ける
    model: Option<String>,
    is_running: bool,
    /// Claude Code を起動した回数（止めたプロセスからの遅れた通知を無視するため）
    generation: u64,
    progress: Progress,
}

#[derive(Default)]
pub struct AiState {
    chat: Arc<Mutex<Chat>>,
    shared: Arc<Shared>,
}

/// 毒された Mutex（他のスレッドが処理中に異常終了した）でも中身を使い続ける
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 質問する（結果は `on_event` へイベントで届く）。`cwd` は Claude Code の作業フォルダ、
/// `model` は使うモデル（None なら Claude Code の設定のまま）
#[tauri::command]
pub fn ai_send(
    app: AppHandle,
    state: State<'_, AiState>,
    text: String,
    tabs: Vec<TabInfo>,
    cwd: String,
    model: Option<String>,
    on_event: Channel<ChatEvent>,
) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("the question is empty".to_string());
    }
    let cwd = working_folder(&cwd)?;
    let model = model.map(|model| model.trim().to_string()).filter(|model| !model.is_empty());
    if let Some(model) = model.as_deref().filter(|model| !claude_cli::is_valid_model(model)) {
        return Err(format!("invalid model name: {model}"));
    }
    let _ = state.shared.app.set(app);
    let mut chat = lock(&state.chat);
    if chat.is_running {
        return Err("Claude Code is still answering the previous question".to_string());
    }
    *lock(&state.shared.channel) = Some(on_event);
    *lock(&state.shared.tabs) = tabs;
    state.shared.approvals.cancelled.store(false, Ordering::Relaxed);
    // セッションは作業フォルダごとに分かれるため、フォルダを変えたら新しい会話にする
    if chat.cwd.as_ref() != Some(&cwd) {
        chat.process = None;
        chat.session_id = None;
        chat.cwd = Some(cwd);
    }
    // モデルを変えたら起動し直す（同じセッションを --resume で続ける）
    if chat.model != model {
        chat.process = None;
        chat.model = model;
    }
    if chat.process.is_none() {
        start_process(&mut chat, &state)?;
    }
    let sent = chat.process.as_mut().map(|process| process.send(&text));
    if let Some(Err(_)) = sent {
        // 前のプロセスが終了していた場合は、起動し直して同じセッションを続ける
        chat.process = None;
        start_process(&mut chat, &state)?;
        chat.process.as_mut().ok_or("Claude Code is not running")?.send(&text)?;
    }
    chat.is_running = true;
    chat.progress = Progress::default();
    Ok(())
}

fn working_folder(cwd: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(cwd.trim());
    if path.as_os_str().is_empty() || !path.is_dir() {
        return Err(format!("the working folder does not exist: {cwd}"));
    }
    Ok(path)
}

/// Claude Code を起動する（MCP サーバは初回だけ立てて使い回す）
fn start_process(chat: &mut Chat, state: &AiState) -> Result<(), String> {
    if chat.mcp.is_none() {
        chat.mcp = Some(McpServer::start(Arc::new(OrcaTools { shared: state.shared.clone() }))?);
    }
    let mcp_config = chat.mcp.as_ref().map(|mcp| mcp.config_json(MCP_SERVER_NAME)).ok_or("the MCP server is not running")?;
    let cwd = chat.cwd.clone().ok_or("the working folder is not set")?;
    // 作業フォルダの CLAUDE.md は、起動のたびに読み直す（書き換えが次の起動から反映されるように）
    let project_instructions = project_instructions::read(&cwd);
    let options = LaunchOptions {
        cwd,
        mcp_config,
        resume: chat.session_id.clone(),
        project_instructions,
        model: chat.model.clone(),
    };
    chat.generation += 1;
    let generation = chat.generation;
    let (event_chat, event_shared) = (state.chat.clone(), state.shared.clone());
    let (exit_chat, exit_shared) = (state.chat.clone(), state.shared.clone());
    chat.process = Some(ClaudeProcess::spawn(
        &claude_cli::find_claude()?,
        &options,
        move |event| handle_cli_event(&event_chat, &event_shared, generation, event),
        move |stderr| handle_exit(&exit_chat, &exit_shared, generation, stderr),
    )?);
    Ok(())
}

fn handle_cli_event(chat: &Mutex<Chat>, shared: &Shared, generation: u64, event: CliEvent) {
    let mut chat = lock(chat);
    if chat.generation != generation {
        return;
    }
    let progress = chat.progress.update(&event, Instant::now());
    let event = match event {
        CliEvent::Init { session_id, model } => {
            chat.session_id = Some(session_id.clone());
            Some(ChatEvent::Session { session_id, model })
        }
        CliEvent::Text { text } => Some(ChatEvent::Text { text }),
        CliEvent::ToolUse { id, name, input } => {
            let display = describe_tool(&name, &input);
            Some(ChatEvent::ToolStarted {
                id,
                name: display.name,
                summary: display.summary,
                command: display.command,
                session: display.session,
            })
        }
        CliEvent::ToolResult { id, output, is_error } => Some(ChatEvent::ToolFinished { id, output, is_error }),
        CliEvent::MessageStart | CliEvent::Delta { .. } | CliEvent::MessageUsage { .. } => None,
        CliEvent::Result { session_id, error } => {
            chat.is_running = false;
            if !session_id.is_empty() {
                chat.session_id = Some(session_id);
            }
            Some(match error {
                Some(message) => ChatEvent::Error { message },
                None => ChatEvent::Finished { reason: "completed".to_string() },
            })
        }
    };
    drop(chat);
    for event in progress.into_iter().chain(event) {
        shared.send(event);
    }
}

/// Claude Code が終了した（回答の途中で終わった場合はエラーを知らせる）
fn handle_exit(chat: &Mutex<Chat>, shared: &Shared, generation: u64, stderr: Option<String>) {
    let mut chat = lock(chat);
    if chat.generation != generation {
        return;
    }
    chat.process = None;
    if !chat.is_running {
        return;
    }
    chat.is_running = false;
    drop(chat);
    let detail = stderr.map(|text| format!(": {text}")).unwrap_or_default();
    shared.send(ChatEvent::Error { message: format!("Claude Code exited unexpectedly{detail}") });
}

/// 承認の要求に回答する
#[tauri::command]
pub fn ai_approve(state: State<'_, AiState>, id: String, allow: bool) -> Result<(), String> {
    state.shared.approvals.answer(&id, if allow { Approval::Allow } else { Approval::Deny })
}

/// 処理中の質問を中止する（Claude Code を終了させ、次の質問で同じセッションを続ける）
#[tauri::command]
pub fn ai_cancel(state: State<'_, AiState>) -> Result<(), String> {
    state.shared.approvals.cancelled.store(true, Ordering::Relaxed);
    state.shared.approvals.clear_grants();
    let mut chat = lock(&state.chat);
    chat.process = None;
    let was_running = std::mem::replace(&mut chat.is_running, false);
    drop(chat);
    if was_running {
        state.shared.send(ChatEvent::Finished { reason: "cancelled".to_string() });
    }
    Ok(())
}

/// 新しい会話を始める
#[tauri::command]
pub fn ai_reset(state: State<'_, AiState>) -> Result<(), String> {
    let mut chat = lock(&state.chat);
    if chat.is_running {
        return Err("stop the current answer before starting a new chat".to_string());
    }
    chat.process = None;
    chat.session_id = None;
    state.shared.approvals.clear_grants();
    Ok(())
}

/// 作業フォルダのセッション（新しい順）
#[tauri::command]
pub fn ai_list_sessions(cwd: String) -> Result<Vec<SessionSummary>, String> {
    Ok(sessions::list_sessions(&working_folder(&cwd)?))
}

/// 作業フォルダで Claude Code が使う設定（settings.json）から読んだ、モデルの選択肢
#[tauri::command]
pub fn ai_model_options(cwd: String) -> Result<ModelOptions, String> {
    Ok(model_settings::load_model_options(&working_folder(&cwd)?))
}

/// 再開したセッションの表示用の履歴の 1 件（フロントエンドの ChatItem と同じ形）
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HistoryItem {
    #[serde(rename_all = "camelCase")]
    User { text: String },
    #[serde(rename_all = "camelCase")]
    Assistant { text: String },
    #[serde(rename_all = "camelCase")]
    Tool { id: String, name: String, summary: String, command: Option<String>, output: Option<String>, is_error: bool },
}

/// 道具の呼び出しと結果を組にして、表示用の履歴にする
fn history_items(entries: Vec<HistoryEntry>) -> Vec<HistoryItem> {
    let mut items: Vec<HistoryItem> = Vec::new();
    for entry in entries {
        match entry {
            HistoryEntry::User { text } => items.push(HistoryItem::User { text }),
            HistoryEntry::Assistant { text } => items.push(HistoryItem::Assistant { text }),
            HistoryEntry::ToolUse { id, name, input } => {
                let display = describe_tool(&name, &input);
                items.push(HistoryItem::Tool {
                    id,
                    name: display.name,
                    summary: display.summary,
                    command: display.command,
                    output: None,
                    is_error: false,
                });
            }
            HistoryEntry::ToolResult { id: result_id, output: result, is_error: failed } => {
                let tool = items.iter_mut().rev().find(|item| matches!(item, HistoryItem::Tool { id, .. } if *id == result_id));
                if let Some(HistoryItem::Tool { output, is_error, .. }) = tool {
                    *output = Some(result);
                    *is_error = failed;
                }
            }
        }
    }
    items
}

/// セッションの会話の履歴（再開したときにチャット欄へ表示する）
#[tauri::command]
pub fn ai_load_history(session_id: String, cwd: String) -> Result<Vec<HistoryItem>, String> {
    Ok(history_items(sessions::load_history(&working_folder(&cwd)?, &session_id)?))
}

/// 指定したセッションの続きから再開する（次の質問から）
#[tauri::command]
pub fn ai_resume(state: State<'_, AiState>, session_id: String, cwd: String) -> Result<(), String> {
    // Claude Code の起動の引数に渡すため、セッション ID の形（UUID）を確認する（`-` で始まる値をオプションとみなさせない）
    if !sessions::is_session_id(&session_id) {
        return Err(format!("invalid session id: {session_id}"));
    }
    let cwd = working_folder(&cwd)?;
    let mut chat = lock(&state.chat);
    if chat.is_running {
        return Err("stop the current answer before switching sessions".to_string());
    }
    chat.process = None;
    chat.session_id = Some(session_id);
    chat.cwd = Some(cwd);
    Ok(())
}

/// 承認カード・道具の表示に出す内容
#[derive(Debug, PartialEq)]
struct ToolDisplay {
    /// 表示名（Orca の道具は `mcp__orca__` を外した名前）
    name: String,
    /// 引数の要約
    summary: String,
    /// サーバで実際に実行するコマンド（サーバで実行しない道具は None）
    command: Option<String>,
    /// 対象のタブ
    session: Option<String>,
}

/// 道具の表示（Orca の道具は実際に実行するコマンドを、組み込みの道具は主な引数を見せる）。
/// 見えない文字・文字の向きを変える文字は `<U+202E>` の形で見せ、承認カードの見た目を偽装させない
fn describe_tool(name: &str, input: &Value) -> ToolDisplay {
    let display = describe_tool_raw(name, input);
    ToolDisplay {
        name: tools::reveal_hidden(&display.name),
        summary: tools::reveal_hidden(&display.summary),
        command: display.command.as_deref().map(tools::reveal_hidden),
        session: display.session,
    }
}

fn describe_tool_raw(name: &str, input: &Value) -> ToolDisplay {
    let prefix = format!("mcp__{MCP_SERVER_NAME}__");
    if let Some(orca_tool) = name.strip_prefix(&prefix) {
        return match ToolCall::parse(orca_tool, input) {
            Ok(call) => ToolDisplay {
                name: orca_tool.to_string(),
                summary: call.summary(),
                command: call.remote_command(),
                session: call.session().map(str::to_string),
            },
            Err(_) => ToolDisplay { name: orca_tool.to_string(), summary: compact(input), command: None, session: None },
        };
    }
    let key = match name {
        "Read" => "file_path",
        "Grep" | "Glob" => "pattern",
        "WebFetch" => "url",
        "WebSearch" => "query",
        _ => "",
    };
    let Some(main) = input[key].as_str() else {
        return ToolDisplay { name: name.to_string(), summary: compact(input), command: None, session: None };
    };
    // 主な引数以外（Grep・Glob の検索する場所 `path` 等）も省略せずに見せる（承認の範囲を偽らせない）
    let others: Vec<String> = input
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(field, _)| field.as_str() != key)
        .map(|(field, value)| format!("{field}: {}", value.as_str().map_or_else(|| value.to_string(), str::to_string)))
        .collect();
    let summary = if others.is_empty() { main.to_string() } else { format!("{main} ({})", others.join(", ")) };
    ToolDisplay { name: name.to_string(), summary, command: None, session: None }
}

/// 組み込みの道具が読もうとしているネットワーク上のパス（`\\server\share` 等）。
/// 開こうとするだけで Windows がその PC に接続し、ログインの情報（NetNTLM のハッシュ）を送ってしまうため断る
fn network_path<'a>(tool_name: &str, input: &'a Value) -> Option<&'a str> {
    let fields: &[&str] = match tool_name {
        "Read" => &["file_path"],
        "Grep" => &["path"],
        "Glob" => &["path", "pattern"],
        _ => &[],
    };
    fields
        .iter()
        .filter_map(|field| input[*field].as_str())
        .find(|path| path.starts_with("\\\\") || path.starts_with("//") || path.starts_with("\\/") || path.starts_with("/\\"))
}

fn compact(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() <= MAX_SUMMARY_CHARS {
        return text;
    }
    format!("{}…", text.chars().take(MAX_SUMMARY_CHARS).collect::<String>())
}

/// Orca の MCP サーバが提供する道具（サーバの読み取り・承認）
struct OrcaTools {
    shared: Arc<Shared>,
}

/// MCP で提供する道具（サーバの読み取りの道具と、承認の道具）
fn orca_tool_definitions() -> Vec<Value> {
    let mut tools = tools::mcp_definitions();
    tools.push(json!({
        "name": APPROVE_TOOL,
        "description": "Used by Orca to ask the user for permission. Do not call this tool yourself.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" }
            },
            "required": ["tool_name", "input"]
        }
    }));
    tools
}

impl ToolHandler for OrcaTools {
    fn tools(&self) -> Vec<Value> {
        orca_tool_definitions()
    }

    fn call(&self, name: &str, arguments: &Value) -> ToolOutcome {
        if name == APPROVE_TOOL {
            return self.approve(arguments);
        }
        // 承認カードで許可した呼び出しだけを実行する（引数が許可したときと違うものも断る）
        if !self.shared.approvals.take_grant(name, arguments) {
            return ToolOutcome {
                text: "This call was not approved by the user in Orca, so it was not run.".to_string(),
                is_error: true,
            };
        }
        let Some(app) = self.shared.app.get() else {
            return ToolOutcome { text: "Orca is not ready".to_string(), is_error: true };
        };
        let tabs = lock(&self.shared.tabs).clone();
        let session_tools = SessionTools { app, tabs: &tabs };
        let result = ToolCall::parse(name, arguments).and_then(|call| session_tools.execute(&call));
        match result {
            Ok(text) if text.is_empty() => ToolOutcome { text: "(no output)".to_string(), is_error: false },
            Ok(text) => ToolOutcome { text, is_error: false },
            Err(message) => ToolOutcome { text: message, is_error: true },
        }
    }
}

impl OrcaTools {
    /// Claude Code の許可の求め（`--permission-prompt-tool`）。承認カードを出して回答を待つ
    fn approve(&self, arguments: &Value) -> ToolOutcome {
        let tool_name = arguments["tool_name"].as_str().unwrap_or_default();
        let input = arguments.get("input").cloned().unwrap_or_else(|| json!({}));
        let id = arguments["tool_use_id"].as_str().map_or_else(|| format!("approval-{}", random_id()), str::to_string);
        if let Some(path) = network_path(tool_name, &input) {
            let message = format!("Orca does not allow reading network paths: {}", tools::reveal_hidden(path));
            return ToolOutcome { text: json!({ "behavior": "deny", "message": message }).to_string(), is_error: false };
        }
        let Some(answer) = self.shared.approvals.register(&id) else {
            let message = "Another approval with the same id is already waiting.";
            return ToolOutcome { text: json!({ "behavior": "deny", "message": message }).to_string(), is_error: false };
        };
        let display = describe_tool(tool_name, &input);
        let tabs = lock(&self.shared.tabs).clone();
        let session = display.session.as_deref();
        let tab = session.and_then(|session| tabs.iter().find(|tab| tab.session_id == session)).cloned();
        let read_user = match (self.shared.app.get(), session) {
            (Some(app), Some(session)) => SessionTools { app, tabs: &tabs }.read_user(session),
            _ => None,
        };
        self.shared.send(ChatEvent::ApprovalRequested {
            id: id.clone(),
            tool: display.name,
            summary: display.summary,
            command: display.command,
            tab,
            read_user,
            builtin: !tool_name.starts_with(&format!("mcp__{MCP_SERVER_NAME}__")),
        });
        let decision = self.shared.approvals.wait_registered(&id, &answer);
        self.shared.approvals.record(tool_name, &input, decision);
        ToolOutcome { text: approval_response(decision, input).to_string(), is_error: false }
    }
}

fn random_id() -> u64 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one(std::time::SystemTime::now())
}

/// サーバの読み取りの道具をセッションスレッド経由で実行する
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

/// 読み取りの道具の結果として AI に渡すテキスト
fn format_run_output(text: &str, status: i32, truncated: bool) -> String {
    let mut result = text.to_string();
    if truncated {
        result.push_str(&format!("\n\n[Output truncated at {MAX_OUTPUT_BYTES} bytes. Read a smaller range.]"));
    }
    if status != 0 {
        result.push_str(&format!("\n(exit status {status})"));
    }
    result
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
    fn describes_orca_tools_with_the_actual_command() {
        let display = describe_tool("mcp__orca__tail_file", &json!({ "session": "t1", "path": "/var/log/messages" }));
        assert_eq!(
            display,
            ToolDisplay {
                name: "tail_file".into(),
                summary: "/var/log/messages (last 100 lines)".into(),
                command: Some(format!(
                    "if [ -f '/var/log/messages' ]; then tail -n 100 -- '/var/log/messages' | head -c {}; \
                     else echo 'orca: not a regular file' >&2; fi",
                    MAX_OUTPUT_BYTES + 1
                )),
                session: Some("t1".into()),
            }
        );
        // サーバでコマンドを実行しない道具はコマンドなし
        let terminal = describe_tool("mcp__orca__read_terminal", &json!({ "session": "t1" }));
        assert_eq!((terminal.command, terminal.session.as_deref()), (None, Some("t1")));
    }

    #[test]
    fn describes_builtin_tools_by_their_main_argument() {
        let read = describe_tool("Read", &json!({ "file_path": "C:/work/README.md" }));
        assert_eq!((read.name.as_str(), read.summary.as_str(), read.command), ("Read", "C:/work/README.md", None));
        assert_eq!(describe_tool("WebSearch", &json!({ "query": "ORA-12541" })).summary, "ORA-12541");
        let unknown = describe_tool("Unknown", &json!({ "a": "x".repeat(400) })).summary;
        assert!(unknown.ends_with('…') && unknown.chars().count() == MAX_SUMMARY_CHARS + 1);
    }

    #[test]
    fn builtin_tool_cards_show_every_argument() {
        // Grep・Glob の検索する場所（path）も見せる（検索語だけ見せて作業フォルダの外を読ませない）
        let grep = describe_tool("Grep", &json!({ "pattern": "BEGIN", "path": r"C:\Users\me\.ssh", "output_mode": "content" }));
        assert_eq!(grep.summary, r"BEGIN (output_mode: content, path: C:\Users\me\.ssh)");
        let read = describe_tool("Read", &json!({ "file_path": "C:/a.txt", "offset": 10, "limit": 5 }));
        assert_eq!(read.summary, "C:/a.txt (limit: 5, offset: 10)");
    }

    #[test]
    fn network_paths_are_refused_for_builtin_tools() {
        // 開こうとするだけで Windows がその PC に接続し、ログインの情報を送ってしまう
        assert_eq!(network_path("Read", &json!({ "file_path": r"\\evil\share\a.txt" })), Some(r"\\evil\share\a.txt"));
        assert!(network_path("Read", &json!({ "file_path": "//evil/share/a.txt" })).is_some());
        assert!(network_path("Grep", &json!({ "pattern": "x", "path": r"\\?\UNC\evil\share" })).is_some());
        assert!(network_path("Glob", &json!({ "pattern": r"\\evil\share\**" })).is_some());
        // 普通のパス・Grep の検索語（`//` で始まる正規表現など）は対象外
        assert_eq!(network_path("Read", &json!({ "file_path": r"C:\work\a.txt" })), None);
        assert_eq!(network_path("Grep", &json!({ "pattern": "// TODO", "path": "C:/work" })), None);
        assert_eq!(network_path("Glob", &json!({ "pattern": "**/*.rs" })), None);
    }

    #[test]
    fn a_second_approval_with_the_same_id_is_refused() {
        let approvals = Approvals::default();
        let first = approvals.register("tu-1").expect("first registration");
        // 同じ ID の承認を重ねて登録させない（先に待っている承認の回答を横取りさせない）
        assert!(approvals.register("tu-1").is_none());
        approvals.answer("tu-1", Approval::Allow).unwrap();
        assert_eq!(approvals.wait_registered("tu-1", &first), Approval::Allow);
        // 回答が済めば、同じ ID をまた使える
        assert!(approvals.register("tu-1").is_some());
    }

    #[test]
    fn approval_cards_show_hidden_characters_of_every_tool() {
        // 組み込みの道具（Orca が引数を検証しない）も、見えない文字を見える形で表示する
        let read = describe_tool("Read", &json!({ "file_path": "C:/work/\u{202E}txt.terces" }));
        assert_eq!(read.summary, "C:/work/<U+202E>txt.terces");
        let fetch = describe_tool("WebFetch", &json!({ "url": "https://example.com/\u{200B}?d=1" }));
        assert_eq!(fetch.summary, "https://example.com/<U+200B>?d=1");
        // 検証で断る Orca の道具でも、カードに出る引数は見える形にする
        let stat = describe_tool("mcp__orca__stat", &json!({ "session": "t1", "path": "/etc/\u{202E}wodahs" }));
        assert!(stat.summary.contains("<U+202E>") && !stat.summary.contains('\u{202E}'));
    }

    #[test]
    fn progress_estimates_tokens_and_uses_the_exact_count_when_it_arrives() {
        let start = Instant::now();
        let mut progress = Progress::default();
        let phase_of = |event: Option<ChatEvent>| match event {
            Some(ChatEvent::Progress { phase, output_tokens }) => Some((phase, output_tokens)),
            _ => None,
        };
        assert_eq!(phase_of(progress.update(&CliEvent::MessageStart, start)), Some(("thinking".into(), 0)));
        // 同じ状態のまま間隔が空いていなければ知らせない
        assert_eq!(phase_of(progress.update(&CliEvent::Delta { phase: Phase::Thinking, chars: 40 }, start)), None);
        // 状態が変わったら知らせる（見積もり：44 文字 → 11 トークン）
        let writing = progress.update(&CliEvent::Delta { phase: Phase::Writing, chars: 4 }, start);
        assert_eq!(phase_of(writing), Some(("writing".into(), 11)));
        // 応答の終わりに正確な値が届いたら置き換え、次の応答はその値に足していく
        progress.update(&CliEvent::MessageUsage { output_tokens: 1200 }, start);
        progress.update(&CliEvent::MessageStart, start + PROGRESS_INTERVAL);
        let next = progress.update(&CliEvent::Delta { phase: Phase::Writing, chars: 8 }, start + PROGRESS_INTERVAL * 2);
        assert_eq!(phase_of(next), Some(("writing".into(), 1202)));
        assert_eq!(phase_of(progress.update(&CliEvent::Text { text: "x".into() }, start)), None);
    }

    #[test]
    fn the_approve_tool_is_listed_with_the_read_tools() {
        let tools = orca_tool_definitions();
        let names: Vec<&str> = tools.iter().filter_map(|tool| tool["name"].as_str()).collect();
        assert!(names.contains(&"tail_file") && names.contains(&APPROVE_TOOL));
    }

    #[test]
    fn approvals_wait_for_the_answer_or_cancellation() {
        let approvals = Arc::new(Approvals::default());
        let answer = std::thread::spawn({
            let approvals = approvals.clone();
            move || loop {
                if approvals.answer("tu-1", Approval::Allow).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        assert_eq!(approvals.wait("tu-1"), Approval::Allow);
        answer.join().unwrap();
        // 回答済み・存在しない承認には答えられない
        assert!(approvals.answer("tu-1", Approval::Allow).is_err());

        // 中止されていれば、回答を待たずに拒否する
        approvals.cancelled.store(true, Ordering::Relaxed);
        assert_eq!(approvals.wait("tu-2"), Approval::Deny);
    }

    #[test]
    fn only_approved_orca_calls_can_run_and_only_once() {
        let approvals = Approvals::default();
        let input = json!({ "session": "s1", "path": "/etc/hosts" });

        // 承認を通らずに来た呼び出しは断る
        assert!(!approvals.take_grant("read_file", &input));

        // 拒否した呼び出し・組み込みの道具の許可は、実行の許可にならない
        approvals.record("mcp__orca__read_file", &input, Approval::Deny);
        approvals.record("Read", &input, Approval::Allow);
        assert!(!approvals.take_grant("read_file", &input));

        // 許可した呼び出しは、同じ道具・同じ引数で 1 回だけ実行できる
        approvals.record("mcp__orca__read_file", &input, Approval::Allow);
        assert!(!approvals.take_grant("read_file", &json!({ "session": "s1", "path": "/etc/shadow" })));
        assert!(!approvals.take_grant("tail_file", &input));
        assert!(approvals.take_grant("read_file", &input));
        assert!(!approvals.take_grant("read_file", &input));

        // 新しい会話・中止で、使われなかった許可は消える
        approvals.record("mcp__orca__read_file", &input, Approval::Allow);
        approvals.clear_grants();
        assert!(!approvals.take_grant("read_file", &input));
    }

    #[test]
    fn unused_grants_do_not_pile_up() {
        let approvals = Approvals::default();
        let first = json!({ "n": 0 });
        approvals.record("mcp__orca__stat", &first, Approval::Allow);
        for n in 1..=MAX_GRANTS {
            approvals.record("mcp__orca__stat", &json!({ "n": n }), Approval::Allow);
        }
        assert!(!approvals.take_grant("stat", &first));
        assert!(approvals.take_grant("stat", &json!({ "n": MAX_GRANTS })));
    }

    #[test]
    fn approval_responses_follow_the_permission_prompt_format() {
        let input = json!({ "file_path": "C:/outside/secret.txt" });
        assert_eq!(
            approval_response(Approval::Allow, input.clone()),
            json!({ "behavior": "allow", "updatedInput": input })
        );
        assert_eq!(approval_response(Approval::Deny, input)["behavior"], "deny");
    }

    #[test]
    fn history_pairs_tool_calls_with_their_results() {
        let entries = vec![
            HistoryEntry::User { text: "q".into() },
            HistoryEntry::ToolUse { id: "t1".into(), name: "mcp__orca__stat".into(), input: json!({ "session": "a", "path": "/etc/hosts" }) },
            HistoryEntry::ToolUse { id: "t2".into(), name: "WebSearch".into(), input: json!({ "query": "x" }) },
            HistoryEntry::ToolResult { id: "t1".into(), output: "Size: 10".into(), is_error: false },
            HistoryEntry::ToolResult { id: "unknown".into(), output: "ignored".into(), is_error: true },
            HistoryEntry::Assistant { text: "a".into() },
        ];
        let items = history_items(entries);
        assert_eq!(items.len(), 4);
        assert_eq!(
            items[1],
            HistoryItem::Tool {
                id: "t1".into(),
                name: "stat".into(),
                summary: "/etc/hosts".into(),
                command: Some("stat -- '/etc/hosts'".into()),
                output: Some("Size: 10".into()),
                is_error: false,
            }
        );
        // 結果が見つからない道具は、結果なしのまま
        assert!(matches!(&items[2], HistoryItem::Tool { name, output: None, .. } if name == "WebSearch"));
        assert_eq!(
            serde_json::to_value(&items[0]).unwrap(),
            json!({ "kind": "user", "text": "q" })
        );
    }

    #[test]
    fn serializes_events_for_the_frontend() {
        let event = ChatEvent::Session { session_id: "s1".into(), model: Some("claude-x".into()) };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({ "event": "session", "data": { "sessionId": "s1", "model": "claude-x" } })
        );
        let read_user = serde_json::to_value(ReadUser::Switched("root".into())).unwrap();
        assert_eq!(read_user, json!({ "kind": "switched", "user": "root" }));
    }
}
