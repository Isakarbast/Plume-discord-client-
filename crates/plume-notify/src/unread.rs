//! Which messages deserve a notification, and what it says.

use plume_core::Store;
use plume_core::models::{ChannelKind, Message, User};

const MAX_BODY: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

/// A notification for a new message, or `None` if it shouldn't make one.
///
/// Call this *before* the message is applied to the store. `watching` means
/// the user can already see that channel (it's open and the window has focus).
///
/// Rules, like Discord's defaults: DMs and mentions notify, unless they come
/// from us, the channel is muted, or the user is already looking at it.
pub fn notification_for(
    store: &Store,
    message: &Message,
    author: &User,
    watching: bool,
) -> Option<Notification> {
    let me = store.me()?.id;
    if message.author_id == me || watching || store.is_muted(message.channel_id) {
        return None;
    }
    let channel = store.channel(message.channel_id)?;
    let is_dm = matches!(channel.kind, ChannelKind::Dm | ChannelKind::GroupDm);
    if !is_dm {
        // The server's notification setting: all / @mentions / nothing.
        match store.notification_level(channel.guild_id) {
            0 => {}
            2 => return None,
            _ if !message.mentions_user(me) => return None,
            _ => {}
        }
    }

    let title = match channel.guild_id.and_then(|id| store.guild(id)) {
        Some(guild) => format!(
            "{} (#{}, {})",
            author.display_name(),
            channel.name.as_deref().unwrap_or("?"),
            guild.name
        ),
        None if channel.kind == ChannelKind::GroupDm => match channel.name.as_deref() {
            Some(name) if !name.is_empty() => format!("{} ({name})", author.display_name()),
            _ => author.display_name().to_owned(),
        },
        None => author.display_name().to_owned(),
    };
    Some(Notification {
        title,
        body: body_text(message),
    })
}

fn body_text(message: &Message) -> String {
    let text = message.content.trim();
    let text = if !text.is_empty() {
        text.to_owned()
    } else if let Some(file) = message.attachments.first() {
        format!("sent {}", file.filename)
    } else {
        "sent a message".to_owned()
    };
    if text.chars().count() <= MAX_BODY {
        return text;
    }
    let mut short: String = text.chars().take(MAX_BODY - 1).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;
    use plume_core::models::{Channel, GuildSettings};
    use plume_core::{Event, Profile, Snowflake};

    const ME: u64 = 1;
    const ALICE: u64 = 2;

    fn user(id: u64, name: &str) -> User {
        User {
            id: Snowflake(id),
            username: name.into(),
            global_name: None,
            avatar: None,
        }
    }

    fn channel(id: u64, guild: Option<u64>, kind: ChannelKind) -> Channel {
        Channel {
            id: Snowflake(id),
            guild_id: guild.map(Snowflake),
            name: guild.map(|_| "general".to_owned()),
            kind,
            parent_id: None,
            position: 0,
            recipients: vec![Snowflake(ALICE)],
            last_message_id: None,
            overwrites: Vec::new(),
            thread: None,
        }
    }

    fn message(channel: u64, author: u64, content: &str) -> Message {
        Message {
            id: Snowflake(99),
            channel_id: Snowflake(channel),
            author_id: Snowflake(author),
            content: content.into(),
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

    fn store() -> Store {
        let mut store = Store::new(Profile::FEATHER);
        store.apply(Event::Ready {
            me: user(ME, "me"),
            guilds: vec![plume_core::models::Guild {
                id: Snowflake(10),
                name: "Server".into(),
                icon: None,
                channels: vec![Snowflake(100)],
                owner_id: None,
                roles: Vec::new(),
                my_roles: Vec::new(),
                voice_states: Vec::new(),
                stickers: Vec::new(),
                emojis: Vec::new(),
            }],
            channels: vec![
                channel(100, Some(10), ChannelKind::Text),
                channel(200, None, ChannelKind::Dm),
            ],
            users: vec![user(ALICE, "alice")],
            members: Vec::new(),
            read_states: Vec::new(),
            settings: Vec::new(),
        });
        store
    }

    #[test]
    fn dms_notify_with_the_senders_name() {
        let n = notification_for(
            &store(),
            &message(200, ALICE, "hey"),
            &user(ALICE, "alice"),
            false,
        )
        .unwrap();
        assert_eq!(
            n,
            Notification {
                title: "alice".into(),
                body: "hey".into()
            }
        );
    }

    #[test]
    fn server_messages_notify_only_when_mentioned() {
        let store = store();
        let alice = user(ALICE, "alice");
        assert!(notification_for(&store, &message(100, ALICE, "hi all"), &alice, false).is_none());

        let mut ping = message(100, ALICE, "hi <@1>");
        ping.mentions = vec![Snowflake(ME)];
        let n = notification_for(&store, &ping, &alice, false).unwrap();
        assert_eq!(n.title, "alice (#general, Server)");
    }

    #[test]
    fn no_notification_when_watching_muted_or_from_me() {
        let mut store = store();
        let alice = user(ALICE, "alice");
        assert!(notification_for(&store, &message(200, ALICE, "hey"), &alice, true).is_none());
        assert!(
            notification_for(&store, &message(200, ME, "hey"), &user(ME, "me"), false).is_none()
        );

        store.apply(Event::GuildSettingsUpdate(GuildSettings {
            guild_id: None,
            muted: false,
            muted_channels: vec![Snowflake(200)],
            notifications: 1,
        }));
        assert!(notification_for(&store, &message(200, ALICE, "hey"), &alice, false).is_none());
    }

    #[test]
    fn long_or_empty_bodies() {
        let long = "x".repeat(500);
        let n = notification_for(
            &store(),
            &message(200, ALICE, &long),
            &user(ALICE, "alice"),
            false,
        )
        .unwrap();
        assert_eq!(n.body.chars().count(), MAX_BODY);
        assert!(n.body.ends_with('…'));
    }
}
