import { invoke } from '@tauri-apps/api/core';
import type { Terminal } from '@xterm/xterm';

/** ステータスバーに表示する結果 */
export interface StatusMessage {
  kind: 'info' | 'error';
  text: string;
}

/** 複数行の貼り付けの確認に使う内容 */
export interface PasteRequest {
  text: string;
  lineCount: number;
}

export interface ClipboardHandlers {
  onStatus: (message: StatusMessage) => void;
  /** true で貼り付ける */
  confirmPaste: (request: PasteRequest) => Promise<boolean>;
}

const LINE_BREAK = /\r\n|\r|\n/;
const TRAILING_LINE_BREAKS = /(?:\r\n|\r|\n)+$/;

const isCtrlShift = (event: KeyboardEvent, code: string) =>
  event.type === 'keydown' &&
  event.ctrlKey &&
  event.shiftKey &&
  !event.altKey &&
  !event.metaKey &&
  event.code === code;

/** 選択範囲のコピー（Ctrl+Shift+C）。Ctrl+C は常にリモートへ送る */
export const isCopyShortcut = (event: KeyboardEvent) => isCtrlShift(event, 'KeyC');
/** 貼り付け（Ctrl+Shift+V） */
export const isPasteShortcut = (event: KeyboardEvent) => isCtrlShift(event, 'KeyV');

/**
 * 末尾の改行を除き、行数を数える。
 * Excel のセル等は末尾に改行が付くため、そのまま送ると貼り付けた瞬間にコマンドが実行されてしまう。
 */
export function preparePaste(text: string): PasteRequest {
  const trimmed = text.replace(TRAILING_LINE_BREAKS, '');
  return { text: trimmed, lineCount: trimmed === '' ? 0 : trimmed.split(LINE_BREAK).length };
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`;

async function copySelection(term: Terminal, handlers: ClipboardHandlers) {
  const selection = term.getSelection();
  if (selection === '') {
    handlers.onStatus({ kind: 'info', text: 'Nothing selected' });
    return;
  }
  try {
    await invoke('clipboard_write', { text: selection });
    const lineCount = selection.split(LINE_BREAK).length;
    handlers.onStatus({ kind: 'info', text: `Copied ${plural(lineCount, 'line')}` });
  } catch (error: unknown) {
    handlers.onStatus({ kind: 'error', text: errorText(error) });
  }
}

async function pasteFromClipboard(
  term: Terminal,
  handlers: ClipboardHandlers,
  isDisposed: () => boolean,
) {
  let text: string | null;
  try {
    text = await invoke<string | null>('clipboard_read');
  } catch (error: unknown) {
    handlers.onStatus({ kind: 'error', text: errorText(error) });
    return;
  }
  const request = preparePaste(text ?? '');
  if (request.lineCount === 0) {
    handlers.onStatus({ kind: 'error', text: 'Nothing to paste' });
    return;
  }
  if (request.lineCount > 1 && !(await handlers.confirmPaste(request))) {
    handlers.onStatus({ kind: 'info', text: 'Paste cancelled' });
  } else if (!isDisposed()) {
    // 改行の変換（CR）と bracketed paste は xterm.js が行う
    term.paste(request.text);
  }
  // 確認ダイアログが閉じた後にフォーカスを戻すため、次のフレームで行う
  window.requestAnimationFrame(() => {
    if (!isDisposed()) term.focus();
  });
}

/**
 * ターミナルにコピー・貼り付けの操作を付ける。
 * - 右クリック: 貼り付け
 * - ブラウザ標準の貼り付け（Ctrl+V 等）は無効にし、確認を通る経路だけで貼り付ける
 * 戻り値の `handleKeyEvent` は xterm.js のカスタムキーハンドラから呼ぶ（false でリモートへ送らない）。
 */
export function attachClipboard(
  term: Terminal,
  container: HTMLElement,
  getHandlers: () => ClipboardHandlers,
) {
  let disposed = false;
  const isDisposed = () => disposed;
  const paste = () => void pasteFromClipboard(term, getHandlers(), isDisposed);

  const handleContextMenu = (event: MouseEvent) => {
    event.preventDefault();
    paste();
  };
  const blockNativePaste = (event: ClipboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
  };
  container.addEventListener('contextmenu', handleContextMenu);
  // xterm.js 内部の paste リスナーより先に止めるため、キャプチャで受ける
  container.addEventListener('paste', blockNativePaste, true);

  const handleKeyEvent = (event: KeyboardEvent): boolean => {
    if (isCopyShortcut(event)) {
      event.preventDefault();
      void copySelection(term, getHandlers());
      return false;
    }
    if (isPasteShortcut(event)) {
      event.preventDefault();
      paste();
      return false;
    }
    return true;
  };

  const dispose = () => {
    disposed = true;
    container.removeEventListener('contextmenu', handleContextMenu);
    container.removeEventListener('paste', blockNativePaste, true);
  };

  return { handleKeyEvent, dispose };
}
