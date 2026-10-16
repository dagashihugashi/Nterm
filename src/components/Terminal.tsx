import React, { useEffect, useRef } from 'react';
import { Terminal as XTerm, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { attachSshSession, type SessionInfo, type SessionPrompts } from '../lib/sshSession';
import { matchAppShortcut } from '../lib/shortcuts';
import { attachClipboard, type PasteRequest, type StatusMessage } from '../lib/terminalClipboard';

interface TerminalComponentProps {
  /** 接続先（servers.json の id） */
  serverId: string;
  /** パスワード入力・ホスト鍵確認。参照が変わると再接続されるため、呼び出し側で固定すること */
  prompts: SessionPrompts;
  /** 接続状態の変化（参照が変わっても再接続しない） */
  onSessionChange?: (info: SessionInfo) => void;
  /** ログの書き込みに失敗し、ログ取得が止まった（参照が変わっても再接続しない） */
  onLogFailed?: (message: string) => void;
  /** コピー・貼り付けの結果（ステータスバー表示用） */
  onStatus: (message: StatusMessage) => void;
  /** 複数行の貼り付けの確認。true で貼り付ける */
  confirmPaste: (request: PasteRequest) => Promise<boolean>;
  isActive: boolean;
  /** 値が変わると（表示中なら）ターミナルにフォーカスを戻す（タブ名の編集後など） */
  focusToken: number;
  /** ターミナルの色（themes.json）。変わるとその場で反映する（再接続しない） */
  theme: ITheme;
  /** ファイルツリーの一覧を取得するユーザが変わった（null ならログインユーザ） */
  onTreeUser?: (user: string | null) => void;
}

function createTerminal(container: HTMLElement, theme: ITheme): { term: XTerm; fitAddon: FitAddon } {
  const term = new XTerm({
    fontFamily: 'Cascadia Mono, BIZ UDGothic, monospace',
    fontSize: 14,
    cursorStyle: 'block',
    cursorBlink: true,
    theme,
  });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.open(container);

  try {
    const webglAddon = new WebglAddon();
    webglAddon.onContextLoss(() => webglAddon.dispose());
    term.loadAddon(webglAddon);
  } catch (error) {
    console.warn('WebGL addon could not be loaded', error);
  }

  fitAddon.fit();
  return { term, fitAddon };
}

const EDITABLE_SELECTOR = 'input, textarea, select, [contenteditable="true"]';

/** ターミナルにフォーカスする。ただし他の入力欄（ダイアログ等）を操作中なら奪わない */
function focusTerminal(term: XTerm, container: HTMLElement) {
  const active = document.activeElement;
  const isEditingElsewhere =
    active instanceof HTMLElement && !container.contains(active) && active.matches(EDITABLE_SELECTOR);
  if (!isEditingElsewhere) term.focus();
}

export const TerminalComponent: React.FC<TerminalComponentProps> = ({
  serverId, prompts, onSessionChange, onLogFailed, onStatus, confirmPaste, isActive, focusToken, theme, onTreeUser
}) => {
  const terminalRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<XTerm | null>(null);
  // 作成時の色（以降の変更は下の effect で反映する）
  const themeRef = useRef(theme);

  // テーマの変更をその場で反映する（ターミナルを作り直すと再接続になるため）
  useEffect(() => {
    themeRef.current = theme;
    if (termRef.current) termRef.current.options.theme = theme;
  }, [theme]);
  // 最新の値を参照する（effect の依存に入れると再接続されてしまうため）
  const onSessionChangeRef = useRef(onSessionChange);
  const onLogFailedRef = useRef(onLogFailed);
  const onTreeUserRef = useRef(onTreeUser);
  const clipboardHandlersRef = useRef({ onStatus, confirmPaste });
  const isActiveRef = useRef(isActive);
  useEffect(() => {
    onSessionChangeRef.current = onSessionChange;
    onLogFailedRef.current = onLogFailed;
    onTreeUserRef.current = onTreeUser;
    clipboardHandlersRef.current = { onStatus, confirmPaste };
    isActiveRef.current = isActive;
  });

  // タブが表示されたら（または focusToken が変わったら）ターミナルにフォーカスする
  // （display: none 解除後・入力欄が消えた後に行うため次フレームで）
  useEffect(() => {
    const term = termRef.current;
    const container = terminalRef.current;
    if (!isActive || !term || !container) return;
    const frame = window.requestAnimationFrame(() => focusTerminal(term, container));
    return () => window.cancelAnimationFrame(frame);
  }, [isActive, focusToken]);

  useEffect(() => {
    const container = terminalRef.current;
    if (!container) return;

    const { term, fitAddon } = createTerminal(container, themeRef.current);
    termRef.current = term;

    const clipboard = attachClipboard(term, container, () => clipboardHandlersRef.current);
    // アプリのショートカットはリモートへ送らない（イベントは window へ伝わり、App が処理する）
    term.attachCustomKeyEventHandler(
      (event) => matchAppShortcut(event) === null && clipboard.handleKeyEvent(event),
    );

    const handleSessionChange = (info: SessionInfo) => {
      onSessionChangeRef.current?.(info);
      // 接続できたらすぐに操作できるよう、表示中のタブならフォーカスする
      if (info.state === 'connected' && isActiveRef.current) focusTerminal(term, container);
    };
    const detachSession = attachSshSession(term, serverId, prompts, {
      onSessionChange: handleSessionChange,
      onLogFailed: (message) => onLogFailedRef.current?.(message),
      onStatus: (message) => clipboardHandlersRef.current.onStatus(message),
      onTreeUser: (user) => onTreeUserRef.current?.(user),
    });

    let frame: number | undefined;
    const resizeObserver = new ResizeObserver(() => {
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        // 非表示タブ（display: none）ではサイズが取れないため fit をスキップする
        if (container.clientWidth > 0 && container.clientHeight > 0) fitAddon.fit();
      });
    });
    resizeObserver.observe(container);

    return () => {
      resizeObserver.disconnect();
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      detachSession();
      clipboard.dispose();
      termRef.current = null;
      term.dispose();
    };
  }, [serverId, prompts]);

  return (
    <div
      className={`w-full h-full p-2 ${isActive ? '' : 'hidden'}`}
      style={{ background: theme.background }}
    >
      <div ref={terminalRef} className="w-full h-full" />
    </div>
  );
};
