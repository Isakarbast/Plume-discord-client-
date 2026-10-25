//! Turns raw dispatch JSON into [`plume_core::Event`]s.
//!
//! Shared JSON shapes live in `plume_core::wire`; the ones here only exist
//! in gateway events.

use plume_core::events::{MemberListEntry, MemberListOp};
use plume_core::models::{Member, Presence, User};
use plume_core::wire::{
    WireChannel, WireEmbed, WireGuild, WireGuildSettings, WireList, WireMember, WireMessage,
    WirePresence, WireReactionEmoji, WireReadState, WireRelationship, WireRole, WireUser,
    WireVoiceState,
};
use plume_core::{Event, Snowflake};
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::resume::ResumeState;

/// A decoded dispatch, plus session info if it was READY.
pub struct Decoded {
    pub event: Event,
    /// More events carried by the same dispatch (sent after `event`).
    pub extra: Vec<Event>,
    pub session: Option<ResumeState>,
}

impl From<Event> for Decoded {
    fn from(event: Event) -> Self {
        Decoded {
            event,
            extra: Vec::new(),
            session: None,
        }
    }
}

pub fn decode(name: &str, data: &RawValue) -> Result<Decoded, serde_json::Error> {
    let json = data.get();
    let event = match name {
        "READY" => {
            let ready: WireReady = serde_json::from_str(json)?;
            return Ok(ready.into_decoded());
        }
        "RESUMED" => Event::Resumed,
        "GUILD_CREATE" => {
            let guild: WireGuild = serde_json::from_str(json)?;
            let (guild, channels, members) = guild.into_models();
            Event::GuildCreate {
                guild,
                channels,
                members,
            }
        }
        "GUILD_DELETE" => {
            let gone: WireGuildDelete = serde_json::from_str(json)?;
            Event::GuildDelete {
                id: gone.id,
                unavailable: gone.unavailable,
            }
        }
        "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
            let update: WireRoleUpdate = serde_json::from_str(json)?;
            Event::GuildRoleUpsert {
                guild_id: update.guild_id,
                role: update.role.into_model(),
            }
        }
        "GUILD_ROLE_DELETE" => {
            let gone: WireRoleDelete = serde_json::from_str(json)?;
            Event::GuildRoleDelete {
                guild_id: gone.guild_id,
                role_id: gone.role_id,
            }
        }
        "GUILD_MEMBER_UPDATE" => {
            let update: WireMemberUpdate = serde_json::from_str(json)?;
            match update.member.into_model(update.guild_id) {
                Some(member) => Event::GuildMemberUpdate(member),
                None => Event::Other(name.to_owned()),
            }
        }
        "USER_GUILD_SETTINGS_UPDATE" => {
            let settings: WireGuildSettings = serde_json::from_str(json)?;
            Event::GuildSettingsUpdate(settings.into_model())
        }
        "THREAD_LIST_SYNC" => {
            #[derive(Deserialize)]
            struct Sync {
                guild_id: Snowflake,
                #[serde(default)]
                threads: Vec<WireChannel>,
            }
            let sync: Sync = serde_json::from_str(json)?;
            let mut events = sync
                .threads
                .into_iter()
                .map(|t| Event::ChannelUpsert(t.into_model(Some(sync.guild_id))));
            let Some(first) = events.next() else {
                return Ok(Event::Other(name.to_owned()).into());
            };
            let mut decoded = Decoded::from(first);
            decoded.extra.extend(events);
            return Ok(decoded);
        }
        "THREAD_DELETE" => {
            let thread: WireId = serde_json::from_str(json)?;
            Event::ChannelDelete { id: thread.id }
        }
        "CHANNEL_CREATE" | "CHANNEL_UPDATE" | "THREAD_CREATE" | "THREAD_UPDATE" => {
            let channel: WireChannel = serde_json::from_str(json)?;
            Event::ChannelUpsert(channel.into_model(None))
        }
        "CHANNEL_DELETE" => {
            let channel: WireId = serde_json::from_str(json)?;
            Event::ChannelDelete { id: channel.id }
        }
        "MESSAGE_CREATE" => {
            let mut message: WireMessage = serde_json::from_str(json)?;
            let member = message.member.take().zip(message.guild_id);
            let (message, author) = message.into_models();
            let mut decoded = Decoded::from(Event::MessageCreate {
                message,
                author: author.clone(),
            });
            if let Some((member, guild_id)) = member {
                decoded.extra.push(Event::GuildMemberUpdate(
                    member.into_model_for(guild_id, author.id),
                ));
            }
            return Ok(decoded);
        }
        "PRESENCE_UPDATE" => {
            let presence: WirePresence = serde_json::from_str(json)?;
            match presence.user_id() {
                Some(user_id) => Event::PresenceUpdate {
                    user_id,
                    presence: presence.into_model(),
                },
                None => Event::Other(name.to_owned()),
            }
        }
        "RELATIONSHIP_ADD" => {
            let relationship: WireRelationship = serde_json::from_str(json)?;
            match relationship.into_model() {
                Some((relationship, user)) => Event::RelationshipAdd { relationship, user },
                None => Event::Other(name.to_owned()),
            }
        }
        "RELATIONSHIP_REMOVE" => {
            let gone: WireId = serde_json::from_str(json)?;
            Event::RelationshipRemove { user_id: gone.id }
        }
        "GUILD_MEMBER_LIST_UPDATE" => {
            let update: WireMemberList = serde_json::from_str(json)?;
            update.into_event()
        }
        "MESSAGE_UPDATE" => {
            let update: WireMessageUpdate = serde_json::from_str(json)?;
            Event::MessageUpdate {
                id: update.id,
                channel_id: update.channel_id,
                content: update.content,
                embeds: update
                    .embeds
                    .map(|e| e.into_iter().map(WireEmbed::into_model).collect()),
                edited: update.edited_timestamp.is_some(),
            }
        }
        "MESSAGE_DELETE" => {
            let gone: WireMessageRef = serde_json::from_str(json)?;
            Event::MessageDelete {
                id: gone.id,
                channel_id: gone.channel_id,
            }
        }
        "MESSAGE_REACTION_ADD" | "MESSAGE_REACTION_REMOVE" => {
            let reaction: WireReactionEvent = serde_json::from_str(json)?;
            let (channel_id, message_id, user_id) =
                (reaction.channel_id, reaction.message_id, reaction.user_id);
            let emoji = reaction.emoji.into_model();
            if name == "MESSAGE_REACTION_ADD" {
                Event::ReactionAdd {
                    channel_id,
                    message_id,
                    user_id,
                    emoji,
                }
            } else {
                Event::ReactionRemove {
                    channel_id,
                    message_id,
                    user_id,
                    emoji,
                }
            }
        }
        "MESSAGE_REACTION_REMOVE_ALL" | "MESSAGE_REACTION_REMOVE_EMOJI" => {
            let clear: WireReactionClear = serde_json::from_str(json)?;
            Event::ReactionClear {
                channel_id: clear.channel_id,
                message_id: clear.message_id,
                emoji: clear.emoji.map(WireReactionEmoji::into_model),
            }
        }
        "MESSAGE_ACK" => {
            let ack: WireAck = serde_json::from_str(json)?;
            Event::MessageAck {
                channel_id: ack.channel_id,
                message_id: ack.message_id,
            }
        }
        "TYPING_START" => {
            let typing: WireTypingStart = serde_json::from_str(json)?;
            Event::TypingStart {
                channel_id: typing.channel_id,
                user_id: typing.user_id,
            }
        }
        "VOICE_STATE_UPDATE" => {
            let state: WireVoiceState = serde_json::from_str(json)?;
            Event::VoiceStateUpdate(state.into_model(None))
        }
        "VOICE_SERVER_UPDATE" => {
            let server: WireVoiceServer = serde_json::from_str(json)?;
            Event::VoiceServerUpdate {
                guild_id: server.guild_id,
                channel_id: server.channel_id,
                token: server.token,
                endpoint: server.endpoint,
            }
        }
        // Everything else isn't parsed at all, which costs nothing.
        other => Event::Other(other.to_owned()),
    };
    Ok(event.into())
}

#[derive(Deserialize)]
struct WireMemberList {
    guild_id: Snowflake,
    #[serde(default)]
    id: String,
    #[serde(default)]
    ops: Vec<WireListOp>,
}

#[derive(Deserialize)]
struct WireListOp {
    op: String,
    range: Option<(usize, usize)>,
    index: Option<usize>,
    #[serde(default)]
    items: Vec<WireListItem>,
    item: Option<WireListItem>,
}

#[derive(Deserialize)]
struct WireListItem {
    group: Option<WireListGroup>,
    member: Option<plume_core::wire::WireMember>,
}

#[derive(Deserialize)]
struct WireListGroup {
    id: String,
    #[serde(default)]
    count: u32,
}

impl WireListItem {
    fn into_entry(self, guild_id: Snowflake) -> Option<MemberListEntry> {
        if let Some(group) = self.group {
            return Some(MemberListEntry::Group {
                id: group.id,
                count: group.count,
            });
        }
        let mut member = self.member?;
        let user = member.user.take()?.into_model();
        let presence = member
            .presence
            .take()
            .map(WirePresence::into_model)
            .unwrap_or_default();
        let member = member.into_model_for(guild_id, user.id);
        Some(MemberListEntry::Member {
            user,
            member,
            presence,
        })
    }
}

impl WireMemberList {
    fn into_event(self) -> Event {
        let guild_id = self.guild_id;
        let ops = self
            .ops
            .into_iter()
            .filter_map(|op| {
                let entry = |item: Option<WireListItem>| item.and_then(|i| i.into_entry(guild_id));
                Some(match op.op.as_str() {
                    "SYNC" => MemberListOp::Sync {
                        start: op.range?.0,
                        items: op
                            .items
                            .into_iter()
                            .filter_map(|i| i.into_entry(guild_id))
                            .collect(),
                    },
                    "INSERT" => MemberListOp::Insert {
                        index: op.index?,
                        item: entry(op.item)?,
                    },
                    "UPDATE" => MemberListOp::Update {
                        index: op.index?,
                        item: entry(op.item)?,
                    },
                    "DELETE" => MemberListOp::Delete { index: op.index? },
                    "INVALIDATE" => {
                        let (start, end) = op.range?;
                        MemberListOp::Invalidate { start, end }
                    }
                    _ => return None,
                })
            })
            .collect();
        Event::MemberListUpdate {
            guild_id,
            list_id: self.id,
            ops,
        }
    }
}

#[derive(Default, Deserialize)]
struct WireMergedPresences {
    #[serde(default)]
    friends: Vec<WirePresence>,
}

#[derive(Deserialize)]
struct WireVoiceServer {
    guild_id: Option<Snowflake>,
    channel_id: Option<Snowflake>,
    token: String,
    endpoint: Option<String>,
}

#[derive(Deserialize)]
struct WireReady {
    user: WireUser,
    session_id: String,
    resume_gateway_url: String,
    #[serde(default)]
    guilds: Vec<WireGuild>,
    #[serde(default)]
    private_channels: Vec<WireChannel>,
    #[serde(default)]
    users: Vec<WireUser>,
    /// Our member objects, one list per guild, in the same order as `guilds`.
    #[serde(default)]
    merged_members: Vec<Vec<WireMember>>,
    #[serde(default)]
    read_state: WireList<WireReadState>,
    #[serde(default)]
    user_guild_settings: WireList<WireGuildSettings>,
    #[serde(default)]
    relationships: Vec<WireRelationship>,
    #[serde(default)]
    merged_presences: WireMergedPresences,
}

impl WireReady {
    fn into_decoded(self) -> Decoded {
        let mut guilds = Vec::with_capacity(self.guilds.len());
        let mut channels = Vec::new();
        let mut members: Vec<Member> = Vec::new();
        let mut users: Vec<User> = self.users.into_iter().map(WireUser::into_model).collect();

        let mut merged = self.merged_members.into_iter();
        for guild in self.guilds {
            let (guild, guild_channels, guild_members) = guild.into_models();
            members.extend(guild_members);
            if let Some(extra) = merged.next() {
                members.extend(extra.into_iter().filter_map(|m| m.into_model(guild.id)));
            }
            guilds.push(guild);
            channels.extend(guild_channels);
        }
        for channel in self.private_channels {
            let (channel, recipients) = channel.into_parts(None);
            channels.push(channel);
            users.extend(recipients);
        }

        let mut relationships = Vec::new();
        for relationship in self.relationships {
            if let Some((relationship, user)) = relationship.into_model() {
                users.extend(user);
                relationships.push(relationship);
            }
        }
        let presences: Vec<(Snowflake, Presence)> = self
            .merged_presences
            .friends
            .into_iter()
            .filter_map(|p| Some((p.user_id()?, p.into_model())))
            .collect();

        Decoded {
            extra: vec![
                Event::Relationships(relationships),
                Event::Presences(presences),
            ],
            event: Event::Ready {
                me: self.user.into_model(),
                guilds,
                channels,
                users,
                members,
                read_states: self
                    .read_state
                    .into_vec()
                    .into_iter()
                    .filter_map(WireReadState::into_model)
                    .collect(),
                settings: self
                    .user_guild_settings
                    .into_vec()
                    .into_iter()
                    .map(WireGuildSettings::into_model)
                    .collect(),
            },
            session: Some(ResumeState {
                session_id: self.session_id,
                resume_url: self.resume_gateway_url,
            }),
        }
    }
}

#[derive(Deserialize)]
struct WireGuildDelete {
    id: Snowflake,
    #[serde(default)]
    unavailable: bool,
}

#[derive(Deserialize)]
struct WireRoleUpdate {
    guild_id: Snowflake,
    role: WireRole,
}

#[derive(Deserialize)]
struct WireRoleDelete {
    guild_id: Snowflake,
    role_id: Snowflake,
}

#[derive(Deserialize)]
struct WireMemberUpdate {
    guild_id: Snowflake,
    #[serde(flatten)]
    member: WireMember,
}

#[derive(Deserialize)]
struct WireId {
    id: Snowflake,
}

#[derive(Deserialize)]
struct WireMessageUpdate {
    id: Snowflake,
    channel_id: Snowflake,
    content: Option<String>,
    embeds: Option<Vec<WireEmbed>>,
    edited_timestamp: Option<String>,
}

#[derive(Deserialize)]
struct WireMessageRef {
    id: Snowflake,
    channel_id: Snowflake,
}

#[derive(Deserialize)]
struct WireReactionEvent {
    user_id: Snowflake,
    channel_id: Snowflake,
    message_id: Snowflake,
    emoji: WireReactionEmoji,
}

#[derive(Deserialize)]
struct WireReactionClear {
    channel_id: Snowflake,
    message_id: Snowflake,
    emoji: Option<WireReactionEmoji>,
}

#[derive(Deserialize)]
struct WireAck {
    channel_id: Snowflake,
    message_id: Snowflake,
}

#[derive(Deserialize)]
struct WireTypingStart {
    channel_id: Snowflake,
    user_id: Snowflake,
}

#[cfg(test)]
mod tests {
    use super::*;
    use plume_core::models::{ChannelKind, ReactionEmoji};

    fn decode_str(name: &str, json: &str) -> Decoded {
        let raw = RawValue::from_string(json.to_owned()).unwrap();
        decode(name, &raw).unwrap()
    }

    #[test]
    fn decodes_message_create_and_ignores_unknown_fields() {
        let decoded = decode_str(
            "MESSAGE_CREATE",
            r#"{
                "id": "3", "channel_id": "2", "content": "hi",
                "edited_timestamp": null, "some_new_field": [1, 2, 3],
                "author": { "id": "1", "username": "alice", "global_name": "Alice" }
            }"#,
        );
        let Event::MessageCreate { message, author } = decoded.event else {
            panic!("wrong event");
        };
        assert_eq!(message.content, "hi");
        assert_eq!(message.author_id, Snowflake(1));
        assert!(!message.edited);
        assert_eq!(author.display_name(), "Alice");
    }

    #[test]
    fn decodes_ready_with_session_members_and_read_state() {
        let decoded = decode_str(
            "READY",
            r#"{
                "user": { "id": "1", "username": "me" },
                "session_id": "abc",
                "resume_gateway_url": "wss://resume.example",
                "guilds": [{
                    "id": "10",
                    "properties": { "name": "My Server", "owner_id": "7" },
                    "roles": [{ "id": "10", "permissions": "1024" }],
                    "channels": [{ "id": "100", "type": 0, "name": "general", "position": 1 }]
                }],
                "merged_members": [[{ "user_id": "1", "roles": ["55"] }]],
                "private_channels": [{
                    "id": "200", "type": 1,
                    "recipients": [{ "id": "5", "username": "friend" }]
                }],
                "read_state": { "entries": [{ "id": "100", "last_message_id": "90", "mention_count": 1 }] },
                "user_guild_settings": { "entries": [{ "guild_id": "10", "muted": true }] }
            }"#,
        );

        let session = decoded.session.expect("READY carries a session");
        assert_eq!(session.session_id, "abc");

        let Event::Ready {
            me,
            guilds,
            channels,
            users,
            members,
            read_states,
            settings,
        } = decoded.event
        else {
            panic!("wrong event");
        };
        assert_eq!(me.username, "me");
        assert_eq!(guilds[0].name, "My Server");
        assert_eq!(guilds[0].owner_id, Some(Snowflake(7)));
        assert_eq!(guilds[0].channels, [Snowflake(100)]);
        assert_eq!(channels[0].guild_id, Some(Snowflake(10)));
        assert_eq!(channels[1].kind, ChannelKind::Dm);
        assert_eq!(channels[1].recipients, [Snowflake(5)]);
        assert_eq!(users[0].username, "friend");
        assert_eq!(members[0].roles, [Snowflake(55)]);
        assert_eq!(members[0].guild_id, Snowflake(10));
        assert_eq!(read_states[0].last_read, Some(Snowflake(90)));
        assert!(settings[0].muted);
    }

    #[test]
    fn decodes_reactions_acks_and_member_updates() {
        let add = decode_str(
            "MESSAGE_REACTION_ADD",
            r#"{ "user_id": "1", "channel_id": "2", "message_id": "3", "emoji": { "id": null, "name": "🔥" } }"#,
        );
        assert!(
            matches!(add.event, Event::ReactionAdd { emoji, .. } if emoji == ReactionEmoji::unicode("🔥"))
        );

        let clear = decode_str(
            "MESSAGE_REACTION_REMOVE_ALL",
            r#"{ "channel_id": "2", "message_id": "3" }"#,
        );
        assert!(matches!(
            clear.event,
            Event::ReactionClear { emoji: None, .. }
        ));

        let ack = decode_str(
            "MESSAGE_ACK",
            r#"{ "channel_id": "2", "message_id": "3", "version": 9 }"#,
        );
        assert!(matches!(
            ack.event,
            Event::MessageAck {
                message_id: Snowflake(3),
                ..
            }
        ));

        let member = decode_str(
            "GUILD_MEMBER_UPDATE",
            r#"{ "guild_id": "10", "user": { "id": "1", "username": "me" }, "roles": ["4"] }"#,
        );
        assert!(matches!(member.event, Event::GuildMemberUpdate(m) if m.roles == [Snowflake(4)]));
    }

    #[test]
    fn unknown_events_are_not_parsed() {
        let decoded = decode_str("STAGE_INSTANCE_CREATE", r#"{"anything": "goes"}"#);
        assert!(matches!(decoded.event, Event::Other(name) if name == "STAGE_INSTANCE_CREATE"));
    }

    #[test]
    fn member_list_sync() {
        let decoded = decode_str(
            "GUILD_MEMBER_LIST_UPDATE",
            r#"{ "guild_id": "10", "id": "everyone", "ops": [ { "op": "SYNC", "range": [0, 99], "items": [
                { "group": { "id": "online", "count": 1 } },
                { "member": { "user": { "id": "5", "username": "al" }, "roles": [], "nick": "Al",
                  "presence": { "status": "dnd", "activities": [ { "type": 0, "name": "Chess" } ] } } }
            ] } ] }"#,
        );
        let Event::MemberListUpdate { ops, .. } = decoded.event else {
            panic!("not a member list")
        };
        let [MemberListOp::Sync { items, .. }] = &ops[..] else {
            panic!("not one sync")
        };
        let MemberListEntry::Member {
            presence, member, ..
        } = &items[1]
        else {
            panic!("not a member")
        };
        assert_eq!(presence.activity.as_deref(), Some("Playing Chess"));
        assert_eq!(member.nick.as_deref(), Some("Al"));
    }

    #[test]
    fn messages_carry_the_authors_member() {
        let decoded = decode_str(
            "MESSAGE_CREATE",
            r#"{ "id": "1", "channel_id": "2", "guild_id": "10", "content": "hi",
                 "author": { "id": "5", "username": "al" }, "member": { "roles": ["7"], "nick": "Al" } }"#,
        );
        assert!(
            matches!(&decoded.extra[..], [Event::GuildMemberUpdate(m)] if m.user_id == Snowflake(5))
        );
    }
}
