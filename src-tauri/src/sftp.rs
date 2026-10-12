//! SFTP によるリモートファイルの閲覧。
//! ターミナルと同じ SSH セッション上に SFTP チャネルを開き、セッションスレッド上で処理する
//! （libssh2 のセッションはスレッド間で同時に操作できないため）。

use std::cmp::Ordering;
use std::path::Path;
use std::sync::mpsc::Sender;

use serde::Serialize;
use ssh2::{Session, Sftp};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    /// リモートの絶対パス（`/` 区切り）
    pub path: String,
    /// ディレクトリ、またはディレクトリを指すシンボリックリンク
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: Option<u64>,
    /// 最終更新日時（UNIX 秒）
    pub modified: Option<u64>,
}

/// セッションスレッドへの SFTP 要求。結果は `reply` で返す。
pub enum SftpRequest {
    Home {
        reply: Sender<Result<String, String>>,
    },
    List {
        path: String,
        reply: Sender<Result<Vec<RemoteEntry>, String>>,
    },
}

/// セッションスレッドが保持する SFTP の状態。SFTP チャネルは最初の要求時に開く。
#[derive(Default)]
pub struct SftpClient {
    sftp: Option<Sftp>,
}

impl SftpClient {
    /// 要求を処理して結果を返信する。SFTP の操作はブロッキングで行い、終了後にノンブロッキングへ戻す。
    pub fn handle(&mut self, session: &Session, request: SftpRequest) {
        session.set_blocking(true);
        // 送信失敗＝要求元がタイムアウト等で待つのをやめているため、結果は捨ててよい
        match request {
            SftpRequest::Home { reply } => {
                let _ = reply.send(self.home(session));
            }
            SftpRequest::List { path, reply } => {
                let _ = reply.send(self.list(session, &path));
            }
        }
        session.set_blocking(false);
    }

    fn sftp(&mut self, session: &Session) -> Result<&Sftp, String> {
        if self.sftp.is_none() {
            let opened = session.sftp().map_err(|e| {
                format!("SFTP is not available on this server (the SFTP subsystem may be disabled): {e}")
            })?;
            self.sftp = Some(opened);
        }
        self.sftp
            .as_ref()
            .ok_or_else(|| "SFTP is not available".to_string())
    }

    fn home(&mut self, session: &Session) -> Result<String, String> {
        let home = self
            .sftp(session)?
            .realpath(Path::new("."))
            .map_err(|e| format!("failed to resolve home directory: {e}"))?;
        Ok(to_remote_path(&home.to_string_lossy()))
    }

    fn list(&mut self, session: &Session, dir: &str) -> Result<Vec<RemoteEntry>, String> {
        let sftp = self.sftp(session)?;
        let listing = sftp
            .readdir(Path::new(dir))
            .map_err(|e| format!("failed to list {dir}: {e}"))?;
        let entries = listing
            .into_iter()
            .filter_map(|(path, stat)| {
                let name = path.file_name()?.to_string_lossy().into_owned();
                let entry_path = join_remote(dir, &name);
                let is_symlink = stat.file_type().is_symlink();
                // シンボリックリンクはリンク先を確認してディレクトリかどうかを判定する
                let is_dir = stat.is_dir()
                    || (is_symlink
                        && sftp
                            .stat(Path::new(&entry_path))
                            .is_ok_and(|target| target.is_dir()));
                Some(RemoteEntry {
                    name,
                    path: entry_path,
                    is_dir,
                    is_symlink,
                    size: stat.size,
                    modified: stat.mtime,
                })
            })
            .collect();
        Ok(sort_entries(entries))
    }
}

/// サーバから返ったパスを `/` 区切りにそろえる（Windows 上の ssh2 は区切りを `\` にすることがある）
fn to_remote_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// リモートのディレクトリとファイル名を `/` で結合する
pub fn join_remote(dir: &str, name: &str) -> String {
    let dir = to_remote_path(dir);
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// ディレクトリを先に、それぞれ名前順（大文字小文字を区別しない）に並べた一覧を返す
pub fn sort_entries(entries: Vec<RemoteEntry>) -> Vec<RemoteEntry> {
    let mut sorted = entries;
    sorted.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => a
            .name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name)),
    });
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> RemoteEntry {
        RemoteEntry {
            name: name.to_string(),
            path: join_remote("/home/app", name),
            is_dir,
            is_symlink: false,
            size: None,
            modified: None,
        }
    }

    #[test]
    fn joins_with_slash() {
        assert_eq!(join_remote("/home/app", "logs"), "/home/app/logs");
        assert_eq!(join_remote("/home/app/", "logs"), "/home/app/logs");
        assert_eq!(join_remote("/", "etc"), "/etc");
    }

    #[test]
    fn normalizes_backslashes_from_windows_paths() {
        assert_eq!(join_remote("\\home\\app", "logs"), "/home/app/logs");
        assert_eq!(to_remote_path("/home\\app"), "/home/app");
    }

    #[test]
    fn sorts_directories_first_then_by_name() {
        let sorted = sort_entries(vec![
            entry("b.txt", false),
            entry("Zeta", true),
            entry("a.txt", false),
            entry("alpha", true),
        ]);
        let names: Vec<&str> = sorted.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "a.txt", "b.txt"]);
    }

    #[test]
    fn sort_is_case_insensitive_and_stable_for_same_name() {
        let sorted = sort_entries(vec![entry("readme", false), entry("README", false)]);
        let names: Vec<&str> = sorted.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["README", "readme"]);
    }
}
