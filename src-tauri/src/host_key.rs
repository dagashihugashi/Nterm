use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use ssh2::{CheckResult, HashType, HostKeyType, KnownHostFileKind, KnownHosts, Session};

use crate::base64;
use crate::config::config_dir;

const KNOWN_HOSTS_FILE: &str = "known_hosts";
const DEFAULT_SSH_PORT: u16 = 22;
const ENTRY_COMMENT: &str = "added by Orca";

/// known_hosts の「読み込み→更新→書き込み」を直列化する
static KNOWN_HOSTS_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyStatus {
    Trusted,
    Unknown,
    Changed,
}

/// ユーザーに確認を求めるホスト鍵の情報。`key` はフロントへは送らず、承認時の登録に使う。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    /// true: 保存済みの鍵と不一致（なりすましの可能性） / false: 未登録
    pub changed: bool,
    #[serde(skip)]
    pub key: Vec<u8>,
    #[serde(skip)]
    pub raw_key_type: HostKeyType,
}

pub fn known_hosts_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join(KNOWN_HOSTS_FILE))
}

/// known_hosts のホスト名表記（OpenSSH 互換: ポート22以外は `[host]:port`）
pub fn host_entry_name(host: &str, port: u16) -> String {
    if port == DEFAULT_SSH_PORT {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// 接続中のサーバのホスト鍵を known_hosts と照合する。
/// 信頼済みなら `None`、ユーザーの確認が必要なら `Some(prompt)` を返す。
pub fn verify(
    session: &Session,
    host: &str,
    port: u16,
    known_hosts: &Path,
) -> Result<Option<HostKeyPrompt>, String> {
    let (key, raw_key_type) = session
        .host_key()
        .ok_or("server did not provide a host key")?;
    let status = check(known_hosts, host, port, key)?;
    if status == HostKeyStatus::Trusted {
        return Ok(None);
    }
    let hash = session
        .host_key_hash(HashType::Sha256)
        .ok_or("failed to compute host key fingerprint")?;
    Ok(Some(HostKeyPrompt {
        host: host.to_string(),
        port,
        key_type: key_type_name(raw_key_type).to_string(),
        fingerprint: fingerprint_sha256(hash),
        changed: status == HostKeyStatus::Changed,
        key: key.to_vec(),
        raw_key_type,
    }))
}

pub fn check(path: &Path, host: &str, port: u16, key: &[u8]) -> Result<HostKeyStatus, String> {
    let session = Session::new().map_err(|e| format!("failed to initialize libssh2: {e}"))?;
    let known_hosts = load(&session, path)?;
    match known_hosts.check_port(host, port, key) {
        CheckResult::Match => Ok(HostKeyStatus::Trusted),
        CheckResult::NotFound => Ok(HostKeyStatus::Unknown),
        CheckResult::Mismatch => Ok(HostKeyStatus::Changed),
        CheckResult::Failure => Err(format!("failed to check {}", path.display())),
    }
}

/// ユーザーが承認したホスト鍵を登録する。同じホスト名の既存エントリは置き換える。
pub fn trust(path: &Path, prompt: &HostKeyPrompt) -> Result<(), String> {
    let _guard = KNOWN_HOSTS_WRITE_LOCK
        .lock()
        .map_err(|_| "known_hosts lock is unavailable".to_string())?;
    let session = Session::new().map_err(|e| format!("failed to initialize libssh2: {e}"))?;
    let mut known_hosts = load(&session, path)?;
    let name = host_entry_name(&prompt.host, prompt.port);

    // ハッシュ化されたホスト名（|1|...）は名前で照合できないため対象外（本アプリは平文で書き込む）
    let entries = known_hosts
        .iter()
        .map_err(|e| format!("failed to list known_hosts entries: {e}"))?;
    for entry in entries.iter().filter(|entry| entry.name() == Some(name.as_str())) {
        known_hosts
            .remove(entry)
            .map_err(|e| format!("failed to remove old host key: {e}"))?;
    }
    known_hosts
        .add(&name, &prompt.key, ENTRY_COMMENT, prompt.raw_key_type.into())
        .map_err(|e| format!("failed to add host key: {e}"))?;
    save(&known_hosts, path)
}

fn load(session: &Session, path: &Path) -> Result<KnownHosts, String> {
    let mut known_hosts = session
        .known_hosts()
        .map_err(|e| format!("failed to initialize known_hosts: {e}"))?;
    if path.exists() {
        known_hosts
            .read_file(path, KnownHostFileKind::OpenSSH)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    }
    Ok(known_hosts)
}

/// 一時ファイルに書いてから置き換えることで、書き込み途中の破損を防ぐ。
fn save(known_hosts: &KnownHosts, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    known_hosts
        .write_file(&tmp, KnownHostFileKind::OpenSSH)
        .map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("failed to replace {}: {e}", path.display()))
}

fn key_type_name(key_type: HostKeyType) -> &'static str {
    match key_type {
        HostKeyType::Rsa => "ssh-rsa",
        HostKeyType::Dss => "ssh-dss",
        HostKeyType::Ecdsa256 => "ecdsa-sha2-nistp256",
        HostKeyType::Ecdsa384 => "ecdsa-sha2-nistp384",
        HostKeyType::Ecdsa521 => "ecdsa-sha2-nistp521",
        HostKeyType::Ed25519 => "ssh-ed25519",
        HostKeyType::Unknown => "unknown",
    }
}

/// OpenSSH と同じ表記（`SHA256:` + パディングなし Base64）
fn fingerprint_sha256(hash: &[u8]) -> String {
    format!("SHA256:{}", base64::encode_no_pad(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_known_hosts(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nterm-kh-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join(KNOWN_HOSTS_FILE)
    }

    fn prompt(host: &str, port: u16, key: &[u8]) -> HostKeyPrompt {
        HostKeyPrompt {
            host: host.to_string(),
            port,
            key_type: "ssh-ed25519".to_string(),
            fingerprint: String::new(),
            changed: false,
            key: key.to_vec(),
            raw_key_type: HostKeyType::Ed25519,
        }
    }

    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn formats_fingerprint_like_openssh() {
        assert_eq!(fingerprint_sha256(b"foo"), "SHA256:Zm9v");
    }

    #[test]
    fn uses_bracket_notation_for_non_default_port() {
        assert_eq!(host_entry_name("192.0.2.1", 22), "192.0.2.1");
        assert_eq!(host_entry_name("192.0.2.1", 2222), "[192.0.2.1]:2222");
    }

    #[test]
    fn unknown_when_known_hosts_does_not_exist() {
        let path = temp_known_hosts("missing");
        assert_eq!(check(&path, "192.0.2.1", 22, b"key-a").unwrap(), HostKeyStatus::Unknown);
    }

    #[test]
    fn trusted_after_registration() {
        let path = temp_known_hosts("trusted");
        trust(&path, &prompt("192.0.2.1", 22, b"key-a")).unwrap();
        let status = check(&path, "192.0.2.1", 22, b"key-a");
        let other_host = check(&path, "192.0.2.2", 22, b"key-a");
        cleanup(&path);
        assert_eq!(status.unwrap(), HostKeyStatus::Trusted);
        assert_eq!(other_host.unwrap(), HostKeyStatus::Unknown);
    }

    #[test]
    fn detects_changed_key() {
        let path = temp_known_hosts("changed");
        trust(&path, &prompt("192.0.2.1", 22, b"key-a")).unwrap();
        let status = check(&path, "192.0.2.1", 22, b"key-b");
        cleanup(&path);
        assert_eq!(status.unwrap(), HostKeyStatus::Changed);
    }

    #[test]
    fn replacing_key_trusts_only_the_new_one() {
        let path = temp_known_hosts("replace");
        trust(&path, &prompt("192.0.2.1", 22, b"key-a")).unwrap();
        trust(&path, &prompt("192.0.2.1", 22, b"key-b")).unwrap();
        let new_key = check(&path, "192.0.2.1", 22, b"key-b");
        let old_key = check(&path, "192.0.2.1", 22, b"key-a");
        cleanup(&path);
        assert_eq!(new_key.unwrap(), HostKeyStatus::Trusted);
        assert_eq!(old_key.unwrap(), HostKeyStatus::Changed);
    }

    #[test]
    fn distinguishes_ports() {
        let path = temp_known_hosts("ports");
        trust(&path, &prompt("192.0.2.1", 2222, b"key-a")).unwrap();
        let same_port = check(&path, "192.0.2.1", 2222, b"key-a");
        let default_port = check(&path, "192.0.2.1", 22, b"key-a");
        let content = fs::read_to_string(&path).unwrap_or_default();
        cleanup(&path);
        assert_eq!(same_port.unwrap(), HostKeyStatus::Trusted);
        assert_eq!(default_port.unwrap(), HostKeyStatus::Unknown);
        assert!(content.starts_with("[192.0.2.1]:2222 ssh-ed25519 "));
    }
}
