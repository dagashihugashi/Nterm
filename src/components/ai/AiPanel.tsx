import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, History, MessageSquarePlus, Quote, Send, Square, X } from "lucide-react";
import { ResizablePanel, type WidthLimits } from "../ResizablePanel";
import type { StatusMessage } from "../../lib/terminalClipboard";
import {
  getAiStatus,
  loadModelOptions,
  type AiStatus,
  type ModelOptions,
  type SessionSummary,
  type TabInfo,
} from "./aiApi";
import { ChatItems } from "./ChatItems";
import { ModelPicker } from "./ModelPicker";
import { ProgressLine } from "./ProgressLine";
import { SessionList } from "./SessionList";
import { useAiChat } from "./useAiChat";

const WIDTH_LIMITS: WidthLimits = {
  defaultWidth: 360,
  minWidth: 260,
  maxWidth: 900,
  maxWidthRatio: 0.5,
};

/** 一番下から この距離（px）以内を見ているときだけ、新しい項目で自動スクロールする */
const AUTO_SCROLL_THRESHOLD = 80;
/** 作業フォルダの記憶（利用者ごとの利便性のため。消えても既定の ai-workspace に戻るだけ） */
const CWD_STORAGE_KEY = "nterm.aiCwd";
/** 選んだモデルの記憶（消えても Claude Code の設定のモデルに戻るだけ） */
const MODEL_STORAGE_KEY = "nterm.aiModel";

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

function readSaved(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function save(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // 記憶できなくても、次回は既定の値になるだけ
  }
}

/** AI チャット（Claude Code）の右パネル（AI あり版のみバンドルされる） */
export default function AiPanel({ isOpen, onClose, tabs, quote, getSelection, onInsert, onStatus }: Props) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [savedCwd, setSavedCwd] = useState(() => readSaved(CWD_STORAGE_KEY));
  /** 選んだモデル（null なら Claude Code の設定のまま） */
  const [model, setModel] = useState(() => readSaved(MODEL_STORAGE_KEY));
  /** Claude Code の settings.json から読んだモデルの選択肢（作業フォルダの設定も含むため、フォルダごとに読む） */
  const [modelOptions, setModelOptions] = useState<ModelOptions | null>(null);
  const [isSessionListOpen, setIsSessionListOpen] = useState(false);
  const [input, setInput] = useState("");
  const chat = useAiChat();
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  /** Claude Code の作業フォルダ（未選択なら exe の隣の ai-workspace） */
  const cwd = savedCwd ?? status?.defaultCwd ?? null;

  useEffect(() => {
    getAiStatus()
      .then(setStatus)
      .catch((error: unknown) => setStatus({ ready: false, message: errorText(error), defaultCwd: null }));
  }, []);

  useEffect(() => {
    if (!cwd) return;
    let isCurrent = true;
    loadModelOptions(cwd)
      .then((options) => {
        if (isCurrent) setModelOptions(options);
      })
      .catch((error: unknown) => {
        // 読めなくても別名の選択肢で選べる（理由は選択欄のツールチップに出す）
        if (isCurrent) {
          setModelOptions({ defaultModel: null, options: [], sources: [], warnings: [errorText(error)] });
        }
      });
    return () => {
      isCurrent = false;
    };
  }, [cwd]);

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

  /** 作業フォルダを変える（セッションはフォルダごとに分かれるため、新しい会話にする） */
  const changeCwd = async () => {
    const selected = await open({ directory: true, defaultPath: cwd ?? undefined, title: "Select the working folder for Claude Code" });
    if (typeof selected !== "string" || selected === cwd) return;
    save(CWD_STORAGE_KEY, selected);
    setSavedCwd(selected);
    chat.newChat();
  };

  /** モデルを変える（次の質問から。会話は同じセッションのまま続く） */
  const changeModel = (next: string | null) => {
    save(MODEL_STORAGE_KEY, next);
    setModel(next);
  };

  const resume = (session: SessionSummary) => {
    setIsSessionListOpen(false);
    if (cwd) void chat.resume(session, cwd);
  };

  const send = () => {
    const text = input.trim();
    if (text === "" || chat.isRunning || !status?.ready || !cwd) return;
    chat.send(text, tabs, cwd, model);
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

  const isReady = status?.ready === true && cwd !== null;
  /** Claude Code のバージョンと、実際に使っているモデル（最初の質問で届く） */
  const headerText = status?.ready ? [status.message, chat.activeModel].filter(Boolean).join(" · ") : "";
  const isWaitingForApproval = chat.items.some((item) => item.kind === "approval" && item.status === "pending");
  const canSend = isReady && !chat.isRunning && input.trim() !== "";

  return (
    <div className={isOpen ? "contents" : "hidden"}>
      <ResizablePanel limits={WIDTH_LIMITS} storageKey="nterm.aiPanelWidth" handleEdge="left">
        <div className="relative flex-1 min-w-0 flex flex-col rounded-lg border border-[var(--nt-border)] overflow-hidden bg-[var(--nt-sidebar-bg)]">
          <div className="h-9 flex items-center gap-1 px-3 border-b border-[var(--nt-border)] shrink-0">
            <span className="text-xs font-semibold uppercase tracking-wide shrink-0">AI Assistant</span>
            <span className="text-[11px] text-[var(--nt-fg-subtle)] truncate ml-1" title={headerText}>
              {headerText}
            </span>
            <span className="ml-auto flex items-center shrink-0">
              <button
                type="button"
                onClick={() => setIsSessionListOpen((open) => !open)}
                disabled={chat.isRunning || !cwd}
                title="Continue a saved session"
                className="p-1 rounded hover:bg-[var(--nt-hover)] disabled:opacity-40"
              >
                <History size={14} />
              </button>
              <button
                type="button"
                onClick={chat.newChat}
                disabled={chat.isRunning}
                title="New chat"
                className="p-1 rounded hover:bg-[var(--nt-hover)] disabled:opacity-40"
              >
                <MessageSquarePlus size={14} />
              </button>
              <button type="button" onClick={onClose} title="Close" className="p-1 rounded hover:bg-[var(--nt-hover)]">
                <X size={14} />
              </button>
            </span>
          </div>
          {isSessionListOpen && cwd && (
            <SessionList
              cwd={cwd}
              currentSessionId={chat.sessionId}
              onSelect={resume}
              onClose={() => setIsSessionListOpen(false)}
            />
          )}

          <div className="flex items-center gap-1.5 px-3 py-1 border-b border-[var(--nt-border)] shrink-0 text-[11px] text-[var(--nt-fg-muted)]">
            <FolderOpen size={12} className="shrink-0" />
            <span className="truncate" title={cwd ?? undefined}>
              {cwd ?? "No working folder"}
            </span>
            <button
              type="button"
              onClick={() => void changeCwd()}
              disabled={chat.isRunning}
              title="Claude Code reads CLAUDE.md and saves sessions per working folder"
              className="ml-auto shrink-0 px-1.5 rounded hover:bg-[var(--nt-hover)] hover:text-[var(--nt-fg)] disabled:opacity-40"
            >
              Change
            </button>
          </div>
          <div className="flex items-center gap-1.5 px-3 py-1 border-b border-[var(--nt-border)] shrink-0 text-[11px] text-[var(--nt-fg-muted)]">
            <span className="shrink-0">Model</span>
            <ModelPicker value={model} onChange={changeModel} settings={modelOptions} disabled={chat.isRunning} />
          </div>

          <div ref={scrollRef} className="flex-1 min-h-0 overflow-auto px-3 py-3 flex flex-col gap-3">
            {status === null && <div className="text-xs text-[var(--nt-fg-subtle)]">Loading...</div>}
            {status && !status.ready && <div className="text-xs text-[var(--nt-warning)] select-text">{status.message}</div>}
            {isReady && chat.items.length === 0 && (
              <div className="text-xs text-[var(--nt-fg-subtle)] leading-relaxed">
                Ask about terminal output, logs or configuration files on the connected servers. Claude Code reads the
                servers through Orca and asks before each read. Select text in the terminal and press Ctrl+Shift+A to
                quote it.
              </div>
            )}
            <ChatItems items={chat.items} onApprove={chat.approve} onInsert={onInsert} onCopy={copy} />
          </div>

          <div className="border-t border-[var(--nt-border)] p-2 shrink-0 flex flex-col gap-1.5">
            {chat.progress && <ProgressLine progress={chat.progress} isWaitingForApproval={isWaitingForApproval} />}
            <textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              rows={3}
              placeholder={isReady ? "Ask Claude Code (Enter to send, Shift+Enter for a new line)" : "Claude Code is not available"}
              disabled={!isReady}
              className="w-full resize-none px-2 py-1.5 rounded bg-[var(--nt-input-bg)] text-[var(--nt-fg)] border border-[var(--nt-border)] text-[13px] outline-none focus:border-[var(--nt-bar-bg)] disabled:opacity-50 select-text"
            />
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={quoteSelection}
                title="Quote the selection in the terminal (Ctrl+Shift+A)"
                className="flex items-center gap-1 px-2 py-0.5 rounded text-xs text-[var(--nt-fg-muted)] hover:text-[var(--nt-fg)] hover:bg-[var(--nt-hover)]"
              >
                <Quote size={12} /> Quote selection
              </button>
              {chat.isRunning ? (
                <button
                  type="button"
                  onClick={chat.cancel}
                  className="ml-auto flex items-center gap-1 px-2 py-0.5 rounded text-xs bg-[var(--nt-hover)] hover:bg-[var(--nt-border)]"
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
