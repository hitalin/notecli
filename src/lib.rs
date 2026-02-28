pub mod api;
pub mod db;
pub mod error;
pub mod event_bus;
pub mod http_server;
pub mod keychain;
pub mod models;
pub mod streaming;

use db::Database;
use error::NoteDeckError;
use zeroize::Zeroize;

/// Retrieve host and API token for an account.
/// Tries keychain first, falls back to DB token with lazy migration.
pub fn get_credentials(db: &Database, account_id: &str) -> Result<(String, String), NoteDeckError> {
    let account = db
        .get_account(account_id)?
        .ok_or_else(|| NoteDeckError::AccountNotFound(account_id.to_string()))?;
    let host = account.host.clone();

    // Try keychain first (ignore errors — keychain may be unavailable)
    if let Some(token) = keychain::get_token(account_id).ok().flatten() {
        if !account.token.is_empty() {
            let _ = db.clear_token(account_id);
        }
        return Ok((host, token));
    }

    // Fallback: use DB token
    let mut db_token = account.token.clone();
    if !db_token.is_empty() {
        // Try lazy migration to keychain; verify before clearing DB
        if keychain::store_token(account_id, &db_token).is_ok()
            && keychain::get_token(account_id)
                .ok()
                .flatten()
                .is_some()
        {
            let _ = db.clear_token(account_id);
        }
        let token = db_token.clone();
        db_token.zeroize();
        return Ok((host, token));
    }

    Err(NoteDeckError::Auth(format!(
        "No token found for account {account_id}"
    )))
}
