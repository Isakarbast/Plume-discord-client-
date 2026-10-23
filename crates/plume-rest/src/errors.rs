use reqwest::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RestError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("invalid JSON from Discord: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Discord didn't accept the token")]
    Unauthorized,

    #[error("you don't have access to that")]
    Forbidden,

    #[error("not found")]
    NotFound,

    #[error("still rate limited after several retries")]
    RateLimited,

    #[error("Discord returned {status}: {message}")]
    Api { status: StatusCode, message: String },
}
