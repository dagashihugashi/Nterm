import { useEffect, type ReactNode } from 'react';

export const inputClass =
  'w-full px-2 py-1 bg-[#3c3c3c] border border-[#3c3c3c] rounded text-[#d4d4d4] outline-none focus:border-[#007acc] select-text';

type DialogProps = {
  title: string;
  onClose: () => void;
  children: ReactNode;
};

/** モーダルダイアログの外枠。Escキーまたは背景クリックで閉じる。 */
export function Dialog({ title, onClose, children }: DialogProps) {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onMouseDown={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="w-[420px] max-w-[90vw] bg-[#252526] border border-[#454545] rounded shadow-2xl text-sm"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="px-4 py-2 border-b border-[#3c3c3c] font-semibold text-white">{title}</div>
        {children}
      </div>
    </div>
  );
}

type FieldProps = {
  label: string;
  children: ReactNode;
};

export function Field({ label, children }: FieldProps) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-gray-400">{label}</span>
      {children}
    </label>
  );
}

type DialogFooterProps = {
  submitLabel: string;
  isSubmitting?: boolean;
  error?: string | null;
  onCancel: () => void;
};

export function DialogFooter({ submitLabel, isSubmitting = false, error, onCancel }: DialogFooterProps) {
  return (
    <>
      {error && (
        <div className="p-2 text-xs text-[#f48771] bg-[#5a1d1d] border border-[#be1100] rounded break-all select-text">
          {error}
        </div>
      )}
      <div className="flex justify-end gap-2 pt-1">
        <button
          type="button"
          onClick={onCancel}
          className="px-3 py-1 rounded bg-[#3c3c3c] hover:bg-[#4a4a4a] text-gray-200"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={isSubmitting}
          className="px-3 py-1 rounded bg-[#0e639c] hover:bg-[#1177bb] disabled:opacity-50 text-white"
        >
          {submitLabel}
        </button>
      </div>
    </>
  );
}
