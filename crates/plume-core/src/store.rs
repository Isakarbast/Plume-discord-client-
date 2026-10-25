use std::collections::{HashMap, HashSet, VecDeque};

use crate::events::{Event, MemberListEntry, MemberListOp};
use crate::ids::Snowflake;
use crate::models::{
    Channel, ChannelKind, Guild, GuildSettings, Member, MemberListItem, Message, Presence,
    Reaction, ReactionEmoji, ReadState, Relationship, User, VoiceState,
};
use crate::permissions::{Permissions, channel_permissions};
use crate::profile::Profile;

/// Unread state for one channel (or a whole server).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Unread {
    /// There are messages newer than the last one we read.
    pub unread: bool,
    pub mentions: u32,
}

/// The single source of truth for everything the client knows.
///
/// Feed it every [`Event`] with [`Store::apply`]; the UI reads from it.
///
/// Messages are only kept for the few channels you opened most recently
/// (how many depends on the [`Profile`]). Other channels just remember how
/// far you've read, which keeps memory flat no matter how many busy servers
/// you're in.
#[derive(Debug, Default)]
pub struct Store {
    profile: Profile,
    me: Option<Snowflake>,
    users: HashMap<Snowflake, User>,
    guilds: HashMap<Snowflake, Guild>,
    channels: HashMap<Snowflake, Channel>,
    messages: HashMap<Snowflake, VecDeque<Message>>,
    /// Channels we keep messages for, most recently opened first.
    recent_channels: VecDeque<Snowflake>,
    /// Channels whose latest history has been fetched since they were opened.
    history_loaded: HashSet<Snowflake>,
    /// Channels where we've scrolled back to the very first message.
    history_complete: HashSet<Snowflake>,
    /// The channel on screen right now.
    active_channel: Option<Snowflake>,
    /// The last message we've read in each channel.
    read_up_to: HashMap<Snowflake, Snowflake>,
    mentions: HashMap<Snowflake, u32>,
    /// Mute settings per server; `None` holds the DM settings.
    settings: HashMap<Option<Snowflake>, GuildSettings>,
    /// Who is in voice, keyed by (server, user). DM calls use `None`.
    voice: HashMap<(Option<Snowflake>, Snowflake), VoiceState>,
    /// Nicknames and roles of people we've seen, per (server, user).
    members: HashMap<(Snowflake, Snowflake), Member>,
    presences: HashMap<Snowflake, Presence>,
    relationships: HashMap<Snowflake, Relationship>,
    /// The member list Discord sent for each server's open channel.
    member_lists: HashMap<Snowflake, MemberList>,
}

#[derive(Debug, Default)]
struct MemberList {
    id: String,
    items: Vec<MemberListItem>,
}

impl Store {
    pub fn new(profile: Profile) -> Self {
        Store {
            profile,
            ..Store::default()
        }
    }

    pub fn profile(&self) -> Profile {
        self.profile
    }

    pub fn me(&self) -> Option<&User> {
        self.me.and_then(|id| self.users.get(&id))
    }

    pub fn user(&self, id: Snowflake) -> Option<&User> {
        self.users.get(&id)
    }

    pub fn guild(&self, id: Snowflake) -> Option<&Guild> {
        self.guilds.get(&id)
    }

    pub fn guilds(&self) -> impl Iterator<Item = &Guild> {
        self.guilds.values()
    }

    pub fn channel(&self, id: Snowflake) -> Option<&Channel> {
        self.channels.get(&id)
    }

    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.values()
    }

    /// Cached messages for a channel, oldest first.
    pub fn messages(&self, channel_id: Snowflake) -> impl Iterator<Item = &Message> {
        self.messages.get(&channel_id).into_iter().flatten()
    }

    pub fn message(&self, channel_id: Snowflake, id: Snowflake) -> Option<&Message> {
        self.messages(channel_id).find(|m| m.id == id)
    }

    pub fn active_channel(&self) -> Option<Snowflake> {
        self.active_channel
    }

    pub fn users(&self) -> impl Iterator<Item = &User> {
        self.users.values()
    }

    /// Channels that currently hold messages, most recently opened first.
    pub fn cached_channels(&self) -> impl Iterator<Item = Snowflake> + '_ {
        self.recent_channels.iter().copied()
    }

    pub fn member(&self, guild_id: Snowflake, user_id: Snowflake) -> Option<&Member> {
        self.members.get(&(guild_id, user_id))
    }

    /// Everyone we know of in a server (from the member list and messages).
    pub fn guild_members(&self, guild_id: Snowflake) -> impl Iterator<Item = &Member> {
        self.members
            .iter()
            .filter(move |((g, _), _)| *g == guild_id)
            .map(|(_, m)| m)
    }

    /// Their nickname in this server, else their display name.
    pub fn display_name_in(&self, guild_id: Option<Snowflake>, user_id: Snowflake) -> String {
        let nick = guild_id
            .and_then(|g| self.member(g, user_id))
            .and_then(|m| m.nick.clone());
        nick.or_else(|| self.user(user_id).map(|u| u.display_name().to_owned()))
            .unwrap_or_else(|| "Unknown user".to_owned())
    }

    /// The colour of their highest coloured role (`0xRRGGBB`).
    pub fn role_color(&self, guild_id: Snowflake, user_id: Snowflake) -> Option<u32> {
        let guild = self.guilds.get(&guild_id)?;
        let member = self.member(guild_id, user_id)?;
        guild
            .roles
            .iter()
            .filter(|r| r.color != 0 && member.roles.contains(&r.id))
            .max_by_key(|r| r.position)
            .map(|r| r.color)
    }

    pub fn presence(&self, user_id: Snowflake) -> Presence {
        self.presences.get(&user_id).cloned().unwrap_or_default()
    }

    pub fn relationships(&self) -> impl Iterator<Item = &Relationship> {
        self.relationships.values()
    }

    pub fn relationship(&self, user_id: Snowflake) -> Option<&Relationship> {
        self.relationships.get(&user_id)
    }

    /// A server's member list (headings and people), as last sent.
    pub fn member_list(&self, guild_id: Snowflake) -> &[MemberListItem] {
        self.member_lists
            .get(&guild_id)
            .map_or(&[], |list| list.items.as_slice())
    }

    /// Everyone in a voice channel, in a stable order.
    pub fn voice_members(&self, channel_id: Snowflake) -> Vec<&VoiceState> {
        let mut members: Vec<&VoiceState> = self
            .voice
            .values()
            .filter(|v| v.channel_id == Some(channel_id))
            .collect();
        members.sort_by_key(|v| v.user_id);
        members
    }

    /// The voice channel we're in, according to Discord.
    pub fn my_voice_state(&self) -> Option<&VoiceState> {
        let me = self.me?;
        self.voice.values().find(|v| v.user_id == me)
    }

    /// After pinning or unpinning a message ourselves.
    pub fn set_pinned(&mut self, channel_id: Snowflake, message_id: Snowflake, pinned: bool) {
        if let Some(message) = self
            .messages
            .get_mut(&channel_id)
            .and_then(|list| list.iter_mut().find(|m| m.id == message_id))
        {
            message.pinned = pinned;
        }
    }

    /// Active threads under a channel, newest first.
    pub fn threads(&self, parent_id: Snowflake) -> Vec<&Channel> {
        let mut threads: Vec<&Channel> = self
            .channels
            .values()
            .filter(|c| {
                c.kind == ChannelKind::Thread
                    && c.parent_id == Some(parent_id)
                    && c.thread.is_some_and(|t| !t.archived)
            })
            .collect();
        threads.sort_by_key(|c| std::cmp::Reverse(c.last_message_id.unwrap_or(c.id)));
        threads
    }

    /// The last message we'd read in a channel.
    pub fn last_read(&self, channel_id: Snowflake) -> Option<Snowflake> {
        self.read_up_to.get(&channel_id).copied()
    }

    /// How far we've read everywhere, for saving to disk.
    pub fn read_states(&self) -> Vec<ReadState> {
        self.read_up_to
            .iter()
            .map(|(&channel_id, &last_read)| ReadState {
                channel_id,
                last_read: Some(last_read),
                mentions: self.mentions.get(&channel_id).copied().unwrap_or(0),
            })
            .collect()
    }

    pub fn settings(&self) -> impl Iterator<Item = &GuildSettings> {
        self.settings.values()
    }

    // ---- Permissions and mutes ----------------------------------------------

    /// Our permissions in a channel. DMs allow everything.
    pub fn permissions(&self, channel_id: Snowflake) -> Permissions {
        let Some(mut channel) = self.channels.get(&channel_id) else {
            return Permissions::default();
        };
        // Threads use their parent channel's permissions.
        if channel.kind == ChannelKind::Thread
            && let Some(parent) = channel.parent_id.and_then(|p| self.channels.get(&p))
        {
            channel = parent;
        }
        let (Some(guild), Some(me)) = (channel.guild_id.and_then(|g| self.guilds.get(&g)), self.me)
        else {
            return Permissions::ALL;
        };
        if guild.roles.is_empty() {
            return Permissions::ALL; // no role data yet: don't hide anything
        }
        channel_permissions(guild, channel, me)
    }

    pub fn can_view(&self, channel_id: Snowflake) -> bool {
        self.permissions(channel_id)
            .contains(Permissions::VIEW_CHANNEL)
    }

    pub fn can_send(&self, channel_id: Snowflake) -> bool {
        self.permissions(channel_id)
            .contains(Permissions::SEND_MESSAGES)
    }

    /// Whether we may join this voice channel.
    pub fn can_connect(&self, channel_id: Snowflake) -> bool {
        self.permissions(channel_id).contains(Permissions::CONNECT)
    }

    /// Muted directly, through its category, or through its whole server.
    pub fn is_muted(&self, channel_id: Snowflake) -> bool {
        let Some(channel) = self.channels.get(&channel_id) else {
            return false;
        };
        let Some(settings) = self.settings.get(&channel.guild_id) else {
            return false;
        };
        let muted = |id: Snowflake| settings.muted_channels.contains(&id);
        (channel.guild_id.is_some() && settings.muted)
            || muted(channel_id)
            || channel.parent_id.is_some_and(muted)
    }

    /// 0 all messages, 1 only mentions (the default), 2 nothing.
    pub fn notification_level(&self, guild_id: Option<Snowflake>) -> u8 {
        self.settings.get(&guild_id).map_or(1, |s| s.notifications)
    }

    /// What we may do in a whole server (kick, ban, …).
    pub fn guild_permissions(&self, guild_id: Snowflake) -> Permissions {
        match (self.guilds.get(&guild_id), self.me) {
            (Some(guild), Some(me)) if !guild.roles.is_empty() => {
                crate::permissions::guild_permissions(guild, me)
            }
            _ => Permissions::default(),
        }
    }

    /// The whole server is muted.
    pub fn is_guild_muted(&self, guild_id: Snowflake) -> bool {
        self.settings.get(&Some(guild_id)).is_some_and(|s| s.muted)
    }

    // ---- Unread state -------------------------------------------------------

    pub fn unread(&self, channel_id: Snowflake) -> Unread {
        if Some(channel_id) == self.active_channel || !self.can_view(channel_id) {
            return Unread::default();
        }
        let mentions = self.mentions.get(&channel_id).copied().unwrap_or(0);
        let newer = match (
            self.channels
                .get(&channel_id)
                .and_then(|c| c.last_message_id),
            self.read_up_to.get(&channel_id),
        ) {
            (Some(last), Some(read)) => last > *read,
            (Some(_), None) => true,
            _ => false,
        };
        Unread {
            unread: newer && !self.is_muted(channel_id),
            mentions,
        }
    }

    /// A server is unread if any channel you can see is; mentions add up.
    pub fn guild_unread(&self, guild_id: Snowflake) -> Unread {
        let mut total = Unread::default();
        if let Some(guild) = self.guilds.get(&guild_id) {
            for &channel_id in &guild.channels {
                let unread = self.unread(channel_id);
                total.unread |= unread.unread;
                total.mentions += unread.mentions;
            }
        }
        if self.is_guild_muted(guild_id) {
            total.unread = false;
        }
        total
    }

    /// Marks a channel read up to its newest message. Returns that message's
    /// ID if it wasn't read before, so the caller can tell Discord (ack).
    pub fn mark_read(&mut self, channel_id: Snowflake) -> Option<Snowflake> {
        self.mentions.remove(&channel_id);
        let last = self.channels.get(&channel_id)?.last_message_id?;
        let previous = self.read_up_to.insert(channel_id, last);
        (previous != Some(last)).then_some(last)
    }

    // ---- Opening channels and history ---------------------------------------

    /// Shows a channel and starts keeping its messages. Returns `true` if its
    /// history still needs to be fetched.
    pub fn open_channel(&mut self, channel_id: Snowflake) -> bool {
        // The channel we're leaving drops any extra scrollback.
        if let Some(previous) = self.active_channel.filter(|&c| c != channel_id) {
            self.trim(previous, self.profile.messages_per_channel);
        }
        self.active_channel = Some(channel_id);
        self.touch_channel(channel_id);
        self.messages.entry(channel_id).or_default();
        !self.history_loaded.contains(&channel_id)
    }

    /// Shows messages saved on disk while the real history loads. Unlike
    /// [`Store::insert_history`], the channel still counts as not loaded.
    pub fn insert_cached(&mut self, channel_id: Snowflake, history: Vec<(Message, User)>) {
        let limit = self.limit_for(channel_id);
        self.merge(channel_id, history, limit);
    }

    /// Merges the latest messages (in any order) into a channel.
    pub fn insert_history(&mut self, channel_id: Snowflake, history: Vec<(Message, User)>) {
        self.history_loaded.insert(channel_id);
        let limit = self.limit_for(channel_id);
        self.merge(channel_id, history, limit);
    }

    /// Adds older messages from scrolling back. `reached_start` means there
    /// are none older than these.
    pub fn insert_older(
        &mut self,
        channel_id: Snowflake,
        history: Vec<(Message, User)>,
        reached_start: bool,
    ) {
        if reached_start {
            self.history_complete.insert(channel_id);
        }
        self.merge(channel_id, history, self.profile.max_scrollback);
    }

    /// Whether scrolling up could load more.
    pub fn has_older_messages(&self, channel_id: Snowflake) -> bool {
        let at_limit = self
            .messages
            .get(&channel_id)
            .is_some_and(|m| m.len() >= self.profile.max_scrollback);
        !self.history_complete.contains(&channel_id) && !at_limit
    }

    pub fn oldest_message(&self, channel_id: Snowflake) -> Option<Snowflake> {
        self.messages.get(&channel_id)?.front().map(|m| m.id)
    }

    fn merge(&mut self, channel_id: Snowflake, history: Vec<(Message, User)>, limit: usize) {
        // The channel may have been evicted while the request was running.
        if !self.recent_channels.contains(&channel_id) {
            return;
        }
        let mut merged: Vec<Message> = self.messages.remove(&channel_id).unwrap_or_default().into();
        for (message, author) in history {
            self.users.insert(author.id, author);
            merged.push(message);
        }
        merged.sort_by_key(|m| m.id);
        merged.dedup_by_key(|m| m.id);
        let excess = merged.len().saturating_sub(limit);
        self.messages
            .insert(channel_id, merged.into_iter().skip(excess).collect());
    }

    /// The open channel may keep its scrollback; others keep the normal amount.
    fn limit_for(&self, channel_id: Snowflake) -> usize {
        if self.active_channel == Some(channel_id) {
            let current = self.messages.get(&channel_id).map_or(0, VecDeque::len);
            current.clamp(
                self.profile.messages_per_channel,
                self.profile.max_scrollback,
            )
        } else {
            self.profile.messages_per_channel
        }
    }

    fn trim(&mut self, channel_id: Snowflake, limit: usize) {
        if let Some(list) = self.messages.get_mut(&channel_id)
            && list.len() > limit
        {
            list.drain(..list.len() - limit);
            self.history_complete.remove(&channel_id);
        }
    }

    // ---- Events -------------------------------------------------------------

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Ready {
                me,
                guilds,
                channels,
                users,
                members,
                read_states,
                settings,
            } => {
                // A fresh login is the whole truth: forget servers we've left
                // since a cached snapshot was loaded. Messages stay.
                self.guilds.clear();
                self.channels.clear();
                self.read_up_to.clear();
                self.mentions.clear();
                self.settings.clear();
                self.voice.clear();
                self.members.clear();
                self.member_lists.clear();
                self.me = Some(me.id);
                self.insert_user(me);
                for user in users {
                    self.insert_user(user);
                }
                for guild in guilds {
                    self.insert_guild(guild);
                }
                for state in read_states {
                    if let Some(last_read) = state.last_read {
                        self.read_up_to.insert(state.channel_id, last_read);
                    }
                    if state.mentions > 0 {
                        self.mentions.insert(state.channel_id, state.mentions);
                    }
                }
                for channel in channels {
                    self.insert_channel(channel);
                }
                self.apply_members(members);
                for setting in settings {
                    self.settings.insert(setting.guild_id, setting);
                }
            }
            Event::GuildCreate {
                guild,
                channels,
                members,
            } => {
                self.insert_guild(guild);
                for channel in channels {
                    self.insert_channel(channel);
                }
                self.apply_members(members);
            }
            Event::GuildDelete { id, unavailable } => {
                // During an outage we keep the data so the UI doesn't go blank.
                if !unavailable {
                    self.remove_guild(id);
                }
            }
            Event::GuildRoleUpsert { guild_id, role } => {
                if let Some(guild) = self.guilds.get_mut(&guild_id) {
                    match guild.roles.iter_mut().find(|r| r.id == role.id) {
                        Some(existing) => *existing = role,
                        None => guild.roles.push(role),
                    }
                }
            }
            Event::GuildRoleDelete { guild_id, role_id } => {
                if let Some(guild) = self.guilds.get_mut(&guild_id) {
                    guild.roles.retain(|r| r.id != role_id);
                    guild.my_roles.retain(|&r| r != role_id);
                }
            }
            Event::GuildMemberUpdate(member) => self.apply_members(vec![member]),
            Event::GuildSettingsUpdate(settings) => {
                self.settings.insert(settings.guild_id, settings);
            }
            Event::ChannelUpsert(channel) => {
                if let Some(guild) = channel.guild_id.and_then(|id| self.guilds.get_mut(&id))
                    && !guild.channels.contains(&channel.id)
                {
                    guild.channels.push(channel.id);
                }
                self.insert_channel(channel);
            }
            Event::ChannelDelete { id } => self.remove_channel(id),
            Event::MessageCreate { message, author } => {
                self.insert_user(author);
                self.push_message(message);
            }
            Event::MessageUpdate {
                id,
                channel_id,
                content,
                embeds,
                edited,
            } => {
                if let Some(message) = self.find_message_mut(channel_id, id) {
                    if let Some(content) = content {
                        message.content = content;
                    }
                    if let Some(embeds) = embeds {
                        message.embeds = embeds;
                    }
                    message.edited |= edited;
                }
            }
            Event::MessageDelete { id, channel_id } => {
                if let Some(list) = self.messages.get_mut(&channel_id) {
                    list.retain(|m| m.id != id);
                }
            }
            Event::ReactionAdd {
                channel_id,
                message_id,
                user_id,
                emoji,
            } => {
                let is_me = Some(user_id) == self.me;
                if let Some(message) = self.find_message_mut(channel_id, message_id) {
                    add_reaction(message, emoji, is_me);
                }
            }
            Event::ReactionRemove {
                channel_id,
                message_id,
                user_id,
                emoji,
            } => {
                let is_me = Some(user_id) == self.me;
                if let Some(message) = self.find_message_mut(channel_id, message_id) {
                    remove_reaction(message, &emoji, is_me);
                }
            }
            Event::ReactionClear {
                channel_id,
                message_id,
                emoji,
            } => {
                if let Some(message) = self.find_message_mut(channel_id, message_id) {
                    match emoji {
                        Some(emoji) => message.reactions.retain(|r| r.emoji != emoji),
                        None => message.reactions.clear(),
                    }
                }
            }
            Event::MessageAck {
                channel_id,
                message_id,
            } => {
                let read = self.read_up_to.entry(channel_id).or_insert(message_id);
                *read = (*read).max(message_id);
                self.mentions.remove(&channel_id);
            }
            Event::VoiceStateUpdate(mut state) => {
                if let Some(user) = state.user.take() {
                    self.insert_user(user);
                }
                let key = (state.guild_id, state.user_id);
                if state.channel_id.is_some() {
                    self.voice.insert(key, state);
                } else {
                    self.voice.remove(&key);
                }
            }
            Event::Relationships(list) => {
                self.relationships = list.into_iter().map(|r| (r.user_id, r)).collect();
            }
            Event::RelationshipAdd { relationship, user } => {
                if let Some(user) = user {
                    self.insert_user(user);
                }
                self.relationships
                    .insert(relationship.user_id, relationship);
            }
            Event::RelationshipRemove { user_id } => {
                self.relationships.remove(&user_id);
            }
            Event::Presences(list) => self.presences.extend(list),
            Event::PresenceUpdate { user_id, presence } => {
                self.presences.insert(user_id, presence);
            }
            Event::MemberListUpdate {
                guild_id,
                list_id,
                ops,
            } => self.apply_member_list(guild_id, list_id, ops),
            Event::Resumed
            | Event::TypingStart { .. }
            | Event::VoiceServerUpdate { .. }
            | Event::Other(_) => {}
        }
    }

    fn insert_guild(&mut self, mut guild: Guild) {
        let id = guild.id;
        self.voice.retain(|&(guild_id, _), _| guild_id != Some(id));
        for mut state in std::mem::take(&mut guild.voice_states) {
            if let Some(user) = state.user.take() {
                self.insert_user(user);
            }
            self.voice.insert((Some(id), state.user_id), state);
        }
        self.guilds.insert(id, guild);
    }

    fn insert_user(&mut self, user: User) {
        self.users.insert(user.id, user);
    }

    fn insert_channel(&mut self, channel: Channel) {
        // With no read state, Discord treats a channel as read so far.
        if let Some(last) = channel.last_message_id
            && !self.read_up_to.contains_key(&channel.id)
            && channel.guild_id.is_some()
        {
            self.read_up_to.insert(channel.id, last);
        }
        self.channels.insert(channel.id, channel);
    }

    /// Picks our own roles out of a list of members.
    fn apply_members(&mut self, members: Vec<Member>) {
        for member in members {
            if Some(member.user_id) == self.me
                && let Some(guild) = self.guilds.get_mut(&member.guild_id)
            {
                guild.my_roles = member.roles.clone();
            }
            self.members
                .insert((member.guild_id, member.user_id), member);
        }
    }

    fn apply_member_list(&mut self, guild_id: Snowflake, list_id: String, ops: Vec<MemberListOp>) {
        let list = self.member_lists.entry(guild_id).or_default();
        if list.id != list_id {
            // A different channel's list (different permissions): start over.
            *list = MemberList {
                id: list_id,
                items: Vec::new(),
            };
        }
        let mut seen = Vec::new();
        for op in ops {
            match op {
                MemberListOp::Sync { start, items } => {
                    let list = self.member_lists.get_mut(&guild_id).expect("created above");
                    if list.items.len() < start + items.len() {
                        list.items
                            .resize(start + items.len(), MemberListItem::Member(Snowflake(0)));
                    }
                    for (offset, item) in items.into_iter().enumerate() {
                        list.items[start + offset] = item.to_item();
                        seen.push(item);
                    }
                }
                MemberListOp::Insert { index, item } => {
                    let list = self.member_lists.get_mut(&guild_id).expect("created above");
                    let index = index.min(list.items.len());
                    list.items.insert(index, item.to_item());
                    seen.push(item);
                }
                MemberListOp::Update { index, item } => {
                    let list = self.member_lists.get_mut(&guild_id).expect("created above");
                    if let Some(slot) = list.items.get_mut(index) {
                        *slot = item.to_item();
                    }
                    seen.push(item);
                }
                MemberListOp::Delete { index } => {
                    let list = self.member_lists.get_mut(&guild_id).expect("created above");
                    if index < list.items.len() {
                        list.items.remove(index);
                    }
                }
                MemberListOp::Invalidate { start, end } => {
                    let list = self.member_lists.get_mut(&guild_id).expect("created above");
                    let end = (end + 1).min(list.items.len());
                    if start < end {
                        for slot in &mut list.items[start..end] {
                            *slot = MemberListItem::Member(Snowflake(0));
                        }
                    }
                }
            }
        }
        if let Some(list) = self.member_lists.get_mut(&guild_id) {
            // Placeholders from out-of-order syncs carry nothing to show.
            while list.items.last() == Some(&MemberListItem::Member(Snowflake(0))) {
                list.items.pop();
            }
        }
        for entry in seen {
            if let MemberListEntry::Member {
                user,
                member,
                presence,
            } = entry
            {
                self.presences.insert(user.id, presence);
                self.insert_user(user);
                self.apply_members(vec![member]);
            }
        }
    }

    fn push_message(&mut self, message: Message) {
        let channel_id = message.channel_id;
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.last_message_id = Some(message.id);
        }

        let from_me = Some(message.author_id) == self.me;
        if from_me {
            // Anything we sent ourselves counts as read.
            self.read_up_to.insert(channel_id, message.id);
        } else if self.active_channel != Some(channel_id) {
            let mentioned = self.me.is_some_and(|me| message.mentions_user(me));
            let is_dm = self
                .channels
                .get(&channel_id)
                .is_some_and(|c| matches!(c.kind, ChannelKind::Dm | ChannelKind::GroupDm));
            // In DMs every message works like a mention, as in Discord.
            if mentioned || (is_dm && !self.is_muted(channel_id)) {
                *self.mentions.entry(channel_id).or_default() += 1;
            }
        }

        let limit = self.limit_for(channel_id);
        // Only channels we're keeping store messages; the rest are just counted.
        let Some(list) = self.messages.get_mut(&channel_id) else {
            return;
        };
        // Our own sent message can arrive twice (REST reply + gateway event).
        if let Some(existing) = list.iter_mut().rev().take(20).find(|m| m.id == message.id) {
            *existing = message;
            return;
        }
        list.push_back(message);
        while list.len() > limit {
            list.pop_front();
        }
    }

    /// Marks a channel as most recently used and evicts the least recently
    /// used ones if we're over the profile's limit.
    fn touch_channel(&mut self, channel_id: Snowflake) {
        self.recent_channels.retain(|&id| id != channel_id);
        self.recent_channels.push_front(channel_id);
        while self.recent_channels.len() > self.profile.cached_channels {
            if let Some(evicted) = self.recent_channels.pop_back() {
                self.messages.remove(&evicted);
                self.history_loaded.remove(&evicted);
                self.history_complete.remove(&evicted);
            }
        }
    }

    fn find_message_mut(&mut self, channel_id: Snowflake, id: Snowflake) -> Option<&mut Message> {
        self.messages
            .get_mut(&channel_id)?
            .iter_mut()
            .find(|m| m.id == id)
    }

    fn remove_channel(&mut self, id: Snowflake) {
        if let Some(channel) = self.channels.remove(&id)
            && let Some(guild) = channel.guild_id.and_then(|g| self.guilds.get_mut(&g))
        {
            guild.channels.retain(|&c| c != id);
        }
        self.messages.remove(&id);
        self.history_loaded.remove(&id);
        self.history_complete.remove(&id);
        self.mentions.remove(&id);
        self.read_up_to.remove(&id);
        self.recent_channels.retain(|&c| c != id);
        if self.active_channel == Some(id) {
            self.active_channel = None;
        }
    }

    fn remove_guild(&mut self, id: Snowflake) {
        if let Some(guild) = self.guilds.remove(&id) {
            for channel_id in guild.channels {
                self.remove_channel(channel_id);
            }
        }
        self.settings.remove(&Some(id));
        self.voice.retain(|&(guild_id, _), _| guild_id != Some(id));
        self.members.retain(|&(guild_id, _), _| guild_id != id);
        self.member_lists.remove(&id);
    }
}

fn add_reaction(message: &mut Message, emoji: ReactionEmoji, is_me: bool) {
    match message.reactions.iter_mut().find(|r| r.emoji == emoji) {
        // Our own reaction may already be counted (we add it before the event arrives).
        Some(reaction) if is_me && reaction.me => {}
        Some(reaction) => {
            reaction.count += 1;
            reaction.me |= is_me;
        }
        None => message.reactions.push(Reaction {
            emoji,
            count: 1,
            me: is_me,
        }),
    }
}

fn remove_reaction(message: &mut Message, emoji: &ReactionEmoji, is_me: bool) {
    if let Some(reaction) = message.reactions.iter_mut().find(|r| r.emoji == *emoji) {
        if is_me && !reaction.me {
            return; // already removed locally
        }
        reaction.count = reaction.count.saturating_sub(1);
        if is_me {
            reaction.me = false;
        }
    }
    message.reactions.retain(|r| r.count > 0);
}

impl Store {
    /// Applies our own reaction right away, before Discord confirms it, so
    /// the UI feels instant. The gateway's echo is then ignored.
    pub fn toggle_own_reaction(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &ReactionEmoji,
    ) -> bool {
        let Some(message) = self.find_message_mut(channel_id, message_id) else {
            return false;
        };
        let already = message.reactions.iter().any(|r| r.emoji == *emoji && r.me);
        if already {
            remove_reaction(message, emoji, true);
        } else {
            add_reaction(message, emoji.clone(), true);
        }
        !already
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Overwrite, OverwriteKind, ReadState, Role};

    const ME: u64 = 1;
    const OTHER: u64 = 2;

    fn user(id: u64) -> User {
        User {
            id: Snowflake(id),
            username: format!("user{id}"),
            global_name: None,
            avatar: None,
        }
    }

    fn msg(id: u64, channel: u64, author: u64) -> Message {
        Message {
            id: Snowflake(id),
            channel_id: Snowflake(channel),
            author_id: Snowflake(author),
            content: format!("message {id}"),
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

    fn create(id: u64, channel: u64, author: u64) -> Event {
        Event::MessageCreate {
            message: msg(id, channel, author),
            author: user(author),
        }
    }

    fn text_channel(id: u64, guild: u64, last: Option<u64>) -> Channel {
        Channel {
            id: Snowflake(id),
            guild_id: Some(Snowflake(guild)),
            name: Some(format!("channel{id}")),
            kind: ChannelKind::Text,
            parent_id: None,
            position: 0,
            recipients: Vec::new(),
            last_message_id: last.map(Snowflake),
            overwrites: Vec::new(),
            thread: None,
        }
    }

    fn guild(id: u64, channels: &[u64]) -> Guild {
        Guild {
            id: Snowflake(id),
            name: format!("guild{id}"),
            icon: None,
            channels: channels.iter().map(|&c| Snowflake(c)).collect(),
            owner_id: None,
            roles: Vec::new(),
            my_roles: Vec::new(),
            voice_states: Vec::new(),
            stickers: Vec::new(),
            emojis: Vec::new(),
        }
    }

    fn ready(guilds: Vec<Guild>, channels: Vec<Channel>, read_states: Vec<ReadState>) -> Event {
        Event::Ready {
            me: user(ME),
            guilds,
            channels,
            users: Vec::new(),
            members: Vec::new(),
            read_states,
            settings: Vec::new(),
        }
    }

    fn store_with_channels(profile: Profile) -> Store {
        let mut store = Store::new(profile);
        store.apply(ready(
            vec![guild(10, &[100, 200, 300])],
            vec![
                text_channel(100, 10, None),
                text_channel(200, 10, None),
                text_channel(300, 10, None),
            ],
            Vec::new(),
        ));
        store
    }

    #[test]
    fn keeps_only_the_newest_messages_per_channel() {
        let mut store = store_with_channels(Profile::FEATHER); // 50 per channel
        store.open_channel(Snowflake(100));
        for id in 0..60 {
            store.apply(create(id, 100, OTHER));
        }
        let ids: Vec<u64> = store.messages(Snowflake(100)).map(|m| m.id.0).collect();
        assert_eq!(ids.len(), 50);
        assert_eq!(ids.first(), Some(&10));
        assert_eq!(ids.last(), Some(&59));
    }

    #[test]
    fn unread_comes_from_read_state_and_new_messages() {
        let mut store = Store::new(Profile::FEATHER);
        store.apply(ready(
            vec![guild(10, &[100, 200])],
            vec![
                text_channel(100, 10, Some(50)),
                text_channel(200, 10, Some(50)),
            ],
            vec![
                ReadState {
                    channel_id: Snowflake(100),
                    last_read: Some(Snowflake(40)),
                    mentions: 2,
                },
                ReadState {
                    channel_id: Snowflake(200),
                    last_read: Some(Snowflake(50)),
                    mentions: 0,
                },
            ],
        ));
        assert_eq!(
            store.unread(Snowflake(100)),
            Unread {
                unread: true,
                mentions: 2
            }
        );
        assert_eq!(store.unread(Snowflake(200)), Unread::default());

        store.apply(create(60, 200, OTHER));
        assert!(store.unread(Snowflake(200)).unread);
        assert_eq!(
            store.guild_unread(Snowflake(10)),
            Unread {
                unread: true,
                mentions: 2
            }
        );

        // Reading it (here or on another device) clears it.
        assert_eq!(store.mark_read(Snowflake(200)), Some(Snowflake(60)));
        assert_eq!(
            store.mark_read(Snowflake(200)),
            None,
            "already read: nothing to ack"
        );
        store.apply(Event::MessageAck {
            channel_id: Snowflake(100),
            message_id: Snowflake(50),
        });
        assert_eq!(store.guild_unread(Snowflake(10)), Unread::default());
    }

    #[test]
    fn channels_without_read_state_start_read() {
        let mut store = Store::new(Profile::FEATHER);
        store.apply(ready(
            vec![guild(10, &[100])],
            vec![text_channel(100, 10, Some(50))],
            Vec::new(),
        ));
        assert!(!store.unread(Snowflake(100)).unread);
    }

    #[test]
    fn mentions_and_dms_count_but_my_messages_dont() {
        let mut store = store_with_channels(Profile::FEATHER);
        let mut ping = msg(1, 100, OTHER);
        ping.mentions = vec![Snowflake(ME)];
        store.apply(Event::MessageCreate {
            message: ping,
            author: user(OTHER),
        });
        store.apply(create(2, 100, ME));
        assert_eq!(
            store.unread(Snowflake(100)),
            Unread {
                unread: false,
                mentions: 1
            },
            "my message marks it read"
        );

        store.apply(Event::ChannelUpsert(Channel {
            guild_id: None,
            kind: ChannelKind::Dm,
            ..text_channel(500, 0, None)
        }));
        store.apply(create(3, 500, OTHER));
        assert_eq!(
            store.unread(Snowflake(500)).mentions,
            1,
            "DMs count like mentions"
        );
    }

    #[test]
    fn muted_channels_are_not_unread_but_keep_mentions() {
        let mut store = store_with_channels(Profile::FEATHER);
        store.apply(Event::GuildSettingsUpdate(GuildSettings {
            guild_id: Some(Snowflake(10)),
            muted: false,
            muted_channels: vec![Snowflake(100)],
            notifications: 1,
        }));
        let mut ping = msg(1, 100, OTHER);
        ping.mention_everyone = true;
        store.apply(Event::MessageCreate {
            message: ping,
            author: user(OTHER),
        });
        store.apply(create(2, 200, OTHER));

        assert_eq!(
            store.unread(Snowflake(100)),
            Unread {
                unread: false,
                mentions: 1
            }
        );
        assert!(store.unread(Snowflake(200)).unread);
    }

    #[test]
    fn hidden_channels_are_never_unread() {
        let mut store = Store::new(Profile::FEATHER);
        let mut server = guild(10, &[100]);
        server.roles = vec![Role {
            id: Snowflake(10),
            permissions: Permissions::VIEW_CHANNEL.0,
            ..Default::default()
        }];
        let mut secret = text_channel(100, 10, Some(5));
        secret.overwrites = vec![Overwrite {
            id: Snowflake(10),
            kind: OverwriteKind::Role,
            allow: 0,
            deny: Permissions::VIEW_CHANNEL.0,
        }];
        store.apply(ready(vec![server], vec![secret], Vec::new()));
        store.apply(create(9, 100, OTHER));

        assert!(!store.can_view(Snowflake(100)));
        assert_eq!(store.unread(Snowflake(100)), Unread::default());
    }

    #[test]
    fn my_roles_come_from_member_events() {
        let mut store = store_with_channels(Profile::FEATHER);
        store.apply(Event::GuildMemberUpdate(Member {
            guild_id: Snowflake(10),
            user_id: Snowflake(OTHER),
            roles: vec![Snowflake(7)],
            nick: None,
        }));
        assert!(
            store.guild(Snowflake(10)).unwrap().my_roles.is_empty(),
            "someone else's roles"
        );
        store.apply(Event::GuildMemberUpdate(Member {
            guild_id: Snowflake(10),
            user_id: Snowflake(ME),
            roles: vec![Snowflake(7)],
            nick: None,
        }));
        assert_eq!(store.guild(Snowflake(10)).unwrap().my_roles, [Snowflake(7)]);
    }

    #[test]
    fn evicts_least_recently_opened_channel() {
        let mut store = store_with_channels(Profile::FEATHER); // 2 channels
        for channel in [100, 200] {
            store.open_channel(Snowflake(channel));
            store.insert_history(
                Snowflake(channel),
                vec![(msg(channel + 1, channel, OTHER), user(OTHER))],
            );
        }
        store.open_channel(Snowflake(100)); // 100 is now the most recent
        store.open_channel(Snowflake(300)); // pushes out 200

        assert_eq!(store.messages(Snowflake(100)).count(), 1);
        assert_eq!(store.messages(Snowflake(200)).count(), 0);
        assert!(!store.open_channel(Snowflake(100)), "100 kept its history");
        assert!(
            store.open_channel(Snowflake(200)),
            "200 must be fetched again"
        );
    }

    #[test]
    fn scrollback_grows_while_open_and_shrinks_after() {
        let mut store = store_with_channels(Profile::FEATHER); // 50 normally, 250 scrollback
        store.open_channel(Snowflake(100));
        let page = |from: u64| {
            (from..from + 50)
                .map(|id| (msg(id, 100, OTHER), user(OTHER)))
                .collect::<Vec<_>>()
        };
        store.insert_history(Snowflake(100), page(1000));
        store.insert_older(Snowflake(100), page(950), false);
        store.insert_older(Snowflake(100), page(900), true);

        assert_eq!(store.messages(Snowflake(100)).count(), 150);
        assert_eq!(store.oldest_message(Snowflake(100)), Some(Snowflake(900)));
        assert!(
            !store.has_older_messages(Snowflake(100)),
            "reached the start"
        );

        store.open_channel(Snowflake(200));
        assert_eq!(
            store.messages(Snowflake(100)).count(),
            50,
            "trimmed when we left"
        );
        assert!(store.has_older_messages(Snowflake(100)));
    }

    #[test]
    fn history_merges_with_live_messages_without_duplicates() {
        let mut store = store_with_channels(Profile::STANDARD);
        assert!(store.open_channel(Snowflake(100)));
        store.apply(create(5, 100, OTHER)); // arrived while history was loading
        store.insert_history(
            Snowflake(100),
            vec![
                (msg(5, 100, OTHER), user(OTHER)),
                (msg(3, 100, OTHER), user(OTHER)),
                (msg(4, 100, OTHER), user(OTHER)),
            ],
        );
        let ids: Vec<u64> = store.messages(Snowflake(100)).map(|m| m.id.0).collect();
        assert_eq!(ids, [3, 4, 5]);
        assert!(!store.open_channel(Snowflake(100)), "history is loaded now");
    }

    #[test]
    fn reactions_add_remove_and_ignore_own_echo() {
        let mut store = store_with_channels(Profile::STANDARD);
        store.open_channel(Snowflake(100));
        store.apply(create(1, 100, OTHER));
        let thumbs = ReactionEmoji::unicode("👍");
        let react = |user_id: u64| Event::ReactionAdd {
            channel_id: Snowflake(100),
            message_id: Snowflake(1),
            user_id: Snowflake(user_id),
            emoji: thumbs.clone(),
        };

        store.apply(react(OTHER));
        assert!(
            store.toggle_own_reaction(Snowflake(100), Snowflake(1), &thumbs),
            "added"
        );
        store.apply(react(ME)); // Discord's echo of our own reaction
        let reaction = &store
            .message(Snowflake(100), Snowflake(1))
            .unwrap()
            .reactions[0];
        assert_eq!((reaction.count, reaction.me), (2, true));

        assert!(
            !store.toggle_own_reaction(Snowflake(100), Snowflake(1), &thumbs),
            "removed"
        );
        store.apply(Event::ReactionRemove {
            channel_id: Snowflake(100),
            message_id: Snowflake(1),
            user_id: Snowflake(ME),
            emoji: thumbs.clone(),
        });
        let reaction = &store
            .message(Snowflake(100), Snowflake(1))
            .unwrap()
            .reactions[0];
        assert_eq!((reaction.count, reaction.me), (1, false));
    }

    #[test]
    fn ready_replaces_a_stale_snapshot() {
        let mut store = store_with_channels(Profile::FEATHER);
        store.open_channel(Snowflake(100));
        store.insert_cached(Snowflake(100), vec![(msg(1, 100, OTHER), user(OTHER))]);
        assert!(
            store.open_channel(Snowflake(100)),
            "cached messages don't count as loaded"
        );

        // Logging in again without guild 10 (we left it).
        store.apply(ready(
            vec![guild(20, &[300])],
            vec![text_channel(300, 20, None)],
            Vec::new(),
        ));
        assert!(store.guild(Snowflake(10)).is_none());
        assert!(store.guild(Snowflake(20)).is_some());
    }

    #[test]
    fn edits_and_deletes_messages() {
        let mut store = store_with_channels(Profile::STANDARD);
        store.open_channel(Snowflake(100));
        store.apply(create(1, 100, OTHER));
        store.apply(create(2, 100, OTHER));
        store.apply(Event::MessageUpdate {
            id: Snowflake(1),
            channel_id: Snowflake(100),
            content: Some("edited".into()),
            embeds: None,
            edited: true,
        });
        store.apply(Event::MessageDelete {
            id: Snowflake(2),
            channel_id: Snowflake(100),
        });

        let messages: Vec<&Message> = store.messages(Snowflake(100)).collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "edited");
        assert!(messages[0].edited);
    }

    #[test]
    fn deleting_a_guild_removes_its_channels() {
        let mut store = store_with_channels(Profile::STANDARD);
        store.open_channel(Snowflake(100));
        store.apply(create(1, 100, OTHER));
        store.apply(Event::GuildDelete {
            id: Snowflake(10),
            unavailable: false,
        });

        assert!(store.guild(Snowflake(10)).is_none());
        assert!(store.channel(Snowflake(100)).is_none());
        assert_eq!(store.messages(Snowflake(100)).count(), 0);
        assert_eq!(store.active_channel(), None);
    }

    #[test]
    fn voice_states_follow_joins_and_leaves() {
        let mut store = store_with_channels(Profile::STANDARD);
        let state = |user: u64, channel: Option<u64>| VoiceState {
            guild_id: Some(Snowflake(10)),
            channel_id: channel.map(Snowflake),
            user_id: Snowflake(user),
            session_id: String::new(),
            self_mute: false,
            self_deaf: false,
            mute: false,
            deaf: false,
            user: None,
        };
        store.apply(Event::VoiceStateUpdate(state(2, Some(100))));
        store.apply(Event::VoiceStateUpdate(state(1, Some(100))));
        let ids: Vec<u64> = store
            .voice_members(Snowflake(100))
            .iter()
            .map(|v| v.user_id.0)
            .collect();
        assert_eq!(ids, [1, 2]);

        // Moving channels replaces the old state; leaving removes it.
        store.apply(Event::VoiceStateUpdate(state(2, Some(101))));
        store.apply(Event::VoiceStateUpdate(state(1, None)));
        assert!(store.voice_members(Snowflake(100)).is_empty());
        assert_eq!(store.voice_members(Snowflake(101)).len(), 1);

        store.apply(Event::GuildDelete {
            id: Snowflake(10),
            unavailable: false,
        });
        assert!(store.voice_members(Snowflake(101)).is_empty());
    }

    #[test]
    fn member_list_ops_and_role_colours() {
        use crate::events::{MemberListEntry, MemberListOp};
        use crate::models::{Presence, Role, Status};
        let mut store = store_with_channels(Profile::STANDARD);
        store.apply(Event::GuildRoleUpsert {
            guild_id: Snowflake(10),
            role: Role {
                id: Snowflake(7),
                color: 0xFF0000,
                position: 1,
                ..Default::default()
            },
        });
        let person = |id: u64, nick: &str| MemberListEntry::Member {
            user: user(id),
            member: Member {
                guild_id: Snowflake(10),
                user_id: Snowflake(id),
                roles: vec![Snowflake(7)],
                nick: Some(nick.into()),
            },
            presence: Presence {
                status: Status::Online,
                activity: None,
            },
        };
        store.apply(Event::MemberListUpdate {
            guild_id: Snowflake(10),
            list_id: "everyone".into(),
            ops: vec![MemberListOp::Sync {
                start: 0,
                items: vec![
                    MemberListEntry::Group {
                        id: "online".into(),
                        count: 2,
                    },
                    person(50, "Al"),
                    person(51, "Bo"),
                ],
            }],
        });
        assert_eq!(store.member_list(Snowflake(10)).len(), 3);
        assert_eq!(
            store.display_name_in(Some(Snowflake(10)), Snowflake(50)),
            "Al"
        );
        assert_eq!(
            store.role_color(Snowflake(10), Snowflake(50)),
            Some(0xFF0000)
        );
        assert_eq!(store.presence(Snowflake(51)).status, Status::Online);

        store.apply(Event::MemberListUpdate {
            guild_id: Snowflake(10),
            list_id: "everyone".into(),
            ops: vec![MemberListOp::Delete { index: 1 }],
        });
        assert_eq!(
            store.member_list(Snowflake(10))[1],
            MemberListItem::Member(Snowflake(51))
        );
    }
}
