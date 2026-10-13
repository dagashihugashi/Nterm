//! SFTP によるファイル転送（アップロード・ダウンロード）。
//!
//! 数十 GB のファイルでも UI・ターミナルを止めないよう、次のように処理する。
//! - ターミナルとは別の SSH 接続（タブごとに1本。最初の転送時に接続）を専用スレッドで使い、タブ内の転送は順に処理する
//! - 一定サイズずつ読み書きし、ファイル全体をメモリに載せない
//! - 進捗の通知は間引く
//!
//! 転送中は `<名前>.part` に書き込み、サイズを確認してから本来の名前に変更する
//! （失敗・キャンセル時は `.part` を削除し、壊れたファイルが本来の名前で残らないようにする）。

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ssh2::{OpenFlags, OpenType, Session, Sftp};
use tauri::ipc::Channel;
use tauri::State;

use crate::host_key;
use crate::sftp::join_remote;
use crate::ssh_client::{self, ConnectError, ConnectParams};

/// 1回に読み書きするサイズ（libssh2 は大きなバッファを複数のパケットに分けて先行送信する）
const CHUNK_SIZE: usize = 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
const PART_SUFFIX: &str = ".part";
const REMOTE_FILE_MODE: i32 = 0o644;
/// 転送用接続の1操作あたりのタイムアウト（相手からの応答が止まった場合に検知する）
const TRANSFER_TIMEOUT_MS: u32 = 60_000;
/// Windows のファイル名に使えない文字
const INVALID_WINDOWS_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// フロントエンドへ送る転送の進行状況
#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum TransferEvent {
    /// 転送を開始した（`total` はファイルサイズ。不明な場合は null）
    Started { total: Option<u64> },
    Progress { transferred: u64 },
    Completed { transferred: u64 },
    Failed { message: String },
    Cancelled,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TransferDirection {
    /// ローカルのファイルをリモートのディレクトリへ
    Upload { local_path: String, remote_dir: String },
    /// リモートのファイルをローカルのフォルダへ
    Download { remote_path: String, local_dir: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    transfer_id: String,
    /// 転送元・転送先のサーバ（ターミナルのセッション ID）
    session_id: String,
    direction: TransferDirection,
    /// 転送先に同名のファイルがある場合に上書きする（false なら失敗にする）
    #[serde(default)]
    overwrite: bool,
}

enum TransferError {
    Cancelled,
    Failed(String),
}

fn failed(message: impl Into<String>) -> TransferError {
    TransferError::Failed(message.into())
}

struct TransferJob {
    transfer_id: String,
    direction: TransferDirection,
    overwrite: bool,
    cancel: Arc<AtomicBool>,
    events: Channel<TransferEvent>,
}

/// 転送 ID → (セッション ID, キャンセルフラグ)。ワーカーは転送の終了時にエントリを削除する。
type CancelMap = Arc<Mutex<HashMap<String, (String, Arc<AtomicBool>)>>>;

/// 接続中のタブの接続情報と、タブごとの転送ワーカー
#[derive(Default)]
pub struct TransferRegistry {
    inner: Mutex<RegistryInner>,
    cancels: CancelMap,
}

#[derive(Default)]
struct RegistryInner {
    /// 転送用の接続に使う接続情報（入力されたパスワードを含むため、タブを閉じたら破棄する）
    connections: HashMap<String, ConnectParams>,
    workers: HashMap<String, Sender<TransferJob>>,
}

impl TransferRegistry {
    fn inner(&self) -> Result<MutexGuard<'_, RegistryInner>, String> {
        self.inner
            .lock()
            .map_err(|_| "transfer registry is unavailable".to_string())
    }

    /// ターミナルの接続に成功したタブを登録する（以後、そのタブのサーバと転送できる）
    pub fn register_session(&self, session_id: &str, params: ConnectParams) {
        if let Ok(mut inner) = self.inner() {
            inner.connections.insert(session_id.to_string(), params);
        }
    }

    /// タブの切断時に呼ぶ。接続情報を破棄し、実行中・待機中の転送をキャンセルする。
    pub fn close_session(&self, session_id: &str) {
        if let Ok(mut inner) = self.inner() {
            inner.connections.remove(session_id);
            // 送信側を破棄すると、ワーカーは残りの（キャンセル済みの）転送を処理して終了する
            inner.workers.remove(session_id);
        }
        if let Ok(cancels) = self.cancels.lock() {
            cancels
                .values()
                .filter(|(owner, _)| owner == session_id)
                .for_each(|(_, flag)| flag.store(true, Ordering::Relaxed));
        }
    }

    fn enqueue(&self, request: TransferRequest, events: Channel<TransferEvent>) -> Result<(), String> {
        let TransferRequest { transfer_id, session_id, direction, overwrite } = request;
        let mut inner = self.inner()?;
        let params = inner
            .connections
            .get(&session_id)
            .cloned()
            .ok_or("the session is not connected")?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels
            .lock()
            .map_err(|_| "transfer registry is unavailable".to_string())?
            .insert(transfer_id.clone(), (session_id.clone(), Arc::clone(&cancel)));
        let job = TransferJob { transfer_id, direction, overwrite, cancel, events };

        let sender = match inner.workers.get(&session_id) {
            Some(sender) => sender.clone(),
            None => {
                let sender = spawn_worker(params.clone(), host_key::known_hosts_path()?, Arc::clone(&self.cancels));
                inner.workers.insert(session_id.clone(), sender.clone());
                sender
            }
        };
        // ワーカーが異常終了していた場合は作り直して送り直す
        if let Err(mpsc::SendError(job)) = sender.send(job) {
            let sender = spawn_worker(params, host_key::known_hosts_path()?, Arc::clone(&self.cancels));
            inner.workers.insert(session_id, sender.clone());
            sender
                .send(job)
                .map_err(|_| "failed to start the transfer".to_string())?;
        }
        Ok(())
    }

    fn cancel(&self, transfer_id: &str) -> Result<(), String> {
        let cancels = self
            .cancels
            .lock()
            .map_err(|_| "transfer registry is unavailable".to_string())?;
        // 既に終了している場合は何もしない
        if let Some((_, flag)) = cancels.get(transfer_id) {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

fn spawn_worker(params: ConnectParams, known_hosts: PathBuf, cancels: CancelMap) -> Sender<TransferJob> {
    let (sender, jobs) = mpsc::channel();
    thread::spawn(move || {
        let worker = TransferWorker { params, known_hosts, connection: None };
        worker.run(jobs, &cancels);
    });
    sender
}

/// 転送用の SSH 接続（SFTP チャネルはセッションより先に閉じるため、この順で宣言する）
struct SftpConnection {
    sftp: Sftp,
    _session: Session,
}

fn open_connection(params: &ConnectParams, known_hosts: &Path) -> Result<SftpConnection, String> {
    let session = ssh_client::open_session(params, known_hosts).map_err(describe_connect_error)?;
    session.set_timeout(TRANSFER_TIMEOUT_MS);
    let sftp = session.sftp().map_err(|e| format!("SFTP is not available on this server: {e}"))?;
    Ok(SftpConnection { sftp, _session: session })
}

fn describe_connect_error(error: ConnectError) -> String {
    match error {
        ConnectError::HostKey(_) => {
            "the host key does not match known_hosts (reconnect the tab to check it)".to_string()
        }
        ConnectError::CredentialRequired => "a password is required".to_string(),
        ConnectError::AuthFailed { message } => format!("authentication failed: {message}"),
        ConnectError::Failed { message } => message,
    }
}

/// タブ1つ分の転送を順に処理するワーカー
struct TransferWorker {
    params: ConnectParams,
    known_hosts: PathBuf,
    connection: Option<SftpConnection>,
}

impl TransferWorker {
    fn run(mut self, jobs: Receiver<TransferJob>, cancels: &CancelMap) {
        for job in jobs {
            let event = self.execute(&job);
            // フロントエンド側が既に破棄されている場合は届かなくてよい
            let _ = job.events.send(event);
            if let Ok(mut cancels) = cancels.lock() {
                cancels.remove(&job.transfer_id);
            }
        }
    }

    fn execute(&mut self, job: &TransferJob) -> TransferEvent {
        let is_reused = self.connection.is_some();
        let mut progress = Progress::new(&job.events);
        match self.attempt(job, &mut progress) {
            Ok(transferred) => TransferEvent::Completed { transferred },
            Err(TransferError::Cancelled) => TransferEvent::Cancelled,
            Err(TransferError::Failed(message)) => {
                // 接続に問題がある可能性があるため、次の転送では接続し直す
                self.connection = None;
                let can_retry = is_reused && progress.transferred == 0 && !job.is_cancelled();
                if !can_retry {
                    return TransferEvent::Failed { message };
                }
                // 待機中に無通信で切断されていた可能性があるため、接続し直して1回だけ再試行する
                let mut retry = Progress::new(&job.events);
                match self.attempt(job, &mut retry) {
                    Ok(transferred) => TransferEvent::Completed { transferred },
                    Err(TransferError::Cancelled) => TransferEvent::Cancelled,
                    Err(TransferError::Failed(message)) => {
                        self.connection = None;
                        TransferEvent::Failed { message }
                    }
                }
            }
        }
    }

    fn attempt(&mut self, job: &TransferJob, progress: &mut Progress) -> Result<u64, TransferError> {
        if job.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        let sftp = self.sftp().map_err(TransferError::Failed)?;
        match &job.direction {
            TransferDirection::Upload { local_path, remote_dir } => {
                upload(sftp, Path::new(local_path), remote_dir, job, progress)
            }
            TransferDirection::Download { remote_path, local_dir } => {
                download(sftp, remote_path, Path::new(local_dir), job, progress)
            }
        }
    }

    fn sftp(&mut self) -> Result<&Sftp, String> {
        let connection = match self.connection.take() {
            Some(connection) => connection,
            None => open_connection(&self.params, &self.known_hosts)?,
        };
        Ok(&self.connection.insert(connection).sftp)
    }
}

impl TransferJob {
    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// 進捗の通知（間引いて送る）
struct Progress<'a> {
    events: &'a Channel<TransferEvent>,
    transferred: u64,
    last_report: Option<Instant>,
}

impl<'a> Progress<'a> {
    fn new(events: &'a Channel<TransferEvent>) -> Self {
        Self { events, transferred: 0, last_report: None }
    }

    fn start(&mut self, total: Option<u64>) {
        let _ = self.events.send(TransferEvent::Started { total });
        self.last_report = Some(Instant::now());
    }

    fn advance(&mut self, bytes: u64) {
        self.transferred += bytes;
        let now = Instant::now();
        if should_report(self.last_report, now) {
            let _ = self.events.send(TransferEvent::Progress { transferred: self.transferred });
            self.last_report = Some(now);
        }
    }
}

fn should_report(last_report: Option<Instant>, now: Instant) -> bool {
    last_report.is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL)
}

/// `src` から `dst` へ一定サイズずつコピーする。チャンクごとにキャンセルを確認する。
fn copy_chunks(
    src: &mut impl Read,
    dst: &mut impl Write,
    cancel: &AtomicBool,
    mut on_chunk: impl FnMut(u64),
) -> Result<(), TransferError> {
    let mut buffer = vec![0u8; CHUNK_SIZE];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(TransferError::Cancelled);
        }
        let read = src
            .read(&mut buffer)
            .map_err(|e| failed(format!("failed to read: {e}")))?;
        if read == 0 {
            break;
        }
        dst.write_all(&buffer[..read])
            .map_err(|e| failed(format!("failed to write: {e}")))?;
        on_chunk(read as u64);
    }
    dst.flush().map_err(|e| failed(format!("failed to write: {e}")))
}

fn remote_exists(sftp: &Sftp, path: &str) -> bool {
    sftp.stat(Path::new(path)).is_ok()
}

fn local_file_name(path: &Path) -> Result<String, TransferError> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| failed(format!("invalid file path: {}", path.display())))
}

/// リモートのパスからファイル名を取り出す（Windows に保存できない名前はエラー）
fn remote_file_name(path: &str) -> Result<String, TransferError> {
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    if name.is_empty() || name == "." || name == ".." {
        return Err(failed(format!("invalid file path: {path}")));
    }
    if name.chars().any(|c| c.is_control() || INVALID_WINDOWS_CHARS.contains(&c)) {
        return Err(failed(format!("\"{name}\" cannot be saved on Windows (invalid characters in the name)")));
    }
    Ok(name.to_string())
}

/// `C:\dir\file.iso` → `C:\dir\file.iso.part`
fn local_part_path(target: &Path) -> PathBuf {
    let mut part = OsString::from(target.as_os_str());
    part.push(PART_SUFFIX);
    PathBuf::from(part)
}

fn upload(
    sftp: &Sftp,
    local: &Path,
    remote_dir: &str,
    job: &TransferJob,
    progress: &mut Progress,
) -> Result<u64, TransferError> {
    let target = join_remote(remote_dir, &local_file_name(local)?);
    if !job.overwrite && remote_exists(sftp, &target) {
        return Err(failed(format!("{target} already exists")));
    }
    let total = fs::metadata(local)
        .map_err(|e| failed(format!("failed to read {}: {e}", local.display())))?
        .len();
    let mut source =
        File::open(local).map_err(|e| failed(format!("failed to open {}: {e}", local.display())))?;
    let part = format!("{target}{PART_SUFFIX}");
    progress.start(Some(total));
    let result = upload_to_part(sftp, &mut source, &part, total, job, progress)
        .and_then(|()| replace_remote(sftp, &part, &target, job.overwrite));
    if result.is_err() {
        // 途中までのファイルを残さない（削除できなくても転送の結果は変わらない）
        let _ = sftp.unlink(Path::new(&part));
    }
    result.map(|()| total)
}

fn upload_to_part(
    sftp: &Sftp,
    source: &mut File,
    part: &str,
    total: u64,
    job: &TransferJob,
    progress: &mut Progress,
) -> Result<(), TransferError> {
    let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE;
    let mut remote = sftp
        .open_mode(Path::new(part), flags, REMOTE_FILE_MODE, OpenType::File)
        .map_err(|e| failed(format!("failed to create {part}: {e}")))?;
    copy_chunks(source, &mut remote, &job.cancel, |bytes| progress.advance(bytes))?;
    drop(remote);
    let size = sftp
        .stat(Path::new(part))
        .map_err(|e| failed(format!("failed to check {part}: {e}")))?
        .size;
    if size != Some(total) {
        return Err(failed(format!(
            "the uploaded size does not match ({} of {total} bytes)",
            size.unwrap_or(0)
        )));
    }
    Ok(())
}

/// `.part` を本来の名前に変更する（SFTP v3 の rename は上書きできないため、上書き時は先に削除する）
fn replace_remote(sftp: &Sftp, part: &str, target: &str, overwrite: bool) -> Result<(), TransferError> {
    if overwrite && remote_exists(sftp, target) {
        sftp.unlink(Path::new(target))
            .map_err(|e| failed(format!("failed to replace {target}: {e}")))?;
    }
    sftp.rename(Path::new(part), Path::new(target), None)
        .map_err(|e| failed(format!("failed to rename {part} to {target}: {e}")))
}

fn download(
    sftp: &Sftp,
    remote: &str,
    local_dir: &Path,
    job: &TransferJob,
    progress: &mut Progress,
) -> Result<u64, TransferError> {
    let target = local_dir.join(remote_file_name(remote)?);
    if !job.overwrite && target.exists() {
        return Err(failed(format!("{} already exists", target.display())));
    }
    let stat = sftp
        .stat(Path::new(remote))
        .map_err(|e| failed(format!("failed to read {remote}: {e}")))?;
    if stat.is_dir() {
        return Err(failed("folders cannot be downloaded yet"));
    }
    let mut source = sftp
        .open(Path::new(remote))
        .map_err(|e| failed(format!("failed to open {remote}: {e}")))?;
    let part = local_part_path(&target);
    progress.start(stat.size);
    let result = download_to_part(&mut source, &part, stat.size, job, progress).and_then(|size| {
        // Windows の rename は既存のファイルを置き換える
        fs::rename(&part, &target)
            .map(|()| size)
            .map_err(|e| failed(format!("failed to rename to {}: {e}", target.display())))
    });
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn download_to_part(
    source: &mut ssh2::File,
    part: &Path,
    total: Option<u64>,
    job: &TransferJob,
    progress: &mut Progress,
) -> Result<u64, TransferError> {
    let mut local =
        File::create(part).map_err(|e| failed(format!("failed to create {}: {e}", part.display())))?;
    copy_chunks(source, &mut local, &job.cancel, |bytes| progress.advance(bytes))?;
    local
        .sync_all()
        .map_err(|e| failed(format!("failed to write {}: {e}", part.display())))?;
    drop(local);
    let size = fs::metadata(part)
        .map_err(|e| failed(format!("failed to check {}: {e}", part.display())))?
        .len();
    if total.is_some_and(|total| total != size) {
        return Err(failed(format!(
            "the downloaded size does not match ({size} of {} bytes)",
            total.unwrap_or(0)
        )));
    }
    Ok(size)
}

/// 開始前の確認（転送元が存在するか・フォルダでないか）。ワーカーに渡す前にエラーを返す。
fn validate(direction: &TransferDirection) -> Result<(), String> {
    match direction {
        TransferDirection::Upload { local_path, .. } => {
            let metadata = fs::metadata(local_path).map_err(|e| format!("{local_path}: {e}"))?;
            if !metadata.is_file() {
                return Err(format!("{local_path}: only files can be uploaded (folders are not supported yet)"));
            }
        }
        TransferDirection::Download { local_dir, .. } => {
            if !Path::new(local_dir).is_dir() {
                return Err(format!("{local_dir} is not a folder"));
            }
        }
    }
    Ok(())
}

/// 転送をキューに追加する。進行状況は `on_event` に届く（タブ内の転送は順に処理する）。
#[tauri::command]
pub fn start_transfer(
    registry: State<'_, TransferRegistry>,
    request: TransferRequest,
    on_event: Channel<TransferEvent>,
) -> Result<(), String> {
    validate(&request.direction)?;
    registry.enqueue(request, on_event)
}

/// 実行中・待機中の転送をキャンセルする。既に終了している場合は何もしない。
#[tauri::command]
pub fn cancel_transfer(registry: State<'_, TransferRegistry>, transfer_id: String) -> Result<(), String> {
    registry.cancel(&transfer_id)
}

/// ダウンロード先に同名のファイルがあるか（上書きの確認に使う）
#[tauri::command]
pub fn local_file_exists(dir: String, name: String) -> bool {
    Path::new(&dir).join(name).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn copies_all_chunks_and_reports_each() {
        let data = vec![7u8; CHUNK_SIZE * 2 + 10];
        let mut dst = Vec::new();
        let mut reported = Vec::new();
        let cancel = AtomicBool::new(false);
        let result = copy_chunks(&mut Cursor::new(&data), &mut dst, &cancel, |n| reported.push(n));
        assert!(result.is_ok());
        assert_eq!(dst, data);
        assert_eq!(reported.iter().sum::<u64>(), data.len() as u64);
        assert!(reported.len() >= 3);
    }

    #[test]
    fn stops_copying_when_cancelled() {
        let data = vec![1u8; CHUNK_SIZE * 3];
        let mut dst = Vec::new();
        let cancel = AtomicBool::new(false);
        let result = copy_chunks(&mut Cursor::new(&data), &mut dst, &cancel, |_| {
            cancel.store(true, Ordering::Relaxed)
        });
        assert!(matches!(result, Err(TransferError::Cancelled)));
        assert_eq!(dst.len(), CHUNK_SIZE);
    }

    #[test]
    fn throttles_progress_reports() {
        let start = Instant::now();
        assert!(should_report(None, start));
        assert!(!should_report(Some(start), start + Duration::from_millis(100)));
        assert!(should_report(Some(start), start + PROGRESS_INTERVAL));
    }

    #[test]
    fn extracts_remote_file_name() {
        assert_eq!(remote_file_name("/var/tmp/image.iso").ok().as_deref(), Some("image.iso"));
        assert!(remote_file_name("/var/tmp/..").is_err());
        assert!(remote_file_name("/").is_err());
    }

    #[test]
    fn rejects_remote_names_that_windows_cannot_store() {
        assert!(remote_file_name("/tmp/a:b.log").is_err());
        assert!(remote_file_name("/tmp/what?.txt").is_err());
    }

    #[test]
    fn appends_part_suffix_to_local_path() {
        let part = local_part_path(Path::new("C:/downloads/image.iso"));
        assert_eq!(part, PathBuf::from("C:/downloads/image.iso.part"));
    }

    #[test]
    fn rejects_folder_upload_and_missing_download_dir() {
        let dir = std::env::temp_dir();
        let upload = TransferDirection::Upload {
            local_path: dir.display().to_string(),
            remote_dir: "/tmp".into(),
        };
        assert!(validate(&upload).unwrap_err().contains("only files"));
        let download = TransferDirection::Download {
            remote_path: "/tmp/a".into(),
            local_dir: dir.join("nterm-no-such-dir").display().to_string(),
        };
        assert!(validate(&download).is_err());
    }

    #[test]
    fn parses_request_from_frontend() {
        let json = r#"{
            "transferId": "t1", "sessionId": "s1", "overwrite": true,
            "direction": { "kind": "upload", "localPath": "C:\\a.iso", "remoteDir": "/tmp" }
        }"#;
        let request: TransferRequest = serde_json::from_str(json).unwrap();
        assert!(request.overwrite);
        assert!(matches!(
            request.direction,
            TransferDirection::Upload { ref remote_dir, .. } if remote_dir == "/tmp"
        ));
    }
}
