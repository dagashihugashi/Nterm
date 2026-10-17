import { useCallback, useRef, useState } from "react";
import {
  answerApproval,
  cancelQuestion,
  loadHistory,
  rememberKey,
  resetChat,
  resumeSession,
  sendQuestion,
  type ChatEvent,
  type HistoryItem,
  type ProgressPhase,
  type ReadUser,
  type SessionSummary,
  type TabInfo,
} from "./aiApi";

/** 回答中の進み具合（入力欄の上に表示する） */
export type Progress = {
  /** 質問を送った時刻（経過時間の表示用。Date.now() の値） */
  startedAt: number;
  /** まだ Claude Code から何も届いていなければ null（起動中） */
  phase: ProgressPhase | null;
  outputTokens: number;
};

/** チャットに表示する項目 */
export type ChatItem =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string }
  | {
      kind: "approval";
      id: string;
      tool: string;
      summary: string;
      /** サーバで実際に実行するコマンド（サーバで実行しない道具は null） */
      command: string | null;
      tab: TabInfo | null;
      readUser: ReadUser | null;
      builtin: boolean;
      status: "pending" | "allowed" | "denied";
    }
  | {
      kind: "tool";
      id: string;
      name: string;
      summary: string;
      command: string | null;
      output: string | null;
      isError: boolean;
    }
  | { kind: "notice"; id: string; level: "error" | "warning" | "info"; text: string };

const localId = () => crypto.randomUUID();

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** イベントを表示用の項目に反映する（新しい配列を返す） */
export function applyEvent(items: readonly ChatItem[], event: ChatEvent): ChatItem[] {
  switch (event.event) {
    // 進み具合とセッションは表示の項目にしない（useAiChat の状態で持つ）
    case "session":
    case "progress":
      return [...items];
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
      return settled;
    }
    case "error":
      return [...items, { kind: "notice", id: localId(), level: "error", text: event.data.message }];
  }
}

/** 再開したセッションの履歴を、表示用の項目にする */
const toChatItem = (item: HistoryItem): ChatItem =>
  item.kind === "tool" ? { ...item } : { ...item, id: localId() };

const setApprovalStatus = (items: readonly ChatItem[], id: string, status: "allowed" | "denied") =>
  items.map((item) => (item.kind === "approval" && item.id === id ? { ...item, status } : item));

/** チャットの表示と操作（会話の履歴そのものは Claude Code が保存する） */
export function useAiChat() {
  const [items, setItems] = useState<ChatItem[]>([]);
  const [isRunning, setIsRunning] = useState(false);
  /** Claude Code のセッション（質問すると決まる。再開した場合はそのセッション） */
  const [sessionId, setSessionId] = useState<string | null>(null);
  /** 回答中の進み具合（回答中でなければ null） */
  const [progress, setProgress] = useState<Progress | null>(null);
  /** Claude Code が使っているモデル（起動したときに届く。まだ届いていなければ null） */
  const [activeModel, setActiveModel] = useState<string | null>(null);
  /** 「このチャットでは許可」を選んだ道具とタブと読み取りユーザの組（rememberKey。新しいチャットで消える） */
  const allowedToolsRef = useRef(new Set<string>());

  const addNotice = (level: "error" | "info", text: string) =>
    setItems((prev) => [...prev, { kind: "notice", id: localId(), level, text }]);

  const handleEvent = useCallback((event: ChatEvent) => {
    // 「このチャットでは許可」した道具（同じタブ・同じ読み取りユーザ）は、承認の要求に自動で回答する。
    // 本番環境・root・組み込みの道具は rememberKey が null で、毎回確認する
    const key = event.event === "approvalRequested" ? rememberKey(event.data) : null;
    if (event.event === "approvalRequested" && key !== null && allowedToolsRef.current.has(key)) {
      setItems((prev) => setApprovalStatus(applyEvent(prev, event), event.data.id, "allowed"));
      answerApproval(event.data.id, true).catch((error: unknown) => addNotice("error", errorText(error)));
      return;
    }
    if (event.event === "session") {
      setSessionId(event.data.sessionId);
      setActiveModel(event.data.model);
      return;
    }
    if (event.event === "progress") {
      const { phase, outputTokens } = event.data;
      setProgress((prev) => (prev ? { ...prev, phase, outputTokens } : prev));
      return;
    }
    if (event.event === "finished" || event.event === "error") {
      setIsRunning(false);
      setProgress(null);
    }
    setItems((prev) => applyEvent(prev, event));
  }, []);

  const send = useCallback(
    (text: string, tabs: TabInfo[], cwd: string, model: string | null) => {
      setItems((prev) => [...prev, { kind: "user", id: localId(), text }]);
      setIsRunning(true);
      setProgress({ startedAt: Date.now(), phase: null, outputTokens: 0 });
      sendQuestion(text, tabs, cwd, model, handleEvent).catch((error: unknown) => {
        setIsRunning(false);
        setProgress(null);
        addNotice("error", errorText(error));
      });
    },
    [handleEvent],
  );

  /** 承認カードへの回答。`remember`（rememberKey）を渡すと、このチャットの間は同じ組を自動で許可する */
  const approve = useCallback((id: string, allow: boolean, remember: string | null = null) => {
    if (allow && remember !== null) allowedToolsRef.current.add(remember);
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
        setSessionId(null);
        setItems([]);
      })
      .catch((error: unknown) => addNotice("error", errorText(error)));
  }, []);

  /** 保存されたセッションの続きから再開し、前の会話を表示する */
  const resume = useCallback(async (session: SessionSummary, cwd: string) => {
    try {
      await resumeSession(session.id, cwd);
    } catch (error: unknown) {
      addNotice("error", errorText(error));
      return;
    }
    allowedToolsRef.current = new Set();
    setSessionId(session.id);
    try {
      const history = await loadHistory(session.id, cwd);
      setItems([
        { kind: "notice", id: localId(), level: "info", text: `Continuing "${session.title}".` },
        ...history.map(toChatItem),
      ]);
    } catch (error: unknown) {
      // 履歴が読めなくても、続きから質問はできる
      setItems([
        {
          kind: "notice",
          id: localId(),
          level: "warning",
          text: `Continuing "${session.title}". Earlier messages could not be shown: ${errorText(error)}`,
        },
      ]);
    }
  }, []);

  return { items, isRunning, progress, sessionId, activeModel, send, approve, cancel, newChat, resume };
}
