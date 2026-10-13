import { invoke } from '@tauri-apps/api/core';
import type { ITheme } from '@xterm/xterm';
import type { ServerConfig } from '../types/server';

/** ターミナル（xterm.js）に渡す色のキー */
const TERMINAL_KEYS = [
  'background',
  'foreground',
  'cursor',
  'selectionBackground',
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
] as const;

type TerminalKey = (typeof TERMINAL_KEYS)[number];

/** Rust 側 `themes.rs` のキーと対応（読み込み時に default-dark で補われ、すべてそろっている） */
export type AppTheme = Record<TerminalKey, string> & {
  /** サイドバー・タブバーの背景 */
  sidebarBackground: string;
  /** ステータスバーとタイトルバー（最小化・最大化・閉じるボタンのあるバー） */
  barBackground: string;
  barForeground: string;
};

export type ThemeMap = Record<string, AppTheme>;

/** Rust 側 `themes::LoadedThemes` */
interface LoadedThemes {
  themes: ThemeMap;
  warning: string | null;
}

export const DEFAULT_THEME_NAME = 'default-dark';
/** env が production のサーバの既定テーマ（settings.json の envThemes 実装までの組み込みの対応） */
export const PRODUCTION_THEME_NAME = 'danger-red';

/** themes.json を読み込むまでの表示に使う（Rust 側の組み込みの default-dark と同じ） */
export const FALLBACK_THEME: AppTheme = {
  background: '#1e1e1e',
  foreground: '#d4d4d4',
  cursor: '#ffffff',
  selectionBackground: '#264f78',
  black: '#000000',
  red: '#cd3131',
  green: '#0dbc79',
  yellow: '#e5e510',
  blue: '#2472c8',
  magenta: '#bc3fbc',
  cyan: '#11a8cd',
  white: '#e5e5e5',
  brightBlack: '#666666',
  brightRed: '#f14c4c',
  brightGreen: '#23d18b',
  brightYellow: '#f5f543',
  brightBlue: '#3b8eea',
  brightMagenta: '#d670d6',
  brightCyan: '#29b8db',
  brightWhite: '#e5e5e5',
  sidebarBackground: '#252526',
  barBackground: '#007acc',
  barForeground: '#ffffff',
};

export const isProduction = (server: ServerConfig | undefined): boolean =>
  server?.env?.trim().toLowerCase() === 'production';

/** サーバに適用するテーマ名（サーバ設定の theme → env に応じた既定） */
export function themeNameFor(server: ServerConfig | undefined): string {
  if (server?.theme) return server.theme;
  return isProduction(server) ? PRODUCTION_THEME_NAME : DEFAULT_THEME_NAME;
}

/** サーバに適用するテーマ（指定したテーマが themes.json にない場合は default-dark） */
export function resolveTheme(themes: ThemeMap, server: ServerConfig | undefined): AppTheme {
  return themes[themeNameFor(server)] ?? themes[DEFAULT_THEME_NAME] ?? FALLBACK_THEME;
}

/** xterm.js に渡すターミナルの色 */
export function terminalTheme(theme: AppTheme): ITheme {
  return Object.fromEntries(TERMINAL_KEYS.map((key) => [key, theme[key]]));
}

export const loadThemes = (): Promise<LoadedThemes> => invoke<LoadedThemes>('load_themes');

/** タイトルバーの色を変える（Windows 11 のみ。対応していなければ false） */
export const setTitleBarColor = (background: string, foreground: string): Promise<boolean> =>
  invoke<boolean>('set_title_bar_color', { background, foreground });
