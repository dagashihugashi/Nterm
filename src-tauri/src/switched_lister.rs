//! su した後のユーザ（基本は root）の権限で、リモートのファイル一覧を取得する（ファイルツリー用）。
//!
//! ターミナルとは別の隠しシェルチャネルを開き、ターミナルと同じ手順で su してから
//! `find <dir> -mindepth 1 -maxdepth 1 -printf ...` で一覧を取得する（読み取りのみ）。
//! 出力の前後に目印の行を出し、プロンプト等と区別する。目印は printf の引数を分けて組み立てるため、
//! コマンドのエコー（`__NTERM_ BEGIN_1`）と実際の出力（`__NTERM_BEGIN_1`）は一致しない。

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use ssh2::Channel;

use crate::sftp::{join_remote, sort_entries, RemoteEntry};
use crate::switch_user::{Action, SwitchStep, SwitchUserAutomation, SwitchUserNotice};

const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const MARKER_PREFIX: &str = "__NTERM_";
const READ_BUFFER_SIZE: usize = 16 * 1024;
/// 一覧取得のコマンドを履歴に残さない（root の .bash_history を汚さない）・入力を表示させない
const SETUP_COMMAND: &str = "unset HISTFILE; stty -echo\r";

pub type ListReply = Sender<Result<Vec<RemoteEntry>, String>>;

pub enum ListerEvent {
    /// su が完了し、このユーザの権限で一覧を取得できるようになった
    Ready { user: String },
    /// su に失敗した（ログインユーザの SFTP で表示を続ける）
    Failed { message: String },
}

enum State {
    Switching(Box<SwitchUserAutomation>),
    Ready,
    Failed,
}

struct Job {
    id: u64,
    dir: String,
    reply: ListReply,
    deadline: Instant,
    has_begun: bool,
    lines: Vec<String>,
}

pub struct SwitchedLister {
    channel: Channel,
    state: State,
    user: String,
    pending_input: Vec<u8>,
    /// 未処理の受信データ（一覧の取得中は行単位で処理する）
    output: Vec<u8>,
    queue: VecDeque<(String, ListReply)>,
    current: Option<Job>,
    next_id: u64,
}

/// 受信した1行の意味
#[derive(Debug, PartialEq)]
enum LineKind {
    Begin,
    End { status: i32 },
    Content,
    Ignore,
}

fn classify_line(id: u64, has_begun: bool, line: &str) -> LineKind {
    if !has_begun {
        let begin = format!("{MARKER_PREFIX}BEGIN_{id}");
        return if line.contains(&begin) { LineKind::Begin } else { LineKind::Ignore };
    }
    let end = format!("{MARKER_PREFIX}END_{id}_");
    match line.find(&end) {
        Some(index) => {
            let digits: String = line[index + end.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            LineKind::End { status: digits.parse().unwrap_or(-1) }
        }
        None => LineKind::Content,
    }
}

/// シェルのシングルクォートで囲む
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// 一覧取得のコマンド（目印の行 → find の出力 → 終了コード付きの目印の行）
fn list_command(dir: &str, id: u64) -> Result<String, String> {
    if !dir.starts_with('/') || dir.chars().any(char::is_control) {
        return Err(format!("invalid path: {dir}"));
    }
    Ok(format!(
        "printf '\\n%s%s\\n' {MARKER_PREFIX} BEGIN_{id}; \
         LC_ALL=C find {} -mindepth 1 -maxdepth 1 -printf '%y\\t%Y\\t%s\\t%T@\\t%f\\n' 2>/dev/null; \
         printf '%s%s_%s\\n' {MARKER_PREFIX} END_{id} $?\r",
        shell_quote(dir)
    ))
}

/// `find -printf '%y\t%Y\t%s\t%T@\t%f'` の出力を一覧にする（種類, リンク先の種類, サイズ, 更新日時, 名前）
fn parse_entries(dir: &str, lines: &[String]) -> Vec<RemoteEntry> {
    let entries = lines
        .iter()
        .filter_map(|line| {
            let mut fields = line.splitn(5, '\t');
            let (kind, target_kind, size, modified, name) =
                (fields.next()?, fields.next()?, fields.next()?, fields.next()?, fields.next()?);
            if name.is_empty() || name.contains('/') {
                return None;
            }
            Some(RemoteEntry {
                name: name.to_string(),
                path: join_remote(dir, name),
                is_dir: target_kind == "d",
                is_symlink: kind == "l",
                size: size.parse().ok(),
                modified: modified.split('.').next().and_then(|s| s.parse().ok()),
            })
        })
        .collect();
    sort_entries(entries)
}

impl SwitchedLister {
    /// 隠しシェルで su を始める。手順がなければ `None`。
    pub fn start(channel: Channel, steps: Vec<SwitchStep>, now: Instant) -> Option<Self> {
        let user = steps.last()?.user.clone();
        let automation = SwitchUserAutomation::new(steps, now)?;
        Some(Self {
            channel,
            state: State::Switching(Box::new(automation)),
            user,
            pending_input: Vec::new(),
            output: Vec::new(),
            queue: VecDeque::new(),
            current: None,
            next_id: 1,
        })
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.state, State::Ready)
    }

    /// 一覧の取得を依頼する（準備ができている場合のみ呼ぶ）
    pub fn enqueue(&mut self, dir: String, reply: ListReply) {
        self.queue.push_back((dir, reply));
    }

    /// 受信・送信・タイムアウトを処理する。チャネルが使えなくなった場合は Err。
    pub fn poll(&mut self, now: Instant) -> Result<Vec<ListerEvent>, String> {
        let mut events = Vec::new();
        let mut buf = vec![0u8; READ_BUFFER_SIZE];
        loop {
            match self.channel.read(&mut buf) {
                Ok(0) if self.channel.eof() => return Err("the file tree shell was closed".to_string()),
                Ok(0) => break,
                Ok(n) => self.receive(&buf[..n], now, &mut events),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => return Err(format!("the file tree shell failed: {e}")),
            }
        }
        if let State::Switching(automation) = &mut self.state {
            let actions = automation.tick(now);
            self.apply_actions(actions, &mut events);
        }
        if self.is_ready() {
            self.expire_job(now);
            self.start_next_job(now);
        }
        self.flush()?;
        Ok(events)
    }

    fn receive(&mut self, bytes: &[u8], now: Instant, events: &mut Vec<ListerEvent>) {
        match &mut self.state {
            State::Switching(automation) => {
                let actions = automation.feed(&String::from_utf8_lossy(bytes), now);
                self.apply_actions(actions, events);
            }
            State::Ready => {
                self.output.extend_from_slice(bytes);
                self.process_lines();
            }
            State::Failed => {}
        }
    }

    fn apply_actions(&mut self, actions: Vec<Action>, events: &mut Vec<ListerEvent>) {
        for action in actions {
            match action {
                Action::Send(text) => self.pending_input.extend_from_slice(text.as_bytes()),
                Action::PersistSecret { .. } => {}
                Action::Notify(SwitchUserNotice::Finished) => {
                    self.state = State::Ready;
                    self.pending_input.extend_from_slice(SETUP_COMMAND.as_bytes());
                    events.push(ListerEvent::Ready { user: self.user.clone() });
                }
                Action::Notify(
                    SwitchUserNotice::Failed { message, .. } | SwitchUserNotice::AuthFailed { message, .. },
                ) => self.fail(message, events),
                Action::Notify(SwitchUserNotice::SecretRequired { user, .. }) => {
                    self.fail(format!("the password for {user} is not available"), events);
                }
                Action::Notify(SwitchUserNotice::Cancelled { .. }) => {
                    self.fail("cancelled".to_string(), events);
                }
                Action::Notify(_) => {}
            }
        }
    }

    fn fail(&mut self, message: String, events: &mut Vec<ListerEvent>) {
        if !matches!(self.state, State::Failed) {
            self.state = State::Failed;
            events.push(ListerEvent::Failed { message });
        }
    }

    fn process_lines(&mut self) {
        while let Some(end) = self.output.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.output.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw).trim_end_matches(['\r', '\n']).to_string();
            self.handle_line(line);
        }
    }

    fn handle_line(&mut self, line: String) {
        let Some(job) = self.current.as_mut() else {
            return;
        };
        match classify_line(job.id, job.has_begun, &line) {
            LineKind::Begin => job.has_begun = true,
            LineKind::Content => job.lines.push(line),
            LineKind::Ignore => {}
            LineKind::End { status } => {
                let Some(job) = self.current.take() else {
                    return;
                };
                let result = if status == 0 {
                    Ok(parse_entries(&job.dir, &job.lines))
                } else {
                    Err(format!("failed to list {} (exit status {status})", job.dir))
                };
                // 要求元が待つのをやめている場合は届かなくてよい
                let _ = job.reply.send(result);
            }
        }
    }

    fn expire_job(&mut self, now: Instant) {
        if self.current.as_ref().is_some_and(|job| now >= job.deadline) {
            if let Some(job) = self.current.take() {
                let _ = job.reply.send(Err(format!("listing {} timed out", job.dir)));
            }
        }
    }

    fn start_next_job(&mut self, now: Instant) {
        while self.current.is_none() {
            let Some((dir, reply)) = self.queue.pop_front() else {
                return;
            };
            let id = self.next_id;
            self.next_id += 1;
            match list_command(&dir, id) {
                Ok(command) => {
                    self.pending_input.extend_from_slice(command.as_bytes());
                    self.current = Some(Job {
                        id,
                        dir,
                        reply,
                        deadline: now + LIST_TIMEOUT,
                        has_begun: false,
                        lines: Vec::new(),
                    });
                }
                Err(message) => {
                    let _ = reply.send(Err(message));
                }
            }
        }
    }

    /// 未送信の入力を書き込む（WouldBlock の場合は次のループで再送する）
    fn flush(&mut self) -> Result<(), String> {
        if self.pending_input.is_empty() {
            return Ok(());
        }
        match self.channel.write(&self.pending_input) {
            Ok(written) => {
                self.pending_input.drain(..written);
                Ok(())
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(format!("the file tree shell failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_list_command_with_markers_split_from_echo() {
        let command = list_command("/var/log", 3).unwrap();
        assert!(command.contains("find '/var/log' -mindepth 1 -maxdepth 1"));
        // エコーされるコマンドの中に、目印そのもの（__NTERM_BEGIN_3）は現れない
        assert!(!command.contains("__NTERM_BEGIN_3"));
        assert!(!command.contains("__NTERM_END_3_"));
        assert!(command.ends_with('\r'));
    }

    #[test]
    fn quotes_paths_with_single_quotes() {
        let command = list_command("/tmp/it's", 1).unwrap();
        assert!(command.contains(r"find '/tmp/it'\''s'"));
    }

    #[test]
    fn rejects_relative_paths_and_control_characters() {
        assert!(list_command("tmp", 1).is_err());
        assert!(list_command("/tmp\nrm -rf /", 1).is_err());
    }

    #[test]
    fn classifies_marker_lines() {
        assert_eq!(classify_line(3, false, "# printf '\\n%s%s\\n' __NTERM_ BEGIN_3"), LineKind::Ignore);
        assert_eq!(classify_line(3, false, "__NTERM_BEGIN_3"), LineKind::Begin);
        assert_eq!(classify_line(3, true, "d\td\t4096\t1.0\tetc"), LineKind::Content);
        assert_eq!(classify_line(3, true, "__NTERM_END_3_0"), LineKind::End { status: 0 });
        assert_eq!(classify_line(3, true, "__NTERM_END_3_1"), LineKind::End { status: 1 });
        // 別の要求の目印には反応しない
        assert_eq!(classify_line(3, false, "__NTERM_BEGIN_2"), LineKind::Ignore);
    }

    #[test]
    fn parses_find_output() {
        let lines = [
            "f\tf\t1234\t1700000000.5000000000\tmessages",
            "d\td\t4096\t1700000001.0000000000\tnginx",
            "l\td\t7\t1700000002.0000000000\tbin",
            "l\tN\t9\t1700000003.0000000000\tbroken link",
        ]
        .map(String::from);
        let entries = parse_entries("/var/log", &lines);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        // ディレクトリ（リンク先がディレクトリを含む）が先、名前順
        assert_eq!(names, ["bin", "nginx", "broken link", "messages"]);
        let bin = &entries[0];
        assert!(bin.is_dir && bin.is_symlink);
        assert_eq!(bin.path, "/var/log/bin");
        let messages = &entries[3];
        assert_eq!((messages.size, messages.modified), (Some(1234), Some(1_700_000_000)));
    }
}
