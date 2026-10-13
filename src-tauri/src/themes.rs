//! カラーテーマ（themes.json）。
//!
//! テーマは「キー → 色（#rgb / #rrggbb / #rrggbbaa）」の対応。省略したキーは `default-dark` から引き継ぐ。
//! themes.json がなければ、初回に既定のテーマを書き出す（利用者が編集・追加する見本として）。
//! themes.json が壊れている場合は、組み込みのテーマだけで動作し、警告を返す。

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::config_dir;

const THEMES_FILE: &str = "themes.json";
pub const DEFAULT_THEME_NAME: &str = "default-dark";
const MAX_THEME_NAME_LENGTH: usize = 50;

/// 設定できるキー（ターミナルの色は xterm.js の ITheme と同じ名前）
const ALLOWED_KEYS: &[&str] = &[
    // ターミナル
    "background",
    "foreground",
    "cursor",
    "selectionBackground",
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightMagenta",
    "brightCyan",
    "brightWhite",
    // アプリの画面
    "sidebarBackground",
    "barBackground",
    "barForeground",
];

/// 組み込みのテーマ（themes.json の初期内容）。`default-dark` はすべてのキーを持つ。
const DEFAULT_THEMES_JSON: &str = r##"{
  "default-dark": {
    "background": "#1e1e1e",
    "foreground": "#d4d4d4",
    "sidebarBackground": "#252526",
    "barBackground": "#007acc",
    "barForeground": "#ffffff",
    "cursor": "#ffffff",
    "selectionBackground": "#264f78",
    "black": "#000000",
    "red": "#cd3131",
    "green": "#0dbc79",
    "yellow": "#e5e510",
    "blue": "#2472c8",
    "magenta": "#bc3fbc",
    "cyan": "#11a8cd",
    "white": "#e5e5e5",
    "brightBlack": "#666666",
    "brightRed": "#f14c4c",
    "brightGreen": "#23d18b",
    "brightYellow": "#f5f543",
    "brightBlue": "#3b8eea",
    "brightMagenta": "#d670d6",
    "brightCyan": "#29b8db",
    "brightWhite": "#e5e5e5"
  },
  "danger-red": {
    "background": "#241414",
    "foreground": "#ffd2d2",
    "sidebarBackground": "#2b1818",
    "barBackground": "#a1260d",
    "barForeground": "#ffffff",
    "cursor": "#ff5555",
    "selectionBackground": "#661111"
  }
}
"##;

type ThemeColors = BTreeMap<String, String>;
type ThemeMap = BTreeMap<String, ThemeColors>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedThemes {
    /// テーマ名 → すべてのキーがそろった色の対応
    pub themes: ThemeMap,
    /// themes.json を読めなかった場合など、利用者に伝える問題
    pub warning: Option<String>,
}

fn is_hex_color(value: &str) -> bool {
    value
        .strip_prefix('#')
        .is_some_and(|hex| matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// themes.json を解析して検証する（キー・色の形式・テーマ名）
fn parse_themes(json: &str) -> Result<ThemeMap, String> {
    let themes: ThemeMap = serde_json::from_str(json).map_err(|e| format!("invalid format: {e}"))?;
    for (name, colors) in &themes {
        if name.trim().is_empty() || name.len() > MAX_THEME_NAME_LENGTH {
            return Err(format!("invalid theme name: \"{name}\""));
        }
        for (key, value) in colors {
            if !ALLOWED_KEYS.contains(&key.as_str()) {
                return Err(format!("theme \"{name}\": unknown key \"{key}\""));
            }
            if !is_hex_color(value) {
                return Err(format!("theme \"{name}\": {key} must be a color like #1e1e1e (got \"{value}\")"));
            }
        }
    }
    Ok(themes)
}

fn built_in_themes() -> ThemeMap {
    // 組み込みの JSON はテストで検証済みのため、失敗しない
    parse_themes(DEFAULT_THEMES_JSON).unwrap_or_default()
}

/// 組み込みのテーマに利用者のテーマを重ね、各テーマの省略したキーを `default-dark` で補う
fn resolve_themes(built_in: ThemeMap, user: ThemeMap) -> ThemeMap {
    let mut raw = built_in;
    let mut base = raw.get(DEFAULT_THEME_NAME).cloned().unwrap_or_default();
    if let Some(user_default) = user.get(DEFAULT_THEME_NAME) {
        base.extend(user_default.clone());
    }
    raw.extend(user);
    raw.into_iter()
        .map(|(name, colors)| {
            let mut merged = base.clone();
            merged.extend(colors);
            (name, merged)
        })
        .collect()
}

/// themes.json を読み込む。存在しなければ既定の内容で作成する。
pub fn load_themes_from(path: &Path) -> LoadedThemes {
    let built_in = built_in_themes();
    let (user, warning) = match fs::read_to_string(path) {
        Ok(content) => match parse_themes(&content) {
            Ok(user) => (user, None),
            Err(e) => (ThemeMap::new(), Some(format!("{}: {e} (using the built-in themes)", path.display()))),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let warning = fs::write(path, DEFAULT_THEMES_JSON)
                .err()
                .map(|e| format!("failed to create {}: {e}", path.display()));
            (ThemeMap::new(), warning)
        }
        Err(e) => (ThemeMap::new(), Some(format!("failed to read {}: {e}", path.display()))),
    };
    LoadedThemes { themes: resolve_themes(built_in, user), warning }
}

fn themes_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join(THEMES_FILE))
}

#[tauri::command]
pub fn load_themes() -> Result<LoadedThemes, String> {
    Ok(load_themes_from(&themes_path()?))
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa` を (r, g, b) にする（透明度は無視）
fn parse_rgb(value: &str) -> Result<(u8, u8, u8), String> {
    if !is_hex_color(value) {
        return Err(format!("invalid color: {value}"));
    }
    let hex = &value[1..];
    let channel = |start: usize, len: usize| -> u8 {
        let digits = &hex[start..start + len];
        let parsed = u8::from_str_radix(digits, 16).unwrap_or(0);
        if len == 1 { parsed * 17 } else { parsed }
    };
    Ok(if hex.len() == 3 {
        (channel(0, 1), channel(1, 1), channel(2, 1))
    } else {
        (channel(0, 2), channel(2, 2), channel(4, 2))
    })
}

/// タイトルバー（最小化・最大化・閉じるボタンのあるバー）の色を変える。
/// Windows 11 のみ対応。対応していない場合は false を返す（エラーにはしない）。
#[tauri::command]
pub fn set_title_bar_color(
    window: tauri::WebviewWindow,
    background: String,
    foreground: String,
) -> Result<bool, String> {
    let background = parse_rgb(&background)?;
    let foreground = parse_rgb(&foreground)?;
    #[cfg(windows)]
    {
        let hwnd = window.hwnd().map_err(|e| format!("failed to get the window handle: {e}"))?;
        Ok(title_bar::apply(hwnd.0, background, foreground))
    }
    #[cfg(not(windows))]
    {
        let _ = (window, background, foreground);
        Ok(false)
    }
}

#[cfg(windows)]
mod title_bar {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWINDOWATTRIBUTE,
    };

    fn colorref((r, g, b): (u8, u8, u8)) -> COLORREF {
        COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
    }

    fn set_color(hwnd: HWND, attribute: DWMWINDOWATTRIBUTE, color: COLORREF) -> bool {
        // SAFETY: hwnd は Tauri から取得した有効なウィンドウハンドル。
        // color は呼び出し中有効な COLORREF で、サイズを正しく渡している。
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute,
                (&color as *const COLORREF).cast::<c_void>(),
                size_of::<COLORREF>() as u32,
            )
        }
        .is_ok()
    }

    /// Windows 11（build 22000 以降）でのみ成功する。Windows 10 では失敗して false を返す。
    pub fn apply(raw_hwnd: *mut c_void, background: (u8, u8, u8), foreground: (u8, u8, u8)) -> bool {
        // Tauri が使う windows クレートとバージョンが異なり得るため、生のポインタから作り直す
        let hwnd = HWND(raw_hwnd);
        let caption = set_color(hwnd, DWMWA_CAPTION_COLOR, colorref(background));
        let text = set_color(hwnd, DWMWA_TEXT_COLOR, colorref(foreground));
        caption && text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nterm-themes-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join(THEMES_FILE)
    }

    fn cleanup(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn built_in_themes_are_valid_and_complete() {
        let themes = resolve_themes(built_in_themes(), ThemeMap::new());
        for name in [DEFAULT_THEME_NAME, "danger-red"] {
            let colors = &themes[name];
            for key in ALLOWED_KEYS {
                assert!(colors.contains_key(*key), "{name} lacks {key}");
            }
        }
    }

    #[test]
    fn omitted_keys_inherit_from_default_dark() {
        let themes = resolve_themes(built_in_themes(), ThemeMap::new());
        let danger = &themes["danger-red"];
        assert_eq!(danger["background"], "#241414");
        assert_eq!(danger["red"], "#cd3131");
    }

    #[test]
    fn user_themes_are_added_and_override_built_in() {
        let user = parse_themes(
            r##"{ "light": { "background": "#ffffff", "foreground": "#000000" },
                  "danger-red": { "background": "#300000" },
                  "default-dark": { "barBackground": "#333333" } }"##,
        )
        .unwrap();
        let themes = resolve_themes(built_in_themes(), user);
        assert_eq!(themes["light"]["background"], "#ffffff");
        // 利用者が変えた default-dark の値は、他のテーマの省略したキーにも反映される
        assert_eq!(themes["light"]["barBackground"], "#333333");
        // 利用者が定義したテーマは組み込みの同名テーマを置き換える（省略したキーは default-dark から）
        assert_eq!(themes["danger-red"]["background"], "#300000");
        assert_eq!(themes["danger-red"]["barBackground"], "#333333");
    }

    #[test]
    fn rejects_unknown_keys_and_invalid_colors() {
        assert!(parse_themes(r##"{ "x": { "backgrond": "#000000" } }"##).unwrap_err().contains("unknown key"));
        assert!(parse_themes(r##"{ "x": { "background": "red" } }"##).is_err());
        assert!(parse_themes(r##"{ "x": { "background": "#12345" } }"##).is_err());
        assert!(parse_themes(r##"{ "x": { "background": "#000;color:red" } }"##).is_err());
    }

    #[test]
    fn creates_default_file_when_missing() {
        let path = temp_path("missing");
        let loaded = load_themes_from(&path);
        let written = fs::read_to_string(&path);
        cleanup(&path);
        assert!(loaded.warning.is_none());
        assert!(loaded.themes.contains_key("danger-red"));
        assert_eq!(written.unwrap(), DEFAULT_THEMES_JSON);
    }

    #[test]
    fn falls_back_to_built_in_themes_when_file_is_broken() {
        let path = temp_path("broken");
        fs::write(&path, "{ not json").unwrap();
        let loaded = load_themes_from(&path);
        cleanup(&path);
        assert!(loaded.warning.unwrap().contains("using the built-in themes"));
        assert!(loaded.themes.contains_key(DEFAULT_THEME_NAME));
    }

    #[test]
    fn parses_short_and_long_hex_colors() {
        assert_eq!(parse_rgb("#007acc").unwrap(), (0x00, 0x7a, 0xcc));
        assert_eq!(parse_rgb("#fff").unwrap(), (255, 255, 255));
        assert_eq!(parse_rgb("#a1260dff").unwrap(), (0xa1, 0x26, 0x0d));
        assert!(parse_rgb("blue").is_err());
    }
}
