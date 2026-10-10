import type { HostKeyPrompt } from '../lib/sshSession';
import { Dialog } from './Dialog';

type HostKeyDialogProps = {
  prompt: HostKeyPrompt;
  onAnswer: (accepted: boolean) => void;
};

const DEFAULT_SSH_PORT = 22;

/**
 * ホスト鍵の確認ダイアログ（Tera Term のセキュリティ警告相当）。
 * 誤操作で信頼しないよう、Esc・背景クリック・初期フォーカスはすべて「キャンセル」。
 */
export function HostKeyDialog({ prompt, onAnswer }: HostKeyDialogProps) {
  const target = prompt.port === DEFAULT_SSH_PORT ? prompt.host : `${prompt.host}:${prompt.port}`;
  const handleCancel = () => onAnswer(false);

  return (
    <Dialog title={prompt.changed ? 'Host key has changed' : 'Unknown host key'} onClose={handleCancel}>
      <div className="p-4 flex flex-col gap-3">
        {prompt.changed ? (
          <div className="p-2 text-xs text-[#f48771] bg-[#5a1d1d] border border-[#be1100] rounded">
            <div className="font-semibold mb-1">
              WARNING: The host key for {target} does not match the saved key.
            </div>
            Someone may be intercepting the connection (man-in-the-middle attack), or the server
            may have been reinstalled. Check with the server administrator before continuing.
          </div>
        ) : (
          <div className="text-gray-300">
            The authenticity of <span className="font-semibold text-white">{target}</span> can't be
            established. Verify the fingerprint before connecting.
          </div>
        )}

        <dl className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1 text-xs select-text">
          <dt className="text-gray-400">Key type</dt>
          <dd className="font-mono">{prompt.keyType}</dd>
          <dt className="text-gray-400">Fingerprint</dt>
          <dd className="font-mono break-all">{prompt.fingerprint}</dd>
        </dl>

        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            autoFocus
            onClick={handleCancel}
            className="px-3 py-1 rounded bg-[#3c3c3c] hover:bg-[#4a4a4a] text-gray-200"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => onAnswer(true)}
            className={`px-3 py-1 rounded text-white ${
              prompt.changed ? 'bg-[#a1260d] hover:bg-[#be1100]' : 'bg-[#0e639c] hover:bg-[#1177bb]'
            }`}
          >
            {prompt.changed ? 'Replace key and connect' : 'Trust and connect'}
          </button>
        </div>
      </div>
    </Dialog>
  );
}
