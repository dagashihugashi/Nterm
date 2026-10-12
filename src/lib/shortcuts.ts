/**
 * アプリのショートカット（設計書 Tech-stack ④ keybindings）。
 * ターミナルにフォーカスがあるときもリモートへ送らず、App が処理する。
 * リモート側で使う Ctrl+C / Ctrl+V / Ctrl+W 等と衝突しないよう、Ctrl+Shift / Alt / F キーを使う。
 * コピー・貼り付け（Ctrl+Shift+C / V）はターミナル固有のため terminalClipboard.ts で扱う。
 */
export type AppAction =
  | 'toggleLog'
  | 'nextTab'
  | 'prevTab'
  | 'closeTab'
  | 'duplicateTab'
  | 'renameTab';

interface KeyBinding {
  action: AppAction;
  code: string;
  ctrl?: boolean;
  shift?: boolean;
  alt?: boolean;
}

const KEY_BINDINGS: readonly KeyBinding[] = [
  { action: 'toggleLog', code: 'KeyL', alt: true },
  { action: 'nextTab', code: 'Tab', ctrl: true },
  { action: 'prevTab', code: 'Tab', ctrl: true, shift: true },
  { action: 'closeTab', code: 'KeyW', ctrl: true, shift: true },
  { action: 'duplicateTab', code: 'KeyD', ctrl: true, shift: true },
  { action: 'renameTab', code: 'F2' },
];

/** キー入力がアプリのショートカットなら、その操作を返す */
export function matchAppShortcut(event: KeyboardEvent): AppAction | null {
  if (event.type !== 'keydown' || event.metaKey) return null;
  const binding = KEY_BINDINGS.find(
    (b) =>
      b.code === event.code &&
      Boolean(b.ctrl) === event.ctrlKey &&
      Boolean(b.shift) === event.shiftKey &&
      Boolean(b.alt) === event.altKey,
  );
  return binding?.action ?? null;
}

/** ターミナル以外の入力欄（タブ名の編集・ダイアログ等）を操作中か */
export function isEditingText(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    target.matches('input, textarea, select, [contenteditable="true"]') &&
    !target.classList.contains('xterm-helper-textarea')
  );
}
