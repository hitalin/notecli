use thiserror::Error;

#[derive(Debug, Error)]
pub enum NoteDeckError {
    #[error("Database error")]
    Database(#[from] rusqlite::Error),

    #[error("Network error")]
    Network(#[from] reqwest::Error),

    #[error("JSON parse error")]
    Json(#[from] serde_json::Error),

    #[error("Account not found: {0}")]
    AccountNotFound(String),

    #[error("{message}")]
    Api {
        endpoint: String,
        status: u16,
        message: String,
    },

    #[error("{0}")]
    Auth(String),

    #[error("WebSocket: {0}")]
    WebSocket(String),

    #[error("No connection for account: {0}")]
    NoConnection(String),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Keychain error: {0}")]
    Keychain(String),
}

impl NoteDeckError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Database(_) => "DATABASE",
            Self::Network(_) => "NETWORK",
            Self::Json(_) => "JSON",
            Self::AccountNotFound(_) => "ACCOUNT_NOT_FOUND",
            Self::Api { .. } => "API",
            Self::Auth(_) => "AUTH",
            Self::WebSocket(_) => "WEBSOCKET",
            Self::NoConnection(_) => "NO_CONNECTION",
            Self::ConnectionClosed => "CONNECTION_CLOSED",
            Self::InvalidInput(_) => "INVALID_INPUT",
            Self::Keychain(_) => "KEYCHAIN",
        }
    }
}

impl NoteDeckError {
    /// Returns a sanitized message safe for the frontend.
    /// Internal details (DB queries, network traces, keychain internals) are
    /// logged to stderr and replaced with generic messages.
    pub fn safe_message(&self) -> String {
        match self {
            Self::Database(e) => {
                eprintln!("[error] Database: {e}");
                "Database operation failed".to_string()
            }
            Self::Network(e) => {
                eprintln!("[error] Network: {e}");
                "Network request failed".to_string()
            }
            Self::Json(e) => {
                eprintln!("[error] JSON: {e}");
                "Invalid response format".to_string()
            }
            Self::WebSocket(e) => {
                eprintln!("[error] WebSocket: {e}");
                "Connection error".to_string()
            }
            Self::Keychain(e) => {
                eprintln!("[error] Keychain: {e}");
                "Credential storage error".to_string()
            }
            // These contain messages we control — safe to expose
            Self::Api { message, .. } => message.clone(),
            Self::Auth(msg) => msg.clone(),
            Self::AccountNotFound(id) => format!("Account not found: {id}"),
            Self::NoConnection(id) => format!("No connection for account: {id}"),
            Self::ConnectionClosed => "Connection closed".to_string(),
            Self::InvalidInput(msg) => format!("Invalid input: {msg}"),
        }
    }
}

impl serde::Serialize for NoteDeckError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("NoteDeckError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.safe_message())?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_mapping() {
        assert_eq!(
            NoteDeckError::AccountNotFound("x".into()).code(),
            "ACCOUNT_NOT_FOUND"
        );
        assert_eq!(
            NoteDeckError::Api {
                endpoint: "test".into(),
                status: 400,
                message: "bad".into()
            }
            .code(),
            "API"
        );
        assert_eq!(NoteDeckError::Auth("x".into()).code(), "AUTH");
        assert_eq!(NoteDeckError::WebSocket("x".into()).code(), "WEBSOCKET");
        assert_eq!(
            NoteDeckError::NoConnection("x".into()).code(),
            "NO_CONNECTION"
        );
        assert_eq!(NoteDeckError::ConnectionClosed.code(), "CONNECTION_CLOSED");
        assert_eq!(
            NoteDeckError::InvalidInput("x".into()).code(),
            "INVALID_INPUT"
        );
        assert_eq!(NoteDeckError::Keychain("x".into()).code(), "KEYCHAIN");
    }

    #[test]
    fn safe_message_sanitizes_internals() {
        // These variants should NOT leak internal details
        let db_err = NoteDeckError::Database(
            rusqlite::Connection::open_in_memory()
                .unwrap()
                .execute("INVALID SQL", [])
                .unwrap_err(),
        );
        assert_eq!(db_err.safe_message(), "Database operation failed");

        let ws_err = NoteDeckError::WebSocket("tungstenite internal detail".into());
        assert_eq!(ws_err.safe_message(), "Connection error");

        let kc_err = NoteDeckError::Keychain("keyring internal detail".into());
        assert_eq!(kc_err.safe_message(), "Credential storage error");
    }

    #[test]
    fn safe_message_passes_controlled_messages() {
        let api_err = NoteDeckError::Api {
            endpoint: "/api/test".into(),
            status: 404,
            message: "Note not found".into(),
        };
        assert_eq!(api_err.safe_message(), "Note not found");

        let auth_err = NoteDeckError::Auth("Authentication failed".into());
        assert_eq!(auth_err.safe_message(), "Authentication failed");

        let not_found = NoteDeckError::AccountNotFound("acc123".into());
        assert_eq!(not_found.safe_message(), "Account not found: acc123");

        let no_conn = NoteDeckError::NoConnection("acc456".into());
        assert_eq!(no_conn.safe_message(), "No connection for account: acc456");

        assert_eq!(
            NoteDeckError::ConnectionClosed.safe_message(),
            "Connection closed"
        );

        let invalid = NoteDeckError::InvalidInput("empty text".into());
        assert_eq!(invalid.safe_message(), "Invalid input: empty text");
    }

    #[test]
    fn serialize_to_json() {
        let err = NoteDeckError::Api {
            endpoint: "/api/notes/show".into(),
            status: 404,
            message: "Note not found".into(),
        };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "API");
        assert_eq!(json["message"], "Note not found");
        // endpoint and status should NOT appear in serialized output
        assert!(json.get("endpoint").is_none());
        assert!(json.get("status").is_none());
    }

    #[test]
    fn display_trait() {
        let err = NoteDeckError::AccountNotFound("acc1".into());
        assert_eq!(format!("{err}"), "Account not found: acc1");

        let err = NoteDeckError::ConnectionClosed;
        assert_eq!(format!("{err}"), "Connection closed");

        let err = NoteDeckError::Api {
            endpoint: "test".into(),
            status: 500,
            message: "Internal error".into(),
        };
        assert_eq!(format!("{err}"), "Internal error");
    }
}
