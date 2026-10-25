//! Desktop notifications: *when* to notify ([`unread`]) and *how* ([`desktop`]).
//!
//! The rules are plain functions over the store, so every frontend shares
//! them and they're easy to test. (A tray icon comes with the GTK app.)

pub mod desktop;
pub mod unread;

pub use desktop::show;
pub use unread::{Notification, notification_for};
