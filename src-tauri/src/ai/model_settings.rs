//! Claude Code の設定ファイル（settings.json）から、モデルの選択肢を読み取る（AI あり版のみ）。
//!
//! 読むのはモデルに関わる値（`model`、`availableModels`、`env` のモデルの環境変数）だけで、
//! トークン等のほかの値はフロントエンドへ渡さない。
//! 設定は Claude Code と同じく、ユーザ → プロジェクト → プロジェクト（ローカル）→ 管理者の順に上書きする。

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use super::claude_cli::is_valid_model;

/// Claude Code の既定のモデルを決める環境変数
const MODEL_ENV: &str = "ANTHROPIC_MODEL";
/// 別名（sonnet / opus / haiku）と、その実際のモデルを決める環境変数
const ALIASES: [(&str, &str, &str); 3] = [
    ("sonnet", "Sonnet", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
    ("opus", "Opus", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
    ("haiku", "Haiku", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
];
/// 管理者の設定（Windows）。新しい場所と古い場所の両方を見る
const MANAGED_SETTINGS: [&str; 2] = [
    r"C:\Program Files\ClaudeCode\managed-settings.json",
    r"C:\ProgramData\ClaudeCode\managed-settings.json",
];

/// モデルの選択肢の 1 件
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    /// `--model` に渡す値
    pub value: String,
    pub label: String,
    /// 別名の実際のモデル（設定で決まっている場合）
    pub detail: Option<String>,
}

/// モデルの選択肢（フロントエンドの ModelPicker に渡す）
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOptions {
    /// 何も選ばないときに Claude Code が使うモデル（設定がなければ None）
    pub default_model: Option<String>,
    pub options: Vec<ModelOption>,
    /// 読み込んだ設定ファイル
    pub sources: Vec<String>,
    /// 読めなかった設定ファイル（選択肢は読めたファイルだけで作る）
    pub warnings: Vec<String>,
}

/// モデルに関わる設定（設定ファイルを重ねた結果）
#[derive(Debug, Default, PartialEq)]
struct ModelSettings {
    model: Option<String>,
    available_models: Option<Vec<String>>,
    /// 設定ファイルの `env` のうちモデルの環境変数
    env: Vec<(String, String)>,
}

impl ModelSettings {
    /// 後の設定で上書きする（`env` は変数ごと）
    fn merged(self, settings: &Value) -> Self {
        let model = settings.get("model").and_then(Value::as_str).map(str::to_owned).or(self.model);
        let available_models = settings
            .get("availableModels")
            .and_then(Value::as_array)
            .map(|models| models.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .or(self.available_models);
        let overrides: Vec<(String, String)> = model_env_names()
            .filter_map(|name| {
                let value = settings.get("env")?.get(name)?.as_str()?;
                Some((name.to_owned(), value.to_owned()))
            })
            .collect();
        let kept: Vec<(String, String)> = self
            .env
            .into_iter()
            .filter(|(name, _)| !overrides.iter().any(|(other, _)| other == name))
            .collect();
        let env = kept.into_iter().chain(overrides).collect();
        Self { model, available_models, env }
    }

    /// 設定ファイルの `env` → Orca を起動したときの環境変数 の順に探す
    fn env_value(&self, name: &str, process_env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        self.env
            .iter()
            .find(|(other, _)| other == name)
            .map(|(_, value)| value.clone())
            .or_else(|| process_env(name))
            .filter(|value| is_valid_model(value))
    }
}

fn model_env_names() -> impl Iterator<Item = &'static str> {
    std::iter::once(MODEL_ENV).chain(ALIASES.iter().map(|(_, _, env)| *env))
}

/// 設定を選択肢にする（`availableModels` があればその中から、なければ別名）
fn model_options(settings: &ModelSettings, process_env: &dyn Fn(&str) -> Option<String>) -> Vec<ModelOption> {
    let alias_option = |value: &str| {
        ALIASES.iter().find(|(alias, _, _)| *alias == value).map(|(alias, label, env)| ModelOption {
            value: (*alias).to_owned(),
            label: (*label).to_owned(),
            detail: settings.env_value(env, process_env),
        })
    };
    match &settings.available_models {
        Some(models) => models
            .iter()
            .filter(|model| is_valid_model(model))
            .map(|model| {
                alias_option(model).unwrap_or_else(|| ModelOption {
                    value: model.clone(),
                    label: model.clone(),
                    detail: None,
                })
            })
            .collect(),
        None => ALIASES.iter().filter_map(|(alias, _, _)| alias_option(alias)).collect(),
    }
}

fn options_from(settings: &ModelSettings, process_env: &dyn Fn(&str) -> Option<String>) -> (Option<String>, Vec<ModelOption>) {
    let default_model = settings
        .env_value(MODEL_ENV, process_env)
        .or_else(|| settings.model.clone().filter(|model| is_valid_model(model)));
    (default_model, model_options(settings, process_env))
}

/// Claude Code の設定フォルダ（`CLAUDE_CONFIG_DIR` があればそこ）
fn claude_config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".claude")))
}

/// 読む設定ファイル（上書きされる順）
fn settings_files(config_dir: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let project = cwd.join(".claude");
    config_dir
        .map(|dir| dir.join("settings.json"))
        .into_iter()
        .chain([project.join("settings.json"), project.join("settings.local.json")])
        .chain(MANAGED_SETTINGS.iter().map(PathBuf::from))
        .collect()
}

fn read_options(files: &[PathBuf], process_env: &dyn Fn(&str) -> Option<String>) -> ModelOptions {
    let mut sources = Vec::new();
    let mut warnings = Vec::new();
    let mut settings = ModelSettings::default();
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => {
                settings = settings.merged(&value);
                sources.push(file.display().to_string());
            }
            Err(error) => warnings.push(format!("{}: {error}", file.display())),
        }
    }
    let (default_model, options) = options_from(&settings, process_env);
    ModelOptions { default_model, options, sources, warnings }
}

/// 作業フォルダで Claude Code が使う設定から、モデルの選択肢を読む
pub fn load_model_options(cwd: &Path) -> ModelOptions {
    let files = settings_files(claude_config_dir().as_deref(), cwd);
    read_options(&files, &|name| std::env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn offers_the_aliases_when_nothing_is_set() {
        let (default_model, options) = options_from(&ModelSettings::default(), &no_env);
        assert_eq!(default_model, None);
        let values: Vec<_> = options.iter().map(|option| option.value.as_str()).collect();
        assert_eq!(values, ["sonnet", "opus", "haiku"]);
        assert!(options.iter().all(|option| option.detail.is_none()));
    }

    #[test]
    fn later_settings_override_earlier_ones_per_variable() {
        let settings = ModelSettings::default()
            .merged(&json!({
                "model": "opus",
                "env": {
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": "user-sonnet",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": "user-opus",
                    "AWS_BEARER_TOKEN_BEDROCK": "secret"
                }
            }))
            .merged(&json!({ "env": { "ANTHROPIC_DEFAULT_SONNET_MODEL": "project-sonnet" } }));

        assert_eq!(settings.model.as_deref(), Some("opus"));
        let (default_model, options) = options_from(&settings, &no_env);
        assert_eq!(default_model.as_deref(), Some("opus"));
        assert_eq!(options[0].detail.as_deref(), Some("project-sonnet"));
        assert_eq!(options[1].detail.as_deref(), Some("user-opus"));
        // モデル以外の環境変数は持たない
        assert!(settings.env.iter().all(|(name, _)| name.ends_with("_MODEL")));
    }

    #[test]
    fn the_model_variable_wins_over_the_model_setting() {
        let settings = ModelSettings::default().merged(&json!({ "model": "sonnet" }));
        let env = |name: &str| (name == MODEL_ENV).then(|| "arn:aws:bedrock:us-east-1:1:inference-profile/x".to_owned());
        let (default_model, _) = options_from(&settings, &env);
        assert_eq!(default_model.as_deref(), Some("arn:aws:bedrock:us-east-1:1:inference-profile/x"));

        // 設定ファイルの env は、Orca を起動したときの環境変数より優先する
        let settings = settings.merged(&json!({ "env": { "ANTHROPIC_MODEL": "from-settings" } }));
        assert_eq!(options_from(&settings, &env).0.as_deref(), Some("from-settings"));
    }

    #[test]
    fn available_models_limit_the_choices_and_skip_odd_values() {
        let settings = ModelSettings::default().merged(&json!({
            "availableModels": ["sonnet", "claude-sonnet-4-5", "bad model; rm", 3],
            "env": { "ANTHROPIC_DEFAULT_SONNET_MODEL": "resolved-sonnet" }
        }));
        let (_, options) = options_from(&settings, &no_env);
        assert_eq!(
            options,
            [
                ModelOption { value: "sonnet".into(), label: "Sonnet".into(), detail: Some("resolved-sonnet".into()) },
                ModelOption { value: "claude-sonnet-4-5".into(), label: "claude-sonnet-4-5".into(), detail: None },
            ]
        );
    }

    #[test]
    fn reads_the_files_in_order_and_reports_broken_ones() {
        let dir = std::env::temp_dir().join(format!("orca-model-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let user = dir.join("user.json");
        let project = dir.join("project.json");
        let broken = dir.join("broken.json");
        std::fs::write(&user, r#"{ "model": "haiku" }"#).unwrap();
        std::fs::write(&project, r#"{ "model": "opus" }"#).unwrap();
        std::fs::write(&broken, "{ not json").unwrap();

        let files = [user.clone(), dir.join("missing.json"), project.clone(), broken.clone()];
        let options = read_options(&files, &no_env);
        assert_eq!(options.default_model.as_deref(), Some("opus"));
        assert_eq!(options.sources, [user.display().to_string(), project.display().to_string()]);
        assert_eq!(options.warnings.len(), 1);
        assert!(options.warnings[0].starts_with(&broken.display().to_string()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn looks_in_the_user_project_and_managed_settings() {
        let files = settings_files(Some(Path::new(r"C:\u\.claude")), Path::new(r"C:\work"));
        assert_eq!(files[0], Path::new(r"C:\u\.claude\settings.json"));
        assert_eq!(files[1], Path::new(r"C:\work\.claude\settings.json"));
        assert_eq!(files[2], Path::new(r"C:\work\.claude\settings.local.json"));
        assert_eq!(files.len(), 3 + MANAGED_SETTINGS.len());
    }
}
