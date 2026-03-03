# notecli

Misskey をヘッドレスで操作する Rust ライブラリ & CLI デーモン。

GUI なしで Misskey の主要機能（タイムライン取得、投稿、リアクション、ストリーミング等）を CLI または REST API 経由で利用できます。[NoteDeck](https://github.com/hitalin/notedeck) の Rust バックエンドとしても利用されています。

## インストール

```sh
cargo install --git https://github.com/hitalin/notecli.git
```

## CLI

```sh
# デーモン起動（デフォルト: localhost:19820）
notecli daemon [--port 19820]

# アカウント一覧
notecli accounts

# 投稿
notecli post "Hello from notecli!" [--cw TEXT] [--visibility home] [--local-only]

# タイムライン取得
notecli tl [home|local|social|global] [--limit 20]

# 検索
notecli search "キーワード" [--limit 20]

# 通知
notecli notifications [--limit 20]

# ノート詳細 / 削除
notecli note <ID>
notecli delete <ID>
```

全コマンド共通オプション: `--account / -a`（アカウント指定）、`--json`（JSON 出力）

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
