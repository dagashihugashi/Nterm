import { useCallback, useEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import { ArrowUp, ChevronDown, ChevronRight, File, Folder, FolderOpen, RefreshCw } from 'lucide-react';
import { formatBytes } from '../lib/transfer';

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

/** 再読み込みの要求（seq が変わるたびに dir を読み込み直す） */
export interface TreeRefreshRequest {
  dir: string;
  seq: number;
}

type RemoteFileTreeProps = {
  /** 接続中のセッション。セッションが変わる場合は key で作り直すこと */
  sessionId: string;
  /** ローカルのファイルを指定したディレクトリへアップロードする */
  onUpload: (localPaths: string[], remoteDir: string) => void;
  /** リモートのファイルを指定したローカルのフォルダへダウンロードする */
  onDownload: (remotePath: string, localDir: string) => void;
  refreshRequest?: TreeRefreshRequest | null;
  /** su した後のユーザの権限で一覧を表示している場合のユーザ名（ログインユーザなら null） */
  viewUser?: string | null;
};

type ContextMenuState = { x: number; y: number; entry: RemoteEntry };

const INDENT_PX = 12;
const BASE_PADDING_PX = 8;
/** 前回選んだダウンロード先（利用者ごとの利便性のための記憶。消えても動作に影響しない） */
const LAST_DOWNLOAD_DIR_KEY = 'nterm.lastDownloadDir';

export function parentPath(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  const index = trimmed.lastIndexOf('/');
  return index <= 0 ? '/' : trimmed.slice(0, index);
}

function describeEntry(entry: RemoteEntry): string {
  const lines = [entry.path];
  if (entry.isSymlink) lines.push('(symbolic link)');
  if (!entry.isDir && entry.size !== null) lines.push(formatBytes(entry.size));
  if (entry.modified !== null) lines.push(new Date(entry.modified * 1000).toLocaleString());
  return lines.join('\n');
}

function readLastDownloadDir(): string | undefined {
  try {
    return localStorage.getItem(LAST_DOWNLOAD_DIR_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

function saveLastDownloadDir(dir: string) {
  try {
    localStorage.setItem(LAST_DOWNLOAD_DIR_KEY, dir);
  } catch {
    // 記憶できなくても、次回の初期フォルダが変わるだけ
  }
}

/** ドラッグ中のポインタ位置（物理ピクセル）の下にあるドロップ先ディレクトリ（data-drop-dir 属性） */
function dropDirAt(container: HTMLElement | null, x: number, y: number): string | null {
  const scale = window.devicePixelRatio || 1;
  const element = document.elementFromPoint(x / scale, y / scale);
  if (!container || !element || !container.contains(element)) return null;
  return element.closest<HTMLElement>('[data-drop-dir]')?.dataset.dropDir ?? null;
}

const iconButtonClass =
  'p-0.5 rounded text-gray-400 hover:text-white hover:bg-[#37373d] disabled:opacity-30 disabled:hover:bg-transparent';
const menuItemClass = 'w-full px-3 py-1 text-left hover:bg-[#094771]';

/** 接続中セッションのリモートファイルを SFTP でツリー表示する。ファイルのドロップでアップロード、右クリックでダウンロード。 */
export function RemoteFileTree({ sessionId, onUpload, onDownload, refreshRequest, viewUser }: RemoteFileTreeProps) {
  const [root, setRoot] = useState<string | null>(null);
  const [rootError, setRootError] = useState<string | null>(null);
  const [dirs, setDirs] = useState<Record<string, DirState>>({});
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const treeRef = useRef<HTMLDivElement>(null);
  // ドラッグ＆ドロップのリスナーから最新の値を参照する
  const onUploadRef = useRef(onUpload);
  useEffect(() => {
    onUploadRef.current = onUpload;
  });

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

  // アップロード完了後、そのディレクトリを読み込み直す
  useEffect(() => {
    if (refreshRequest) void loadDir(refreshRequest.dir);
  }, [refreshRequest, loadDir]);

  // エクスプローラーからのファイルのドロップ（Tauri のファイルドロップ。ファイルのパスが取得できる）
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type === 'leave') {
          setDropTarget(null);
          return;
        }
        const dir = dropDirAt(treeRef.current, payload.position.x, payload.position.y);
        if (payload.type === 'drop') {
          setDropTarget(null);
          if (dir && payload.paths.length > 0) onUploadRef.current(payload.paths, dir);
          return;
        }
        setDropTarget(dir);
      })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch((error: unknown) => setRootError(`Drag and drop is unavailable: ${String(error)}`));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // コンテキストメニューは外側のクリック・Esc で閉じる
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close();
    };
    window.addEventListener('pointerdown', close);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('pointerdown', close);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [menu]);

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

  const openMenu = (event: MouseEvent, entry: RemoteEntry) => {
    event.preventDefault();
    setMenu({ x: event.clientX, y: event.clientY, entry });
  };

  const downloadEntry = async (entry: RemoteEntry) => {
    setMenu(null);
    const dir = await open({ directory: true, title: `Download ${entry.name} to`, defaultPath: readLastDownloadDir() });
    if (typeof dir !== 'string') return;
    saveLastDownloadDir(dir);
    onDownload(entry.path, dir);
  };

  const uploadInto = async (dir: string) => {
    setMenu(null);
    const selected = await open({ multiple: true, directory: false, title: `Upload files to ${dir}` });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    if (paths.length > 0) onUpload(paths, dir);
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
    // ファイルの行にドロップした場合は、そのファイルがあるディレクトリへアップロードする
    const dropDir = entry.isDir ? entry.path : parentPath(entry.path);
    const isDropTarget = entry.isDir && dropTarget === entry.path;
    return (
      <div key={entry.path} data-drop-dir={dropDir}>
        <div
          title={describeEntry(entry)}
          className={`flex items-center gap-1 py-0.5 pr-2 text-sm hover:bg-[#37373d] ${
            entry.isDir ? 'cursor-pointer text-gray-200' : 'text-gray-400'
          } ${isDropTarget ? 'bg-[#094771] outline outline-1 outline-[#007acc]' : ''}`}
          style={{ paddingLeft: BASE_PADDING_PX + depth * INDENT_PX }}
          onClick={entry.isDir ? () => toggle(entry) : undefined}
          onDoubleClick={entry.isDir ? () => changeRoot(entry.path) : undefined}
          onContextMenu={(event) => openMenu(event, entry)}
        >
          {entry.isDir ? <Chevron size={14} className="shrink-0" /> : <span className="w-[14px] shrink-0" />}
          <Icon size={14} className={`shrink-0 ${entry.isDir ? 'text-[#dcb67a]' : 'text-[#c5c5c5]'}`} />
          <span className={`truncate ${entry.isSymlink ? 'italic' : ''}`}>{entry.name}</span>
        </div>
        {isOpen && renderDir(entry.path, depth + 1)}
      </div>
    );
  };

  const isRootDropTarget = root !== null && dropTarget === root;

  return (
    <div className="flex flex-col min-h-0 h-full">
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-[#3c3c3c]">
        <span className="flex-1 truncate text-xs text-gray-300 select-text" title={root ?? ''}>
          {root ?? '…'}
        </span>
        {viewUser && (
          <span
            className="shrink-0 px-1 rounded text-[10px] bg-[var(--nt-bar-bg)] text-[var(--nt-bar-fg)]"
            title={`Listing as ${viewUser} (the user switched to after login). File transfers still use the login user.`}
          >
            as {viewUser}
          </span>
        )}
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
      <div
        ref={treeRef}
        data-drop-dir={root ?? undefined}
        className={`flex-1 overflow-auto py-1 ${isRootDropTarget ? 'bg-[#094771]/40 outline outline-1 outline-[#007acc]' : ''}`}
      >
        {rootError && (
          <div className="mx-2 p-2 text-xs text-[#f48771] bg-[#5a1d1d] border border-[#be1100] rounded break-all select-text">
            {rootError}
          </div>
        )}
        {!rootError && !root && <div className="px-3 py-1 text-xs text-gray-500">Loading…</div>}
        {root && renderDir(root, 0)}
        {root && (
          <div className="px-3 pt-2 text-[11px] text-gray-600">
            Drop files here to upload. Right-click a file to download.
          </div>
        )}
      </div>
      {menu && (
        <div
          role="menu"
          className="fixed z-40 min-w-[180px] py-1 bg-[#252526] border border-[#454545] rounded shadow-2xl text-sm text-[#d4d4d4]"
          style={{ left: menu.x, top: menu.y }}
          onPointerDown={(event) => event.stopPropagation()}
        >
          {menu.entry.isDir ? (
            <button type="button" role="menuitem" className={menuItemClass} onClick={() => void uploadInto(menu.entry.path)}>
              Upload files here...
            </button>
          ) : (
            <>
              <button type="button" role="menuitem" className={menuItemClass} onClick={() => void downloadEntry(menu.entry)}>
                Download...
              </button>
              <button
                type="button"
                role="menuitem"
                className={menuItemClass}
                onClick={() => void uploadInto(parentPath(menu.entry.path))}
              >
                Upload files to this folder...
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
