//! 作業フォルダの CLAUDE.md（プロジェクトの指示）を読む（AI あり版のみ。AI.md 12.2）。
//!
//! Orca は Claude Code を `--setting-sources user` で起動し、作業フォルダの `.claude/settings.json` を読ませない
//! （hooks・apiKeyHelper 等で PC 上のコマンドを動かせるうえ、`-p` ではフォルダの信頼の確認が省かれるため）。
//! その設定で読まれなくなる作業フォルダの CLAUDE.md は、文章だけを Orca が読み、追加のシステムプロンプトとして渡す
//! （文章なので PC 上で何も動かない。`@ファイル名` の取り込みは行わない）。

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// 読むファイル（Claude Code がプロジェクトの指示として読む場所）
const FILES: [&str; 3] = ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"];
/// 1 つのファイルから読む最大バイト数
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// 渡す文章の上限（Windows の起動の引数は全体で約 32,000 文字まで。UTF-16 の長さで数え、
/// 引数に入れるときにエスケープされる `"` と `\` は 2 文字として数える）
const MAX_PROMPT_UNITS: usize = 16_000;
const OMITTED_NOTICE: &str = "\n\n[Orca omitted the rest of the project instructions because they are too long.]";

/// 作業フォルダの CLAUDE.md の文章（なければ None）
pub fn read(cwd: &Path) -> Option<String> {
    let sections: Vec<String> = FILES
        .iter()
        .filter_map(|name| {
            let text = read_limited(&cwd.join(name))?;
            let text = text.trim();
            (!text.is_empty()).then(|| {
                format!("Contents of {name} in the working folder (project instructions, provided by Orca):\n\n{text}")
            })
        })
        .collect();
    (!sections.is_empty()).then(|| limit(&sections.join("\n\n")))
}

fn read_limited(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// 起動の引数に収まるよう、長すぎる文章は先頭だけにする
fn limit(text: &str) -> String {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        units += c.len_utf16() + usize::from(matches!(c, '"' | '\\'));
        if units > MAX_PROMPT_UNITS {
            return format!("{}{OMITTED_NOTICE}", &text[..index]);
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("orca-instructions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    #[test]
    fn reads_the_claude_md_files_of_the_working_folder() {
        let dir = temp_dir("read");
        std::fs::write(dir.join("CLAUDE.md"), "Use Japanese.\n").unwrap();
        std::fs::write(dir.join(".claude").join("CLAUDE.md"), "  ").unwrap();
        std::fs::write(dir.join("CLAUDE.local.md"), "My server is web01.").unwrap();

        let text = read(&dir).unwrap();
        assert!(text.starts_with("Contents of CLAUDE.md in the working folder"));
        assert!(text.contains("Use Japanese.") && text.contains("My server is web01."));
        // 空のファイルは載せない
        assert!(!text.contains(".claude/CLAUDE.md"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn returns_nothing_without_claude_md() {
        let dir = temp_dir("none");
        assert_eq!(read(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_instructions_are_shortened_to_fit_the_command_line() {
        assert_eq!(limit("short"), "short");
        let long = "あ".repeat(MAX_PROMPT_UNITS + 100);
        let shortened = limit(&long);
        assert!(shortened.ends_with(OMITTED_NOTICE));
        assert_eq!(shortened.trim_end_matches(OMITTED_NOTICE).chars().count(), MAX_PROMPT_UNITS);
        // エスケープされる文字は 2 文字として数える
        let quotes = "\"".repeat(MAX_PROMPT_UNITS);
        assert_eq!(limit(&quotes).trim_end_matches(OMITTED_NOTICE).len(), MAX_PROMPT_UNITS / 2);
    }
}
