//! Made-up servers and messages for `plume --demo`: try the UI (or take
//! screenshots) without logging in or touching the network.

use plume_core::events::{MemberListEntry, MemberListOp};
use plume_core::models::{
    Attachment, Channel, ChannelKind, Embed, Guild, Member, Message, Presence, Reaction,
    ReactionEmoji, ReadState, Relationship, RelationshipKind, Role, Status, User, VoiceState,
};
use plume_core::{Event, Permissions, Snowflake};

/// The channel the demo opens first.
pub const GENERAL: Snowflake = Snowflake(101);

const ME: u64 = 1;
const ALICE: u64 = 2;
const BOB: u64 = 3;
const CAROL: u64 = 4;
/// The demo conversation starts an hour ago (fixed for the whole run).
fn start_ms() -> u64 {
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *START.get_or_init(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        now.saturating_sub(60 * 60 * 1000)
    })
}

fn user(id: u64, name: &str) -> User {
    User {
        id: Snowflake(id),
        username: name.to_lowercase(),
        global_name: Some(name.to_owned()),
        avatar: None,
    }
}

fn channel(
    id: u64,
    guild: Option<u64>,
    name: Option<&str>,
    kind: ChannelKind,
    parent: Option<u64>,
    position: i32,
) -> Channel {
    Channel {
        id: Snowflake(id),
        guild_id: guild.map(Snowflake),
        name: name.map(str::to_owned),
        kind,
        parent_id: parent.map(Snowflake),
        position,
        recipients: Vec::new(),
        last_message_id: None,
        overwrites: Vec::new(),
        thread: None,
    }
}

fn guild(id: u64, name: &str, channels: &[u64]) -> Guild {
    Guild {
        id: Snowflake(id),
        name: name.to_owned(),
        icon: None,
        channels: channels.iter().map(|&c| Snowflake(c)).collect(),
        owner_id: Some(Snowflake(999)),
        roles: vec![
            Role {
                id: Snowflake(id * 100 + 1),
                name: "Moderator".into(),
                color: 0xf47b67,
                position: 2,
                hoist: true,
                ..Default::default()
            },
            Role {
                id: Snowflake(id * 100 + 2),
                name: "Rustacean".into(),
                color: 0x57f287,
                position: 1,
                ..Default::default()
            },
            Role {
                id: Snowflake(id),
                permissions: Permissions::VIEW_CHANNEL.0
                    | Permissions::SEND_MESSAGES.0
                    | Permissions::ADD_REACTIONS.0
                    | Permissions::CONNECT.0,
                ..Default::default()
            },
        ],
        my_roles: Vec::new(),
        voice_states: Vec::new(),
        emojis: Vec::new(),
        stickers: Vec::new(),
    }
}

fn voice(user: u64, channel: u64, muted: bool) -> VoiceState {
    VoiceState {
        guild_id: Some(Snowflake(10)),
        channel_id: Some(Snowflake(channel)),
        user_id: Snowflake(user),
        session_id: String::new(),
        self_mute: muted,
        self_deaf: false,
        mute: false,
        deaf: false,
        user: None,
    }
}

pub fn ready() -> Event {
    let mut dm = channel(500, None, None, ChannelKind::Dm, None, 0);
    dm.recipients = vec![Snowflake(ALICE)];
    dm.last_message_id = Some(message_id(600));
    let mut group = channel(
        501,
        None,
        Some("Weekend plans"),
        ChannelKind::GroupDm,
        None,
        0,
    );
    group.recipients = vec![Snowflake(BOB), Snowflake(CAROL)];
    let mut help = channel(102, Some(10), Some("help"), ChannelKind::Text, Some(100), 2);
    help.last_message_id = Some(message_id(900));

    let mut hangout = guild(10, "Rust Hangout", &[100, 101, 102, 103, 104, 105]);
    hangout.voice_states = vec![voice(ALICE, 105, false), voice(BOB, 105, true)];

    Event::Ready {
        me: user(ME, "You"),
        guilds: vec![hangout, guild(20, "Plume Dev", &[200, 201])],
        channels: vec![
            channel(
                100,
                Some(10),
                Some("Text Channels"),
                ChannelKind::Category,
                None,
                0,
            ),
            channel(
                101,
                Some(10),
                Some("general"),
                ChannelKind::Text,
                Some(100),
                1,
            ),
            help,
            channel(
                103,
                Some(10),
                Some("showcase"),
                ChannelKind::Text,
                Some(100),
                3,
            ),
            channel(
                104,
                Some(10),
                Some("Voice Channels"),
                ChannelKind::Category,
                None,
                1,
            ),
            channel(
                105,
                Some(10),
                Some("Lounge"),
                ChannelKind::Voice,
                Some(104),
                0,
            ),
            channel(
                200,
                Some(20),
                Some("announcements"),
                ChannelKind::Text,
                None,
                0,
            ),
            channel(201, Some(20), Some("dev"), ChannelKind::Text, None, 1),
            dm,
            group,
        ],
        users: vec![user(ALICE, "Alice"), user(BOB, "Bob"), user(CAROL, "Carol")],
        members: Vec::new(),
        read_states: vec![ReadState {
            channel_id: Snowflake(102),
            last_read: Some(message_id(10)),
            mentions: 2,
        }],
        settings: Vec::new(),
    }
}

/// Friends, statuses and a member list, sent right after `ready()`.
pub fn extras() -> Vec<Event> {
    let presence = |status, activity: Option<&str>| Presence {
        status,
        activity: activity.map(str::to_owned),
    };
    let member = |id: u64, roles: &[u64], nick: Option<&str>| Member {
        guild_id: Snowflake(10),
        user_id: Snowflake(id),
        roles: roles.iter().map(|&r| Snowflake(r)).collect(),
        nick: nick.map(str::to_owned),
    };
    let entry = |id: u64, name: &str, roles: &[u64], nick: Option<&str>, p: Presence| {
        MemberListEntry::Member {
            user: user(id, name),
            member: member(id, roles, nick),
            presence: p,
        }
    };
    let group = |id: &str, count| MemberListEntry::Group {
        id: id.to_owned(),
        count,
    };
    vec![
        Event::Relationships(vec![
            Relationship {
                user_id: Snowflake(ALICE),
                kind: RelationshipKind::Friend,
                nickname: None,
            },
            Relationship {
                user_id: Snowflake(BOB),
                kind: RelationshipKind::Friend,
                nickname: None,
            },
            Relationship {
                user_id: Snowflake(CAROL),
                kind: RelationshipKind::Incoming,
                nickname: None,
            },
        ]),
        Event::Presences(vec![
            (Snowflake(ME), presence(Status::Online, None)),
            (
                Snowflake(ALICE),
                presence(Status::Online, Some("Playing Factorio")),
            ),
            (
                Snowflake(BOB),
                presence(Status::Idle, Some("Listening to Spotify")),
            ),
            (
                Snowflake(CAROL),
                presence(Status::Dnd, Some("busy shipping 🚀")),
            ),
        ]),
        Event::MemberListUpdate {
            guild_id: Snowflake(10),
            list_id: "everyone".into(),
            ops: vec![MemberListOp::Sync {
                start: 0,
                items: vec![
                    group("1001", 1),
                    entry(
                        ALICE,
                        "Alice",
                        &[1001],
                        Some("Alice 🦀"),
                        presence(Status::Online, Some("Playing Factorio")),
                    ),
                    group("online", 2),
                    entry(
                        BOB,
                        "Bob",
                        &[1002],
                        None,
                        presence(Status::Idle, Some("Listening to Spotify")),
                    ),
                    entry(ME, "You", &[], None, presence(Status::Online, None)),
                    group("offline", 1),
                    entry(
                        CAROL,
                        "Carol",
                        &[1002],
                        None,
                        presence(Status::Offline, None),
                    ),
                ],
            }],
        },
    ]
}

fn message_id(seconds: u64) -> Snowflake {
    Snowflake::from_timestamp_ms(start_ms() + seconds * 1000)
}

fn message(seconds: u64, channel: u64, author: u64, content: &str) -> Message {
    Message {
        id: message_id(seconds),
        channel_id: Snowflake(channel),
        author_id: Snowflake(author),
        content: content.to_owned(),
        edited: false,
        reply_to: None,
        mentions: Vec::new(),
        mention_everyone: false,
        attachments: Vec::new(),
        embeds: Vec::new(),
        reactions: Vec::new(),
        stickers: Vec::new(),
        pinned: false,
    }
}

fn author(id: u64) -> User {
    match id {
        ALICE => user(ALICE, "Alice"),
        BOB => user(BOB, "Bob"),
        CAROL => user(CAROL, "Carol"),
        _ => user(ME, "You"),
    }
}

/// Messages for a channel, as if fetched from Discord.
pub fn history(channel_id: Snowflake) -> Vec<(Message, User)> {
    let channel = channel_id.0;
    let messages = match channel {
        101 => {
            let mut link = message(
                120,
                channel,
                BOB,
                "This explains it well: https://en.wikipedia.org/wiki/Rust_(programming_language)",
            );
            link.embeds.push(Embed {
                provider: Some("Wikipedia".into()),
                title: Some("Rust (programming language)".into()),
                url: Some("https://en.wikipedia.org/wiki/Rust_(programming_language)".into()),
                description: Some("Rust is a general-purpose programming language emphasizing performance, type safety, and concurrency.".into()),
                ..Embed::default()
            });
            link.reactions = vec![
                Reaction {
                    emoji: ReactionEmoji::unicode("🦀"),
                    count: 4,
                    me: true,
                },
                Reaction {
                    emoji: ReactionEmoji::unicode("👍"),
                    count: 2,
                    me: false,
                },
            ];
            let mut reply = message(
                200,
                channel,
                ALICE,
                "Thanks <@1>! That's exactly what I needed",
            );
            reply.reply_to = Some(message_id(150));
            reply.mentions = vec![Snowflake(ME)];
            let mut file = message(260, channel, CAROL, "Here are my notes");
            file.attachments.push(Attachment {
                filename: "borrow-checker-notes.pdf".into(),
                url: "https://example.invalid/notes.pdf".into(),
                size: 182_000,
                width: None,
                height: None,
            });
            let mut edited = message(
                320,
                channel,
                ME,
                "I *think* the trick is to clone less and borrow more",
            );
            edited.edited = true;
            vec![
                message(
                    0,
                    channel,
                    ALICE,
                    "Hey everyone! Has anyone figured out **lifetimes** yet?",
                ),
                message(
                    30,
                    channel,
                    ALICE,
                    "I keep getting `borrowed value does not live long enough`",
                ),
                link,
                message(150, channel, ME, "Show us the code and we'll take a look"),
                reply,
                message(
                    230,
                    channel,
                    ALICE,
                    "```rust\nfn longest<'a>(x: &'a str, y: &'a str) -> &'a str {\n    if x.len() > y.len() { x } else { y }\n}\n```",
                ),
                file,
                edited,
                message(
                    400,
                    channel,
                    BOB,
                    "> clone less and borrow more\nwords to live by ||(until you need an Arc)||",
                ),
            ]
        }
        102 => vec![
            message(
                10,
                channel,
                BOB,
                "Is there a way to pin a dependency version?",
            ),
            message(900, channel, CAROL, "@everyone the help desk is open again"),
        ],
        500 => vec![
            message(500, channel, ALICE, "are we still on for saturday?"),
            message(600, channel, ME, "yep! 🎉"),
        ],
        _ => vec![message(0, channel, BOB, "Nothing much here yet.")],
    };
    messages
        .into_iter()
        .map(|m| {
            let a = author(m.author_id.0);
            (m, a)
        })
        .collect()
}
