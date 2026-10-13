import { Channel, invoke } from '@tauri-apps/api/core';

export type TransferKind = 'upload' | 'download';
export type TransferStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

/** 転送パネルに表示する1件分の転送 */
export interface TransferItem {
  id: string;
  /** 転送に使うタブのセッション */
  sessionId: string;
  serverName: string;
  kind: TransferKind;
  /** ファイル名 */
  name: string;
  /** 転送先（リモートのディレクトリ または ローカルのフォルダ） */
  destination: string;
  status: TransferStatus;
  transferred: number;
  /** ファイルサイズ（不明なら null） */
  total: number | null;
  /** 転送を開始した時刻（待機中は null） */
  startedAt: number | null;
  /** 失敗時の理由 */
  message: string | null;
}

/** Rust 側 `transfer::TransferEvent` と対応 */
export type TransferEvent =
  | { event: 'started'; data: { total: number | null } }
  | { event: 'progress'; data: { transferred: number } }
  | { event: 'completed'; data: { transferred: number } }
  | { event: 'failed'; data: { message: string } }
  | { event: 'cancelled' };

/** Rust 側 `transfer::TransferDirection` と対応 */
export type TransferDirection =
  | { kind: 'upload'; localPath: string; remoteDir: string }
  | { kind: 'download'; remotePath: string; localDir: string };

export interface TransferRequest {
  transferId: string;
  sessionId: string;
  direction: TransferDirection;
  overwrite: boolean;
}

/** 転送をキューに追加する（転送はタブごとに順に処理され、進行状況は onEvent に届く） */
export function startTransfer(
  request: TransferRequest,
  onEvent: (event: TransferEvent) => void,
): Promise<void> {
  const channel = new Channel<TransferEvent>();
  channel.onmessage = onEvent;
  return invoke('start_transfer', { request, onEvent: channel });
}

export const cancelTransfer = (transferId: string): Promise<void> =>
  invoke('cancel_transfer', { transferId });

/** リモートの各パスが存在するか（アップロード時の上書き確認） */
export const remoteExists = (sessionId: string, paths: string[]): Promise<boolean[]> =>
  invoke<boolean[]>('sftp_exists', { sessionId, paths });

/** ローカルのフォルダに同名のファイルがあるか（ダウンロード時の上書き確認） */
export const localFileExists = (dir: string, name: string): Promise<boolean> =>
  invoke<boolean>('local_file_exists', { dir, name });

export const isActive = (item: TransferItem): boolean =>
  item.status === 'queued' || item.status === 'running';

/** パスの最後の要素（`\` と `/` の両方を区切りとして扱う） */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  return trimmed.slice(Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\')) + 1);
}

/** リモートのディレクトリとファイル名を `/` で結合する */
export function joinRemote(dir: string, name: string): string {
  return dir.endsWith('/') ? `${dir}${name}` : `${dir}/${name}`;
}

const SIZE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < SIZE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${SIZE_UNITS[unit]}`;
}

export function formatDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}
