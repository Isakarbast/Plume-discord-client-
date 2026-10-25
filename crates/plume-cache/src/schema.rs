//! The tables. Each row stores one object as JSON next to the columns we
//! look it up by, so adding a field to a model never needs a migration.

pub const CREATE: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE users (
    id   INTEGER PRIMARY KEY,
    data TEXT NOT NULL
);
CREATE TABLE guilds (
    id   INTEGER PRIMARY KEY,
    data TEXT NOT NULL
);
CREATE TABLE channels (
    id       INTEGER PRIMARY KEY,
    guild_id INTEGER,
    data     TEXT NOT NULL
);
CREATE TABLE read_states (
    channel_id INTEGER PRIMARY KEY,
    data       TEXT NOT NULL
);
CREATE TABLE settings (
    guild_id INTEGER PRIMARY KEY, -- 0 = DM settings
    data     TEXT NOT NULL
);
CREATE TABLE messages (
    id         INTEGER PRIMARY KEY,
    channel_id INTEGER NOT NULL,
    data       TEXT NOT NULL
);
CREATE INDEX messages_by_channel ON messages (channel_id, id);
";

pub const DROP: &str = "
DROP TABLE IF EXISTS meta;
DROP TABLE IF EXISTS users;
DROP TABLE IF EXISTS guilds;
DROP TABLE IF EXISTS channels;
DROP TABLE IF EXISTS read_states;
DROP TABLE IF EXISTS settings;
DROP TABLE IF EXISTS messages;
";
