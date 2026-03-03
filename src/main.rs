use std::sync::Arc;

use clap::{Parser, Subcommand};
use notecli::api::MisskeyClient;
use notecli::db::Database;
use notecli::error::NoteDeckError;
use notecli::event_bus::EventBus;
use notecli::models::{
    Account, AccountPublic, CreateNoteParams, NormalizedNote, NormalizedNotification,
    SearchOptions, TimelineOptions, TimelineType,
};
use notecli::streaming::{EventBusEmitter, StreamingManager};

#[derive(Parser)]
#[command(name = "notecli", about = "Headless Misskey client")]
struct Cli {
    /// Account ID or @user@host
    #[arg(long, short = 'a', global = true)]
    account: Option<String>,

    /// Output as JSON (for AI agents)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start HTTP API server
    Daemon {
        /// Port number
        #[arg(long, default_value_t = 19820)]
        port: u16,
    },
    /// List accounts
    Accounts,
    /// Post a note
    Post {
        /// Note text
        text: String,
        /// Content Warning
        #[arg(long)]
        cw: Option<String>,
        /// Visibility: public, home, followers, specified
        #[arg(long, default_value = "public")]
        visibility: String,
        /// Reply to note ID
        #[arg(long)]
        reply_to: Option<String>,
        /// Local only
        #[arg(long)]
        local_only: bool,
    },
    /// Get timeline
    Tl {
        /// Timeline type: home, local, social, global
        #[arg(default_value = "home")]
        r#type: String,
        /// Number of notes
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },
    /// Search notes
    Search {
        /// Search query
        query: String,
        /// Number of notes
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },
    /// Get notifications
    Notifications {
        /// Number of notifications
        #[arg(long, short, default_value_t = 20)]
        limit: i64,
    },
    /// Show a note
    Note {
        /// Note ID
        id: String,
    },
    /// Delete a note
    Delete {
        /// Note ID
        id: String,
    },
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

    // accounts doesn't need credentials
    if matches!(cmd, Commands::Accounts) {
        let accounts = db.load_accounts()?;
        if json {
            let public: Vec<AccountPublic> = accounts.iter().map(AccountPublic::from).collect();
            println!("{}", serde_json::to_string(&public).unwrap());
        } else {
            if accounts.is_empty() {
                println!("No accounts found.");
            } else {
                for a in &accounts {
                    let name = a.display_name.as_deref().unwrap_or(&a.username);
                    println!("@{}@{} ({}) {}", a.username, a.host, a.id, name);
                }
            }
        }
        return Ok(());
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
        Commands::Note { id } => {
            let note = client.get_note(&host, &token, &account.id, &id).await?;
            if json {
                println!("{}", serde_json::to_string(&note).unwrap());
            } else {
                print_note_detail(&note);
            }
        }
        Commands::Delete { id } => {
            client.delete_note(&host, &token, &id).await?;
            if json {
                println!(r#"{{"deleted":"{}"}}"#, id);
            } else {
                println!("Deleted: {}", id);
            }
        }
        Commands::Accounts | Commands::Daemon { .. } => unreachable!(),
    }

    Ok(())
}

fn resolve_account(db: &Database, spec: Option<&str>) -> Result<Account, NoteDeckError> {
    let accounts = db.load_accounts()?;
    if accounts.is_empty() {
        return Err(NoteDeckError::AccountNotFound("no accounts".to_string()));
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
    println!("{user}  {time}");
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
