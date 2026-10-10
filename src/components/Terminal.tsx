import React, { useEffect, useRef } from 'react';
import { Terminal as XTerm, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { attachSshSession, type SessionPrompts } from '../lib/sshSession';

interface TerminalComponentProps {
  /** 未指定の場合はローカルのモックアップ表示 */
  serverId?: string;
  /** パスワード入力・ホスト鍵確認。参照が変わると再接続されるため、呼び出し側で固定すること */
  prompts: SessionPrompts;
  isActive: boolean;
}

const DEFAULT_THEME: ITheme = {
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
};

function createTerminal(container: HTMLElement): { term: XTerm; fitAddon: FitAddon } {
  const term = new XTerm({
    fontFamily: 'Cascadia Mono, BIZ UDGothic, monospace',
    fontSize: 14,
    cursorStyle: 'block',
    cursorBlink: true,
    theme: DEFAULT_THEME,
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

export const TerminalComponent: React.FC<TerminalComponentProps> = ({
  serverId, prompts, isActive
}) => {
  const terminalRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = terminalRef.current;
    if (!container) return;

    const { term, fitAddon } = createTerminal(container);

    const detachSession = serverId
      ? attachSshSession(term, serverId, prompts)
      : undefined;
    if (!serverId) {
      term.writeln('\x1b[1;32mWelcome to Nterm\x1b[0m');
      term.writeln('This is a local mockup tab. Select a server to connect.');
    }

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
      detachSession?.();
      term.dispose();
    };
  }, [serverId, prompts]);

  return (
    <div className={`w-full h-full p-2 bg-[#1e1e1e] ${isActive ? '' : 'hidden'}`}>
      <div ref={terminalRef} className="w-full h-full" />
    </div>
  );
};
