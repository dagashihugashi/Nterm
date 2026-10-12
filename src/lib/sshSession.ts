import { Channel, invoke } from '@tauri-apps/api/core';
import type { Terminal } from '@xterm/xterm';

/** Rust 側 `ConnectError::HostKey(HostKeyPrompt)` と対応 */
export interface HostKeyPrompt {
  kind: 'hostKey';
  host: string;
  port: number;
  keyType: string;
  fingerprint: string;
  /** true: 保存済みの鍵と不一致（なりすましの可能性） / false: 未登録 */
  changed: boolean;
}

export interface CredentialRequest {
  serverId: string;
  /** 直前の認証失敗の理由（初回入力時は undefined） */
  error?: string;
}

export interface CredentialAnswer {
  /** パスワード認証ではパスワード、鍵認証ではパスフレーズ（空なら暗号化なしの鍵） */
  secret: string;
  /** 認証成功時に保存する */
  remember: boolean;
}

/** 接続中にユーザーへ問い合わせる処理。参照が変わると再接続されるため、呼び出し側で固定すること */
export interface SessionPrompts {
  /** true でホスト鍵を信頼して接続を続行 */
  confirmHostKey: (prompt: HostKeyPrompt) => Promise<boolean>;
  /** null でキャンセル */
  requestCredential: (request: CredentialRequest) => Promise<CredentialAnswer | null>;
}

/** Rust 側 `ConnectError`（hostKey 以外） */
type ConnectError =
  | { kind: 'credentialRequired' }
  | { kind: 'authFailed'; message: string }
  | { kind: 'failed'; message: string };

/** Rust 側 `ConnectOutcome` */
interface ConnectOutcome {
  warning: string | null;
}

/** Rust 側 `SshEvent` と対応 */
type SshEvent =
  | { event: 'data'; data: string }
  | { event: 'closed'; data: { reason: string | null } };

export type SessionState = 'connecting' | 'connected' | 'closed';

/** セッションの状態変化の通知（SFTP ツリー等、同じセッションを使う機能のため） */
export interface SessionInfo {
  sessionId: string;
  state: SessionState;
}

const writeNotice = (term: Terminal, color: number, message: string) =>
  term.writeln(`\r\n\x1b[${color}m${message}\x1b[0m`);

const errorKind = (error: unknown): string | undefined =>
  typeof error === 'object' && error !== null
    ? String((error as { kind?: unknown }).kind)
    : undefined;

const isHostKeyPrompt = (error: unknown): error is HostKeyPrompt => errorKind(error) === 'hostKey';

/** Rust 側のエラー（文字列 または `{ kind, message }`）を表示用の文字列にする */
const errorMessage = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
};

/**
 * xterm.js の端末を SSH セッションに接続する。
 * - パスワード未保存・認証失敗時は `requestCredential` で入力を求めて再接続する
 * - ホスト鍵が未登録・不一致の場合は `confirmHostKey` で確認し、承認されたら登録して再接続する
 * 戻り値の関数を呼ぶとセッションを切断し、購読を解除する（React effect の cleanup 用）。
 */
export function attachSshSession(
  term: Terminal,
  serverId: string,
  prompts: SessionPrompts,
  onSessionChange?: (info: SessionInfo) => void,
): () => void {
  // 接続ごとに一意なIDを使い、StrictMode の再マウント等で前回のセッションと混線しないようにする
  const sessionId = crypto.randomUUID();
  let state: SessionState = 'connecting';
  const setState = (next: SessionState) => {
    if (state === next) return;
    state = next;
    onSessionChange?.({ sessionId, state: next });
  };
  let disposed = false;
  /** 入力された資格情報。null の間は保存済みの値を使う */
  let credential: CredentialAnswer | null = null;

  const disconnect = () =>
    invoke('disconnect_ssh', { sessionId }).catch((error: unknown) =>
      console.error('disconnect_ssh failed', error),
    );

  const fail = (message: string) => {
    setState('closed');
    if (!disposed) writeNotice(term, 31, message);
  };

  /**
   * 出力受信用のチャネル。接続を試みるたびに新しく作る。
   * Rust 側の Channel が破棄される（接続失敗で connect_ssh が終了する）と、
   * フロント側の受信処理も解除されるため、再接続で使い回すと出力を受け取れなくなる。
   */
  const createEventChannel = () => {
    const onEvent = new Channel<SshEvent>();
    onEvent.onmessage = (message) => {
      if (disposed) return;
      if (message.event === 'data') {
        term.write(message.data);
        return;
      }
      setState('closed');
      const reason = message.data.reason ? `: ${message.data.reason}` : '';
      writeNotice(term, 33, `[Disconnected${reason}]`);
    };
    return onEvent;
  };

  const askCredential = async (error?: string) => {
    if (error) writeNotice(term, 31, `Authentication failed: ${error}`);
    const answer = await prompts.requestCredential({ serverId, error });
    if (disposed) return;
    if (!answer) {
      fail('Connection cancelled.');
      return;
    }
    credential = answer;
    term.writeln('\x1b[36mConnecting...\x1b[0m');
    connect();
  };

  const handleHostKey = async (prompt: HostKeyPrompt) => {
    writeNotice(
      term,
      prompt.changed ? 31 : 33,
      prompt.changed ? 'WARNING: The host key has changed!' : 'The host key is not registered yet.',
    );
    const accepted = await prompts.confirmHostKey(prompt);
    if (disposed) return;
    if (!accepted) {
      fail('Connection aborted: the host key was not trusted.');
      return;
    }
    try {
      await invoke('trust_host_key', { host: prompt.host, port: prompt.port });
    } catch (error: unknown) {
      fail(`Failed to save the host key: ${errorMessage(error)}`);
      return;
    }
    term.writeln('\x1b[36mHost key saved. Reconnecting...\x1b[0m');
    connect();
  };

  const handleConnectError = (error: unknown) => {
    if (isHostKeyPrompt(error)) {
      void handleHostKey(error);
      return;
    }
    const kind = errorKind(error) as ConnectError['kind'] | undefined;
    if (kind === 'credentialRequired') {
      void askCredential();
    } else if (kind === 'authFailed') {
      void askCredential(errorMessage(error));
    } else {
      fail(`Connection failed: ${errorMessage(error)}`);
    }
  };

  const connect = () => {
    invoke<ConnectOutcome>('connect_ssh', {
      request: {
        sessionId,
        serverId,
        secret: credential?.secret ?? null,
        remember: credential?.remember ?? false,
        cols: term.cols,
        rows: term.rows,
      },
      onEvent: createEventChannel(),
    })
      .then((outcome) => {
        if (disposed) {
          void disconnect();
          return;
        }
        if (state === 'connecting') setState('connected');
        if (outcome.warning) writeNotice(term, 33, `Warning: ${outcome.warning}`);
      })
      .catch((error: unknown) => {
        if (!disposed) handleConnectError(error);
      });
  };

  term.writeln('\x1b[36mConnecting...\x1b[0m');
  connect();

  const reportError = (error: unknown) => {
    if (!disposed) writeNotice(term, 31, `[Error: ${errorMessage(error)}]`);
  };
  const dataSubscription = term.onData((data) => {
    if (state === 'connected') invoke('write_ssh', { sessionId, data }).catch(reportError);
  });
  const resizeSubscription = term.onResize(({ cols, rows }) => {
    if (state === 'connected') invoke('resize_ssh', { sessionId, cols, rows }).catch(reportError);
  });

  return () => {
    disposed = true;
    dataSubscription.dispose();
    resizeSubscription.dispose();
    // 接続中(connecting)の場合は connect_ssh 完了時に切断される
    if (state === 'connected') void disconnect();
  };
}
