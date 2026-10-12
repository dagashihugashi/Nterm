import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowUp, ChevronDown, ChevronRight, File, Folder, FolderOpen, RefreshCw } from 'lucide-react';

/** Rust 側 `sftp::RemoteEntry` と対応 */
interface RemoteEntry {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number | null;
  /** UNIX 秒 */
  modified: number | null;
}

type DirState =
  | { status: 'loading' }
  | { status: 'loaded'; entries: RemoteEntry[] }
  | { status: 'error'; message: string };

type RemoteFileTreeProps = {
  /** 接続中のセッション。セッションが変わる場合は key で作り直すこと */
  sessionId: string;
};

const INDENT_PX = 12;
const BASE_PADDING_PX = 8;
const SIZE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

export function parentPath(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  const index = trimmed.lastIndexOf('/');
  return index <= 0 ? '/' : trimmed.slice(0, index);
}

function formatSize(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < SIZE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${SIZE_UNITS[unit]}`;
}

function describeEntry(entry: RemoteEntry): string {
  const lines = [entry.path];
  if (entry.isSymlink) lines.push('(symbolic link)');
  if (!entry.isDir && entry.size !== null) lines.push(formatSize(entry.size));
  if (entry.modified !== null) lines.push(new Date(entry.modified * 1000).toLocaleString());
  return lines.join('\n');
}

const iconButtonClass =
  'p-0.5 rounded text-gray-400 hover:text-white hover:bg-[#37373d] disabled:opacity-30 disabled:hover:bg-transparent';

/** 接続中セッションのリモートファイルを SFTP でツリー表示する */
export function RemoteFileTree({ sessionId }: RemoteFileTreeProps) {
  const [root, setRoot] = useState<string | null>(null);
  const [rootError, setRootError] = useState<string | null>(null);
  const [dirs, setDirs] = useState<Record<string, DirState>>({});
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());

  const loadDir = useCallback(
    async (path: string) => {
      setDirs((prev) => ({ ...prev, [path]: { status: 'loading' } }));
      try {
        const entries = await invoke<RemoteEntry[]>('sftp_list', { sessionId, path });
        setDirs((prev) => ({ ...prev, [path]: { status: 'loaded', entries } }));
      } catch (error: unknown) {
        setDirs((prev) => ({ ...prev, [path]: { status: 'error', message: String(error) } }));
      }
    },
    [sessionId],
  );

  useEffect(() => {
    invoke<string>('sftp_home', { sessionId })
      .then((home) => {
        setRoot(home);
        void loadDir(home);
      })
      .catch((error: unknown) => setRootError(String(error)));
  }, [sessionId, loadDir]);

  const ensureLoaded = (path: string) => {
    if (dirs[path]?.status !== 'loaded') void loadDir(path);
  };

  const toggle = (entry: RemoteEntry) => {
    const isOpen = expanded.has(entry.path);
    setExpanded((prev) => {
      const next = new Set(prev);
      if (isOpen) next.delete(entry.path);
      else next.add(entry.path);
      return next;
    });
    if (!isOpen) ensureLoaded(entry.path);
  };

  const changeRoot = (path: string) => {
    setRoot(path);
    ensureLoaded(path);
  };

  const refresh = () => {
    if (!root) return;
    const paths = [root, ...expanded];
    setDirs({});
    paths.forEach((path) => void loadDir(path));
  };

  const renderDir = (path: string, depth: number): ReactNode => {
    const state = dirs[path];
    const padding = { paddingLeft: BASE_PADDING_PX + depth * INDENT_PX + 18 };
    if (!state || state.status === 'loading') {
      return <div className="py-0.5 text-xs text-gray-500" style={padding}>Loading…</div>;
    }
    if (state.status === 'error') {
      return (
        <div className="py-0.5 pr-2 text-xs text-[#f48771] break-all select-text" style={padding}>
          {state.message}
        </div>
      );
    }
    if (state.entries.length === 0) {
      return <div className="py-0.5 text-xs text-gray-500" style={padding}>(empty)</div>;
    }
    return state.entries.map((entry) => renderEntry(entry, depth));
  };

  const renderEntry = (entry: RemoteEntry, depth: number): ReactNode => {
    const isOpen = entry.isDir && expanded.has(entry.path);
    const Chevron = isOpen ? ChevronDown : ChevronRight;
    const Icon = entry.isDir ? (isOpen ? FolderOpen : Folder) : File;
    return (
      <div key={entry.path}>
        <div
          title={describeEntry(entry)}
          className={`flex items-center gap-1 py-0.5 pr-2 text-sm hover:bg-[#37373d] ${
            entry.isDir ? 'cursor-pointer text-gray-200' : 'text-gray-400'
          }`}
          style={{ paddingLeft: BASE_PADDING_PX + depth * INDENT_PX }}
          onClick={entry.isDir ? () => toggle(entry) : undefined}
          onDoubleClick={entry.isDir ? () => changeRoot(entry.path) : undefined}
        >
          {entry.isDir ? <Chevron size={14} className="shrink-0" /> : <span className="w-[14px] shrink-0" />}
          <Icon size={14} className={`shrink-0 ${entry.isDir ? 'text-[#dcb67a]' : 'text-[#c5c5c5]'}`} />
          <span className={`truncate ${entry.isSymlink ? 'italic' : ''}`}>{entry.name}</span>
        </div>
        {isOpen && renderDir(entry.path, depth + 1)}
      </div>
    );
  };

  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-[#3c3c3c]">
        <span className="flex-1 truncate text-xs text-gray-300 select-text" title={root ?? ''}>
          {root ?? '…'}
        </span>
        <button
          type="button"
          title="Parent directory"
          aria-label="Parent directory"
          className={iconButtonClass}
          disabled={!root || root === '/'}
          onClick={() => root && changeRoot(parentPath(root))}
        >
          <ArrowUp size={14} />
        </button>
        <button
          type="button"
          title="Refresh"
          aria-label="Refresh"
          className={iconButtonClass}
          disabled={!root}
          onClick={refresh}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      <div className="flex-1 overflow-auto py-1">
        {rootError && (
          <div className="mx-2 p-2 text-xs text-[#f48771] bg-[#5a1d1d] border border-[#be1100] rounded break-all select-text">
            {rootError}
          </div>
        )}
        {!rootError && !root && <div className="px-3 py-1 text-xs text-gray-500">Loading…</div>}
        {root && renderDir(root, 0)}
      </div>
    </div>
  );
}
