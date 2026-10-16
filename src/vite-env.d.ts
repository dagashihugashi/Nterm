/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** "1" のとき AI あり版（.env.ai / `vite --mode ai`） */
  readonly VITE_NTERM_AI?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
