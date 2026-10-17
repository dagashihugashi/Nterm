//! AI の接続先（プロバイダ）の決定（AI.md 8.0a・8.0）。
//!
//! Bedrock の設定は Claude Code の設定ファイル（`%USERPROFILE%\.claude\settings.json` の `env`）から読む。
//! 同じ名前の環境変数があればそちらを優先する。Bedrock が有効でなければ Anthropic API を使い、
//! API キーは環境変数・settings.json・Orca で保存したキー（credentials.json、DPAPI）の順に探す。
//! Orca は settings.json を読むだけで、書き換えない。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Orca で保存する Anthropic の API キーのキー（credentials.json。サーバの id と重ならない）
pub const ANTHROPIC_KEY_ID: &str = "#ai:anthropic-api-key";
/// Anthropic API で ANTHROPIC_MODEL がない場合のモデル
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5-5";

/// Claude Code の `ANTHROPIC_MODEL` に書ける別名（モデル ID ではないため Bedrock では使えない）
const MODEL_ALIASES: &[&str] = &["default", "sonnet", "opus", "haiku", "opusplan", "sonnet[1m]", "opus[1m]"];

#[derive(Clone, PartialEq)]
pub enum Provider {
    Bedrock { region: String, token: String, model: String },
    Anthropic { api_key: String, model: String },
}

impl Provider {
    /// 画面に表示する説明（キー・トークンは含めない）
    pub fn describe(&self) -> String {
        match self {
            Self::Bedrock { region, model, .. } => format!("Amazon Bedrock ({region}, {model})"),
            Self::Anthropic { model, .. } => format!("Anthropic API ({model})"),
        }
    }
}

// トークン・キーを誤ってログやエラーに出さない
impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

/// 接続先を決める。`process_env` は環境変数の読み取り（テストで差し替える）
pub fn resolve(
    process_env: &dyn Fn(&str) -> Option<String>,
    settings_env: &HashMap<String, String>,
    saved_api_key: Option<String>,
) -> Result<Provider, String> {
    let get = |key: &str| {
        process_env(key)
            .or_else(|| settings_env.get(key).cloned())
            .filter(|value| !value.trim().is_empty())
    };
    if get("CLAUDE_CODE_USE_BEDROCK").is_some_and(|value| is_truthy(&value)) {
        let region = get("AWS_REGION")
            .or_else(|| get("AWS_DEFAULT_REGION"))
            .ok_or("Bedrock is enabled but AWS_REGION is not set")?;
        let token = get("AWS_BEARER_TOKEN_BEDROCK")
            .ok_or("Bedrock is enabled but AWS_BEARER_TOKEN_BEDROCK is not set (only API key authentication is supported)")?;
        let model = model_id(get("ANTHROPIC_MODEL"))
            .or_else(|| model_id(get("ANTHROPIC_DEFAULT_SONNET_MODEL")))
            .ok_or("set a Bedrock model ID (e.g. an inference profile ID) in ANTHROPIC_MODEL or ANTHROPIC_DEFAULT_SONNET_MODEL")?;
        return Ok(Provider::Bedrock { region, token, model });
    }
    let api_key = get("ANTHROPIC_API_KEY")
        .or(saved_api_key.filter(|key| !key.trim().is_empty()))
        .ok_or("no AI provider is configured: enter an Anthropic API key, or enable Bedrock in Claude Code's settings.json")?;
    let model = model_id(get("ANTHROPIC_MODEL")).unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.to_string());
    Ok(Provider::Anthropic { api_key, model })
}

fn is_truthy(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

/// 別名（sonnet 等）を除いたモデル ID
fn model_id(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !MODEL_ALIASES.contains(&value.to_ascii_lowercase().as_str()))
}

/// Claude Code の設定ファイルの場所
pub fn claude_settings_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".claude").join("settings.json"))
}

/// settings.json の `env`（文字列の値のみ）。ファイルがなければ空
pub fn read_settings_env(path: &Path) -> Result<HashMap<String, String>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(format!("failed to read {}: {e}", path.display())),
    };
    parse_settings_env(&text).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

fn parse_settings_env(text: &str) -> Result<HashMap<String, String>, String> {
    let settings: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Some(env) = settings.get("env").and_then(Value::as_object) else {
        return Ok(HashMap::new());
    };
    Ok(env
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    const BEDROCK: &[(&str, &str)] = &[
        ("CLAUDE_CODE_USE_BEDROCK", "1"),
        ("AWS_REGION", "ap-northeast-1"),
        ("AWS_BEARER_TOKEN_BEDROCK", "secret-token"),
        ("ANTHROPIC_MODEL", "apac.anthropic.claude-sonnet-x"),
    ];

    #[test]
    fn uses_bedrock_from_claude_code_settings() {
        let provider = resolve(&no_env, &settings(BEDROCK), None).unwrap();
        assert_eq!(
            provider,
            Provider::Bedrock {
                region: "ap-northeast-1".into(),
                token: "secret-token".into(),
                model: "apac.anthropic.claude-sonnet-x".into()
            }
        );
        assert_eq!(provider.describe(), "Amazon Bedrock (ap-northeast-1, apac.anthropic.claude-sonnet-x)");
    }

    #[test]
    fn environment_variables_take_precedence_over_settings() {
        let env = |key: &str| (key == "AWS_REGION").then(|| "us-east-1".to_string());
        let provider = resolve(&env, &settings(BEDROCK), None).unwrap();
        assert!(matches!(provider, Provider::Bedrock { region, .. } if region == "us-east-1"));
    }

    #[test]
    fn bedrock_reports_missing_settings() {
        let without_token = settings(&[("CLAUDE_CODE_USE_BEDROCK", "true"), ("AWS_REGION", "us-east-1")]);
        let error = resolve(&no_env, &without_token, None).unwrap_err();
        assert!(error.contains("AWS_BEARER_TOKEN_BEDROCK"));
    }

    #[test]
    fn bedrock_does_not_accept_model_aliases() {
        let mut pairs = settings(BEDROCK);
        pairs.insert("ANTHROPIC_MODEL".into(), "sonnet".into());
        assert!(resolve(&no_env, &pairs, None).unwrap_err().contains("model ID"));
        pairs.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), "global.anthropic.claude-y".into());
        let provider = resolve(&no_env, &pairs, None).unwrap();
        assert!(matches!(provider, Provider::Bedrock { model, .. } if model == "global.anthropic.claude-y"));
    }

    #[test]
    fn uses_the_anthropic_api_with_a_saved_key() {
        let provider = resolve(&no_env, &HashMap::new(), Some("sk-saved".into())).unwrap();
        assert_eq!(
            provider,
            Provider::Anthropic { api_key: "sk-saved".into(), model: DEFAULT_ANTHROPIC_MODEL.into() }
        );
        // 環境変数のキーは保存したキーより優先する
        let env = |key: &str| (key == "ANTHROPIC_API_KEY").then(|| "sk-env".to_string());
        let provider = resolve(&env, &HashMap::new(), Some("sk-saved".into())).unwrap();
        assert!(matches!(provider, Provider::Anthropic { api_key, .. } if api_key == "sk-env"));
    }

    #[test]
    fn reports_when_nothing_is_configured() {
        assert!(resolve(&no_env, &HashMap::new(), None).is_err());
        assert!(resolve(&no_env, &settings(&[("CLAUDE_CODE_USE_BEDROCK", "0")]), Some(" ".into())).is_err());
    }

    #[test]
    fn debug_output_hides_secrets() {
        let provider = resolve(&no_env, &settings(BEDROCK), None).unwrap();
        assert!(!format!("{provider:?}").contains("secret-token"));
    }

    #[test]
    fn parses_only_string_values_of_env() {
        let env = parse_settings_env(r#"{ "model": "opus", "env": { "AWS_REGION": "us-west-2", "N": 1 } }"#).unwrap();
        assert_eq!(env, settings(&[("AWS_REGION", "us-west-2")]));
        assert!(parse_settings_env("{}").unwrap().is_empty());
        assert!(parse_settings_env("not json").is_err());
    }

    #[test]
    fn a_missing_settings_file_is_empty() {
        let path = std::env::temp_dir().join("orca-no-such-settings.json");
        assert!(read_settings_env(&path).unwrap().is_empty());
    }
}
