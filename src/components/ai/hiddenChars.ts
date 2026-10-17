import type { ElementContent, Root, RootContent } from "hast";

/**
 * 見た目を偽装できる文字（Rust の ai::tools::is_hidden_char とほぼ同じ範囲）。
 * 文字の向きを変える文字（RLO 等）・ゼロ幅の文字・見えない書式の文字・空白に見える文字・異体字セレクタ・タグ文字・制御文字（ESC 等）。
 * AI の回答に混ぜられると、画面に見えるコマンドと実際に挿入されるコマンドが食い違う（"Trojan Source"）。
 * Rust 側との違い：改行・タブは除く（回答の本文やコードブロックで使う）。絵文字の見た目の指定（U+FE0E・U+FE0F）も
 * 警告の絵文字などで普通に使われるため除く（向きや並びは変えられない）。
 * ソースに見えない文字そのものを書かないよう、すべて \u{…} の形で書く
 */
const HIDDEN_CHARS = new RegExp(
  [
    "[",
    "\\u{0000}-\\u{0008}\\u{000B}\\u{000C}\\u{000E}-\\u{001F}\\u{007F}-\\u{009F}",
    "\\u{00AD}\\u{034F}\\u{061C}\\u{115F}\\u{1160}\\u{17B4}\\u{17B5}\\u{180B}-\\u{180F}",
    "\\u{200B}-\\u{200F}\\u{2028}\\u{2029}\\u{202A}-\\u{202E}\\u{2060}-\\u{2064}\\u{2066}-\\u{2069}",
    "\\u{2800}\\u{3164}\\u{FE00}-\\u{FE0D}\\u{FEFF}\\u{FFA0}\\u{FFF9}-\\u{FFFB}",
    "\\u{E0000}-\\u{E007F}\\u{E0100}-\\u{E01EF}",
    "]",
  ].join(""),
  "gu",
);

export const hasHiddenChars = (text: string): boolean => text.search(HIDDEN_CHARS) >= 0;

/** 見えない文字を `<U+202E>` の形にして見えるようにする（承認カードの表示と同じ形） */
export const revealHidden = (text: string): string =>
  text.replace(HIDDEN_CHARS, (c) => `<U+${(c.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, "0")}>`);

/** 見えない文字を含むコードブロック（`pre`）に付ける印（Insert を無効にする） */
export const HIDDEN_CHARS_PROPERTY = "dataHiddenChars";

/** 子孫の文字を見える形にする（見えない文字があれば true） */
function revealIn(nodes: (RootContent | ElementContent)[]): boolean {
  let found = false;
  for (const node of nodes) {
    if (node.type === "text" && hasHiddenChars(node.value)) {
      node.value = revealHidden(node.value);
      found = true;
    } else if (node.type === "element") {
      const inner = revealIn(node.children);
      if (inner && node.tagName === "pre") node.properties = { ...node.properties, [HIDDEN_CHARS_PROPERTY]: "true" };
      found = inner || found;
    }
  }
  return found;
}

/**
 * react-markdown の rehype プラグイン：回答のすべての文字（本文・インラインのコード・コードブロック）で
 * 見えない文字を見える形にし、そうしたコードブロックに印を付ける（unified のプラグインは木を直接書き換える作り）
 */
export function rehypeRevealHidden() {
  return (tree: Root) => {
    revealIn(tree.children);
  };
}
