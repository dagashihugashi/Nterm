import { useState } from 'react';
import { ArrowDown, ArrowUp, ChevronDown, ChevronRight, X } from 'lucide-react';
import { formatBytes, formatDuration, isActive, type TransferItem } from '../lib/transfer';

type TransferPanelProps = {
  transfers: readonly TransferItem[];
  onCancel: (id: string) => void;
  onDismiss: (id: string) => void;
  onClearFinished: () => void;
};

/** 実行中の転送の進捗（割合・平均速度・残り時間） */
function describeRunning(item: TransferItem, now: number): string {
  const parts: string[] = [];
  if (item.total) parts.push(`${Math.floor((item.transferred / item.total) * 100)}%`);
  parts.push(formatBytes(item.transferred) + (item.total ? ` / ${formatBytes(item.total)}` : ''));
  const elapsedSeconds = item.startedAt ? (now - item.startedAt) / 1000 : 0;
  if (elapsedSeconds >= 1 && item.transferred > 0) {
    const bytesPerSecond = item.transferred / elapsedSeconds;
    parts.push(`${formatBytes(bytesPerSecond)}/s`);
    if (item.total) parts.push(`${formatDuration((item.total - item.transferred) / bytesPerSecond)} left`);
  }
  return parts.join(' · ');
}

function describeStatus(item: TransferItem, now: number): string {
  switch (item.status) {
    case 'queued':
      return 'Waiting';
    case 'running':
      return describeRunning(item, now);
    case 'completed':
      return `Done · ${formatBytes(item.transferred)}`;
    case 'failed':
      return item.message ?? 'Failed';
    case 'cancelled':
      return 'Cancelled';
  }
}

function TransferRow({ item, onCancel, onDismiss }: {
  item: TransferItem;
  onCancel: (id: string) => void;
  onDismiss: (id: string) => void;
}) {
  const Icon = item.kind === 'upload' ? ArrowUp : ArrowDown;
  const percent = item.total ? Math.min(100, (item.transferred / item.total) * 100) : 0;
  const route = item.kind === 'upload' ? `→ ${item.serverName}:${item.destination}` : `→ ${item.destination}`;
  const active = isActive(item);
  return (
    <div className="px-2 py-1.5 border-t border-[#3c3c3c] text-xs" title={`${item.name}\n${route}`}>
      <div className="flex items-center gap-1.5">
        <Icon size={12} className="shrink-0 text-gray-400" />
        <span className="flex-1 truncate text-gray-200">{item.name}</span>
        <button
          type="button"
          title={active ? 'Cancel' : 'Dismiss'}
          aria-label={active ? 'Cancel transfer' : 'Dismiss'}
          onClick={() => (active ? onCancel(item.id) : onDismiss(item.id))}
          className="p-0.5 rounded text-gray-400 hover:text-white hover:bg-[#37373d] shrink-0"
        >
          <X size={12} />
        </button>
      </div>
      {item.status === 'running' && (
        <div className="mt-1 h-1 bg-[#3c3c3c] rounded overflow-hidden">
          <div className="h-full bg-[#0e639c]" style={{ width: `${percent}%` }} />
        </div>
      )}
      <div
        className={`mt-0.5 truncate select-text ${
          item.status === 'failed' ? 'text-[#f48771]' : 'text-gray-500'
        }`}
        title={item.message ?? undefined}
      >
        {describeStatus(item, Date.now())}
      </div>
    </div>
  );
}

/** サイドバー下部の転送一覧 */
export function TransferPanel({ transfers, onCancel, onDismiss, onClearFinished }: TransferPanelProps) {
  const [isCollapsed, setIsCollapsed] = useState(false);
  if (transfers.length === 0) return null;
  const activeCount = transfers.filter(isActive).length;
  const hasFinished = activeCount < transfers.length;
  const Chevron = isCollapsed ? ChevronRight : ChevronDown;

  return (
    <div className="border-t border-[#3c3c3c] bg-[var(--nt-sidebar-bg)] shrink-0">
      <div className="flex items-center gap-1 px-2 py-1 text-xs text-gray-300">
        <button
          type="button"
          onClick={() => setIsCollapsed((collapsed) => !collapsed)}
          className="flex-1 flex items-center gap-1 text-left"
        >
          <Chevron size={12} />
          Transfers{activeCount > 0 ? ` (${activeCount} active)` : ''}
        </button>
        {hasFinished && (
          <button type="button" onClick={onClearFinished} className="text-[#3794ff] hover:underline">
            Clear finished
          </button>
        )}
      </div>
      {!isCollapsed && (
        <div className="max-h-56 overflow-y-auto">
          {transfers.map((item) => (
            <TransferRow key={item.id} item={item} onCancel={onCancel} onDismiss={onDismiss} />
          ))}
        </div>
      )}
    </div>
  );
}
