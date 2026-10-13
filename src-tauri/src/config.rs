use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::codec::TerminalCodec;
use crate::credentials;

/// 設定フォルダを上書きする環境変数（開発時など）。未設定時は実行ファイルと同じフォルダ。
const CONFIG_DIR_ENV: &str = "NTERM_CONFIG_DIR";
const SERVERS_FILE: &str = "servers.json";

/// servers.json の「読み込み→追加→書き込み」を直列化する
static SERVERS_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthType {
    Password,
    Publickey,
    KeyboardInteractive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogMode {
    Append,
    Overwrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileTransferMode {
    Auto,
    Sftp,
    Scp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthConfig {
    pub r#type: AuthType,
    pub key_file: Option<String>,
    #[serde(default)]
    pub remember: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogConfig {
    pub dir: Option<String>,
    pub mode: Option<LogMode>,
    pub filename_pattern: Option<String>,
}

/// 切り替えコマンドの既定値（`{user}` は切り替え先のユーザー名に置き換える）
pub const DEFAULT_SWITCH_COMMAND: &str = "su - {user}";
const MAX_SWITCH_STEPS: usize = 5;
const MAX_SWITCH_USER_LENGTH: usize = 32;
const MAX_SWITCH_COMMAND_LENGTH: usize = 200;

/// ログイン後のユーザ切り替え（su 等）の1手順。パスワードは credentials.json に保存する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchUserStep {
    pub user: String,
    /// 省略時は `su - {user}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl SwitchUserStep {
    /// 送信するコマンド（`{user}` を展開済み）
    pub fn command_line(&self) -> String {
        self.command
            .as_deref()
            .unwrap_or(DEFAULT_SWITCH_COMMAND)
            .replace("{user}", &self.user)
    }

    fn normalized(&self) -> Self {
        Self {
            user: self.user.trim().to_string(),
            command: non_empty(self.command.clone()),
        }
    }
}

/// ユーザー名はコマンドに埋め込むため、シェルで特別な意味を持つ文字を受け付けない
fn is_valid_user_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SWITCH_USER_LENGTH
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn validate_switch_users(id: &str, steps: &[SwitchUserStep]) -> Result<(), String> {
    if steps.len() > MAX_SWITCH_STEPS {
        return Err(format!("server {id}: up to {MAX_SWITCH_STEPS} user switch steps are allowed"));
    }
    for step in steps {
        if !is_valid_user_name(&step.user) {
            return Err(format!(
                "server {id}: invalid user name to switch to: \"{}\" (letters, digits, '.', '_' and '-' only)",
                step.user
            ));
        }
        if let Some(command) = &step.command {
            if command.trim().is_empty()
                || command.len() > MAX_SWITCH_COMMAND_LENGTH
                || command.chars().any(char::is_control)
            {
                return Err(format!("server {id}: invalid switch command for {}", step.user));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthConfig,
    pub env: Option<String>,
    pub theme: Option<String>,
    pub encoding: Option<String>,
    pub file_transfer: Option<FileTransferMode>,
    pub log: Option<LogConfig>,
    /// ログイン後に順に実行するユーザ切り替え
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub switch_users: Vec<SwitchUserStep>,
}

/// サーバ追加・編集ダイアログからの入力
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_type: AuthType,
    pub key_file: Option<String>,
    pub env: Option<String>,
    pub encoding: Option<String>,
    /// themes.json のテーマ名。省略時は env に応じた既定のテーマ
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub switch_users: Vec<SwitchUserStep>,
}

impl ServerInput {
    fn into_config(self, id: String) -> ServerConfig {
        let base = ServerConfig {
            id,
            name: String::new(),
            host: String::new(),
            port: 0,
            user: String::new(),
            auth: AuthConfig {
                r#type: self.auth_type,
                key_file: None,
                remember: false,
            },
            env: None,
            theme: None,
            encoding: None,
            file_transfer: None,
            log: None,
            switch_users: Vec::new(),
        };
        self.apply_to(base)
    }

    /// 既存の設定に入力値を反映する。ダイアログで扱わない項目（log・fileTransfer 等）は維持する。
    fn apply_to(self, base: ServerConfig) -> ServerConfig {
        ServerConfig {
            name: self.name.trim().to_string(),
            host: self.host.trim().to_string(),
            port: self.port,
            user: self.user.trim().to_string(),
            auth: AuthConfig {
                r#type: self.auth_type,
                key_file: non_empty(self.key_file),
                remember: base.auth.remember,
            },
            env: non_empty(self.env),
            encoding: non_empty(self.encoding),
            theme: non_empty(self.theme),
            switch_users: self.switch_users.iter().map(SwitchUserStep::normalized).collect(),
            ..base
        }
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 設定ファイルを置くフォルダ。ポータブル運用のため、カレントディレクトリではなく実行ファイルの場所を基準にする。
pub fn config_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    let exe = std::env::current_exe()
        .map_err(|e| format!("failed to locate executable: {e}"))?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("executable has no parent directory: {}", exe.display()))
}

/// `servers.json` を読み込む。ファイルが存在しない場合は未登録として空の一覧を返す。
pub fn read_servers(path: &Path) -> Result<Vec<ServerConfig>, String> {
    match fs::read_to_string(path) {
        Ok(content) => parse_servers(&content).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("failed to read {}: {e}", path.display())),
    }
}

pub fn parse_servers(json: &str) -> Result<Vec<ServerConfig>, String> {
    let servers: Vec<ServerConfig> =
        serde_json::from_str(json).map_err(|e| format!("invalid format: {e}"))?;
    validate_servers(&servers)?;
    Ok(servers)
}

fn validate_servers(servers: &[ServerConfig]) -> Result<(), String> {
    let mut seen_ids = HashSet::new();
    for server in servers {
        validate_server(server)?;
        if !seen_ids.insert(server.id.as_str()) {
            return Err(format!("duplicate server id: {}", server.id));
        }
    }
    Ok(())
}

fn validate_server(server: &ServerConfig) -> Result<(), String> {
    if server.id.trim().is_empty() {
        return Err(format!("server \"{}\": id is required", server.name));
    }
    let id = &server.id;
    if server.name.trim().is_empty() {
        return Err(format!("server {id}: name is required"));
    }
    if server.host.trim().is_empty() {
        return Err(format!("server {id}: host is required"));
    }
    if server.user.trim().is_empty() {
        return Err(format!("server {id}: user is required"));
    }
    if server.port == 0 {
        return Err(format!("server {id}: port must be 1-65535"));
    }
    let has_key_file = server
        .auth
        .key_file
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty());
    if server.auth.r#type == AuthType::Publickey && !has_key_file {
        return Err(format!("server {id}: keyFile is required for publickey authentication"));
    }
    if let Some(encoding) = &server.encoding {
        TerminalCodec::new(Some(encoding)).map_err(|e| format!("server {id}: {e}"))?;
    }
    validate_switch_users(id, &server.switch_users)
}

/// 既存の一覧に新しいサーバを加えた一覧を返す（元の一覧は変更しない）。
pub fn append_server(
    servers: &[ServerConfig],
    new_server: ServerInput,
    id: String,
) -> Result<Vec<ServerConfig>, String> {
    let updated: Vec<ServerConfig> = servers
        .iter()
        .cloned()
        .chain(std::iter::once(new_server.into_config(id)))
        .collect();
    validate_servers(&updated)?;
    Ok(updated)
}

/// 指定IDのサーバを入力値で更新した一覧を返す（元の一覧は変更しない）。
pub fn replace_server(
    servers: &[ServerConfig],
    id: &str,
    input: ServerInput,
) -> Result<Vec<ServerConfig>, String> {
    let index = position_of(servers, id)?;
    let replaced = input.apply_to(servers[index].clone());
    let updated: Vec<ServerConfig> = servers
        .iter()
        .enumerate()
        .map(|(i, server)| if i == index { replaced.clone() } else { server.clone() })
        .collect();
    validate_servers(&updated)?;
    Ok(updated)
}

/// 指定IDのサーバを除いた一覧を返す（元の一覧は変更しない）。
pub fn remove_server(servers: &[ServerConfig], id: &str) -> Result<Vec<ServerConfig>, String> {
    position_of(servers, id)?;
    Ok(servers.iter().filter(|s| s.id != id).cloned().collect())
}

fn position_of(servers: &[ServerConfig], id: &str) -> Result<usize, String> {
    servers
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| format!("server not found: {id}"))
}

/// servers.json を排他的に「読み込み→変更→書き込み」する。
/// 既存ファイルが壊れている場合は read_servers がエラーを返すため、上書きしない。
fn modify_servers(
    change: impl FnOnce(&[ServerConfig]) -> Result<Vec<ServerConfig>, String>,
) -> Result<Vec<ServerConfig>, String> {
    let _guard = SERVERS_WRITE_LOCK
        .lock()
        .map_err(|_| "servers.json lock is unavailable".to_string())?;
    let path = servers_path()?;
    let updated = change(&read_servers(&path)?)?;
    write_servers(&path, &updated)?;
    Ok(updated)
}

/// 既存IDと重複しないサーバIDを生成する。
fn generate_server_id(servers: &[ServerConfig]) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let base = format!("srv-{millis:x}");
    let is_taken = |candidate: &str| servers.iter().any(|s| s.id == candidate);
    let mut candidate = base.clone();
    let mut suffix = 1;
    while is_taken(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    candidate
}

/// 一時ファイルに書いてから置き換えることで、書き込み途中の破損を防ぐ。
pub fn write_servers(path: &Path, servers: &[ServerConfig]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(servers)
        .map_err(|e| format!("failed to serialize servers: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("failed to replace {}: {e}", path.display()))
}

fn servers_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join(SERVERS_FILE))
}

pub fn find_server(server_id: &str) -> Result<ServerConfig, String> {
    read_servers(&servers_path()?)?
        .into_iter()
        .find(|server| server.id == server_id)
        .ok_or_else(|| format!("server not found: {server_id}"))
}

#[tauri::command]
pub fn load_servers() -> Result<Vec<ServerConfig>, String> {
    read_servers(&servers_path()?)
}

#[tauri::command]
pub fn add_server(server: ServerInput) -> Result<ServerConfig, String> {
    let updated = modify_servers(|servers| {
        append_server(servers, server, generate_server_id(servers))
    })?;
    updated
        .last()
        .cloned()
        .ok_or_else(|| "failed to add server".to_string())
}

/// 手順から外れた切り替え先ユーザー（保存済みパスワードを削除する対象）
fn removed_switch_users(before: &[SwitchUserStep], after: &[SwitchUserStep]) -> Vec<String> {
    before
        .iter()
        .filter(|old| !after.iter().any(|new| new.user == old.user))
        .map(|old| old.user.clone())
        .collect()
}

#[tauri::command]
pub fn update_server(id: String, server: ServerInput) -> Result<ServerConfig, String> {
    let previous = find_server(&id)?;
    let updated = modify_servers(|servers| replace_server(servers, &id, server))?;
    let saved = updated
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("server not found: {id}"))?;
    let path = credentials::credentials_path()?;
    for user in removed_switch_users(&previous.switch_users, &saved.switch_users) {
        credentials::delete_secret(&path, &credentials::switch_user_key(&id, &user)).map_err(|e| {
            format!("server was saved, but the saved password for {user} could not be removed: {e}")
        })?;
    }
    Ok(saved)
}

/// サーバを削除し、保存済みパスワード（ユーザ切り替え用を含む）も合わせて削除する
/// （known_hosts は他サーバと共有し得るため残す）。
#[tauri::command]
pub fn delete_server(id: String) -> Result<(), String> {
    modify_servers(|servers| remove_server(servers, &id))?;
    credentials::delete_server_secrets(&credentials::credentials_path()?, &id)
        .map_err(|e| format!("server was deleted, but its saved password could not be removed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"[{
        "id": "srv-001", "name": "Web01", "host": "192.0.2.1", "port": 22,
        "user": "app", "auth": { "type": "password" }
    }]"#;

    fn with_server(field: &str) -> String {
        format!(
            r#"[{{ "id": "srv-001", "name": "Web01", "host": "192.0.2.1", "port": 22,
                  "user": "app", "auth": {{ "type": "password" }}, {field} }}]"#
        )
    }

    #[test]
    fn parses_full_server_entry() {
        let json = r#"[{
            "id": "srv-001", "name": "本番 Web01", "host": "192.0.2.1", "port": 2222,
            "user": "app",
            "auth": { "type": "publickey", "keyFile": "C:/keys/id_ed25519", "remember": true },
            "env": "production", "theme": "danger-red", "encoding": "Shift_JIS",
            "fileTransfer": "scp",
            "log": { "dir": "./logs", "mode": "overwrite", "filenamePattern": "{name}.log" }
        }]"#;
        let server = &parse_servers(json).unwrap()[0];
        assert_eq!(server.port, 2222);
        assert_eq!(server.auth.r#type, AuthType::Publickey);
        assert!(server.auth.remember);
        assert_eq!(server.file_transfer, Some(FileTransferMode::Scp));
        assert_eq!(server.log.as_ref().unwrap().mode, Some(LogMode::Overwrite));
    }

    #[test]
    fn optional_fields_can_be_omitted() {
        let server = &parse_servers(MINIMAL).unwrap()[0];
        assert!(!server.auth.remember);
        assert!(server.env.is_none());
        assert!(server.encoding.is_none());
        assert!(server.log.is_none());
    }

    #[test]
    fn rejects_unknown_auth_type() {
        let json = MINIMAL.replace(r#""password""#, r#""pasword""#);
        assert!(parse_servers(&json).is_err());
    }

    #[test]
    fn rejects_unknown_file_transfer_mode() {
        assert!(parse_servers(&with_server(r#""fileTransfer": "ftp""#)).is_err());
    }

    #[test]
    fn rejects_unsupported_encoding() {
        let error = parse_servers(&with_server(r#""encoding": "no-such""#)).unwrap_err();
        assert!(error.contains("srv-001"));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let entry = MINIMAL.trim().trim_start_matches('[').trim_end_matches(']');
        let json = format!("[{entry}, {entry}]");
        let error = parse_servers(&json).unwrap_err();
        assert!(error.contains("duplicate server id: srv-001"));
    }

    #[test]
    fn rejects_empty_host() {
        let json = MINIMAL.replace("192.0.2.1", " ");
        assert!(parse_servers(&json).unwrap_err().contains("host is required"));
    }

    #[test]
    fn rejects_port_zero() {
        let json = MINIMAL.replace(r#""port": 22"#, r#""port": 0"#);
        assert!(parse_servers(&json).is_err());
    }

    #[test]
    fn reports_position_of_syntax_error() {
        let error = parse_servers("[{ \"id\": }]").unwrap_err();
        assert!(error.contains("line 1"));
    }

    #[test]
    fn missing_file_means_no_servers() {
        let path = std::env::temp_dir().join("nterm-test-missing-servers.json");
        assert!(read_servers(&path).unwrap().is_empty());
    }

    fn new_server(name: &str) -> ServerInput {
        ServerInput {
            name: format!("  {name}  "),
            host: " 192.0.2.10 ".to_string(),
            port: 22,
            user: "app".to_string(),
            auth_type: AuthType::Password,
            key_file: None,
            env: Some("  ".to_string()),
            encoding: Some("Shift_JIS".to_string()),
            theme: None,
            switch_users: Vec::new(),
        }
    }

    fn switch_step(user: &str, command: Option<&str>) -> SwitchUserStep {
        SwitchUserStep { user: user.to_string(), command: command.map(str::to_string) }
    }

    #[test]
    fn parses_switch_users_and_expands_default_command() {
        let json = with_server(
            r#""switchUsers": [{ "user": "app2" }, { "user": "oracle", "command": "sudo -iu {user}" }]"#,
        );
        let server = &parse_servers(&json).unwrap()[0];
        assert_eq!(server.switch_users[0].command_line(), "su - app2");
        assert_eq!(server.switch_users[1].command_line(), "sudo -iu oracle");
    }

    #[test]
    fn switch_users_are_omitted_from_json_when_empty() {
        let server = parse_servers(MINIMAL).unwrap().remove(0);
        assert!(server.switch_users.is_empty());
        assert!(!serde_json::to_string(&server).unwrap().contains("switchUsers"));
    }

    #[test]
    fn rejects_user_names_that_could_inject_commands() {
        for user in ["oracle; rm -rf ~", "a b", "$(id)", "-c", ""] {
            let json = with_server(&format!(r#""switchUsers": [{{ "user": "{user}" }}]"#));
            assert!(parse_servers(&json).is_err(), "accepted {user:?}");
        }
    }

    #[test]
    fn rejects_switch_command_with_line_break() {
        let json = with_server(r#""switchUsers": [{ "user": "oracle", "command": "su - {user}\nid" }]"#);
        assert!(parse_servers(&json).is_err());
    }

    #[test]
    fn rejects_too_many_switch_steps() {
        let steps = [r#"{ "user": "a" }"#; MAX_SWITCH_STEPS + 1].join(",");
        let json = with_server(&format!(r#""switchUsers": [{steps}]"#));
        assert!(parse_servers(&json).unwrap_err().contains("up to"));
    }

    #[test]
    fn replace_server_normalizes_switch_users() {
        let original = parse_servers(MINIMAL).unwrap();
        let input = ServerInput {
            switch_users: vec![switch_step(" oracle ", Some("  "))],
            ..new_server("Web01")
        };
        let updated = replace_server(&original, "srv-001", input).unwrap();
        assert_eq!(updated[0].switch_users, [switch_step("oracle", None)]);
    }

    #[test]
    fn finds_switch_users_removed_by_edit() {
        let before = [switch_step("app2", None), switch_step("oracle", None)];
        let after = [switch_step("oracle", Some("sudo -iu {user}"))];
        assert_eq!(removed_switch_users(&before, &after), ["app2"]);
    }

    #[test]
    fn append_server_returns_new_list_without_mutating_original() {
        let original = parse_servers(MINIMAL).unwrap();
        let updated = append_server(&original, new_server("Web02"), "srv-new".into()).unwrap();
        assert_eq!(original.len(), 1);
        assert_eq!(updated.len(), 2);
        let added = &updated[1];
        assert_eq!(added.id, "srv-new");
        assert_eq!(added.name, "Web02");
        assert_eq!(added.host, "192.0.2.10");
        assert!(added.env.is_none());
        assert_eq!(added.encoding.as_deref(), Some("Shift_JIS"));
    }

    #[test]
    fn append_server_rejects_blank_name() {
        let error = append_server(&[], new_server(" "), "srv-new".into()).unwrap_err();
        assert!(error.contains("name is required"));
    }

    #[test]
    fn append_server_rejects_unsupported_encoding() {
        let server = ServerInput {
            encoding: Some("no-such".to_string()),
            ..new_server("Web02")
        };
        assert!(append_server(&[], server, "srv-new".into()).is_err());
    }

    #[test]
    fn publickey_server_requires_key_file() {
        let json = MINIMAL.replace(r#""password""#, r#""publickey""#);
        assert!(parse_servers(&json).unwrap_err().contains("keyFile is required"));
    }

    #[test]
    fn append_server_keeps_key_file() {
        let server = ServerInput {
            auth_type: AuthType::Publickey,
            key_file: Some(" ~/.ssh/id_ed25519 ".to_string()),
            ..new_server("Web02")
        };
        let updated = append_server(&[], server, "srv-new".into()).unwrap();
        assert_eq!(updated[0].auth.key_file.as_deref(), Some("~/.ssh/id_ed25519"));
    }

    #[test]
    fn append_server_rejects_publickey_without_key_file() {
        let server = ServerInput {
            auth_type: AuthType::Publickey,
            ..new_server("Web02")
        };
        assert!(append_server(&[], server, "srv-new".into()).is_err());
    }

    #[test]
    fn replace_server_updates_fields_and_keeps_others() {
        let json = with_server(r#""theme": "danger-red", "log": { "mode": "append" }"#);
        let original = parse_servers(&json).unwrap();
        let input = ServerInput { theme: Some(" light ".to_string()), ..new_server("Web01-renamed") };
        let updated = replace_server(&original, "srv-001", input).unwrap();
        let server = &updated[0];
        assert_eq!(server.id, "srv-001");
        assert_eq!(server.name, "Web01-renamed");
        assert_eq!(server.host, "192.0.2.10");
        assert_eq!(server.theme.as_deref(), Some("light"));
        // ダイアログで扱わない項目は維持する
        assert_eq!(server.log.as_ref().unwrap().mode, Some(LogMode::Append));
        assert_eq!(original[0].name, "Web01");
    }

    #[test]
    fn empty_theme_means_default_for_environment() {
        let json = with_server(r#""theme": "danger-red""#);
        let original = parse_servers(&json).unwrap();
        let input = ServerInput { theme: Some(String::new()), ..new_server("Web01") };
        let updated = replace_server(&original, "srv-001", input).unwrap();
        assert!(updated[0].theme.is_none());
    }

    #[test]
    fn replace_server_rejects_unknown_id() {
        let original = parse_servers(MINIMAL).unwrap();
        let error = replace_server(&original, "srv-999", new_server("X")).unwrap_err();
        assert!(error.contains("server not found"));
    }

    #[test]
    fn replace_server_validates_input() {
        let original = parse_servers(MINIMAL).unwrap();
        assert!(replace_server(&original, "srv-001", new_server(" ")).is_err());
    }

    #[test]
    fn remove_server_returns_list_without_target() {
        let original = append_server(
            &parse_servers(MINIMAL).unwrap(),
            new_server("Web02"),
            "srv-002".into(),
        )
        .unwrap();
        let updated = remove_server(&original, "srv-001").unwrap();
        assert_eq!(original.len(), 2);
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].id, "srv-002");
    }

    #[test]
    fn remove_server_rejects_unknown_id() {
        let original = parse_servers(MINIMAL).unwrap();
        assert!(remove_server(&original, "srv-999").is_err());
    }

    #[test]
    fn generated_id_does_not_collide() {
        let first = generate_server_id(&[]);
        let existing = parse_servers(&MINIMAL.replace("srv-001", &first)).unwrap();
        let second = generate_server_id(&existing);
        assert!(first.starts_with("srv-"));
        assert_ne!(first, second);
    }

    #[test]
    fn written_servers_can_be_read_back() {
        let dir = std::env::temp_dir().join(format!("nterm-test-{}", std::process::id()));
        let path = dir.join(SERVERS_FILE);
        let servers = append_server(&[], new_server("Web02"), "srv-new".into()).unwrap();

        write_servers(&path, &servers).unwrap();
        let loaded = read_servers(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "srv-new");
        assert_eq!(loaded[0].auth.r#type, AuthType::Password);
    }

    #[test]
    fn serializes_enums_in_config_format() {
        let mut server = parse_servers(MINIMAL).unwrap().remove(0);
        server.auth.r#type = AuthType::KeyboardInteractive;
        server.file_transfer = Some(FileTransferMode::Sftp);
        let json = serde_json::to_string(&server).unwrap();
        assert!(json.contains(r#""type":"keyboard-interactive""#));
        assert!(json.contains(r#""fileTransfer":"sftp""#));
    }
}
