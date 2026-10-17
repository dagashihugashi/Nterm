# Orca

Windows 向けの SSH ターミナルクライアントです（Tera Term / RLogin の代替）。
インストール不要の exe 1 つで動作し、設定ファイルはすべて exe と同じフォルダに保存されます。

## 動作環境

- Windows 10 / 11（x64）
- WebView2 ランタイム（Windows 11 は標準搭載。Windows 10 で起動しない場合は Microsoft の「WebView2 Runtime」をインストール）

## 使い始め方

1. `Orca.exe` を**書き込み可能な任意のフォルダ**に置いて起動します（インストール不要）。
   - 設定ファイルを exe と同じフォルダに作成するため、`C:\Program Files` などの書き込みできない場所は避けてください。
2. 左のサイドバーの **＋** からサーバを追加します。
3. サーバ名をクリックすると、新しいタブで接続します。

> コード署名をしていないため、初回起動時に SmartScreen の警告が表示されることがあります（「詳細情報」→「実行」で起動できます）。

## 主な機能

| 機能 | 操作 |
| :--- | :--- |
| SSH 接続 | パスワード認証 / 公開鍵認証（OpenSSH 形式の鍵）。初回接続時にホスト鍵を確認し、`known_hosts` に保存 |
| パスワード保存 | 接続時のダイアログで「Save password」。Windows の DPAPI で暗号化して保存（同じ PC・同じ Windows ユーザーでのみ復号可能） |
| ログイン後のユーザ切り替え | サーバ編集の「Switch user after login」に `su - oracle` などの手順を登録（最大5つ）。パスワード入力欄を検知したときだけパスワードを送信 |
| ログ取得 | ステータスバーの「Start logging...」または `Alt+L`。色などのエスケープを除去し、各行にタイムスタンプを付けて保存 |
| コピー / 貼り付け | 選択して `Ctrl+Shift+C` / 右クリックまたは `Ctrl+Shift+V`（複数行は確認あり） |
| ファイル転送（SFTP） | 左端のアイコンの「File Tree」で表示するツリーへファイルをドラッグ＆ドロップでアップロード。ファイルを右クリック →「Download...」でダウンロード。大きなファイルも別の接続で転送するため、転送中もターミナルを操作できます |
| 文字コード | ステータスバー右端の表示をクリック（UTF-8 / Shift_JIS / EUC-JP） |
| テーマ | `themes.json` で色を定義し、サーバ編集の「Theme」で選択。環境が Production のサーバは既定で赤系 |
| Keepalive | 60 秒ごとに送信し、無通信による切断を防止 |

### ショートカット

| キー | 動作 |
| :--- | :--- |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 次 / 前のタブ |
| `Ctrl+Shift+W` | タブを閉じる |
| `Ctrl+Shift+D` | 同じサーバへの新しい接続を開く |
| `F2` | タブ名の変更（タブのダブルクリックでも可） |
| `Alt+L` | ログ取得の開始 / 停止 |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | コピー / 貼り付け |
| `Ctrl+Shift+A` | 選択範囲を AI のチャットに引用（Orca のみ。Orca-NoAI ではリモートへ送られます） |

`Ctrl+C` などはそのままリモートへ送られます。上記のキーはアプリ側で使うため、リモートには送られません。

## AI アシスタント（Orca のみ）

左端の ✨ アイコンで右側にチャットを開き、ターミナルの出力やサーバのログ・設定ファイルについて質問できます。AI 機能を含まない版は `Orca-NoAI.exe` です。

- **読み取りのみ**: AI はファイルの一覧・属性・範囲読み・末尾読み・文字列検索と、ターミナルの直近の出力の読み取りしかできません。コマンドの実行・ファイルの変更・ターミナルへの入力はできません。AI が提案したコマンドは、コードブロックの「Insert」でターミナルに挿入できます（Enter は送らないので、内容を確認してから実行してください）。
- **毎回確認**: AI がサーバを読むたびに、ツール・サーバ・環境・読み取るユーザー・パスを表示して確認します。「Allow ○○ in this chat」を選ぶと、そのチャットの間は同じツールを確認なしで許可します。
- **読み取るユーザー**: ユーザー切り替えの自動化を設定しているサーバでは、切り替えた後のユーザー（root 等）の権限で読みます（ファイルツリーと同じ）。
- **引用**: ターミナルで選択して `Ctrl+Shift+A`（またはチャット欄の「Quote selection」）で、選択範囲を質問に貼り付けます。
- **送信されるデータ**: 質問と、AI が読んだファイルの内容・ターミナルの出力は、AI サービスへ送信されます。

### 接続先の設定

| 接続先 | 設定 |
| :--- | :--- |
| Amazon Bedrock | Claude Code の設定ファイル（`%USERPROFILE%\.claude\settings.json`）の `env` を読みます。`CLAUDE_CODE_USE_BEDROCK`・`AWS_REGION`・`AWS_BEARER_TOKEN_BEDROCK`（API キー）・`ANTHROPIC_MODEL`（モデル ID。`sonnet` 等の別名の場合は `ANTHROPIC_DEFAULT_SONNET_MODEL`）。同じ名前の環境変数があればそちらを優先します。Orca はこのファイルを書き換えません |
| Anthropic API | チャット欄の鍵のアイコンから API キーを入力します（DPAPI で暗号化して `credentials.json` に保存）。環境変数 `ANTHROPIC_API_KEY` でも指定できます |

プロキシは環境変数（`HTTPS_PROXY` 等）に従います（Claude Code と同じ）。

AI とのやりとり（質問・承認/拒否・AI が読んだ内容・回答）は、`logs\ai-audit-YYYY-MM-DD.jsonl` に記録されます。

## 作成されるファイル（exe と同じフォルダ）

| ファイル | 内容 |
| :--- | :--- |
| `servers.json` | サーバの一覧（接続先・認証方式・環境・テーマなど） |
| `credentials.json` | 保存したパスワード（DPAPI で暗号化。**他の PC・他のユーザーでは使えません**） |
| `known_hosts` | 接続したサーバのホスト鍵（OpenSSH 形式） |
| `themes.json` | カラーテーマ（初回起動時に既定の内容で作成） |
| `logs\` | ログ取得の既定の保存先。AI の監査ログ（`ai-audit-YYYY-MM-DD.jsonl`、Orca のみ）もここに保存 |

新しいバージョンに入れ替えるときは、これらのファイルを新しい exe と同じフォルダにコピーしてください。

### themes.json の例

```json
{
  "my-theme": {
    "background": "#1b2b34",
    "foreground": "#d8dee9",
    "sidebarBackground": "#22323c",
    "barBackground": "#2e7d32",
    "barForeground": "#ffffff"
  }
}
```

省略した色は `default-dark` の値を使います。タイトルバーの色の変更は Windows 11 のみ対応です。

## 注意事項・制限

- ファイルツリーは、ユーザー切り替えの自動化を設定している場合は**切り替えた後のユーザー（root 等）の権限**で表示します（表示中のユーザーはツリーの上部に「as root」のように表示）。ターミナル内で手動で `su` した場合は追従しません。
- ファイル転送は**ログインしたユーザーの権限**で動作します（root のファイルは `/tmp` 経由でやり取りしてください）。
- フォルダごとの転送、SCP、ポートフォワーディング、踏み台（ProxyJump）経由の接続には対応していません。
- `vi` / `top` などの全画面アプリの画面は、ログに正しく残りません。

## 開発者向け

```sh
npm install
npm run tauri dev                  # 開発用に起動（設定は src-tauri/target/debug に保存）
npm run tauri build -- --no-bundle    # src-tauri/target/release/Orca.exe を作成
```

`Orca` は AI あり版（正規版。AI 機能は開発中、設計は [AI.md](AI.md)）です。AI 機能を含まない `Orca-NoAI` は次のコマンドで作ります。

```sh
npm run tauri:dev:noai                 # 開発用に起動
npm run tauri:build:noai -- --no-bundle   # src-tauri/target/release/Orca-NoAI.exe を作成
npm run check:no-ai                    # Orca-NoAI.exe と dist/ に AI のコードが含まれていないことを確認
```

フロントエンドの出力先は分けています（Orca は `dist-ai/`、Orca-NoAI は `dist/`）。exe は同じ `src-tauri/target/release/` に出力し、Windows はファイル名の大文字・小文字を区別しないため、cargo の出力 `orca.exe` と `Orca.exe` は同じファイルです。**両方が必要な場合は Orca-NoAI → Orca の順にビルドしてください**（逆の順にすると、Orca-NoAI のビルドで `Orca.exe` が `Orca-NoAI.exe` に置き換わります）。

### 社内でビルドする場合（クレートのミラーを使う環境）

- OpenSSL をソースからビルドするため Perl が必要です（Strawberry Perl のポータブル版で可）。
- `Cargo.toml` の依存は「メジャー.マイナー」までの指定にしてあり、ミラーにある互換の版で解決できます。ただし `Cargo.lock` に記録された版がミラーにない場合はビルドが止まるため、その場合は lock ファイルをミラーの版で作り直してからビルドしてください（作り直した `Cargo.lock` はコミットしなくてかまいません）。

```sh
cd src-tauri
cargo generate-lockfile   # ミラーにある版で Cargo.lock を作り直す
cd ..
npm run tauri build -- --no-bundle
```

設計資料: [RequirementsSpecification.md](RequirementsSpecification.md) / [Tech-stack.md](Tech-stack.md) / [TODO.md](TODO.md)
