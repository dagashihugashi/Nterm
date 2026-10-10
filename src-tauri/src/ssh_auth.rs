use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use ssh2::{ErrorCode, Session};

use crate::config::{AuthConfig, AuthType};

// libssh2 のエラーコード（libssh2.h）
const LIBSSH2_ERROR_FILE: i32 = -16;
const LIBSSH2_ERROR_AUTHENTICATION_FAILED: i32 = -18;
const LIBSSH2_ERROR_PUBLICKEY_UNVERIFIED: i32 = -19;

/// 鍵ファイルの形式判定に読む先頭バイト数
const KEY_HEADER_LEN: usize = 64;

/// 認証に使う資格情報
pub enum Credential {
    Password(String),
    PublicKey {
        key_file: PathBuf,
        passphrase: Option<String>,
    },
}

/// ユーザーのホームフォルダ（`~` の展開に使う）
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// 鍵ファイルのパスを解決する。
/// `~/` はホームフォルダ基準、相対パスは設定フォルダ（アプリと同階層）基準、絶対パスはそのまま。
pub fn resolve_key_path(raw: &str, config_dir: &Path, home: Option<&Path>) -> PathBuf {
    let raw = raw.trim();
    let home_relative = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\"));
    match (home_relative, home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if Path::new(raw).is_absolute() => PathBuf::from(raw),
        _ => config_dir.join(raw),
    }
}

/// servers.json の認証設定とユーザー入力（パスワード／パスフレーズ）から資格情報を組み立てる。
pub fn build_credential(
    auth: &AuthConfig,
    secret: String,
    config_dir: &Path,
    home: Option<&Path>,
) -> Result<Credential, String> {
    match auth.r#type {
        AuthType::Password => Ok(Credential::Password(secret)),
        AuthType::Publickey => {
            let key_file = auth
                .key_file
                .as_deref()
                .filter(|path| !path.trim().is_empty())
                .ok_or("key file is not configured for public key authentication")?;
            Ok(Credential::PublicKey {
                key_file: resolve_key_path(key_file, config_dir, home),
                passphrase: Some(secret).filter(|s| !s.is_empty()),
            })
        }
        AuthType::KeyboardInteractive => {
            Err("keyboard-interactive authentication is not supported yet".to_string())
        }
    }
}

/// 接続前に鍵ファイルを確認し、よくある設定ミスを分かりやすいエラーにする。
pub fn check_private_key(path: &Path) -> Result<(), String> {
    let mut header = [0u8; KEY_HEADER_LEN];
    let read = File::open(path)
        .and_then(|mut file| file.read(&mut header))
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => format!("key file not found: {}", path.display()),
            _ => format!("failed to read key file {}: {e}", path.display()),
        })?;
    inspect_key_header(&header[..read]).map_err(|e| format!("{}: {e}", path.display()))
}

fn inspect_key_header(header: &[u8]) -> Result<(), String> {
    if header.starts_with(b"PuTTY-User-Key-File") {
        return Err("PuTTY format (.ppk) is not supported. \
            Convert it with PuTTYgen: Conversions > Export OpenSSH key"
            .to_string());
    }
    let public_key_prefixes: [&[u8]; 3] = [b"ssh-", b"ecdsa-", b"sk-"];
    if public_key_prefixes.iter().any(|prefix| header.starts_with(prefix)) {
        return Err("this is a public key (.pub). Specify the private key file".to_string());
    }
    Ok(())
}

pub fn authenticate(session: &Session, user: &str, credential: &Credential) -> Result<(), String> {
    match credential {
        Credential::Password(password) => session
            .userauth_password(user, password)
            .map_err(|e| format!("password authentication failed: {e}")),
        Credential::PublicKey { key_file, passphrase } => session
            .userauth_pubkey_file(user, None, key_file, passphrase.as_deref())
            .map_err(|e| describe_pubkey_error(&e)),
    }
}

fn describe_pubkey_error(error: &ssh2::Error) -> String {
    let hint = match error.code() {
        ErrorCode::Session(LIBSSH2_ERROR_FILE) => {
            "the private key could not be loaded (wrong passphrase or unsupported key format)"
        }
        ErrorCode::Session(LIBSSH2_ERROR_AUTHENTICATION_FAILED)
        | ErrorCode::Session(LIBSSH2_ERROR_PUBLICKEY_UNVERIFIED) => {
            "the server rejected this key (is the public key registered in authorized_keys?)"
        }
        _ => "",
    };
    if hint.is_empty() {
        format!("public key authentication failed: {error}")
    } else {
        format!("public key authentication failed: {hint} [{error}]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(r#type: AuthType, key_file: Option<&str>) -> AuthConfig {
        AuthConfig {
            r#type,
            key_file: key_file.map(str::to_string),
            remember: false,
        }
    }

    fn temp_file(name: &str, content: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("nterm-{}-{name}", std::process::id()));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn resolves_tilde_from_home() {
        let path = resolve_key_path("~/.ssh/id_ed25519", Path::new("C:/app"), Some(Path::new("C:/Users/me")));
        assert_eq!(path, Path::new("C:/Users/me").join(".ssh/id_ed25519"));
    }

    #[test]
    fn resolves_relative_path_from_config_dir() {
        let path = resolve_key_path(" keys/id_rsa ", Path::new("C:/app"), None);
        assert_eq!(path, Path::new("C:/app").join("keys/id_rsa"));
    }

    #[test]
    fn keeps_absolute_path() {
        let path = resolve_key_path("C:\\keys\\id_rsa", Path::new("C:/app"), None);
        assert_eq!(path, PathBuf::from("C:\\keys\\id_rsa"));
    }

    #[test]
    fn builds_password_credential() {
        let credential = build_credential(&auth(AuthType::Password, None), "pw".into(), Path::new("."), None);
        assert!(matches!(credential, Ok(Credential::Password(p)) if p == "pw"));
    }

    #[test]
    fn empty_passphrase_means_unencrypted_key() {
        let credential =
            build_credential(&auth(AuthType::Publickey, Some("key")), String::new(), Path::new("."), None);
        assert!(matches!(credential, Ok(Credential::PublicKey { passphrase: None, .. })));
    }

    #[test]
    fn keeps_non_empty_passphrase() {
        let credential =
            build_credential(&auth(AuthType::Publickey, Some("key")), "secret".into(), Path::new("."), None);
        assert!(matches!(credential, Ok(Credential::PublicKey { passphrase: Some(p), .. }) if p == "secret"));
    }

    #[test]
    fn publickey_requires_key_file() {
        let credential = build_credential(&auth(AuthType::Publickey, Some(" ")), String::new(), Path::new("."), None);
        assert!(credential.is_err());
    }

    #[test]
    fn keyboard_interactive_is_not_supported_yet() {
        let credential =
            build_credential(&auth(AuthType::KeyboardInteractive, None), String::new(), Path::new("."), None);
        assert!(credential.is_err());
    }

    #[test]
    fn accepts_openssh_private_key() {
        let path = temp_file("openssh", b"-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blbn...\n");
        let result = check_private_key(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_putty_key_with_conversion_hint() {
        let path = temp_file("ppk", b"PuTTY-User-Key-File-3: ssh-ed25519\n");
        let error = check_private_key(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(error.contains("PuTTYgen"));
    }

    #[test]
    fn rejects_public_key_file() {
        let path = temp_file("pub", b"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 user@host\n");
        let error = check_private_key(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(error.contains("public key"));
    }

    #[test]
    fn reports_missing_key_file() {
        let path = std::env::temp_dir().join("nterm-no-such-key");
        assert!(check_private_key(&path).unwrap_err().contains("key file not found"));
    }
}
