
# 技術設計書：自分用ターミナルクライアント (Tera Term代替ツール)

## 1. 技術スタック選定

| レイヤー | 採用技術 / ライブラリ | 選定理由 |
| :--- | :--- | :--- |
| **デスクトップ基盤** | **Tauri v2** | メモリ消費が少なく軽量。Windows標準のWebView2を利用するため、インストーラ不要の単一フォルダ展開（ポータブル実行）に最適。WebView2のユーザーデータフォルダはアプリと同階層に指定する。 |
| **フロントエンド** | **TypeScript + React** | タブのD&D並び替え（`@dnd-kit` 等）やディレクトリツリーコンポーネントのエコシステムが充実しているため。 |
| **ターミナル描画** | **xterm.js (`@xterm/xterm`)** | VS Code等でも採用されている業界標準。ANSIエスケープシーケンスの解釈とWebGLアクセラレーションによる高速描画が可能。 |
| **SSH/SFTP/SCP通信** | **`ssh2` クレート (libssh2)** | C言語ベースのlibssh2ラッパー。`ssh-rsa`（SHA-1）や古い暗号スイートを幅広くサポートし、社内のレガシー機器とも確実に通信可能。SFTP API（`Session::sftp`）とSCP API（`scp_send` / `scp_recv`）を両方提供。SFTP優先・SCPはファイル転送のフォールバック。暗号バックエンドは**OpenSSL（`vendored-openssl` で静的リンク）**を採用（WinCNGは `ssh-ed25519` / `ecdsa` / `curve25519` 等が使えないため）。ホスト鍵検証は `KnownHosts` API、Keepaliveは `set_keepalive` を使用。 |
| **ログ整形** | **`vte` クレート** | Alacritty由来のステートフルなVTパーサ。チャンク境界で分断されたエスケープシーケンスも正しく扱えるため、ストリーミングでのANSI除去に使用。パース結果を「行バッファ＋カーソル位置」に適用し（BS・CR・`CSI n D/C`・`CSI K`・`CSI P`・`CSI @` 等）、LF受信時に確定行として書き出す。 |
| **パスワード暗号化** | **Windows DPAPI（`windows` クレートの `CryptProtectData` / `CryptUnprotectData`）** | OS標準機能のため鍵管理が不要。ログオンユーザに紐づくため、同一PC・同一ユーザ以外では復号できない。 |
| **文字コード変換** | **`encoding_rs`** | Mozilla開発のエンコーディング変換ライブラリ。デフォルトUTF-8、ユーザ選択でShift_JIS / EUC-JP等へ切り替え。ストリーミングデコーダ（`Decoder`）でチャンク境界のマルチバイト分断にも対応。自動判定は行わないため `chardetng` は不採用。 |
| **設定シリアライズ** | **`serde` + `serde_json`** | Rust標準の型安全なJSONパース・シリアライズ機構。 |

---

## 2. データモデル & 設定スキーマ設計

### ① `servers.json`
```json
[
  {
    "id": "srv-001",
    "name": "本番 Web01",
    "host": "192.168.10.11",
    "port": 22,
    "user": "appuser",
    "auth": {
      "type": "password",
      "keyFile": null
    },
    "env": "production",
    "theme": "danger-red",
    "encoding": "UTF-8",
    "fileTransfer": "auto",
    "log": {
      "dir": "./logs/production",
      "mode": "append",
      "filenamePattern": "{env}_{name}_{YYYYMMDD}.log"
    }
  }
]
```

| フィールド | 値 | 説明 |
| :--- | :--- | :--- |
| `auth.type` | `"password"` / `"publickey"` / `"keyboard-interactive"` | 認証方式。 |
| `auth.keyFile` | 文字列 / `null` | `publickey` 時の秘密鍵ファイルパス（OpenSSH形式）。 |
| `auth.remember` | （使用しない） | 旧仕様の項目。記載があっても無視する。パスワードを保存しているかどうかは `credentials.json` にエントリがあるかで判定する（③参照）。 |
| `env` | `"production"` / `"staging"` / `"development"` 等 | 環境識別子。`theme` 省略時の既定テーマ選択に使用。 |
| `theme` | `themes.json` のテーマ名 / 省略可 | 指定時はenvに関係なくこのテーマを適用。省略時はenvに応じた既定テーマ。 |
| `encoding` | `"UTF-8"`（既定） / `"Shift_JIS"` / `"EUC-JP"` 等 | 画面描画とログ保存に適用する文字コード。省略時はUTF-8。接続中もメニューから変更可能。 |
| `fileTransfer` | `"auto"`（既定） / `"sftp"` / `"scp"` | `auto` はSFTPを試し、使えなければファイル転送のみSCPにフォールバック。 |
| `log.dir` | 文字列 | ログ保存先の既定値。省略時は全体設定の値。 |
| `log.mode` | `"append"` / `"overwrite"` | 同名ファイルが存在する場合に追記するか上書きするか。 |
| `log.filenamePattern` | 文字列 | ログ開始ダイアログに表示する既定ファイル名。接続ごとに分けたい場合は `{HHmmss}` 等を含める。 |

* ログは自動開始しない。メニュー「ログ取得」→ ダイアログ → 「スタート」で開始する（`log` はダイアログの初期値）。
* ログはデコード後のテキストからANSIエスケープシーケンスを除去し、BS等による行内編集を適用した確定行のみを、各行先頭にタイムスタンプを付与してUTF-8で書き出す（改行位置は受信どおり）。

### ② `themes.json`
```json
{
  "default-dark": {
    "background": "#1e1e1e",
    "foreground": "#d4d4d4",
    "cursor": "#ffffff",
    "selectionBackground": "#264f78",
    "black": "#000000",
    "red": "#cd3131",
    "green": "#0dbc79",
    "yellow": "#e5e510",
    "blue": "#2472c8",
    "magenta": "#bc3fbc",
    "cyan": "#11a8cd",
    "white": "#e5e5e5",
    "brightBlack": "#666666",
    "brightRed": "#f14c4c",
    "brightGreen": "#23d18b",
    "brightYellow": "#f5f543",
    "brightBlue": "#3b8eea",
    "brightMagenta": "#d670d6",
    "brightCyan": "#29b8db",
    "brightWhite": "#e5e5e5"
  },
  "danger-red": {
    "background": "#241414",
    "foreground": "#ffd2d2",
    "cursor": "#ff5555",
    "selectionBackground": "#661111"
  }
}
```

* キー名は xterm.js の `ITheme` に合わせる（通常8色 `black`〜`white` ＋ 明るい8色 `brightBlack`〜`brightWhite` ＝ ANSI 16色）。
* テーマで省略したキーは `default-dark` の値を引き継ぐ（例: `danger-red` は背景・文字・カーソル・選択色のみ上書きし、ANSI 16色は `default-dark` を使用）。

### ③ `credentials.json`
```json
{
  "srv-001": {
    "secret": "<DPAPIで暗号化したバイト列のBase64>"
  }
}
```

* キーは `servers.json` の `id`。`secret` はパスワード（`password` / `keyboard-interactive`）またはパスフレーズ（`publickey`）。
* DPAPIで暗号化されているため、別PC・別ユーザでは復号できない（持ち運び対象外、Git管理対象外）。
* 暗号化時にアプリ固有の追加エントロピーを付与し、同じWindowsユーザーで動く他アプリがDPAPIで復号しても値を取り出せないようにする。
* **保存・削除のタイミング**:
  * 接続時の入力ダイアログで「Save password」にチェックを入れ、**認証に成功した場合のみ**保存する（誤ったパスワードは保存しない）。
  * チェックを外して認証に成功した場合は、保存済みの値を削除する（チェック状態と保存状態を一致させる）。
  * サーバ編集ダイアログの「Forget」、またはサーバ削除時にも削除する。
* **接続時の動作**:
  * 保存済みの値があれば入力ダイアログを出さずに接続する。
  * 保存済みの値で認証に失敗した場合（パスワード変更等）は、エラー内容を表示して入力ダイアログを出す。
  * 保存に失敗した場合も接続は継続し、ターミナルに警告を表示する。
* 書き込みは一時ファイル経由で置き換え、ロックで直列化する（`servers.json` / `known_hosts` と同様）。

### ④ `settings.json`（全体設定・たたき台）
```json
{
  "log": {
    "dir": "./logs",
    "mode": "append",
    "filenamePattern": "{env}_{name}_{YYYYMMDD}.log"
  },
  "envThemes": {
    "production": "danger-red",
    "default": "default-dark"
  },
  "terminal": {
    "fontFamily": "Cascadia Mono, BIZ UDGothic, monospace",
    "fontSize": 14,
    "scrollback": 10000,
    "cursorStyle": "block",
    "cursorBlink": true,
    "copyOnSelect": true,
    "rightClickPaste": true,
    "confirmMultilinePaste": true,
    "bell": "visual"
  },
  "ssh": {
    "connectTimeoutSec": 15,
    "keepaliveIntervalSec": 60
  },
  "fileTransfer": {
    "downloadDir": "./downloads",
    "onConflict": "ask"
  },
  "keybindings": {
    "nextTab": "Ctrl+Tab",
    "prevTab": "Ctrl+Shift+Tab",
    "closeTab": "Ctrl+Shift+W",
    "duplicateTab": "Ctrl+Shift+D",
    "renameTab": "F2",
    "copy": "Ctrl+Shift+C",
    "paste": "Ctrl+Shift+V",
    "find": "Ctrl+Shift+F",
    "toggleLog": "Alt+L",
    "toggleSidebar": "Ctrl+B"
  }
}
```

| フィールド | 値 | 説明 |
| :--- | :--- | :--- |
| `log.*` | `servers.json` の `log` と同じ | 全体の既定値。サーバ側の `log` に指定があればそちらを優先。 |
| `envThemes` | `{ env名: テーマ名 }` | `servers.json` で `theme` 省略時に使うテーマ。該当envがなければ `default`。 |
| `terminal.fontFamily` | CSSフォント指定 | 英字フォント → 日本語フォント → 汎用の順にフォールバック。 |
| `terminal.scrollback` | 数値（既定 10000） | スクロールバック保持行数。 |
| `terminal.cursorStyle` | `"block"` / `"underline"` / `"bar"` | カーソル形状。 |
| `terminal.copyOnSelect` | `true`（既定） / `false` | マウス選択で自動コピー（Tera Term方式）。 |
| `terminal.rightClickPaste` | `true`（既定） / `false` | 右クリックで貼り付け。`false` の場合は右クリックでコンテキストメニュー。 |
| `terminal.confirmMultilinePaste` | `true`（既定） / `false` | 複数行貼り付け時に確認ダイアログを表示。 |
| `terminal.bell` | `"visual"` / `"sound"` / `"none"` | BEL受信時の動作。 |
| `ssh.connectTimeoutSec` | 秒 | 接続タイムアウト。 |
| `ssh.keepaliveIntervalSec` | 秒（`0` で無効） | Keepalive送信間隔。 |
| `fileTransfer.downloadDir` | 文字列 | ダウンロード時の既定保存先（右クリック「ダウンロード」時）。 |
| `fileTransfer.onConflict` | `"ask"` / `"overwrite"` / `"skip"` | 転送先に同名ファイルがある場合の動作。 |
| `keybindings.*` | キー文字列 | アプリのショートカット。 |

* `settings.json` が存在しない、またはキーが省略された場合はアプリ内蔵の既定値（上記の値）を使用する。
* ショートカットは、リモート側で使う `Ctrl+C` / `Ctrl+V` / `Ctrl+W` 等と衝突しないよう、`Ctrl+Shift+〜` / `Alt+〜` を基本とする。
* `toggleLog` はログ未取得時は「ログ取得ダイアログ」を開き、取得中は停止する。
* 設定値の優先順位: セッション中の手動変更 ＞ `servers.json` ＞ `settings.json` ＞ アプリ内蔵既定値。

---

## 3. 技術リスク・検証事項
* **レガシーアルゴリズムの実機検証**: 実装初期に、社内の古い機器・サーバ（`diffie-hellman-group1-sha1` / `ssh-rsa` / `3des-cbc` 等を要求するもの）への接続をlibssh2（OpenSSLバックエンド）で確認する。接続できない機器があれば、その時点で対応方針（ビルド設定の調整・ライブラリ見直し）を検討する。