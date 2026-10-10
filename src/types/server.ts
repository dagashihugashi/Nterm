export type AuthType = 'password' | 'publickey' | 'keyboard-interactive';

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
}

/** サーバ追加ダイアログで選べる認証方式（keyboard-interactive は未対応） */
export const AUTH_TYPE_OPTIONS = [
  { value: 'password', label: 'Password' },
  { value: 'publickey', label: 'Public key' },
] as const satisfies readonly { value: AuthType; label: string }[];

export const ENCODINGS = ['UTF-8', 'Shift_JIS', 'EUC-JP'] as const;
export const ENV_SUGGESTIONS = ['production', 'staging', 'development'] as const;
export const DEFAULT_ENV = 'default';
