//! Recent messages per channel, so opening a channel shows something
//! immediately while the real history loads.

use plume_core::models::{Message, User};
use plume_core::{Snowflake, Store};
use rusqlite::params;

use crate::guilds::{id, json};
use crate::{Cache, CacheError};

impl Cache {
    /// Saves the newest `per_channel` messages of every channel the store
    /// holds, replacing what was saved for those channels before.
    pub fn save_messages(&mut self, store: &Store, per_channel: usize) -> Result<(), CacheError> {
        let tx = self.conn.transaction()?;
        {
            let mut clear = tx.prepare("DELETE FROM messages WHERE channel_id = ?1")?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO messages (id, channel_id, data) VALUES (?1, ?2, ?3)",
            )?;
            for channel_id in store.cached_channels() {
                let messages: Vec<&Message> = store.messages(channel_id).collect();
                if messages.is_empty() {
                    continue;
                }
                clear.execute([id(channel_id)])?;
                let skip = messages.len().saturating_sub(per_channel);
                for message in &messages[skip..] {
                    insert.execute(params![id(message.id), id(channel_id), json(message)?])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The saved messages for a channel (oldest first), with their authors
    /// when we know them.
    pub fn load_messages(
        &self,
        store: &Store,
        channel_id: Snowflake,
    ) -> Result<Vec<(Message, User)>, CacheError> {
        let mut statement = self
            .conn
            .prepare("SELECT data FROM messages WHERE channel_id = ?1 ORDER BY id")?;
        let rows = statement.query_map([id(channel_id)], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let message: Message = serde_json::from_str(&row?)?;
            let author = store
                .user(message.author_id)
                .cloned()
                .unwrap_or_else(|| User {
                    id: message.author_id,
                    username: "unknown".to_owned(),
                    global_name: None,
                    avatar: None,
                });
            out.push((message, author));
        }
        Ok(out)
    }

    /// Keeps the file from growing forever: drops channels beyond the
    /// `keep_channels` most recently saved.
    pub fn prune_messages(&mut self, keep_channels: usize) -> Result<(), CacheError> {
        self.conn.execute(
            "DELETE FROM messages WHERE channel_id NOT IN (
                SELECT channel_id FROM messages GROUP BY channel_id
                ORDER BY MAX(id) DESC LIMIT ?1
            )",
            [keep_channels as i64],
        )?;
        Ok(())
    }
}
