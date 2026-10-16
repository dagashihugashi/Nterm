import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { X } from "lucide-react";
import { ResizablePanel, type WidthLimits } from "../ResizablePanel";

const WIDTH_LIMITS: WidthLimits = {
  defaultWidth: 360,
  minWidth: 260,
  maxWidth: 900,
  maxWidthRatio: 0.5,
};

/** Rust の ai::AiStatus と同じ形 */
type AiStatus = {
  ready: boolean;
  message: string;
  build?: string;
};

type Props = {
  onClose: () => void;
};

/** AI チャットの右パネル（AI あり版のみバンドルされる） */
export default function AiPanel({ onClose }: Props) {
  const [status, setStatus] = useState<AiStatus | null>(null);

  useEffect(() => {
    invoke<AiStatus>("ai_status")
      .then(setStatus)
      .catch((error: unknown) =>
        setStatus({ ready: false, message: error instanceof Error ? error.message : String(error) }),
      );
  }, []);

  return (
    <ResizablePanel limits={WIDTH_LIMITS} storageKey="nterm.aiPanelWidth" handleEdge="left">
      <div className="flex-1 min-w-0 flex flex-col rounded-lg border border-white/10 overflow-hidden bg-[var(--nt-sidebar-bg)]">
        <div className="h-9 flex items-center px-3 border-b border-white/10 shrink-0">
          <span className="text-xs font-semibold uppercase tracking-wide">AI Assistant</span>
          <button
            type="button"
            onClick={onClose}
            title="Close"
            className="ml-auto p-1 rounded hover:bg-white/10"
          >
            <X size={14} />
          </button>
        </div>
        <div className="flex-1 min-h-0 overflow-auto px-3 py-3 text-xs text-gray-400">
          {status ? status.message : "Loading..."}
        </div>
      </div>
    </ResizablePanel>
  );
}
