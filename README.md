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
| ファイル転送（SFTP） | サイドバー「Files」のツリーへファイルをドラッグ＆ドロップでアップロード。ファイルを右クリック →「Download...」でダウンロード。大きなファイルも別の接続で転送するため、転送中もターミナルを操作できます |
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

`Ctrl+C` などはそのままリモートへ送られます。上記のキーはアプリ側で使うため、リモートには送られません。

## 作成されるファイル（exe と同じフォルダ）

| ファイル | 内容 |
| :--- | :--- |
| `servers.json` | サーバの一覧（接続先・認証方式・環境・テーマなど） |
| `credentials.json` | 保存したパスワード（DPAPI で暗号化。**他の PC・他のユーザーでは使えません**） |
| `known_hosts` | 接続したサーバのホスト鍵（OpenSSH 形式） |
| `themes.json` | カラーテーマ（初回起動時に既定の内容で作成） |
| `logs\` | ログ取得の既定の保存先 |

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
bun install
bun run tauri dev                  # 開発用に起動（設定は src-tauri/target/debug に保存）
bun run tauri build --no-bundle    # src-tauri/target/release/Orca.exe を作成
```

`Orca` は AI あり版（正規版。AI 機能は開発中、設計は [AI.md](AI.md)）です。AI 機能を含まない `Orca-NoAI` は次のコマンドで作ります。

```sh
bun run tauri:dev:noai                 # 開発用に起動
bun run tauri:build:noai --no-bundle   # src-tauri/target/release/Orca-NoAI.exe を作成
bun run check:no-ai                    # Orca-NoAI.exe と dist/ に AI のコードが含まれていないことを確認
```

フロントエンドの出力先は分けています（Orca は `dist-ai/`、Orca-NoAI は `dist/`）。exe は同じ `src-tauri/target/release/` に出力し、Windows はファイル名の大文字・小文字を区別しないため、cargo の出力 `orca.exe` と `Orca.exe` は同じファイルです。**両方が必要な場合は Orca-NoAI → Orca の順にビルドしてください**（逆の順にすると、Orca-NoAI のビルドで `Orca.exe` が `Orca-NoAI.exe` に置き換わります）。

設計資料: [RequirementsSpecification.md](RequirementsSpecification.md) / [Tech-stack.md](Tech-stack.md) / [TODO.md](TODO.md)
