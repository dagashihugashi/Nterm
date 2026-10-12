use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use ssh2::{Channel, Session};
use tauri::ipc::Channel as EventChannel;

use crate::codec::TerminalCodec;
use crate::credentials;
use crate::host_key::{self, HostKeyPrompt};
use crate::session_log::SessionLog;
use crate::sftp::{SftpClient, SftpRequest};
use crate::ssh_auth::{self, Credential};
use crate::switch_user::{Action as SwitchAction, SwitchUserAutomation, SwitchUserNotice};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 無通信時の Keepalive 送信間隔（ファイアウォール等による無通信切断を防ぐ）。
/// settings.json の `ssh.keepaliveIntervalSec` 実装までは固定値。
const KEEPALIVE_INTERVAL_SECS: u32 = 60;
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
    /// ログの書き込みに失敗し、ログ取得を停止した（接続は継続）
    LogFailed { message: String },
    /// ログイン後のユーザ切り替えの進行状況
    SwitchUser(SwitchUserNotice),
}

/// セッションスレッドへの指示。
pub enum SessionCommand {
    Write(String),
    Resize { cols: u32, rows: u32 },
    /// 同じ SSH セッション上での SFTP 操作
    Sftp(SftpRequest),
    /// 開いたログファイルを渡してログ取得を開始する
    StartLog(Box<SessionLog>),
    StopLog,
    /// 文字コードを切り替える（画面表示・入力・ログのすべてに適用）
    SetEncoding(TerminalCodec),
    /// ユーザ切り替えのパスワード入力の回答（パスワード, 保存するか）。None はキャンセル
    SwitchUserSecret(Option<(String, bool)>),
    Disconnect,
}

/// セッション開始時に渡す設定
pub struct SessionSetup {
    pub codec: TerminalCodec,
    /// パスワードの保存先（credentials.json のキー）に使う
    pub server_id: String,
    /// ログイン後のユーザ切り替え（設定がなければ None）
    pub switch_user: Option<SwitchUserAutomation>,
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

    // 応答は不要（通信を発生させることが目的。相手が応答しない場合は TCP の送信エラーで切断を検知する）
    session.set_keepalive(false, KEEPALIVE_INTERVAL_SECS);
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
    setup: SessionSetup,
    events: EventChannel<SshEvent>,
    commands: Receiver<SessionCommand>,
    on_exit: impl FnOnce() + Send + 'static,
) {
    thread::spawn(move || {
        let reason = run_session(&session, channel, setup, &events, &commands).err();
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

/// セッションスレッドが所有する状態
struct SessionLoop<'a> {
    session: &'a Session,
    channel: Channel,
    codec: TerminalCodec,
    events: &'a EventChannel<SshEvent>,
    /// 未送信の入力（WouldBlock 時に次のループで再送する）
    pending_input: Vec<u8>,
    sftp: SftpClient,
    /// 取得中のログ（未取得なら None）
    log: Option<SessionLog>,
    /// 次に Keepalive の送信を確認する時刻
    next_keepalive: Instant,
    server_id: String,
    /// 実行中のユーザ切り替え（完了・中断したら None）
    switch_user: Option<SwitchUserAutomation>,
}

fn run_session(
    session: &Session,
    channel: Channel,
    setup: SessionSetup,
    events: &EventChannel<SshEvent>,
    commands: &Receiver<SessionCommand>,
) -> Result<(), String> {
    let mut state = SessionLoop {
        session,
        channel,
        codec: setup.codec,
        events,
        pending_input: Vec::new(),
        sftp: SftpClient::default(),
        log: None,
        next_keepalive: Instant::now(),
        server_id: setup.server_id,
        switch_user: setup.switch_user,
    };
    let result = state.run(commands);
    state.shutdown();
    result
}

impl SessionLoop<'_> {
    fn run(&mut self, commands: &Receiver<SessionCommand>) -> Result<(), String> {
        let mut buf = vec![0u8; READ_BUFFER_SIZE];
        loop {
            if let Flow::Stop = self.drain_commands(commands)? {
                return Ok(());
            }
            let wrote = flush_input(&mut self.channel, &mut self.pending_input)?;
            let received = match read_output(&mut self.channel, &mut buf)? {
                ReadOutcome::Data(n) => {
                    if let Flow::Stop = self.handle_output(&buf[..n]) {
                        return Ok(());
                    }
                    true
                }
                ReadOutcome::Eof => return Ok(()),
                ReadOutcome::Idle => false,
            };
            self.send_keepalive()?;
            self.tick_switch_user();
            if !wrote && !received {
                thread::sleep(IDLE_SLEEP);
            }
        }
    }

    /// 送信時刻になっていれば Keepalive を送る。
    /// libssh2 は前回の通信からの経過時間で送信要否を判断し、次の送信までの秒数を返す。
    fn send_keepalive(&mut self) -> Result<(), String> {
        if Instant::now() < self.next_keepalive {
            return Ok(());
        }
        match self.session.keepalive_send() {
            Ok(seconds) => {
                let wait = Duration::from_secs(u64::from(seconds.max(1)));
                self.next_keepalive = Instant::now() + wait;
                Ok(())
            }
            Err(e) => {
                let error = std::io::Error::from(e);
                // 送信バッファが一杯の場合は次のループで再試行する
                if error.kind() == ErrorKind::WouldBlock {
                    Ok(())
                } else {
                    Err(format!("connection lost: {error}"))
                }
            }
        }
    }

    fn drain_commands(&mut self, commands: &Receiver<SessionCommand>) -> Result<Flow, String> {
        loop {
            match commands.try_recv() {
                Ok(SessionCommand::Write(text)) => {
                    if self.accepts_input(&text) {
                        self.pending_input.extend(self.codec.encode(&text))
                    }
                }
                Ok(SessionCommand::Resize { cols, rows }) => {
                    resize(self.session, &mut self.channel, cols, rows)?
                }
                Ok(SessionCommand::Sftp(request)) => self.sftp.handle(self.session, request),
                Ok(SessionCommand::StartLog(log)) => {
                    // 取得中に開始し直した場合は、前のログを閉じてから切り替える
                    self.stop_log();
                    self.log = Some(*log);
                }
                Ok(SessionCommand::StopLog) => self.stop_log(),
                // 旧デコーダが保持していた途中のマルチバイト文字は破棄される
                Ok(SessionCommand::SetEncoding(codec)) => self.codec = codec,
                Ok(SessionCommand::SwitchUserSecret(answer)) => self.provide_switch_secret(answer),
                Ok(SessionCommand::Disconnect) | Err(TryRecvError::Disconnected) => {
                    return Ok(Flow::Stop)
                }
                Err(TryRecvError::Empty) => return Ok(Flow::Continue),
            }
        }
    }

    /// 受信データを画面とログへ送る。フロントエンド側のチャネルが閉じていたら終了する。
    fn handle_output(&mut self, bytes: &[u8]) -> Flow {
        let text = self.codec.decode(bytes);
        if text.is_empty() {
            return Flow::Continue;
        }
        self.record(&text);
        self.feed_switch_user(&text);
        match self.events.send(SshEvent::Data(text)) {
            Ok(()) => Flow::Continue,
            Err(_) => Flow::Stop,
        }
    }

    /// ユーザ切り替えの実行中は、利用者のキー入力を送らない（自動入力と混ざらないようにする）。
    /// Ctrl+C は送ったうえで切り替えを中断、Esc は切り替えを中断する。
    fn accepts_input(&mut self, text: &str) -> bool {
        let Some(automation) = self.switch_user.as_mut() else {
            return true;
        };
        if text.contains('\x03') {
            let actions = automation.interrupt();
            self.apply_switch_actions(actions);
            return true;
        }
        if text == "\x1b" {
            let actions = automation.cancel();
            self.apply_switch_actions(actions);
        }
        false
    }

    fn feed_switch_user(&mut self, text: &str) {
        if let Some(automation) = self.switch_user.as_mut() {
            let actions = automation.feed(text, Instant::now());
            self.apply_switch_actions(actions);
        }
    }

    fn tick_switch_user(&mut self) {
        if let Some(automation) = self.switch_user.as_mut() {
            let actions = automation.tick(Instant::now());
            self.apply_switch_actions(actions);
        }
    }

    fn provide_switch_secret(&mut self, answer: Option<(String, bool)>) {
        if let Some(automation) = self.switch_user.as_mut() {
            let actions = automation.provide_secret(answer, Instant::now());
            self.apply_switch_actions(actions);
        }
    }

    fn apply_switch_actions(&mut self, actions: Vec<SwitchAction>) {
        for action in actions {
            match action {
                SwitchAction::Send(text) => self.pending_input.extend(self.codec.encode(&text)),
                SwitchAction::PersistSecret { user, secret, remember } => {
                    if let Err(message) = persist_switch_secret(&self.server_id, &user, &secret, remember) {
                        self.notify_switch(SwitchUserNotice::Warning { message });
                    }
                }
                SwitchAction::Notify(notice) => self.notify_switch(notice),
            }
        }
        if !self.switch_user.as_ref().is_some_and(SwitchUserAutomation::is_active) {
            self.switch_user = None;
        }
    }

    fn notify_switch(&self, notice: SwitchUserNotice) {
        // フロントエンド側が既に破棄されている場合は届かなくてよい
        let _ = self.events.send(SshEvent::SwitchUser(notice));
    }

    /// ログに書き込む。書き込めなくなった場合はログを止めて通知する（接続は継続する）。
    fn record(&mut self, text: &str) {
        let Some(log) = self.log.as_mut() else {
            return;
        };
        if let Err(message) = log.write(text) {
            self.log = None;
            self.notify_log_failed(message);
        }
    }

    fn stop_log(&mut self) {
        if let Some(Err(message)) = self.log.take().map(SessionLog::finish) {
            self.notify_log_failed(message);
        }
    }

    fn notify_log_failed(&self, message: String) {
        // フロントエンド側が既に破棄されている場合は届かなくてよい
        let _ = self.events.send(SshEvent::LogFailed { message });
    }

    fn shutdown(mut self) {
        self.stop_log();
        // SFTP チャネルはセッションを閉じる前に、ブロッキングモードで解放する
        self.session.set_blocking(true);
        drop(self.sftp);
        close(self.session, &mut self.channel);
    }
}

/// 切り替えに成功したパスワードを保存する（remember = false なら保存済みの値を削除する）
fn persist_switch_secret(
    server_id: &str,
    user: &str,
    secret: &str,
    remember: bool,
) -> Result<(), String> {
    let path = credentials::credentials_path()?;
    let key = credentials::switch_user_key(server_id, user);
    let result = if remember {
        credentials::save_secret(&path, &key, secret)
    } else {
        credentials::delete_secret(&path, &key)
    };
    result.map_err(|e| format!("the password for {user} could not be saved: {e}"))
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
