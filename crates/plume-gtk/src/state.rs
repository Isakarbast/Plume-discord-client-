//! Glue between the async world (tokio: gateway, REST, media) and GTK's
//! single-threaded main loop.

pub mod bridge;
pub mod view_models;
