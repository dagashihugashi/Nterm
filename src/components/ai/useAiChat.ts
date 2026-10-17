import { useCallback, useRef, useState } from "react";
import {
  answerApproval,
  cancelQuestion,
  resetChat,
  sendQuestion,
  type ChatEvent,
  type ReadUser,
  type TabInfo,
} from "./aiApi";

/** チャットに表示する項目 */
export type ChatItem =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string }
  | {
      kind: "approval";
      id: string;
      tool: string;
      summary: string;
      tab: TabInfo | null;
      readUser: ReadUser | null;
      status: "pending" | "allowed" | "denied";
    }
  | { kind: "tool"; id: string; name: string; summary: string; output: string | null; isError: boolean }
  | { kind: "notice"; id: string; level: "error" | "warning" | "info"; text: string };

const localId = () => crypto.randomUUID();

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** イベントを表示用の項目に反映する（新しい配列を返す） */
export function applyEvent(items: readonly ChatItem[], event: ChatEvent): ChatItem[] {
  switch (event.event) {
    case "text":
      return [...items, { kind: "assistant", id: localId(), text: event.data.text }];
    case "approvalRequested":
      return [...items, { kind: "approval", ...event.data, status: "pending" }];
    case "toolStarted": {
      // 承認カードを、実行中のツールの表示に置き換える
      const tool: ChatItem = { kind: "tool", ...event.data, output: null, isError: false };
      const index = items.findIndex((item) => item.kind === "approval" && item.id === event.data.id);
      return index < 0 ? [...items, tool] : items.map((item, i) => (i === index ? tool : item));
    }
    case "toolFinished":
      return items.map((item) =>
        item.kind === "tool" && item.id === event.data.id
          ? { ...item, output: event.data.output, isError: event.data.isError }
          : item,
      );
    case "finished": {
      // 中止で回答されなかった承認は、拒否として表示する
      const settled = items.map((item) =>
        item.kind === "approval" && item.status === "pending" ? { ...item, status: "denied" as const } : item,
      );
      if (event.data.reason === "cancelled") {
        return [...settled, { kind: "notice", id: localId(), level: "info", text: "Stopped." }];
      }
      if (event.data.reason === "iterationLimit") {
        const text = "The AI used tools too many times in a row and was stopped. Ask again to continue.";
        return [...settled, { kind: "notice", id: localId(), level: "warning", text }];
      }
      return settled;
    }
    case "error":
      return [...items, { kind: "notice", id: localId(), level: "error", text: event.data.message }];
    case "warning":
      return [...items, { kind: "notice", id: localId(), level: "warning", text: event.data.message }];
  }
}

const setApprovalStatus = (items: readonly ChatItem[], id: string, status: "allowed" | "denied") =>
  items.map((item) => (item.kind === "approval" && item.id === id ? { ...item, status } : item));

/** チャットの表示と操作（会話の履歴そのものは Rust 側が持つ） */
export function useAiChat() {
  const [items, setItems] = useState<ChatItem[]>([]);
  const [isRunning, setIsRunning] = useState(false);
  /** 「このチャットでは許可」を選んだツール（新しいチャットで消える） */
  const allowedToolsRef = useRef(new Set<string>());

  const addNotice = (level: "error" | "info", text: string) =>
    setItems((prev) => [...prev, { kind: "notice", id: localId(), level, text }]);

  const handleEvent = useCallback((event: ChatEvent) => {
    if (event.event === "approvalRequested" && allowedToolsRef.current.has(event.data.tool)) {
      setItems((prev) => setApprovalStatus(applyEvent(prev, event), event.data.id, "allowed"));
      answerApproval(event.data.id, true).catch((error: unknown) => addNotice("error", errorText(error)));
      return;
    }
    if (event.event === "finished" || event.event === "error") setIsRunning(false);
    setItems((prev) => applyEvent(prev, event));
  }, []);

  const send = useCallback(
    (text: string, tabs: TabInfo[]) => {
      setItems((prev) => [...prev, { kind: "user", id: localId(), text }]);
      setIsRunning(true);
      sendQuestion(text, tabs, handleEvent).catch((error: unknown) => {
        setIsRunning(false);
        addNotice("error", errorText(error));
      });
    },
    [handleEvent],
  );

  /** 承認カードへの回答。`rememberTool` はこのチャットの間、同じツールを自動で許可する */
  const approve = useCallback((id: string, tool: string, allow: boolean, rememberTool = false) => {
    if (allow && rememberTool) allowedToolsRef.current.add(tool);
    setItems((prev) => setApprovalStatus(prev, id, allow ? "allowed" : "denied"));
    answerApproval(id, allow).catch((error: unknown) => addNotice("error", errorText(error)));
  }, []);

  const cancel = useCallback(() => {
    cancelQuestion().catch((error: unknown) => addNotice("error", errorText(error)));
  }, []);

  const newChat = useCallback(() => {
    resetChat()
      .then(() => {
        allowedToolsRef.current = new Set();
        setItems([]);
      })
      .catch((error: unknown) => addNotice("error", errorText(error)));
  }, []);

  return { items, isRunning, send, approve, cancel, newChat };
}
