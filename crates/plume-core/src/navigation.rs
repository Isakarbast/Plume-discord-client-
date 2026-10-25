//! How servers, channels and DMs are ordered and named in a sidebar.
//! Shared by every frontend so they all agree.

use crate::ids::Snowflake;
use crate::models::{Channel, ChannelKind, Guild};
use crate::store::Store;

/// One row of a server's channel list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelEntry {
    Category(Snowflake),
    Channel(Snowflake),
    Voice(Snowflake),
    /// A joined thread, listed under its channel.
    Thread(Snowflake),
}

fn is_listed(kind: ChannelKind) -> bool {
    matches!(kind, ChannelKind::Text | ChannelKind::Voice)
}

fn entry(channel: &Channel) -> ChannelEntry {
    if channel.kind == ChannelKind::Voice {
        ChannelEntry::Voice(channel.id)
    } else {
        ChannelEntry::Channel(channel.id)
    }
}

pub fn is_dm(channel: &Channel) -> bool {
    matches!(channel.kind, ChannelKind::Dm | ChannelKind::GroupDm)
}

/// DMs, most recently active first.
pub fn sorted_dms(store: &Store) -> Vec<Snowflake> {
    let mut dms: Vec<&Channel> = store.channels().filter(|c| is_dm(c)).collect();
    dms.sort_by_key(|c| std::cmp::Reverse(c.last_message_id.unwrap_or(c.id)));
    dms.into_iter().map(|c| c.id).collect()
}

/// Servers alphabetically. (Discord's own folder order lives in a settings
/// blob we don't decode yet.)
pub fn sorted_guilds(store: &Store) -> Vec<&Guild> {
    let mut guilds: Vec<&Guild> = store.guilds().collect();
    guilds.sort_by_cached_key(|g| (g.name.to_lowercase(), g.id));
    guilds
}

/// A server's channels in Discord's order: uncategorised ones first, then
/// each category followed by its channels (text before voice). Channels you can't view are
/// left out, and so are categories with nothing visible in them.
pub fn guild_channels(store: &Store, guild: &Guild) -> Vec<ChannelEntry> {
    let channels: Vec<&Channel> = guild
        .channels
        .iter()
        .filter_map(|&id| store.channel(id))
        .filter(|c| c.kind == ChannelKind::Category || store.can_view(c.id))
        .collect();
    let order = |c: &&Channel| (c.kind == ChannelKind::Voice, c.position, c.id);

    let mut top: Vec<&Channel> = channels
        .iter()
        .copied()
        .filter(|c| is_listed(c.kind) && c.parent_id.is_none())
        .collect();
    top.sort_by_key(order);

    let mut categories: Vec<&Channel> = channels
        .iter()
        .copied()
        .filter(|c| c.kind == ChannelKind::Category)
        .collect();
    categories.sort_by_key(order);

    let with_threads = |list: &mut Vec<ChannelEntry>, c: &Channel| {
        list.push(entry(c));
        for thread in store.threads(c.id) {
            if thread.thread.is_some_and(|t| t.joined) {
                list.push(ChannelEntry::Thread(thread.id));
            }
        }
    };
    let mut entries: Vec<ChannelEntry> = Vec::new();
    for c in &top {
        with_threads(&mut entries, c);
    }
    for category in categories {
        let mut children: Vec<&Channel> = channels
            .iter()
            .copied()
            .filter(|c| is_listed(c.kind) && c.parent_id == Some(category.id))
            .collect();
        if children.is_empty() {
            continue;
        }
        children.sort_by_key(order);
        entries.push(ChannelEntry::Category(category.id));
        for c in &children {
            with_threads(&mut entries, c);
        }
    }
    entries
}

/// "Alice" for a DM, the group's name, or its members for an unnamed group.
pub fn dm_name(store: &Store, channel: &Channel) -> String {
    if let Some(name) = channel.name.as_deref().filter(|n| !n.is_empty()) {
        return name.to_owned();
    }
    let names: Vec<&str> = channel
        .recipients
        .iter()
        .filter_map(|&id| store.user(id))
        .map(|u| u.display_name())
        .collect();
    if names.is_empty() {
        "(unknown)".to_owned()
    } else {
        names.join(", ")
    }
}

/// A short name for any channel: "#general" or "@Alice".
pub fn channel_label(store: &Store, channel_id: Snowflake) -> String {
    match store.channel(channel_id) {
        Some(channel) if is_dm(channel) => format!("@{}", dm_name(store, channel)),
        Some(channel) if channel.kind == ChannelKind::Thread => {
            format!("🧵{}", channel.name.as_deref().unwrap_or("?"))
        }
        Some(channel) if channel.kind == ChannelKind::Voice => {
            format!("🔊{}", channel.name.as_deref().unwrap_or("?"))
        }
        Some(channel) => format!("#{}", channel.name.as_deref().unwrap_or("?")),
        None => "#unknown".to_owned(),
    }
}
