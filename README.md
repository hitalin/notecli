# notecli

Misskey をヘッドレスで操作する Rust ライブラリ & CLI デーモン。

GUI なしで Misskey の全機能（タイムライン取得、投稿、リアクション、ストリーミング等）を REST API 経由で利用できます。

## 使い方

```sh
cargo install --git https://github.com/hitalin/notecli.git
notecli
```

起動すると `localhost:19820` で HTTP API サーバーが立ち上がります。

```sh
# トークンを読み取り
TOKEN=$(cat ~/.local/share/notecli/api-token)

# アカウント一覧
curl -H "Authorization: Bearer $TOKEN" http://localhost:19820/api/accounts

# タイムライン取得
curl -H "Authorization: Bearer $TOKEN" http://localhost:19820/api/{host}/timeline/home

# ノート投稿
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"text": "Hello from notecli!"}' \
  http://localhost:19820/api/{host}/note

# SSE でリアルタイムイベントを受信
curl -N -H "Authorization: Bearer $TOKEN" http://localhost:19820/api/events
```

認証不要の `GET /api` でエンドポイント一覧とトークンファイルのパスを確認できます。

## ライブラリとして使う

```toml
[dependencies]
notecli = { git = "https://github.com/hitalin/notecli.git" }
```

```rust
use std::sync::Arc;
use notecli::streaming::{FrontendEmitter, StreamingManager};

// FrontendEmitter を実装すればストリーミングイベントを任意の宛先に転送可能
struct MyEmitter;
impl FrontendEmitter for MyEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        println!("[{event}] {payload}");
    }
}
```

[NoteDeck](https://github.com/hitalin/notedeck) の Rust バックエンドとしても利用されています。

## 認証

起動ごとにランダムトークンを生成し `{data_dir}/api-token` に書き出します（Unix: 0600）。
全リクエストに `Authorization: Bearer {token}` ヘッダーが必要です。

## License

MIT
