//! クリップボードの読み書き（Unicode テキスト）。
//!
//! Windows のクリップボードは同時に1つのアプリしか開けず、コピー直後のアプリ（Excel・Teams 等）が
//! 一瞬占有していることがある。1回で諦めず、短い間隔で再試行してから失敗を返す。

use std::thread;
use std::time::Duration;

use clipboard_win::{formats, Clipboard};

/// 再試行の間隔と回数（最大で約 0.5 秒待つ）
const RETRY_INTERVAL: Duration = Duration::from_millis(25);
const OPEN_ATTEMPTS: u32 = 20;
const BUSY_MESSAGE: &str = "Clipboard is busy. Try again.";

/// クリップボードを開く。他のアプリが占有している間は再試行する。
/// 戻り値を drop すると閉じる。
fn open() -> Result<Clipboard, String> {
    for _ in 1..OPEN_ATTEMPTS {
        if let Ok(clipboard) = Clipboard::new() {
            return Ok(clipboard);
        }
        thread::sleep(RETRY_INTERVAL);
    }
    Clipboard::new().map_err(|_| BUSY_MESSAGE.to_string())
}

/// テキストを読み込む。テキストがない（空・画像のみ等）場合は `None`。
pub fn read_text() -> Result<Option<String>, String> {
    let _clipboard = open()?;
    if !clipboard_win::is_format_avail(formats::CF_UNICODETEXT) {
        return Ok(None);
    }
    clipboard_win::get(formats::Unicode)
        .map(|text: String| Some(text).filter(|t| !t.is_empty()))
        .map_err(|e| format!("failed to read the clipboard: {e}"))
}

/// テキストを書き込む。他のアプリへ貼り付けやすいよう、改行は CRLF にそろえる。
pub fn write_text(text: &str) -> Result<(), String> {
    let _clipboard = open()?;
    clipboard_win::set(formats::Unicode, to_crlf(text))
        .map_err(|e| format!("failed to write to the clipboard: {e}"))
}

/// 改行（CRLF / LF / CR）を CRLF にそろえる
fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_line_breaks_to_crlf() {
        assert_eq!(to_crlf("a\nb\r\nc\rd"), "a\r\nb\r\nc\r\nd");
    }

    #[test]
    fn keeps_text_without_line_breaks() {
        assert_eq!(to_crlf("日本語 text"), "日本語 text");
    }
}
