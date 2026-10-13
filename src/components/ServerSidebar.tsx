import { useState } from 'react';
import { ChevronDown, ChevronRight, Folder, Pencil, Plus, Server, Trash2 } from 'lucide-react';
import { DEFAULT_ENV, envLabel, type ServerConfig } from '../types/server';

type ServerSidebarProps = {
  servers: ServerConfig[];
  loadError: string | null;
  onConnect: (server: ServerConfig) => void;
  onAddClick: () => void;
  onEdit: (server: ServerConfig) => void;
  onDelete: (server: ServerConfig) => void;
};

const iconButtonClass = 'p-0.5 rounded text-gray-400 hover:text-white hover:bg-[#4a4a4a]';

function groupByEnv(servers: ServerConfig[]): [string, ServerConfig[]][] {
  const groups = new Map<string, ServerConfig[]>();
  for (const server of servers) {
    const env = server.env || DEFAULT_ENV;
    groups.set(env, [...(groups.get(env) ?? []), server]);
  }
  return [...groups.entries()];
}

export function ServerSidebar({
  servers,
  loadError,
  onConnect,
  onAddClick,
  onEdit,
  onDelete,
}: ServerSidebarProps) {
  // 折りたたんだ env のみ保持する（新しく追加された env は展開状態で表示される）
  const [collapsedEnvs, setCollapsedEnvs] = useState<Record<string, boolean>>({});

  const toggleEnv = (env: string) =>
    setCollapsedEnvs((prev) => ({ ...prev, [env]: !prev[env] }));

  return (
    <div className="flex flex-col">
      <div className="flex items-center justify-between p-2 pl-4">
        <span className="text-xs font-semibold text-gray-400 uppercase tracking-wider">Servers</span>
        <button
          type="button"
          title="Add server"
          aria-label="Add server"
          onClick={onAddClick}
          className="p-0.5 rounded text-gray-400 hover:text-white hover:bg-[#37373d]"
        >
          <Plus size={16} />
        </button>
      </div>

      {loadError && (
        <div className="mx-2 mb-2 p-2 text-xs text-[#f48771] bg-[#5a1d1d] border border-[#be1100] rounded break-all select-text">
          <div className="font-semibold mb-1">Failed to load servers.json</div>
          {loadError}
        </div>
      )}
      {!loadError && servers.length === 0 && (
        <div className="px-4 py-2 text-xs text-gray-500">No servers registered. Click + to add.</div>
      )}

      {groupByEnv(servers).map(([env, envServers]) => {
        const isExpanded = !collapsedEnvs[env];
        return (
          <div key={env}>
            <div
              className="px-2 py-1.5 text-sm hover:bg-[#37373d] cursor-pointer flex items-center gap-1"
              onClick={() => toggleEnv(env)}
            >
              {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              <Folder size={14} className="text-[#c5c5c5]" />
              <span className="ml-1 font-medium">{envLabel(env)}</span>
            </div>
            {isExpanded && envServers.map((server) => (
              <div
                key={server.id}
                title={`${server.user}@${server.host}:${server.port}`}
                className="group pl-8 pr-2 py-1.5 text-sm hover:bg-[#37373d] cursor-pointer text-gray-300 flex items-center gap-2"
                onClick={() => onConnect(server)}
              >
                <Server size={14} className="text-[#c5c5c5] shrink-0" />
                <span className="truncate flex-1">{server.name}</span>
                <span className="hidden group-hover:flex items-center gap-0.5 shrink-0">
                  <button
                    type="button"
                    title="Edit"
                    aria-label={`Edit ${server.name}`}
                    className={iconButtonClass}
                    onClick={(event) => {
                      event.stopPropagation();
                      onEdit(server);
                    }}
                  >
                    <Pencil size={13} />
                  </button>
                  <button
                    type="button"
                    title="Delete"
                    aria-label={`Delete ${server.name}`}
                    className={iconButtonClass}
                    onClick={(event) => {
                      event.stopPropagation();
                      onDelete(server);
                    }}
                  >
                    <Trash2 size={13} />
                  </button>
                </span>
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}
