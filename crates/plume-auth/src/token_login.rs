use plume_core::Token;
use plume_rest::{RestClient, RestError};

use crate::AuthError;

/// Reads the token from the `DISCORD_TOKEN` environment variable, if set.
pub fn token_from_env() -> Option<Token> {
    std::env::var("DISCORD_TOKEN")
        .ok()
        .map(Token::new)
        .filter(|t| !t.expose().is_empty())
}

/// Asks Discord who this token belongs to. Returns the username, or
/// [`AuthError::InvalidToken`] if Discord doesn't accept it.
pub async fn verify_token(token: &Token) -> Result<String, AuthError> {
    match RestClient::new(token)?.current_user().await {
        Ok(me) => Ok(me.username),
        Err(RestError::Unauthorized) => Err(AuthError::InvalidToken),
        Err(error) => Err(error.into()),
    }
}
