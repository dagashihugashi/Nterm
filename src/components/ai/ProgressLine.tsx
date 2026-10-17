import { useEffect, useState } from "react";
import { LoaderCircle } from "lucide-react";
import type { Progress } from "./useAiChat";

type Props = {
  progress: Progress;
  /** 承認カードの回答待ちがある（他の状態より優先して表示する） */
  isWaitingForApproval: boolean;
};

/** 経過時間の表示を更新する間隔 */
const TICK_MS = 1000;

/** 1.2k のようなトークン数の表示 */
export function formatTokens(tokens: number): string {
  return tokens < 1000 ? `${tokens}` : `${(tokens / 1000).toFixed(1)}k`;
}

function phaseLabel(progress: Progress, isWaitingForApproval: boolean): string {
  if (isWaitingForApproval) return "Waiting for your approval";
  switch (progress.phase) {
    case null:
      return "Starting Claude Code";
    case "thinking":
      return "Thinking";
    case "writing":
      return "Writing";
    case "tool":
      return "Running a tool";
  }
}

/** 回答中の進み具合（状態・経過時間・出力トークン数）。Claude Code とつながっているかの目安にもなる */
export function ProgressLine({ progress, isWaitingForApproval }: Props) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.floor((now - progress.startedAt) / 1000));
  const tokens = progress.outputTokens > 0 ? ` · ↓ ${formatTokens(progress.outputTokens)} tokens` : "";
  return (
    <div className="flex items-center gap-1.5 px-1 text-[11px] text-[var(--nt-fg-muted)]" role="status" aria-live="polite">
      <LoaderCircle size={12} className="animate-spin shrink-0" />
      <span className="truncate">
        {phaseLabel(progress, isWaitingForApproval)}… {seconds}s{tokens}
      </span>
    </div>
  );
}
