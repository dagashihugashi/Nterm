import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { Server, X } from 'lucide-react';

/** タブIDは接続ごとに一意。同じサーバを複数タブで開ける。 */
export interface Tab {
  id: string;
  title: string;
  serverId: string;
}

/** ドラッグ開始とみなす移動量（クリックと区別する） */
const DRAG_THRESHOLD_PX = 5;
const MAX_TITLE_LENGTH = 100;

/** 指定タブを toIndex（他のタブの並びでの位置）へ移した新しい配列を返す */
export function moveTab(tabs: readonly Tab[], id: string, toIndex: number): Tab[] {
  const from = tabs.findIndex((t) => t.id === id);
  if (from < 0 || from === toIndex) return [...tabs];
  const rest = tabs.filter((t) => t.id !== id);
  return [...rest.slice(0, toIndex), tabs[from], ...rest.slice(toIndex)];
}

type RenameInputProps = {
  initialTitle: string;
  onCommit: (title: string) => void;
  onCancel: () => void;
};

/** タブ名の編集欄。Enter・フォーカス外れで確定、Esc で取り消し。空欄なら変更しない。 */
function RenameInput({ initialTitle, onCommit, onCancel }: RenameInputProps) {
  const [value, setValue] = useState(initialTitle);
  // Enter で確定した後の blur で二重に確定しないようにする
  const isDoneRef = useRef(false);

  const finish = (commit: boolean) => {
    if (isDoneRef.current) return;
    isDoneRef.current = true;
    const title = value.trim();
    if (commit && title !== '' && title !== initialTitle) onCommit(title);
    else onCancel();
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') finish(true);
    if (event.key === 'Escape') finish(false);
  };

  return (
    <input
      autoFocus
      value={value}
      maxLength={MAX_TITLE_LENGTH}
      onChange={(event) => setValue(event.target.value)}
      onKeyDown={handleKeyDown}
      onBlur={() => finish(true)}
      onFocus={(event) => event.target.select()}
      onPointerDown={(event) => event.stopPropagation()}
      className="flex-1 min-w-0 px-1 bg-[#3c3c3c] border border-[#007acc] rounded text-white outline-none select-text"
    />
  );
}

type TabBarProps = {
  tabs: readonly Tab[];
  activeTabId: string;
  /** ログ取得中のタブIDと書き込み先のパス */
  tabLogs: Readonly<Record<string, string>>;
  renamingTabId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  /** タブを他のタブの並びでの位置 toIndex へ移す */
  onMove: (id: string, toIndex: number) => void;
  onRenameStart: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onRenameEnd: () => void;
};

export function TabBar({
  tabs,
  activeTabId,
  tabLogs,
  renamingTabId,
  onSelect,
  onClose,
  onMove,
  onRenameStart,
  onRename,
  onRenameEnd,
}: TabBarProps) {
  const tabElements = useRef(new Map<string, HTMLDivElement>());
  const [draggingId, setDraggingId] = useState<string | null>(null);
  // ドラッグ中のリスナーから最新の値を参照する
  const latestRef = useRef({ tabs, onMove });
  const stopDragRef = useRef<(() => void) | null>(null);
  useEffect(() => {
    latestRef.current = { tabs, onMove };
  });
  useEffect(() => () => stopDragRef.current?.(), []);

  /** ポインタ位置より左にある（中心が左側の）他のタブの数 = 移動先の位置 */
  const dropIndex = (id: string, clientX: number) =>
    latestRef.current.tabs.filter((tab) => {
      const element = tabElements.current.get(tab.id);
      if (tab.id === id || !element) return false;
      const rect = element.getBoundingClientRect();
      return rect.left + rect.width / 2 < clientX;
    }).length;

  // HTML5 の Drag and Drop は Tauri のファイルドロップ処理と競合するため、ポインタイベントで実装する
  const startDrag = (event: PointerEvent<HTMLDivElement>, id: string) => {
    if (event.button !== 0 || renamingTabId === id) return;
    const startX = event.clientX;
    let isDragging = false;

    const handleMove = (moveEvent: globalThis.PointerEvent) => {
      if (!isDragging && Math.abs(moveEvent.clientX - startX) < DRAG_THRESHOLD_PX) return;
      isDragging = true;
      setDraggingId(id);
      latestRef.current.onMove(id, dropIndex(id, moveEvent.clientX));
    };
    const stop = () => {
      window.removeEventListener('pointermove', handleMove);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
      stopDragRef.current = null;
      setDraggingId(null);
    };
    stopDragRef.current?.();
    stopDragRef.current = stop;
    window.addEventListener('pointermove', handleMove);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
  };

  return (
    <div className="flex shrink-0 min-h-[38px] bg-[var(--nt-sidebar-bg)] border-b border-white/10 items-end overflow-x-auto no-scrollbar">
      {tabs.map((tab) => {
        const isActive = tab.id === activeTabId;
        const logPath = tabLogs[tab.id];
        return (
          <div
            key={tab.id}
            ref={(element) => {
              if (element) tabElements.current.set(tab.id, element);
              else tabElements.current.delete(tab.id);
            }}
            onPointerDown={(event) => startDrag(event, tab.id)}
            onClick={() => onSelect(tab.id)}
            onDoubleClick={() => onRenameStart(tab.id)}
            title={tab.title}
            className={`px-4 py-2 text-sm cursor-pointer border-r border-[#3c3c3c] flex items-center gap-2 group min-w-[120px] max-w-[200px] ${
              isActive
                ? 'bg-[var(--nt-bg)] border-t-2 border-t-[var(--nt-bar-bg)] text-[var(--nt-fg)]'
                : 'bg-[var(--nt-sidebar-bg)] hover:bg-white/5 text-gray-400 border-t-2 border-t-transparent'
            } ${draggingId === tab.id ? 'opacity-60' : ''}`}
          >
            <Server size={14} className={isActive ? 'text-[var(--nt-bar-bg)] shrink-0' : 'text-gray-500 shrink-0'} />
            {renamingTabId === tab.id ? (
              <RenameInput
                initialTitle={tab.title}
                onCommit={(title) => {
                  onRename(tab.id, title);
                  onRenameEnd();
                }}
                onCancel={onRenameEnd}
              />
            ) : (
              <span className="truncate flex-1">{tab.title}</span>
            )}
            {logPath && (
              <span className="text-[10px] font-bold text-[#f14c4c] shrink-0" title={logPath}>
                ● REC
              </span>
            )}
            <X
              size={14}
              className={`hover:bg-[#444] rounded shrink-0 ${isActive ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}
              onPointerDown={(event) => event.stopPropagation()}
              onDoubleClick={(event) => event.stopPropagation()}
              onClick={(event) => {
                event.stopPropagation();
                onClose(tab.id);
              }}
            />
          </div>
        );
      })}
    </div>
  );
}
