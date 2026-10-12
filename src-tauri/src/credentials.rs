//! 保存パスワード（credentials.json）。
//! 値は Windows DPAPI で暗号化して保存するため、同じPC・同じWindowsユーザーでのみ復号できる。

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::base64;
use crate::config::config_dir;

const CREDENTIALS_FILE: &str = "credentials.json";
/// DPAPI の追加エントロピー（同じユーザーの他アプリが DPAPI で復号しても意味をなさないようにする）
const DPAPI_ENTROPY: &[u8] = b"MyTerminal/credentials/v1";

/// credentials.json の「読み込み→更新→書き込み」を直列化する
static CREDENTIALS_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCredential {
    /// DPAPI で暗号化したバイト列の Base64
    secret: String,
}

/// キーは servers.json のサーバ `id`（ログイン用）または `switch_user_key` の値（ユーザ切り替え用）
type CredentialFile = BTreeMap<String, StoredCredential>;

/// ユーザ切り替え（su 等）のパスワードのキー
pub fn switch_user_key(server_id: &str, user: &str) -> String {
    format!("{server_id}#su:{user}")
}

/// サーバに属するキー（ログイン用・ユーザ切り替え用）か
fn belongs_to_server(key: &str, server_id: &str) -> bool {
    key == server_id || key.starts_with(&format!("{server_id}#"))
}

pub fn credentials_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join(CREDENTIALS_FILE))
}

/// 保存済みのパスワード／パスフレーズを復号して返す。未保存なら `None`。
pub fn load_secret(path: &Path, server_id: &str) -> Result<Option<String>, String> {
    let Some(stored) = read_file(path)?.remove(server_id) else {
        return Ok(None);
    };
    let encrypted = base64::decode(&stored.secret)
        .map_err(|e| format!("saved password for {server_id} is corrupted: {e}"))?;
    let plain = dpapi::unprotect(&encrypted, DPAPI_ENTROPY).map_err(|e| {
        format!("saved password for {server_id} cannot be decrypted (saved by another Windows user or PC?): {e}")
    })?;
    String::from_utf8(plain)
        .map(Some)
        .map_err(|_| format!("saved password for {server_id} is corrupted"))
}

pub fn has_secret(path: &Path, server_id: &str) -> Result<bool, String> {
    Ok(read_file(path)?.contains_key(server_id))
}

pub fn save_secret(path: &Path, server_id: &str, secret: &str) -> Result<(), String> {
    let encrypted = dpapi::protect(secret.as_bytes(), DPAPI_ENTROPY)?;
    let stored = StoredCredential {
        secret: base64::encode(&encrypted),
    };
    update_file(path, |credentials| {
        credentials.insert(server_id.to_string(), stored);
    })
}

/// 保存済みのパスワードを削除する。未保存でも成功とする。
pub fn delete_secret(path: &Path, server_id: &str) -> Result<(), String> {
    if !has_secret(path, server_id)? {
        return Ok(());
    }
    update_file(path, |credentials| {
        credentials.remove(server_id);
    })
}

/// サーバの保存済みパスワード（ログイン用・ユーザ切り替え用）をすべて削除する
pub fn delete_server_secrets(path: &Path, server_id: &str) -> Result<(), String> {
    if !read_file(path)?.keys().any(|key| belongs_to_server(key, server_id)) {
        return Ok(());
    }
    update_file(path, |credentials| {
        credentials.retain(|key, _| !belongs_to_server(key, server_id));
    })
}

fn update_file(path: &Path, update: impl FnOnce(&mut CredentialFile)) -> Result<(), String> {
    let _guard = CREDENTIALS_WRITE_LOCK
        .lock()
        .map_err(|_| "credentials.json lock is unavailable".to_string())?;
    let mut credentials = read_file(path)?;
    update(&mut credentials);
    write_file(path, &credentials)
}

fn read_file(path: &Path) -> Result<CredentialFile, String> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("{}: invalid format: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(CredentialFile::new()),
        Err(e) => Err(format!("failed to read {}: {e}", path.display())),
    }
}

/// 一時ファイルに書いてから置き換えることで、書き込み途中の破損を防ぐ。
fn write_file(path: &Path, credentials: &CredentialFile) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(credentials)
        .map_err(|e| format!("failed to serialize credentials: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("failed to replace {}: {e}", path.display()))
}

#[cfg(windows)]
mod dpapi {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn input_blob(data: &[u8]) -> Result<CRYPT_INTEGER_BLOB, String> {
        let len = u32::try_from(data.len()).map_err(|_| "data is too large".to_string())?;
        Ok(CRYPT_INTEGER_BLOB {
            cbData: len,
            // DPAPI は入力を読み取るだけなので、const ポインタを mut にキャストしても書き換えは起きない
            pbData: data.as_ptr().cast_mut(),
        })
    }

    fn empty_blob() -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        }
    }

    /// DPAPI が確保した出力バッファをコピーし、解放する。
    fn take_output(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if output.pbData.is_null() {
            return Vec::new();
        }
        // SAFETY: DPAPI の成功時、pbData は cbData バイトの初期化済みバッファを指す
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        // SAFETY: pbData は DPAPI が LocalAlloc で確保したもので、解放はここで一度だけ行う
        unsafe { LocalFree(Some(HLOCAL(output.pbData.cast()))) };
        bytes
    }

    pub fn protect(plain: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
        let input = input_blob(plain)?;
        let entropy = input_blob(entropy)?;
        let mut output = empty_blob();
        // SAFETY: input と entropy は呼び出し中有効なスライスを指し、output は DPAPI が書き込む
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                Some(&entropy as *const _),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(|e| format!("failed to encrypt password: {e}"))?;
        Ok(take_output(output))
    }

    pub fn unprotect(encrypted: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
        let input = input_blob(encrypted)?;
        let entropy = input_blob(entropy)?;
        let mut output = empty_blob();
        // SAFETY: input と entropy は呼び出し中有効なスライスを指し、output は DPAPI が書き込む
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy as *const _),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(|e| e.to_string())?;
        Ok(take_output(output))
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(_plain: &[u8], _entropy: &[u8]) -> Result<Vec<u8>, String> {
        Err("saving passwords is supported only on Windows".to_string())
    }

    pub fn unprotect(_encrypted: &[u8], _entropy: &[u8]) -> Result<Vec<u8>, String> {
        Err("saving passwords is supported only on Windows".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nterm-cred-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join(CREDENTIALS_FILE)
    }

    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn returns_none_when_not_saved() {
        let path = temp_path("missing");
        assert_eq!(load_secret(&path, "srv-001").unwrap(), None);
        assert!(!has_secret(&path, "srv-001").unwrap());
    }

    #[test]
    fn round_trips_saved_secret_without_storing_plaintext() {
        let path = temp_path("roundtrip");
        save_secret(&path, "srv-001", "p@ss日本語").unwrap();
        let loaded = load_secret(&path, "srv-001");
        let content = fs::read_to_string(&path).unwrap_or_default();
        cleanup(&path);
        assert_eq!(loaded.unwrap().as_deref(), Some("p@ss日本語"));
        assert!(!content.contains("p@ss"));
    }

    #[test]
    fn keeps_other_servers_when_saving_and_deleting() {
        let path = temp_path("multi");
        save_secret(&path, "srv-001", "one").unwrap();
        save_secret(&path, "srv-002", "two").unwrap();
        delete_secret(&path, "srv-001").unwrap();
        let first = has_secret(&path, "srv-001");
        let second = load_secret(&path, "srv-002");
        cleanup(&path);
        assert!(!first.unwrap());
        assert_eq!(second.unwrap().as_deref(), Some("two"));
    }

    #[test]
    fn overwrites_existing_secret() {
        let path = temp_path("overwrite");
        save_secret(&path, "srv-001", "old").unwrap();
        save_secret(&path, "srv-001", "new").unwrap();
        let loaded = load_secret(&path, "srv-001");
        cleanup(&path);
        assert_eq!(loaded.unwrap().as_deref(), Some("new"));
    }

    #[test]
    fn deletes_login_and_switch_user_secrets_of_server_only() {
        let path = temp_path("server-secrets");
        save_secret(&path, "srv-1", "login").unwrap();
        save_secret(&path, &switch_user_key("srv-1", "oracle"), "su").unwrap();
        save_secret(&path, "srv-10", "other").unwrap();
        delete_server_secrets(&path, "srv-1").unwrap();
        let login = has_secret(&path, "srv-1");
        let su = has_secret(&path, &switch_user_key("srv-1", "oracle"));
        let other = has_secret(&path, "srv-10");
        cleanup(&path);
        assert!(!login.unwrap());
        assert!(!su.unwrap());
        assert!(other.unwrap());
    }

    #[test]
    fn deleting_unsaved_secret_succeeds() {
        let path = temp_path("delete-missing");
        assert!(delete_secret(&path, "srv-001").is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn reports_corrupted_entry() {
        let path = temp_path("corrupted");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{ "srv-001": { "secret": "not base64!" } }"#).unwrap();
        let result = load_secret(&path, "srv-001");
        cleanup(&path);
        assert!(result.unwrap_err().contains("corrupted"));
    }
}
