use std::sync::Arc;

use clap::{Parser, Subcommand};
use notecli::api::MisskeyClient;
use notecli::db::Database;
use notecli::error::NoteDeckError;
use notecli::event_bus::EventBus;
use notecli::models::{
    Account, AccountPublic, CreateNoteParams, NormalizedNote, NormalizedNotification,
    NormalizedUserDetail, SearchOptions, ServerEmoji, TimelineOptions, TimelineType,
};
use notecli::streaming::{EventBusEmitter, StreamingManager};

#[derive(Parser)]
#[command(
    name = "notecli",
    about = "Headless Misskey client (CLI & HTTP API)",
    long_about = "Headless Misskey client for humans and AI agents.\n\n\
        Misskey インスタンスへの投稿、タイムライン取得、リアクション、\n\
        ユーザー操作などを CLI から実行できます。\n\n\
        データは ~/.local/share/notecli/ に保存されます。\n\
        認証トークンは OS のキーチェーン（利用可能な場合）に安全に保管されます。\n\n\
        全コマンドで --json フラグを使うと、AI エージェント向けの\n\
        構造化 JSON 出力に切り替わります。",
    after_long_help = "使用例:\n\
        \x20 アカウント登録:    notecli login misskey.io\n\
        \x20 投稿:              notecli post \"Hello, world!\"\n\
        \x20 CW付き投稿:        notecli post \"内容\" --cw \"注意\"\n\
        \x20 返信:              notecli post \"返事\" --reply-to <NOTE_ID>\n\
        \x20 タイムライン:      notecli tl home -l 10\n\
        \x20 通知確認:          notecli notifications\n\
        \x20 リアクション:      notecli react <NOTE_ID> \":star:\"\n\
        \x20 ユーザー表示:      notecli user @user@host\n\
        \x20 JSON出力(AI向け):  notecli --json tl home"
)]
struct Cli {
    /// 操作するアカウントを指定 (形式: @user@host, アカウントID, ユーザー名)
    #[arg(long, short = 'a', global = true)]
    account: Option<String>,

    /// JSON形式で出力 (AIエージェント・プログラム連携向け)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
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
            \x20 notecli tl global --limit 50"
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
    #[command(
        long_about = "自分宛てのメンション（@付き投稿）を一覧表示します。"
    )]
    Mentions {
        /// 取得する件数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// ノートの詳細を表示
    #[command(
        long_about = "指定したIDのノートの詳細情報を表示します。\n\
            テキスト、リアクション数、リノート数、返信数などが確認できます。"
    )]
    Note {
        /// ノートID
        id: String,
    },

    /// ノートへの返信一覧を取得
    #[command(
        long_about = "指定したノートに対する返信（子ノート）を一覧表示します。"
    )]
    Replies {
        /// 対象ノートID
        id: String,
        /// 取得する件数 (1-100)
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },

    /// ノートの会話スレッドを表示
    #[command(
        long_about = "指定したノートに至るまでの会話（親ノートの連鎖）を\n\
            時系列で表示します。会話の文脈を追うのに便利です。"
    )]
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
            \x20 notecli react 9abcdef123 \":thumbsup:\"\n\
            \x20 notecli react 9abcdef123 \"👍\""
    )]
    React {
        /// 対象ノートID
        note_id: String,
        /// リアクション (例: :star:, :thumbsup:, 👍)
        reaction: String,
    },

    /// ノートからリアクションを削除
    #[command(
        long_about = "指定したノートから自分のリアクションを削除します。"
    )]
    Unreact {
        /// 対象ノートID
        note_id: String,
    },

    /// ノートをリノート（ブースト）
    #[command(
        long_about = "指定したノートをリノート（ブースト/シェア）します。\n\
            引用リノートは post コマンドに --quote オプションで行えます。",
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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        None | Some(Commands::Daemon { .. }) => {
            let port = match &cli.command {
                Some(Commands::Daemon { port }) => *port,
                _ => 19820,
            };
            run_daemon(port).await;
        }
        Some(cmd) => {
            if let Err(e) = run_cli(cmd, cli.account.as_deref(), cli.json).await {
                if cli.json {
                    let err = serde_json::json!({ "error": e.code(), "message": e.safe_message() });
                    eprintln!("{}", err);
                } else {
                    eprintln!("Error: {}", e.safe_message());
                }
                std::process::exit(1);
            }
        }
    }
}

async fn run_daemon(port: u16) {
    let data_dir = dirs_data_dir().join("notecli");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let db_path = data_dir.join("notecli.db");
    let db = Arc::new(Database::open(&db_path).expect("Failed to open database"));

    let client = Arc::new(MisskeyClient::new().expect("Failed to create HTTP client"));

    let event_bus = Arc::new(EventBus::new());

    let emitter = Arc::new(EventBusEmitter::new(event_bus.clone()));
    let _streaming = StreamingManager::new(emitter, event_bus.clone(), db.clone());

    let api_token = uuid::Uuid::new_v4().to_string();
    let token_path = data_dir.join("api-token");
    std::fs::write(&token_path, &api_token).expect("Failed to write API token");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))
            .expect("Failed to set token file permissions");
    }
    let token_path_str = token_path.to_string_lossy().to_string();

    eprintln!("[notecli] data dir: {}", data_dir.display());
    eprintln!("[notecli] token: {token_path_str}");
    eprintln!("[notecli] port: {port}");

    notecli::http_server::start_on_port(db, client, event_bus, api_token, token_path_str, port)
        .await;
}

async fn run_cli(
    cmd: Commands,
    account_spec: Option<&str>,
    json: bool,
) -> Result<(), NoteDeckError> {
    let data_dir = dirs_data_dir().join("notecli");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let db_path = data_dir.join("notecli.db");
    let db = Database::open(&db_path)?;

    // Commands that don't need credentials
    match &cmd {
        Commands::Accounts => {
            let accounts = db.load_accounts()?;
            if json {
                let public: Vec<AccountPublic> = accounts.iter().map(AccountPublic::from).collect();
                println!("{}", serde_json::to_string(&public).unwrap());
            } else {
                if accounts.is_empty() {
                    println!("No accounts found. Use 'notecli login <HOST>' to add one.");
                } else {
                    for a in &accounts {
                        let name = a.display_name.as_deref().unwrap_or(&a.username);
                        println!("@{}@{} ({}) {}", a.username, a.host, a.id, name);
                    }
                }
            }
            return Ok(());
        }
        Commands::Login { host } => {
            return run_login(&db, host, json).await;
        }
        Commands::Logout { target } => {
            return run_logout(&db, target, json);
        }
        _ => {}
    }

    let account = resolve_account(&db, account_spec)?;
    let (host, token) = notecli::get_credentials(&db, &account.id)?;
    let client = MisskeyClient::new()?;

    match cmd {
        Commands::Post {
            text,
            cw,
            visibility,
            reply_to,
            local_only,
        } => {
            let params = CreateNoteParams {
                text: Some(text),
                cw,
                visibility: Some(visibility),
                local_only: if local_only { Some(true) } else { None },
                mode_flags: None,
                reply_id: reply_to,
                renote_id: None,
                file_ids: None,
            };
            let note = client
                .create_note(&host, &token, &account.id, params)
                .await?;
            if json {
                println!("{}", serde_json::to_string(&note).unwrap());
            } else {
                println!("Posted: https://{}/notes/{}", host, note.id);
            }
        }
        Commands::Tl { r#type, limit } => {
            let notes = client
                .get_timeline(
                    &host,
                    &token,
                    &account.id,
                    TimelineType::new(&r#type),
                    TimelineOptions::new(limit, None, None),
                )
                .await?;
            print_notes(&notes, json);
        }
        Commands::Search { query, limit } => {
            let notes = client
                .search_notes(
                    &host,
                    &token,
                    &account.id,
                    &query,
                    SearchOptions::new(limit),
                )
                .await?;
            print_notes(&notes, json);
        }
        Commands::Notifications { limit } => {
            let notifications = client
                .get_notifications(
                    &host,
                    &token,
                    &account.id,
                    TimelineOptions::new(limit, None, None),
                )
                .await?;
            if json {
                println!("{}", serde_json::to_string(&notifications).unwrap());
            } else {
                for n in &notifications {
                    print_notification(n);
                }
            }
        }
        Commands::Mentions { limit } => {
            let notes = client
                .get_mentions(&host, &token, &account.id, limit, None, None, None)
                .await?;
            print_notes(&notes, json);
        }
        Commands::Note { id } => {
            let note = client.get_note(&host, &token, &account.id, &id).await?;
            if json {
                println!("{}", serde_json::to_string(&note).unwrap());
            } else {
                print_note_detail(&note);
            }
        }
        Commands::Replies { id, limit } => {
            let notes = client
                .get_note_children(&host, &token, &account.id, &id, limit as u32)
                .await?;
            print_notes(&notes, json);
        }
        Commands::Thread { id, limit } => {
            let notes = client
                .get_note_conversation(&host, &token, &account.id, &id, limit as u32)
                .await?;
            print_notes(&notes, json);
        }
        Commands::Delete { id } => {
            client.delete_note(&host, &token, &id).await?;
            if json {
                println!(r#"{{"deleted":"{}"}}"#, id);
            } else {
                println!("Deleted: {}", id);
            }
        }
        Commands::Update { id, text, cw } => {
            let params = CreateNoteParams {
                text: Some(text),
                cw,
                visibility: None,
                local_only: None,
                mode_flags: None,
                reply_id: None,
                renote_id: None,
                file_ids: None,
            };
            client.update_note(&host, &token, &id, params).await?;
            if json {
                println!(r#"{{"updated":"{}"}}"#, id);
            } else {
                println!("Updated: {}", id);
            }
        }
        Commands::React { note_id, reaction } => {
            client
                .create_reaction(&host, &token, &note_id, &reaction)
                .await?;
            if json {
                println!(
                    r#"{{"reacted":"{}","reaction":"{}"}}"#,
                    note_id, reaction
                );
            } else {
                println!("Reacted {} to {}", reaction, note_id);
            }
        }
        Commands::Unreact { note_id } => {
            client.delete_reaction(&host, &token, &note_id).await?;
            if json {
                println!(r#"{{"unreacted":"{}"}}"#, note_id);
            } else {
                println!("Unreacted from {}", note_id);
            }
        }
        Commands::Renote { note_id } => {
            let params = CreateNoteParams {
                text: None,
                cw: None,
                visibility: Some("public".to_string()),
                local_only: None,
                mode_flags: None,
                reply_id: None,
                renote_id: Some(note_id.clone()),
                file_ids: None,
            };
            let note = client
                .create_note(&host, &token, &account.id, params)
                .await?;
            if json {
                println!("{}", serde_json::to_string(&note).unwrap());
            } else {
                println!("Renoted: https://{}/notes/{}", host, note.id);
            }
        }
        Commands::User { target } => {
            let detail = resolve_and_get_user(&client, &host, &token, &target).await?;
            if json {
                println!("{}", serde_json::to_string(&detail).unwrap());
            } else {
                print_user_detail(&detail);
            }
        }
        Commands::UserNotes { user_id, limit } => {
            let notes = client
                .get_user_notes(
                    &host,
                    &token,
                    &account.id,
                    &user_id,
                    TimelineOptions::new(limit, None, None),
                )
                .await?;
            print_notes(&notes, json);
        }
        Commands::Follow { user_id } => {
            client.follow_user(&host, &token, &user_id).await?;
            if json {
                println!(r#"{{"followed":"{}"}}"#, user_id);
            } else {
                println!("Followed: {}", user_id);
            }
        }
        Commands::Unfollow { user_id } => {
            client.unfollow_user(&host, &token, &user_id).await?;
            if json {
                println!(r#"{{"unfollowed":"{}"}}"#, user_id);
            } else {
                println!("Unfollowed: {}", user_id);
            }
        }
        Commands::Favorite { note_id } => {
            client.create_favorite(&host, &token, &note_id).await?;
            if json {
                println!(r#"{{"favorited":"{}"}}"#, note_id);
            } else {
                println!("Favorited: {}", note_id);
            }
        }
        Commands::Unfavorite { note_id } => {
            client.delete_favorite(&host, &token, &note_id).await?;
            if json {
                println!(r#"{{"unfavorited":"{}"}}"#, note_id);
            } else {
                println!("Unfavorited: {}", note_id);
            }
        }
        Commands::Favorites { limit } => {
            let notes = client
                .get_favorites(&host, &token, &account.id, limit, None, None)
                .await?;
            print_notes(&notes, json);
        }
        Commands::Emojis => {
            let emojis = client.get_server_emojis(&host, &token).await?;
            if json {
                println!("{}", serde_json::to_string(&emojis).unwrap());
            } else {
                print_emojis(&emojis);
            }
        }
        Commands::Accounts | Commands::Daemon { .. } | Commands::Login { .. } | Commands::Logout { .. } => {
            unreachable!()
        }
    }

    Ok(())
}

// --- Login / Logout ---

async fn run_login(db: &Database, host: &str, json: bool) -> Result<(), NoteDeckError> {
    let client = MisskeyClient::new()?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let permissions = [
        "read:account",
        "write:account",
        "read:blocks",
        "write:blocks",
        "read:drive",
        "write:drive",
        "read:favorites",
        "write:favorites",
        "read:following",
        "write:following",
        "read:messaging",
        "write:messaging",
        "read:mutes",
        "write:mutes",
        "read:notes",
        "write:notes",
        "read:notifications",
        "write:notifications",
        "read:reactions",
        "write:reactions",
        "write:votes",
    ];
    let permission_str = permissions.join(",");
    let auth_url = format!(
        "https://{host}/miauth/{session_id}?name=notecli&permission={permission_str}"
    );

    if json {
        println!(
            r#"{{"authUrl":"{}","sessionId":"{}","status":"waiting"}}"#,
            auth_url, session_id
        );
    } else {
        println!("以下のURLをブラウザで開いて認証してください:");
        println!();
        println!("  {}", auth_url);
        println!();
        println!("認証が完了したらEnterを押してください...");
    }

    // Wait for user to press Enter
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| NoteDeckError::InvalidInput(e.to_string()))?;

    let auth = client.complete_auth(host, &session_id).await?;

    let account_id = uuid::Uuid::new_v4().to_string();
    let account = Account {
        id: account_id.clone(),
        host: host.to_string(),
        token: auth.token.clone(),
        user_id: auth.user.id.clone(),
        username: auth.user.username.clone(),
        display_name: auth.user.name.clone(),
        avatar_url: auth.user.avatar_url.clone(),
        software: "misskey".to_string(),
    };

    db.upsert_account(&account)?;

    // Try to store token in keychain
    if notecli::keychain::store_token(&account_id, &auth.token).is_ok() {
        let _ = db.clear_token(&account_id);
    }

    if json {
        let public = AccountPublic::from(&account);
        println!("{}", serde_json::to_string(&public).unwrap());
    } else {
        println!(
            "Login successful: @{}@{}",
            auth.user.username, host
        );
    }

    Ok(())
}

fn run_logout(db: &Database, target: &str, json: bool) -> Result<(), NoteDeckError> {
    let account = resolve_account(db, Some(target))?;
    let username = account.username.clone();
    let host = account.host.clone();
    let id = account.id.clone();

    // Remove from keychain
    let _ = notecli::keychain::delete_token(&id);

    db.delete_account(&id)?;

    if json {
        println!(r#"{{"loggedOut":"{}","username":"{}","host":"{}"}}"#, id, username, host);
    } else {
        println!("Logged out: @{}@{}", username, host);
    }

    Ok(())
}

// --- User resolution ---

fn resolve_account(db: &Database, spec: Option<&str>) -> Result<Account, NoteDeckError> {
    let accounts = db.load_accounts()?;
    if accounts.is_empty() {
        return Err(NoteDeckError::AccountNotFound(
            "no accounts found. Use 'notecli login <HOST>' to add one".to_string(),
        ));
    }

    let Some(spec) = spec else {
        return Ok(accounts.into_iter().next().unwrap());
    };

    // @user@host format
    if let Some(rest) = spec.strip_prefix('@') {
        if let Some((user, host)) = rest.split_once('@') {
            return accounts
                .into_iter()
                .find(|a| a.username.eq_ignore_ascii_case(user) && a.host.contains(host))
                .ok_or_else(|| NoteDeckError::AccountNotFound(spec.to_string()));
        }
    }

    // Try as account ID
    if let Some(account) = db.get_account(spec)? {
        return Ok(account);
    }

    // Try as username (partial match)
    accounts
        .into_iter()
        .find(|a| a.username.eq_ignore_ascii_case(spec))
        .ok_or_else(|| NoteDeckError::AccountNotFound(spec.to_string()))
}

async fn resolve_and_get_user(
    client: &MisskeyClient,
    host: &str,
    token: &str,
    target: &str,
) -> Result<NormalizedUserDetail, NoteDeckError> {
    // @user@host or @user format
    if let Some(rest) = target.strip_prefix('@') {
        let (username, user_host) = if let Some((u, h)) = rest.split_once('@') {
            (u, Some(h))
        } else {
            (rest, None)
        };
        let user = client.lookup_user(host, token, username, user_host).await?;
        return client.get_user_detail(host, token, &user.id).await;
    }

    // Try as user ID
    client.get_user_detail(host, token, target).await
}

// --- Output formatting ---

fn print_notes(notes: &[NormalizedNote], json: bool) {
    if json {
        println!("{}", serde_json::to_string(notes).unwrap());
        return;
    }
    for note in notes {
        print_note_detail(note);
    }
}

fn print_note_detail(note: &NormalizedNote) {
    let user = format_user(&note.user);
    let time = &note.created_at[..16].replace('T', " ");
    println!("{user}  {time}  id:{}", note.id);
    if let Some(ref cw) = note.cw {
        println!("[CW: {cw}]");
    }
    if let Some(ref text) = note.text {
        println!("{text}");
    }
    if let Some(ref renote) = note.renote {
        let ru = format_user(&renote.user);
        println!("  RN {ru}: {}", renote.text.as_deref().unwrap_or(""));
    }
    let reactions: i64 = note.reactions.values().sum();
    if reactions > 0 || note.renote_count > 0 || note.replies_count > 0 {
        println!(
            "  reactions:{} renotes:{} replies:{}",
            reactions, note.renote_count, note.replies_count
        );
    }
    println!();
}

fn print_notification(n: &NormalizedNotification) {
    let user = n
        .user
        .as_ref()
        .map(|u| format_user(u))
        .unwrap_or_default();
    let note_preview = n
        .note
        .as_ref()
        .and_then(|n| n.text.as_deref())
        .unwrap_or("");
    let preview = if note_preview.len() > 50 {
        format!("{}...", &note_preview[..50])
    } else {
        note_preview.to_string()
    };

    match n.notification_type.as_str() {
        "reaction" => {
            let reaction = n.reaction.as_deref().unwrap_or("?");
            println!("reaction  {user}  {reaction}  \"{preview}\"");
        }
        "follow" => println!("follow    {user}"),
        "reply" => println!("reply     {user}  \"{preview}\""),
        "renote" => println!("renote    {user}  \"{preview}\""),
        "mention" => println!("mention   {user}  \"{preview}\""),
        "quote" => println!("quote     {user}  \"{preview}\""),
        other => println!("{:<10}{user}  \"{preview}\"", other),
    }
}

fn print_user_detail(u: &NormalizedUserDetail) {
    let host_str = u.host.as_deref().unwrap_or("(local)");
    let name = u.name.as_deref().unwrap_or(&u.username);
    println!("{} (@{}@{})", name, u.username, host_str);
    println!("  ID: {}", u.id);
    if let Some(ref desc) = u.description {
        println!("  Bio: {}", desc);
    }
    println!(
        "  Notes: {}  Following: {}  Followers: {}",
        u.notes_count, u.following_count, u.followers_count
    );
    if u.is_bot {
        print!("  [Bot]");
    }
    if u.is_cat {
        print!("  [Cat]");
    }
    if u.is_following {
        print!("  [Following]");
    }
    if u.is_followed {
        print!("  [Followed by]");
    }
    if u.is_bot || u.is_cat || u.is_following || u.is_followed {
        println!();
    }
    println!();
}

fn print_emojis(emojis: &[ServerEmoji]) {
    let mut by_category: std::collections::BTreeMap<&str, Vec<&ServerEmoji>> =
        std::collections::BTreeMap::new();
    for emoji in emojis {
        let cat = emoji.category.as_deref().unwrap_or("(uncategorized)");
        by_category.entry(cat).or_default().push(emoji);
    }
    for (category, list) in &by_category {
        println!("[{}]", category);
        for emoji in list {
            let aliases = if emoji.aliases.is_empty() {
                String::new()
            } else {
                format!(" ({})", emoji.aliases.join(", "))
            };
            println!("  :{}: {}", emoji.name, aliases);
        }
    }
    println!("\nTotal: {} emojis", emojis.len());
}

fn format_user(user: &notecli::models::NormalizedUser) -> String {
    match &user.host {
        Some(host) => format!("@{}@{}", user.username, host),
        None => format!("@{}", user.username),
    }
}

fn dirs_data_dir() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(".local/share")
    })
}
