export type AuthType = 'password' | 'publickey' | 'keyboard-interactive';

/** Rust 側 `config::SwitchUserStep`（ログイン後のユーザ切り替えの1手順） */
export interface SwitchUserStep {
  user: string;
  /** 省略時は `su - {user}` */
  command?: string | null;
}

/** 切り替えコマンドの既定値（Rust 側 `DEFAULT_SWITCH_COMMAND` と同じ） */
export const DEFAULT_SWITCH_COMMAND = 'su - {user}';
/** 切り替え手順の上限（Rust 側 `MAX_SWITCH_STEPS` と同じ） */
export const MAX_SWITCH_STEPS = 5;

/** Rust 側 `config::ServerConfig` と対応（Option は null で届く） */
export interface ServerConfig {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  auth: {
    type: AuthType;
    keyFile: string | null;
    remember: boolean;
  };
  env: string | null;
  theme: string | null;
  encoding: string | null;
  /** 未設定の場合は省略される */
  switchUsers?: SwitchUserStep[];
}

/** Rust 側 `config::ServerInput` と対応（サーバ追加・編集ダイアログの入力） */
export interface ServerInput {
  name: string;
  host: string;
  port: number;
  user: string;
  authType: AuthType;
  keyFile: string | null;
  env: string | null;
  encoding: string | null;
  switchUsers: SwitchUserStep[];
}

/** サーバ追加ダイアログで選べる認証方式（keyboard-interactive は未対応） */
export const AUTH_TYPE_OPTIONS = [
  { value: 'password', label: 'Password' },
  { value: 'publickey', label: 'Public key' },
] as const satisfies readonly { value: AuthType; label: string }[];

export const ENCODINGS = ['UTF-8', 'Shift_JIS', 'EUC-JP'] as const;
export const ENV_SUGGESTIONS = ['production', 'staging', 'development'] as const;
export const DEFAULT_ENV = 'default';
