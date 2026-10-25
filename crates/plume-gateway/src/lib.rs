//! Connection to Discord's real-time gateway (a WebSocket).
//!
//! [`Gateway::run`] connects, logs in, keeps the connection alive, reconnects
//! when it drops, and turns everything Discord sends into
//! [`plume_core::Event`]s on a channel.

mod compression;
mod connection;
mod dispatch;
mod error;
mod heartbeat;
mod opcodes;
mod resume;

pub use connection::{Command, Gateway, GatewayConfig, GatewayHandle};
pub use error::GatewayError;
