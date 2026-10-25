use thiserror::Error;

use crate::compression::CompressionError;
use crate::opcodes;

#[derive(Debug, Error)]
pub enum GatewayError {
    // Boxed because tungstenite's error type is large and would make every
    // `Result` in this crate large too.
    #[error("WebSocket error: {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),

    #[error("invalid JSON from the gateway: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Compression(#[from] CompressionError),

    #[error("expected HELLO as the first message, got op {0}")]
    NoHello(u8),

    #[error("the gateway closed the connection (code {code:?}): {reason}")]
    Closed { code: Option<u16>, reason: String },

    #[error("authentication failed: Discord rejected the token")]
    AuthenticationFailed,
}

impl GatewayError {
    /// Whether reconnecting is pointless (e.g. the token is wrong).
    pub fn is_fatal(&self) -> bool {
        match self {
            GatewayError::AuthenticationFailed => true,
            GatewayError::Closed {
                code: Some(code), ..
            } => opcodes::is_fatal_close(*code),
            _ => false,
        }
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for GatewayError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        GatewayError::WebSocket(Box::new(error))
    }
}
