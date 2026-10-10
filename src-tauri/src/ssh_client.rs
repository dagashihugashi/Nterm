use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use ssh2::{Channel, Session};
use tauri::ipc::Channel as EventChannel;

use crate::codec::TerminalCodec;
use crate::host_key::{self, HostKeyPrompt};
use crate::ssh_auth::{self, Credential};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CLOSE_TIMEOUT_MS: u32 = 3_000;
const READ_BUFFER_SIZE: usize = 16 * 1024;
const IDLE_SLEEP: Duration = Duration::from_millis(5);
const TERM_TYPE: &str = "xterm";

/// フロントエンドへ送るセッションイベント。
#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum SshEvent {
    Data(String),
    Closed { reason: Option<String> },
}

/// セッションスレッドへの指示。
pub enum SessionCommand {
    Write(String),
    Resize { cols: u32, rows: u32 },
    Disconnect,
}

/// 接続失敗の理由。フロントエンドへは `{ kind: "hostKey" | "failed", ... }` として届く。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConnectError {
    /// ホスト鍵が未登録または不一致。ユーザーの確認後に再接続する
    HostKey(HostKeyPrompt),
    /// パスワード（パスフレーズ）が保存されておらず、入力が必要
    CredentialRequired,
    /// 認証に失敗した（パスワード誤り・保存済みパスワードの失効など）。再入力を求める
    AuthFailed { message: String },
    Failed { message: String },
}

impl From<String> for ConnectError {
    fn from(message: String) -> Self {
        Self::Failed { message }
    }
}

pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub credential: Credential,
    pub cols: u32,
    pub rows: u32,
}

/// TCP接続・ハンドシェイク・ホスト鍵検証・認証・シェル起動までを行う（ブロッキング）。
/// ホスト鍵の検証は認証より前に行い、なりすましサーバへパスワード等を送らないようにする。
pub fn connect(
    params: &ConnectParams,
    known_hosts: &Path,
) -> Result<(Session, Channel), ConnectError> {
    let tcp = open_tcp(&params.host, params.port)?;
    let mut session = Session::new().map_err(|e| format!("failed to create SSH session: {e}"))?;
    session.set_tcp_stream(tcp);
    session.set_timeout(CONNECT_TIMEOUT.as_millis() as u32);
    session
        .handshake()
        .map_err(|e| format!("SSH handshake failed: {e}"))?;
    if let Some(prompt) = host_key::verify(&session, &params.host, params.port, known_hosts)? {
        return Err(ConnectError::HostKey(prompt));
    }
    ssh_auth::authenticate(&session, &params.user, &params.credential)
        .map_err(|message| ConnectError::AuthFailed { message })?;

    let mut channel = session
        .channel_session()
        .map_err(|e| format!("failed to open channel: {e}"))?;
    channel
        .request_pty(TERM_TYPE, None, Some((params.cols, params.rows, 0, 0)))
        .map_err(|e| format!("failed to request PTY: {e}"))?;
    channel
        .shell()
        .map_err(|e| format!("failed to start shell: {e}"))?;

    session.set_blocking(false);
    Ok((session, channel))
}

fn open_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("failed to resolve {host}: {e}"))?;
    let mut last_error = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_error = Some(e),
        }
    }
    Err(match last_error {
        Some(e) => format!("failed to connect to {host}:{port}: {e}"),
        None => format!("no address found for {host}"),
    })
}

/// セッションの入出力ループを別スレッドで開始する。
/// 終了時（EOF・エラー・切断指示）には `Closed` を送信し、`on_exit` を呼ぶ。
pub fn spawn_session(
    session: Session,
    channel: Channel,
    codec: TerminalCodec,
    events: EventChannel<SshEvent>,
    commands: Receiver<SessionCommand>,
    on_exit: impl FnOnce() + Send + 'static,
) {
    thread::spawn(move || {
        let reason = run_session(&session, channel, codec, &events, &commands).err();
        // フロントエンド側が既に破棄されている場合は届かなくてよい
        let _ = events.send(SshEvent::Closed { reason });
        on_exit();
    });
}

enum Flow {
    Continue,
    Stop,
}

enum ReadOutcome {
    Data(usize),
    Eof,
    Idle,
}

fn run_session(
    session: &Session,
    mut channel: Channel,
    mut codec: TerminalCodec,
    events: &EventChannel<SshEvent>,
    commands: &Receiver<SessionCommand>,
) -> Result<(), String> {
    let mut pending_input: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; READ_BUFFER_SIZE];

    let result = loop {
        match drain_commands(session, &mut channel, &codec, commands, &mut pending_input) {
            Ok(Flow::Continue) => {}
            Ok(Flow::Stop) => break Ok(()),
            Err(e) => break Err(e),
        }
        let wrote = match flush_input(&mut channel, &mut pending_input) {
            Ok(wrote) => wrote,
            Err(e) => break Err(e),
        };
        let received = match read_output(&mut channel, &mut buf) {
            Ok(ReadOutcome::Data(n)) => {
                let text = codec.decode(&buf[..n]);
                if !text.is_empty() && events.send(SshEvent::Data(text)).is_err() {
                    break Ok(()); // フロントエンド側のチャネルが閉じた
                }
                true
            }
            Ok(ReadOutcome::Eof) => break Ok(()),
            Ok(ReadOutcome::Idle) => false,
            Err(e) => break Err(e),
        };
        if !wrote && !received {
            thread::sleep(IDLE_SLEEP);
        }
    };

    close(session, &mut channel);
    result
}

fn drain_commands(
    session: &Session,
    channel: &mut Channel,
    codec: &TerminalCodec,
    commands: &Receiver<SessionCommand>,
    pending_input: &mut Vec<u8>,
) -> Result<Flow, String> {
    loop {
        match commands.try_recv() {
            Ok(SessionCommand::Write(text)) => pending_input.extend(codec.encode(&text)),
            Ok(SessionCommand::Resize { cols, rows }) => resize(session, channel, cols, rows)?,
            Ok(SessionCommand::Disconnect) | Err(TryRecvError::Disconnected) => {
                return Ok(Flow::Stop)
            }
            Err(TryRecvError::Empty) => return Ok(Flow::Continue),
        }
    }
}

/// 未送信の入力を書き込む。WouldBlock の場合は残りを保持して次のループで再送する。
fn flush_input(channel: &mut Channel, pending_input: &mut Vec<u8>) -> Result<bool, String> {
    if pending_input.is_empty() {
        return Ok(false);
    }
    match channel.write(pending_input) {
        Ok(written) => {
            pending_input.drain(..written);
            Ok(written > 0)
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(false),
        Err(e) => Err(format!("failed to send input: {e}")),
    }
}

fn read_output(channel: &mut Channel, buf: &mut [u8]) -> Result<ReadOutcome, String> {
    match channel.read(buf) {
        Ok(0) if channel.eof() => Ok(ReadOutcome::Eof),
        Ok(0) => Ok(ReadOutcome::Idle),
        Ok(n) => Ok(ReadOutcome::Data(n)),
        Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(ReadOutcome::Idle),
        Err(e) => Err(format!("connection lost: {e}")),
    }
}

fn resize(session: &Session, channel: &mut Channel, cols: u32, rows: u32) -> Result<(), String> {
    session.set_blocking(true);
    let result = channel.request_pty_size(cols, rows, None, None);
    session.set_blocking(false);
    result.map_err(|e| format!("failed to resize terminal: {e}"))
}

fn close(session: &Session, channel: &mut Channel) {
    session.set_blocking(true);
    session.set_timeout(CLOSE_TIMEOUT_MS);
    // 切断処理はベストエフォート（相手側が既に切断済みの場合は失敗して当然のため無視する）
    let _ = channel.close();
    let _ = session.disconnect(None, "session closed", None);
}
