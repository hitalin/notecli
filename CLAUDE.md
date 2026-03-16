# notecli - Claude Code 設定

## プロジェクト概要

Misskey クライアントライブラリ + CLI ツールの単一クレート。
NoteDeck が最大の消費者で、Rust crate 依存としてコアモジュールを直接利用している。
CLI は同じクレート内に同居する独立したフロントエンド。

## ビルド & テスト

```sh
cargo build            # ビルド
cargo clippy           # lint
cargo test             # テスト（現状少ない）
```

## アーキテクチャ原則

- **コアはライブラリ**: api, models, db, streaming, event_bus がコアロジック。CLI と HTTP daemon はフロントエンド
- **ステートレス API クライアント**: `MisskeyClient` は副作用を持たない純粋な HTTP ラッパー
- **イベント駆動**: streaming → EventBus → 複数の消費者（SSE, Tauri IPC 等）
- 詳細は ARCHITECTURE.md を参照

## notedeck からの変更を受け入れる基準

| 変更対象 | 受け入れ条件 |
|---------|-------------|
| models.rs | notecli 単体でも意味がある型・フィールドの追加 |
| api.rs | Misskey API カバレッジの向上 |
| http_server.rs | 汎用的なエンドポイント。notedeck 専用なら notedeck 側で `build_core_routes()` を拡張 |
| streaming.rs | 汎用的なイベント処理の改善 |

## コーディング規約

- エラー型は `error.rs` の `NotecliError` に統一
- API トークンをログ・エラーメッセージに含めない（`safe_message()` を使用）
- 認証情報は keychain 優先、DB フォールバック
- 新しい Misskey API エンドポイントは `api.rs` の `MisskeyClient` に追加
