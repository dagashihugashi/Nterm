//! Claude Code のセッションの一覧（AI あり版のみ。AI.md 12.2）。
//!
//! Claude Code は作業フォルダごとに `~/.claude/projects/<作業フォルダを変換した名前>/<セッション ID>.jsonl` へ
//! 会話を保存する。Orca はここを読むだけで（書き込まない）、「続きから再開」の一覧に使う。
//! ファイルの形式は Claude Code の内部の形式のため、読めない行は無視し、読める範囲で題名を作る。

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use serde_json::Value;

/// 一覧に出す最大件数（新しい順）
const MAX_SESSIONS: usize = 50;
/// 再開したときに表示する履歴の最大件数（新しいものを残す）
const MAX_HISTORY_ENTRIES: usize = 300;
/// 題名を探すために読む行数・バイト数の上限（巨大なファイルを置かれても読みすぎない）
const MAX_LINES_FOR_TITLE: usize = 200;
const MAX_BYTES_FOR_TITLE: u64 = 4 * 1024 * 1024;
const MAX_TITLE_CHARS: usize = 80;
/// 履歴の 1 件の本文の最大文字数（画面に送る量を抑える）
const MAX_HISTORY_TEXT_CHARS: usize = 20_000;

/// Claude Code のセッション ID の形（UUID）か。起動の引数（`--resume`）とファイル名に使うため、
/// `-` で始まる値（オプションとみなされる）やパスの区切りを含む値を通さない
pub fn is_session_id(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    /// 最初の質問（なければ最後の質問）
    pub title: String,
    /// 最後に更新した日時（UNIX 時刻の秒）
    pub updated: u64,
}

/// 作業フォルダのセッション（新しい順）
pub fn list_sessions(cwd: &Path) -> Vec<SessionSummary> {
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Vec::new();
    };
    list_sessions_in(&home.join(".claude").join("projects"), cwd)
}

fn list_sessions_in(projects: &Path, cwd: &Path) -> Vec<SessionSummary> {
    let encoded = encode_project_dir(cwd);
    let Ok(dirs) = std::fs::read_dir(projects) else {
        return Vec::new();
    };
    // ドライブ名の大文字・小文字が違うフォルダ（C-- と c--）がありうるため、区別せずに探す
    let mut files: Vec<(PathBuf, u64)> = dirs
        .filter_map(Result::ok)
        .filter(|dir| dir.file_name().to_string_lossy().eq_ignore_ascii_case(&encoded))
        .filter_map(|dir| std::fs::read_dir(dir.path()).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|file| file.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter(|path| path.file_stem().is_some_and(|stem| is_session_id(&stem.to_string_lossy())))
        .map(|path| {
            let updated = modified_secs(&path);
            (path, updated)
        })
        .collect();
    // 新しいものから開く（題名のないファイルを飛ばしても足りるよう、一覧に出す件数の 2 倍まで）
    files.sort_by_key(|(_, updated)| std::cmp::Reverse(*updated));
    files.truncate(MAX_SESSIONS * 2);
    let mut sessions: Vec<SessionSummary> = files.iter().filter_map(|(path, updated)| summarize(path, *updated)).collect();
    sessions.truncate(MAX_SESSIONS);
    sessions
}

fn modified_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

/// 会話の履歴の 1 件（再開したセッションの表示用）
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryEntry {
    User { text: String },
    Assistant { text: String },
    ToolUse { id: String, name: String, input: Value },
    ToolResult { id: String, output: String, is_error: bool },
}

/// セッションの会話の履歴（古い順。新しい `MAX_HISTORY_ENTRIES` 件まで）
pub fn load_history(cwd: &Path, session_id: &str) -> Result<Vec<HistoryEntry>, String> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).ok_or("the user folder is unknown")?;
    load_history_in(&home.join(".claude").join("projects"), cwd, session_id)
}

fn load_history_in(projects: &Path, cwd: &Path, session_id: &str) -> Result<Vec<HistoryEntry>, String> {
    // ファイル名に使うため、セッション ID の形を確認する（パスの区切り等を含めない）
    if !is_session_id(session_id) {
        return Err(format!("invalid session id: {session_id}"));
    }
    let encoded = encode_project_dir(cwd);
    let file_name = format!("{session_id}.jsonl");
    let path = std::fs::read_dir(projects)
        .map_err(|e| format!("failed to read {}: {e}", projects.display()))?
        .filter_map(Result::ok)
        .filter(|dir| dir.file_name().to_string_lossy().eq_ignore_ascii_case(&encoded))
        .map(|dir| dir.path().join(&file_name))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("the session was not found: {session_id}"))?;
    let file = File::open(&path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    Ok(history_from_lines(BufReader::new(file).lines().map_while(Result::ok)))
}

/// 履歴の行から表示する出来事を取り出す（付属情報・サブエージェントの作業は除く）
fn history_from_lines(lines: impl Iterator<Item = String>) -> Vec<HistoryEntry> {
    let mut entries: VecDeque<HistoryEntry> = VecDeque::new();
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
        if entry["isSidechain"] == true || entry["isMeta"] == true {
            continue;
        }
        for item in history_items(&entry) {
            if entries.len() == MAX_HISTORY_ENTRIES {
                entries.pop_front();
            }
            entries.push_back(item);
        }
    }
    entries.into()
}

fn history_items(entry: &Value) -> Vec<HistoryEntry> {
    let content = &entry["message"]["content"];
    let blocks: Vec<Value> = match content {
        Value::String(text) => vec![serde_json::json!({ "type": "text", "text": text })],
        Value::Array(blocks) => blocks.clone(),
        _ => return Vec::new(),
    };
    let is_user = match entry["type"].as_str() {
        Some("user") => true,
        Some("assistant") => false,
        _ => return Vec::new(),
    };
    blocks
        .iter()
        .filter_map(|block| match block["type"].as_str() {
            Some("text") if is_user => question_text(&Value::String(block["text"].as_str()?.to_string()))
                .map(|text| HistoryEntry::User { text: limit_text(&text) }),
            Some("text") => {
                let text = block["text"].as_str()?.trim();
                (!text.is_empty()).then(|| HistoryEntry::Assistant { text: limit_text(text) })
            }
            Some("tool_use") => Some(HistoryEntry::ToolUse {
                id: block["id"].as_str()?.to_string(),
                name: block["name"].as_str()?.to_string(),
                input: block["input"].clone(),
            }),
            Some("tool_result") => Some(HistoryEntry::ToolResult {
                id: block["tool_use_id"].as_str()?.to_string(),
                output: limit_text(&result_text(&block["content"])),
                is_error: block["is_error"].as_bool().unwrap_or(false),
            }),
            _ => None,
        })
        .collect()
}

/// 長すぎる本文は先頭だけにする（画面に送る量を抑える）
fn limit_text(text: &str) -> String {
    match text.char_indices().nth(MAX_HISTORY_TEXT_CHARS) {
        Some((end, _)) => format!("{}\n[… shortened by Orca]", &text[..end]),
        None => text.to_string(),
    }
}

/// 道具の結果の本文（文字列、または text のブロックの並び）
fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks.iter().filter_map(|block| block["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// 作業フォルダからフォルダ名を作る（英数字以外を `-` にする。Claude Code と同じ変換）
fn encode_project_dir(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn summarize(path: &Path, updated: u64) -> Option<SessionSummary> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    let file = File::open(path).ok()?;
    let lines: Vec<String> = BufReader::new(file.take(MAX_BYTES_FOR_TITLE))
        .lines()
        .map_while(Result::ok)
        .take(MAX_LINES_FOR_TITLE)
        .collect();
    let title = title_from_lines(&lines)?;
    Some(SessionSummary { id, title, updated })
}

/// 最初の利用者の質問（コマンドの出力などの付属情報は除く）。なければ最後の質問
fn title_from_lines(lines: &[String]) -> Option<String> {
    let entries: Vec<Value> = lines.iter().filter_map(|line| serde_json::from_str(line).ok()).collect();
    let first_question = entries.iter().find_map(|entry| {
        if entry["type"] != "user" || entry["isMeta"] == true {
            return None;
        }
        question_text(&entry["message"]["content"])
    });
    let last_prompt = || {
        entries.iter().find_map(|entry| {
            (entry["type"] == "last-prompt").then(|| entry["lastPrompt"].as_str()).flatten().map(str::to_string)
        })
    };
    first_question.or_else(last_prompt).map(|title| shorten(&title))
}

fn question_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .find_map(|block| (block["type"] == "text").then(|| block["text"].as_str()).flatten())?
            .to_string(),
        _ => return None,
    };
    // `<command-name>` 等のタグで始まるもの（スラッシュコマンドの記録など）は質問として扱わない
    let text = text.trim();
    (!text.is_empty() && !text.starts_with('<')).then(|| text.to_string())
}

fn shorten(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= MAX_TITLE_CHARS {
        return line.to_string();
    }
    let short: String = line.chars().take(MAX_TITLE_CHARS).collect();
    format!("{short}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_working_folder_like_claude_code() {
        assert_eq!(
            encode_project_dir(Path::new(r"C:\Users\rinno\workspace\windowsApplications\MyTerminal")),
            "C--Users-rinno-workspace-windowsApplications-MyTerminal"
        );
    }

    #[test]
    fn titles_use_the_first_real_question() {
        let lines = [
            r#"{"type":"queue-operation","operation":"enqueue"}"#,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"Caveat: messages below..."}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/compact</command-name>"}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"なぜ nginx が 502 を返す？\n詳細は…"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}"#,
        ]
        .map(String::from);
        assert_eq!(title_from_lines(&lines).as_deref(), Some("なぜ nginx が 502 を返す？"));
    }

    #[test]
    fn falls_back_to_the_last_prompt_and_shortens_long_titles() {
        let lines = [r#"{"type":"last-prompt","lastPrompt":"ログを見て"}"#.to_string()];
        assert_eq!(title_from_lines(&lines).as_deref(), Some("ログを見て"));
        let long = "あ".repeat(100);
        assert_eq!(shorten(&long).chars().count(), MAX_TITLE_CHARS + 1);
        assert_eq!(title_from_lines(&["not json".to_string()]), None);
    }

    #[test]
    fn reads_questions_answers_and_tool_runs_from_the_history() {
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"ログを見て"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"Caveat"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"..."}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"確認します。"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"mcp__orca__tail_file","input":{"session":"a","path":"/var/log/messages"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ERROR disk full"}]},"toolUseResult":{}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"subagent"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"ディスクがいっぱいです。"}]}}"#,
            r#"{"type":"last-prompt","lastPrompt":"ログを見て"}"#,
        ]
        .map(String::from);
        assert_eq!(
            history_from_lines(lines.into_iter()),
            [
                HistoryEntry::User { text: "ログを見て".into() },
                HistoryEntry::Assistant { text: "確認します。".into() },
                HistoryEntry::ToolUse {
                    id: "t1".into(),
                    name: "mcp__orca__tail_file".into(),
                    input: serde_json::json!({ "session": "a", "path": "/var/log/messages" }),
                },
                HistoryEntry::ToolResult { id: "t1".into(), output: "ERROR disk full".into(), is_error: false },
                HistoryEntry::Assistant { text: "ディスクがいっぱいです。".into() },
            ]
        );
    }

    const SESSION_1: &str = "6fc1fc00-0320-492e-ba61-20ce626e05d3";
    const SESSION_2: &str = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

    #[test]
    fn session_ids_must_be_uuids() {
        assert!(is_session_id(SESSION_1));
        assert!(is_session_id(&SESSION_1.to_uppercase()));
        for odd in ["", "s-1", "--dangerously-skip-permissions", "../../secret", "6fc1fc00-0320-492e-ba61-20ce626e05d3x"] {
            assert!(!is_session_id(odd), "{odd}");
        }
        assert!(!is_session_id("-fc1fc00-0320-492e-ba61-20ce626e05d3"));
    }

    #[test]
    fn shortens_very_long_history_texts() {
        let long = "あ".repeat(MAX_HISTORY_TEXT_CHARS + 10);
        let line = format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{long}"}}]}}}}"#);
        let entries = history_from_lines(std::iter::once(line));
        let HistoryEntry::Assistant { text } = &entries[0] else { panic!("not an answer") };
        assert!(text.starts_with(&"あ".repeat(MAX_HISTORY_TEXT_CHARS)) && text.ends_with("[… shortened by Orca]"));
    }

    #[test]
    fn keeps_only_the_newest_history_entries() {
        let lines = (0..MAX_HISTORY_ENTRIES + 5)
            .map(|i| format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{i}"}}]}}}}"#));
        let entries = history_from_lines(lines);
        assert_eq!(entries.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(entries[0], HistoryEntry::Assistant { text: "5".into() });
    }

    #[test]
    fn loads_the_history_of_a_session_and_rejects_odd_ids() {
        let projects = std::env::temp_dir().join(format!("orca-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&projects);
        let dir = projects.join("C--work-app");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{SESSION_1}.jsonl")), r#"{"type":"user","message":{"content":"hello"}}"#).unwrap();
        let cwd = Path::new(r"c:\work\app");
        assert_eq!(load_history_in(&projects, cwd, SESSION_1).unwrap(), [HistoryEntry::User { text: "hello".into() }]);
        assert!(load_history_in(&projects, cwd, SESSION_2).is_err());
        assert!(load_history_in(&projects, cwd, "../../secret").is_err());
        let _ = std::fs::remove_dir_all(&projects);
    }

    #[test]
    fn lists_sessions_of_the_folder_newest_first_ignoring_case() {
        let projects = std::env::temp_dir().join(format!("orca-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&projects);
        let cwd = Path::new(r"C:\work\app");
        // ドライブ名が小文字のフォルダも同じ作業フォルダとして扱う
        let dir = projects.join("c--work-app");
        std::fs::create_dir_all(&dir).unwrap();
        let question = |text: &str| format!(r#"{{"type":"user","message":{{"role":"user","content":"{text}"}}}}"#);
        std::fs::write(dir.join(format!("{SESSION_1}.jsonl")), question("first")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.join(format!("{SESSION_2}.jsonl")), question("second")).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        // セッション ID の形でないファイル（`-` で始まる名前等）は一覧に出さない
        std::fs::write(dir.join("--dangerous.jsonl"), question("odd")).unwrap();
        std::fs::create_dir_all(projects.join("C--other")).unwrap();

        let sessions = list_sessions_in(&projects, cwd);
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, [SESSION_2, SESSION_1]);
        assert_eq!(sessions[0].title, "second");
        let _ = std::fs::remove_dir_all(&projects);
    }
}
