import { useEffect, useRef, useState } from 'react';
import { Check } from 'lucide-react';
import { ENCODINGS } from '../types/server';

type EncodingMenuProps = {
  /** 表示中のセッションの文字コード */
  current: string;
  onSelect: (encoding: string) => void;
};

const isSameEncoding = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** ステータスバーの文字コード表示。クリックで切り替えメニューを開く */
export function EncodingMenu({ current, onSelect }: EncodingMenuProps) {
  const [isOpen, setIsOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  // メニューの外側をクリック・Esc で閉じる
  useEffect(() => {
    if (!isOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setIsOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setIsOpen(false);
    };
    window.addEventListener('pointerdown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('pointerdown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [isOpen]);

  const select = (encoding: string) => {
    setIsOpen(false);
    if (!isSameEncoding(encoding, current)) onSelect(encoding);
  };

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={() => setIsOpen((open) => !open)}
        title="Change encoding"
        className="px-2 rounded hover:bg-white/20"
      >
        {current}
      </button>
      {isOpen && (
        <div
          role="menu"
          className="absolute bottom-full right-0 mb-1 min-w-[140px] py-1 bg-[#252526] border border-[#454545] rounded shadow-2xl text-[#d4d4d4]"
        >
          {ENCODINGS.map((encoding) => (
            <button
              key={encoding}
              type="button"
              role="menuitem"
              onClick={() => select(encoding)}
              className="w-full flex items-center gap-2 px-3 py-1 text-left hover:bg-[#094771]"
            >
              <span className="w-3">
                {isSameEncoding(encoding, current) && <Check size={12} />}
              </span>
              {encoding}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
