//! Bedrock / Anthropic API の呼び出し（Messages API・非ストリーミング。AI.md 8.0a・8.0）。
//!
//! 両者の要求・応答はほぼ同じ形で、違いはエンドポイント・認証ヘッダ・モデルの指定方法のみ。
//! キー・トークンはヘッダにだけ入れ、エラーの文言には含めない。

use std::time::Duration;

use serde_json::{Map, Value};
use ureq::tls::{RootCerts, TlsConfig};

use super::config::Provider;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const BEDROCK_ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";
/// 1 回の API 呼び出しの上限時間（応答が全部そろうまで待つため長め）
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
/// エラーの応答本文を表示する最大文字数
const MAX_ERROR_BODY_CHARS: usize = 300;

/// 送信する HTTP 要求（通信せずに組み立てて確認できるよう分けている）
#[derive(Debug, PartialEq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

/// Messages API の要求を接続先に合わせて組み立てる。
/// `request` は `max_tokens`・`system`・`tools`・`messages` を持つオブジェクト（`model` は含めない）
pub fn build_request(provider: &Provider, request: &Map<String, Value>) -> Result<HttpRequest, String> {
    let mut body = request.clone();
    match provider {
        Provider::Anthropic { api_key, model } => {
            body.insert("model".into(), Value::String(model.clone()));
            Ok(HttpRequest {
                url: ANTHROPIC_URL.to_string(),
                headers: vec![
                    ("x-api-key", api_key.clone()),
                    ("anthropic-version", ANTHROPIC_VERSION.to_string()),
                    ("content-type", "application/json".to_string()),
                ],
                body: Value::Object(body),
            })
        }
        Provider::Bedrock { region, token, model } => {
            if region.is_empty() || !region.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                return Err(format!("invalid AWS region: {region}"));
            }
            body.insert("anthropic_version".into(), Value::String(BEDROCK_ANTHROPIC_VERSION.into()));
            Ok(HttpRequest {
                url: format!(
                    "https://bedrock-runtime.{region}.amazonaws.com/model/{}/invoke",
                    encode_path_segment(model)
                ),
                headers: vec![
                    ("authorization", format!("Bearer {token}")),
                    ("content-type", "application/json".to_string()),
                    ("accept", "application/json".to_string()),
                ],
                body: Value::Object(body),
            })
        }
    }
}

/// URL のパスの 1 区間として使えるようにエンコードする（推論プロファイルの ARN は `:` や `/` を含む）
fn encode_path_segment(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// エラー応答から利用者に見せる文言を取り出す（Anthropic: error.message、Bedrock: message）
fn error_message(status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let message = parsed.as_ref().and_then(|value| {
        value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .or_else(|| value.get("Message"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let detail = message.unwrap_or_else(|| body.chars().take(MAX_ERROR_BODY_CHARS).collect());
    format!("the AI service returned HTTP {status}: {detail}")
}

/// Messages API を呼ぶ（テストではモックに差し替える）
pub trait ModelClient: Send {
    fn create_message(&self, request: &Map<String, Value>) -> Result<Value, String>;
}

pub struct HttpModelClient {
    provider: Provider,
    agent: ureq::Agent,
}

impl HttpModelClient {
    pub fn new(provider: Provider) -> Self {
        // 証明書の検証は Windows に任せる（社内プロキシの独自のルート証明書に対応）。
        // プロキシは環境変数（HTTPS_PROXY 等）と Windows の設定から読む（ureq の既定）
        let agent = ureq::Agent::config_builder()
            .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self { provider, agent }
    }
}

impl ModelClient for HttpModelClient {
    fn create_message(&self, request: &Map<String, Value>) -> Result<Value, String> {
        let http = build_request(&self.provider, request)?;
        let mut call = self.agent.post(&http.url);
        for (name, value) in &http.headers {
            call = call.header(*name, value);
        }
        let mut response = call
            .send(http.body.to_string())
            .map_err(|e| format!("failed to reach the AI service: {e}"))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("failed to read the AI service response: {e}"))?;
        if !(200..300).contains(&status) {
            return Err(error_message(status, &body));
        }
        serde_json::from_str(&body).map_err(|e| format!("the AI service returned invalid JSON: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Map<String, Value> {
        json!({ "max_tokens": 1024, "messages": [{ "role": "user", "content": "hi" }] })
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn builds_an_anthropic_request() {
        let provider = Provider::Anthropic { api_key: "sk-test".into(), model: "claude-x".into() };
        let http = build_request(&provider, &request()).unwrap();
        assert_eq!(http.url, "https://api.anthropic.com/v1/messages");
        assert!(http.headers.contains(&("x-api-key", "sk-test".to_string())));
        assert!(http.headers.contains(&("anthropic-version", "2023-06-01".to_string())));
        assert_eq!(http.body["model"], "claude-x");
        assert_eq!(http.body["max_tokens"], 1024);
    }

    #[test]
    fn builds_a_bedrock_request_with_the_model_in_the_path() {
        let provider = Provider::Bedrock {
            region: "ap-northeast-1".into(),
            token: "bedrock-token".into(),
            model: "arn:aws:bedrock:ap-northeast-1:123:inference-profile/apac.anthropic.claude-x".into(),
        };
        let http = build_request(&provider, &request()).unwrap();
        assert_eq!(
            http.url,
            "https://bedrock-runtime.ap-northeast-1.amazonaws.com/model/\
             arn%3Aaws%3Abedrock%3Aap-northeast-1%3A123%3Ainference-profile%2Fapac.anthropic.claude-x/invoke"
        );
        assert!(http.headers.contains(&("authorization", "Bearer bedrock-token".to_string())));
        assert_eq!(http.body["anthropic_version"], "bedrock-2023-05-31");
        // Bedrock ではモデルを本文に入れない
        assert!(http.body.get("model").is_none());
    }

    #[test]
    fn rejects_a_region_that_would_change_the_host() {
        let provider = Provider::Bedrock {
            region: "evil.example.com/x?".into(),
            token: "t".into(),
            model: "m".into(),
        };
        assert!(build_request(&provider, &request()).is_err());
    }

    #[test]
    fn extracts_error_messages_from_both_services() {
        let anthropic = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        assert_eq!(error_message(401, anthropic), "the AI service returned HTTP 401: invalid x-api-key");
        let bedrock = r#"{"message":"The security token included in the request is invalid."}"#;
        assert!(error_message(403, bedrock).ends_with("The security token included in the request is invalid."));
        let html = "<html>".repeat(100);
        assert!(error_message(502, &html).len() < 400);
    }
}
