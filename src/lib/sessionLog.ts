import { invoke } from '@tauri-apps/api/core';

/** Rust 側 `config::LogMode` と対応 */
export type LogMode = 'append' | 'overwrite';

/** Rust 側 `session_log::LogDefaults`（ログ開始ダイアログの初期値） */
export interface LogDefaults {
  dir: string;
  fileName: string;
  mode: LogMode;
}

export interface LogRequest {
  dir: string;
  fileName: string;
  mode: LogMode;
}

export const fetchLogDefaults = (serverId: string): Promise<LogDefaults> =>
  invoke<LogDefaults>('log_defaults', { serverId });

/** ログ取得を開始し、書き込み先のフルパスを返す */
export const startLog = (sessionId: string, request: LogRequest): Promise<string> =>
  invoke<string>('start_log', { sessionId, ...request });

export const stopLog = (sessionId: string): Promise<void> => invoke('stop_log', { sessionId });

/**
 * ログ取得の開始／停止のショートカット（Alt+L）。
 * ターミナルにフォーカスがあるときもリモートへ送らず、アプリ側で処理する。
 */
export const isToggleLogShortcut = (event: KeyboardEvent): boolean =>
  event.type === 'keydown' &&
  event.altKey &&
  !event.ctrlKey &&
  !event.shiftKey &&
  !event.metaKey &&
  event.code === 'KeyL';
