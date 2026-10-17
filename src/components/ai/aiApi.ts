import { Channel, invoke } from "@tauri-apps/api/core";

/** Rust の ai::AiStatus と同じ形 */
export type AiStatus = {
  /** Claude Code を使える状態か */
  ready: boolean;
  /** Claude Code のバージョン（使える場合）または使えない理由 */
  message: string;
  /** 作業フォルダの既定値（exe の隣の空のフォルダ `ai-workspace`） */
  defaultCwd: string | null;
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

/** Rust の ai::sessions::SessionSummary と同じ形（Claude Code のセッション） */
export type SessionSummary = {
  id: string;
  title: string;
  /** 最後に更新した日時（UNIX 時刻の秒） */
  updated: number;
};

/** 回答中の状態（Rust の ai::claude_cli::Phase） */
export type ProgressPhase = "thinking" | "writing" | "tool";

/** Rust の ai::chat::ChatEvent と同じ形 */
export type ChatEvent =
  | { event: "session"; data: { sessionId: string; model: string | null } }
  | { event: "text"; data: { text: string } }
  | {
      event: "approvalRequested";
      data: {
        id: string;
        tool: string;
        summary: string;
        /** サーバで実際に実行するコマンド（サーバで実行しない道具は null） */
        command: string | null;
        tab: TabInfo | null;
        readUser: ReadUser | null;
        /** Claude Code の組み込みの道具（Read 等で、作業フォルダの外を読むとき）。「このチャットでは許可」を出さない */
        builtin: boolean;
      };
    }
  | {
      event: "toolStarted";
      data: { id: string; name: string; summary: string; command: string | null; session: string | null };
    }
  /** 回答中の進み具合（出力トークン数は応答の途中は見積もり） */
  | { event: "progress"; data: { phase: ProgressPhase; outputTokens: number } }
  | { event: "toolFinished"; data: { id: string; output: string; isError: boolean } }
  | { event: "finished"; data: { reason: "completed" | "cancelled" } }
  | { event: "error"; data: { message: string } };

export const getAiStatus = () => invoke<AiStatus>("ai_status");

/**
 * 質問する。進行状況は `onEvent` に届く（"finished" か "error" で終わる）。
 * `cwd` は Claude Code の作業フォルダ、`model` は使うモデル（null なら Claude Code の設定のまま）
 */
export function sendQuestion(
  text: string,
  tabs: TabInfo[],
  cwd: string,
  model: string | null,
  onEvent: (event: ChatEvent) => void,
) {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<void>("ai_send", { text, tabs, cwd, model, onEvent: channel });
}

/** Rust の ai::chat::HistoryItem と同じ形（再開したセッションの履歴） */
export type HistoryItem =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string }
  | {
      kind: "tool";
      id: string;
      name: string;
      summary: string;
      command: string | null;
      output: string | null;
      isError: boolean;
    };

export const loadHistory = (sessionId: string, cwd: string) =>
  invoke<HistoryItem[]>("ai_load_history", { sessionId, cwd });

/** Rust の ai::model_settings::ModelOption と同じ形 */
export type ModelOption = {
  /** `--model` に渡す値 */
  value: string;
  label: string;
  /** 別名の実際のモデル（設定で決まっている場合） */
  detail: string | null;
};

/** Rust の ai::model_settings::ModelOptions と同じ形（Claude Code の settings.json から読んだもの） */
export type ModelOptions = {
  /** 何も選ばないときに Claude Code が使うモデル */
  defaultModel: string | null;
  options: ModelOption[];
  /** 読み込んだ設定ファイル */
  sources: string[];
  /** 読めなかった設定ファイル */
  warnings: string[];
};

export const loadModelOptions = (cwd: string) => invoke<ModelOptions>("ai_model_options", { cwd });

export const answerApproval =(id: string, allow: boolean) => invoke<void>("ai_approve", { id, allow });
export const cancelQuestion = () => invoke<void>("ai_cancel");
export const resetChat = () => invoke<void>("ai_reset");
export const listSessions = (cwd: string) => invoke<SessionSummary[]>("ai_list_sessions", { cwd });
export const resumeSession = (sessionId: string, cwd: string) => invoke<void>("ai_resume", { sessionId, cwd });

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

/** 本番環境のタブか（承認カードで強調する） */
export const isProductionTab = (tab: TabInfo | null) => tab?.env?.trim().toLowerCase() === "production";

/**
 * 「このチャットでは許可」の単位（道具・タブ・読み取りユーザ）。別のタブや、su した後のユーザには広げない。
 * 本番環境・root で読む呼び出しと、組み込みの道具（作業フォルダの外の読み取り）は毎回確認するため null
 */
export function rememberKey(request: {
  tool: string;
  tab: TabInfo | null;
  readUser: ReadUser | null;
  builtin: boolean;
}): string | null {
  if (request.builtin || isProductionTab(request.tab) || readsAsRoot(request.readUser, request.tab)) return null;
  return [request.tool, request.tab?.sessionId ?? "", readUserLabel(request.readUser, request.tab)].join("\u0000");
}
