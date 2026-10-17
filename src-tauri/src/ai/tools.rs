//! AI に提供するツールの定義と、引数の検証・コマンドの組み立て（AI.md 4 章・3.2）。
//!
//! AI はコマンドを書かない。AI が指定できるのはツールの引数（パス・行数・検索語）だけで、
//! 実行するコマンドはここで固定の形に組み立てる。引数はシングルクォートで囲み、`--` の後に置く。

use serde_json::{json, Value};

/// 1 回のツール結果として返す最大バイト数（トークン消費の抑制）
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_PATH_LENGTH: usize = 4096;
const MAX_PATTERN_LENGTH: usize = 200;
const MAX_LINES: u64 = 500;
const DEFAULT_READ_LINES: u64 = 200;
const DEFAULT_TAIL_LINES: u64 = 100;
const DEFAULT_TERMINAL_LINES: u64 = 100;
const MAX_CONTEXT_LINES: u64 = 5;
const DEFAULT_CONTEXT_LINES: u64 = 2;
/// search_file で返す一致の最大件数
const MAX_MATCHES: u64 = 100;

/// 検証済みのツール呼び出し
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCall {
    ListSessions,
    ListDir { session: String, path: String },
    Stat { session: String, path: String },
    ReadFile { session: String, path: String, offset: u64, limit: u64 },
    TailFile { session: String, path: String, lines: u64 },
    SearchFile { session: String, path: String, pattern: String, context: u64 },
    ReadTerminal { session: String, lines: u64 },
}

impl ToolCall {
    /// AI から受け取ったツール名と引数（JSON）を検証する
    pub fn parse(name: &str, input: &Value) -> Result<Self, String> {
        match name {
            "list_sessions" => Ok(Self::ListSessions),
            "list_dir" => Ok(Self::ListDir { session: session(input)?, path: path(input)? }),
            "stat" => Ok(Self::Stat { session: session(input)?, path: path(input)? }),
            "read_file" => Ok(Self::ReadFile {
                session: session(input)?,
                path: path(input)?,
                offset: number(input, "offset", 1, 1, u64::MAX)?,
                limit: number(input, "limit", DEFAULT_READ_LINES, 1, MAX_LINES)?,
            }),
            "tail_file" => Ok(Self::TailFile {
                session: session(input)?,
                path: path(input)?,
                lines: number(input, "lines", DEFAULT_TAIL_LINES, 1, MAX_LINES)?,
            }),
            "search_file" => Ok(Self::SearchFile {
                session: session(input)?,
                path: path(input)?,
                pattern: pattern(input)?,
                context: number(input, "context", DEFAULT_CONTEXT_LINES, 0, MAX_CONTEXT_LINES)?,
            }),
            "read_terminal" => Ok(Self::ReadTerminal {
                session: session(input)?,
                lines: number(input, "lines", DEFAULT_TERMINAL_LINES, 1, MAX_LINES)?,
            }),
            other => Err(format!("unknown tool: {other}")),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::ListSessions => "list_sessions",
            Self::ListDir { .. } => "list_dir",
            Self::Stat { .. } => "stat",
            Self::ReadFile { .. } => "read_file",
            Self::TailFile { .. } => "tail_file",
            Self::SearchFile { .. } => "search_file",
            Self::ReadTerminal { .. } => "read_terminal",
        }
    }

    /// 対象のタブ（サーバに触れないツールは None）
    pub fn session(&self) -> Option<&str> {
        match self {
            Self::ListSessions => None,
            Self::ListDir { session, .. }
            | Self::Stat { session, .. }
            | Self::ReadFile { session, .. }
            | Self::TailFile { session, .. }
            | Self::SearchFile { session, .. }
            | Self::ReadTerminal { session, .. } => Some(session),
        }
    }

    /// 承認ダイアログ・監査ログに表示する引数の要約
    pub fn summary(&self) -> String {
        match self {
            Self::ListSessions => "open tabs".to_string(),
            Self::ListDir { path, .. } | Self::Stat { path, .. } => path.clone(),
            Self::ReadFile { path, offset, limit, .. } => {
                format!("{path} (lines {offset}-{})", offset.saturating_add(limit - 1))
            }
            Self::TailFile { path, lines, .. } => format!("{path} (last {lines} lines)"),
            Self::SearchFile { path, pattern, context, .. } => {
                format!("{path} (\"{pattern}\", {context} lines of context)")
            }
            Self::ReadTerminal { lines, .. } => format!("last {lines} lines of the terminal"),
        }
    }

    /// サーバの隠しシェルで実行するコマンド（サーバで実行しないツールは None）
    pub fn remote_command(&self) -> Option<String> {
        let limit = MAX_OUTPUT_BYTES + 1;
        let command = match self {
            Self::ListSessions | Self::ReadTerminal { .. } => return None,
            Self::ListDir { path, .. } => format!("ls -la -- {} | head -c {limit}", quote(path)),
            Self::Stat { path, .. } => format!("stat -- {}", quote(path)),
            Self::ReadFile { path, offset, limit: lines, .. } => format!(
                "sed -n '{offset},{}p' -- {} | head -c {limit}",
                offset.saturating_add(lines - 1),
                quote(path)
            ),
            Self::TailFile { path, lines, .. } => {
                format!("tail -n {lines} -- {} | head -c {limit}", quote(path))
            }
            Self::SearchFile { path, pattern, context, .. } => format!(
                "grep -F -n -m {MAX_MATCHES} -C {context} -e {} -- {} | head -c {limit}",
                quote(pattern),
                quote(path)
            ),
        };
        Some(command)
    }
}

/// シェルのシングルクォートで囲む
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn string<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("\"{key}\" is required and must be a string"))
}

fn session(input: &Value) -> Result<String, String> {
    let session = string(input, "session")?;
    if session.is_empty() || session.chars().any(char::is_control) {
        return Err("\"session\" is invalid".to_string());
    }
    Ok(session.to_string())
}

fn path(input: &Value) -> Result<String, String> {
    let path = string(input, "path")?;
    if !path.starts_with('/') {
        return Err("\"path\" must be an absolute path".to_string());
    }
    if path.len() > MAX_PATH_LENGTH || path.chars().any(char::is_control) {
        return Err("\"path\" is invalid".to_string());
    }
    Ok(path.to_string())
}

fn pattern(input: &Value) -> Result<String, String> {
    let pattern = string(input, "pattern")?;
    if pattern.is_empty() || pattern.len() > MAX_PATTERN_LENGTH || pattern.chars().any(char::is_control) {
        return Err(format!("\"pattern\" must be 1-{MAX_PATTERN_LENGTH} characters on one line"));
    }
    Ok(pattern.to_string())
}

/// 省略時は既定値、範囲外は上限・下限に丸める（整数以外はエラー）
fn number(input: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .map(|n| n.clamp(min, max))
            .ok_or_else(|| format!("\"{key}\" must be a non-negative integer")),
    }
}

/// API に渡すツールの定義（Anthropic Messages API の `tools`）
pub fn definitions() -> Value {
    let session = json!({
        "type": "string",
        "description": "The tab id from list_sessions."
    });
    let path = json!({ "type": "string", "description": "Absolute path on the server." });
    json!([
        {
            "name": "list_sessions",
            "description": "List the open terminal tabs: tab id, server name, host, environment (production etc.), connection state and the user whose permissions the read tools use.",
            "input_schema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_dir",
            "description": "List a directory on the server (ls -la).",
            "input_schema": {
                "type": "object",
                "properties": { "session": session, "path": path },
                "required": ["session", "path"]
            }
        },
        {
            "name": "stat",
            "description": "Show the attributes of a file on the server (stat).",
            "input_schema": {
                "type": "object",
                "properties": { "session": session, "path": path },
                "required": ["session", "path"]
            }
        },
        {
            "name": "read_file",
            "description": format!("Read a range of lines from a text file on the server. Returns at most {MAX_LINES} lines."),
            "input_schema": {
                "type": "object",
                "properties": {
                    "session": session,
                    "path": path,
                    "offset": { "type": "integer", "description": "First line number (1-based). Default 1." },
                    "limit": { "type": "integer", "description": format!("Number of lines. Default {DEFAULT_READ_LINES}, max {MAX_LINES}.") }
                },
                "required": ["session", "path"]
            }
        },
        {
            "name": "tail_file",
            "description": "Read the last lines of a text file on the server (e.g. a log file).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "session": session,
                    "path": path,
                    "lines": { "type": "integer", "description": format!("Number of lines. Default {DEFAULT_TAIL_LINES}, max {MAX_LINES}.") }
                },
                "required": ["session", "path"]
            }
        },
        {
            "name": "search_file",
            "description": format!("Search a file on the server for a fixed string (not a regular expression) and return the matching lines with line numbers and surrounding lines. Returns at most {MAX_MATCHES} matches."),
            "input_schema": {
                "type": "object",
                "properties": {
                    "session": session,
                    "path": path,
                    "pattern": { "type": "string", "description": "The text to search for (fixed string)." },
                    "context": { "type": "integer", "description": format!("Lines of context around each match. Default {DEFAULT_CONTEXT_LINES}, max {MAX_CONTEXT_LINES}.") }
                },
                "required": ["session", "path", "pattern"]
            }
        },
        {
            "name": "read_terminal",
            "description": "Read the recent output of a terminal tab (escape sequences removed). Full-screen programs such as vi or top are not reproduced accurately.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "session": session,
                    "lines": { "type": "integer", "description": format!("Number of lines. Default {DEFAULT_TERMINAL_LINES}, max {MAX_LINES}.") }
                },
                "required": ["session"]
            }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(name: &str, input: Value) -> Result<ToolCall, String> {
        ToolCall::parse(name, &input)
    }

    #[test]
    fn parses_tools_with_defaults() {
        assert_eq!(
            parse("tail_file", json!({ "session": "t1", "path": "/var/log/messages" })).unwrap(),
            ToolCall::TailFile { session: "t1".into(), path: "/var/log/messages".into(), lines: 100 }
        );
        assert_eq!(
            parse("read_file", json!({ "session": "t1", "path": "/etc/hosts" })).unwrap(),
            ToolCall::ReadFile { session: "t1".into(), path: "/etc/hosts".into(), offset: 1, limit: 200 }
        );
        assert_eq!(parse("list_sessions", json!({})).unwrap(), ToolCall::ListSessions);
    }

    #[test]
    fn clamps_numbers_to_the_limits() {
        let call = parse("tail_file", json!({ "session": "t1", "path": "/a", "lines": 100000 })).unwrap();
        assert_eq!(call, ToolCall::TailFile { session: "t1".into(), path: "/a".into(), lines: 500 });
        let call = parse("search_file", json!({ "session": "t1", "path": "/a", "pattern": "x", "context": 99 })).unwrap();
        assert!(matches!(call, ToolCall::SearchFile { context: 5, .. }));
        let call = parse("read_file", json!({ "session": "t1", "path": "/a", "offset": 0, "limit": 0 })).unwrap();
        assert!(matches!(call, ToolCall::ReadFile { offset: 1, limit: 1, .. }));
    }

    #[test]
    fn rejects_invalid_arguments() {
        assert!(parse("list_dir", json!({ "session": "t1", "path": "relative/path" })).is_err());
        assert!(parse("list_dir", json!({ "session": "t1", "path": "/tmp\n; rm -rf /" })).is_err());
        assert!(parse("list_dir", json!({ "path": "/tmp" })).is_err());
        assert!(parse("tail_file", json!({ "session": "t1", "path": "/a", "lines": "10; reboot" })).is_err());
        assert!(parse("tail_file", json!({ "session": "t1", "path": "/a", "lines": -1 })).is_err());
        assert!(parse("search_file", json!({ "session": "t1", "path": "/a", "pattern": "" })).is_err());
        assert!(parse("run_command", json!({ "command": "rm -rf /" })).is_err());
    }

    #[test]
    fn builds_fixed_form_commands_with_quoted_arguments() {
        let call = parse("tail_file", json!({ "session": "t1", "path": "/var/log/it's.log", "lines": 20 })).unwrap();
        assert_eq!(
            call.remote_command().unwrap(),
            format!(r"tail -n 20 -- '/var/log/it'\''s.log' | head -c {}", MAX_OUTPUT_BYTES + 1)
        );
        let call = parse("read_file", json!({ "session": "t1", "path": "/a", "offset": 10, "limit": 5 })).unwrap();
        assert!(call.remote_command().unwrap().starts_with("sed -n '10,14p' -- '/a' |"));
    }

    #[test]
    fn shell_syntax_in_arguments_stays_inside_quotes() {
        // パスや検索語にシェルの記号が入っていても、クォートの外に出ない
        let call = parse(
            "search_file",
            json!({ "session": "t1", "path": "/tmp/$(reboot)", "pattern": "'; rm -rf / #" }),
        )
        .unwrap();
        let command = call.remote_command().unwrap();
        assert!(command.contains(r"-e ''\''; rm -rf / #'"));
        assert!(command.contains("-- '/tmp/$(reboot)'"));
        // 検索語が "-" で始まってもオプションとみなされない（-e の引数として渡す）
        let call = parse("search_file", json!({ "session": "t1", "path": "/a", "pattern": "-rf" })).unwrap();
        assert!(call.remote_command().unwrap().contains("-e '-rf' -- '/a'"));
    }

    #[test]
    fn local_tools_do_not_run_on_the_server() {
        assert_eq!(ToolCall::ListSessions.remote_command(), None);
        let call = parse("read_terminal", json!({ "session": "t1" })).unwrap();
        assert_eq!(call.remote_command(), None);
        assert_eq!(call.session(), Some("t1"));
    }

    #[test]
    fn summarizes_calls_for_the_approval_dialog() {
        let call = parse("read_file", json!({ "session": "t1", "path": "/etc/hosts", "offset": 10, "limit": 5 })).unwrap();
        assert_eq!(call.summary(), "/etc/hosts (lines 10-14)");
    }

    #[test]
    fn every_definition_parses_with_its_required_arguments() {
        let definitions = definitions();
        let sample = json!({ "session": "t1", "path": "/a", "pattern": "x" });
        for tool in definitions.as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            let call = ToolCall::parse(name, &sample).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(call.name(), name);
        }
    }
}
