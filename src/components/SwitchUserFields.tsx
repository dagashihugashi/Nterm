import { X } from 'lucide-react';
import { DEFAULT_SWITCH_COMMAND, MAX_SWITCH_STEPS } from '../types/server';
import { inputClass } from './Dialog';

/** 編集中の切り替え手順（command が空なら既定のコマンドを使う） */
export type SwitchUserRow = {
  user: string;
  command: string;
};

type SwitchUserFieldsProps = {
  rows: readonly SwitchUserRow[];
  onChange: (rows: SwitchUserRow[]) => void;
};

/** ユーザー名はコマンドに埋め込むため、Rust 側と同じく英数字と . _ - のみ許可する */
const USER_NAME_PATTERN = '[A-Za-z0-9._][A-Za-z0-9._\\-]*';

/** サーバ編集ダイアログの「ログイン後のユーザ切り替え」欄 */
export function SwitchUserFields({ rows, onChange }: SwitchUserFieldsProps) {
  const update = (index: number, changes: Partial<SwitchUserRow>) =>
    onChange(rows.map((row, i) => (i === index ? { ...row, ...changes } : row)));
  const remove = (index: number) => onChange(rows.filter((_, i) => i !== index));
  const add = () => onChange([...rows, { user: '', command: '' }]);

  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs text-gray-400">Switch user after login</span>
      {rows.map((row, index) => (
        <div key={index} className="flex gap-2 items-center">
          <span className="w-4 text-xs text-gray-500 shrink-0">{index + 1}.</span>
          <div className="w-28 shrink-0">
            <input
              required
              aria-label={`User ${index + 1}`}
              value={row.user}
              pattern={USER_NAME_PATTERN}
              title="Letters, digits, '.', '_' and '-' only"
              placeholder="oracle"
              onChange={(event) => update(index, { user: event.target.value })}
              className={inputClass}
            />
          </div>
          <div className="flex-1 min-w-0">
            <input
              aria-label={`Command ${index + 1}`}
              value={row.command}
              placeholder={DEFAULT_SWITCH_COMMAND}
              onChange={(event) => update(index, { command: event.target.value })}
              className={inputClass}
            />
          </div>
          <button
            type="button"
            title="Remove"
            onClick={() => remove(index)}
            className="p-1 rounded hover:bg-[#3c3c3c] text-gray-400 shrink-0"
          >
            <X size={14} />
          </button>
        </div>
      ))}
      {rows.length < MAX_SWITCH_STEPS && (
        <button type="button" onClick={add} className="self-start text-xs text-[#3794ff] hover:underline">
          + Add step
        </button>
      )}
      {rows.length > 0 && (
        <span className="text-xs text-gray-500">
          Runs in order after login. {'{user}'} in the command is replaced with the user name.
          Passwords are asked at the first switch and can be saved.
        </span>
      )}
    </div>
  );
}
