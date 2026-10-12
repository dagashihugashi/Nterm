//! 受信テキストからログ用の確定行を作る。
//!
//! ANSI エスケープシーケンスを除去し、BS・CR・カーソル移動・行内消去などの編集を
//! 「1行分の画面バッファ」に適用して、LF を受けた時点の内容を確定行として返す。
//! 行の結合・折り返しは行わない（改行位置は受信どおり）。

use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

const TAB_WIDTH: usize = 8;
/// カーソル移動・空白挿入で広げられる行幅の上限。
/// 不正なシーケンス（`CSI 65535 C` の連続など）で行バッファが際限なく大きくならないようにする。
/// 文字の出力で長くなった行はこの限りではない。
const MAX_MOVE_COLUMN: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Blank,
    /// 1文字分（結合文字が続く場合はまとめて保持する）
    Text(String),
    /// 全角文字の右半分
    WideTail,
}

/// 1行分の画面バッファ。列はターミナルの表示桁に対応する。
#[derive(Default)]
struct LineBuffer {
    cells: Vec<Cell>,
    cursor: usize,
}

impl LineBuffer {
    fn print(&mut self, c: char) {
        match c.width() {
            Some(0) | None => self.append_combining(c),
            Some(width) => self.put(c, width),
        }
    }

    fn put(&mut self, c: char, width: usize) {
        let col = self.cursor;
        self.ensure_len(col + width);
        for offset in 0..width {
            self.clear_cell(col + offset);
        }
        self.cells[col] = Cell::Text(c.to_string());
        if width == 2 {
            self.cells[col + 1] = Cell::WideTail;
        }
        self.cursor = col + width;
    }

    /// 結合文字は直前の文字に付ける（行頭なら単独の文字として扱う）
    fn append_combining(&mut self, c: char) {
        let previous = (0..self.cursor.min(self.cells.len()))
            .rev()
            .find(|&i| self.cells[i] != Cell::WideTail);
        match previous.map(|i| &mut self.cells[i]) {
            Some(Cell::Text(text)) => text.push(c),
            _ => self.put(c, 1),
        }
    }

    /// 指定列を空白にする。全角文字の片側だけが消える場合は、もう片側も空白にする
    fn clear_cell(&mut self, col: usize) {
        match self.cells.get(col) {
            Some(Cell::WideTail) if col > 0 => self.cells[col - 1] = Cell::Blank,
            Some(Cell::Text(_)) if self.cells.get(col + 1) == Some(&Cell::WideTail) => {
                self.cells[col + 1] = Cell::Blank
            }
            _ => {}
        }
        if let Some(cell) = self.cells.get_mut(col) {
            *cell = Cell::Blank;
        }
    }

    /// 列の境界で全角文字が分断される場合（境界が全角文字の右半分）、その文字を空白にする
    fn split_wide_char_at(&mut self, col: usize) {
        if self.cells.get(col) == Some(&Cell::WideTail) {
            self.clear_cell(col);
        }
    }

    fn ensure_len(&mut self, len: usize) {
        if self.cells.len() < len {
            self.cells.resize(len, Cell::Blank);
        }
    }

    /// カーソルを移動する（右方向は上限あり）
    fn move_to(&mut self, col: usize) {
        self.cursor = col.min(self.cells.len().max(MAX_MOVE_COLUMN));
    }

    fn backspace(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn tab(&mut self) {
        self.move_to((self.cursor / TAB_WIDTH + 1) * TAB_WIDTH);
    }

    /// CSI K（0: カーソルから行末 / 1: 行頭からカーソル / 2: 行全体）
    fn erase_in_line(&mut self, mode: u16) {
        match mode {
            0 => {
                self.split_wide_char_at(self.cursor);
                self.cells.truncate(self.cursor);
            }
            1 => {
                let end = (self.cursor + 1).min(self.cells.len());
                (0..end).for_each(|col| self.clear_cell(col));
            }
            2 => self.cells.clear(),
            _ => {}
        }
    }

    /// CSI P: カーソル位置から n 文字削除し、右側を左に詰める
    fn delete_chars(&mut self, count: usize) {
        if self.cursor >= self.cells.len() {
            return;
        }
        let end = (self.cursor + count).min(self.cells.len());
        self.split_wide_char_at(self.cursor);
        self.split_wide_char_at(end);
        self.cells.drain(self.cursor..end);
    }

    /// CSI @: カーソル位置に n 個の空白を挿入し、右側を右にずらす
    fn insert_blanks(&mut self, count: usize) {
        if self.cursor >= self.cells.len() {
            return;
        }
        self.split_wide_char_at(self.cursor);
        // 端末と同様、右端からあふれた分は捨てる（行幅は元の長さか上限の大きい方まで）
        let max_len = self.cells.len().max(MAX_MOVE_COLUMN);
        let blanks = std::iter::repeat_n(Cell::Blank, count.min(MAX_MOVE_COLUMN));
        self.cells.splice(self.cursor..self.cursor, blanks);
        self.cells.truncate(max_len);
    }

    /// CSI X: カーソル位置から n 文字を空白にする（詰めない）
    fn erase_chars(&mut self, count: usize) {
        let end = (self.cursor + count).min(self.cells.len());
        (self.cursor..end).for_each(|col| self.clear_cell(col));
    }

    /// 現在の内容を1行の文字列として取り出し、バッファを空にする（行末の空白は除く）
    fn take_line(&mut self) -> String {
        let line: String = self
            .cells
            .drain(..)
            .map(|cell| match cell {
                Cell::Blank => " ".to_string(),
                Cell::Text(text) => text,
                Cell::WideTail => String::new(),
            })
            .collect();
        self.cursor = 0;
        line.trim_end().to_string()
    }

    fn is_empty(&self) -> bool {
        self.cells.iter().all(|cell| *cell == Cell::Blank)
    }
}

/// vte のパース結果を行バッファへ適用し、確定行を集める
#[derive(Default)]
struct LinePerformer {
    line: LineBuffer,
    completed: Vec<String>,
}

/// CSI の引数を取り出す（省略・0 の場合は既定値）
fn param(params: &Params, index: usize, default: u16) -> u16 {
    params
        .iter()
        .nth(index)
        .and_then(|values| values.first().copied())
        .filter(|&value| value != 0)
        .unwrap_or(default)
}

impl Perform for LinePerformer {
    fn print(&mut self, c: char) {
        self.line.print(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\x08' => self.line.backspace(),
            b'\t' => self.line.tab(),
            b'\r' => self.line.move_to(0),
            // LF・VT・FF は改行として扱う
            b'\n' | b'\x0b' | b'\x0c' => self.completed.push(self.line.take_line()),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        // DEC private（`CSI ? ...`）等はモード設定なので行の内容には影響しない
        if ignore || !intermediates.is_empty() {
            return;
        }
        let count = usize::from(param(params, 0, 1));
        match action {
            'C' => self.line.move_to(self.line.cursor + count),
            'D' => self.line.move_to(self.line.cursor.saturating_sub(count)),
            'G' | '`' => self.line.move_to(count - 1),
            'K' => self.line.erase_in_line(param(params, 0, 0)),
            'P' => self.line.delete_chars(count),
            '@' => self.line.insert_blanks(count),
            'X' => self.line.erase_chars(count),
            // 行の指定は無視し、列だけ反映する（全画面アプリの内容は保証しない）
            'H' | 'f' => self.line.move_to(usize::from(param(params, 1, 1)) - 1),
            _ => {}
        }
    }
}

/// ストリーミングでログ用の確定行を作る。エスケープシーケンスがチャンク境界で分断されても扱える。
#[derive(Default)]
pub struct LogFormatter {
    parser: Parser,
    performer: LinePerformer,
}

impl LogFormatter {
    /// 受信テキストを処理し、新たに確定した行を返す。
    pub fn feed(&mut self, text: &str) -> Vec<String> {
        self.parser.advance(&mut self.performer, text.as_bytes());
        std::mem::take(&mut self.performer.completed)
    }

    /// 未確定の行（プロンプト等）を取り出す。ログ停止時に使う。空なら `None`。
    pub fn finish(&mut self) -> Option<String> {
        if self.performer.line.is_empty() {
            return None;
        }
        Some(self.performer.line.take_line())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(chunks: &[&str]) -> Vec<String> {
        let mut formatter = LogFormatter::default();
        chunks.iter().flat_map(|chunk| formatter.feed(chunk)).collect()
    }

    #[test]
    fn splits_lines_on_lf() {
        assert_eq!(lines(&["a\r\nb\r\n"]), ["a", "b"]);
    }

    #[test]
    fn keeps_empty_lines() {
        assert_eq!(lines(&["a\r\n\r\nb\r\n"]), ["a", "", "b"]);
    }

    #[test]
    fn removes_color_sequences() {
        assert_eq!(lines(&["\x1b[1;32mok\x1b[0m done\r\n"]), ["ok done"]);
    }

    #[test]
    fn handles_sequences_split_across_chunks() {
        assert_eq!(lines(&["\x1b[3", "1mred\x1b", "[0m\r\n"]), ["red"]);
    }

    #[test]
    fn applies_backspace_edit() {
        // `lss` と打って BS で 1 文字消した場合（bash は "\b \b" または "\b\x1b[K" を返す）
        assert_eq!(lines(&["$ lss\x08 \x08\r\n"]), ["$ ls"]);
        assert_eq!(lines(&["$ lss\x08\x1b[K\r\n"]), ["$ ls"]);
    }

    #[test]
    fn keeps_only_final_state_of_carriage_return_progress() {
        assert_eq!(lines(&["10%\r50%\r100%\r\n"]), ["100%"]);
    }

    #[test]
    fn shorter_overwrite_after_cr_keeps_remaining_chars_like_terminal() {
        assert_eq!(lines(&["abcdef\rXY\r\n"]), ["XYcdef"]);
    }

    #[test]
    fn applies_history_recall_rewrite() {
        // 上キーで履歴を呼び出すと、プロンプトの後ろへ戻って行末まで消してから書き直す
        assert_eq!(lines(&["$ ls\r\x1b[C\x1b[C\x1b[Kcat a.txt\r\n"]), ["$ cat a.txt"]);
    }

    #[test]
    fn applies_cursor_movement_and_insert() {
        // "ac" と打ってから左に1つ戻り "b" を挿入（readline は CSI @ で挿入する）
        assert_eq!(lines(&["ac\x1b[D\x1b[1@b\r\n"]), ["abc"]);
    }

    #[test]
    fn applies_delete_chars() {
        assert_eq!(lines(&["abcd\x1b[3D\x1b[2P\r\n"]), ["ad"]);
    }

    #[test]
    fn applies_erase_whole_line() {
        assert_eq!(lines(&["garbage\x1b[2K\rclean\r\n"]), ["clean"]);
    }

    #[test]
    fn expands_tab_to_next_stop() {
        assert_eq!(lines(&["a\tb\r\n"]), ["a       b"]);
    }

    #[test]
    fn trims_trailing_spaces() {
        assert_eq!(lines(&["abc   \r\n"]), ["abc"]);
    }

    #[test]
    fn ignores_title_and_mode_sequences() {
        assert_eq!(lines(&["\x1b]0;user@host: ~\x07\x1b[?2004h$ ls\r\n"]), ["$ ls"]);
    }

    #[test]
    fn handles_wide_chars_and_backspace() {
        // 全角文字は2桁。BS 2回で1文字分戻り、行末まで消す
        assert_eq!(lines(&["あい\x08\x08\x1b[K\r\n"]), ["あ"]);
    }

    #[test]
    fn overwriting_half_of_wide_char_clears_it() {
        assert_eq!(lines(&["あ\rx\r\n"]), ["x"]);
    }

    #[test]
    fn deleting_from_middle_of_wide_char_blanks_it() {
        // カーソルが「あ」の右半分にある状態で1桁削除すると、「あ」は空白になり b が詰まる
        assert_eq!(lines(&["xあb\x1b[2D\x1b[P\r\n"]), ["x b"]);
    }

    #[test]
    fn limits_line_growth_from_cursor_movement() {
        let mut formatter = LogFormatter::default();
        formatter.feed(&"\x1b[65535C".repeat(100));
        formatter.feed("x");
        assert_eq!(formatter.performer.line.cells.len(), MAX_MOVE_COLUMN + 1);

        formatter.feed(&"\x1b[1G\x1b[65535@".repeat(100));
        assert_eq!(formatter.performer.line.cells.len(), MAX_MOVE_COLUMN + 1);
    }

    #[test]
    fn keeps_combining_chars() {
        assert_eq!(lines(&["e\u{301}x\r\n"]), ["e\u{301}x"]);
    }

    #[test]
    fn finish_returns_pending_line() {
        let mut formatter = LogFormatter::default();
        assert_eq!(formatter.feed("done\r\n$ "), ["done"]);
        assert_eq!(formatter.finish().as_deref(), Some("$"));
        assert_eq!(formatter.finish(), None);
    }
}
