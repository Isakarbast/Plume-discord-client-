//! Numbers Discord uses to say what a gateway message is.
//! See <https://discord.com/developers/docs/topics/opcodes-and-status-codes>.

/// An event, like a new message. `t` holds its name.
pub const DISPATCH: u8 = 0;
/// Keep-alive. We send it on a timer; Discord can also ask for one.
pub const HEARTBEAT: u8 = 1;
/// Log in on a fresh session.
pub const IDENTIFY: u8 = 2;
/// Set our status (online, idle, …).
pub const PRESENCE_UPDATE: u8 = 3;
/// Join, leave or move between voice channels, and set mute/deafen.
pub const VOICE_STATE_UPDATE: u8 = 4;
/// Continue a dropped session without losing events.
pub const RESUME: u8 = 6;
/// Discord wants us to reconnect (and resume).
pub const RECONNECT: u8 = 7;
/// Our session is invalid. `d` says whether it can still be resumed.
pub const INVALID_SESSION: u8 = 9;
/// First message after connecting. Carries the heartbeat interval.
pub const HELLO: u8 = 10;
/// Discord received our heartbeat.
pub const HEARTBEAT_ACK: u8 = 11;
/// Subscribe to a server's channel ("lazy guilds"). User accounts only.
pub const GUILD_SUBSCRIBE: u8 = 14;

/// Close codes that mean "stop, reconnecting won't help".
pub fn is_fatal_close(code: u16) -> bool {
    matches!(
        code,
        4004 // authentication failed
        | 4010 // invalid shard
        | 4011 // sharding required
        | 4012 // invalid API version
        | 4013 // invalid intents
        | 4014 // disallowed intents
    )
}

/// Close codes after which the old session can't be resumed.
pub fn invalidates_session(code: u16) -> bool {
    matches!(
        code,
        4007 // invalid sequence number
        | 4009 // session timed out
    )
}
