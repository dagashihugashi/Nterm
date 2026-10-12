import { useState, type FormEvent } from 'react';
import type { CredentialAnswer } from '../lib/sshSession';
import type { ServerConfig } from '../types/server';
import { Dialog, DialogFooter, Field, inputClass } from './Dialog';

type SwitchUserDialogProps = {
  /** 接続先（servers.json から削除済みの場合は undefined） */
  server: ServerConfig | undefined;
  /** 切り替え先のユーザー */
  user: string;
  /** 直前の認証失敗の理由 */
  error: string | null;
  onSubmit: (answer: CredentialAnswer) => void;
  onCancel: () => void;
};

/** ログイン後のユーザ切り替え（su 等）のパスワード入力 */
export function SwitchUserDialog({ server, user, error, onSubmit, onCancel }: SwitchUserDialogProps) {
  const [secret, setSecret] = useState('');
  const [remember, setRemember] = useState(false);

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    onSubmit({ secret, remember });
  };

  return (
    <Dialog title={`Switch to ${user}`} onClose={onCancel}>
      <form onSubmit={handleSubmit} className="p-4 flex flex-col gap-3">
        {server && (
          <div className="text-gray-400 select-text">
            {server.name} ({server.user}@{server.host})
          </div>
        )}
        <Field label={`Password for ${user}`}>
          <input
            type="password"
            autoFocus
            value={secret}
            onChange={(event) => setSecret(event.target.value)}
            className={inputClass}
          />
        </Field>
        <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
          <input
            type="checkbox"
            checked={remember}
            onChange={(event) => setRemember(event.target.checked)}
          />
          Save password (encrypted for this Windows user)
        </label>
        <DialogFooter
          submitLabel="Switch"
          error={error ? `Switch failed: ${error}` : null}
          onCancel={onCancel}
        />
      </form>
    </Dialog>
  );
}
