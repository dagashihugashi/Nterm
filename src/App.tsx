import { useState, useEffect, useMemo, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TerminalComponent } from "./components/Terminal";
import { TabBar, moveTab, type Tab } from "./components/TabBar";
import { ServerSidebar } from "./components/ServerSidebar";
import { Sidebar, type SidebarView } from "./components/Sidebar";
import { RemoteFileTree, type TreeRefreshRequest } from "./components/RemoteFileTree";
import { TransferPanel } from "./components/TransferPanel";
import { ConflictDialog } from "./components/ConflictDialog";
import { useTransfers, type ConflictAnswer, type ConflictRequest } from "./hooks/useTransfers";
import { ServerDialog } from "./components/ServerDialog";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { CredentialDialog } from "./components/CredentialDialog";
import { HostKeyDialog } from "./components/HostKeyDialog";
import { LogDialog } from "./components/LogDialog";
import { EncodingMenu } from "./components/EncodingMenu";
import { SwitchUserDialog } from "./components/SwitchUserDialog";
import { usePromptQueue } from "./hooks/usePromptQueue";
import { stopLog } from "./lib/sessionLog";
import {
  loadThemes,
  resolveTheme,
  setTitleBarColor,
  terminalTheme,
  type ThemeMap,
} from "./lib/themes";
import { isEditingText, matchAppShortcut, type AppAction } from "./lib/shortcuts";
import type { PasteRequest, StatusMessage } from "./lib/terminalClipboard";
import type {
  CredentialAnswer,
  CredentialRequest,
  HostKeyPrompt,
  SessionInfo,
  SessionPrompts,
  SwitchSecretRequest,
} from "./lib/sshSession";
import { envLabel, type ServerConfig } from "./types/server";
import "./App.css";

type DialogState =
  | { kind: "add-server" }
  | { kind: "edit-server"; server: ServerConfig }
  | { kind: "delete-server"; server: ServerConfig }
  | { kind: "start-log"; tabId: string }
  | { kind: "close-tab"; tabId: string; activeTransfers: number }
  | null;


/** 指定キーを除いた新しいオブジェクトを返す */
const omitKey = <T,>(record: Record<string, T>, key: string): Record<string, T> =>
  Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** ステータスバーのメッセージを表示しておく時間 */
const STATUS_TIMEOUT_MS = 5000;
/** servers.json で encoding を省略した場合の文字コード（Rust 側の既定値と同じ） */
const DEFAULT_ENCODING = "UTF-8";
/** 貼り付け確認ダイアログで表示する行数 */
const PASTE_PREVIEW_LINES = 10;

/** 指定タブのログ開始ダイアログを閉じる（タブを閉じた・切断された場合） */
const withoutLogDialogFor = (tabId: string) => (prev: DialogState): DialogState =>
  prev?.kind === "start-log" && prev.tabId === tabId ? null : prev;

function App() {
  const [tabs, setTabs] = useState<Tab[]>([]);
  /** 表示中のタブID（タブがなければ空文字） */
  const [activeTab, setActiveTab] = useState("");
  const [servers, setServers] = useState<ServerConfig[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<DialogState>(null);
  const [sidebarView, setSidebarView] = useState<SidebarView>("servers");
  /** タブIDごとの SSH セッション（Files ビューで現在のタブのセッションを使う） */
  const [tabSessions, setTabSessions] = useState<Record<string, SessionInfo>>({});
  /** ログ取得中のタブIDと書き込み先のパス */
  const [tabLogs, setTabLogs] = useState<Record<string, string>>({});
  /** ステータスバーに一時的に表示するメッセージ（コピー結果・エラー等） */
  const [status, setStatus] = useState<StatusMessage | null>(null);
  /** タブごとの現在の文字コード（接続時はサーバ設定の値、接続中にメニューで切り替え可能） */
  const [tabEncodings, setTabEncodings] = useState<Record<string, string>>({});
  /** 名前を編集中のタブ */
  const [renamingTabId, setRenamingTabId] = useState<string | null>(null);
  /** 増やすと表示中のターミナルにフォーカスを戻す */
  const [focusToken, setFocusToken] = useState(0);
  const hostKeyQueue = usePromptQueue<HostKeyPrompt, boolean>();
  const credentialQueue = usePromptQueue<CredentialRequest, CredentialAnswer | null>();
  const pasteQueue = usePromptQueue<PasteRequest, boolean>();
  const switchSecretQueue = usePromptQueue<SwitchSecretRequest, CredentialAnswer | null>();
  const conflictQueue = usePromptQueue<ConflictRequest, ConflictAnswer>();
  /** アップロード完了後のファイルツリーの再読み込み要求 */
  const [treeRefresh, setTreeRefresh] = useState<(TreeRefreshRequest & { sessionId: string }) | null>(null);
  const transfers = useTransfers({
    askConflict: conflictQueue.ask,
    onError: (text) => setStatus({ kind: "error", text }),
    onUploaded: (sessionId, dir) =>
      setTreeRefresh((prev) => ({ sessionId, dir, seq: (prev?.seq ?? 0) + 1 })),
  });

  useEffect(() => {
    if (!status) return;
    const timer = window.setTimeout(() => setStatus(null), STATUS_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [status]);

  // 参照が変わると全タブが再接続されるため、安定した ask だけに依存させる
  const prompts = useMemo<SessionPrompts>(
    () => ({
      confirmHostKey: hostKeyQueue.ask,
      requestCredential: credentialQueue.ask,
      requestSwitchSecret: switchSecretQueue.ask,
    }),
    [hostKeyQueue.ask, credentialQueue.ask, switchSecretQueue.ask],
  );

  /** themes.json のテーマ（読み込むまでは組み込みの既定のテーマで表示する） */
  const [themes, setThemes] = useState<ThemeMap>({});

  // themes.json を読み込む（なければ既定の内容で作成される）
  useEffect(() => {
    loadThemes()
      .then((loaded) => {
        setThemes(loaded.themes);
        if (loaded.warning) setStatus({ kind: "error", text: loaded.warning });
      })
      .catch((error: unknown) =>
        setStatus({ kind: "error", text: `Failed to load themes: ${errorText(error)}` }),
      );
  }, []);

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
    rememberEncoding(tab.id, server);
    setActiveTab(tab.id);
  };

  /**
   * 接続時の文字コードを記録する（接続後にサーバ設定を編集しても、表示は実際のセッションに合わせる）。
   * 接続処理（Rust 側）も同じく servers.json の encoding を使う。
   */
  const rememberEncoding = (tabId: string, server: ServerConfig | undefined) =>
    setTabEncodings((prev) => ({ ...prev, [tabId]: server?.encoding ?? DEFAULT_ENCODING }));

  /** 同じサーバへの新しい接続を、元のタブの右隣に開く */
  const duplicateTab = (source: Tab) => {
    const tab: Tab = { id: crypto.randomUUID(), title: source.title, serverId: source.serverId };
    setTabs((prev) => {
      const index = prev.findIndex((t) => t.id === source.id);
      return [...prev.slice(0, index + 1), tab, ...prev.slice(index + 1)];
    });
    rememberEncoding(tab.id, servers.find((s) => s.id === source.serverId));
    setActiveTab(tab.id);
  };

  const renameTab = (id: string, title: string) =>
    setTabs((prev) => prev.map((t) => (t.id === id ? { ...t, title } : t)));

  const handleTabMove = (id: string, toIndex: number) =>
    setTabs((prev) => {
      const moved = moveTab(prev, id, toIndex);
      // 位置が変わらない場合は再描画しない
      return moved.every((t, i) => t === prev[i]) ? prev : moved;
    });

  const endRename = () => {
    setRenamingTabId(null);
    setFocusToken((n) => n + 1);
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

  const closeTab = (id: string) => {
    const newTabs = tabs.filter((t) => t.id !== id);
    setTabs(newTabs);
    setTabSessions((prev) => omitKey(prev, id));
    setTabLogs((prev) => omitKey(prev, id));
    setTabEncodings((prev) => omitKey(prev, id));
    setDialog(withoutLogDialogFor(id));
    if (renamingTabId === id) setRenamingTabId(null);
    if (activeTab === id) {
      setActiveTab(newTabs.length > 0 ? newTabs[newTabs.length - 1].id : "");
    }
  };

  /** タブを閉じる。転送中の場合は（閉じると転送がキャンセルされるため）確認する */
  const requestCloseTab = (id: string) => {
    const sessionId = tabSessions[id]?.sessionId;
    const activeTransfers = sessionId ? transfers.activeCountFor(sessionId) : 0;
    if (activeTransfers > 0) {
      setDialog({ kind: "close-tab", tabId: id, activeTransfers });
      return;
    }
    closeTab(id);
  };

  /** 表示中のタブから offset 個先のタブへ切り替える（端では反対側へ回る） */
  const switchTab = (offset: number) => {
    const index = tabs.findIndex((t) => t.id === activeTab);
    if (index < 0 || tabs.length === 0) return;
    setActiveTab(tabs[(index + offset + tabs.length) % tabs.length].id);
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
      .catch((error: unknown) =>
        setStatus({ kind: "error", text: `Failed to stop logging: ${errorText(error)}` }),
      )
      .finally(() => setTabLogs((prev) => omitKey(prev, tabId)));
  };

  const activeTabObject = tabs.find((t) => t.id === activeTab);
  const activeServer = servers.find((s) => s.id === activeTabObject?.serverId);
  /** 画面全体（サイドバー・バー・文字色）は表示中のタブのサーバのテーマに合わせる */
  const appTheme = resolveTheme(themes, activeServer);
  const themeNames = Object.keys(themes);

  // ターミナルの色はタブごと（タブのサーバのテーマ）。参照を保つため、関係する値が変わったときだけ作り直す
  const tabTerminalThemes = useMemo(
    () =>
      Object.fromEntries(
        tabs.map((tab) => [
          tab.id,
          terminalTheme(resolveTheme(themes, servers.find((s) => s.id === tab.serverId))),
        ]),
      ),
    [tabs, servers, themes],
  );

  // タイトルバー（最小化・最大化・閉じるボタンのあるバー）をステータスバーと同じ色にする（Windows 11 のみ）
  useEffect(() => {
    setTitleBarColor(appTheme.barBackground, appTheme.barForeground).catch((error: unknown) =>
      setStatus({ kind: "error", text: `Failed to change the title bar color: ${errorText(error)}` }),
    );
  }, [appTheme.barBackground, appTheme.barForeground]);
  /** 表示中のタブ（接続中）のファイル転送先 */
  const transferTarget =
    connectedSessionId && activeTabObject
      ? { sessionId: connectedSessionId, serverName: activeTabObject.title }
      : null;
  const activeEncoding = tabEncodings[activeTab] ?? DEFAULT_ENCODING;

  /** 表示中のセッションの文字コードを切り替える（servers.json は変更しない） */
  const changeEncoding = (encoding: string) => {
    if (!connectedSessionId) return;
    const tabId = activeTab;
    invoke("set_encoding", { sessionId: connectedSessionId, encoding })
      .then(() => {
        setTabEncodings((prev) => ({ ...prev, [tabId]: encoding }));
        setStatus({ kind: "info", text: `Encoding: ${encoding}` });
      })
      .catch((error: unknown) =>
        setStatus({ kind: "error", text: `Failed to change encoding: ${errorText(error)}` }),
      )
      .finally(() => setFocusToken((n) => n + 1));
  };
  const shortcutActions: Record<AppAction, () => void> = {
    toggleLog,
    nextTab: () => switchTab(1),
    prevTab: () => switchTab(-1),
    closeTab: () => activeTabObject && requestCloseTab(activeTabObject.id),
    duplicateTab: () => activeTabObject && duplicateTab(activeTabObject),
    renameTab: () => activeTabObject && setRenamingTabId(activeTabObject.id),
  };

  // アプリのショートカット（ダイアログ表示中・入力欄の編集中は無効）。毎回のレンダーで最新の処理を参照する
  const shortcutActionsRef = useRef(shortcutActions);
  const isDialogOpenRef = useRef(false);
  useEffect(() => {
    shortcutActionsRef.current = shortcutActions;
    isDialogOpenRef.current =
      dialog !== null ||
      credentialQueue.current !== undefined ||
      hostKeyQueue.current !== undefined ||
      pasteQueue.current !== undefined ||
      switchSecretQueue.current !== undefined ||
      conflictQueue.current !== undefined;
  });
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const action = matchAppShortcut(event);
      if (!action || isDialogOpenRef.current || isEditingText(event.target)) return;
      event.preventDefault();
      shortcutActionsRef.current[action]();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const logDialogTab = dialog?.kind === "start-log" ? tabs.find((t) => t.id === dialog.tabId) : undefined;
  const logDialogSession = logDialogTab ? tabSessions[logDialogTab.id] : undefined;

  return (
    <div
      className="flex flex-col h-screen bg-[var(--nt-gutter)] text-[var(--nt-fg)] select-none"
      style={
        {
          "--nt-bg": appTheme.background,
          "--nt-fg": appTheme.foreground,
          "--nt-sidebar-bg": appTheme.sidebarBackground,
          "--nt-bar-bg": appTheme.barBackground,
          "--nt-bar-fg": appTheme.barForeground,
          // カードの間のすき間の色（サイドバーの色を暗くしたもの）
          "--nt-gutter": `color-mix(in srgb, ${appTheme.sidebarBackground} 65%, #000)`,
        } as React.CSSProperties
      }
    >
      {/* Main Content Area（サイドバーとターミナルを角丸のカードとして並べる） */}
      <div className="flex flex-1 overflow-hidden gap-1.5 p-1.5">
        <Sidebar
          view={sidebarView}
          onViewChange={setSidebarView}
          footer={
            <TransferPanel
              transfers={transfers.transfers}
              onCancel={transfers.cancel}
              onDismiss={transfers.dismiss}
              onClearFinished={transfers.clearFinished}
            />
          }
        >
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
            (connectedSessionId && transferTarget ? (
              <RemoteFileTree
                key={connectedSessionId}
                sessionId={connectedSessionId}
                onUpload={(paths, dir) => void transfers.upload(transferTarget, paths, dir)}
                onDownload={(path, dir) => void transfers.download(transferTarget, path, dir)}
                refreshRequest={treeRefresh?.sessionId === connectedSessionId ? treeRefresh : null}
              />
            ) : (
              <div className="px-4 py-3 text-xs text-gray-500">
                Select a connected tab to browse its files.
              </div>
            ))}
        </Sidebar>

        {/* Terminal Area（タブバーとターミナルで1枚のカード） */}
        <div className="flex-1 min-w-0 flex flex-col rounded-lg border border-white/10 overflow-hidden bg-[var(--nt-bg)]">
          <TabBar
            tabs={tabs}
            activeTabId={activeTab}
            tabLogs={tabLogs}
            renamingTabId={renamingTabId}
            onSelect={setActiveTab}
            onClose={requestCloseTab}
            onMove={handleTabMove}
            onRenameStart={setRenamingTabId}
            onRename={renameTab}
            onRenameEnd={endRename}
          />
          <div className="flex-1 min-h-0 relative">
            {tabs.length === 0 && (
              <div className="absolute inset-0 flex items-center justify-center text-sm text-gray-600">
                Select a server from the sidebar to connect.
              </div>
            )}
            {tabs.map((tab) => (
              <TerminalComponent
                key={tab.id}
                serverId={tab.serverId}
                prompts={prompts}
                onSessionChange={(info) => handleSessionChange(tab.id, info)}
                onLogFailed={() => handleLogFailed(tab.id)}
                onStatus={setStatus}
                confirmPaste={pasteQueue.ask}
                isActive={activeTab === tab.id}
                focusToken={focusToken}
                theme={tabTerminalThemes[tab.id]}
              />
            ))}
          </div>
        </div>
      </div>

      {/* Status Bar */}
      <div className="h-6 bg-[var(--nt-bar-bg)] flex items-center px-3 text-xs text-[var(--nt-bar-fg)] gap-4 shrink-0">
        {activeServer?.env && (
          <span className="shrink-0 px-1.5 rounded font-semibold bg-black/20" title="Environment">
            {envLabel(activeServer.env)}
          </span>
        )}
        {status ? (
          <span className={`truncate ${status.kind === "error" ? "font-semibold text-[#ffd2d2]" : ""}`}>
            {status.kind === "error" ? "⚠ " : ""}
            {status.text}
          </span>
        ) : activeLogPath ? (
          <span className="truncate select-text" title={activeLogPath}>
            <span className="font-bold text-[#ffd2d2]">● REC</span> {activeLogPath}
          </span>
        ) : (
          <span className="truncate">Ready</span>
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
          {connectedSessionId && (
            <EncodingMenu current={activeEncoding} onSelect={changeEncoding} />
          )}
        </span>
      </div>

      {dialog?.kind === "add-server" && (
        <ServerDialog themeNames={themeNames} onSaved={handleServerSaved} onCancel={closeDialog} />
      )}
      {dialog?.kind === "edit-server" && (
        <ServerDialog
          server={dialog.server}
          themeNames={themeNames}
          onSaved={handleServerSaved}
          onCancel={closeDialog}
        />
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
            closeDialog();
          }}
          onCancel={closeDialog}
        />
      )}
      {pasteQueue.current && (
        <ConfirmDialog
          title="Paste multiple lines"
          confirmLabel="Paste"
          onConfirm={async () => pasteQueue.answer(true)}
          onCancel={() => pasteQueue.answer(false)}
        >
          <div className="mb-2">
            Paste {pasteQueue.current.lineCount} lines into the terminal? Each line may run as a
            command.
          </div>
          <pre className="max-h-48 overflow-auto p-2 bg-[#1e1e1e] border border-[#3c3c3c] rounded text-xs whitespace-pre select-text">
            {pasteQueue.current.text.split(/\r\n|\r|\n/).slice(0, PASTE_PREVIEW_LINES).join("\n")}
            {pasteQueue.current.lineCount > PASTE_PREVIEW_LINES &&
              `\n... (${pasteQueue.current.lineCount - PASTE_PREVIEW_LINES} more lines)`}
          </pre>
        </ConfirmDialog>
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
      {dialog?.kind === "close-tab" && (
        <ConfirmDialog
          title="Close tab"
          confirmLabel="Close and cancel"
          isDestructive
          onConfirm={async () => {
            closeTab(dialog.tabId);
            closeDialog();
          }}
          onCancel={closeDialog}
        >
          {dialog.activeTransfers} file transfer{dialog.activeTransfers === 1 ? " is" : "s are"} in
          progress for this tab. Closing the tab cancels {dialog.activeTransfers === 1 ? "it" : "them"}.
        </ConfirmDialog>
      )}
      {conflictQueue.current && (
        <ConflictDialog
          key={conflictQueue.current.path}
          request={conflictQueue.current}
          onAnswer={conflictQueue.answer}
        />
      )}
      {switchSecretQueue.current && (
        <SwitchUserDialog
          key={`${switchSecretQueue.current.serverId}:${switchSecretQueue.current.user}:${switchSecretQueue.current.error ?? ""}`}
          server={servers.find((s) => s.id === switchSecretQueue.current?.serverId)}
          user={switchSecretQueue.current.user}
          error={switchSecretQueue.current.error}
          onSubmit={(answer) => switchSecretQueue.answer(answer)}
          onCancel={() => switchSecretQueue.answer(null)}
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
