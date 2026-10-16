import type { LucideIcon } from 'lucide-react';

export type ActivityItem = {
  id: string;
  label: string;
  Icon: LucideIcon;
  isActive: boolean;
  onClick: () => void;
};

type ActivityBarProps = {
  items: readonly ActivityItem[];
};

/** 左端のアイコンの列（VS Code のアクティビティバーと同様）。サイドバーの表示内容や右パネルを切り替える。 */
export function ActivityBar({ items }: ActivityBarProps) {
  return (
    <nav aria-label="Views" className="w-10 shrink-0 flex flex-col items-center gap-1 py-1">
      {items.map(({ id, label, Icon, isActive, onClick }) => (
        <button
          key={id}
          type="button"
          title={label}
          aria-label={label}
          aria-pressed={isActive}
          onClick={onClick}
          className={`relative w-9 h-9 flex items-center justify-center rounded-md transition-colors ${
            isActive
              ? 'text-[var(--nt-fg)] bg-white/10'
              : 'text-gray-500 hover:text-gray-200 hover:bg-white/5'
          }`}
        >
          {/* 表示中の印（左端の縦線） */}
          {isActive && (
            <span className="absolute -left-1 top-2 bottom-2 w-0.5 rounded-full bg-[var(--nt-bar-bg)]" />
          )}
          <Icon size={20} strokeWidth={1.75} />
        </button>
      ))}
    </nav>
  );
}
