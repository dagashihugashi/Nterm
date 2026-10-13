import { useEffect, useRef, useState, type PointerEvent, type ReactNode } from 'react';
import { FolderTree, Server } from 'lucide-react';

export type SidebarView = 'servers' | 'files';

type SidebarProps = {
  view: SidebarView;
  onViewChange: (view: SidebarView) => void;
  children: ReactNode;
  /** 表示内容によらず下部に表示する（転送の一覧など） */
  footer?: ReactNode;
};

const VIEWS = [
  { value: 'servers', label: 'Servers', Icon: Server },
  { value: 'files', label: 'Files', Icon: FolderTree },
] as const satisfies readonly { value: SidebarView; label: string; Icon: typeof Server }[];

const DEFAULT_WIDTH = 256;
const MIN_WIDTH = 180;
const MAX_WIDTH = 600;
/** ターミナルの表示領域を残すため、ウィンドウ幅に対する上限の割合 */
const MAX_WIDTH_RATIO = 0.5;
/** 幅の記憶（利用者ごとの利便性のため。消えても既定の幅に戻るだけ） */
const WIDTH_STORAGE_KEY = 'nterm.sidebarWidth';

export function clampWidth(width: number, windowWidth: number): number {
  const max = Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, windowWidth * MAX_WIDTH_RATIO));
  return Math.round(Math.min(max, Math.max(MIN_WIDTH, width)));
}

function readSavedWidth(): number {
  try {
    const saved = Number(localStorage.getItem(WIDTH_STORAGE_KEY));
    return Number.isFinite(saved) && saved > 0 ? clampWidth(saved, window.innerWidth) : DEFAULT_WIDTH;
  } catch {
    return DEFAULT_WIDTH;
  }
}

function saveWidth(width: number) {
  try {
    localStorage.setItem(WIDTH_STORAGE_KEY, String(width));
  } catch {
    // 記憶できなくても、次回は既定の幅になるだけ
  }
}

/** サイドバーの外枠と、表示内容（サーバ一覧 / リモートファイル）の切り替え。右端のドラッグで幅を変えられる。 */
export function Sidebar({ view, onViewChange, children, footer }: SidebarProps) {
  const [width, setWidth] = useState(readSavedWidth);
  const [isResizing, setIsResizing] = useState(false);
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  // ウィンドウを狭めたときに、サイドバーがターミナルの領域を圧迫しないよう縮める
  useEffect(() => {
    const handleResize = () => setWidth((current) => clampWidth(current, window.innerWidth));
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, []);

  const startResize = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { startX: event.clientX, startWidth: width };
    setIsResizing(true);
  };

  const resize = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    setWidth(clampWidth(drag.startWidth + event.clientX - drag.startX, window.innerWidth));
  };

  const endResize = () => {
    if (!dragRef.current) return;
    dragRef.current = null;
    setIsResizing(false);
    saveWidth(width);
  };

  const resetWidth = () => {
    const reset = clampWidth(DEFAULT_WIDTH, window.innerWidth);
    setWidth(reset);
    saveWidth(reset);
  };

  return (
    <div className="relative flex shrink-0 min-h-0" style={{ width }}>
      {/* 角丸のカード（中身の角も丸く切り抜く） */}
      <div className="flex-1 min-w-0 flex flex-col min-h-0 bg-[var(--nt-sidebar-bg)] rounded-lg border border-white/10 overflow-hidden">
        <div role="tablist" className="flex border-b border-white/10 shrink-0">
          {VIEWS.map(({ value, label, Icon }) => {
            const isActive = view === value;
            return (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={isActive}
                onClick={() => onViewChange(value)}
                className={`flex-1 flex items-center justify-center gap-1.5 py-1.5 text-xs border-b-2 ${
                  isActive
                    ? 'text-[var(--nt-fg)] border-[var(--nt-bar-bg)]'
                    : 'text-gray-400 border-transparent hover:text-gray-200'
                }`}
              >
                <Icon size={13} />
                {label}
              </button>
            );
          })}
        </div>
        <div className="flex-1 min-h-0 overflow-y-auto">{children}</div>
        {footer}
      </div>
      {/* 幅を変える取っ手。カードの間のすき間に置く */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={width}
        aria-valuemin={MIN_WIDTH}
        aria-valuemax={MAX_WIDTH}
        title="Drag to resize. Double-click to reset."
        onPointerDown={startResize}
        onPointerMove={resize}
        onPointerUp={endResize}
        onPointerCancel={endResize}
        onDoubleClick={resetWidth}
        className={`absolute top-2 bottom-2 -right-[6px] z-10 w-[6px] rounded-full cursor-ew-resize transition-colors ${
          isResizing ? 'bg-[var(--nt-bar-bg)]' : 'hover:bg-[var(--nt-bar-bg)]/60'
        }`}
      />
    </div>
  );
}
