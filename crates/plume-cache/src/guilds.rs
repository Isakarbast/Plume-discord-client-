//! The "everything but messages" snapshot: you, servers, channels, users,
//! read state and mute settings.

use plume_core::models::{Channel, Guild, GuildSettings, ReadState, User};
use plume_core::{Event, Snowflake, Store};
use rusqlite::params;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{Cache, CacheError};

/// What was saved last time, ready to be turned into a [`Event::Ready`].
#[derive(Debug)]
pub struct Snapshot {
    pub me: User,
    pub guilds: Vec<Guild>,
    pub channels: Vec<Channel>,
    pub users: Vec<User>,
    pub read_states: Vec<ReadState>,
    pub settings: Vec<GuildSettings>,
}

impl Snapshot {
    /// Lets the store load a snapshot exactly like a login.
    pub fn into_event(self) -> Event {
        Event::Ready {
            me: self.me,
            guilds: self.guilds,
            channels: self.channels,
            users: self.users,
            members: Vec::new(), // our roles are already inside each Guild
            read_states: self.read_states,
            settings: self.settings,
        }
    }
}

impl Cache {
    /// Replaces the saved snapshot with what the store knows now.
    pub fn save_snapshot(&mut self, store: &Store) -> Result<(), CacheError> {
        let Some(me) = store.me() else {
            return Ok(()); // not logged in yet: nothing worth saving
        };
        let tx = self.conn.transaction()?;
        for table in ["users", "guilds", "channels", "read_states", "settings"] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('me', ?1)",
            [me.id.0.to_string()],
        )?;
        {
            let mut users = tx.prepare("INSERT INTO users (id, data) VALUES (?1, ?2)")?;
            for user in store.users() {
                users.execute(params![id(user.id), json(user)?])?;
            }
            let mut guilds = tx.prepare("INSERT INTO guilds (id, data) VALUES (?1, ?2)")?;
            for guild in store.guilds() {
                guilds.execute(params![id(guild.id), json(guild)?])?;
            }
            let mut channels =
                tx.prepare("INSERT INTO channels (id, guild_id, data) VALUES (?1, ?2, ?3)")?;
            for channel in store.channels() {
                channels.execute(params![
                    id(channel.id),
                    channel.guild_id.map(id),
                    json(channel)?
                ])?;
            }
            let mut reads =
                tx.prepare("INSERT INTO read_states (channel_id, data) VALUES (?1, ?2)")?;
            for state in store.read_states() {
                reads.execute(params![id(state.channel_id), json(&state)?])?;
            }
            let mut settings =
                tx.prepare("INSERT INTO settings (guild_id, data) VALUES (?1, ?2)")?;
            for setting in store.settings() {
                settings.execute(params![setting.guild_id.map_or(0, id), json(setting)?])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The saved snapshot, or `None` if there isn't one yet.
    pub fn load_snapshot(&self) -> Result<Option<Snapshot>, CacheError> {
        let me_id: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'me'", [], |row| {
                row.get(0)
            })
            .ok();
        let Some(me_id) = me_id.and_then(|v| v.parse::<u64>().ok()).map(Snowflake) else {
            return Ok(None);
        };

        let users: Vec<User> = self.load_all("SELECT data FROM users")?;
        let Some(me) = users.iter().find(|u| u.id == me_id).cloned() else {
            return Ok(None);
        };
        Ok(Some(Snapshot {
            me,
            guilds: self.load_all("SELECT data FROM guilds")?,
            channels: self.load_all("SELECT data FROM channels")?,
            users,
            read_states: self.load_all("SELECT data FROM read_states")?,
            settings: self.load_all("SELECT data FROM settings")?,
        }))
    }

    fn load_all<T: DeserializeOwned>(&self, sql: &str) -> Result<Vec<T>, CacheError> {
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }
}

/// SQLite integers are signed; Discord IDs fit in 63 bits, so this is lossless.
pub(crate) fn id(snowflake: Snowflake) -> i64 {
    snowflake.0 as i64
}

pub(crate) fn json<T: Serialize>(value: &T) -> Result<String, CacheError> {
    Ok(serde_json::to_string(value)?)
}
