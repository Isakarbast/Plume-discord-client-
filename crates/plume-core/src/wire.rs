//! Discord's JSON shapes, shared by the gateway and REST crates.
//!
//! These mirror the API exactly and convert into the clean types in
//! [`crate::models`]. Fields we don't list are skipped by serde (without
//! allocating anything), and fields Discord might leave out are `Option` or
//! `#[serde(default)]`, so new or missing fields don't break parsing.

use serde::{Deserialize, Deserializer};

use crate::ids::Snowflake;
use crate::models::{
    Attachment, Channel, ChannelKind, Embed, EmbedImage, Guild, GuildEmoji, GuildSettings, Member,
    Message, Overwrite, OverwriteKind, Presence, Reaction, ReactionEmoji, ReadState, Relationship,
    RelationshipKind, Role, Status, Sticker, ThreadInfo, User, VoiceState,
};

#[derive(Debug, Deserialize)]
pub struct WireUser {
    pub id: Snowflake,
    pub username: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
}

impl WireUser {
    pub fn into_model(self) -> User {
        User {
            id: self.id,
            username: self.username,
            global_name: self.global_name,
            avatar: self.avatar,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WireGuild {
    pub id: Snowflake,
    name: Option<String>,
    icon: Option<String>,
    owner_id: Option<Snowflake>,
    /// Newer READY formats nest the name and owner in here instead.
    properties: Option<WireGuildProperties>,
    #[serde(default)]
    channels: Vec<WireChannel>,
    #[serde(default)]
    roles: Vec<WireRole>,
    #[serde(default)]
    emojis: Vec<WireEmoji>,
    #[serde(default)]
    members: Vec<WireMember>,
    #[serde(default)]
    voice_states: Vec<WireVoiceState>,
    /// Active threads we can see.
    #[serde(default)]
    threads: Vec<WireChannel>,
    #[serde(default)]
    stickers: Vec<WireSticker>,
}

#[derive(Debug, Deserialize)]
pub struct WireSticker {
    id: Snowflake,
    #[serde(default)]
    name: String,
    #[serde(default = "png")]
    format_type: u8,
}

fn png() -> u8 {
    1
}

impl WireSticker {
    pub fn into_model(self) -> Sticker {
        Sticker {
            id: self.id,
            name: self.name,
            format: self.format_type,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WireGuildProperties {
    name: Option<String>,
    icon: Option<String>,
    owner_id: Option<Snowflake>,
}

impl WireGuild {
    pub fn into_models(self) -> (Guild, Vec<Channel>, Vec<Member>) {
        let (name, icon, owner) = match self.properties {
            Some(p) => (
                self.name.or(p.name),
                self.icon.or(p.icon),
                self.owner_id.or(p.owner_id),
            ),
            None => (self.name, self.icon, self.owner_id),
        };
        let guild_id = self.id;
        // Channels inside a guild payload don't repeat the guild ID.
        let channels: Vec<Channel> = self
            .channels
            .into_iter()
            .chain(self.threads)
            .map(|c| c.into_model(Some(guild_id)))
            .collect();
        let members = self
            .members
            .into_iter()
            .filter_map(|m| m.into_model(guild_id))
            .collect();
        let guild = Guild {
            id: guild_id,
            name: name.unwrap_or_default(),
            icon,
            channels: channels.iter().map(|c| c.id).collect(),
            owner_id: owner,
            roles: self.roles.into_iter().map(WireRole::into_model).collect(),
            my_roles: Vec::new(),
            voice_states: self
                .voice_states
                .into_iter()
                .map(|v| v.into_model(Some(guild_id)))
                .collect(),
            emojis: self
                .emojis
                .into_iter()
                .filter_map(WireEmoji::into_model)
                .collect(),
            stickers: self
                .stickers
                .into_iter()
                .map(WireSticker::into_model)
                .collect(),
        };
        (guild, channels, members)
    }
}

#[derive(Debug, Deserialize)]
pub struct WireRole {
    id: Snowflake,
    #[serde(default, deserialize_with = "bits")]
    permissions: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    color: u32,
    #[serde(default)]
    position: i32,
    #[serde(default)]
    hoist: bool,
}

impl WireRole {
    pub fn into_model(self) -> Role {
        Role {
            id: self.id,
            permissions: self.permissions,
            name: self.name,
            color: self.color,
            position: self.position,
            hoist: self.hoist,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WireEmoji {
    id: Option<Snowflake>,
    name: Option<String>,
    #[serde(default)]
    animated: bool,
}

impl WireEmoji {
    fn into_model(self) -> Option<GuildEmoji> {
        Some(GuildEmoji {
            id: self.id?,
            name: self.name?,
            animated: self.animated,
        })
    }
}

/// A server member: either with a nested `user` or just a `user_id`.
#[derive(Debug, Deserialize)]
pub struct WireMember {
    pub user: Option<WireUser>,
    user_id: Option<Snowflake>,
    #[serde(default)]
    roles: Vec<Snowflake>,
    nick: Option<String>,
    /// Only in member-list items.
    pub presence: Option<WirePresence>,
}

impl WireMember {
    pub fn into_model(self, guild_id: Snowflake) -> Option<Member> {
        let user_id = self.user_id.or(self.user.as_ref().map(|u| u.id))?;
        Some(Member {
            guild_id,
            user_id,
            roles: self.roles,
            nick: self.nick,
        })
    }

    /// For a message's `member`, which leaves out the user.
    pub fn into_model_for(self, guild_id: Snowflake, user_id: Snowflake) -> Member {
        Member {
            guild_id,
            user_id,
            roles: self.roles,
            nick: self.nick,
        }
    }
}

/// Someone's status and activities.
#[derive(Debug, Deserialize)]
pub struct WirePresence {
    pub user: Option<WireId>,
    pub user_id: Option<Snowflake>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    activities: Vec<WireActivity>,
}

#[derive(Debug, Deserialize)]
pub struct WireId {
    pub id: Snowflake,
}

#[derive(Debug, Deserialize)]
struct WireActivity {
    #[serde(rename = "type", default)]
    kind: u8,
    #[serde(default)]
    name: String,
    state: Option<String>,
}

impl WirePresence {
    pub fn user_id(&self) -> Option<Snowflake> {
        self.user_id.or(self.user.as_ref().map(|u| u.id))
    }

    pub fn into_model(self) -> Presence {
        // A custom status wins; otherwise the first game/music/etc.
        let custom = self
            .activities
            .iter()
            .find(|a| a.kind == 4)
            .and_then(|a| a.state.clone())
            .filter(|s| !s.is_empty());
        let activity = custom.or_else(|| {
            self.activities.iter().find(|a| a.kind != 4).map(|a| {
                let verb = match a.kind {
                    0 => "Playing",
                    1 => "Streaming",
                    2 => "Listening to",
                    3 => "Watching",
                    5 => "Competing in",
                    _ => "",
                };
                format!("{verb} {}", a.name).trim().to_owned()
            })
        });
        Presence {
            status: Status::from_api(&self.status),
            activity,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct WireRelationship {
    id: Snowflake,
    #[serde(rename = "type")]
    kind: u8,
    pub user: Option<WireUser>,
    nickname: Option<String>,
}

impl WireRelationship {
    pub fn into_model(self) -> Option<(Relationship, Option<User>)> {
        let kind = RelationshipKind::from_api(self.kind)?;
        Some((
            Relationship {
                user_id: self.id,
                kind,
                nickname: self.nickname,
            },
            self.user.map(WireUser::into_model),
        ))
    }
}

#[derive(Debug, Deserialize)]
pub struct WireChannel {
    pub id: Snowflake,
    #[serde(rename = "type")]
    kind: u8,
    guild_id: Option<Snowflake>,
    name: Option<String>,
    parent_id: Option<Snowflake>,
    #[serde(default)]
    position: i32,
    #[serde(default)]
    recipients: Vec<WireUser>,
    last_message_id: Option<Snowflake>,
    #[serde(default)]
    permission_overwrites: Vec<WireOverwrite>,
    thread_metadata: Option<WireThreadMetadata>,
    /// Present when we've joined the thread.
    member: Option<serde::de::IgnoredAny>,
    #[serde(default)]
    message_count: u32,
}

#[derive(Debug, Deserialize)]
struct WireOverwrite {
    id: Snowflake,
    #[serde(rename = "type")]
    kind: WireOverwriteKind,
    #[serde(default, deserialize_with = "bits")]
    allow: u64,
    #[serde(default, deserialize_with = "bits")]
    deny: u64,
}

/// `0`/`1` today; older payloads used `"role"`/`"member"`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WireOverwriteKind {
    Number(u8),
    Text(String),
}

impl WireChannel {
    /// `guild_id` fills in the guild when the payload itself doesn't say.
    pub fn into_model(self, guild_id: Option<Snowflake>) -> Channel {
        self.into_parts(guild_id).0
    }

    /// The channel plus the full user objects of its DM recipients.
    pub fn into_parts(self, guild_id: Option<Snowflake>) -> (Channel, Vec<User>) {
        let recipients: Vec<User> = self
            .recipients
            .into_iter()
            .map(WireUser::into_model)
            .collect();
        let overwrites = self
            .permission_overwrites
            .into_iter()
            .map(|o| Overwrite {
                id: o.id,
                kind: match o.kind {
                    WireOverwriteKind::Number(1) => OverwriteKind::Member,
                    WireOverwriteKind::Text(t) if t == "member" => OverwriteKind::Member,
                    _ => OverwriteKind::Role,
                },
                allow: o.allow,
                deny: o.deny,
            })
            .collect();
        let channel = Channel {
            id: self.id,
            guild_id: self.guild_id.or(guild_id),
            name: self.name,
            kind: ChannelKind::from_api(self.kind),
            parent_id: self.parent_id,
            position: self.position,
            recipients: recipients.iter().map(|u| u.id).collect(),
            last_message_id: self.last_message_id,
            overwrites,
            thread: self.thread_metadata.map(|meta| ThreadInfo {
                archived: meta.archived,
                joined: self.member.is_some(),
                message_count: self.message_count,
            }),
        };
        (channel, recipients)
    }
}

#[derive(Debug, Deserialize)]
pub struct WireMessage {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    pub guild_id: Option<Snowflake>,
    /// The author's nickname and roles (gateway messages in servers only).
    pub member: Option<WireMember>,
    author: WireUser,
    #[serde(default)]
    content: String,
    edited_timestamp: Option<String>,
    message_reference: Option<WireMessageReference>,
    #[serde(default)]
    mentions: Vec<WireUser>,
    #[serde(default)]
    mention_everyone: bool,
    #[serde(default)]
    attachments: Vec<WireAttachment>,
    #[serde(default)]
    embeds: Vec<WireEmbed>,
    #[serde(default)]
    reactions: Vec<WireReaction>,
    #[serde(default)]
    sticker_items: Vec<WireSticker>,
    #[serde(default)]
    pinned: bool,
}

#[derive(Debug, Deserialize)]
struct WireMessageReference {
    message_id: Option<Snowflake>,
}

#[derive(Debug, Deserialize)]
struct WireAttachment {
    filename: String,
    url: String,
    #[serde(default)]
    size: u64,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct WireReaction {
    emoji: WireReactionEmoji,
    #[serde(default)]
    count: u32,
    #[serde(default)]
    me: bool,
}

#[derive(Debug, Deserialize)]
pub struct WireReactionEmoji {
    id: Option<Snowflake>,
    name: Option<String>,
}

impl WireReactionEmoji {
    pub fn into_model(self) -> ReactionEmoji {
        ReactionEmoji {
            id: self.id,
            // A deleted custom emoji can come without a name.
            name: self.name.unwrap_or_else(|| "?".to_owned()),
        }
    }
}

impl WireMessage {
    /// The message and its author.
    pub fn into_models(self) -> (Message, User) {
        let author = self.author.into_model();
        let message = Message {
            id: self.id,
            channel_id: self.channel_id,
            author_id: author.id,
            content: self.content,
            edited: self.edited_timestamp.is_some(),
            reply_to: self.message_reference.and_then(|r| r.message_id),
            mentions: self.mentions.iter().map(|u| u.id).collect(),
            mention_everyone: self.mention_everyone,
            attachments: self
                .attachments
                .into_iter()
                .map(|a| Attachment {
                    filename: a.filename,
                    url: a.url,
                    size: a.size,
                    width: a.width,
                    height: a.height,
                })
                .collect(),
            embeds: self.embeds.into_iter().map(WireEmbed::into_model).collect(),
            reactions: self
                .reactions
                .into_iter()
                .map(|r| Reaction {
                    emoji: r.emoji.into_model(),
                    count: r.count,
                    me: r.me,
                })
                .collect(),
            stickers: self
                .sticker_items
                .into_iter()
                .map(WireSticker::into_model)
                .collect(),
            pinned: self.pinned,
        };
        (message, author)
    }
}

#[derive(Debug, Deserialize)]
pub struct WireEmbed {
    title: Option<String>,
    description: Option<String>,
    url: Option<String>,
    provider: Option<WireName>,
    author: Option<WireName>,
    #[serde(default)]
    fields: Vec<WireEmbedField>,
    #[serde(rename = "type", default)]
    kind: String,
    color: Option<u32>,
    image: Option<WireEmbedImage>,
    thumbnail: Option<WireEmbedImage>,
}

#[derive(Debug, Deserialize)]
struct WireEmbedImage {
    url: Option<String>,
    /// Discord's copy; it can be resized with ?width=&height=.
    proxy_url: Option<String>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

impl WireEmbedImage {
    fn into_model(self) -> Option<EmbedImage> {
        Some(EmbedImage {
            url: self.proxy_url.or(self.url)?,
            width: self.width,
            height: self.height,
        })
    }
}

#[derive(Debug, Deserialize)]
struct WireName {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireEmbedField {
    name: String,
    value: String,
}

impl WireEmbed {
    pub fn into_model(self) -> Embed {
        Embed {
            provider: self.provider.and_then(|p| p.name),
            author: self.author.and_then(|a| a.name),
            title: self.title,
            url: self.url,
            description: self.description,
            fields: self.fields.into_iter().map(|f| (f.name, f.value)).collect(),
            kind: self.kind,
            color: self.color,
            image: self.image.and_then(WireEmbedImage::into_model),
            thumbnail: self.thumbnail.and_then(WireEmbedImage::into_model),
        }
    }
}

/// Lists that READY sometimes sends bare and sometimes as `{"entries": […]}`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum WireList<T> {
    Wrapped { entries: Vec<T> },
    Plain(Vec<T>),
}

impl<T> WireList<T> {
    pub fn into_vec(self) -> Vec<T> {
        match self {
            WireList::Wrapped { entries } | WireList::Plain(entries) => entries,
        }
    }
}

impl<T> Default for WireList<T> {
    fn default() -> Self {
        WireList::Plain(Vec::new())
    }
}

#[derive(Debug, Deserialize)]
pub struct WireReadState {
    id: Snowflake,
    last_message_id: Option<Snowflake>,
    #[serde(default)]
    mention_count: u32,
    /// 0 (or missing) = a channel; other kinds track things we don't show.
    #[serde(default)]
    read_state_type: u8,
}

impl WireReadState {
    pub fn into_model(self) -> Option<ReadState> {
        (self.read_state_type == 0).then_some(ReadState {
            channel_id: self.id,
            last_read: self.last_message_id.filter(|id| id.0 != 0),
            mentions: self.mention_count,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct WireGuildSettings {
    guild_id: Option<Snowflake>,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    channel_overrides: Vec<WireChannelOverride>,
    #[serde(default = "mentions_level")]
    message_notifications: u8,
}

fn mentions_level() -> u8 {
    1
}

#[derive(Debug, Deserialize)]
struct WireChannelOverride {
    channel_id: Snowflake,
    #[serde(default)]
    muted: bool,
}

impl WireGuildSettings {
    pub fn into_model(self) -> GuildSettings {
        GuildSettings {
            guild_id: self.guild_id,
            muted: self.muted,
            muted_channels: self
                .channel_overrides
                .into_iter()
                .filter(|o| o.muted)
                .map(|o| o.channel_id)
                .collect(),
            notifications: self.message_notifications,
        }
    }
}

/// Permission bit sets arrive as strings ("1024") in modern payloads.
fn bits<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Bits {
        Text(String),
        Number(u64),
    }
    match Bits::deserialize(deserializer)? {
        Bits::Text(text) => text.parse().map_err(serde::de::Error::custom),
        Bits::Number(n) => Ok(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_with_reply_mentions_embed_and_reactions() {
        let wire: WireMessage = serde_json::from_str(
            r#"{
                "id": "3", "channel_id": "2", "content": "look <@9>",
                "edited_timestamp": "2026-01-01T00:00:00+00:00",
                "author": { "id": "1", "username": "alice" },
                "message_reference": { "message_id": "2", "channel_id": "2" },
                "mentions": [{ "id": "9", "username": "bob" }],
                "attachments": [{ "filename": "cat.png", "url": "https://x/cat.png", "size": 1234 }],
                "embeds": [{ "title": "Rust", "provider": { "name": "Wikipedia" },
                             "fields": [{ "name": "Year", "value": "2015" }] }],
                "reactions": [{ "emoji": { "id": null, "name": "👍" }, "count": 3, "me": true },
                              { "emoji": { "id": "77", "name": null }, "count": 1, "me": false }],
                "brand_new_field": true
            }"#,
        )
        .unwrap();
        let (message, author) = wire.into_models();
        assert_eq!(author.username, "alice");
        assert!(message.edited);
        assert_eq!(message.reply_to, Some(Snowflake(2)));
        assert!(message.mentions_user(Snowflake(9)));
        assert_eq!(message.attachments[0].filename, "cat.png");
        assert_eq!(message.embeds[0].provider.as_deref(), Some("Wikipedia"));
        assert_eq!(message.embeds[0].fields[0], ("Year".into(), "2015".into()));
        assert_eq!(message.reactions[0].emoji, ReactionEmoji::unicode("👍"));
        assert!(message.reactions[0].me);
        assert_eq!(message.reactions[1].emoji.api_name(), "?:77");
    }

    #[test]
    fn guild_with_roles_overwrites_and_members() {
        let wire: WireGuild = serde_json::from_str(
            r#"{
                "id": "1", "name": "Server", "owner_id": "5",
                "roles": [{ "id": "1", "permissions": "1024" }, { "id": "2", "permissions": 8 }],
                "emojis": [{ "id": "40", "name": "pepe", "animated": true }],
                "members": [{ "user": { "id": "9", "username": "me" }, "roles": ["2"] }],
                "channels": [{
                    "id": "10", "type": 0, "name": "secret",
                    "permission_overwrites": [
                        { "id": "1", "type": 0, "allow": "0", "deny": "1024" },
                        { "id": "9", "type": "member", "allow": "1024", "deny": "0" }
                    ]
                }]
            }"#,
        )
        .unwrap();
        let (guild, channels, members) = wire.into_models();
        assert_eq!(guild.owner_id, Some(Snowflake(5)));
        assert_eq!(guild.roles[0].permissions, 1024);
        assert_eq!(guild.roles[1].permissions, 8);
        assert_eq!(guild.emojis[0].name, "pepe");
        assert_eq!(members[0].user_id, Snowflake(9));
        assert_eq!(channels[0].overwrites[0].deny, 1024);
        assert_eq!(channels[0].overwrites[1].kind, OverwriteKind::Member);
    }

    #[test]
    fn read_states_in_both_shapes() {
        let wrapped: WireList<WireReadState> = serde_json::from_str(
            r#"{ "entries": [{ "id": "1", "last_message_id": "5", "mention_count": 2 },
                             { "id": "2", "read_state_type": 1 }], "version": 3 }"#,
        )
        .unwrap();
        let plain: WireList<WireReadState> =
            serde_json::from_str(r#"[{ "id": "1", "last_message_id": 0 }]"#).unwrap();

        let states: Vec<ReadState> = wrapped
            .into_vec()
            .into_iter()
            .filter_map(WireReadState::into_model)
            .collect();
        assert_eq!(states.len(), 1, "non-channel read states are dropped");
        assert_eq!(states[0].mentions, 2);
        let states: Vec<ReadState> = plain
            .into_vec()
            .into_iter()
            .filter_map(WireReadState::into_model)
            .collect();
        assert_eq!(states[0].last_read, None, "0 means never read");
    }
}

#[derive(Debug, Deserialize)]
struct WireThreadMetadata {
    #[serde(default)]
    archived: bool,
}

#[derive(Debug, Deserialize)]
pub struct WireVoiceState {
    guild_id: Option<Snowflake>,
    channel_id: Option<Snowflake>,
    user_id: Snowflake,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    self_mute: bool,
    #[serde(default)]
    self_deaf: bool,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    deaf: bool,
    member: Option<WireMember>,
}

impl WireVoiceState {
    /// Voice states inside a guild payload don't repeat the guild ID.
    pub fn into_model(self, guild_id: Option<Snowflake>) -> VoiceState {
        VoiceState {
            guild_id: self.guild_id.or(guild_id),
            channel_id: self.channel_id,
            user_id: self.user_id,
            session_id: self.session_id,
            self_mute: self.self_mute,
            self_deaf: self.self_deaf,
            mute: self.mute,
            deaf: self.deaf,
            user: self.member.and_then(|m| m.user).map(WireUser::into_model),
        }
    }
}
