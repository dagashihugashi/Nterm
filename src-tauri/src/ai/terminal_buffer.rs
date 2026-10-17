//! ターミナルの直近の出力（AI の read_terminal ツール用）。
//!
//! ログと同じ整形（ANSI エスケープの除去・BS 等の行内編集の反映）を行い、確定した行を上限まで保持する。
//! vi / top 等の全画面アプリの画面は、ログと同じく正確には再現できない。

use std::collections::VecDeque;

use crate::log_formatter::LogFormatter;

/// 保持する行数の上限
pub const MAX_LINES: usize = 2_000;

pub struct TerminalBuffer {
    formatter: LogFormatter,
    lines: VecDeque<String>,
    max_lines: usize,
}

impl TerminalBuffer {
    pub fn new(max_lines: usize) -> Self {
        Self {
            formatter: LogFormatter::default(),
            lines: VecDeque::with_capacity(max_lines.min(MAX_LINES)),
            max_lines,
        }
    }

    /// 受信したテキスト（文字コード変換済み）を追加する
    pub fn feed(&mut self, text: &str) {
        for line in self.formatter.feed(text) {
            if self.lines.len() == self.max_lines {
                self.lines.pop_front();
            }
            self.lines.push_back(line);
        }
    }

    /// 直近 `count` 行を古い順に返す。未確定の行（プロンプト等）があれば最後に含める
    pub fn recent(&self, count: usize) -> Vec<String> {
        let current = self.formatter.current_line();
        let has_current = !current.trim().is_empty();
        let from_lines = count.saturating_sub(usize::from(has_current));
        let skip = self.lines.len().saturating_sub(from_lines);
        let mut recent: Vec<String> = self.lines.iter().skip(skip).cloned().collect();
        if has_current && count > 0 {
            recent.push(current);
        }
        recent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_formatted_lines_without_escape_sequences() {
        let mut buffer = TerminalBuffer::new(10);
        buffer.feed("\x1b[31merror\x1b[0m: disk full\r\nabd\x08c\r\n");
        assert_eq!(buffer.recent(10), ["error: disk full", "abc"]);
    }

    #[test]
    fn includes_the_current_prompt_line() {
        let mut buffer = TerminalBuffer::new(10);
        buffer.feed("line 1\r\n[root@web01 ~]# ");
        assert_eq!(buffer.recent(2), ["line 1", "[root@web01 ~]#"]);
        // 件数にはプロンプトの行も含む
        assert_eq!(buffer.recent(1), ["[root@web01 ~]#"]);
    }

    #[test]
    fn drops_the_oldest_lines_over_the_limit() {
        let mut buffer = TerminalBuffer::new(3);
        buffer.feed("1\r\n2\r\n3\r\n4\r\n5\r\n");
        assert_eq!(buffer.recent(10), ["3", "4", "5"]);
        assert_eq!(buffer.recent(2), ["4", "5"]);
    }

    #[test]
    fn returns_nothing_for_zero_lines() {
        let mut buffer = TerminalBuffer::new(3);
        buffer.feed("1\r\n$ ");
        assert!(buffer.recent(0).is_empty());
    }
}
