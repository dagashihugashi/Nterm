//! AI の監査ログ（AI.md 5 章）：質問・ツールの承認/拒否・ツールの結果・回答・エラーをすべて記録する。
//!
//! 保存先は `<設定フォルダ>/logs/ai-audit-YYYY-MM-DD.jsonl`（1 行 1 件の JSON）。
//! API キー・トークンは記録しない（記録するのは画面に表示する内容と同じ範囲）。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Local, NaiveDateTime};
use serde_json::{json, Value};

const LOG_DIR: &str = "logs";

pub struct AuditLog {
    dir: PathBuf,
    /// 複数のスレッドから書き込んでも行が混ざらないようにする
    lock: Mutex<()>,
}

impl AuditLog {
    /// `config_dir` は servers.json と同じ設定フォルダ
    pub fn new(config_dir: &Path) -> Self {
        Self { dir: config_dir.join(LOG_DIR), lock: Mutex::new(()) }
    }

    /// 1 件記録する。`kind` は question / approval / tool_result / answer / error
    pub fn record(&self, kind: &str, fields: Value) -> Result<(), String> {
        self.record_at(Local::now().naive_local(), kind, fields)
    }

    fn record_at(&self, at: NaiveDateTime, kind: &str, fields: Value) -> Result<(), String> {
        let mut entry = json!({ "time": at.format("%Y-%m-%dT%H:%M:%S%.3f").to_string(), "kind": kind });
        if let (Some(entry), Value::Object(fields)) = (entry.as_object_mut(), fields) {
            entry.extend(fields);
        }
        let path = self.dir.join(format!("ai-audit-{}.jsonl", at.format("%Y-%m-%d")));
        let _guard = self.lock.lock().map_err(|_| "the AI audit log is unavailable".to_string())?;
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("failed to create {}: {e}", self.dir.display()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("failed to open {}: {e}", path.display()))?;
        writeln!(file, "{entry}").map_err(|e| format!("failed to write {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orca-audit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_milli_opt(9, 30, 0, 125).unwrap()
    }

    #[test]
    fn appends_one_json_line_per_entry_in_a_daily_file() {
        let dir = temp_dir("daily");
        let log = AuditLog::new(&dir);
        log.record_at(at(17), "question", json!({ "text": "なぜ止まった？" })).unwrap();
        log.record_at(at(17), "approval", json!({ "tool": "tail_file", "decision": "allow" })).unwrap();
        log.record_at(at(18), "answer", json!({ "text": "next day" })).unwrap();

        let text = std::fs::read_to_string(dir.join("logs").join("ai-audit-2026-10-17.jsonl")).unwrap();
        let lines: Vec<Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["time"], "2026-10-17T09:30:00.125");
        assert_eq!(lines[0]["kind"], "question");
        assert_eq!(lines[0]["text"], "なぜ止まった？");
        assert_eq!(lines[1]["decision"], "allow");
        assert!(dir.join("logs").join("ai-audit-2026-10-18.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_line_output_stays_on_one_line() {
        let dir = temp_dir("multiline");
        let log = AuditLog::new(&dir);
        log.record_at(at(17), "tool_result", json!({ "output": "line1\nline2" })).unwrap();
        let text = std::fs::read_to_string(dir.join("logs").join("ai-audit-2026-10-17.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
