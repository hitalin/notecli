use crate::api::MisskeyClient;
use crate::db::Database;
use crate::error::NoteDeckError;
use crate::format::{print_action, OutputFormat};
use crate::models::{Account, AccountPublic};

pub fn run_accounts(db: &Database, fmt: OutputFormat) -> Result<(), NoteDeckError> {
    let accounts = db.load_accounts()?;
    match fmt {
        OutputFormat::Json => {
            let public: Vec<AccountPublic> = accounts.iter().map(AccountPublic::from).collect();
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
    Ok(())
}

pub async fn run_login(
    db: &Database,
    host: &str,
    fmt: OutputFormat,
) -> Result<(), NoteDeckError> {
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

    if crate::keychain::store_token(&account_id, &auth.token).is_ok() {
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

pub fn run_logout(
    db: &Database,
    account: &Account,
    fmt: OutputFormat,
) -> Result<(), NoteDeckError> {
    let username = account.username.clone();
    let host = account.host.clone();
    let id = account.id.clone();

    let _ = crate::keychain::delete_token(&id);
    db.delete_account(&id)?;

    print_action(
        fmt,
        &format!(r#"{{"loggedOut":"{id}","username":"{username}","host":"{host}"}}"#),
        &id,
        &format!("Logged out: @{username}@{host}"),
    );

    Ok(())
}
