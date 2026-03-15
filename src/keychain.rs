use crate::error::NoteDeckError;

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
