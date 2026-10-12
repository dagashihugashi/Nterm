import { useState, useEffect, useMemo, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Server, X } from "lucide-react";
import { TerminalComponent } from "./components/Terminal";
import { ServerSidebar } from "./components/ServerSidebar";
import { Sidebar, type SidebarView } from "./components/Sidebar";
import { RemoteFileTree } from "./components/RemoteFileTree";
import { ServerDialog } from "./components/ServerDialog";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { CredentialDialog } from "./components/CredentialDialog";
import { HostKeyDialog } from "./components/HostKeyDialog";
import { LogDialog } from "./components/LogDialog";
import { usePromptQueue } from "./hooks/usePromptQueue";
import { isToggleLogShortcut, stopLog } from "./lib/sessionLog";
import type {
  CredentialAnswer,
  CredentialRequest,
  HostKeyPrompt,
  SessionInfo,
  SessionPrompts,
} from "./lib/sshSession";
import type { ServerConfig } from "./types/server";
import "./App.css";

/** タブIDは接続ごとに一意。同じサーバを複数タブで開ける。 */
interface Tab {
  id: string;
  title: string;
  serverId?: string;
}

type DialogState =
  | { kind: "add-server" }
  | { kind: "edit-server"; server: ServerConfig }
  | { kind: "delete-server"; server: ServerConfig }
  | { kind: "start-log"; tabId: string }
  | null;

const MOCKUP_TAB: Tab = { id: "mockup", title: "Local Mockup" };

/** 指定キーを除いた新しいオブジェクトを返す */
const omitKey = <T,>(record: Record<string, T>, key: string): Record<string, T> =>
  Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** 指定タブのログ開始ダイアログを閉じる（タブを閉じた・切断された場合） */
const withoutLogDialogFor = (tabId: string) => (prev: DialogState): DialogState =>
  prev?.kind === "start-log" && prev.tabId === tabId ? null : prev;

function App() {
  const [tabs, setTabs] = useState<Tab[]>([MOCKUP_TAB]);
  const [activeTab, setActiveTab] = useState(MOCKUP_TAB.id);
  const [servers, setServers] = useState<ServerConfig[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<DialogState>(null);
  const [sidebarView, setSidebarView] = useState<SidebarView>("servers");
  /** タブIDごとの SSH セッション（Files ビューで現在のタブのセッションを使う） */
  const [tabSessions, setTabSessions] = useState<Record<string, SessionInfo>>({});
  /** ログ取得中のタブIDと書き込み先のパス */
  const [tabLogs, setTabLogs] = useState<Record<string, string>>({});
  /** ステータスバーに表示する直近のエラー */
  const [statusError, setStatusError] = useState<string | null>(null);
  const hostKeyQueue = usePromptQueue<HostKeyPrompt, boolean>();
  const credentialQueue = usePromptQueue<CredentialRequest, CredentialAnswer | null>();

  // 参照が変わると全タブが再接続されるため、安定した ask だけに依存させる
  const prompts = useMemo<SessionPrompts>(
    () => ({ confirmHostKey: hostKeyQueue.ask, requestCredential: credentialQueue.ask }),
    [hostKeyQueue.ask, credentialQueue.ask],
  );

  useEffect(() => {
    invoke<ServerConfig[]>("load_servers")
      .then((data) => {
        setServers(data);
        setLoadError(null);
      })
      .catch((error: unknown) => setLoadError(String(error)));
  }, []);

  const closeDialog = () => setDialog(null);

  /** 保存済みパスワードがあればそのまま接続し、なければ接続処理側が入力を求める */
  const openSession = (server: ServerConfig) => {
    const tab: Tab = { id: crypto.randomUUID(), title: server.name, serverId: server.id };
    setTabs((prev) => [...prev, tab]);
    setActiveTab(tab.id);
  };

  const handleServerSaved = (saved: ServerConfig) => {
    setServers((prev) =>
      prev.some((s) => s.id === saved.id)
        ? prev.map((s) => (s.id === saved.id ? saved : s))
        : [...prev, saved],
    );
    closeDialog();
  };

  const deleteServer = async (server: ServerConfig) => {
    await invoke("delete_server", { id: server.id });
    setServers((prev) => prev.filter((s) => s.id !== server.id));
    closeDialog();
  };

  const handleSessionChange = useCallback((tabId: string, info: SessionInfo) => {
    setTabSessions((prev) => ({ ...prev, [tabId]: info }));
    // 切断時はセッション側でログも閉じられる
    if (info.state === "closed") {
      setTabLogs((prev) => omitKey(prev, tabId));
      setDialog(withoutLogDialogFor(tabId));
    }
  }, []);

  const handleLogFailed = useCallback((tabId: string) => {
    setTabLogs((prev) => omitKey(prev, tabId));
  }, []);

  const closeTab = (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    const newTabs = tabs.filter((t) => t.id !== id);
    setTabs(newTabs);
    setTabSessions((prev) => omitKey(prev, id));
    setTabLogs((prev) => omitKey(prev, id));
    setDialog(withoutLogDialogFor(id));
    if (activeTab === id && newTabs.length > 0) {
      setActiveTab(newTabs[newTabs.length - 1].id);
    }
  };

  const credentialRequest = credentialQueue.current;
  const activeSession = tabSessions[activeTab];
  const connectedSessionId =
    activeSession?.state === "connected" ? activeSession.sessionId : null;
  const activeLogPath = tabLogs[activeTab];

  /** 表示中のタブのログ取得を開始（ダイアログを開く）／停止する */
  const toggleLog = () => {
    if (!connectedSessionId) return;
    if (!activeLogPath) {
      setDialog({ kind: "start-log", tabId: activeTab });
      return;
    }
    const tabId = activeTab;
    stopLog(connectedSessionId)
      .then(() => setStatusError(null))
      .catch((error: unknown) => setStatusError(`Failed to stop logging: ${errorText(error)}`))
      .finally(() => setTabLogs((prev) => omitKey(prev, tabId)));
  };

  // Alt+L（ダイアログ表示中は無効）。毎回のレンダーで最新の toggleLog を参照する
  const toggleLogRef = useRef(toggleLog);
  const isDialogOpenRef = useRef(false);
  useEffect(() => {
    toggleLogRef.current = toggleLog;
    isDialogOpenRef.current =
      dialog !== null || credentialQueue.current !== null || hostKeyQueue.current !== null;
  });
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!isToggleLogShortcut(event) || isDialogOpenRef.current) return;
      event.preventDefault();
      toggleLogRef.current();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const logDialogTab = dialog?.kind === "start-log" ? tabs.find((t) => t.id === dialog.tabId) : undefined;
  const logDialogSession = logDialogTab ? tabSessions[logDialogTab.id] : undefined;

  return (
    <div className="flex flex-col h-screen bg-[#1e1e1e] text-[#d4d4d4] select-none">
      {/* Title/Tab Bar */}
      <div className="flex bg-[#2d2d2d] border-b border-[#3c3c3c] items-end overflow-x-auto no-scrollbar">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`px-4 py-2 text-sm cursor-pointer border-r border-[#3c3c3c] flex items-center gap-2 group min-w-[120px] max-w-[200px] ${
              activeTab === tab.id
                ? "bg-[#1e1e1e] border-t-2 border-t-[#007acc] text-white"
                : "bg-[#2d2d2d] hover:bg-[#333333] text-gray-400 border-t-2 border-t-transparent"
            }`}
          >
            <Server size={14} className={activeTab === tab.id ? "text-[#007acc] shrink-0" : "text-gray-500 shrink-0"} />
            <span className="truncate flex-1">{tab.title}</span>
            {tabLogs[tab.id] && (
              <span className="text-[10px] font-bold text-[#f14c4c] shrink-0" title={tabLogs[tab.id]}>
                ● REC
              </span>
            )}
            <X
              size={14}
              className={`hover:bg-[#444] rounded shrink-0 ${activeTab === tab.id ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}
              onClick={(e) => closeTab(e, tab.id)}
            />
          </div>
        ))}
      </div>

      {/* Main Content Area */}
      <div className="flex flex-1 overflow-hidden">
        <Sidebar view={sidebarView} onViewChange={setSidebarView}>
          {sidebarView === "servers" && (
            <ServerSidebar
              servers={servers}
              loadError={loadError}
              onConnect={openSession}
              onAddClick={() => setDialog({ kind: "add-server" })}
              onEdit={(server) => setDialog({ kind: "edit-server", server })}
              onDelete={(server) => setDialog({ kind: "delete-server", server })}
            />
          )}
          {sidebarView === "files" &&
            (connectedSessionId ? (
              <RemoteFileTree key={connectedSessionId} sessionId={connectedSessionId} />
            ) : (
              <div className="px-4 py-3 text-xs text-gray-500">
                Select a connected tab to browse its files.
              </div>
            ))}
        </Sidebar>

        {/* Terminal Area */}
        <div className="flex-1 relative">
          {tabs.map((tab) => (
            <TerminalComponent
              key={tab.id}
              serverId={tab.serverId}
              prompts={prompts}
              onSessionChange={(info) => handleSessionChange(tab.id, info)}
              onLogFailed={() => handleLogFailed(tab.id)}
              isActive={activeTab === tab.id}
            />
          ))}
        </div>
      </div>

      {/* Status Bar */}
      <div className="h-6 bg-[#007acc] flex items-center px-3 text-xs text-white gap-4 shrink-0">
        {activeLogPath ? (
          <span className="truncate select-text" title={activeLogPath}>
            <span className="font-bold text-[#ffd2d2]">● REC</span> {activeLogPath}
          </span>
        ) : (
          <span className="truncate">{statusError ?? "Ready"}</span>
        )}
        <span className="ml-auto flex items-center gap-4 shrink-0">
          {connectedSessionId && (
            <button
              type="button"
              onClick={toggleLog}
              title="Alt+L"
              className="px-2 rounded hover:bg-white/20"
            >
              {activeLogPath ? "Stop logging" : "Start logging..."}
            </button>
          )}
          <span>UTF-8</span>
        </span>
      </div>

      {dialog?.kind === "add-server" && (
        <ServerDialog onSaved={handleServerSaved} onCancel={closeDialog} />
      )}
      {dialog?.kind === "edit-server" && (
        <ServerDialog server={dialog.server} onSaved={handleServerSaved} onCancel={closeDialog} />
      )}
      {dialog?.kind === "delete-server" && (
        <ConfirmDialog
          title="Delete server"
          confirmLabel="Delete"
          isDestructive
          onConfirm={() => deleteServer(dialog.server)}
          onCancel={closeDialog}
        >
          Delete <span className="font-semibold text-white">{dialog.server.name}</span>
          {" "}({dialog.server.user}@{dialog.server.host})? Its saved password will also be removed.
        </ConfirmDialog>
      )}
      {logDialogTab?.serverId && logDialogSession?.state === "connected" && (
        <LogDialog
          serverId={logDialogTab.serverId}
          sessionId={logDialogSession.sessionId}
          title={logDialogTab.title}
          onStarted={(path) => {
            setTabLogs((prev) => ({ ...prev, [logDialogTab.id]: path }));
            setStatusError(null);
            closeDialog();
          }}
          onCancel={closeDialog}
        />
      )}
      {credentialRequest && (
        <CredentialDialog
          key={`${credentialRequest.serverId}:${credentialRequest.error ?? ""}`}
          server={servers.find((s) => s.id === credentialRequest.serverId)}
          error={credentialRequest.error}
          onSubmit={(answer) => credentialQueue.answer(answer)}
          onCancel={() => credentialQueue.answer(null)}
        />
      )}
      {hostKeyQueue.current && (
        <HostKeyDialog
          key={`${hostKeyQueue.current.host}:${hostKeyQueue.current.port}`}
          prompt={hostKeyQueue.current}
          onAnswer={hostKeyQueue.answer}
        />
      )}
    </div>
  );
}

export default App;
