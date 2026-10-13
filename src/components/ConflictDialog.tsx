import { useState } from 'react';
import type { ConflictAction, ConflictAnswer, ConflictRequest } from '../hooks/useTransfers';
import { Dialog } from './Dialog';

type ConflictDialogProps = {
  request: ConflictRequest;
  onAnswer: (answer: ConflictAnswer) => void;
};

const buttonClass = 'px-3 py-1 rounded text-gray-200';

/** 転送先に同名のファイルがある場合の確認。誤って上書きしないよう、初期フォーカスは「Skip」 */
export function ConflictDialog({ request, onAnswer }: ConflictDialogProps) {
  const [applyToAll, setApplyToAll] = useState(false);
  const answer = (action: ConflictAction) => onAnswer({ action, applyToAll });

  return (
    <Dialog title="File already exists" onClose={() => answer('cancel')}>
      <div className="p-4 flex flex-col gap-3">
        <div className="text-gray-300">
          <div>The destination already has a file with the same name:</div>
          <div className="mt-1 text-white break-all select-text">{request.path}</div>
        </div>
        {request.remaining > 1 && (
          <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
            <input
              type="checkbox"
              checked={applyToAll}
              onChange={(event) => setApplyToAll(event.target.checked)}
            />
            Apply to all {request.remaining} conflicts
          </label>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <button type="button" onClick={() => answer('cancel')} className={`${buttonClass} bg-[#3c3c3c] hover:bg-[#4a4a4a]`}>
            Cancel all
          </button>
          <button type="button" autoFocus onClick={() => answer('skip')} className={`${buttonClass} bg-[#3c3c3c] hover:bg-[#4a4a4a]`}>
            Skip
          </button>
          <button type="button" onClick={() => answer('overwrite')} className={`${buttonClass} bg-[#a1260d] hover:bg-[#be1100] text-white`}>
            Overwrite
          </button>
        </div>
      </div>
    </Dialog>
  );
}
