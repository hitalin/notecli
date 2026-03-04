# notecli

Misskey をヘッドレスで操作する Rust ライブラリ & CLI デーモン。

GUI なしで Misskey の主要機能（タイムライン取得、投稿、リアクション、ストリーミング等）を CLI または REST API 経由で利用できます。[NoteDeck](https://github.com/hitalin/notedeck) の Rust バックエンドとしても利用されています。

## インストール

```sh
cargo install --git https://github.com/hitalin/notecli.git
```

## CLI

### アカウント管理

```sh
notecli login misskey.io          # MiAuth でアカウント登録
notecli accounts                  # 登録済みアカウント一覧
notecli logout @user@misskey.io   # アカウント削除
```

### ノート操作

```sh
notecli post "Hello!" [--cw TEXT] [--visibility home] [--reply-to ID] [--local-only]
notecli note <ID>                 # ノート詳細
notecli update <ID> "新しいテキスト" [--cw TEXT]  # ノート編集
notecli delete <ID>               # ノート削除
notecli renote <ID>               # リノート（ブースト）
```

### タイムライン・検索

```sh
notecli tl [home|local|social|global] [-l 20]   # タイムライン取得
notecli search "キーワード" [-l 20]              # 全文検索
notecli replies <ID> [-l 20]                     # 返信一覧
notecli thread <ID> [-l 20]                      # 会話スレッド
```

### 通知・メンション

```sh
notecli notifications [-l 20]    # 通知一覧
notecli mentions [-l 20]         # メンション一覧
```

### リアクション

```sh
notecli react <NOTE_ID> ":star:"   # リアクション追加
notecli unreact <NOTE_ID>          # リアクション削除
```

### ユーザー操作

```sh
notecli user @user@host           # ユーザー詳細
notecli user-notes <USER_ID> [-l 20]  # ユーザーのノート一覧
notecli follow <USER_ID>          # フォロー
notecli unfollow <USER_ID>        # フォロー解除
```

### お気に入り

```sh
notecli favorite <NOTE_ID>       # お気に入り追加
notecli unfavorite <NOTE_ID>     # お気に入り削除
notecli favorites [-l 20]        # お気に入り一覧
```

### その他

```sh
notecli emojis                    # カスタム絵文字一覧
notecli daemon [--port 19820]     # HTTP APIサーバー起動
```

### 共通オプション

| オプション | 説明 |
|-----------|------|
| `--account / -a` | 操作するアカウントを指定 |
| `--json` | JSON 配列/オブジェクトで出力 |
| `--jsonl` | NDJSON 出力（jq 向け） |
| `--compact / -c` | TSV 1行1レコード（fzf/grep 向け） |
| `--ids` | ID のみ出力（パイプ/xargs 向け） |

### Unix ツール連携

```sh
notecli tl -c | fzf --with-nth=2.. | cut -f1 | xargs -I{} notecli react {} :star:
notecli tl --ids -l 5 | xargs -I{} notecli react {} :thumbsup:
notecli tl --jsonl | jq -r 'select(.user.username == "taka") | .id'
notecli tl -c -l 100 | grep "Rust" | cut -f1
```

## HTTP API

デーモン起動後、`localhost:19820` で REST API を提供します。

```sh
TOKEN=$(cat ~/.local/share/notecli/api-token)

# エンドポイント一覧（認証不要）
curl http://localhost:19820/api

# アカウント一覧（認証不要）
curl http://localhost:19820/api/accounts

# タイムライン
curl -H "Authorization: Bearer $TOKEN" http://localhost:19820/api/{host}/timeline/home

# ノート投稿
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"text": "Hello from notecli!"}' \
  http://localhost:19820/api/{host}/note

# SSE イベントストリーム
curl -N -H "Authorization: Bearer $TOKEN" http://localhost:19820/api/events
```

### エンドポイント一覧

| メソッド | パス | 説明 |
|---------|------|------|
| GET | `/api` | エンドポイント一覧 + トークンパス |
| GET | `/api/accounts` | アカウント一覧 |
| GET | `/{host}/timeline/{type}` | タイムライン取得 |
| GET | `/{host}/notifications` | 通知取得 |
| POST | `/{host}/note` | ノート投稿 |
| GET | `/{host}/notes/{id}` | ノート詳細 |
| DELETE | `/{host}/notes/{id}` | ノート削除 |
| GET | `/{host}/notes/{id}/children` | リプライ一覧 |
| GET | `/{host}/notes/{id}/conversation` | 会話スレッド |
| GET | `/{host}/notes/{id}/reactions` | リアクション一覧 |
| POST | `/{host}/notes/{id}/reactions` | リアクション追加 |
| DELETE | `/{host}/notes/{id}/reactions` | リアクション削除 |
| GET | `/{host}/users/{id}` | ユーザー詳細 |
| GET | `/{host}/users/{id}/notes` | ユーザーのノート |
| GET | `/{host}/search?q=...` | ノート検索 |
| GET | `/events` | SSE イベントストリーム |

## ライブラリとして使う

```toml
[dependencies]
notecli = { git = "https://github.com/hitalin/notecli.git" }
```

```rust
use notecli::streaming::FrontendEmitter;

// FrontendEmitter を実装すればストリーミングイベントを任意の宛先に転送可能
struct MyEmitter;
impl FrontendEmitter for MyEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        println!("[{event}] {payload}");
    }
}
```

### モジュール構成

| モジュール | 役割 |
|-----------|------|
| `api` | Misskey HTTP API クライアント |
| `db` | SQLite データベース（WAL、FTS5 全文検索） |
| `streaming` | WebSocket ストリーミング（自動再接続） |
| `http_server` | Axum HTTP API サーバー |
| `event_bus` | tokio broadcast ベースの pub/sub |
| `models` | データモデル（Raw → Normalized 変換） |
| `keychain` | OS ネイティブ資格情報ストレージ |
| `error` | 統一エラー型 |

## 認証

起動ごとにランダムトークンを生成し `{data_dir}/api-token` に書き出します（Unix: 0600）。
`/api` と `/api/accounts` 以外の全リクエストに `Authorization: Bearer {token}` ヘッダーが必要です。

アカウントの API トークンは OS のキーチェーン（Linux: Secret Service、macOS: Keychain、Windows: Credential Manager）に保存されます。

## License

MIT
