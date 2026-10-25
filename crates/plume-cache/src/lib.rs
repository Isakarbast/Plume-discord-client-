//! Saves what the client knew last time to SQLite, so the next start can
//! show servers, channels and recent messages before Discord even answers.
//!
//! It's a *cache*: it's never the source of truth, so when the format
//! changes we simply throw it away and rebuild it (see `migrations.rs`).
//! The Feather tier doesn't use it at all.

mod guilds;
mod messages;
mod migrations;
mod schema;

use std::path::Path;

use rusqlite::Connection;
use thiserror::Error;

pub use guilds::Snapshot;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("couldn't encode or decode cached data: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct Cache {
    conn: Connection,
}

impl Cache {
    /// Opens (or creates) the cache file.
    pub fn open(path: &Path) -> Result<Self, CacheError> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Cache::setup(Connection::open(path)?)
    }

    /// A throwaway cache, for tests.
    pub fn in_memory() -> Result<Self, CacheError> {
        Cache::setup(Connection::open_in_memory()?)
    }

    fn setup(conn: Connection) -> Result<Self, CacheError> {
        // WAL: readers don't block the writer, and a crash can't corrupt it.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrations::ensure_current(&conn)?;
        Ok(Cache { conn })
    }

    /// Deletes everything (e.g. on logout).
    pub fn clear(&mut self) -> Result<(), CacheError> {
        migrations::recreate(&self.conn)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plume_core::models::{Channel, ChannelKind, Guild, Message, ReadState, User};
    use plume_core::{Event, Profile, Snowflake, Store};

    fn user(id: u64) -> User {
        User {
            id: Snowflake(id),
            username: format!("user{id}"),
            global_name: None,
            avatar: Some("abc".into()),
        }
    }

    fn message(id: u64, channel: u64) -> Message {
        Message {
            id: Snowflake(id),
            channel_id: Snowflake(channel),
            author_id: Snowflake(2),
            content: format!("message {id}"),
            edited: false,
            reply_to: None,
            mentions: Vec::new(),
            mention_everyone: false,
            attachments: Vec::new(),
            embeds: Vec::new(),
            reactions: Vec::new(),
            stickers: Vec::new(),
            pinned: false,
        }
    }

    fn store() -> Store {
        let mut store = Store::new(Profile::STANDARD);
        store.apply(Event::Ready {
            me: user(1),
            guilds: vec![Guild {
                id: Snowflake(10),
                name: "Server".into(),
                icon: None,
                channels: vec![Snowflake(100)],
                owner_id: None,
                roles: Vec::new(),
                my_roles: vec![Snowflake(7)],
                voice_states: Vec::new(),
                stickers: Vec::new(),
                emojis: Vec::new(),
            }],
            channels: vec![Channel {
                id: Snowflake(100),
                guild_id: Some(Snowflake(10)),
                name: Some("general".into()),
                kind: ChannelKind::Text,
                parent_id: None,
                position: 0,
                recipients: Vec::new(),
                last_message_id: Some(Snowflake(60)),
                overwrites: Vec::new(),
                thread: None,
            }],
            users: vec![user(2)],
            members: Vec::new(),
            read_states: vec![ReadState {
                channel_id: Snowflake(100),
                last_read: Some(Snowflake(50)),
                mentions: 3,
            }],
            settings: Vec::new(),
        });
        store
    }

    #[test]
    fn snapshot_round_trips_into_a_new_store() {
        let mut cache = Cache::in_memory().unwrap();
        assert!(cache.load_snapshot().unwrap().is_none(), "empty at first");

        cache.save_snapshot(&store()).unwrap();
        let snapshot = cache.load_snapshot().unwrap().expect("saved");
        assert_eq!(snapshot.me.id, Snowflake(1));

        let mut restored = Store::new(Profile::STANDARD);
        restored.apply(snapshot.into_event());
        assert_eq!(
            restored.guild(Snowflake(10)).unwrap().my_roles,
            [Snowflake(7)]
        );
        assert_eq!(
            restored.channel(Snowflake(100)).unwrap().name.as_deref(),
            Some("general")
        );
        assert_eq!(
            restored.unread(Snowflake(100)).mentions,
            3,
            "read state survives"
        );
        assert_eq!(
            restored.user(Snowflake(2)).unwrap().avatar.as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn messages_round_trip_and_are_capped() {
        let mut cache = Cache::in_memory().unwrap();
        let mut store = store();
        store.open_channel(Snowflake(100));
        store.insert_history(
            Snowflake(100),
            (1..=30).map(|i| (message(i, 100), user(2))).collect(),
        );

        cache.save_messages(&store, 20).unwrap();
        let loaded = cache.load_messages(&store, Snowflake(100)).unwrap();
        assert_eq!(loaded.len(), 20);
        assert_eq!(
            loaded.first().unwrap().0.id,
            Snowflake(11),
            "newest 20 kept, oldest first"
        );
        assert_eq!(loaded[0].1.username, "user2");
    }

    #[test]
    fn prune_keeps_the_most_recent_channels() {
        let mut cache = Cache::in_memory().unwrap();
        let mut store = Store::new(Profile::FULL);
        store.apply(Event::Ready {
            me: user(1),
            guilds: Vec::new(),
            channels: Vec::new(),
            users: Vec::new(),
            members: Vec::new(),
            read_states: Vec::new(),
            settings: Vec::new(),
        });
        for channel in [100, 200, 300] {
            store.open_channel(Snowflake(channel));
            store.insert_history(
                Snowflake(channel),
                vec![(message(channel * 10, channel), user(2))],
            );
        }
        cache.save_messages(&store, 50).unwrap();
        cache.prune_messages(2).unwrap();
        assert!(
            cache
                .load_messages(&store, Snowflake(100))
                .unwrap()
                .is_empty(),
            "oldest channel dropped"
        );
        assert_eq!(
            cache.load_messages(&store, Snowflake(300)).unwrap().len(),
            1
        );
    }

    #[test]
    fn an_old_cache_version_is_rebuilt() {
        let dir = std::env::temp_dir().join(format!("plume-cache-test-{}", std::process::id()));
        let path = dir.join("cache.sqlite");
        let _ = std::fs::remove_file(&path);
        {
            let mut cache = Cache::open(&path).unwrap();
            cache.save_snapshot(&store()).unwrap();
            cache
                .conn
                .execute("UPDATE meta SET value = '0' WHERE key = 'version'", [])
                .unwrap();
        }
        let cache = Cache::open(&path).unwrap();
        assert!(
            cache.load_snapshot().unwrap().is_none(),
            "outdated cache was thrown away"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
