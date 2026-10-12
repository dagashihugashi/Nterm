//! セッションログ（エビデンス記録）の書き込み。
//!
//! 受信テキストを `LogFormatter` で確定行に整形し、各行の先頭にタイムスタンプを付けて
//! UTF-8 で書き出す。ログ開始ダイアログの初期値（保存先・ファイル名・保存モード）もここで決める。

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde::Serialize;

use crate::config::{LogMode, ServerConfig};
use crate::log_formatter::LogFormatter;

/// `servers.json` の `log` に指定がない場合の既定値（settings.json 未実装のためアプリ内蔵値）
const DEFAULT_LOG_DIR: &str = "logs";
const DEFAULT_FILENAME_PATTERN: &str = "{env}_{name}_{YYYYMMDD}.log";
const DEFAULT_LOG_MODE: LogMode = LogMode::Append;

const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.3f";
/// Windows のメモ帳等で扱いやすいよう CRLF で書き出す
const LINE_ENDING: &str = "\r\n";
/// Windows のファイル名に使えない文字
const INVALID_FILENAME_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// ログ開始ダイアログの初期値
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogDefaults {
    pub dir: String,
    pub file_name: String,
    pub mode: LogMode,
}

/// サーバ設定（`log`）からダイアログの初期値を作る。相対パスの保存先は設定フォルダを基準にする。
pub fn defaults_for(server: &ServerConfig, config_dir: &Path, now: NaiveDateTime) -> LogDefaults {
    let log = server.log.as_ref();
    let dir = log
        .and_then(|l| l.dir.as_deref())
        .filter(|d| !d.trim().is_empty())
        .unwrap_or(DEFAULT_LOG_DIR);
    let pattern = log
        .and_then(|l| l.filename_pattern.as_deref())
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(DEFAULT_FILENAME_PATTERN);
    LogDefaults {
        dir: resolve_dir(dir, config_dir).display().to_string(),
        file_name: expand_pattern(pattern, server, now),
        mode: log.and_then(|l| l.mode).unwrap_or(DEFAULT_LOG_MODE),
    }
}

fn resolve_dir(dir: &str, config_dir: &Path) -> PathBuf {
    let path = Path::new(dir.trim());
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    }
}

/// ファイル名パターンの `{env}` `{name}` `{host}` `{user}` `{YYYYMMDD}` `{HHmmss}` を展開する。
/// 空の値（env 未設定等）で生じた余分な `_` は詰める。
pub fn expand_pattern(pattern: &str, server: &ServerConfig, now: NaiveDateTime) -> String {
    let date = now.format("%Y%m%d").to_string();
    let time = now.format("%H%M%S").to_string();
    let replacements = [
        ("{env}", server.env.as_deref().unwrap_or("")),
        ("{name}", server.name.as_str()),
        ("{host}", server.host.as_str()),
        ("{user}", server.user.as_str()),
        ("{YYYYMMDD}", date.as_str()),
        ("{HHmmss}", time.as_str()),
    ];
    let expanded = replacements
        .iter()
        .fold(pattern.to_string(), |name, (token, value)| {
            name.replace(token, &sanitize_filename_part(value))
        });
    tidy_separators(&expanded)
}

fn sanitize_filename_part(value: &str) -> String {
    value
        .chars()
        .map(|c| if is_invalid_filename_char(c) { '_' } else { c })
        .collect()
}

fn is_invalid_filename_char(c: char) -> bool {
    c.is_control() || INVALID_FILENAME_CHARS.contains(&c)
}

/// 連続する `_` を1つにし、先頭と拡張子直前の `_` を取り除く
fn tidy_separators(name: &str) -> String {
    let mut collapsed = String::with_capacity(name.len());
    for c in name.chars() {
        if !(c == '_' && collapsed.ends_with('_')) {
            collapsed.push(c);
        }
    }
    collapsed.trim_start_matches('_').replace("_.", ".")
}

/// 保存先フォルダとファイル名からログファイルのパスを決める（フォルダがなければ作る）。
pub fn prepare_log_path(dir: &str, file_name: &str, config_dir: &Path) -> Result<PathBuf, String> {
    let file_name = file_name.trim();
    if dir.trim().is_empty() {
        return Err("log folder is required".to_string());
    }
    if file_name.is_empty() {
        return Err("log file name is required".to_string());
    }
    if file_name.chars().any(is_invalid_filename_char) {
        return Err(format!(
            "log file name cannot contain any of: {}",
            INVALID_FILENAME_CHARS.iter().collect::<String>()
        ));
    }
    let dir = resolve_dir(dir, config_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    Ok(dir.join(file_name))
}

/// 取得中のログ。セッションスレッドが所有し、受信テキストを書き込む。
pub struct SessionLog<W: Write = BufWriter<File>> {
    formatter: LogFormatter,
    writer: W,
}

impl SessionLog {
    pub fn open(path: &Path, mode: LogMode) -> Result<Self, String> {
        let mut options = OpenOptions::new();
        options.create(true);
        match mode {
            LogMode::Append => options.append(true),
            LogMode::Overwrite => options.write(true).truncate(true),
        };
        let file = options
            .open(path)
            .map_err(|e| format!("failed to open {}: {e}", path.display()))?;
        Ok(Self::new(BufWriter::new(file)))
    }
}

impl<W: Write> SessionLog<W> {
    fn new(writer: W) -> Self {
        Self { formatter: LogFormatter::default(), writer }
    }

    /// 受信テキストを書き込む。行が確定するたびにファイルへ反映する（異常終了時の欠落を防ぐ）。
    pub fn write(&mut self, text: &str) -> Result<(), String> {
        self.write_at(text, Local::now().naive_local())
    }

    fn write_at(&mut self, text: &str, now: NaiveDateTime) -> Result<(), String> {
        let lines = self.formatter.feed(text);
        if lines.is_empty() {
            return Ok(());
        }
        lines.iter().try_for_each(|line| self.write_line(line, now))?;
        self.flush()
    }

    /// 未確定の行（プロンプト等）も書き出して終了する。
    pub fn finish(self) -> Result<(), String> {
        self.finish_at(Local::now().naive_local())
    }

    fn finish_at(mut self, now: NaiveDateTime) -> Result<(), String> {
        if let Some(line) = self.formatter.finish() {
            self.write_line(&line, now)?;
        }
        self.flush()
    }

    /// タイムスタンプは行が確定した（LF を受信した）時刻
    fn write_line(&mut self, line: &str, now: NaiveDateTime) -> Result<(), String> {
        write!(self.writer, "[{}] {line}{LINE_ENDING}", now.format(TIMESTAMP_FORMAT))
            .map_err(|e| format!("failed to write log: {e}"))
    }

    fn flush(&mut self) -> Result<(), String> {
        self.writer.flush().map_err(|e| format!("failed to write log: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{parse_servers, LogConfig};
    use chrono::NaiveDate;

    fn at(hour: u32, min: u32, sec: u32, milli: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 5)
            .unwrap()
            .and_hms_milli_opt(hour, min, sec, milli)
            .unwrap()
    }

    fn server() -> ServerConfig {
        let json = r#"[{
            "id": "srv-001", "name": "Web01", "host": "192.0.2.1", "port": 22,
            "user": "app", "auth": { "type": "password" }, "env": "production"
        }]"#;
        parse_servers(json).unwrap().remove(0)
    }

    #[test]
    fn writes_formatted_lines_with_timestamp() {
        let mut buffer = Vec::new();
        let mut log = SessionLog::new(&mut buffer);
        log.write_at("$ lss\x08 \x08\r\n\x1b[32mfile\x1b[0m\r\n", at(18, 2, 31, 123)).unwrap();
        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "[2026-10-05 18:02:31.123] $ ls\r\n[2026-10-05 18:02:31.123] file\r\n"
        );
    }

    #[test]
    fn finish_writes_pending_line() {
        let mut buffer = Vec::new();
        let mut log = SessionLog::new(&mut buffer);
        log.write_at("$ ", at(9, 0, 0, 0)).unwrap();
        log.write_at("exit", at(9, 0, 1, 0)).unwrap();
        log.finish_at(at(9, 0, 1, 500)).unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), "[2026-10-05 09:00:01.500] $ exit\r\n");
    }

    #[test]
    fn finish_without_pending_line_writes_nothing() {
        let mut buffer = Vec::new();
        let mut log = SessionLog::new(&mut buffer);
        log.write_at("done\r\n", at(9, 0, 0, 0)).unwrap();
        log.finish_at(at(9, 0, 1, 0)).unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), "[2026-10-05 09:00:00.000] done\r\n");
    }

    #[test]
    fn writes_utf8_text() {
        let mut buffer = Vec::new();
        let mut log = SessionLog::new(&mut buffer);
        log.write_at("日本語\r\n", at(0, 0, 0, 0)).unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), "[2026-10-05 00:00:00.000] 日本語\r\n");
    }

    #[test]
    fn expands_default_pattern() {
        let name = expand_pattern(DEFAULT_FILENAME_PATTERN, &server(), at(18, 2, 31, 0));
        assert_eq!(name, "production_Web01_20261005.log");
    }

    #[test]
    fn expands_time_host_and_user() {
        let name = expand_pattern("{user}@{host}_{HHmmss}.log", &server(), at(8, 5, 9, 0));
        assert_eq!(name, "app@192.0.2.1_080509.log");
    }

    #[test]
    fn omits_separator_of_missing_env() {
        let server = ServerConfig { env: None, ..server() };
        let name = expand_pattern(DEFAULT_FILENAME_PATTERN, &server, at(0, 0, 0, 0));
        assert_eq!(name, "Web01_20261005.log");
    }

    #[test]
    fn replaces_invalid_filename_chars_in_values() {
        let server = ServerConfig { name: "web/01:prod".to_string(), ..server() };
        let name = expand_pattern("{name}.log", &server, at(0, 0, 0, 0));
        assert_eq!(name, "web_01_prod.log");
    }

    #[test]
    fn defaults_use_built_in_values_without_log_config() {
        let defaults = defaults_for(&server(), Path::new("C:/nterm"), at(0, 0, 0, 0));
        assert_eq!(PathBuf::from(&defaults.dir), Path::new("C:/nterm").join("logs"));
        assert_eq!(defaults.file_name, "production_Web01_20261005.log");
        assert_eq!(defaults.mode, LogMode::Append);
    }

    #[test]
    fn defaults_use_server_log_config() {
        let server = ServerConfig {
            log: Some(LogConfig {
                dir: Some("D:/evidence".to_string()),
                mode: Some(LogMode::Overwrite),
                filename_pattern: Some("{name}.log".to_string()),
            }),
            ..server()
        };
        let defaults = defaults_for(&server, Path::new("C:/nterm"), at(0, 0, 0, 0));
        assert_eq!(defaults.dir, "D:/evidence");
        assert_eq!(defaults.file_name, "Web01.log");
        assert_eq!(defaults.mode, LogMode::Overwrite);
    }

    #[test]
    fn prepare_log_path_rejects_invalid_input() {
        let dir = std::env::temp_dir();
        let dir = dir.to_str().unwrap();
        assert!(prepare_log_path(dir, "  ", Path::new(".")).is_err());
        assert!(prepare_log_path(dir, "a/b.log", Path::new(".")).is_err());
        assert!(prepare_log_path(" ", "a.log", Path::new(".")).is_err());
    }

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nterm-log-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn append_mode_keeps_existing_content() {
        let dir = temp_dir("append");
        let path = prepare_log_path(dir.to_str().unwrap(), "a.log", Path::new(".")).unwrap();
        fs::write(&path, "old\r\n").unwrap();

        let mut log = SessionLog::open(&path, LogMode::Append).unwrap();
        log.write("new\r\n").unwrap();
        log.finish().unwrap();
        let content = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert!(content.starts_with("old\r\n["));
        assert!(content.ends_with("] new\r\n"));
    }

    #[test]
    fn overwrite_mode_discards_existing_content() {
        let dir = temp_dir("overwrite");
        let path = prepare_log_path(dir.to_str().unwrap(), "a.log", Path::new(".")).unwrap();
        fs::write(&path, "old content that is longer\r\n").unwrap();

        let mut log = SessionLog::open(&path, LogMode::Overwrite).unwrap();
        log.write("new\r\n").unwrap();
        log.finish().unwrap();
        let content = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert!(content.starts_with('['));
        assert!(content.ends_with("] new\r\n"));
        assert!(!content.contains("old"));
    }
}
