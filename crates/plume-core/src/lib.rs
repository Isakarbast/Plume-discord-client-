//! Plume's domain layer: Discord's data types and the in-memory store.
//!
//! Nothing in here knows about networking or any UI toolkit, so every
//! frontend (GTK, TUI, lite) can share it.

pub mod events;
pub mod ids;
pub mod models;
pub mod navigation;
pub mod permissions;
pub mod profile;
pub mod store;
pub mod token;
pub mod wire;

pub use events::Event;
pub use ids::Snowflake;
pub use permissions::Permissions;
pub use profile::{Profile, Tier};
pub use store::{Store, Unread};
pub use token::Token;
