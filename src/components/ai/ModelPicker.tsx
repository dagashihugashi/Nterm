import { useState } from "react";
import type { ModelOption, ModelOptions } from "./aiApi";

type Props = {
  /** 選んだモデル（null なら Claude Code の設定のまま） */
  value: string | null;
  onChange: (model: string | null) => void;
  /** Claude Code の settings.json から読んだ選択肢（読み込み中・読めなかった場合は null） */
  settings: ModelOptions | null;
  disabled: boolean;
};

/** 設定が読めない・選択肢がないときの選択肢（Claude Code の `--model` に渡せる別名） */
const FALLBACK_OPTIONS: ModelOption[] = [
  { value: "sonnet", label: "Sonnet", detail: null },
  { value: "opus", label: "Opus", detail: null },
  { value: "haiku", label: "Haiku", detail: null },
];

const DEFAULT_OPTION = "";
const CUSTOM_OPTION = "\u0000custom";

const optionLabel = (option: ModelOption) => (option.detail ? `${option.label} (${option.detail})` : option.label);

/** 選択肢の一覧のツールチップ（どの設定ファイルから読んだか） */
function settingsTitle(settings: ModelOptions | null): string {
  if (!settings) return "Model used by Claude Code";
  const lines = [
    "Model used by Claude Code",
    settings.sources.length > 0 ? `Read from:\n${settings.sources.join("\n")}` : "No Claude Code settings file found",
    ...settings.warnings.map((warning) => `Could not read ${warning}`),
  ];
  return lines.join("\n");
}

/** 使うモデルの選択（Claude Code の設定の選択肢、または任意のモデル ID・推論プロファイルの ARN） */
export function ModelPicker({ value, onChange, settings, disabled }: Props) {
  const options = settings && settings.options.length > 0 ? settings.options : FALLBACK_OPTIONS;
  const isListed = (model: string | null) => model === null || options.some((option) => option.value === model);
  const [isCustomChosen, setIsCustomChosen] = useState(false);
  const [customText, setCustomText] = useState(value ?? "");
  // 選択肢にないモデル（以前に入力したもの）は、カスタムとして表示する
  const isCustomShown = isCustomChosen || !isListed(value);
  const selected = isCustomShown ? CUSTOM_OPTION : (value ?? DEFAULT_OPTION);
  const defaultLabel = settings?.defaultModel ? `Default (${settings.defaultModel})` : "Default model";

  const select = (option: string) => {
    if (option === CUSTOM_OPTION) {
      setIsCustomChosen(true);
      return;
    }
    setIsCustomChosen(false);
    onChange(option === DEFAULT_OPTION ? null : option);
  };

  const commitCustom = () => {
    const model = customText.trim();
    onChange(model === "" ? null : model);
  };

  return (
    <div className="flex items-center gap-1 min-w-0 flex-1">
      <select
        value={selected}
        onChange={(e) => select(e.target.value)}
        disabled={disabled}
        title={settingsTitle(settings)}
        className="min-w-0 max-w-full truncate bg-[var(--nt-input-bg)] text-[var(--nt-fg)] border border-[var(--nt-border)] rounded px-1 py-0.5 text-[11px] outline-none focus:border-[var(--nt-bar-bg)] disabled:opacity-40"
      >
        <option value={DEFAULT_OPTION}>{defaultLabel}</option>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {optionLabel(option)}
          </option>
        ))}
        <option value={CUSTOM_OPTION}>Custom…</option>
      </select>
      {isCustomShown && (
        <input
          value={customText}
          onChange={(e) => setCustomText(e.target.value)}
          onBlur={commitCustom}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitCustom();
          }}
          disabled={disabled}
          placeholder="model ID"
          spellCheck={false}
          className="min-w-0 flex-1 bg-[var(--nt-input-bg)] text-[var(--nt-fg)] border border-[var(--nt-border)] rounded px-1 py-0.5 text-[11px] font-mono outline-none focus:border-[var(--nt-bar-bg)]"
        />
      )}
    </div>
  );
}
