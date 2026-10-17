import { Channel, invoke } from "@tauri-apps/api/core";

/** Rust の ai::AiStatus と同じ形 */
export type AiStatus = {
  ready: boolean;
  /** 接続先の説明（使える場合）または使えない理由 */
  message: string;
  build?: string;
};

/** Rust の ai::session::ReadUser と同じ形（AI がサーバを読むときのユーザ） */
export type ReadUser =
  | { kind: "login" }
  | { kind: "switched"; user: string }
  | { kind: "pending" };

/** Rust の ai::chat::TabInfo と同じ形（開いているタブの情報） */
export type TabInfo = {
  sessionId: string;
  title: string;
  serverName: string;
  host: string;
  env: string | null;
  loginUser: string;
};

/** Rust の ai::chat::ChatEvent と同じ形 */
export type ChatEvent =
  | { event: "text"; data: { text: string } }
  | {
      event: "approvalRequested";
      data: {
        id: string;
        tool: string;
        summary: string;
        tab: TabInfo | null;
        readUser: ReadUser | null;
      };
    }
  | { event: "toolStarted"; data: { id: string; name: string; summary: string; session: string | null } }
  | { event: "toolFinished"; data: { id: string; output: string; isError: boolean } }
  | { event: "finished"; data: { reason: "completed" | "cancelled" | "iterationLimit" } }
  | { event: "error"; data: { message: string } }
  | { event: "warning"; data: { message: string } };

export const getAiStatus = () => invoke<AiStatus>("ai_status");

/** 質問する。進行状況は `onEvent` に届く（"finished" か "error" で終わる） */
export function sendQuestion(text: string, tabs: TabInfo[], onEvent: (event: ChatEvent) => void) {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("ai_send", { text, tabs, onEvent: channel });
}

export const answerApproval = (id: string, allow: boolean) => invoke<void>("ai_approve", { id, allow });
export const cancelQuestion = () => invoke<void>("ai_cancel");
export const resetChat = () => invoke<void>("ai_reset");
export const saveApiKey = (key: string) => invoke<void>("ai_save_api_key", { key });
export const deleteApiKey = () => invoke<void>("ai_delete_api_key");

/** 読み取りユーザの表示（承認カード等） */
export function readUserLabel(readUser: ReadUser | null, tab: TabInfo | null): string {
  if (!readUser) return "unknown user";
  switch (readUser.kind) {
    case "switched":
      return readUser.user;
    case "login":
      return tab?.loginUser ?? "login user";
    case "pending":
      return "waiting for user switch";
  }
}

/** root の権限で読むか（承認カードで強調する） */
export const readsAsRoot = (readUser: ReadUser | null, tab: TabInfo | null) =>
  readUserLabel(readUser, tab) === "root";
