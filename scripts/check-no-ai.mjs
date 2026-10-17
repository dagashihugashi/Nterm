// AI なし版のビルド結果に AI のコードが含まれていないことを確認する（AI.md 9 章）。
//
// 使い方（AI なし版のビルド後）: npm run check:no-ai -- [exe のパス] [dist のフォルダ]
// （node で直接実行できるよう、TypeScript ではなく JavaScript で書いている）
// dist/ 以下のファイルと exe（既定: src-tauri/target/release/Orca-NoAI.exe）に、
// AI の機能だけが持つ文字列が含まれていたら失敗する。
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

/**
 * AI の機能だけが持つ文字列（AI のコードを追加したら、その機能に固有の文字列をここに足す）。
 * Rust のコマンド名は exe 内で分割されて格納されるため、exe 側は ai/mod.rs の目印（BUILD_MARKER）で検出する
 */
const AI_ONLY_STRINGS = [
  "nterm-ai-build",
  "ai_status",
  "AI Assistant",
  "bedrock-runtime",
  "api.anthropic.com",
];

const DIST_DIR = "dist";
const DEFAULT_EXE = "src-tauri/target/release/Orca-NoAI.exe";

/** @param {string} dir @returns {string[]} */
const listFiles = (dir) =>
  readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? listFiles(path) : [path];
  });

/** @param {string} path @returns {string[]} */
const findAiStrings = (path) => {
  const content = readFileSync(path);
  return AI_ONLY_STRINGS.filter((text) => content.includes(text));
};

const exePath = process.argv[2] ?? DEFAULT_EXE;
const distDir = process.argv[3] ?? DIST_DIR;
const targets = [distDir, exePath];
const missing = targets.filter((path) => !existsSync(path));
if (missing.length > 0) {
  console.error(`Not found: ${missing.join(", ")} (build the app without AI first)`);
  process.exit(2);
}

const files = [...listFiles(distDir), exePath];
const findings = files.flatMap((path) => findAiStrings(path).map((text) => `${path}: "${text}"`));

if (findings.length > 0) {
  console.error("AI code found in the build without AI:");
  findings.forEach((line) => console.error(`  ${line}`));
  process.exit(1);
}
console.log(`OK: no AI code in ${files.length} files (${AI_ONLY_STRINGS.length} patterns checked)`);
