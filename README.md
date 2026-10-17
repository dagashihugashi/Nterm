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

**この PC にインストールされている Claude Code（`claude` コマンド）を使います。** Claude Code をインストールし、ターミナルで `claude` が使える（ログイン済み）状態にしてください。認証（サブスクリプション・Amazon Bedrock 等）・ユーザの CLAUDE.md と設定（`%USERPROFILE%\.claude\`）・会社の管理設定は、普段の Claude Code のものがそのまま使われます。

作業フォルダの設定（`.claude\settings.json`・`settings.local.json`）は、安全のため読み込みません（中の hooks 等で PC 上のコマンドが動いてしまうため）。作業フォルダの CLAUDE.md は、Orca が文章だけを読んで Claude Code に渡します（`@ファイル名` の取り込みは使えません）。

### 仕組み

Orca は Claude Code を窓を出さずに起動し、PC の中だけで動く MCP サーバ（127.0.0.1 でだけ待ち受け、起動ごとの鍵で認証）を通して、サーバを読む道具を Claude Code に渡します。VS Code の拡張機能と同じ仕組みです。

- **サーバは読み取りのみ**: Claude Code がサーバに対してできるのは、Orca の道具（ファイルの一覧・属性・範囲読み・末尾読み・文字列検索、ターミナルの直近の出力の読み取り）だけです。コマンドの実行・ファイルの変更・ターミナルへの入力はできず、サーバから外部への通信も行いません。
- **手元の PC で使える道具**: 読み取り（Read / Grep / Glob）と Web の検索（WebSearch）だけです。Bash・ファイルの編集は使えません（AI が PC から ssh してサーバを操作することもできません）。Web の検索は確認なしで使えます。Web のページの取得（WebFetch）は使えません（PC から直接 Web に接続するため）。
- **毎回確認**: AI がサーバを読むたびに、道具・サーバ・環境・読み取るユーザー・**サーバで実際に実行するコマンド**を表示して確認します（本番環境・root は赤で強調）。「Allow ○○ on this tab in this chat」を選ぶと、そのチャットの間は、同じタブ・同じユーザでの同じ道具を確認なしで許可します（本番環境・root で読む場合は毎回確認します）。作業フォルダの外のファイルを読む場合も、Claude Code が Orca に確認を求めます（毎回確認。ネットワーク上のパスは Orca が自動で断ります）。
- **確認は飛ばせません**: Claude Code の設定（`permissions.allow` や `bypassPermissions` 等）にかかわらず、Orca は承認カードで許可した呼び出しだけをサーバで実行します。
- **読み取るユーザー**: ユーザー切り替えの自動化を設定しているサーバでは、切り替えた後のユーザー（root 等）の権限で読みます（ファイルツリーと同じ）。
- **コマンドの提案**: AI が提案したコマンドは、コードブロックの「Insert」でターミナルに挿入できます（Enter は送らないので、内容を確認してから実行してください）。見えない文字や文字の向きを変える文字（見た目と中身を食い違わせられる文字）は `<U+202E>` のように表示し、それを含むコードブロックは挿入できません。
- **引用**: ターミナルで選択して `Ctrl+Shift+A`（またはチャット欄の「Quote selection」）で、選択範囲を質問に貼り付けます。
- **送信されるデータ**: 質問と、AI が読んだファイルの内容・ターミナルの出力・Web のページの内容は、Claude Code を通して AI サービスへ送信されます。

### 作業フォルダとセッション

- チャット欄の上部に、Claude Code の作業フォルダ（既定は exe の隣の空のフォルダ `ai-workspace\`）が表示されます。作業フォルダの中のファイルは Claude Code が確認なしで読めるため、鍵や認証情報のあるフォルダ（ホームのフォルダ等）は選ばないでください。「Change」で変更できます。作業フォルダの CLAUDE.md が読み込まれ、会話はそのフォルダのセッションとして Claude Code が保存します（`%USERPROFILE%\.claude\projects\`）。
- 時計のアイコンで、作業フォルダの保存されたセッションの一覧を開き、選ぶと続きから質問できます（同じフォルダなら VS Code 等で始めたセッションも再開できます）。前の会話（直近 300 件まで）がチャット欄に表示されます。
- 「Stop」で中止すると Claude Code を終了させ、次の質問で同じセッションを続けます。
- 回答中は、入力欄の上に状態（考えている・書いている・道具の実行中・承認待ち）と経過時間・出力トークン数が表示されます。

### モデル

- チャット欄の上部に、Claude Code のバージョンと実際に使っているモデル（最初の質問の後）が表示されます。
- 「Model」で使うモデルを選べます。選択肢は Claude Code の設定ファイル（`%USERPROFILE%\.claude\settings.json`、作業フォルダの `.claude\settings.json` / `settings.local.json`、管理者の設定）の `model`・`availableModels`・`env` の `ANTHROPIC_MODEL` / `ANTHROPIC_DEFAULT_SONNET_MODEL` 等から作ります（モデル以外の値は読みません）。「Custom…」で任意のモデル ID・推論プロファイルの ARN も指定できます。
- 会話の途中でモデルを変えると、次の質問から新しいモデルで同じ会話を続けます。

## 作成されるファイル（exe と同じフォルダ）

| ファイル | 内容 |
| :--- | :--- |
| `servers.json` | サーバの一覧（接続先・認証方式・環境・テーマなど） |
| `credentials.json` | 保存したパスワード（DPAPI で暗号化。**他の PC・他のユーザーでは使えません**） |
| `known_hosts` | 接続したサーバのホスト鍵（OpenSSH 形式） |
| `themes.json` | カラーテーマ（初回起動時に既定の内容で作成） |
| `logs\` | ログ取得の既定の保存先 |
| `ai-workspace\` | AI アシスタント（Claude Code）の既定の作業フォルダ（Orca のみ。空のフォルダ） |

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

ビルドに必要なもの:

- Node.js（npm）
- Rust（MSVC 版。`rustup` の既定）
- Visual Studio Build Tools（「C++ によるデスクトップ開発」：MSVC と Windows SDK。Rust のリンクと OpenSSL のビルドに使います）
- Perl（OpenSSL をソースからビルドするため。Strawberry Perl のポータブル版で可。Git for Windows に付属の Perl は使えないため、PATH では Strawberry Perl を先にしてください）

実行には WebView2 ランタイム（Windows 11 は標準で入っています）、AI アシスタントには Claude Code が必要です。

```sh
npm install
npm run tauri dev                  # 開発用に起動（設定は src-tauri/target/debug に保存）
npm run tauri build -- --no-bundle    # src-tauri/target/release/Orca.exe を作成
```

`Orca` は AI あり版（正規版。設計は [AI.md](AI.md) 12 章）です。AI 機能を含まない `Orca-NoAI` は次のコマンドで作ります。

```sh
npm run tauri:dev:noai                 # 開発用に起動
npm run tauri:build:noai -- --no-bundle   # src-tauri/target/release/Orca-NoAI.exe を作成
npm run check:no-ai                    # Orca-NoAI.exe と dist/ に AI のコードが含まれていないことを確認
```

フロントエンドの出力先は分けています（Orca は `dist-ai/`、Orca-NoAI は `dist/`）。exe は同じ `src-tauri/target/release/` に出力し、Windows はファイル名の大文字・小文字を区別しないため、cargo の出力 `orca.exe` と `Orca.exe` は同じファイルです。**両方が必要な場合は Orca-NoAI → Orca の順にビルドしてください**（逆の順にすると、Orca-NoAI のビルドで `Orca.exe` が `Orca-NoAI.exe` に置き換わります）。

### 社内でビルドする場合（クレートのミラーを使う環境）

- npm のパッケージ・Rust のクレートは社内のミラーから取得します（bun は使いません）。
- `Cargo.toml` の依存は「メジャー.マイナー」までの指定にしてあり、ミラーにある互換の版で解決できます。ただし `Cargo.lock` に記録された版がミラーにない場合はビルドが止まるため、その場合は lock ファイルをミラーの版で作り直してからビルドしてください（作り直した `Cargo.lock` はコミットしなくてかまいません）。

```sh
cd src-tauri
cargo generate-lockfile   # ミラーにある版で Cargo.lock を作り直す
cd ..
npm run tauri build -- --no-bundle
```

設計資料: [RequirementsSpecification.md](RequirementsSpecification.md) / [Tech-stack.md](Tech-stack.md) / [TODO.md](TODO.md)
