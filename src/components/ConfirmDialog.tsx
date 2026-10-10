import { useState, type ReactNode } from 'react';
import { Dialog } from './Dialog';

type ConfirmDialogProps = {
  title: string;
  children: ReactNode;
  confirmLabel: string;
  /** 取り消せない操作は赤いボタンで表示する */
  isDestructive?: boolean;
  /** 失敗時は例外を投げる（ダイアログ内にエラーを表示する） */
  onConfirm: () => Promise<void>;
  onCancel: () => void;
};

/** 確認ダイアログ。誤操作を防ぐため、初期フォーカス・Esc・背景クリックはキャンセル。 */
export function ConfirmDialog({
  title,
  children,
  confirmLabel,
  isDestructive = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const [error, setError] = useState<string | null>(null);
  const [isRunning, setIsRunning] = useState(false);

  const handleConfirm = async () => {
    setIsRunning(true);
    setError(null);
    try {
      await onConfirm();
    } catch (err: unknown) {
      setError(String(err));
      setIsRunning(false);
    }
  };

  return (
    <Dialog title={title} onClose={onCancel}>
      <div className="p-4 flex flex-col gap-3">
        <div className="text-gray-300">{children}</div>
        {error && (
          <div className="p-2 text-xs text-[#f48771] bg-[#5a1d1d] border border-[#be1100] rounded break-all select-text">
            {error}
          </div>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            autoFocus
            onClick={onCancel}
            className="px-3 py-1 rounded bg-[#3c3c3c] hover:bg-[#4a4a4a] text-gray-200"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={isRunning}
            onClick={handleConfirm}
            className={`px-3 py-1 rounded text-white disabled:opacity-50 ${
              isDestructive ? 'bg-[#a1260d] hover:bg-[#be1100]' : 'bg-[#0e639c] hover:bg-[#1177bb]'
            }`}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </Dialog>
  );
}
