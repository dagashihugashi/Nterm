import { useEffect, useRef, useState, type PointerEvent, type ReactNode } from 'react';

export type WidthLimits = {
  defaultWidth: number;
  minWidth: number;
  maxWidth: number;
  /** ターミナルの表示領域を残すため、ウィンドウ幅に対する上限の割合 */
  maxWidthRatio: number;
};

type ResizablePanelProps = {
  limits: WidthLimits;
  /** 幅の記憶に使うキー（利用者ごとの利便性のため。消えても既定の幅に戻るだけ） */
  storageKey: string;
  /** 幅を変える取っ手を置く辺（左のパネルは right、右のパネルは left） */
  handleEdge: 'left' | 'right';
  children: ReactNode;
};

export function clampWidth(width: number, windowWidth: number, limits: WidthLimits): number {
  const max = Math.max(limits.minWidth, Math.min(limits.maxWidth, windowWidth * limits.maxWidthRatio));
  return Math.round(Math.min(max, Math.max(limits.minWidth, width)));
}

function readSavedWidth(storageKey: string, limits: WidthLimits): number {
  try {
    const saved = Number(localStorage.getItem(storageKey));
    return Number.isFinite(saved) && saved > 0
      ? clampWidth(saved, window.innerWidth, limits)
      : limits.defaultWidth;
  } catch {
    return limits.defaultWidth;
  }
}

function saveWidth(storageKey: string, width: number) {
  try {
    localStorage.setItem(storageKey, String(width));
  } catch {
    // 記憶できなくても、次回は既定の幅になるだけ
  }
}

/** 幅をドラッグで変えられるパネル（ダブルクリックで既定の幅に戻す・幅を記憶する） */
export function ResizablePanel({ limits, storageKey, handleEdge, children }: ResizablePanelProps) {
  const [width, setWidth] = useState(() => readSavedWidth(storageKey, limits));
  const [isResizing, setIsResizing] = useState(false);
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  // ウィンドウを狭めたときに、パネルがターミナルの領域を圧迫しないよう縮める
  useEffect(() => {
    const handleResize = () => setWidth((current) => clampWidth(current, window.innerWidth, limits));
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, [limits]);

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
    // 左端の取っ手は、左へ動かすほど広くなる
    const delta = (event.clientX - drag.startX) * (handleEdge === 'right' ? 1 : -1);
    setWidth(clampWidth(drag.startWidth + delta, window.innerWidth, limits));
  };

  const endResize = () => {
    if (!dragRef.current) return;
    dragRef.current = null;
    setIsResizing(false);
    saveWidth(storageKey, width);
  };

  const resetWidth = () => {
    const reset = clampWidth(limits.defaultWidth, window.innerWidth, limits);
    setWidth(reset);
    saveWidth(storageKey, reset);
  };

  return (
    <div className="relative flex shrink-0 min-h-0" style={{ width }}>
      {children}
      {/* 幅を変える取っ手。カードの間のすき間に置く */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={width}
        aria-valuemin={limits.minWidth}
        aria-valuemax={limits.maxWidth}
        title="Drag to resize. Double-click to reset."
        onPointerDown={startResize}
        onPointerMove={resize}
        onPointerUp={endResize}
        onPointerCancel={endResize}
        onDoubleClick={resetWidth}
        className={`absolute top-2 bottom-2 z-10 w-[6px] rounded-full cursor-ew-resize transition-colors ${
          handleEdge === 'right' ? '-right-[6px]' : '-left-[6px]'
        } ${isResizing ? 'bg-[var(--nt-bar-bg)]' : 'hover:bg-[var(--nt-bar-bg)]/60'}`}
      />
    </div>
  );
}
