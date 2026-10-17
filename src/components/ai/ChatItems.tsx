import { useState, type ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { ElementContent } from "hast";
import { Check, ChevronRight, Copy, LoaderCircle, SquareTerminal, TriangleAlert, Wrench, X } from "lucide-react";
import { isProductionTab, readUserLabel, readsAsRoot, rememberKey } from "./aiApi";
import { HIDDEN_CHARS_PROPERTY, rehypeRevealHidden } from "./hiddenChars";
import type { ChatItem } from "./useAiChat";

type ChatItemsProps = {
  items: readonly ChatItem[];
  /** 承認カードへの回答（`remember` は rememberKey。渡すとこのチャットの間は同じ組を自動で許可する） */
  onApprove: (id: string, allow: boolean, remember?: string | null) => void;
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
    // コードブロックには「ターミナルに挿入」「コピー」を付ける。
    // 見えない文字を含むもの（rehypeRevealHidden が印を付ける）は、見た目と中身が食い違い得るため挿入させない
    pre: ({ node, children }) => {
      // 見えない文字は rehypeRevealHidden で `<U+202E>` の形になっているため、コピーされるのは見えている形
      const text = (node?.children ?? []).map(nodeText).join("").replace(/\n$/, "");
      const hasHidden = node?.properties[HIDDEN_CHARS_PROPERTY] === "true";
      return (
        <div className="my-2 rounded border border-[var(--nt-border)] bg-[var(--nt-surface)]">
          <div className="flex items-center justify-end gap-1 px-1 pt-1">
            {hasHidden && (
              <span className="mr-auto flex items-center gap-1 px-1 text-[11px] text-[var(--nt-warning)]">
                <TriangleAlert size={12} /> Contains hidden characters (shown as &lt;U+…&gt;). Insert is disabled.
              </span>
            )}
            <button
              type="button"
              onClick={() => onInsert(text)}
              disabled={hasHidden}
              title={
                hasHidden
                  ? "This code contains invisible or text-direction characters, so what you see may differ from what would be inserted"
                  : "Insert into the terminal (does not press Enter)"
              }
              className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] text-[var(--nt-fg-muted)] hover:text-[var(--nt-fg)] hover:bg-[var(--nt-hover)] disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <SquareTerminal size={12} /> Insert
            </button>
            <button
              type="button"
              onClick={() => onCopy(text)}
              title="Copy"
              className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[11px] text-[var(--nt-fg-muted)] hover:text-[var(--nt-fg)] hover:bg-[var(--nt-hover)]"
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
        <code className="px-1 rounded bg-[var(--nt-surface)] font-mono text-[0.9em]">{children}</code>
      ),
    // リンクは開かない（WebView が外部のページへ移動しないように）。URL はツールチップで見せる
    a: ({ href, children }) => (
      <span className="underline decoration-dotted" title={href}>
        {children}
      </span>
    ),
    // 画像は読み込まない
    img: ({ alt }) => <span className="text-[var(--nt-fg-subtle)]">[image: {alt}]</span>,
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
    th: ({ children }) => <th className="border border-[var(--nt-border)] px-2 py-0.5 text-left">{children}</th>,
    td: ({ children }) => <td className="border border-[var(--nt-border)] px-2 py-0.5">{children}</td>,
  };
}

function AssistantMessage({ text, onInsert, onCopy }: { text: string; onInsert: (text: string) => void; onCopy: (text: string) => void }) {
  return (
    <div className="text-[13px] leading-relaxed select-text break-words">
      {/* 生の HTML は描画しない（react-markdown の既定）。見えない文字は見える形にする */}
      <Markdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeRevealHidden]}
        components={markdownComponents(onInsert, onCopy)}
      >
        {text}
      </Markdown>
    </div>
  );
}

function Badge({ children, isDanger }: { children: ReactNode; isDanger?: boolean }) {
  return (
    <span
      className={`px-1.5 rounded text-[11px] font-semibold ${
        isDanger ? "bg-red-700/80 text-white" : "bg-[var(--nt-hover)] text-[var(--nt-fg)]"
      }`}
    >
      {children}
    </span>
  );
}

/** サーバでコマンドを実行しない道具と、その説明（承認の判断に使う） */
const LOCAL_TOOL_NOTES: Record<string, string> = {
  read_terminal: "Reads Orca's copy of the terminal output. No command runs on the server.",
  list_sessions: "Lists Orca's open tabs. No command runs on the server.",
};

/** サーバで実際に実行するコマンド（利用者が承認の判断に使う） */
function CommandLine({ tool, command }: { tool: string; command: string | null }) {
  if (command) {
    return (
      <pre className="mt-1 px-1.5 py-1 rounded bg-[var(--nt-surface)] font-mono text-[11px] whitespace-pre-wrap break-all select-text">
        <span className="text-[var(--nt-fg-subtle)]">$ </span>
        {command}
      </pre>
    );
  }
  const note = LOCAL_TOOL_NOTES[tool];
  return note ? <div className="mt-1 text-[var(--nt-fg-subtle)]">{note}</div> : null;
}

function ApprovalCard({ item, onApprove }: { item: Extract<ChatItem, { kind: "approval" }>; onApprove: ChatItemsProps["onApprove"] }) {
  const isProduction = isProductionTab(item.tab);
  const isRoot = readsAsRoot(item.readUser, item.tab);
  // 本番環境・root・組み込みの道具（作業フォルダの外の読み取り）は、毎回確認する（null）
  const remember = rememberKey(item);
  return (
    <div className={`rounded-md border p-2 text-xs ${isProduction || isRoot ? "border-[var(--nt-danger)]" : "border-[var(--nt-border)]"} bg-[var(--nt-surface)]`}>
      <div className="flex items-center gap-1.5 mb-1">
        <Wrench size={13} className="text-[var(--nt-fg-muted)]" />
        <span className="font-semibold">{item.tool}</span>
        {item.tab?.env && <Badge isDanger={isProduction}>{item.tab.env}</Badge>}
      </div>
      <div className="font-mono break-all select-text">{item.summary}</div>
      <CommandLine tool={item.tool} command={item.command} />
      {item.tab && (
        <div className="mt-1 text-[var(--nt-fg-muted)]">
          {item.tab.serverName} ({item.tab.host}) · reads as{" "}
          <span className={isRoot ? "font-semibold text-[var(--nt-danger)]" : "text-[var(--nt-fg)]"}>
            {readUserLabel(item.readUser, item.tab)}
          </span>
        </div>
      )}
      {item.status === "pending" ? (
        <div className="mt-2 flex flex-wrap gap-1.5">
          <button
            type="button"
            onClick={() => onApprove(item.id, true)}
            className="px-2 py-0.5 rounded bg-[var(--nt-bar-bg)] text-[var(--nt-bar-fg)] hover:brightness-110"
          >
            Allow
          </button>
          {remember !== null && (
            <button
              type="button"
              onClick={() => onApprove(item.id, true, remember)}
              title="Allow this tool on this tab as this user until you start a new chat"
              className="px-2 py-0.5 rounded bg-[var(--nt-hover)] hover:bg-[var(--nt-border)]"
            >
              Allow {item.tool} on this tab in this chat
            </button>
          )}
          <button
            type="button"
            onClick={() => onApprove(item.id, false)}
            className="px-2 py-0.5 rounded bg-[var(--nt-hover)] hover:bg-[var(--nt-border)]"
          >
            Deny
          </button>
        </div>
      ) : (
        <div className="mt-1 text-[var(--nt-fg-subtle)]">{item.status === "allowed" ? "Allowed" : "Denied"}</div>
      )}
    </div>
  );
}

function ToolCard({ item }: { item: Extract<ChatItem, { kind: "tool" }> }) {
  const [isOpen, setIsOpen] = useState(false);
  const icon =
    item.output === null ? (
      <LoaderCircle size={13} className="animate-spin text-[var(--nt-fg-muted)]" />
    ) : item.isError ? (
      <X size={13} className="text-[var(--nt-danger)]" />
    ) : (
      <Check size={13} className="text-[var(--nt-success)]" />
    );
  return (
    <div className="rounded-md border border-[var(--nt-border)] bg-[var(--nt-surface)] text-xs">
      <button
        type="button"
        onClick={() => setIsOpen((open) => !open)}
        aria-expanded={isOpen}
        className="w-full flex items-center gap-1.5 px-2 py-1 text-left hover:bg-[var(--nt-hover)]"
      >
        <ChevronRight size={13} className={`shrink-0 transition-transform ${isOpen ? "rotate-90" : ""}`} />
        {icon}
        <span className="font-semibold shrink-0">{item.name}</span>
        <span className="truncate text-[var(--nt-fg-muted)] font-mono">{item.summary}</span>
      </button>
      {isOpen && (
        <div className="px-2 pb-2">
          <CommandLine tool={item.name} command={item.command} />
          {item.output !== null && (
            <pre className="mt-1 max-h-64 overflow-auto font-mono text-[11px] whitespace-pre-wrap break-all select-text">
              {item.output}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function Notice({ item }: { item: Extract<ChatItem, { kind: "notice" }> }) {
  const color = item.level === "error" ? "text-[var(--nt-danger)]" : item.level === "warning" ? "text-[var(--nt-warning)]" : "text-[var(--nt-fg-subtle)]";
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
              <div key={item.id} className="self-end max-w-[90%] rounded-md bg-[var(--nt-hover)] px-2.5 py-1.5 text-[13px] whitespace-pre-wrap break-words select-text">
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
