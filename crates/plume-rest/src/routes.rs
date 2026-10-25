use plume_core::Snowflake;
use reqwest::Method;

/// Every endpoint we call. Keeping them in one enum means the rate limiter
/// always knows which bucket a request belongs to.
#[derive(Debug, Clone)]
pub enum Route {
    CurrentUser,
    GetMessages {
        channel_id: Snowflake,
    },
    CreateMessage {
        channel_id: Snowflake,
    },
    EditMessage {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    DeleteMessage {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    /// Add (`PUT`) or remove (`DELETE`) our own reaction. `emoji` is the
    /// emoji itself or `name:id` for a custom one.
    OwnReaction {
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: String,
        add: bool,
    },
    AckMessage {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    TriggerTyping {
        channel_id: Snowflake,
    },
    RingCall {
        channel_id: Snowflake,
    },
    UserProfile {
        user_id: Snowflake,
    },
    /// Open (or find) the DM with someone.
    CreateDm,
    Pins {
        channel_id: Snowflake,
    },
    /// Search a server (`guild: true`) or a DM's messages.
    Search {
        id: Snowflake,
        guild: bool,
    },
    /// Accept (`PUT`) or remove/decline (`DELETE`) a friend.
    Relationship {
        user_id: Snowflake,
        add: bool,
    },
    /// Send a friend request by username.
    FriendRequest,
    GuildSettings {
        guild_id: Snowflake,
    },
    /// Kick (`DELETE`) or change (`PATCH`, e.g. a timeout) a member.
    Member {
        guild_id: Snowflake,
        user_id: Snowflake,
        remove: bool,
    },
    Ban {
        guild_id: Snowflake,
        user_id: Snowflake,
    },
    /// Pin (`PUT`) or unpin (`DELETE`).
    Pin {
        channel_id: Snowflake,
        message_id: Snowflake,
        add: bool,
    },
    /// Change our display name or avatar.
    EditMe,
    /// Join a server with an invite code.
    Invite {
        code: String,
    },
    /// Discord's Tenor proxy: search (`Some`) or trending (`None`).
    Gifs {
        search: bool,
    },
}

impl Route {
    pub fn method(&self) -> Method {
        match self {
            Route::CurrentUser | Route::GetMessages { .. } | Route::UserProfile { .. } => {
                Method::GET
            }
            Route::CreateDm | Route::FriendRequest => Method::POST,
            Route::Pins { .. } | Route::Search { .. } | Route::Gifs { .. } => Method::GET,
            Route::Relationship { add: true, .. } => Method::PUT,
            Route::Relationship { add: false, .. } => Method::DELETE,
            Route::GuildSettings { .. } | Route::EditMe => Method::PATCH,
            Route::Member { remove: true, .. } | Route::Pin { add: false, .. } => Method::DELETE,
            Route::Member { remove: false, .. } => Method::PATCH,
            Route::Ban { .. } | Route::Pin { add: true, .. } => Method::PUT,
            Route::Invite { .. } => Method::POST,
            Route::CreateMessage { .. }
            | Route::AckMessage { .. }
            | Route::TriggerTyping { .. }
            | Route::RingCall { .. } => Method::POST,
            Route::EditMessage { .. } => Method::PATCH,
            Route::DeleteMessage { .. } => Method::DELETE,
            Route::OwnReaction { add: true, .. } => Method::PUT,
            Route::OwnReaction { add: false, .. } => Method::DELETE,
        }
    }

    pub fn path(&self) -> String {
        match self {
            Route::CurrentUser => "/users/@me".to_owned(),
            Route::GetMessages { channel_id } | Route::CreateMessage { channel_id } => {
                format!("/channels/{channel_id}/messages")
            }
            Route::EditMessage {
                channel_id,
                message_id,
            }
            | Route::DeleteMessage {
                channel_id,
                message_id,
            } => {
                format!("/channels/{channel_id}/messages/{message_id}")
            }
            Route::OwnReaction {
                channel_id,
                message_id,
                emoji,
                ..
            } => format!(
                "/channels/{channel_id}/messages/{message_id}/reactions/{}/@me",
                percent_encode(emoji)
            ),
            Route::AckMessage {
                channel_id,
                message_id,
            } => {
                format!("/channels/{channel_id}/messages/{message_id}/ack")
            }
            Route::TriggerTyping { channel_id } => format!("/channels/{channel_id}/typing"),
            Route::RingCall { channel_id } => format!("/channels/{channel_id}/call/ring"),
            Route::UserProfile { user_id } => format!("/users/{user_id}/profile"),
            Route::CreateDm => "/users/@me/channels".to_owned(),
            Route::Pins { channel_id } => format!("/channels/{channel_id}/pins"),
            Route::Search { id, guild: true } => format!("/guilds/{id}/messages/search"),
            Route::Search { id, guild: false } => format!("/channels/{id}/messages/search"),
            Route::Relationship { user_id, .. } => format!("/users/@me/relationships/{user_id}"),
            Route::FriendRequest => "/users/@me/relationships".to_owned(),
            Route::GuildSettings { guild_id } => format!("/users/@me/guilds/{guild_id}/settings"),
            Route::Member {
                guild_id, user_id, ..
            } => format!("/guilds/{guild_id}/members/{user_id}"),
            Route::Ban { guild_id, user_id } => format!("/guilds/{guild_id}/bans/{user_id}"),
            Route::Pin {
                channel_id,
                message_id,
                ..
            } => format!("/channels/{channel_id}/pins/{message_id}"),
            Route::EditMe => "/users/@me".to_owned(),
            Route::Invite { code } => format!("/invites/{}", percent_encode(code)),
            Route::Gifs { search: true } => "/gifs/search".to_owned(),
            Route::Gifs { search: false } => "/gifs/trending-gifs".to_owned(),
        }
    }

    /// Discord rate limits per route *and* per "major parameter" (the
    /// channel or guild), so sending in two channels uses two buckets.
    /// Message and emoji IDs are left out on purpose: they share a bucket.
    pub fn bucket_key(&self) -> String {
        let (name, channel_id) = match self {
            Route::CurrentUser => return "GET /users/@me".to_owned(),
            Route::UserProfile { .. } => return "GET profile".to_owned(),
            Route::CreateDm => return "POST dm".to_owned(),
            Route::Gifs { .. } => return "gifs".to_owned(),
            Route::EditMe => return "PATCH /users/@me".to_owned(),
            Route::Invite { .. } => return "invites".to_owned(),
            Route::Member { guild_id, .. } => ("members", guild_id),
            Route::Ban { guild_id, .. } => ("bans", guild_id),
            Route::Pin { channel_id, .. } => ("pin", channel_id),
            Route::Relationship { .. } | Route::FriendRequest => {
                return "relationships".to_owned();
            }
            Route::Pins { channel_id } => ("pins", channel_id),
            Route::Search { id, .. } => ("search", id),
            Route::GuildSettings { guild_id } => ("guild settings", guild_id),
            Route::GetMessages { channel_id } => ("GET messages", channel_id),
            Route::CreateMessage { channel_id } => ("POST messages", channel_id),
            Route::EditMessage { channel_id, .. } => ("PATCH message", channel_id),
            Route::DeleteMessage { channel_id, .. } => ("DELETE message", channel_id),
            Route::OwnReaction { channel_id, .. } => ("reactions", channel_id),
            Route::AckMessage { channel_id, .. } => ("ack", channel_id),
            Route::TriggerTyping { channel_id } => ("typing", channel_id),
            Route::RingCall { channel_id } => ("ring", channel_id),
        };
        format!("{name} {channel_id}")
    }
}

/// Makes text safe inside a URL path, e.g. "👍" → "%F0%9F%91%8D".
/// Makes text safe as a query value.
pub fn encode_query(text: &str) -> String {
    percent_encode(text).replace(':', "%3A")
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~:".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_emoji_in_reaction_paths() {
        let route = Route::OwnReaction {
            channel_id: Snowflake(1),
            message_id: Snowflake(2),
            emoji: "👍".into(),
            add: true,
        };
        assert_eq!(
            route.path(),
            "/channels/1/messages/2/reactions/%F0%9F%91%8D/@me"
        );
        assert_eq!(route.method(), Method::PUT);

        let custom = Route::OwnReaction {
            channel_id: Snowflake(1),
            message_id: Snowflake(2),
            emoji: "pepe:123".into(),
            add: false,
        };
        assert!(custom.path().contains("/reactions/pepe:123/@me"));
        assert_eq!(custom.method(), Method::DELETE);
    }

    #[test]
    fn messages_in_one_channel_share_a_bucket() {
        let edit = |message| Route::EditMessage {
            channel_id: Snowflake(1),
            message_id: Snowflake(message),
        };
        assert_eq!(edit(5).bucket_key(), edit(6).bucket_key());
        let other_channel = Route::EditMessage {
            channel_id: Snowflake(2),
            message_id: Snowflake(5),
        };
        assert_ne!(edit(5).bucket_key(), other_channel.bucket_key());
    }
}
