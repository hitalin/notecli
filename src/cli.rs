use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;

/// Metadata for a CLI subcommand (exposed to external consumers like notedeck).
#[derive(Debug, Clone, Serialize)]
pub struct CliCommandInfo {
    pub name: String,
    pub about: Option<String>,
    pub args: Vec<CliArgInfo>,
}

/// Metadata for a single CLI argument.
#[derive(Debug, Clone, Serialize)]
pub struct CliArgInfo {
    pub name: String,
    pub help: Option<String>,
    pub required: bool,
    pub default_value: Option<String>,
}

/// Return metadata for all CLI subcommands using clap's introspection.
pub fn command_metadata() -> Vec<CliCommandInfo> {
    let cmd = Cli::command();
    cmd.get_subcommands()
        .map(|sub| CliCommandInfo {
            name: sub.get_name().to_string(),
            about: sub.get_about().map(|s| s.to_string()),
            args: sub
                .get_arguments()
                .filter(|a| {
                    let id = a.get_id().as_str();
                    // Exclude clap built-ins and global options inherited from parent
                    id != "help" && id != "version"
                })
                .map(|a| CliArgInfo {
                    name: a.get_id().to_string(),
                    help: a.get_help().map(|s| s.to_string()),
                    required: a.is_required_set(),
                    default_value: a
                        .get_default_values()
                        .first()
                        .and_then(|v| v.to_str())
                        .map(|s| s.to_string()),
                })
                .collect(),
        })
        .collect()
}

#[derive(Parser)]
#[command(
    name = "notecli",
    about = "Headless Misskey client (CLI & HTTP API)",
    long_about = "Headless Misskey client for humans and AI agents.\n\n\
        Misskey インスタンスへの投稿、タイムライン取得、リアクション、\n\
        ユーザー操作などを CLI から実行できます。\n\n\
        データは ~/.local/share/notecli/ に保存されます。\n\
        認証トークンは OS のキーチェーン（利用可能な場合）に安全に保管されます。",
    after_long_help = "出力形式:\n\
        \x20 (デフォルト)  人間向け複数行表示\n\
        \x20 --json        JSON配列/オブジェクト\n\
        \x20 --jsonl       NDJSON (1行1JSON、jq向け)\n\
        \x20 --compact/-c  TSV 1行1レコード (fzf/grep向け)\n\
        \x20 --ids         IDのみ (パイプ/xargs向け)\n\n\
        使用例:\n\
        \x20 notecli login misskey.io\n\
        \x20 notecli post \"Hello, world!\"\n\
        \x20 notecli tl home -l 10\n\
        \x20 notecli react <NOTE_ID> \":star:\"\n\n\
        Unix ツール連携:\n\
        \x20 notecli tl -c | fzf --with-nth=2.. | cut -f1 | xargs -I{} notecli react {} :star:\n\
        \x20 notecli tl --ids -l 5 | xargs -I{} notecli react {} :thumbsup:\n\
        \x20 notecli tl --jsonl | jq -r 'select(.user.username == \"taka\") | .id'\n\
        \x20 notecli tl -c -l 100 | grep \"Rust\" | cut -f1"
)]
pub struct Cli {
    /// 操作するアカウントを指定 (形式: @user@host, アカウントID, ユーザー名)
    #[arg(long, short = 'a', global = true)]
    pub account: Option<String>,

    /// JSON配列/オブジェクトで出力
    #[arg(long, global = true, group = "output_format")]
    pub json: bool,

    /// IDのみ出力、1行1ID (パイプ/xargs向け)
    #[arg(long, global = true, group = "output_format")]
    pub ids: bool,

    /// TSV 1行1レコードで出力 (fzf/grep/awk向け)
    #[arg(long, short = 'c', global = true, group = "output_format")]
    pub compact: bool,

    /// NDJSON出力、1行1JSONオブジェクト (jqストリーミング向け)
    #[arg(long, global = true, group = "output_format")]
    pub jsonl: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// HTTP APIサーバーを起動
    #[command(
        long_about = "HTTP APIサーバー（REST + SSE）をバックグラウンドで起動します。\n\
            外部アプリや Web フロントエンドからの連携に使用します。\n\
            起動時にランダムなAPIトークンが生成され、ファイルに保存されます。"
    )]
    Daemon {
        /// 待ち受けポート番号
        #[arg(long, default_value_t = 19820)]
        port: u16,
    },

    /// 登録済みアカウント一覧を表示
    #[command(
        long_about = "データベースに登録されている全アカウントを一覧表示します。\n\
            認証不要で実行できます。"
    )]
    Accounts,

    /// アカウントを登録 (MiAuth認証)
    #[command(
        long_about = "MiAuth を使って Misskey インスタンスにログインし、\n\
            アカウントを登録します。\n\n\
            1. 認証URLが表示されるのでブラウザで開く\n\
            2. インスタンスで認証を許可する\n\
            3. 戻ってきたらEnterを押して完了",
        after_long_help = "使用例:\n\
            \x20 notecli login misskey.io\n\
            \x20 notecli login nijimiss.moe"
    )]
    Login {
        /// Misskey インスタンスのホスト名 (例: misskey.io)
        host: String,
    },

    /// アカウントを削除
    #[command(
        long_about = "登録済みアカウントをローカルデータベースから削除します。\n\
            インスタンス側のアカウントは影響を受けません。"
    )]
    Logout {
        /// 削除するアカウント (形式: @user@host, アカウントID, ユーザー名)
        target: String,
    },

    /// ノートを投稿
    #[command(
        long_about = "Misskey にノート（投稿）を作成します。\n\
            テキスト、CW（Content Warning）、公開範囲、返信先を指定できます。",
        after_long_help = "使用例:\n\
            \x20 notecli post \"Hello, world!\"\n\
            \x20 notecli post \"ネタバレ\" --cw \"映画の感想\"\n\
            \x20 notecli post \"返信です\" --reply-to 9abcdef123\n\
            \x20 notecli post \"ローカルのみ\" --local-only --visibility home"
    )]
    Post {
        /// 投稿するテキスト
        text: String,
        /// Content Warning（閲覧注意の説明文）
        #[arg(long)]
        cw: Option<String>,
        /// 公開範囲: public, home, followers, specified
        #[arg(long, default_value = "public")]
        visibility: String,
        /// 返信先のノートID
        #[arg(long)]
        reply_to: Option<String>,
        /// ローカルのみに公開（連合しない）
        #[arg(long)]
        local_only: bool,
    },

    /// タイムラインを取得
    #[command(
        long_about = "指定したタイプのタイムラインからノートを取得します。\n\n\
            タイプ:\n\
            \x20 home   - ホームタイムライン（フォロー中のユーザーの投稿）\n\
            \x20 local  - ローカルタイムライン（同じインスタンスの投稿）\n\
            \x20 social - ソーシャルタイムライン（ローカル + フォロー中）\n\
            \x20 global - グローバルタイムライン（連合の全投稿）",
        after_long_help = "使用例:\n\
            \x20 notecli tl\n\
            \x20 notecli tl local -l 10\n\
            \x20 notecli tl -c | fzf --with-nth=2.. | cut -f1"
    )]
    Tl {
        /// タイムラインの種類: home, local, social, global
        #[arg(default_value = "home")]
        r#type: String,
        /// 取得するノート数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// ノートを全文検索
    #[command(
        long_about = "キーワードでノートを全文検索します。\n\
            サーバー側の検索APIを使用します。",
        after_long_help = "使用例:\n\
            \x20 notecli search \"Rust言語\"\n\
            \x20 notecli search \"猫の写真\" -l 5"
    )]
    Search {
        /// 検索キーワード
        query: String,
        /// 取得するノート数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// 通知一覧を取得
    #[command(
        long_about = "リアクション、フォロー、返信、リノート、メンションなどの\n\
            通知を一覧表示します。"
    )]
    Notifications {
        /// 取得する通知数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// メンション一覧を取得
    #[command(long_about = "自分宛てのメンション（@付き投稿）を一覧表示します。")]
    Mentions {
        /// 取得する件数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// ノートの詳細を表示
    #[command(long_about = "指定したIDのノートの詳細情報を表示します。\n\
            テキスト、リアクション数、リノート数、返信数などが確認できます。")]
    Note {
        /// ノートID
        id: String,
    },

    /// ノートへの返信一覧を取得
    #[command(long_about = "指定したノートに対する返信（子ノート）を一覧表示します。")]
    Replies {
        /// 対象ノートID
        id: String,
        /// 取得する件数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// ノートの会話スレッドを表示
    #[command(long_about = "指定したノートに至るまでの会話（親ノートの連鎖）を\n\
            時系列で表示します。会話の文脈を追うのに便利です。")]
    Thread {
        /// 対象ノートID
        id: String,
        /// 取得する件数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// ノートを削除
    Delete {
        /// 削除するノートID
        id: String,
    },

    /// ノートを編集
    #[command(
        long_about = "既存のノートのテキストやCWを編集します。\n\
            ※ サーバーがノート編集に対応している必要があります。",
        after_long_help = "使用例:\n\
            \x20 notecli update 9abcdef123 \"修正後のテキスト\"\n\
            \x20 notecli update 9abcdef123 \"内容\" --cw \"注意\""
    )]
    Update {
        /// 編集するノートID
        id: String,
        /// 新しいテキスト
        text: String,
        /// 新しい Content Warning
        #[arg(long)]
        cw: Option<String>,
    },

    /// ノートにリアクションを追加
    #[command(
        long_about = "指定したノートにリアクション（絵文字）を追加します。\n\
            カスタム絵文字は :emoji_name: 形式で指定します。\n\
            Unicode絵文字はそのまま指定できます。",
        after_long_help = "使用例:\n\
            \x20 notecli react 9abcdef123 \":star:\"\n\
            \x20 notecli react 9abcdef123 \"👍\"\n\n\
            バッチ:\n\
            \x20 notecli tl --ids -l 5 | xargs -I{} notecli react {} :star:"
    )]
    React {
        /// 対象ノートID
        note_id: String,
        /// リアクション (例: :star:, :thumbsup:, 👍)
        reaction: String,
    },

    /// ノートからリアクションを削除
    #[command(long_about = "指定したノートから自分のリアクションを削除します。")]
    Unreact {
        /// 対象ノートID
        note_id: String,
    },

    /// ノートをリノート（ブースト）
    #[command(
        long_about = "指定したノートをリノート（ブースト/シェア）します。",
        after_long_help = "使用例:\n\
            \x20 notecli renote 9abcdef123"
    )]
    Renote {
        /// リノートするノートID
        note_id: String,
    },

    /// ユーザーの詳細情報を表示
    #[command(
        long_about = "ユーザーのプロフィール情報を表示します。\n\
            フォロー数、フォロワー数、ノート数、自己紹介などが確認できます。\n\n\
            ユーザーの指定方法:\n\
            \x20 @user@host  - リモートユーザー\n\
            \x20 @user       - ローカルユーザー\n\
            \x20 ユーザーID  - 内部ID指定",
        after_long_help = "使用例:\n\
            \x20 notecli user @taka@misskey.io\n\
            \x20 notecli user @admin\n\
            \x20 notecli user 9abcdef123"
    )]
    User {
        /// ユーザー指定 (@user@host, @user, またはユーザーID)
        target: String,
    },

    /// ユーザーのノート一覧を取得
    #[command(
        long_about = "指定したユーザーが投稿したノートの一覧を取得します。",
        after_long_help = "使用例:\n\
            \x20 notecli user-notes 9abcdef123\n\
            \x20 notecli user-notes 9abcdef123 -l 10"
    )]
    UserNotes {
        /// ユーザーID
        user_id: String,
        /// 取得する件数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// ユーザーをフォロー
    Follow {
        /// フォローするユーザーID
        user_id: String,
    },

    /// ユーザーのフォローを解除
    Unfollow {
        /// フォロー解除するユーザーID
        user_id: String,
    },

    /// ノートをお気に入りに追加
    Favorite {
        /// お気に入りに追加するノートID
        note_id: String,
    },

    /// ノートをお気に入りから削除
    Unfavorite {
        /// お気に入りから削除するノートID
        note_id: String,
    },

    /// お気に入りノート一覧を取得
    Favorites {
        /// 取得する件数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// サーバーのカスタム絵文字一覧を表示
    #[command(
        long_about = "インスタンスで利用可能なカスタム絵文字を一覧表示します。\n\
            リアクションに使える絵文字を確認するのに便利です。"
    )]
    Emojis,
}
