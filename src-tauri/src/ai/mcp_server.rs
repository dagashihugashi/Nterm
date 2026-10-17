//! PC の中だけで動く MCP サーバ（AI あり版のみ。AI.md 12 章）。
//!
//! Claude Code が Orca の道具（サーバの読み取り・承認）を使うための窓口。VS Code の拡張機能と同じく、
//! 127.0.0.1 でだけ待ち受け、起動ごとに作るランダムな鍵（Bearer トークン）を持つ要求だけを受け付ける。
//! 方式は MCP の Streamable HTTP のうち、JSON で 1 回ずつ応答する部分だけ（SSE は使わない）。
//! 外部のライブラリは使わず、標準ライブラリだけで実装する（社内のクレートのミラーで解決できるように）。

use std::io::{BufRead, BufReader, Read, Take, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const ENDPOINT: &str = "/mcp";
/// 対応する MCP のプロトコルのバージョン（クライアントが求めた版がこの中にあればそれを、なければ先頭を返す）
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-11-25", "2025-03-26", "2024-11-05"];
/// 要求の行とヘッダの合計の最大バイト数
const MAX_HEADER_BYTES: u64 = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// 要求を受け取りきるまでの上限（少しずつ送って接続を握り続けさせない。応答の作成＝承認待ちの時間は含まない）
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
/// 同時に処理する接続の最大数（承認待ちの間も接続は続くため、Claude Code が使う数より十分多くする）
const MAX_CONNECTIONS: usize = 16;
/// 待ち受けの順番待ちの数
const LISTEN_BACKLOG: i32 = 128;

/// 道具の実行結果
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub text: String,
    pub is_error: bool,
}

/// MCP サーバが提供する道具（C2 でサーバの読み取り・承認をつなぐ）
pub trait ToolHandler: Send + Sync {
    /// `tools/list` で返す道具の定義（name・description・inputSchema）
    fn tools(&self) -> Vec<Value>;
    /// `tools/call`。承認待ちなどで時間がかかってもよい（接続ごとに別スレッドで呼ぶ）
    fn call(&self, name: &str, arguments: &Value) -> ToolOutcome;
}

/// 起動中の MCP サーバ。破棄すると待ち受けを止める
pub struct McpServer {
    port: u16,
    token: String,
    stopped: Arc<AtomicBool>,
}

impl McpServer {
    /// 127.0.0.1 の空いているポートで待ち受けを始める
    pub fn start(handler: Arc<dyn ToolHandler>) -> Result<Self, String> {
        let listener = listen_exclusively().map_err(|e| format!("failed to start the MCP server: {e}"))?;
        let port = listener.local_addr().map_err(|e| format!("failed to start the MCP server: {e}"))?.port();
        let token = random_token()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let server = Self { port, token: token.clone(), stopped: stopped.clone() };
        let active = Arc::new(AtomicUsize::new(0));
        thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                // 接続の数に上限を設ける（同じ PC の別のプロセスに、接続を開き続けてスレッドを使い切らせない）
                let Some(slot) = ConnectionSlot::take(&active) else { continue };
                let handler = handler.clone();
                let token = token.clone();
                thread::spawn(move || {
                    serve_connection(stream, handler.as_ref(), &token);
                    drop(slot);
                });
            }
        });
        Ok(server)
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}{ENDPOINT}", self.port)
    }

    /// Claude Code の `--mcp-config` に渡す設定（`name` は道具の名前の接頭辞 `mcp__<name>__` になる）
    pub fn config_json(&self, name: &str) -> Value {
        json!({
            "mcpServers": {
                name: {
                    "type": "http",
                    "url": self.url(),
                    "headers": { "Authorization": format!("Bearer {}", self.token) }
                }
            }
        })
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // 待ち受け中の accept を起こして、スレッドを終わらせる
        let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, self.port));
    }
}

/// 127.0.0.1 の空いているポートで、ほかのプロセスに同じポートを使わせない形で待ち受ける。
/// Windows では SO_EXCLUSIVEADDRUSE を付けないと、別のプロセスが SO_REUSEADDR で同じポートに割り込み、
/// Claude Code の接続（鍵を含む）を受け取れてしまうことがある
fn listen_exclusively() -> std::io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    use std::os::windows::io::AsRawSocket;
    use windows::Win32::Networking::WinSock::{setsockopt, SOCKET, SOL_SOCKET, SO_EXCLUSIVEADDRUSE};

    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    let enabled = 1i32.to_ne_bytes();
    // SAFETY: socket は上で作った有効なソケットで、値は長さ付きのスライスで渡す
    let result = unsafe {
        setsockopt(SOCKET(socket.as_raw_socket() as usize), SOL_SOCKET, SO_EXCLUSIVEADDRUSE, Some(&enabled))
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    socket.bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, 0)).into())?;
    socket.listen(LISTEN_BACKLOG)?;
    Ok(socket.into())
}

/// 処理中の接続の数（破棄すると 1 つ減る）
struct ConnectionSlot(Arc<AtomicUsize>);

impl ConnectionSlot {
    /// 上限に達していれば None
    fn take(active: &Arc<AtomicUsize>) -> Option<Self> {
        let previous = active.fetch_add(1, Ordering::SeqCst);
        if previous >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self(active.clone()))
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 起動ごとの鍵（Windows の乱数で 32 バイト）
fn random_token() -> Result<String, String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut bytes = [0u8; 32];
    // SAFETY: 書き込み先は長さ付きのスライスで渡し、アルゴリズムのハンドルは使わない（システムの乱数を使う）
    unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
        .ok()
        .map_err(|e| format!("failed to create a key for the MCP server: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, PartialEq)]
struct HttpResponse {
    status: u16,
    body: Option<Value>,
}

impl HttpResponse {
    fn empty(status: u16) -> Self {
        Self { status, body: None }
    }
}

fn serve_connection(mut stream: TcpStream, handler: &dyn ToolHandler, token: &str) {
    let response = {
        let mut reader = DeadlineReader { stream: &stream, deadline: Instant::now() + REQUEST_DEADLINE };
        handle(&mut reader, handler, token)
    };
    // 相手が切断していても届かなくてよい
    let _ = write_response(&mut stream, &response);
}

/// 要求を受け取りきるまでの期限付きで読む（1 回ごとの待ち時間ではなく、要求全体の期限）
struct DeadlineReader<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(buf)
    }
}

/// 要求を 1 つ読んで応答を作る。鍵などの確認は本文を読む前に行う（確認できない相手に本文の分のメモリを確保させない）
fn handle(stream: &mut impl Read, handler: &dyn ToolHandler, token: &str) -> HttpResponse {
    let mut reader = BufReader::new(stream);
    let head = match read_head(&mut reader) {
        Ok(head) => head,
        Err(status) => return HttpResponse::empty(status),
    };
    if let Some(rejected) = reject(token, &head) {
        return rejected;
    }
    match read_body(&mut reader, &head) {
        Ok(body) => respond(handler, &HttpRequest { body, ..head }),
        Err(status) => HttpResponse::empty(status),
    }
}

/// HTTP/1.1 の要求の行とヘッダを読む（失敗した場合は返す状態コード）。合計の大きさに上限を設ける
fn read_head(reader: &mut impl BufRead) -> Result<HttpRequest, u16> {
    let mut limited = reader.take(MAX_HEADER_BYTES);
    let line = read_line_limited(&mut limited)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().ok_or(400u16)?.to_string(), parts.next().ok_or(400u16)?.to_string());
    let mut headers = Vec::new();
    loop {
        let header = read_line_limited(&mut limited)?;
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').ok_or(400u16)?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }
    Ok(HttpRequest { method, path, headers, body: Vec::new() })
}

/// 1 行を読む（改行で終わらない＝上限に達したか切断された場合は失敗）
fn read_line_limited(reader: &mut Take<impl BufRead>) -> Result<String, u16> {
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|_| 400u16)?;
    if !line.ends_with('\n') {
        return Err(if reader.limit() == 0 { 431 } else { 400 });
    }
    Ok(line)
}

fn read_body(reader: &mut impl Read, head: &HttpRequest) -> Result<Vec<u8>, u16> {
    let length: usize = head.header("content-length").map_or(Ok(0), str::parse).map_err(|_| 400u16)?;
    if length > MAX_BODY_BYTES {
        return Err(413);
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).map_err(|_| 400u16)?;
    Ok(body)
}

/// テスト用：要求を本文まで読む
#[cfg(test)]
fn read_request(stream: &mut impl Read) -> Result<HttpRequest, u16> {
    let mut reader = BufReader::new(stream);
    let head = read_head(&mut reader)?;
    let body = read_body(&mut reader, &head)?;
    Ok(HttpRequest { body, ..head })
}

fn write_response(stream: &mut impl Write, response: &HttpResponse) -> std::io::Result<()> {
    let body = response.body.as_ref().map(Value::to_string).unwrap_or_default();
    let content_type = if response.body.is_some() { "Content-Type: application/json\r\n" } else { "" };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        response.status,
        reason(response.status),
        body.len(),
    )?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    }
}

/// 鍵を時間差で推測されないよう、長さが同じなら全バイトを比べる
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// テスト用：確認と応答をまとめて行う
#[cfg(test)]
fn route(handler: &dyn ToolHandler, token: &str, request: &HttpRequest) -> HttpResponse {
    reject(token, request).unwrap_or_else(|| respond(handler, request))
}

/// 受け付けない要求なら、その応答（本文を読む前に、要求の行とヘッダだけで判断する）
fn reject(token: &str, request: &HttpRequest) -> Option<HttpResponse> {
    if request.path.split('?').next() != Some(ENDPOINT) {
        return Some(HttpResponse::empty(404));
    }
    // ブラウザからの要求（DNS リバインディング等）は受け付けない。Claude Code は Origin を付けず、
    // 接続先は 127.0.0.1 なので Host もそのアドレス（別の名前で届いた要求は受け付けない）
    let host = request.header("host").unwrap_or_default();
    let is_local_host = ["127.0.0.1:", "localhost:"].iter().any(|prefix| host.starts_with(prefix));
    if request.header("origin").is_some() || !is_local_host {
        return Some(HttpResponse::empty(403));
    }
    let authorized = request
        .header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|value| same_secret(value, token));
    if !authorized {
        return Some(HttpResponse::empty(401));
    }
    if request.method != "POST" {
        // SSE の受信（GET）やセッションの終了（DELETE）は使わない
        return Some(HttpResponse::empty(405));
    }
    None
}

fn respond(handler: &dyn ToolHandler, request: &HttpRequest) -> HttpResponse {
    let Ok(message) = serde_json::from_slice::<Value>(&request.body) else {
        return HttpResponse { status: 400, body: Some(rpc_error(Value::Null, -32700, "parse error")) };
    };
    match message {
        Value::Array(messages) => {
            let responses: Vec<Value> = messages.iter().filter_map(|message| rpc(handler, message)).collect();
            if responses.is_empty() {
                HttpResponse::empty(202)
            } else {
                HttpResponse { status: 200, body: Some(Value::Array(responses)) }
            }
        }
        message => match rpc(handler, &message) {
            Some(response) => HttpResponse { status: 200, body: Some(response) },
            None => HttpResponse::empty(202),
        },
    }
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// JSON-RPC のメッセージ 1 つを処理する。通知（id なし）には応答しない
fn rpc(handler: &dyn ToolHandler, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let params = &message["params"];
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let requested = params["protocolVersion"].as_str().unwrap_or_default();
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == requested).unwrap_or(&PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "orca", "version": env!("CARGO_PKG_VERSION") }
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": handler.tools() }),
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return Some(rpc_error(id, -32602, "\"name\" is required"));
            };
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let outcome = handler.call(name, &arguments);
            json!({ "content": [{ "type": "text", "text": outcome.text }], "isError": outcome.is_error })
        }
        _ => return Some(rpc_error(id, -32601, "method not found")),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoTools;
    impl ToolHandler for EchoTools {
        fn tools(&self) -> Vec<Value> {
            vec![json!({ "name": "echo", "description": "echo", "inputSchema": { "type": "object" } })]
        }
        fn call(&self, name: &str, arguments: &Value) -> ToolOutcome {
            match name {
                "echo" => ToolOutcome { text: arguments["text"].as_str().unwrap_or_default().to_string(), is_error: false },
                other => ToolOutcome { text: format!("unknown tool: {other}"), is_error: true },
            }
        }
    }

    fn post(body: Value, headers: &[(&str, &str)]) -> HttpRequest {
        let host = ("Host".to_string(), "127.0.0.1:50000".to_string());
        HttpRequest {
            method: "POST".into(),
            path: ENDPOINT.into(),
            headers: std::iter::once(host).chain(headers.iter().map(|(k, v)| (k.to_string(), v.to_string()))).collect(),
            body: body.to_string().into_bytes(),
        }
    }

    const AUTH: (&str, &str) = ("Authorization", "Bearer secret");

    #[test]
    fn answers_initialize_list_and_call() {
        let init = rpc(&EchoTools, &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } })).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "orca");
        let unknown_version = rpc(&EchoTools, &json!({ "id": 1, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } })).unwrap();
        assert_eq!(unknown_version["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);

        let list = rpc(&EchoTools, &json!({ "id": "a", "method": "tools/list" })).unwrap();
        assert_eq!(list["id"], "a");
        assert_eq!(list["result"]["tools"][0]["name"], "echo");

        let call = rpc(&EchoTools, &json!({ "id": 2, "method": "tools/call", "params": { "name": "echo", "arguments": { "text": "hi" } } })).unwrap();
        assert_eq!(call["result"], json!({ "content": [{ "type": "text", "text": "hi" }], "isError": false }));
    }

    #[test]
    fn reports_errors_and_ignores_notifications() {
        assert_eq!(rpc(&EchoTools, &json!({ "method": "notifications/initialized" })), None);
        let unknown = rpc(&EchoTools, &json!({ "id": 3, "method": "resources/list" })).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        let missing = rpc(&EchoTools, &json!({ "id": 4, "method": "tools/call", "params": {} })).unwrap();
        assert_eq!(missing["error"]["code"], -32602);
    }

    #[test]
    fn requires_the_key_and_rejects_browsers() {
        let message = json!({ "id": 1, "method": "ping" });
        assert_eq!(route(&EchoTools, "secret", &post(message.clone(), &[])).status, 401);
        assert_eq!(route(&EchoTools, "secret", &post(message.clone(), &[("Authorization", "Bearer wrong!")])).status, 401);
        assert_eq!(route(&EchoTools, "secret", &post(message.clone(), &[AUTH, ("Origin", "http://evil.example")])).status, 403);
        let mut other_path = post(message.clone(), &[AUTH]);
        other_path.path = "/other".into();
        assert_eq!(route(&EchoTools, "secret", &other_path).status, 404);
        let mut get = post(message.clone(), &[AUTH]);
        get.method = "GET".into();
        assert_eq!(route(&EchoTools, "secret", &get).status, 405);
        // 127.0.0.1 以外の名前で届いた要求（DNS リバインディング等）・Host のない要求は受け付けない
        let mut other_host = post(message.clone(), &[AUTH]);
        other_host.headers[0].1 = "evil.example:50000".into();
        assert_eq!(route(&EchoTools, "secret", &other_host).status, 403);
        let mut no_host = post(message.clone(), &[AUTH]);
        no_host.headers.remove(0);
        assert_eq!(route(&EchoTools, "secret", &no_host).status, 403);
        assert_eq!(route(&EchoTools, "secret", &post(message, &[AUTH])).status, 200);
    }

    #[test]
    fn checks_the_key_before_reading_the_body() {
        // 鍵のない要求は、本文（大きな Content-Length）を受け取る前に断る
        let raw = format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: {MAX_BODY_BYTES}\r\n\r\n");
        assert_eq!(handle(&mut raw.as_bytes(), &EchoTools, "secret").status, 401);
        // 改行のない巨大な行・大きすぎるヘッダは読み続けない
        let long_line = "A".repeat(MAX_HEADER_BYTES as usize + 10);
        assert_eq!(handle(&mut long_line.as_bytes(), &EchoTools, "secret").status, 431);
        let many_headers = format!("POST /mcp HTTP/1.1\r\n{}\r\n", "X-Pad: aaaaaaaaaaaaaaaa\r\n".repeat(1000));
        assert_eq!(handle(&mut many_headers.as_bytes(), &EchoTools, "secret").status, 431);
        // 途中で切れた要求
        assert_eq!(handle(&mut &b"POST /mcp HTTP/1.1\r\nHost: 127"[..], &EchoTools, "secret").status, 400);
    }

    #[test]
    fn limits_the_number_of_connections() {
        let active = Arc::new(AtomicUsize::new(0));
        let slots: Vec<ConnectionSlot> = (0..MAX_CONNECTIONS).map(|_| ConnectionSlot::take(&active).unwrap()).collect();
        assert!(ConnectionSlot::take(&active).is_none());
        drop(slots);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(ConnectionSlot::take(&active).is_some());
    }

    #[test]
    fn notifications_get_202_and_bad_json_gets_a_parse_error() {
        let notification = post(json!({ "method": "notifications/initialized" }), &[AUTH]);
        assert_eq!(route(&EchoTools, "secret", &notification), HttpResponse::empty(202));
        let mut bad = post(json!({}), &[AUTH]);
        bad.body = b"{not json".to_vec();
        let response = route(&EchoTools, "secret", &bad);
        assert_eq!(response.status, 400);
        assert_eq!(response.body.unwrap()["error"]["code"], -32700);
    }

    #[test]
    fn parses_http_requests_with_limits() {
        let raw = b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        let request = read_request(&mut &raw[..]).unwrap();
        assert_eq!((request.method.as_str(), request.path.as_str(), request.body.as_slice()), ("POST", "/mcp", &b"{}"[..]));
        assert_eq!(request.header("content-type"), Some("application/json"));
        let too_big = format!("POST /mcp HTTP/1.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY_BYTES + 1);
        assert_eq!(read_request(&mut too_big.as_bytes()).unwrap_err(), 413);
    }

    #[test]
    fn serves_requests_over_localhost() {
        let server = McpServer::start(Arc::new(EchoTools)).unwrap();
        let config = server.config_json("orca");
        let url = config["mcpServers"]["orca"]["url"].as_str().unwrap().to_string();
        let auth = config["mcpServers"]["orca"]["headers"]["Authorization"].as_str().unwrap().to_string();
        assert!(url.starts_with("http://127.0.0.1:") && url.ends_with("/mcp"));
        assert_eq!(auth.len(), "Bearer ".len() + 64);

        let body = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": "echo", "arguments": { "text": "日本語" } } }).to_string();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port)).unwrap();
        write!(stream, "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", server.port, body.len()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        let json: Value = serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(json["result"]["content"][0]["text"], "日本語");
    }

    #[test]
    fn keys_differ_each_time() {
        assert_ne!(random_token().unwrap(), random_token().unwrap());
    }
}
