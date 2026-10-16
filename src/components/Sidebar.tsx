import type { ReactNode } from 'react';
import { ResizablePanel, type WidthLimits } from './ResizablePanel';

/** サイドバーの表示内容（左端のアクティビティバーで切り替える） */
export type SidebarView = 'servers' | 'files';

type SidebarProps = {
  children: ReactNode;
  /** 表示内容によらず下部に表示する（転送の一覧など） */
  footer?: ReactNode;
};

const WIDTH_LIMITS: WidthLimits = {
  defaultWidth: 256,
  minWidth: 180,
  maxWidth: 600,
  maxWidthRatio: 0.5,
};

/** サイドバーの外枠。右端のドラッグで幅を変えられる。 */
export function Sidebar({ children, footer }: SidebarProps) {
  return (
    <ResizablePanel limits={WIDTH_LIMITS} storageKey="nterm.sidebarWidth" handleEdge="right">
      {/* 角丸のカード（中身の角も丸く切り抜く） */}
      <div className="flex-1 min-w-0 flex flex-col min-h-0 bg-[var(--nt-sidebar-bg)] rounded-lg border border-white/10 overflow-hidden">
        <div className="flex-1 min-h-0 overflow-y-auto">{children}</div>
        {footer}
      </div>
    </ResizablePanel>
  );
}
