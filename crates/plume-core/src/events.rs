use crate::ids::Snowflake;
use crate::models::{
    Channel, Embed, Guild, GuildSettings, Member, MemberListItem, Message, Presence, ReactionEmoji,
    ReadState, Relationship, Role, User, VoiceState,
};

/// Something that happened on Discord, already decoded into our own types.
///
/// The gateway produces these; the [`Store`](crate::Store) and the UI consume them.
#[derive(Debug, Clone)]
pub enum Event {
    /// Logged in. Contains the initial snapshot of everything we can see.
    Ready {
        me: User,
        guilds: Vec<Guild>,
        channels: Vec<Channel>,
        users: Vec<User>,
        members: Vec<Member>,
        read_states: Vec<ReadState>,
        settings: Vec<GuildSettings>,
    },
    /// A dropped connection was resumed; events we missed have been replayed.
    Resumed,
    GuildCreate {
        guild: Guild,
        channels: Vec<Channel>,
        members: Vec<Member>,
    },
    GuildDelete {
        id: Snowflake,
        /// `true` means a Discord outage, not that we left the server.
        unavailable: bool,
    },
    GuildRoleUpsert {
        guild_id: Snowflake,
        role: Role,
    },
    GuildRoleDelete {
        guild_id: Snowflake,
        role_id: Snowflake,
    },
    /// Someone's nickname or roles (also sent along with their messages).
    GuildMemberUpdate(Member),
    /// Mute settings changed (possibly on another device).
    GuildSettingsUpdate(GuildSettings),
    /// A channel was created or changed.
    ChannelUpsert(Channel),
    ChannelDelete {
        id: Snowflake,
    },
    MessageCreate {
        message: Message,
        author: User,
    },
    /// Only the fields Discord sent are `Some`.
    MessageUpdate {
        id: Snowflake,
        channel_id: Snowflake,
        content: Option<String>,
        embeds: Option<Vec<Embed>>,
        edited: bool,
    },
    MessageDelete {
        id: Snowflake,
        channel_id: Snowflake,
    },
    ReactionAdd {
        channel_id: Snowflake,
        message_id: Snowflake,
        user_id: Snowflake,
        emoji: ReactionEmoji,
    },
    ReactionRemove {
        channel_id: Snowflake,
        message_id: Snowflake,
        user_id: Snowflake,
        emoji: ReactionEmoji,
    },
    /// All reactions (or all of one emoji, if `emoji` is set) were cleared.
    ReactionClear {
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: Option<ReactionEmoji>,
    },
    /// We read a channel up to here (maybe on another device).
    MessageAck {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    TypingStart {
        channel_id: Snowflake,
        user_id: Snowflake,
    },
    /// Someone joined, left or changed mute/deafen in a voice channel.
    VoiceStateUpdate(VoiceState),
    /// Where to connect for voice, after we asked to join a channel.
    /// `endpoint: None` means the server is being moved; wait for the next one.
    VoiceServerUpdate {
        guild_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
        token: String,
        endpoint: Option<String>,
    },
    /// Our whole friends list (right after READY).
    Relationships(Vec<Relationship>),
    RelationshipAdd {
        relationship: Relationship,
        user: Option<User>,
    },
    RelationshipRemove {
        user_id: Snowflake,
    },
    /// Many statuses at once (friends, right after READY).
    Presences(Vec<(Snowflake, Presence)>),
    PresenceUpdate {
        user_id: Snowflake,
        presence: Presence,
    },
    /// Changes to a server's member list (the sidebar on the right).
    MemberListUpdate {
        guild_id: Snowflake,
        list_id: String,
        ops: Vec<MemberListOp>,
    },
    /// An event we don't handle yet. Carries the event name for debugging.
    Other(String),
}

/// One change to a member list. Indexes count group headings too.
#[derive(Debug, Clone)]
pub enum MemberListOp {
    /// Replace everything in `range` (inclusive).
    Sync {
        start: usize,
        items: Vec<MemberListEntry>,
    },
    Insert {
        index: usize,
        item: MemberListEntry,
    },
    Update {
        index: usize,
        item: MemberListEntry,
    },
    Delete {
        index: usize,
    },
    /// Forget a range; Discord will resend it.
    Invalidate {
        start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone)]
pub enum MemberListEntry {
    Group {
        id: String,
        count: u32,
    },
    Member {
        user: User,
        member: Member,
        presence: Presence,
    },
}

impl MemberListEntry {
    pub fn to_item(&self) -> MemberListItem {
        match self {
            MemberListEntry::Group { id, count } => MemberListItem::Group {
                id: id.clone(),
                count: *count,
            },
            MemberListEntry::Member { user, .. } => MemberListItem::Member(user.id),
        }
    }
}
