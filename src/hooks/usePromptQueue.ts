import { useCallback, useState } from 'react';

interface PendingPrompt<TRequest, TAnswer> {
  request: TRequest;
  resolve: (answer: TAnswer) => void;
}

/**
 * ダイアログでユーザーに問い合わせる要求のキュー。
 * 複数タブから同時に要求が来ても1件ずつ表示し、回答で Promise を解決する。
 * `ask` は参照が変わらないため、effect の依存に入れても再実行されない。
 */
export function usePromptQueue<TRequest, TAnswer>() {
  const [queue, setQueue] = useState<PendingPrompt<TRequest, TAnswer>[]>([]);

  const ask = useCallback(
    (request: TRequest) =>
      new Promise<TAnswer>((resolve) => {
        setQueue((prev) => [...prev, { request, resolve }]);
      }),
    [],
  );

  const current = queue[0];

  const answer = (value: TAnswer) => {
    if (!current) return;
    current.resolve(value);
    setQueue((prev) => prev.slice(1));
  };

  return { current: current?.request, ask, answer };
}
