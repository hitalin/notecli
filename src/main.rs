use std::sync::Arc;

use clap::Parser;
use notecli::api::MisskeyClient;
use notecli::cli::{Cli, Commands};
use notecli::db::Database;
use notecli::error::NoteDeckError;
use notecli::event_bus::EventBus;
use notecli::models::{
    Account, AccountPublic, CreateNoteParams, NormalizedNote, NormalizedNotification,
    NormalizedUserDetail, SearchOptions, ServerEmoji, TimelineOptions, TimelineType,
};
use notecli::streaming::{EventBusEmitter, StreamingManager};

// --- Output format ---

#[derive(Debug, Clone, Copy, PartialEq)]
enum OutputFormat {
    Default,
    Json,
    Ids,
    Compact,
    Jsonl,
}

fn output_format(cli: &Cli) -> OutputFormat {
    if cli.json {
        OutputFormat::Json
    } else if cli.ids {
        OutputFormat::Ids
    } else if cli.compact {
        OutputFormat::Compact
    } else if cli.jsonl {
        OutputFormat::Jsonl
    } else {
        OutputFormat::Default
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    if let Err(e) = notecli::keychain::init_store() {
        tracing::warn!(error = %e, "keychain unavailable");
    }

    let cli = Cli::parse();

    match cli.command {
        None | Some(Commands::Daemon { .. }) => {
            let port = match &cli.command {
                Some(Commands::Daemon { port }) => *port,
                _ => 19820,
            };
            run_daemon(port).await;
        }
        Some(ref cmd) => {
            let fmt = output_format(&cli);
            if let Err(e) = run_cli(cmd, cli.account.as_deref(), fmt).await {
                match fmt {
                    OutputFormat::Json | OutputFormat::Jsonl => {
                        let err =
                            serde_json::json!({ "error": e.code(), "message": e.safe_message() });
                        eprintln!("{err}");
                    }
                    _ => {
                        eprintln!("Error: {}", e.safe_message());
                    }
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

    tracing::info!(data_dir = %data_dir.display(), token_path = %token_path_str, port, "daemon starting");

    notecli::http_server::start_on_port(db, client, event_bus, api_token, token_path_str, port)
        .await;
}

async fn run_cli(
    cmd: &Commands,
    account_spec: Option<&str>,
    fmt: OutputFormat,
) -> Result<(), NoteDeckError> {
    let data_dir = dirs_data_dir().join("notecli");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let db_path = data_dir.join("notecli.db");
    let db = Database::open(&db_path)?;

    // Commands that don't need credentials
    match &cmd {
        Commands::Accounts => {
            let accounts = db.load_accounts()?;
            match fmt {
                OutputFormat::Json => {
                    let public: Vec<AccountPublic> =
                        accounts.iter().map(AccountPublic::from).collect();
                    println!("{}", serde_json::to_string(&public).unwrap());
                }
                OutputFormat::Jsonl => {
                    for a in &accounts {
                        let public = AccountPublic::from(a);
                        println!("{}", serde_json::to_string(&public).unwrap());
                    }
                }
                OutputFormat::Ids => {
                    for a in &accounts {
                        println!("{}", a.id);
                    }
                }
                OutputFormat::Compact => {
                    for a in &accounts {
                        let name = a.display_name.as_deref().unwrap_or(&a.username);
                        println!("{}\t@{}@{}\t{}", a.id, a.username, a.host, name);
                    }
                }
                OutputFormat::Default => {
                    if accounts.is_empty() {
                        println!("No accounts found. Use 'notecli login <HOST>' to add one.");
                    } else {
                        for a in &accounts {
                            let name = a.display_name.as_deref().unwrap_or(&a.username);
                            println!("@{}@{} ({}) {}", a.username, a.host, a.id, name);
                        }
                    }
                }
            }
            return Ok(());
        }
        Commands::Login { host } => {
            return run_login(&db, host, fmt).await;
        }
        Commands::Logout { target } => {
            return run_logout(&db, target, fmt);
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
                text: Some(text.clone()),
                cw: cw.clone(),
                visibility: Some(visibility.clone()),
                local_only: if *local_only { Some(true) } else { None },
                mode_flags: None,
                reply_id: reply_to.clone(),
                renote_id: None,
                file_ids: None,
                poll: None,
                scheduled_at: None,
            };
            let note = client
                .create_note(&host, &token, &account.id, params)
                .await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!("{}", serde_json::to_string(&note).unwrap());
                }
                OutputFormat::Ids => println!("{}", note.id),
                OutputFormat::Compact => print_note_compact(&note),
                OutputFormat::Default => {
                    println!("Posted: https://{}/notes/{}", host, note.id);
                }
            }
        }
        Commands::Tl { r#type, limit } => {
            let notes = client
                .get_timeline(
                    &host,
                    &token,
                    &account.id,
                    TimelineType::new(r#type),
                    TimelineOptions::new(*limit, None, None),
                )
                .await?;
            print_notes(&notes, fmt);
        }
        Commands::Search { query, limit } => {
            let notes = client
                .search_notes(
                    &host,
                    &token,
                    &account.id,
                    query,
                    SearchOptions::new(*limit),
                )
                .await?;
            print_notes(&notes, fmt);
        }
        Commands::Notifications { limit } => {
            let notifications = client
                .get_notifications(
                    &host,
                    &token,
                    &account.id,
                    TimelineOptions::new(*limit, None, None),
                )
                .await?;
            print_notifications(&notifications, fmt);
        }
        Commands::Mentions { limit } => {
            let notes = client
                .get_mentions(&host, &token, &account.id, *limit, None, None, None)
                .await?;
            print_notes(&notes, fmt);
        }
        Commands::Note { id } => {
            let note = client.get_note(&host, &token, &account.id, id).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!("{}", serde_json::to_string(&note).unwrap());
                }
                OutputFormat::Ids => println!("{}", note.id),
                OutputFormat::Compact => print_note_compact(&note),
                OutputFormat::Default => print_note_detail(&note),
            }
        }
        Commands::Replies { id, limit } => {
            let notes = client
                .get_note_children(&host, &token, &account.id, id, *limit as u32)
                .await?;
            print_notes(&notes, fmt);
        }
        Commands::Thread { id, limit } => {
            let notes = client
                .get_note_conversation(&host, &token, &account.id, id, *limit as u32)
                .await?;
            print_notes(&notes, fmt);
        }
        Commands::Delete { id } => {
            client.delete_note(&host, &token, id).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!(r#"{{"deleted":"{}"}}"#, id);
                }
                OutputFormat::Ids => println!("{}", id),
                _ => println!("Deleted: {}", id),
            }
        }
        Commands::Update { id, text, cw } => {
            let params = CreateNoteParams {
                text: Some(text.clone()),
                cw: cw.clone(),
                visibility: None,
                local_only: None,
                mode_flags: None,
                reply_id: None,
                renote_id: None,
                file_ids: None,
                poll: None,
                scheduled_at: None,
            };
            client.update_note(&host, &token, id, params).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!(r#"{{"updated":"{}"}}"#, id);
                }
                OutputFormat::Ids => println!("{}", id),
                _ => println!("Updated: {}", id),
            }
        }
        Commands::React { note_id, reaction } => {
            client
                .create_reaction(&host, &token, note_id, reaction)
                .await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!(
                        r#"{{"reacted":"{}","reaction":"{}"}}"#,
                        note_id, reaction
                    );
                }
                OutputFormat::Ids => println!("{}", note_id),
                _ => println!("Reacted {} to {}", reaction, note_id),
            }
        }
        Commands::Unreact { note_id } => {
            client.delete_reaction(&host, &token, note_id).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!(r#"{{"unreacted":"{}"}}"#, note_id);
                }
                OutputFormat::Ids => println!("{}", note_id),
                _ => println!("Unreacted from {}", note_id),
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
                poll: None,
                scheduled_at: None,
            };
            let note = client
                .create_note(&host, &token, &account.id, params)
                .await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!("{}", serde_json::to_string(&note).unwrap());
                }
                OutputFormat::Ids => println!("{}", note.id),
                OutputFormat::Compact => print_note_compact(&note),
                OutputFormat::Default => {
                    println!("Renoted: https://{}/notes/{}", host, note.id);
                }
            }
        }
        Commands::User { target } => {
            let detail = resolve_and_get_user(&client, &host, &token, target).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!("{}", serde_json::to_string(&detail).unwrap());
                }
                OutputFormat::Ids => println!("{}", detail.id),
                OutputFormat::Compact => {
                    let host_str = detail.host.as_deref().unwrap_or("(local)");
                    let name = detail.name.as_deref().unwrap_or(&detail.username);
                    println!(
                        "{}\t@{}@{}\t{}\tnotes:{}\tfollowing:{}\tfollowers:{}",
                        detail.id,
                        detail.username,
                        host_str,
                        oneline(name),
                        detail.notes_count,
                        detail.following_count,
                        detail.followers_count
                    );
                }
                OutputFormat::Default => print_user_detail(&detail),
            }
        }
        Commands::UserNotes { user_id, limit } => {
            let notes = client
                .get_user_notes(
                    &host,
                    &token,
                    &account.id,
                    user_id,
                    TimelineOptions::new(*limit, None, None),
                )
                .await?;
            print_notes(&notes, fmt);
        }
        Commands::Follow { user_id } => {
            client.follow_user(&host, &token, user_id).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!(r#"{{"followed":"{}"}}"#, user_id);
                }
                OutputFormat::Ids => println!("{}", user_id),
                _ => println!("Followed: {}", user_id),
            }
        }
        Commands::Unfollow { user_id } => {
            client.unfollow_user(&host, &token, user_id).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!(r#"{{"unfollowed":"{}"}}"#, user_id);
                }
                OutputFormat::Ids => println!("{}", user_id),
                _ => println!("Unfollowed: {}", user_id),
            }
        }
        Commands::Favorite { note_id } => {
            client.create_favorite(&host, &token, note_id).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!(r#"{{"favorited":"{}"}}"#, note_id);
                }
                OutputFormat::Ids => println!("{}", note_id),
                _ => println!("Favorited: {}", note_id),
            }
        }
        Commands::Unfavorite { note_id } => {
            client.delete_favorite(&host, &token, note_id).await?;
            match fmt {
                OutputFormat::Json | OutputFormat::Jsonl => {
                    println!(r#"{{"unfavorited":"{}"}}"#, note_id);
                }
                OutputFormat::Ids => println!("{}", note_id),
                _ => println!("Unfavorited: {}", note_id),
            }
        }
        Commands::Favorites { limit } => {
            let notes = client
                .get_favorites(&host, &token, &account.id, *limit, None, None)
                .await?;
            print_notes(&notes, fmt);
        }
        Commands::Emojis => {
            let emojis = client.get_server_emojis(&host, &token).await?;
            match fmt {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string(&emojis).unwrap());
                }
                OutputFormat::Jsonl => {
                    for e in &emojis {
                        println!("{}", serde_json::to_string(e).unwrap());
                    }
                }
                OutputFormat::Ids => {
                    for e in &emojis {
                        println!(":{}:", e.name);
                    }
                }
                OutputFormat::Compact => {
                    for e in &emojis {
                        let cat = e.category.as_deref().unwrap_or("");
                        println!(":{}:\t{}\t{}", e.name, cat, e.aliases.join(","));
                    }
                }
                OutputFormat::Default => print_emojis(&emojis),
            }
        }
        Commands::Accounts
        | Commands::Daemon { .. }
        | Commands::Login { .. }
        | Commands::Logout { .. } => {
            unreachable!()
        }
    }

    Ok(())
}

// --- Login / Logout ---

async fn run_login(db: &Database, host: &str, fmt: OutputFormat) -> Result<(), NoteDeckError> {
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

    match fmt {
        OutputFormat::Json | OutputFormat::Jsonl => {
            println!(
                r#"{{"authUrl":"{}","sessionId":"{}","status":"waiting"}}"#,
                auth_url, session_id
            );
        }
        _ => {
            println!("以下のURLをブラウザで開いて認証してください:");
            println!();
            println!("  {}", auth_url);
            println!();
            println!("認証が完了したらEnterを押してください...");
        }
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

    match fmt {
        OutputFormat::Json | OutputFormat::Jsonl => {
            let public = AccountPublic::from(&account);
            println!("{}", serde_json::to_string(&public).unwrap());
        }
        OutputFormat::Ids => println!("{}", account_id),
        OutputFormat::Compact => {
            println!(
                "{}\t@{}@{}\t{}",
                account_id,
                auth.user.username,
                host,
                auth.user.name.as_deref().unwrap_or(&auth.user.username)
            );
        }
        OutputFormat::Default => {
            println!("Login successful: @{}@{}", auth.user.username, host);
        }
    }

    Ok(())
}

fn run_logout(db: &Database, target: &str, fmt: OutputFormat) -> Result<(), NoteDeckError> {
    let account = resolve_account(db, Some(target))?;
    let username = account.username.clone();
    let host = account.host.clone();
    let id = account.id.clone();

    // Remove from keychain
    let _ = notecli::keychain::delete_token(&id);

    db.delete_account(&id)?;

    match fmt {
        OutputFormat::Json | OutputFormat::Jsonl => {
            println!(
                r#"{{"loggedOut":"{}","username":"{}","host":"{}"}}"#,
                id, username, host
            );
        }
        OutputFormat::Ids => println!("{}", id),
        _ => println!("Logged out: @{}@{}", username, host),
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

/// Collapse whitespace (newlines, tabs, spaces) into single spaces.
fn oneline(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build a one-line text representation of a note for compact output.
fn compact_note_text(note: &NormalizedNote) -> String {
    let mut parts = Vec::new();
    if let Some(ref cw) = note.cw {
        parts.push(format!("[CW: {}]", oneline(cw)));
    }
    if let Some(ref text) = note.text {
        parts.push(oneline(text));
    } else if let Some(ref renote) = note.renote {
        let ru = format_user(&renote.user);
        let rt = renote.text.as_deref().unwrap_or("");
        parts.push(format!("[RN {}] {}", ru, oneline(rt)));
    }
    if parts.is_empty() {
        "(empty)".to_string()
    } else {
        parts.join(" ")
    }
}

fn print_notes(notes: &[NormalizedNote], fmt: OutputFormat) {
    match fmt {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string(notes).unwrap());
        }
        OutputFormat::Jsonl => {
            for note in notes {
                println!("{}", serde_json::to_string(note).unwrap());
            }
        }
        OutputFormat::Ids => {
            for note in notes {
                println!("{}", note.id);
            }
        }
        OutputFormat::Compact => {
            for note in notes {
                print_note_compact(note);
            }
        }
        OutputFormat::Default => {
            for note in notes {
                print_note_detail(note);
            }
        }
    }
}

fn print_note_compact(note: &NormalizedNote) {
    let user = format_user(&note.user);
    let time = &note.created_at[..16].replace('T', " ");
    let text = compact_note_text(note);
    println!("{}\t{}\t{}\t{}", note.id, user, time, text);
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

fn print_notifications(notifications: &[NormalizedNotification], fmt: OutputFormat) {
    match fmt {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string(notifications).unwrap());
        }
        OutputFormat::Jsonl => {
            for n in notifications {
                println!("{}", serde_json::to_string(n).unwrap());
            }
        }
        OutputFormat::Ids => {
            for n in notifications {
                if let Some(ref note) = n.note {
                    println!("{}", note.id);
                } else {
                    println!("{}", n.id);
                }
            }
        }
        OutputFormat::Compact => {
            for n in notifications {
                print_notification_compact(n);
            }
        }
        OutputFormat::Default => {
            for n in notifications {
                print_notification(n);
            }
        }
    }
}

fn print_notification_compact(n: &NormalizedNotification) {
    let user = n
        .user
        .as_ref()
        .map(format_user)
        .unwrap_or_default();
    let note_id = n
        .note
        .as_ref()
        .map(|n| n.id.as_str())
        .unwrap_or("-");
    let reaction = n.reaction.as_deref().unwrap_or("");
    let preview = n
        .note
        .as_ref()
        .and_then(|n| n.text.as_deref())
        .map(oneline)
        .unwrap_or_default();
    println!(
        "{}\t{}\t{}\t{}\t{}",
        note_id, n.notification_type, user, reaction, preview
    );
}

fn print_notification(n: &NormalizedNotification) {
    let user = n
        .user
        .as_ref()
        .map(format_user)
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
