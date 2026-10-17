import { useState, type ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { ElementContent } from "hast";
import { Check, ChevronRight, Copy, LoaderCircle, SquareTerminal, TriangleAlert, Wrench, X } from "lucide-react";
import { readUserLabel, readsAsRoot } from "./aiApi";
import type { ChatItem } from "./useAiChat";

type ChatItemsProps = {
  items: readonly ChatItem[];
  onApprove: (id: string, tool: string, allow: boolean, rememberTool?: boolean) => void;
  /** コードブロックの内容をターミナルに挿入する（Enter は送らない） */
  onInsert: (text: string) => void;
  onCopy: (text: string) => void;
};

/** hast のノードからテキストを取り出す（コードブロックの内容） */
function nodeText(node: ElementContent): string {
  if (node.type === "text") return node.value;
  if (node.type === "element") return node.children.map(nodeText).join("");
  return "";
}

function markdownComponents(onInsert: (text: string) => void, onCopy: (text: string) => void): Components {
  return {
    // コードブロックには「ターミナルに挿入」「コピー」を付ける
    pre: ({ node, children }) => {
      const text = (node?.children ?? []).map(nodeText).join("").replace(/\n$/, "");
      return (
        <div className="my-2 rounded border border-white/10 bg-black/30">
          <div className="flex justify-end gap-1 px-1 pt-1">
            <button
              type="button"
              onClick={() => onInsert(text)}
              title="Insert into the terminal (does not press Enter)"
              className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] text-gray-400 hover:text-gray-100 hover:bg-white/10"
            >
              <SquareTerminal size={12} /> Insert
            </button>
            <button
              type="button"
              onClick={() => onCopy(text)}
              title="Copy"
              className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] text-gray-400 hover:text-gray-100 hover:bg-white/10"
            >
              <Copy size={12} /> Copy
            </button>
          </div>
          <pre className="px-2 pb-2 overflow-x-auto text-xs font-mono select-text">{children}</pre>
        </div>
      );
    },
    code: ({ className, children }) =>
      className ? (
        <code className={className}>{children}</code>
      ) : (
        <code className="px-1 rounded bg-black/30 font-mono text-[0.9em]">{children}</code>
      ),
    // リンクは開かない（WebView が外部のページへ移動しないように）。URL はツールチップで見せる
    a: ({ href, children }) => (
      <span className="underline decoration-dotted" title={href}>
        {children}
      </span>
    ),
    // 画像は読み込まない
    img: ({ alt }) => <span className="text-gray-500">[image: {alt}]</span>,
    p: ({ children }) => <p className="my-1.5">{children}</p>,
    ul: ({ children }) => <ul className="my-1.5 pl-5 list-disc">{children}</ul>,
    ol: ({ children }) => <ol className="my-1.5 pl-5 list-decimal">{children}</ol>,
    h1: ({ children }) => <h3 className="mt-3 mb-1 font-semibold">{children}</h3>,
    h2: ({ children }) => <h3 className="mt-3 mb-1 font-semibold">{children}</h3>,
    h3: ({ children }) => <h3 className="mt-3 mb-1 font-semibold">{children}</h3>,
    table: ({ children }) => (
      <div className="my-2 overflow-x-auto">
        <table className="text-xs border-collapse">{children}</table>
      </div>
    ),
    th: ({ children }) => <th className="border border-white/15 px-2 py-0.5 text-left">{children}</th>,
    td: ({ children }) => <td className="border border-white/15 px-2 py-0.5">{children}</td>,
  };
}

function AssistantMessage({ text, onInsert, onCopy }: { text: string; onInsert: (text: string) => void; onCopy: (text: string) => void }) {
  return (
    <div className="text-[13px] leading-relaxed select-text break-words">
      {/* 生の HTML は描画しない（react-markdown の既定） */}
      <Markdown remarkPlugins={[remarkGfm]} components={markdownComponents(onInsert, onCopy)}>
        {text}
      </Markdown>
    </div>
  );
}

function Badge({ children, isDanger }: { children: ReactNode; isDanger?: boolean }) {
  return (
    <span
      className={`px-1.5 rounded text-[11px] font-semibold ${
        isDanger ? "bg-red-700/80 text-white" : "bg-white/10 text-gray-300"
      }`}
    >
      {children}
    </span>
  );
}

function ApprovalCard({ item, onApprove }: { item: Extract<ChatItem, { kind: "approval" }>; onApprove: ChatItemsProps["onApprove"] }) {
  const isProduction = item.tab?.env === "production";
  const isRoot = readsAsRoot(item.readUser, item.tab);
  return (
    <div className={`rounded-md border p-2 text-xs ${isProduction || isRoot ? "border-red-500/60" : "border-white/15"} bg-black/20`}>
      <div className="flex items-center gap-1.5 mb-1">
        <Wrench size={13} className="text-gray-400" />
        <span className="font-semibold">{item.tool}</span>
        {item.tab?.env && <Badge isDanger={isProduction}>{item.tab.env}</Badge>}
      </div>
      <div className="font-mono break-all select-text">{item.summary}</div>
      {item.tab && (
        <div className="mt-1 text-gray-400">
          {item.tab.serverName} ({item.tab.host}) · reads as{" "}
          <span className={isRoot ? "font-semibold text-red-300" : "text-gray-200"}>
            {readUserLabel(item.readUser, item.tab)}
          </span>
        </div>
      )}
      {item.status === "pending" ? (
        <div className="mt-2 flex flex-wrap gap-1.5">
          <button
            type="button"
            onClick={() => onApprove(item.id, item.tool, true)}
            className="px-2 py-0.5 rounded bg-[var(--nt-bar-bg)] text-[var(--nt-bar-fg)] hover:brightness-110"
          >
            Allow
          </button>
          <button
            type="button"
            onClick={() => onApprove(item.id, item.tool, true, true)}
            className="px-2 py-0.5 rounded bg-white/10 hover:bg-white/20"
          >
            Allow {item.tool} in this chat
          </button>
          <button
            type="button"
            onClick={() => onApprove(item.id, item.tool, false)}
            className="px-2 py-0.5 rounded bg-white/10 hover:bg-white/20"
          >
            Deny
          </button>
        </div>
      ) : (
        <div className="mt-1 text-gray-500">{item.status === "allowed" ? "Allowed" : "Denied"}</div>
      )}
    </div>
  );
}

function ToolCard({ item }: { item: Extract<ChatItem, { kind: "tool" }> }) {
  const [isOpen, setIsOpen] = useState(false);
  const icon =
    item.output === null ? (
      <LoaderCircle size={13} className="animate-spin text-gray-400" />
    ) : item.isError ? (
      <X size={13} className="text-red-400" />
    ) : (
      <Check size={13} className="text-green-400" />
    );
  return (
    <div className="rounded-md border border-white/10 bg-black/20 text-xs">
      <button
        type="button"
        onClick={() => setIsOpen((open) => !open)}
        aria-expanded={isOpen}
        className="w-full flex items-center gap-1.5 px-2 py-1 text-left hover:bg-white/5"
      >
        <ChevronRight size={13} className={`shrink-0 transition-transform ${isOpen ? "rotate-90" : ""}`} />
        {icon}
        <span className="font-semibold shrink-0">{item.name}</span>
        <span className="truncate text-gray-400 font-mono">{item.summary}</span>
      </button>
      {isOpen && item.output !== null && (
        <pre className="max-h-64 overflow-auto px-2 pb-2 font-mono text-[11px] whitespace-pre-wrap break-all select-text">
          {item.output}
        </pre>
      )}
    </div>
  );
}

function Notice({ item }: { item: Extract<ChatItem, { kind: "notice" }> }) {
  const color = item.level === "error" ? "text-red-300" : item.level === "warning" ? "text-yellow-300" : "text-gray-500";
  return (
    <div className={`flex gap-1.5 text-xs ${color} select-text`}>
      {item.level !== "info" && <TriangleAlert size={13} className="shrink-0 mt-0.5" />}
      <span className="break-words">{item.text}</span>
    </div>
  );
}

/** チャットの各項目（利用者の質問・AI の回答・承認カード・ツールの実行・お知らせ） */
export function ChatItems({ items, onApprove, onInsert, onCopy }: ChatItemsProps) {
  return (
    <div className="flex flex-col gap-2.5">
      {items.map((item) => {
        switch (item.kind) {
          case "user":
            return (
              <div key={item.id} className="self-end max-w-[90%] rounded-md bg-white/10 px-2.5 py-1.5 text-[13px] whitespace-pre-wrap break-words select-text">
                {item.text}
              </div>
            );
          case "assistant":
            return <AssistantMessage key={item.id} text={item.text} onInsert={onInsert} onCopy={onCopy} />;
          case "approval":
            return <ApprovalCard key={item.id} item={item} onApprove={onApprove} />;
          case "tool":
            return <ToolCard key={item.id} item={item} />;
          case "notice":
            return <Notice key={item.id} item={item} />;
        }
      })}
    </div>
  );
}
