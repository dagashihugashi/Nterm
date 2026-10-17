import { useEffect, useState } from "react";
import { listSessions, type SessionSummary } from "./aiApi";

type Props = {
  /** Claude Code の作業フォルダ（このフォルダのセッションを一覧にする） */
  cwd: string;
  /** 表示中のセッション（一覧で強調する） */
  currentSessionId: string | null;
  onSelect: (session: SessionSummary) => void;
  onClose: () => void;
};

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** 「3 分前」「2 日前」のような相対的な日時 */
function relativeTime(seconds: number): string {
  const elapsed = Math.max(0, Date.now() / 1000 - seconds);
  if (elapsed < 60) return "just now";
  if (elapsed < 3600) return `${Math.floor(elapsed / 60)} min ago`;
  if (elapsed < 86400) return `${Math.floor(elapsed / 3600)} h ago`;
  if (elapsed < 86400 * 30) return `${Math.floor(elapsed / 86400)} d ago`;
  return new Date(seconds * 1000).toLocaleDateString();
}

/** 作業フォルダの Claude Code のセッションの一覧（選ぶと続きから再開する） */
export function SessionList({ cwd, currentSessionId, onSelect, onClose }: Props) {
  const [sessions, setSessions] = useState<SessionSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listSessions(cwd)
      .then(setSessions)
      .catch((e: unknown) => setError(errorText(e)));
  }, [cwd]);

  return (
    <div className="absolute right-2 top-9 z-20 w-72 max-h-80 overflow-auto rounded-md border border-[var(--nt-border)] bg-[var(--nt-sidebar-bg)] shadow-lg text-xs">
      <div className="flex items-center px-2 py-1.5 border-b border-[var(--nt-border)]">
        <span className="font-semibold">Sessions in this folder</span>
        <button type="button" onClick={onClose} className="ml-auto px-1.5 rounded hover:bg-[var(--nt-hover)]">
          Close
        </button>
      </div>
      {error && <div className="px-2 py-2 text-[var(--nt-danger)]">{error}</div>}
      {!error && sessions === null && <div className="px-2 py-2 text-[var(--nt-fg-subtle)]">Loading...</div>}
      {sessions?.length === 0 && <div className="px-2 py-2 text-[var(--nt-fg-subtle)]">No saved sessions.</div>}
      {sessions?.map((session) => (
        <button
          key={session.id}
          type="button"
          onClick={() => onSelect(session)}
          title={session.id}
          className={`w-full text-left px-2 py-1.5 hover:bg-[var(--nt-hover)] ${session.id === currentSessionId ? "bg-[var(--nt-hover)]" : ""}`}
        >
          <div className="truncate">{session.title}</div>
          <div className="text-[11px] text-[var(--nt-fg-subtle)]">{relativeTime(session.updated)}</div>
        </button>
      ))}
    </div>
  );
}
