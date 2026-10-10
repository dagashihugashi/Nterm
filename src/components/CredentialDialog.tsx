import { useState, type FormEvent } from 'react';
import type { CredentialAnswer } from '../lib/sshSession';
import type { ServerConfig } from '../types/server';
import { Dialog, DialogFooter, Field, inputClass } from './Dialog';

type CredentialDialogProps = {
  /** 接続先（servers.json から削除済みの場合は undefined） */
  server: ServerConfig | undefined;
  /** 直前の認証失敗の理由 */
  error?: string;
  onSubmit: (answer: CredentialAnswer) => void;
  onCancel: () => void;
};

/** 接続時のパスワード／パスフレーズ入力 */
export function CredentialDialog({ server, error, onSubmit, onCancel }: CredentialDialogProps) {
  const [secret, setSecret] = useState('');
  const [remember, setRemember] = useState(false);
  const isPublicKey = server?.auth.type === 'publickey';
  const secretLabel = isPublicKey ? 'passphrase' : 'password';

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    onSubmit({ secret, remember });
  };

  return (
    <Dialog title={`Connect to ${server?.name ?? 'server'}`} onClose={onCancel}>
      <form onSubmit={handleSubmit} className="p-4 flex flex-col gap-3">
        {server && (
          <div className="text-gray-400 select-text">
            {server.user}@{server.host}:{server.port}
            {isPublicKey && <div className="text-xs mt-1 break-all">Key: {server.auth.keyFile}</div>}
          </div>
        )}
        <Field label={isPublicKey ? 'Passphrase' : 'Password'}>
          <input
            type="password"
            autoFocus
            value={secret}
            onChange={(event) => setSecret(event.target.value)}
            className={inputClass}
            placeholder={isPublicKey ? 'Leave blank if the key has no passphrase' : undefined}
          />
        </Field>
        <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
          <input
            type="checkbox"
            checked={remember}
            onChange={(event) => setRemember(event.target.checked)}
          />
          Save {secretLabel} (encrypted for this Windows user)
        </label>
        <DialogFooter
          submitLabel="Connect"
          error={error ? `Authentication failed: ${error}` : null}
          onCancel={onCancel}
        />
      </form>
    </Dialog>
  );
}
