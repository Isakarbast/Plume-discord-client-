//! Token storage in the OS keyring (Secret Service on Linux, Keychain on
//! macOS, Credential Manager on Windows).
//!
//! These calls block briefly while talking to the keyring service. From async
//! code, run them with `tokio::task::spawn_blocking`.

use keyring::{Entry, Error};
use plume_core::Token;

use crate::AuthError;

const SERVICE: &str = "plume";
const ACCOUNT: &str = "discord-token";

fn entry() -> Result<Entry, AuthError> {
    Ok(Entry::new(SERVICE, ACCOUNT)?)
}

/// The saved token, or `None` if nobody has logged in yet.
pub fn load_token() -> Result<Option<Token>, AuthError> {
    match entry()?.get_password() {
        Ok(token) => Ok(Some(Token::new(token))),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn save_token(token: &Token) -> Result<(), AuthError> {
    entry()?.set_password(token.expose())?;
    Ok(())
}

/// Removes the saved token. Not an error if there wasn't one.
pub fn delete_token() -> Result<(), AuthError> {
    match entry()?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
