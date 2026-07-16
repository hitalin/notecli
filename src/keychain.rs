use crate::error::NoteDeckError;

#[cfg(feature = "keyring")]
const SERVICE: &str = "notedeck";

/// Initialize the platform-specific credential store.
/// Must be called once before any keychain operations.
#[cfg(feature = "keyring")]
pub fn init_store() -> Result<(), NoteDeckError> {
    #[cfg(target_os = "android")]
    {
        let store = android_native_keyring_store::by_store::Store::new()
            .map_err(|e| NoteDeckError::Keychain(e.to_string()))?;
        keyring_core::set_default_store(store);
    }
    #[cfg(target_os = "macos")]
    {
        let store = apple_native_keyring_store::keychain::Store::new()
            .map_err(|e| NoteDeckError::Keychain(e.to_string()))?;
        keyring_core::set_default_store(store);
    }
    #[cfg(target_os = "ios")]
    {
        let store = apple_native_keyring_store::protected::Store::new()
            .map_err(|e| NoteDeckError::Keychain(e.to_string()))?;
        keyring_core::set_default_store(store);
    }
    #[cfg(target_os = "windows")]
    {
        let store = windows_native_keyring_store::Store::new()
            .map_err(|e| NoteDeckError::Keychain(e.to_string()))?;
        keyring_core::set_default_store(store);
    }
    #[cfg(target_os = "linux")]
    {
        let store = linux_keyutils_keyring_store::Store::new()
            .map_err(|e| NoteDeckError::Keychain(e.to_string()))?;
        keyring_core::set_default_store(store);
    }
    Ok(())
}

/// 現在の credential store が再起動をまたいで永続するかどうか。
///
/// Linux の keyutils store はカーネルメモリ常駐（`UntilReboot`）のため false。
/// false の場合、呼び出し側は DB 等の永続フォールバックを消してはならず、
/// keychain は高速キャッシュとして扱う（notedeck#785）。
#[cfg(feature = "keyring")]
pub fn is_persistent() -> bool {
    match keyring_core::get_default_store() {
        Some(store) => matches!(
            store.persistence(),
            keyring_core::CredentialPersistence::UntilDelete
        ),
        None => false,
    }
}

#[cfg(feature = "keyring")]
pub fn store_token(account_id: &str, token: &str) -> Result<(), NoteDeckError> {
    let entry = keyring_core::Entry::new(SERVICE, account_id)
        .map_err(|e| NoteDeckError::Keychain(e.to_string()))?;
    entry
        .set_password(token)
        .map_err(|e| NoteDeckError::Keychain(e.to_string()))
}

#[cfg(feature = "keyring")]
pub fn get_token(account_id: &str) -> Result<Option<String>, NoteDeckError> {
    let entry = keyring_core::Entry::new(SERVICE, account_id)
        .map_err(|e| NoteDeckError::Keychain(e.to_string()))?;
    match entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(NoteDeckError::Keychain(e.to_string())),
    }
}

#[cfg(feature = "keyring")]
pub fn delete_token(account_id: &str) -> Result<(), NoteDeckError> {
    let entry = keyring_core::Entry::new(SERVICE, account_id)
        .map_err(|e| NoteDeckError::Keychain(e.to_string()))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(NoteDeckError::Keychain(e.to_string())),
    }
}

// Fallback: no-ops (token stays in SQLite)
#[cfg(not(feature = "keyring"))]
pub fn init_store() -> Result<(), NoteDeckError> {
    Ok(())
}

#[cfg(all(test, feature = "keyring", target_os = "linux"))]
mod tests {
    use super::*;

    /// Linux の keyutils store は UntilReboot なので、init 前後どちらでも
    /// is_persistent() は false を返す（DB フォールバックを消してはならない)
    #[test]
    fn is_persistent_is_false_on_linux() {
        assert!(
            !is_persistent(),
            "store 未設定時は安全側 (false) に倒すこと"
        );
        if init_store().is_ok() {
            assert!(!is_persistent(), "keyutils store は再起動非永続");
        }
    }
}

#[cfg(not(feature = "keyring"))]
pub fn is_persistent() -> bool {
    false
}

#[cfg(not(feature = "keyring"))]
pub fn store_token(_account_id: &str, _token: &str) -> Result<(), NoteDeckError> {
    Ok(())
}

#[cfg(not(feature = "keyring"))]
pub fn get_token(_account_id: &str) -> Result<Option<String>, NoteDeckError> {
    Ok(None)
}

#[cfg(not(feature = "keyring"))]
pub fn delete_token(_account_id: &str) -> Result<(), NoteDeckError> {
    Ok(())
}
