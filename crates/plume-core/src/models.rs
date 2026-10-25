use serde::{Deserialize, Serialize};

use crate::ids::Snowflake;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Snowflake,
    pub username: String,
    pub global_name: Option<String>, // display name; not everyone has one
    pub avatar: Option<String>,      // image hash for the CDN; None = default avatar
}

impl User {
    /// What to show in the UI: the display name if set, otherwise the username.
    pub fn display_name(&self) -> &str {
        self.global_name.as_deref().unwrap_or(&self.username)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guild {
    pub id: Snowflake,
    pub name: String,
    pub icon: Option<String>, // image hash for the CDN
    pub channels: Vec<Snowflake>,
    pub owner_id: Option<Snowflake>,
    pub roles: Vec<Role>,
    /// Our own roles in this server (the store fills this in).
    pub my_roles: Vec<Snowflake>,
    pub emojis: Vec<GuildEmoji>,
    #[serde(default)]
    pub stickers: Vec<Sticker>,
    /// Who is in which voice channel right now. Never cached: it goes stale.
    #[serde(default, skip_serializing)]
    pub voice_states: Vec<VoiceState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Role {
    pub id: Snowflake,
    pub permissions: u64,
    #[serde(default)]
    pub name: String,
    /// `0xRRGGBB`; 0 means "no colour".
    #[serde(default)]
    pub color: u32,
    #[serde(default)]
    pub position: i32,
    /// Shown as its own group in the member list.
    #[serde(default)]
    pub hoist: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuildEmoji {
    pub id: Snowflake,
    pub name: String,
    pub animated: bool,
}

/// Someone's nickname and roles in a server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub guild_id: Snowflake,
    pub user_id: Snowflake,
    pub roles: Vec<Snowflake>,
    #[serde(default)]
    pub nick: Option<String>,
}

/// Online status, as shown by the dot on avatars.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Status {
    Online,
    Idle,
    /// Do not disturb.
    Dnd,
    /// Also used for "invisible" when it's us.
    #[default]
    Offline,
}

impl Status {
    pub fn from_api(status: &str) -> Self {
        match status {
            "online" => Status::Online,
            "idle" => Status::Idle,
            "dnd" => Status::Dnd,
            _ => Status::Offline,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Online => "Online",
            Status::Idle => "Idle",
            Status::Dnd => "Do Not Disturb",
            Status::Offline => "Offline",
        }
    }
}

/// What someone is up to: their status and a line like "Playing Minecraft".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Presence {
    pub status: Status,
    pub activity: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipKind {
    Friend,
    Blocked,
    /// A friend request someone sent us.
    Incoming,
    /// A friend request we sent.
    Outgoing,
}

impl RelationshipKind {
    pub fn from_api(kind: u8) -> Option<Self> {
        match kind {
            1 => Some(RelationshipKind::Friend),
            2 => Some(RelationshipKind::Blocked),
            3 => Some(RelationshipKind::Incoming),
            4 => Some(RelationshipKind::Outgoing),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub user_id: Snowflake,
    pub kind: RelationshipKind,
    pub nickname: Option<String>,
}

/// One row of a server's member list, as Discord sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberListItem {
    /// A heading: a hoisted role's ID, "online" or "offline".
    Group {
        id: String,
        count: u32,
    },
    Member(Snowflake),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    Text,
    Voice,
    Category,
    Dm,
    GroupDm,
    /// A thread inside a text channel (its parent is `parent_id`).
    Thread,
    Other, // anything we don't handle yet
}

impl ChannelKind {
    /// Maps Discord's numeric channel `type` to our enum.
    pub fn from_api(kind: u8) -> Self {
        match kind {
            0 | 5 => ChannelKind::Text, // 5 = announcement channel
            1 => ChannelKind::Dm,
            2 | 13 => ChannelKind::Voice, // 13 = stage channel
            3 => ChannelKind::GroupDm,
            4 => ChannelKind::Category,
            10..=12 => ChannelKind::Thread,
            _ => ChannelKind::Other, // threads, forums, …
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: Snowflake,
    pub guild_id: Option<Snowflake>, // None for DMs
    pub name: Option<String>,        // DMs have no name
    pub kind: ChannelKind,
    pub parent_id: Option<Snowflake>, // the category it sits in
    pub position: i32,
    pub recipients: Vec<Snowflake>, // the other people, for DMs
    pub last_message_id: Option<Snowflake>,
    pub overwrites: Vec<Overwrite>, // per-channel permission changes
    /// Set for threads.
    #[serde(default)]
    pub thread: Option<ThreadInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub archived: bool,
    /// We're a member (it shows in the sidebar, like Discord).
    pub joined: bool,
    pub message_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverwriteKind {
    Role,
    Member,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Overwrite {
    pub id: Snowflake, // a role or a user, depending on `kind`
    pub kind: OverwriteKind,
    pub allow: u64,
    pub deny: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    pub author_id: Snowflake, // just the ID, not a whole User
    pub content: String,
    pub edited: bool,
    pub reply_to: Option<Snowflake>, // the message this one replies to
    pub mentions: Vec<Snowflake>,    // users @mentioned in it
    pub mention_everyone: bool,      // @everyone / @here
    pub attachments: Vec<Attachment>,
    pub embeds: Vec<Embed>,
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub stickers: Vec<Sticker>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticker {
    pub id: Snowflake,
    pub name: String,
    /// 1 PNG, 2 APNG, 3 Lottie (not drawable here), 4 GIF.
    pub format: u8,
}

impl Message {
    /// When it was sent, in Unix milliseconds (it's encoded in the ID).
    pub fn timestamp_ms(&self) -> u64 {
        self.id.timestamp_ms()
    }

    pub fn mentions_user(&self, user_id: Snowflake) -> bool {
        self.mention_everyone || self.mentions.contains(&user_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub url: String,
    pub size: u64, // bytes
    /// Pixel size, for images and videos (so the layout doesn't jump while loading).
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

/// A link preview or bot card. Only the text parts; images come later.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Embed {
    pub provider: Option<String>,
    pub author: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub fields: Vec<(String, String)>,
    /// "rich", "image", "gifv", "video", "link", …
    #[serde(default)]
    pub kind: String,
    /// The left bar colour.
    #[serde(default)]
    pub color: Option<u32>,
    /// A big picture (or the thumbnail, for gifs and videos).
    #[serde(default)]
    pub image: Option<EmbedImage>,
    #[serde(default)]
    pub thumbnail: Option<EmbedImage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedImage {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// A unicode emoji (`id` is `None`) or a server's custom emoji.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReactionEmoji {
    pub id: Option<Snowflake>,
    pub name: String,
}

impl ReactionEmoji {
    pub fn unicode(emoji: &str) -> Self {
        ReactionEmoji {
            id: None,
            name: emoji.to_owned(),
        }
    }

    /// How the API wants it in a URL: the emoji itself, or `name:id`.
    pub fn api_name(&self) -> String {
        match self.id {
            Some(id) => format!("{}:{id}", self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: ReactionEmoji,
    pub count: u32,
    /// Whether we're one of the people who reacted.
    pub me: bool,
}

/// How far we've read in a channel, as Discord remembers it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ReadState {
    pub channel_id: Snowflake,
    pub last_read: Option<Snowflake>,
    pub mentions: u32,
}

/// Mute settings for one server (or for DMs, when `guild_id` is `None`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildSettings {
    pub guild_id: Option<Snowflake>,
    pub muted: bool,
    pub muted_channels: Vec<Snowflake>,
    /// 0 = all messages, 1 = only @mentions, 2 = nothing.
    #[serde(default = "only_mentions")]
    pub notifications: u8,
}

fn only_mentions() -> u8 {
    1
}

/// Someone's presence in a voice channel. `channel_id: None` means they left.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceState {
    /// `None` for DM and group calls.
    pub guild_id: Option<Snowflake>,
    pub channel_id: Option<Snowflake>,
    pub user_id: Snowflake,
    pub session_id: String,
    pub self_mute: bool,
    pub self_deaf: bool,
    /// Muted or deafened by a moderator.
    pub mute: bool,
    pub deaf: bool,
    /// Discord includes who they are, which the store keeps for names.
    #[serde(skip)]
    pub user: Option<User>,
}
