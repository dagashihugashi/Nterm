import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { KeyRound, MessageSquarePlus, Quote, Send, Square, X } from "lucide-react";
import { ResizablePanel, type WidthLimits } from "../ResizablePanel";
import type { StatusMessage } from "../../lib/terminalClipboard";
import { deleteApiKey, getAiStatus, saveApiKey, type AiStatus, type TabInfo } from "./aiApi";
import { ChatItems } from "./ChatItems";
import { useAiChat } from "./useAiChat";

const WIDTH_LIMITS: WidthLimits = {
  defaultWidth: 360,
  minWidth: 260,
  maxWidth: 900,
  maxWidthRatio: 0.5,
};

/** 一番下から この距離（px）以内を見ているときだけ、新しい項目で自動スクロールする */
const AUTO_SCROLL_THRESHOLD = 80;

export type QuoteRequest = { text: string; token: number };

type Props = {
  /** 閉じても会話を残すため、非表示にするだけで破棄しない */
  isOpen: boolean;
  onClose: () => void;
  /** 接続中のタブ（質問のたびに AI へ渡す） */
  tabs: TabInfo[];
  /** Ctrl+Shift+A で引用する選択範囲（token が変わるたびに入力欄へ追加する） */
  quote: QuoteRequest | null;
  /** 表示中のターミナルの選択範囲 */
  getSelection: () => string;
  /** コードブロックの内容を表示中のターミナルに挿入する */
  onInsert: (text: string) => void;
  onStatus: (message: StatusMessage) => void;
};

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** 引用する選択範囲（入力欄に追加するテキスト） */
const quoteBlock = (text: string) => `\`\`\`\n${text.replace(/\s+$/, "")}\n\`\`\`\n`;

function ApiKeyForm({ onSaved, onCancel }: { onSaved: () => void; onCancel?: () => void }) {
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const save = () =>
    saveApiKey(key)
      .then(() => {
        setKey("");
        onSaved();
      })
      .catch((e: unknown) => setError(errorText(e)));
  const remove = () =>
    deleteApiKey()
      .then(onSaved)
      .catch((e: unknown) => setError(errorText(e)));
  return (
    <div className="rounded-md border border-white/15 bg-black/20 p-2 text-xs flex flex-col gap-1.5">
      <div className="font-semibold">Anthropic API key</div>
      <div className="text-gray-400">
        Saved encrypted for this Windows user (DPAPI). Not used when Bedrock is enabled in Claude Code&apos;s settings.json.
      </div>
      <input
        type="password"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder="sk-ant-..."
        autoComplete="off"
        className="px-2 py-1 rounded bg-[#1e1e1e] border border-[#3c3c3c] outline-none focus:border-[var(--nt-bar-bg)]"
      />
      {error && <div className="text-red-300">{error}</div>}
      <div className="flex gap-1.5">
        <button
          type="button"
          onClick={save}
          disabled={key.trim() === ""}
          className="px-2 py-0.5 rounded bg-[var(--nt-bar-bg)] text-[var(--nt-bar-fg)] disabled:opacity-40"
        >
          Save
        </button>
        <button type="button" onClick={remove} className="px-2 py-0.5 rounded bg-white/10 hover:bg-white/20">
          Delete saved key
        </button>
        {onCancel && (
          <button type="button" onClick={onCancel} className="ml-auto px-2 py-0.5 rounded hover:bg-white/10">
            Close
          </button>
        )}
      </div>
    </div>
  );
}

/** AI チャットの右パネル（AI あり版のみバンドルされる） */
export default function AiPanel({ isOpen, onClose, tabs, quote, getSelection, onInsert, onStatus }: Props) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [isKeyFormOpen, setIsKeyFormOpen] = useState(false);
  const [input, setInput] = useState("");
  const chat = useAiChat();
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const refreshStatus = useCallback(() => {
    getAiStatus()
      .then(setStatus)
      .catch((error: unknown) => setStatus({ ready: false, message: errorText(error) }));
  }, []);
  useEffect(refreshStatus, [refreshStatus]);

  // 新しい項目が届いたら、一番下付近を見ている場合だけ追従する
  const itemCount = chat.items.length;
  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    if (distance < AUTO_SCROLL_THRESHOLD + element.clientHeight / 2) element.scrollTop = element.scrollHeight;
  }, [itemCount]);

  const appendQuote = useCallback((text: string) => {
    if (text.trim() === "") return;
    setInput((current) => `${current}${current === "" || current.endsWith("\n") ? "" : "\n"}${quoteBlock(text)}`);
    window.requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  // Ctrl+Shift+A で送られた選択範囲を引用する（token が変わったときだけ）
  const quoteToken = quote?.token;
  const quoteRef = useRef(quote);
  useEffect(() => {
    quoteRef.current = quote;
  });
  useEffect(() => {
    if (quoteToken !== undefined && quoteRef.current) appendQuote(quoteRef.current.text);
  }, [quoteToken, appendQuote]);

  const quoteSelection = () => {
    const selection = getSelection();
    if (selection.trim() === "") {
      onStatus({ kind: "info", text: "Select text in the terminal first" });
      return;
    }
    appendQuote(selection);
  };

  const send = () => {
    const text = input.trim();
    if (text === "" || chat.isRunning || !status?.ready) return;
    chat.send(text, tabs);
    setInput("");
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // 日本語入力の変換確定の Enter では送らない
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      send();
    }
  };

  const copy = (text: string) =>
    invoke("clipboard_write", { text })
      .then(() => onStatus({ kind: "info", text: "Copied" }))
      .catch((error: unknown) => onStatus({ kind: "error", text: errorText(error) }));

  const canSend = status?.ready === true && !chat.isRunning && input.trim() !== "";

  return (
    <div className={isOpen ? "contents" : "hidden"}>
      <ResizablePanel limits={WIDTH_LIMITS} storageKey="nterm.aiPanelWidth" handleEdge="left">
        <div className="flex-1 min-w-0 flex flex-col rounded-lg border border-white/10 overflow-hidden bg-[var(--nt-sidebar-bg)]">
          <div className="h-9 flex items-center gap-1 px-3 border-b border-white/10 shrink-0">
            <span className="text-xs font-semibold uppercase tracking-wide shrink-0">AI Assistant</span>
            <span className="text-[11px] text-gray-500 truncate ml-1" title={status?.message}>
              {status?.ready ? status.message : ""}
            </span>
            <span className="ml-auto flex items-center shrink-0">
              <button
                type="button"
                onClick={chat.newChat}
                disabled={chat.isRunning}
                title="New chat"
                className="p-1 rounded hover:bg-white/10 disabled:opacity-40"
              >
                <MessageSquarePlus size={14} />
              </button>
              <button
                type="button"
                onClick={() => setIsKeyFormOpen((open) => !open)}
                title="Anthropic API key"
                className="p-1 rounded hover:bg-white/10"
              >
                <KeyRound size={14} />
              </button>
              <button type="button" onClick={onClose} title="Close" className="p-1 rounded hover:bg-white/10">
                <X size={14} />
              </button>
            </span>
          </div>

          <div ref={scrollRef} className="flex-1 min-h-0 overflow-auto px-3 py-3 flex flex-col gap-3">
            {isKeyFormOpen && (
              <ApiKeyForm
                onSaved={() => {
                  setIsKeyFormOpen(false);
                  refreshStatus();
                }}
                onCancel={() => setIsKeyFormOpen(false)}
              />
            )}
            {status === null && <div className="text-xs text-gray-500">Loading...</div>}
            {status && !status.ready && (
              <div className="flex flex-col gap-2">
                <div className="text-xs text-yellow-300 select-text">{status.message}</div>
                {!isKeyFormOpen && <ApiKeyForm onSaved={refreshStatus} />}
              </div>
            )}
            {status?.ready && chat.items.length === 0 && (
              <div className="text-xs text-gray-500 leading-relaxed">
                Ask about terminal output, logs or configuration files on the connected servers. The AI can only read,
                and asks before each read. Select text in the terminal and press Ctrl+Shift+A to quote it.
              </div>
            )}
            <ChatItems items={chat.items} onApprove={chat.approve} onInsert={onInsert} onCopy={copy} />
          </div>

          <div className="border-t border-white/10 p-2 shrink-0 flex flex-col gap-1.5">
            <textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              rows={3}
              placeholder={status?.ready ? "Ask the AI (Enter to send, Shift+Enter for a new line)" : "AI is not configured"}
              disabled={!status?.ready}
              className="w-full resize-none px-2 py-1.5 rounded bg-[#1e1e1e] border border-[#3c3c3c] text-[13px] outline-none focus:border-[var(--nt-bar-bg)] disabled:opacity-50 select-text"
            />
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={quoteSelection}
                title="Quote the selection in the terminal (Ctrl+Shift+A)"
                className="flex items-center gap-1 px-2 py-0.5 rounded text-xs text-gray-400 hover:text-gray-100 hover:bg-white/10"
              >
                <Quote size={12} /> Quote selection
              </button>
              {chat.isRunning ? (
                <button
                  type="button"
                  onClick={chat.cancel}
                  className="ml-auto flex items-center gap-1 px-2 py-0.5 rounded text-xs bg-white/10 hover:bg-white/20"
                >
                  <Square size={12} /> Stop
                </button>
              ) : (
                <button
                  type="button"
                  onClick={send}
                  disabled={!canSend}
                  className="ml-auto flex items-center gap-1 px-2 py-0.5 rounded text-xs bg-[var(--nt-bar-bg)] text-[var(--nt-bar-fg)] disabled:opacity-40"
                >
                  <Send size={12} /> Send
                </button>
              )}
            </div>
          </div>
        </div>
      </ResizablePanel>
    </div>
  );
}
