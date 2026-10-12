import { useEffect, useState, type FormEvent } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { fetchLogDefaults, startLog, type LogMode, type LogRequest } from '../lib/sessionLog';
import { Dialog, DialogFooter, Field, inputClass } from './Dialog';

type LogDialogProps = {
  serverId: string;
  sessionId: string;
  title: string;
  /** 開始できたら書き込み先のフルパスを渡す */
  onStarted: (path: string) => void;
  onCancel: () => void;
};

const MODE_OPTIONS: readonly { value: LogMode; label: string }[] = [
  { value: 'append', label: 'Append' },
  { value: 'overwrite', label: 'Overwrite' },
];

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** ログ取得の開始ダイアログ。初期値は servers.json の `log`（未指定ならアプリ内蔵の既定値） */
export function LogDialog({ serverId, sessionId, title, onStarted, onCancel }: LogDialogProps) {
  const [form, setForm] = useState<LogRequest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    let isCancelled = false;
    fetchLogDefaults(serverId)
      .then((defaults) => {
        if (!isCancelled) setForm(defaults);
      })
      .catch((reason: unknown) => {
        if (isCancelled) return;
        setError(errorText(reason));
        setForm({ dir: '', fileName: '', mode: 'append' });
      });
    return () => {
      isCancelled = true;
    };
  }, [serverId]);

  const update = (changes: Partial<LogRequest>) =>
    setForm((prev) => (prev ? { ...prev, ...changes } : prev));

  /** フォルダ選択ダイアログを開く（キャンセル時は変更しない） */
  const browseFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        defaultPath: form?.dir || undefined,
        title: 'Select log folder',
      });
      if (typeof selected === 'string') update({ dir: selected });
    } catch (reason: unknown) {
      setError(errorText(reason));
    }
  };

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!form) return;
    setIsSubmitting(true);
    setError(null);
    try {
      onStarted(await startLog(sessionId, form));
    } catch (reason: unknown) {
      setError(errorText(reason));
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog title={`Start logging: ${title}`} onClose={onCancel}>
      {form ? (
        <form onSubmit={handleSubmit} className="p-4 flex flex-col gap-3">
          <Field label="Folder">
            <div className="flex gap-2">
              <input
                value={form.dir}
                onChange={(event) => update({ dir: event.target.value })}
                className={inputClass}
              />
              <button
                type="button"
                onClick={browseFolder}
                className="px-3 py-1 rounded bg-[#3c3c3c] hover:bg-[#4a4a4a] text-gray-200 shrink-0"
              >
                Browse...
              </button>
            </div>
          </Field>
          <Field label="File name">
            <input
              autoFocus
              value={form.fileName}
              onChange={(event) => update({ fileName: event.target.value })}
              className={inputClass}
            />
          </Field>
          <fieldset className="flex flex-col gap-1">
            <legend className="text-xs text-gray-400 mb-1">If the file already exists</legend>
            <div className="flex gap-4">
              {MODE_OPTIONS.map((option) => (
                <label key={option.value} className="flex items-center gap-2 text-gray-300 cursor-pointer">
                  <input
                    type="radio"
                    name="log-mode"
                    checked={form.mode === option.value}
                    onChange={() => update({ mode: option.value })}
                  />
                  {option.label}
                </label>
              ))}
            </div>
          </fieldset>
          <DialogFooter
            submitLabel="Start"
            isSubmitting={isSubmitting}
            error={error}
            onCancel={onCancel}
          />
        </form>
      ) : (
        <div className="p-4 text-gray-400">Loading...</div>
      )}
    </Dialog>
  );
}
