//! ログイン後のユーザ切り替え（su 等）の自動化。
//!
//! 受信テキストから「現在の行」を追跡し、次の順で進める（手順ごとに繰り返す）。
//! 1. シェルのプロンプト（`$` `#` `>` `%` で終わる行）を検知、または出力が少し止まったらコマンドを送る
//! 2. パスワード入力欄（`Password:` 等）を検知したときだけパスワードを送る
//!    （検知できなければ送らずに中断する。シェルにパスワードを打ち込む事故を防ぐため）
//! 3. 失敗メッセージを検知したら失敗、新しいプロンプトが出たら成功
//!
//! 時刻は呼び出し側から渡す（テストで時間経過を再現するため）。

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::log_formatter::LogFormatter;

/// プロンプト・応答を待つ上限
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
/// プロンプトの形で判定できない場合、出力がこの時間止まったら次へ進む
const IDLE_THRESHOLD: Duration = Duration::from_millis(300);
const SHELL_PROMPT_ENDINGS: &[char] = &['$', '#', '>', '%'];
const PASSWORD_PROMPT_WORDS: &[&str] = &["password", "passwort", "パスワード"];
/// パスワード送信後の認証失敗（小文字に変換した行に含まれていたら失敗）
const AUTH_FAILURE_WORDS: &[&str] = &[
    "authentication failure",
    "incorrect password",
    "sorry",
    "認証失敗",
    "認証に失敗",
];
/// コマンド送信直後の失敗（ユーザーが存在しない等）。
/// 切り替え後のログインスクリプトの出力を誤判定しないよう、パスワード送信後は見ない。
const COMMAND_FAILURE_WORDS: &[&str] =
    &["does not exist", "unknown id", "unknown user", "permission denied"];
const CTRL_C: &str = "\x03";

/// 1回分の切り替え手順
pub struct SwitchStep {
    pub user: String,
    /// 送信するコマンド（`{user}` は展開済み）
    pub command: String,
    pub saved_secret: Option<String>,
}

/// フロントエンドへの通知（`{ kind: "switched", user }` 等）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SwitchUserNotice {
    /// 切り替えコマンドを送った（完了までキー入力は送られない）
    Switching { user: String },
    /// パスワードの入力が必要（保存されていない、または直前に認証失敗した）
    SecretRequired { user: String, error: Option<String> },
    Switched { user: String },
    /// 認証に失敗した。プロンプトに戻った後、再入力を求める
    AuthFailed { user: String, message: String },
    /// 中断した（プロンプト未検知・タイムアウト等）
    Failed { user: String, message: String },
    Cancelled { user: String },
    Finished,
    /// 切り替えは成功したが利用者に伝えるべき問題（パスワードの保存失敗など）
    Warning { message: String },
}

#[derive(Debug, PartialEq)]
pub enum Action {
    /// リモートへ送る
    Send(String),
    /// 認証に成功した入力値を保存（remember = false なら保存済みの値を削除）
    PersistSecret { user: String, secret: String, remember: bool },
    Notify(SwitchUserNotice),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    WaitShell { deadline: Instant },
    WaitPasswordPrompt { deadline: Instant },
    WaitSecret,
    WaitResult { deadline: Instant },
    Done,
}

struct EnteredSecret {
    secret: String,
    remember: bool,
}

pub struct SwitchUserAutomation {
    steps: Vec<SwitchStep>,
    index: usize,
    phase: Phase,
    screen: LogFormatter,
    last_output: Instant,
    /// コマンド・パスワード送信後に改行を受信したか（送信前の行と区別するため）
    has_fresh_line: bool,
    entered: Option<EnteredSecret>,
    /// 直前の認証失敗の理由（再入力を求めるときに表示する）
    retry_error: Option<String>,
}

fn is_shell_prompt(line: &str) -> bool {
    line.ends_with(SHELL_PROMPT_ENDINGS)
}

fn is_password_prompt(line: &str) -> bool {
    let lower = line.to_lowercase();
    (lower.ends_with(':') || lower.ends_with('：'))
        && PASSWORD_PROMPT_WORDS.iter().any(|word| lower.contains(word))
}

fn contains_any(line: &str, words: &[&str]) -> bool {
    let lower = line.to_lowercase();
    words.iter().any(|word| lower.contains(word))
}

fn is_auth_failure(line: &str) -> bool {
    contains_any(line, AUTH_FAILURE_WORDS)
}

fn is_command_failure(line: &str) -> bool {
    is_auth_failure(line) || contains_any(line, COMMAND_FAILURE_WORDS)
}

impl SwitchUserAutomation {
    /// 手順がなければ `None`
    pub fn new(steps: Vec<SwitchStep>, now: Instant) -> Option<Self> {
        if steps.is_empty() {
            return None;
        }
        Some(Self {
            steps,
            index: 0,
            phase: Phase::WaitShell { deadline: now + RESPONSE_TIMEOUT },
            screen: LogFormatter::default(),
            last_output: now,
            has_fresh_line: false,
            entered: None,
            retry_error: None,
        })
    }

    pub fn is_active(&self) -> bool {
        self.phase != Phase::Done
    }

    fn user(&self) -> String {
        self.steps[self.index].user.clone()
    }

    /// 受信テキストを処理する
    pub fn feed(&mut self, text: &str, now: Instant) -> Vec<Action> {
        if !self.is_active() {
            return Vec::new();
        }
        self.last_output = now;
        let lines = self.screen.feed(text);
        let mut actions = Vec::new();
        if !lines.is_empty() {
            let is_failure: Option<fn(&str) -> bool> = match self.phase {
                Phase::WaitPasswordPrompt { .. } => Some(is_command_failure),
                Phase::WaitResult { .. } => Some(is_auth_failure),
                _ => None,
            };
            if let Some(is_failure) = is_failure {
                self.has_fresh_line = true;
                if let Some(line) = lines.iter().find(|line| is_failure(line)) {
                    self.fail_step(line.trim().to_string(), now, &mut actions);
                }
            }
        }
        self.evaluate(now, &mut actions);
        actions
    }

    /// 時間経過（出力の停止・タイムアウト）を処理する。セッションのループから定期的に呼ぶ。
    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.is_active() {
            self.evaluate(now, &mut actions);
        }
        actions
    }

    /// パスワード入力ダイアログの回答。`None` はキャンセル。
    pub fn provide_secret(&mut self, answer: Option<(String, bool)>, now: Instant) -> Vec<Action> {
        if self.phase != Phase::WaitSecret {
            return Vec::new();
        }
        let Some((secret, remember)) = answer else {
            return self.cancel();
        };
        let actions = vec![Action::Send(format!("{secret}\r"))];
        self.entered = Some(EnteredSecret { secret, remember });
        self.wait_for(Phase::WaitResult { deadline: now + RESPONSE_TIMEOUT });
        actions
    }

    /// 利用者による中断（Esc）。su がパスワード入力待ちの場合は Ctrl+C で抜ける。
    pub fn cancel(&mut self) -> Vec<Action> {
        if !self.is_active() {
            return Vec::new();
        }
        let mut actions = Vec::new();
        if !matches!(self.phase, Phase::WaitShell { .. }) {
            actions.push(Action::Send(CTRL_C.to_string()));
        }
        actions.push(Action::Notify(SwitchUserNotice::Cancelled { user: self.user() }));
        self.phase = Phase::Done;
        actions
    }

    /// 利用者が Ctrl+C を入力した（入力自体はリモートへ送られる）
    pub fn interrupt(&mut self) -> Vec<Action> {
        if !self.is_active() {
            return Vec::new();
        }
        self.phase = Phase::Done;
        vec![Action::Notify(SwitchUserNotice::Cancelled { user: self.user() })]
    }

    /// 送信後の応答待ちに入る（送信前に表示されていた行を応答と取り違えないようにする）
    fn wait_for(&mut self, phase: Phase) {
        self.phase = phase;
        self.has_fresh_line = false;
    }

    /// 状態が変わらなくなるまで進める
    fn evaluate(&mut self, now: Instant, actions: &mut Vec<Action>) {
        while self.advance(now, actions) {}
    }

    /// 1段階進める。続けて評価すべき場合は true
    fn advance(&mut self, now: Instant, actions: &mut Vec<Action>) -> bool {
        let line = self.screen.current_line();
        let is_idle = !line.is_empty() && now.duration_since(self.last_output) >= IDLE_THRESHOLD;
        let looks_ready = is_shell_prompt(&line) || is_idle;
        match self.phase {
            Phase::WaitShell { deadline } => {
                if looks_ready {
                    let command = self.steps[self.index].command.clone();
                    actions.push(Action::Notify(SwitchUserNotice::Switching { user: self.user() }));
                    actions.push(Action::Send(format!("{command}\r")));
                    self.wait_for(Phase::WaitPasswordPrompt { deadline: now + RESPONSE_TIMEOUT });
                } else if now >= deadline {
                    self.abort("the shell prompt was not detected", actions);
                }
                false
            }
            Phase::WaitPasswordPrompt { deadline } => {
                if self.has_fresh_line && is_password_prompt(&line) {
                    self.answer_password_prompt(now, actions);
                    false
                } else if self.has_fresh_line && looks_ready {
                    // パスワードを求められずにプロンプトに戻った（root からの切り替え等）
                    self.complete_step(now, actions);
                    true
                } else {
                    if now >= deadline {
                        self.abort("the password prompt was not detected (password not sent)", actions);
                    }
                    false
                }
            }
            Phase::WaitResult { deadline } => {
                if self.has_fresh_line && looks_ready {
                    self.complete_step(now, actions);
                    true
                } else {
                    if now >= deadline {
                        self.abort("no response after entering the password", actions);
                    }
                    false
                }
            }
            Phase::WaitSecret | Phase::Done => false,
        }
    }

    fn answer_password_prompt(&mut self, now: Instant, actions: &mut Vec<Action>) {
        let secret = self
            .entered
            .as_ref()
            .map(|entered| entered.secret.clone())
            .or_else(|| self.steps[self.index].saved_secret.clone());
        match secret {
            Some(secret) => {
                actions.push(Action::Send(format!("{secret}\r")));
                self.wait_for(Phase::WaitResult { deadline: now + RESPONSE_TIMEOUT });
            }
            None => {
                actions.push(Action::Notify(SwitchUserNotice::SecretRequired {
                    user: self.user(),
                    error: self.retry_error.take(),
                }));
                self.phase = Phase::WaitSecret;
            }
        }
    }

    fn complete_step(&mut self, now: Instant, actions: &mut Vec<Action>) {
        let user = self.user();
        if let Some(EnteredSecret { secret, remember }) = self.entered.take() {
            actions.push(Action::PersistSecret { user: user.clone(), secret, remember });
        }
        actions.push(Action::Notify(SwitchUserNotice::Switched { user }));
        self.retry_error = None;
        self.index += 1;
        if self.index < self.steps.len() {
            self.phase = Phase::WaitShell { deadline: now + RESPONSE_TIMEOUT };
        } else {
            actions.push(Action::Notify(SwitchUserNotice::Finished));
            self.phase = Phase::Done;
        }
    }

    /// 失敗メッセージを受信した。パスワード送信後なら再入力を求め、それ以外は中断する。
    fn fail_step(&mut self, message: String, now: Instant, actions: &mut Vec<Action>) {
        if !matches!(self.phase, Phase::WaitResult { .. }) {
            self.abort(&message, actions);
            return;
        }
        actions.push(Action::Notify(SwitchUserNotice::AuthFailed {
            user: self.user(),
            message: message.clone(),
        }));
        // 誤っていた値（入力値・保存済みの値）は使わず、プロンプトに戻ったらやり直して再入力を求める
        self.entered = None;
        self.steps[self.index].saved_secret = None;
        self.retry_error = Some(message);
        self.phase = Phase::WaitShell { deadline: now + RESPONSE_TIMEOUT };
    }

    fn abort(&mut self, message: &str, actions: &mut Vec<Action>) {
        actions.push(Action::Notify(SwitchUserNotice::Failed {
            user: self.user(),
            message: message.to_string(),
        }));
        self.phase = Phase::Done;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMPT: &str = "Last login: Mon Oct  5 09:00:00 2026\r\n[app@web01 ~]$ ";

    struct Harness {
        automation: SwitchUserAutomation,
        start: Instant,
    }

    impl Harness {
        fn new(steps: Vec<SwitchStep>) -> Self {
            let start = Instant::now();
            Self { automation: SwitchUserAutomation::new(steps, start).unwrap(), start }
        }

        fn at(&self, millis: u64) -> Instant {
            self.start + Duration::from_millis(millis)
        }

        fn feed(&mut self, millis: u64, text: &str) -> Vec<Action> {
            let now = self.at(millis);
            self.automation.feed(text, now)
        }

        fn tick(&mut self, millis: u64) -> Vec<Action> {
            let now = self.at(millis);
            self.automation.tick(now)
        }
    }

    fn step(user: &str, saved_secret: Option<&str>) -> SwitchStep {
        SwitchStep {
            user: user.to_string(),
            command: format!("su - {user}"),
            saved_secret: saved_secret.map(str::to_string),
        }
    }

    fn send(text: &str) -> Action {
        Action::Send(text.to_string())
    }

    fn notify(notice: SwitchUserNotice) -> Action {
        Action::Notify(notice)
    }

    fn switched(user: &str) -> Action {
        notify(SwitchUserNotice::Switched { user: user.to_string() })
    }

    /// 切り替えコマンドの送信（通知と送信）
    fn start(user: &str) -> [Action; 2] {
        [
            notify(SwitchUserNotice::Switching { user: user.to_string() }),
            send(&format!("su - {user}\r")),
        ]
    }

    #[test]
    fn returns_none_without_steps() {
        assert!(SwitchUserAutomation::new(Vec::new(), Instant::now()).is_none());
    }

    #[test]
    fn sends_command_as_soon_as_prompt_appears() {
        let mut h = Harness::new(vec![step("oracle", Some("pw"))]);
        assert_eq!(h.feed(0, PROMPT), start("oracle"));
    }

    #[test]
    fn waits_for_idle_output_when_prompt_is_unusual() {
        let mut h = Harness::new(vec![step("oracle", Some("pw"))]);
        assert!(h.feed(0, "app@web01 ❯ ").is_empty());
        assert!(h.tick(200).is_empty());
        assert_eq!(h.tick(300), start("oracle"));
    }

    #[test]
    fn switches_with_saved_password() {
        let mut h = Harness::new(vec![step("oracle", Some("pw"))]);
        h.feed(0, PROMPT);
        assert_eq!(h.feed(10, "su - oracle\r\nPassword: "), [send("pw\r")]);
        assert_eq!(
            h.feed(20, "\r\n[oracle@web01 ~]$ "),
            [switched("oracle"), notify(SwitchUserNotice::Finished)]
        );
        assert!(!h.automation.is_active());
    }

    #[test]
    fn asks_for_password_and_persists_it_after_success() {
        let mut h = Harness::new(vec![step("oracle", None)]);
        h.feed(0, PROMPT);
        assert_eq!(
            h.feed(10, "su - oracle\r\nPassword: "),
            [notify(SwitchUserNotice::SecretRequired { user: "oracle".into(), error: None })]
        );
        // 入力を待つ間はタイムアウトしない
        assert!(h.tick(60_000).is_empty());
        let now = h.at(60_000);
        assert_eq!(h.automation.provide_secret(Some(("pw".into(), true)), now), [send("pw\r")]);
        assert_eq!(
            h.feed(60_010, "\r\n[oracle@web01 ~]$ "),
            [
                Action::PersistSecret { user: "oracle".into(), secret: "pw".into(), remember: true },
                switched("oracle"),
                notify(SwitchUserNotice::Finished),
            ]
        );
    }

    #[test]
    fn detects_japanese_password_prompt() {
        let mut h = Harness::new(vec![step("oracle", Some("pw"))]);
        h.feed(0, PROMPT);
        assert_eq!(h.feed(10, "su - oracle\r\nパスワード: "), [send("pw\r")]);
    }

    #[test]
    fn retries_with_new_password_after_auth_failure() {
        let mut h = Harness::new(vec![step("oracle", Some("wrong"))]);
        h.feed(0, PROMPT);
        h.feed(10, "su - oracle\r\nPassword: ");
        let actions = h.feed(2_000, "\r\nsu: Authentication failure\r\n[app@web01 ~]$ ");
        let [switching, resend] = start("oracle");
        assert_eq!(
            actions,
            [
                notify(SwitchUserNotice::AuthFailed {
                    user: "oracle".into(),
                    message: "su: Authentication failure".into(),
                }),
                switching,
                resend,
            ]
        );
        // 保存済みの誤った値は使わず、理由を添えて入力を求める
        assert_eq!(
            h.feed(2_010, "su - oracle\r\nPassword: "),
            [notify(SwitchUserNotice::SecretRequired {
                user: "oracle".into(),
                error: Some("su: Authentication failure".into()),
            })]
        );
    }

    #[test]
    fn ignores_failure_words_from_login_scripts_after_switching() {
        let mut h = Harness::new(vec![step("oracle", Some("pw"))]);
        h.feed(0, PROMPT);
        h.feed(10, "su - oracle\r\nPassword: ");
        assert_eq!(
            h.feed(20, "\r\nORACLE_HOME does not exist\r\n[oracle@web01 ~]$ "),
            [switched("oracle"), notify(SwitchUserNotice::Finished)]
        );
    }

    #[test]
    fn runs_second_step_right_after_first_succeeds() {
        let mut h = Harness::new(vec![step("app2", Some("a")), step("oracle", Some("b"))]);
        h.feed(0, PROMPT);
        h.feed(10, "su - app2\r\nPassword: ");
        let [switching, command] = start("oracle");
        assert_eq!(h.feed(20, "\r\n[app2@web01 ~]$ "), [switched("app2"), switching, command]);
        assert_eq!(h.feed(30, "su - oracle\r\nPassword: "), [send("b\r")]);
    }

    #[test]
    fn completes_without_password_when_not_asked() {
        let mut h = Harness::new(vec![step("oracle", None)]);
        h.feed(0, "[root@web01 ~]# ");
        assert_eq!(
            h.feed(10, "su - oracle\r\n[oracle@web01 ~]$ "),
            [switched("oracle"), notify(SwitchUserNotice::Finished)]
        );
    }

    #[test]
    fn does_not_treat_echoed_command_line_as_result() {
        let mut h = Harness::new(vec![step("oracle", Some("pw"))]);
        h.feed(0, PROMPT);
        // コマンドのエコー（改行前）はプロンプトで始まるが、応答ではない
        assert!(h.feed(10, "su - oracle").is_empty());
        assert!(h.tick(1_000).is_empty());
    }

    #[test]
    fn never_sends_password_without_password_prompt() {
        let mut h = Harness::new(vec![step("oracle", Some("pw"))]);
        h.feed(0, PROMPT);
        h.feed(10, "su - oracle\r\n");
        assert_eq!(
            h.tick(10_010),
            [notify(SwitchUserNotice::Failed {
                user: "oracle".into(),
                message: "the password prompt was not detected (password not sent)".into(),
            })]
        );
        assert!(!h.automation.is_active());
    }

    #[test]
    fn aborts_when_user_does_not_exist() {
        let mut h = Harness::new(vec![step("nobody2", Some("pw"))]);
        h.feed(0, PROMPT);
        assert_eq!(
            h.feed(10, "su - nobody2\r\nsu: user nobody2 does not exist\r\n[app@web01 ~]$ "),
            [notify(SwitchUserNotice::Failed {
                user: "nobody2".into(),
                message: "su: user nobody2 does not exist".into(),
            })]
        );
    }

    #[test]
    fn fails_when_shell_prompt_never_appears() {
        let mut h = Harness::new(vec![step("oracle", Some("pw"))]);
        let actions = h.tick(10_000);
        assert!(matches!(actions[..], [Action::Notify(SwitchUserNotice::Failed { .. })]));
    }

    #[test]
    fn cancel_at_password_prompt_exits_su() {
        let mut h = Harness::new(vec![step("oracle", None)]);
        h.feed(0, PROMPT);
        h.feed(10, "su - oracle\r\nPassword: ");
        let now = h.at(20);
        assert_eq!(
            h.automation.provide_secret(None, now),
            [send(CTRL_C), notify(SwitchUserNotice::Cancelled { user: "oracle".into() })]
        );
        assert!(!h.automation.is_active());
    }

    #[test]
    fn cancel_before_command_sends_nothing() {
        let mut h = Harness::new(vec![step("oracle", None)]);
        assert_eq!(
            h.automation.cancel(),
            [notify(SwitchUserNotice::Cancelled { user: "oracle".into() })]
        );
    }

    #[test]
    fn serializes_notice_for_frontend() {
        let json = serde_json::to_string(&SwitchUserNotice::SecretRequired {
            user: "oracle".into(),
            error: None,
        })
        .unwrap();
        assert_eq!(json, r#"{"kind":"secretRequired","user":"oracle","error":null}"#);
    }
}
