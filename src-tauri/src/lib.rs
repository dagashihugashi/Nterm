mod base64;
mod codec;
mod config;
mod credentials;
mod host_key;
mod sftp;
mod ssh_auth;
mod ssh_client;

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ssh2::{Channel as SshChannel, Session};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use codec::TerminalCodec;
use config::{add_server, delete_server, load_servers, update_server};
use host_key::HostKeyPrompt;
use sftp::{RemoteEntry, SftpRequest};
use ssh_auth::Credential;
use ssh_client::{ConnectError, ConnectParams, SessionCommand, SshEvent};

type SessionMap = HashMap<String, Sender<SessionCommand>>;
/// ユーザーの確認待ちのホスト鍵（キーは known_hosts のホスト名表記）
type PendingHostKeys = HashMap<String, HostKeyPrompt>;

#[derive(Default)]
struct AppState {
    sessions: Mutex<SessionMap>,
    pending_host_keys: Mutex<PendingHostKeys>,
}

impl AppState {
    fn sessions(&self) -> Result<MutexGuard<'_, SessionMap>, String> {
        self.sessions
            .lock()
            .map_err(|_| "session registry is unavailable".to_string())
    }

    fn pending_host_keys(&self) -> Result<MutexGuard<'_, PendingHostKeys>, String> {
        self.pending_host_keys
            .lock()
            .map_err(|_| "pending host key registry is unavailable".to_string())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectRequest {
    session_id: String,
    server_id: String,
    /// パスワード認証ではパスワード、鍵認証ではパスフレーズ（空なら暗号化なしの鍵）。
    /// `None` の場合は保存済みの値を使い、未保存なら `ConnectError::CredentialRequired` を返す。
    secret: Option<String>,
    /// 認証成功時に `secret` を保存する（false の場合は保存済みの値を削除する）
    #[serde(default)]
    remember: bool,
    cols: u32,
    rows: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectOutcome {
    /// 接続は成功したが利用者に伝えるべき問題（パスワードの保存失敗など）
    warning: Option<String>,
}

/// 接続に使う資格情報。入力値か保存済みの値か
enum SecretSource {
    Entered { secret: String, remember: bool },
    Saved(String),
}

impl SecretSource {
    fn resolve(server_id: &str, secret: Option<String>, remember: bool) -> Result<Self, ConnectError> {
        if let Some(secret) = secret {
            return Ok(Self::Entered { secret, remember });
        }
        credentials::load_secret(&credentials::credentials_path()?, server_id)?
            .map(Self::Saved)
            .ok_or(ConnectError::CredentialRequired)
    }

    fn secret(&self) -> String {
        match self {
            Self::Entered { secret, .. } | Self::Saved(secret) => secret.clone(),
        }
    }

    /// 認証成功後に保存・削除を反映する。失敗しても接続は継続し、警告として返す。
    fn persist(&self, server_id: &str) -> Option<String> {
        let Self::Entered { secret, remember } = self else {
            return None;
        };
        let result = credentials::credentials_path().and_then(|path| {
            if *remember {
                credentials::save_secret(&path, server_id, secret)
            } else {
                credentials::delete_secret(&path, server_id)
            }
        });
        result.err().map(|e| format!("the password could not be saved: {e}"))
    }
}

/// servers.json の設定と入力された資格情報から、接続パラメータと文字コード変換器を用意する。
fn prepare_connection(
    server_id: &str,
    secret: String,
    cols: u32,
    rows: u32,
) -> Result<(ConnectParams, TerminalCodec), String> {
    let server = config::find_server(server_id)?;
    let credential = ssh_auth::build_credential(
        &server.auth,
        secret,
        &config::config_dir()?,
        ssh_auth::home_dir().as_deref(),
    )?;
    // 鍵ファイルの設定ミスは TCP 接続前に検出する
    if let Credential::PublicKey { key_file, .. } = &credential {
        ssh_auth::check_private_key(key_file)?;
    }
    let codec = TerminalCodec::new(server.encoding.as_deref())?;
    let params = ConnectParams {
        host: server.host,
        port: server.port,
        user: server.user,
        credential,
        cols,
        rows,
    };
    Ok((params, codec))
}

/// 接続処理（ブロッキング）を専用スレッドで実行する。
/// ホスト鍵の確認が必要な場合は、承認時に登録できるよう受け取った鍵を保持しておく。
async fn establish(
    state: &AppState,
    params: ConnectParams,
) -> Result<(Session, SshChannel), ConnectError> {
    let known_hosts = host_key::known_hosts_path()?;
    // asyncランタイムのワーカーを塞がないよう、ブロッキング処理は専用スレッドで行う
    let result =
        tauri::async_runtime::spawn_blocking(move || ssh_client::connect(&params, &known_hosts))
            .await
            .map_err(|e| format!("connection task failed: {e}"))?;
    if let Err(ConnectError::HostKey(prompt)) = &result {
        let name = host_key::host_entry_name(&prompt.host, prompt.port);
        state.pending_host_keys()?.insert(name, prompt.clone());
    }
    result
}

/// 接続先情報（host・user・文字コード等）はフロントから受け取らず、servers.json から解決する。
/// ホスト鍵が未登録・不一致の場合は `ConnectError::HostKey` を返し、承認（`trust_host_key`）後の再接続を待つ。
#[tauri::command]
async fn connect_ssh(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ConnectRequest,
    on_event: Channel<SshEvent>,
) -> Result<ConnectOutcome, ConnectError> {
    let ConnectRequest { session_id, server_id, secret, remember, cols, rows } = request;
    if state.sessions()?.contains_key(&session_id) {
        return Err(format!("session already exists: {session_id}").into());
    }
    let source = SecretSource::resolve(&server_id, secret, remember)?;
    let (params, codec) = prepare_connection(&server_id, source.secret(), cols, rows)?;
    let (session, channel) = establish(&state, params).await?;
    // 認証に成功した値だけを保存する（誤ったパスワードは保存しない）
    let warning = source.persist(&server_id);

    let (sender, commands) = mpsc::channel();
    state.sessions()?.insert(session_id.clone(), sender);

    let on_exit = move || {
        if let Ok(mut sessions) = app.state::<AppState>().sessions() {
            sessions.remove(&session_id);
        }
    };
    ssh_client::spawn_session(session, channel, codec, on_event, commands, on_exit);
    Ok(ConnectOutcome { warning })
}

/// SFTP 要求の応答待ちの上限（大きなディレクトリの一覧取得を考慮）
const SFTP_REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// セッションスレッドに SFTP 要求を送り、結果を待つ。
async fn request_sftp<T: Send + 'static>(
    state: &AppState,
    session_id: &str,
    make_request: impl FnOnce(Sender<Result<T, String>>) -> SftpRequest,
) -> Result<T, String> {
    let (reply, response) = mpsc::channel();
    send_command(state, session_id, SessionCommand::Sftp(make_request(reply)))?;
    tauri::async_runtime::spawn_blocking(move || response.recv_timeout(SFTP_REPLY_TIMEOUT))
        .await
        .map_err(|e| format!("SFTP task failed: {e}"))?
        .map_err(|_| "SFTP request timed out or the session was closed".to_string())?
}

/// ログインユーザーのホームディレクトリ（ツリーの初期表示位置）
#[tauri::command]
async fn sftp_home(state: State<'_, AppState>, session_id: String) -> Result<String, String> {
    request_sftp(&state, &session_id, |reply| SftpRequest::Home { reply }).await
}

#[tauri::command]
async fn sftp_list(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> Result<Vec<RemoteEntry>, String> {
    request_sftp(&state, &session_id, |reply| SftpRequest::List { path, reply }).await
}

#[tauri::command]
fn has_saved_credential(server_id: String) -> Result<bool, String> {
    credentials::has_secret(&credentials::credentials_path()?, &server_id)
}

#[tauri::command]
fn forget_credential(server_id: String) -> Result<(), String> {
    credentials::delete_secret(&credentials::credentials_path()?, &server_id)
}

/// ユーザーが承認したホスト鍵を known_hosts に登録する。
/// 登録できるのは直前の接続で実際にサーバから受け取った鍵のみ（フロントから鍵は受け取らない）。
#[tauri::command]
fn trust_host_key(state: State<'_, AppState>, host: String, port: u16) -> Result<(), String> {
    let name = host_key::host_entry_name(&host, port);
    let prompt = state
        .pending_host_keys()?
        .remove(&name)
        .ok_or_else(|| format!("no host key is waiting for confirmation: {name}"))?;
    host_key::trust(&host_key::known_hosts_path()?, &prompt)
}

#[tauri::command]
fn write_ssh(state: State<'_, AppState>, session_id: String, data: String) -> Result<(), String> {
    send_command(&state, &session_id, SessionCommand::Write(data))
}

#[tauri::command]
fn resize_ssh(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    send_command(&state, &session_id, SessionCommand::Resize { cols, rows })
}

/// 冪等。既に終了済みのセッションに対しても成功を返す。
#[tauri::command]
fn disconnect_ssh(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    if let Some(sender) = state.sessions()?.remove(&session_id) {
        // 送信失敗＝スレッドが既に終了済みなので問題ない
        let _ = sender.send(SessionCommand::Disconnect);
    }
    Ok(())
}

fn send_command(state: &AppState, session_id: &str, command: SessionCommand) -> Result<(), String> {
    let sessions = state.sessions()?;
    let sender = sessions
        .get(session_id)
        .ok_or_else(|| format!("session not found: {session_id}"))?;
    sender
        .send(command)
        .map_err(|_| format!("session is closed: {session_id}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            load_servers,
            add_server,
            update_server,
            delete_server,
            has_saved_credential,
            forget_credential,
            connect_ssh,
            trust_host_key,
            write_ssh,
            resize_ssh,
            sftp_home,
            sftp_list,
            disconnect_ssh
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
