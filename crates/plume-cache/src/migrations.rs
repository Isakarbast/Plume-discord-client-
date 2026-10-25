use rusqlite::{Connection, OptionalExtension};

use crate::schema;

/// Bump this whenever the tables *or the JSON in them* change shape.
/// An older cache is then deleted and rebuilt, which is fine for a cache.
pub const VERSION: i64 = 2;

pub fn ensure_current(conn: &Connection) -> rusqlite::Result<()> {
    let version: Option<i64> = conn
        .query_row("SELECT value FROM meta WHERE key = 'version'", [], |row| {
            row.get::<_, String>(0).map(|v| v.parse().unwrap_or(-1))
        })
        .optional()
        .unwrap_or(None); // no meta table yet: a brand-new file
    if version != Some(VERSION) {
        recreate(conn)?;
    }
    Ok(())
}

pub fn recreate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(schema::DROP)?;
    conn.execute_batch(schema::CREATE)?;
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('version', ?1)",
        [VERSION.to_string()],
    )?;
    Ok(())
}
