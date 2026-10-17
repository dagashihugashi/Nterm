//! Orca の MCP サーバが Claude Code に提供する、サーバの読み取りの道具（AI.md 4 章・3.2・12 章）。
//!
//! AI はコマンドを書かない。AI が指定できるのは道具の引数（パス・行数・検索語）だけで、
//! 実行するコマンドはここで固定の形に組み立てる。引数はシングルクォートで囲み、`--` の後に置く。
//! サーバでは読み取りのコマンドしか実行しない（サーバから外部への通信は行わない）。

use serde_json::{json, Value};

/// 1 回の道具の結果として返す最大バイト数（トークン消費の抑制）
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// パスの最大バイト数（コマンドは隠しシェルへ 1 行で送るため、端末の 1 行の上限 4096 バイトより十分短くする）
const MAX_PATH_LENGTH: usize = 1024;
/// 組み立てたコマンドの最大バイト数（目印のコマンドを足しても端末の 1 行の上限に収まるように）
const MAX_COMMAND_BYTES: usize = 3000;
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
        let call = Self::parse_arguments(name, input)?;
        // クォートで長くなった結果、端末の 1 行に収まらないコマンドは送らない（途中で切れるとクォートがずれる）
        if call.remote_command().is_some_and(|command| command.len() > MAX_COMMAND_BYTES) {
            return Err("the arguments are too long".to_string());
        }
        Ok(call)
    }

    fn parse_arguments(name: &str, input: &Value) -> Result<Self, String> {
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
            Self::ReadFile { path, offset, limit: lines, .. } => regular_file_only(
                path,
                &format!(
                    "sed -n '{offset},{}p' -- {} | head -c {limit}",
                    offset.saturating_add(lines - 1),
                    quote(path)
                ),
            ),
            Self::TailFile { path, lines, .. } => {
                regular_file_only(path, &format!("tail -n {lines} -- {} | head -c {limit}", quote(path)))
            }
            Self::SearchFile { path, pattern, context, .. } => regular_file_only(
                path,
                &format!(
                    "grep -F -n -m {MAX_MATCHES} -C {context} -e {} -- {} | head -c {limit}",
                    quote(pattern),
                    quote(path)
                ),
            ),
        };
        Some(command)
    }
}

/// 普通のファイルのときだけ読む（`/dev/zero`・FIFO 等を読ませると終わらず、隠しシェルが詰まり、サーバのメモリも使うため）
fn regular_file_only(path: &str, command: &str) -> String {
    format!("if [ -f {} ]; then {command}; else echo 'orca: not a regular file' >&2; fi", quote(path))
}

/// シェルのシングルクォートで囲む
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// 引数に使えない文字：制御文字と、承認カードでの見た目を偽装できる文字
/// （双方向の書式・ゼロ幅・見えない書式の文字。"Trojan Source" 型の表示の偽装を防ぐ）
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{2800}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
                | '\u{E0100}'..='\u{E01EF}'
        )
}

/// 表示用：見えない文字を `<U+202E>` の形にして見えるようにする（組み込みの道具の引数も含め、承認カードに出す前に通す）
pub fn reveal_hidden(text: &str) -> String {
    text.chars().fold(String::with_capacity(text.len()), |mut shown, c| {
        if is_hidden_char(c) {
            shown.push_str(&format!("<U+{:04X}>", u32::from(c)));
        } else {
            shown.push(c);
        }
        shown
    })
}

fn string<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("\"{key}\" is required and must be a string"))
}

fn session(input: &Value) -> Result<String, String> {
    let session = string(input, "session")?;
    if session.is_empty() || session.chars().any(is_hidden_char) {
        return Err("\"session\" is invalid".to_string());
    }
    Ok(session.to_string())
}

fn path(input: &Value) -> Result<String, String> {
    let path = string(input, "path")?;
    if !path.starts_with('/') {
        return Err("\"path\" must be an absolute path".to_string());
    }
    if path.len() > MAX_PATH_LENGTH || path.chars().any(is_hidden_char) {
        return Err("\"path\" is invalid".to_string());
    }
    Ok(path.to_string())
}

fn pattern(input: &Value) -> Result<String, String> {
    let pattern = string(input, "pattern")?;
    if pattern.is_empty() || pattern.len() > MAX_PATTERN_LENGTH || pattern.chars().any(is_hidden_char) {
        return Err(format!(
            "\"pattern\" must be 1-{MAX_PATTERN_LENGTH} characters on one line, without invisible or text-direction characters"
        ));
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

/// MCP の `tools/list` で返す道具の定義（`input_schema` を MCP の `inputSchema` にする）
pub fn mcp_definitions() -> Vec<Value> {
    definitions()
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| {
            json!({
                "name": tool["name"],
                "description": tool["description"],
                "inputSchema": tool["input_schema"],
            })
        })
        .collect()
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
    fn rejects_characters_that_can_disguise_the_approval_card() {
        // 文字の向きを変える文字（RLO 等）・ゼロ幅の文字で、承認カードの見た目を偽装させない
        for hidden in [
            '\u{202E}', '\u{2066}', '\u{200B}', '\u{200D}', '\u{FEFF}', '\u{061C}', '\u{E0041}', '\u{2028}', '\u{3164}',
            '\u{FE0F}', '\u{E0100}', '\u{2800}', '\u{115F}',
        ] {
            let text = format!("/var/log/app{hidden}gol.txt");
            assert!(parse("stat", json!({ "session": "t1", "path": text })).is_err(), "path with {hidden:?}");
            let input = json!({ "session": "t1", "path": "/a", "pattern": format!("ok{hidden}") });
            assert!(parse("search_file", input).is_err(), "pattern with {hidden:?}");
            assert!(parse("read_terminal", json!({ "session": format!("t1{hidden}") })).is_err());
        }
        // 日本語や記号のパスは通常どおり使える
        assert!(parse("stat", json!({ "session": "t1", "path": "/home/田中/ログ（旧）.txt" })).is_ok());
    }

    #[test]
    fn reveals_hidden_characters_for_display() {
        assert_eq!(reveal_hidden("/etc/\u{202E}txt.wodahs"), "/etc/<U+202E>txt.wodahs");
        assert_eq!(reveal_hidden("a\u{200B}b\u{E0041}"), "a<U+200B>b<U+E0041>");
        assert_eq!(reveal_hidden("/home/田中/a b.txt"), "/home/田中/a b.txt");
    }

    #[test]
    fn builds_fixed_form_commands_with_quoted_arguments() {
        let call = parse("tail_file", json!({ "session": "t1", "path": "/var/log/it's.log", "lines": 20 })).unwrap();
        assert_eq!(
            call.remote_command().unwrap(),
            format!(
                r"if [ -f '/var/log/it'\''s.log' ]; then tail -n 20 -- '/var/log/it'\''s.log' | head -c {}; else echo 'orca: not a regular file' >&2; fi",
                MAX_OUTPUT_BYTES + 1
            )
        );
        let call = parse("read_file", json!({ "session": "t1", "path": "/a", "offset": 10, "limit": 5 })).unwrap();
        assert!(call.remote_command().unwrap().contains("then sed -n '10,14p' -- '/a' |"));
    }

    #[test]
    fn reads_only_regular_files() {
        // /dev/zero・FIFO 等は読まない（終わらないコマンドで隠しシェルが詰まらないように）
        for (name, input) in [
            ("read_file", json!({ "session": "t1", "path": "/dev/zero" })),
            ("tail_file", json!({ "session": "t1", "path": "/dev/zero" })),
            ("search_file", json!({ "session": "t1", "path": "/dev/zero", "pattern": "x" })),
        ] {
            let command = parse(name, input).unwrap().remote_command().unwrap();
            assert!(command.starts_with("if [ -f '/dev/zero' ]; then "), "{command}");
            assert!(command.ends_with("; else echo 'orca: not a regular file' >&2; fi"), "{command}");
        }
    }

    #[test]
    fn rejects_arguments_that_make_the_command_too_long() {
        assert!(parse("stat", json!({ "session": "t1", "path": format!("/{}", "a".repeat(MAX_PATH_LENGTH)) })).is_err());
        // クォートで 4 倍に膨らむ文字だけのパスは、パスの上限内でもコマンドの上限で断る
        let quotes = format!("/{}", "'".repeat(MAX_PATH_LENGTH / 2));
        assert!(parse("read_file", json!({ "session": "t1", "path": quotes })).is_err());
        let normal = format!("/{}", "a".repeat(MAX_PATH_LENGTH - 1));
        let command = parse("search_file", json!({ "session": "t1", "path": normal, "pattern": "x" })).unwrap();
        assert!(command.remote_command().unwrap().len() <= MAX_COMMAND_BYTES);
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
    fn no_server_command_talks_to_the_network() {
        // サーバで実行するコマンドは読み取りのコマンドだけで、外部と通信するコマンドは組み立てない
        let calls = [
            parse("list_dir", json!({ "session": "t1", "path": "/a" })).unwrap(),
            parse("stat", json!({ "session": "t1", "path": "/a" })).unwrap(),
            parse("read_file", json!({ "session": "t1", "path": "/a" })).unwrap(),
            parse("tail_file", json!({ "session": "t1", "path": "/a" })).unwrap(),
            parse("search_file", json!({ "session": "t1", "path": "/a", "pattern": "x" })).unwrap(),
        ];
        let programs: Vec<String> = calls
            .iter()
            .map(|call| {
                let command = call.remote_command().unwrap();
                // 普通のファイルかの確認（if [ -f ... ]; then）の後の、実際に読むコマンド
                let body = command.split_once("; then ").map_or(command.as_str(), |(_, body)| body);
                body.split_whitespace().next().unwrap().to_string()
            })
            .collect();
        assert_eq!(programs, ["ls", "stat", "sed", "tail", "grep"]);
    }

    #[test]
    fn mcp_definitions_use_input_schema_in_camel_case() {
        let tools = mcp_definitions();
        let names: Vec<&str> = tools.iter().filter_map(|tool| tool["name"].as_str()).collect();
        assert_eq!(names, ["list_sessions", "list_dir", "stat", "read_file", "tail_file", "search_file", "read_terminal"]);
        assert!(tools.iter().all(|tool| tool["inputSchema"]["type"] == "object" && tool.get("input_schema").is_none()));
    }

    #[test]
    fn every_definition_parses_with_its_required_arguments() {
        let definitions = definitions();
        let sample = json!({ "session": "t1", "path": "/a", "pattern": "x", "url": "https://example.com" });
        for tool in definitions.as_array().unwrap() {
            let name = tool["name"].as_str().unwrap();
            assert!(ToolCall::parse(name, &sample).is_ok(), "{name}");
        }
    }
}
