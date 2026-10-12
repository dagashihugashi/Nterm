import type { ReactNode } from 'react';
import { FolderTree, Server } from 'lucide-react';

export type SidebarView = 'servers' | 'files';

type SidebarProps = {
  view: SidebarView;
  onViewChange: (view: SidebarView) => void;
  children: ReactNode;
};

const VIEWS = [
  { value: 'servers', label: 'Servers', Icon: Server },
  { value: 'files', label: 'Files', Icon: FolderTree },
] as const satisfies readonly { value: SidebarView; label: string; Icon: typeof Server }[];

/** サイドバーの外枠と、表示内容（サーバ一覧 / リモートファイル）の切り替え */
export function Sidebar({ view, onViewChange, children }: SidebarProps) {
  return (
    <div className="w-64 bg-[#252526] border-r border-[#3c3c3c] flex flex-col shrink-0 min-h-0">
      <div role="tablist" className="flex border-b border-[#3c3c3c] shrink-0">
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
                  ? 'text-white border-[#007acc]'
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
    </div>
  );
}
