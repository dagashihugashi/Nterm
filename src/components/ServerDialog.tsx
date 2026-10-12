import { useEffect, useState, type ChangeEvent, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  AUTH_TYPE_OPTIONS,
  ENCODINGS,
  ENV_SUGGESTIONS,
  type AuthType,
  type ServerConfig,
  type ServerInput,
} from '../types/server';
import { Dialog, DialogFooter, Field, inputClass } from './Dialog';
import { SwitchUserFields, type SwitchUserRow } from './SwitchUserFields';

type ServerDialogProps = {
  /** 指定時は編集モード、未指定時は追加モード */
  server?: ServerConfig;
  onSaved: (server: ServerConfig) => void;
  onCancel: () => void;
};

type FormState = {
  name: string;
  host: string;
  port: string;
  user: string;
  authType: AuthType;
  keyFile: string;
  env: string;
  encoding: string;
  switchUsers: SwitchUserRow[];
};

const DEFAULT_KEY_FILE = '~/.ssh/id_ed25519';
const MAX_PORT = 65535;

const EMPTY_FORM: FormState = {
  name: '',
  host: '',
  port: '22',
  user: '',
  authType: 'password',
  keyFile: DEFAULT_KEY_FILE,
  env: '',
  encoding: 'UTF-8',
  switchUsers: [],
};

function toForm(server: ServerConfig | undefined): FormState {
  if (!server) return EMPTY_FORM;
  return {
    name: server.name,
    host: server.host,
    port: String(server.port),
    user: server.user,
    authType: server.auth.type,
    keyFile: server.auth.keyFile ?? DEFAULT_KEY_FILE,
    env: server.env ?? '',
    encoding: server.encoding ?? 'UTF-8',
    switchUsers: (server.switchUsers ?? []).map((step) => ({
      user: step.user,
      command: step.command ?? '',
    })),
  };
}

function toInput(form: FormState): ServerInput | string {
  const port = Number(form.port);
  if (!Number.isInteger(port) || port < 1 || port > MAX_PORT) {
    return `Port must be 1-${MAX_PORT}`;
  }
  return {
    name: form.name,
    host: form.host,
    port,
    user: form.user,
    authType: form.authType,
    keyFile: form.authType === 'publickey' ? form.keyFile.trim() || null : null,
    env: form.env.trim() || null,
    encoding: form.encoding,
    switchUsers: form.switchUsers.map((row) => ({
      user: row.user.trim(),
      command: row.command.trim() || null,
    })),
  };
}

/** 保存済みパスワードの表示と削除（編集モードのみ） */
function SavedCredential({ serverId }: { serverId: string }) {
  const [hasSaved, setHasSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<boolean>('has_saved_credential', { serverId })
      .then(setHasSaved)
      .catch((err: unknown) => setError(String(err)));
  }, [serverId]);

  const handleForget = async () => {
    try {
      await invoke('forget_credential', { serverId });
      setHasSaved(false);
    } catch (err: unknown) {
      setError(String(err));
    }
  };

  if (error) return <div className="text-xs text-[#f48771] break-all">{error}</div>;
  if (!hasSaved) return <div className="text-xs text-gray-500">No saved password.</div>;
  return (
    <div className="flex items-center justify-between text-xs text-gray-300">
      <span>A password is saved for this server.</span>
      <button
        type="button"
        onClick={handleForget}
        className="px-2 py-0.5 rounded bg-[#3c3c3c] hover:bg-[#4a4a4a] text-gray-200"
      >
        Forget
      </button>
    </div>
  );
}

export function ServerDialog({ server, onSaved, onCancel }: ServerDialogProps) {
  const isEdit = server !== undefined;
  const [form, setForm] = useState<FormState>(() => toForm(server));
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  const handleChange =
    (field: keyof FormState) => (event: ChangeEvent<HTMLInputElement | HTMLSelectElement>) =>
      setForm((prev) => ({ ...prev, [field]: event.target.value }));

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    const input = toInput(form);
    if (typeof input === 'string') {
      setError(input);
      return;
    }
    setIsSaving(true);
    setError(null);
    try {
      const saved = isEdit
        ? await invoke<ServerConfig>('update_server', { id: server.id, server: input })
        : await invoke<ServerConfig>('add_server', { server: input });
      onSaved(saved);
    } catch (err: unknown) {
      setError(String(err));
      setIsSaving(false);
    }
  };

  return (
    <Dialog title={isEdit ? 'Edit Server' : 'Add Server'} onClose={onCancel}>
      <form onSubmit={handleSubmit} className="p-4 flex flex-col gap-3">
        <Field label="Name">
          <input required autoFocus value={form.name} onChange={handleChange('name')} className={inputClass} placeholder="Web01 (本番)" />
        </Field>
        <div className="flex gap-2">
          <div className="flex-1">
            <Field label="Host">
              <input required value={form.host} onChange={handleChange('host')} className={inputClass} placeholder="192.168.10.11" />
            </Field>
          </div>
          <div className="w-24">
            <Field label="Port">
              <input required inputMode="numeric" value={form.port} onChange={handleChange('port')} className={inputClass} />
            </Field>
          </div>
        </div>
        <div className="flex gap-2">
          <div className="flex-1">
            <Field label="User">
              <input required value={form.user} onChange={handleChange('user')} className={inputClass} />
            </Field>
          </div>
          <div className="flex-1">
            <Field label="Authentication">
              <select value={form.authType} onChange={handleChange('authType')} className={inputClass}>
                {AUTH_TYPE_OPTIONS.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
              </select>
            </Field>
          </div>
        </div>
        {form.authType === 'publickey' && (
          <Field label="Private key file">
            <input required value={form.keyFile} onChange={handleChange('keyFile')} className={inputClass} placeholder={DEFAULT_KEY_FILE} />
            <span className="text-xs text-gray-500">
              OpenSSH format. "~/" = user folder, relative path = app folder.
            </span>
          </Field>
        )}
        <div className="flex gap-2">
          <div className="flex-1">
            <Field label="Environment">
              <input list="env-suggestions" value={form.env} onChange={handleChange('env')} className={inputClass} placeholder="production" />
              <datalist id="env-suggestions">
                {ENV_SUGGESTIONS.map((env) => <option key={env} value={env} />)}
              </datalist>
            </Field>
          </div>
          <div className="flex-1">
            <Field label="Encoding">
              <select value={form.encoding} onChange={handleChange('encoding')} className={inputClass}>
                {ENCODINGS.map((encoding) => <option key={encoding} value={encoding}>{encoding}</option>)}
              </select>
            </Field>
          </div>
        </div>
        <SwitchUserFields
          rows={form.switchUsers}
          onChange={(switchUsers) => setForm((prev) => ({ ...prev, switchUsers }))}
        />
        {isEdit && <SavedCredential serverId={server.id} />}
        <DialogFooter submitLabel={isEdit ? 'Save' : 'Add'} isSubmitting={isSaving} error={error} onCancel={onCancel} />
      </form>
    </Dialog>
  );
}
