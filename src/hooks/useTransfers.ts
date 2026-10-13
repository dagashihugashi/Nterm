import { useCallback, useState } from 'react';
import {
  baseName,
  cancelTransfer,
  isActive,
  joinRemote,
  localFileExists,
  remoteExists,
  startTransfer,
  type TransferDirection,
  type TransferEvent,
  type TransferItem,
} from '../lib/transfer';

export type ConflictAction = 'overwrite' | 'skip' | 'cancel';

/** 転送先に同名のファイルがある場合の確認 */
export interface ConflictRequest {
  /** 転送先のパス */
  path: string;
  /** 未確認の競合の件数（この件を含む）。2件以上なら「残りすべてに適用」を選べる */
  remaining: number;
}

export interface ConflictAnswer {
  action: ConflictAction;
  applyToAll: boolean;
}

/** 転送先（タブのサーバ） */
export interface TransferTarget {
  sessionId: string;
  serverName: string;
}

interface Selected {
  index: number;
  overwrite: boolean;
}

/** 同名のファイルがあるものを確認し、転送する対象（上書きするか）を返す。キャンセルされたら null */
export async function resolveConflicts(
  targets: readonly string[],
  exists: readonly boolean[],
  ask: (request: ConflictRequest) => Promise<ConflictAnswer>,
): Promise<Selected[] | null> {
  let remaining = exists.filter(Boolean).length;
  let remembered: ConflictAction | null = null;
  const selected: Selected[] = [];
  for (const [index, target] of targets.entries()) {
    if (!exists[index]) {
      selected.push({ index, overwrite: false });
      continue;
    }
    let action: ConflictAction | null = remembered;
    if (action === null) {
      const answer = await ask({ path: target, remaining });
      action = answer.action;
      if (answer.applyToAll) remembered = action;
    }
    remaining -= 1;
    if (action === 'cancel') return null;
    if (action === 'overwrite') selected.push({ index, overwrite: true });
  }
  return selected;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

type UseTransfersOptions = {
  askConflict: (request: ConflictRequest) => Promise<ConflictAnswer>;
  /** 転送を始められなかった場合など、ステータスバーに表示するエラー */
  onError: (message: string) => void;
  /** アップロードが完了した（ファイルツリーの再読み込みに使う） */
  onUploaded?: (sessionId: string, remoteDir: string) => void;
};

/**
 * ファイル転送（アップロード・ダウンロード）の一覧と操作。
 * 転送自体は Rust 側の別スレッド・別接続で行われ、ここには間引かれた進捗だけが届く。
 */
export function useTransfers({ askConflict, onError, onUploaded }: UseTransfersOptions) {
  const [transfers, setTransfers] = useState<TransferItem[]>([]);

  const update = useCallback((id: string, changes: Partial<TransferItem>) => {
    setTransfers((prev) => prev.map((item) => (item.id === id ? { ...item, ...changes } : item)));
  }, []);

  const applyEvent = (item: TransferItem, event: TransferEvent) => {
    switch (event.event) {
      case 'started':
        update(item.id, { status: 'running', total: event.data.total, transferred: 0, startedAt: Date.now() });
        break;
      case 'progress':
        update(item.id, { transferred: event.data.transferred });
        break;
      case 'completed':
        update(item.id, { status: 'completed', transferred: event.data.transferred });
        if (item.kind === 'upload') onUploaded?.(item.sessionId, item.destination);
        break;
      case 'failed':
        update(item.id, { status: 'failed', message: event.data.message });
        break;
      case 'cancelled':
        update(item.id, { status: 'cancelled' });
        break;
    }
  };

  const enqueue = (
    target: TransferTarget,
    name: string,
    destination: string,
    direction: TransferDirection,
    overwrite: boolean,
  ) => {
    const item: TransferItem = {
      id: crypto.randomUUID(),
      sessionId: target.sessionId,
      serverName: target.serverName,
      kind: direction.kind,
      name,
      destination,
      status: 'queued',
      transferred: 0,
      total: null,
      startedAt: null,
      message: null,
    };
    setTransfers((prev) => [...prev, item]);
    startTransfer(
      { transferId: item.id, sessionId: target.sessionId, direction, overwrite },
      (event) => applyEvent(item, event),
    ).catch((error: unknown) => update(item.id, { status: 'failed', message: errorText(error) }));
  };

  /** ローカルのファイルをリモートのディレクトリへアップロードする */
  const upload = async (target: TransferTarget, localPaths: string[], remoteDir: string) => {
    const names = localPaths.map(baseName);
    const remotePaths = names.map((name) => joinRemote(remoteDir, name));
    try {
      const exists = await remoteExists(target.sessionId, remotePaths);
      const selected = await resolveConflicts(remotePaths, exists, askConflict);
      selected?.forEach(({ index, overwrite }) =>
        enqueue(target, names[index], remoteDir, {
          kind: 'upload',
          localPath: localPaths[index],
          remoteDir,
        }, overwrite),
      );
    } catch (error: unknown) {
      onError(`Upload failed: ${errorText(error)}`);
    }
  };

  /** リモートのファイルをローカルのフォルダへダウンロードする */
  const download = async (target: TransferTarget, remotePath: string, localDir: string) => {
    const name = baseName(remotePath);
    try {
      const exists = await localFileExists(localDir, name);
      const selected = await resolveConflicts([`${localDir}\\${name}`], [exists], askConflict);
      if (selected?.length) {
        enqueue(target, name, localDir, { kind: 'download', remotePath, localDir }, selected[0].overwrite);
      }
    } catch (error: unknown) {
      onError(`Download failed: ${errorText(error)}`);
    }
  };

  const cancel = (id: string) => {
    cancelTransfer(id).catch((error: unknown) => onError(`Failed to cancel: ${errorText(error)}`));
  };

  /** 終了した転送を一覧から消す */
  const dismiss = (id: string) =>
    setTransfers((prev) => prev.filter((item) => item.id !== id || isActive(item)));

  const clearFinished = () => setTransfers((prev) => prev.filter(isActive));

  /** 指定したタブで実行中・待機中の転送の件数 */
  const activeCountFor = (sessionId: string) =>
    transfers.filter((item) => item.sessionId === sessionId && isActive(item)).length;

  return { transfers, upload, download, cancel, dismiss, clearFinished, activeCountFor };
}
