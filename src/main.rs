use std::sync::Arc;

use notecli::api::MisskeyClient;
use notecli::db::Database;
use notecli::event_bus::EventBus;
use notecli::streaming::{NoopEmitter, StreamingManager};

#[tokio::main]
async fn main() {
    let data_dir = dirs_data_dir().join("notecli");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let db_path = data_dir.join("notecli.db");
    let db = Arc::new(
        Database::open(&db_path).expect("Failed to open database"),
    );

    let client = Arc::new(
        MisskeyClient::new().expect("Failed to create HTTP client"),
    );

    let event_bus = Arc::new(EventBus::new());

    let emitter = Arc::new(NoopEmitter);
    let _streaming = StreamingManager::new(emitter, event_bus.clone(), db.clone());

    // Generate API token
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

    notecli::http_server::start(db, client, event_bus, api_token, token_path_str).await;
}

fn dirs_data_dir() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(".local/share")
    })
}
