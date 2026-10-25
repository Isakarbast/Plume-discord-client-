//! The client logic for the GTK app: owns the store and the connections,
//! reacts to Discord events, and tells the widgets what to show.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gio, glib};
use plume_cache::Cache;
use plume_core::models::{Message, ReactionEmoji, User};
use plume_core::navigation;
use plume_core::{Event, Snowflake, Store, Tier, Token};
use plume_gateway::{Gateway, GatewayError, GatewayHandle};
use plume_media::{MediaConfig, MediaStore};
use plume_rest::{RestClient, RestError, Upload};
use tracing::warn;

use crate::demo;
use crate::settings::{self, Options};
use crate::state::bridge;
use crate::widgets::avatar::TextureCache;
use crate::window::Ui;

mod social;
mod voice;
pub use voice::{Call, VoiceStatus};

/// Discord's limit for one message (without Nitro).
const MAX_MESSAGE_LENGTH: usize = 2000;
const TYPING_TIMEOUT: Duration = Duration::from_secs(10);
const TYPING_RESEND: Duration = Duration::from_secs(8);
const ACK_INTERVAL: Duration = Duration::from_secs(3);
const OLDER_PAGE: u8 = 50;
const CACHE_SAVE_SECONDS: u32 = 60;
const CACHED_CHANNELS_ON_DISK: usize = 50;

/// What Enter in the composer does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compose {
    New,
    Reply(Snowflake),
    Edit(Snowflake),
}

pub struct AppState {
    pub app: adw::Application,
    pub options: RefCell<Options>,
    pub store: RefCell<Store>,
    pub media: Option<MediaStore>,
    pub textures: RefCell<TextureCache>,
    pub ui: OnceCell<Ui>,
    /// Which server's channels are listed (`None` = DMs).
    pub selected_guild: Cell<Option<Snowflake>>,
    pub online: Cell<bool>,
    rest: Option<RestClient>,
    gateway: RefCell<Option<GatewayHandle>>,
    cache: RefCell<Option<Cache>>,
    compose: Cell<Compose>,
    typing: RefCell<HashMap<Snowflake, Vec<(Snowflake, Instant)>>>,
    last_typing_sent: Cell<Option<(Snowflake, Instant)>>,
    pending_ack: Cell<Option<(Snowflake, Snowflake)>>,
    last_ack: Cell<Option<Instant>>,
    loading_older: Cell<bool>,
    ticks: Cell<u32>,
    /// The call we're in, if any.
    pub voice: RefCell<Option<Call>>,
    voice_muted: Cell<bool>,
    voice_deafened: Cell<bool>,
    /// The status we've set.
    pub status: Cell<plume_core::models::Status>,
    /// The Friends page is showing instead of a channel.
    pub showing_friends: Cell<bool>,
    people_pending: Cell<bool>,
    sidebar_pending: Cell<bool>,
    /// Where "NEW" goes: the last message read before opening this channel.
    pub unread_marker: Cell<Option<(Snowflake, Snowflake)>>,
}

impl AppState {
    pub fn new(app: &adw::Application, options: Options, token: Option<&Token>) -> Rc<Self> {
        let rest = token.and_then(|token| RestClient::new(token).ok());
        let media = (!options.demo)
            .then(|| {
                let (memory, disk) = match options.profile.tier {
                    Tier::Full => (256 * 1024 * 1024, 1024 * 1024 * 1024),
                    _ => (96 * 1024 * 1024, 500 * 1024 * 1024),
                };
                MediaStore::new(MediaConfig {
                    // Decoded images are kept as GTK textures instead (see TextureCache).
                    memory_bytes: 0,
                    disk: settings::cache_dir().map(|dir| (dir.join("media"), disk)),
                })
                .map(|store| (store, memory))
                .ok()
            })
            .flatten();
        // Decoded images in memory (RGBA): avatars, emoji, previews.
        let texture_budget = match options.profile.tier {
            Tier::Full => 64 * 1024 * 1024,
            _ => 24 * 1024 * 1024,
        };
        if let Some((store, _)) = &media {
            let store = store.clone();
            bridge::runtime().spawn_blocking(move || store.prune_disk());
        }

        let status = options.status;
        Rc::new(AppState {
            app: app.clone(),
            store: RefCell::new(Store::new(options.profile)),
            options: RefCell::new(options),
            media: media.map(|(store, _)| store),
            textures: RefCell::new(TextureCache::new(texture_budget)),
            ui: OnceCell::new(),
            selected_guild: Cell::new(None),
            online: Cell::new(false),
            rest,
            gateway: RefCell::new(None),
            cache: RefCell::new(None),
            compose: Cell::new(Compose::New),
            typing: RefCell::new(HashMap::new()),
            last_typing_sent: Cell::new(None),
            pending_ack: Cell::new(None),
            last_ack: Cell::new(None),
            loading_older: Cell::new(false),
            ticks: Cell::new(0),
            voice: RefCell::new(None),
            voice_muted: Cell::new(false),
            voice_deafened: Cell::new(false),
            status: Cell::new(status),
            showing_friends: Cell::new(false),
            people_pending: Cell::new(false),
            sidebar_pending: Cell::new(false),
            unread_marker: Cell::new(None),
        })
    }

    fn ui(&self) -> &Ui {
        self.ui.get().expect("UI is built before any events arrive")
    }

    // ---- Starting up --------------------------------------------------------

    /// Loads the cache, connects to Discord (or loads the demo), and starts
    /// the once-a-second timer.
    pub fn start(self: &Rc<Self>, token: Option<Token>) {
        if self.options.borrow().demo {
            self.on_event(demo::ready());
            for event in demo::extras() {
                self.on_event(event);
            }
            // Start somewhere interesting (PLUME_DEMO_CHANNEL picks another one).
            let channel = std::env::var("PLUME_DEMO_CHANNEL")
                .ok()
                .and_then(|c| c.parse().ok())
                .unwrap_or(demo::GENERAL);
            self.selected_guild.set(
                self.store
                    .borrow()
                    .channel(channel)
                    .and_then(|c| c.guild_id),
            );
            self.refresh_sidebar();
            self.open_channel(channel);
        } else {
            self.open_cache();
            if let Some(token) = token {
                self.connect(token);
            }
        }
        self.refresh_sidebar();
        self.ui().messages.sync(self);
        self.update_header_buttons();

        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
            Some(state) => {
                state.on_tick();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }

    fn connect(self: &Rc<Self>, token: Token) {
        let (events_tx, events_rx) = tokio::sync::mpsc::channel(256);
        let gateway = Gateway::new(token, events_tx);
        let custom = Some(self.options.borrow().custom_status.clone());
        gateway.handle().set_status(self.status.get(), custom);
        *self.gateway.borrow_mut() = Some(gateway.handle());

        let events = bridge::forward(events_rx);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(state) = weak.upgrade() else { break };
                state.on_event(event);
            }
        });

        let weak = Rc::downgrade(self);
        bridge::spawn(gateway.run(), move |result| {
            let Some(state) = weak.upgrade() else { return };
            state.online.set(false);
            match result {
                Ok(()) => {}
                Err(GatewayError::AuthenticationFailed) => {
                    state.toast(
                        "Discord rejected the saved token. Log out from the menu and log in again.",
                    );
                }
                Err(error) => state.toast(&format!("Disconnected: {error}")),
            }
        });
    }

    fn open_cache(&self) {
        let Some(path) = settings::cache_dir().map(|dir| dir.join("cache.sqlite")) else {
            return;
        };
        let cache = match Cache::open(&path) {
            Ok(cache) => cache,
            Err(error) => {
                warn!("cache unavailable: {error}");
                return;
            }
        };
        match cache.load_snapshot() {
            Ok(Some(snapshot)) => self.store.borrow_mut().apply(snapshot.into_event()),
            Ok(None) => {}
            Err(error) => warn!("couldn't read the cache: {error}"),
        }
        *self.cache.borrow_mut() = Some(cache);
    }

    pub fn save_cache(&self) {
        if !self.online.get() {
            return;
        }
        let Some(cache) = &mut *self.cache.borrow_mut() else {
            return;
        };
        let store = self.store.borrow();
        let per_channel = store.profile().messages_per_channel;
        let result = cache
            .save_snapshot(&store)
            .and_then(|()| cache.save_messages(&store, per_channel))
            .and_then(|()| cache.prune_messages(CACHED_CHANNELS_ON_DISK));
        if let Err(error) = result {
            warn!("couldn't save the cache: {error}");
        }
    }

    /// Called when the window closes.
    pub fn shutdown(&self) {
        self.end_call();
        self.flush_ack();
        self.save_cache();
    }

    // ---- Discord events -----------------------------------------------------

    pub fn on_event(self: &Rc<Self>, event: Event) {
        self.on_voice_gateway_event(&event);
        let active = self.store.borrow().active_channel();
        let mut sidebar = false;
        let mut messages = false;
        let mut refresh_one = None;
        let mut read_after = None;

        match &event {
            Event::Ready { .. } | Event::Resumed => {
                // Parsing READY allocates a lot that's freed right after.
                glib::idle_add_local_once(crate::memory::give_back);
                self.online.set(true);
                sidebar = true;
                messages = true;
            }
            Event::GuildCreate { .. }
            | Event::GuildDelete { .. }
            | Event::GuildRoleUpsert { .. }
            | Event::GuildRoleDelete { .. }
            | Event::GuildSettingsUpdate(_)
            | Event::ChannelUpsert(_)
            | Event::ChannelDelete { .. }
            | Event::MessageAck { .. } => sidebar = true,
            Event::MessageCreate { message, author } => {
                if let Some(typers) = self.typing.borrow_mut().get_mut(&message.channel_id) {
                    typers.retain(|(user, _)| *user != message.author_id);
                }
                let watching = active == Some(message.channel_id) && self.ui().window.is_active();
                if self.options.borrow().notifications {
                    self.notify(message, author, watching);
                }
                if watching {
                    read_after = Some(message.channel_id);
                }
                // Most messages land in channels that are already unread (or
                // on screen): nothing in the sidebar changes, so don't redraw it.
                sidebar = self.sidebar_changes_with(message);
                messages = active == Some(message.channel_id);
            }
            Event::MessageDelete { channel_id, .. } => messages = active == Some(*channel_id),
            Event::MessageUpdate { id, channel_id, .. }
            | Event::ReactionAdd {
                message_id: id,
                channel_id,
                ..
            }
            | Event::ReactionRemove {
                message_id: id,
                channel_id,
                ..
            }
            | Event::ReactionClear {
                message_id: id,
                channel_id,
                ..
            } => {
                if active == Some(*channel_id) {
                    refresh_one = Some(*id);
                }
            }
            Event::TypingStart {
                channel_id,
                user_id,
            } => {
                if Some(*user_id) == self.store.borrow().me().map(|u| u.id) {
                    return;
                }
                let mut typing = self.typing.borrow_mut();
                let typers = typing.entry(*channel_id).or_default();
                typers.retain(|(user, _)| user != user_id);
                typers.push((*user_id, Instant::now()));
            }
            // Sent with every message in a server: only our own roles change what's shown.
            Event::GuildMemberUpdate(member) => {
                sidebar = Some(member.user_id) == self.store.borrow().me().map(|u| u.id);
            }
            // Only the server on screen shows who's in voice.
            Event::VoiceStateUpdate(state) => sidebar = state.guild_id == self.selected_guild.get(),
            Event::Relationships(_)
            | Event::RelationshipAdd { .. }
            | Event::RelationshipRemove { .. }
            | Event::Presences(_) => {
                self.store.borrow_mut().apply(event);
                self.refresh_people();
                return;
            }
            Event::PresenceUpdate { user_id, .. } => {
                // Big servers send a stream of these; only redraw for people on screen.
                let shown = self.shows_person(*user_id);
                self.store.borrow_mut().apply(event);
                if shown {
                    self.refresh_people();
                }
                return;
            }
            Event::MemberListUpdate { guild_id, .. } => {
                let shown = self.selected_guild.get() == Some(*guild_id);
                self.store.borrow_mut().apply(event);
                if shown {
                    self.refresh_people();
                }
                return;
            }
            Event::VoiceServerUpdate { .. } | Event::Other(_) => return,
        }

        let is_typing = matches!(event, Event::TypingStart { .. });
        self.store.borrow_mut().apply(event);
        if let Some(channel_id) = read_after {
            self.read_channel(channel_id);
        }
        if sidebar {
            self.refresh_sidebar_soon();
        }
        if messages {
            self.ui().messages.sync(self);
        }
        if let Some(id) = refresh_one {
            self.ui().messages.refresh(id);
        }
        if is_typing || messages {
            self.refresh_typing();
        }
    }

    fn notify(&self, message: &Message, author: &User, watching: bool) {
        let store = self.store.borrow();
        let Some(note) = plume_notify::notification_for(&store, message, author, watching) else {
            return;
        };
        let notification = gio::Notification::new(&note.title);
        notification.set_body(Some(&note.body));
        notification.set_default_action_and_target_value(
            "app.open-channel",
            Some(&message.channel_id.to_string().to_variant()),
        );
        self.app.send_notification(
            Some(&format!("channel-{}", message.channel_id)),
            &notification,
        );
    }

    fn on_tick(&self) {
        self.ticks.set(self.ticks.get() + 1);
        let now = Instant::now();
        let mut changed = false;
        self.typing.borrow_mut().retain(|_, typers| {
            let before = typers.len();
            typers.retain(|(_, since)| now - *since < TYPING_TIMEOUT);
            changed |= typers.len() != before;
            !typers.is_empty()
        });
        if changed {
            self.refresh_typing();
        }
        if self
            .last_ack
            .get()
            .is_none_or(|at| now - at >= ACK_INTERVAL)
        {
            self.flush_ack();
        }
        if self.ticks.get().is_multiple_of(CACHE_SAVE_SECONDS) {
            self.save_cache();
            crate::memory::give_back();
        }
    }

    // ---- Navigation ---------------------------------------------------------

    pub fn select_guild(self: &Rc<Self>, guild: Option<Snowflake>) {
        self.selected_guild.set(guild);
        self.ui().channels.rebuild(self);
        self.ui().split.set_show_content(false);
    }

    pub fn rest(&self) -> Option<RestClient> {
        self.rest.clone()
    }

    /// Open the DM with someone, creating it if needed.
    pub fn open_dm(self: &Rc<Self>, user_id: Snowflake) {
        let existing = self
            .store
            .borrow()
            .channels()
            .find(|c| c.kind == plume_core::models::ChannelKind::Dm && c.recipients == [user_id])
            .map(|c| c.id);
        if let Some(channel_id) = existing {
            self.open_channel(channel_id);
            return;
        }
        let Some(rest) = self.rest.clone() else {
            self.toast("Not available in demo mode");
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(async move { rest.open_dm(user_id).await }, move |result| {
            let Some(state) = weak.upgrade() else { return };
            match result {
                Ok((channel, _recipients)) => {
                    let channel_id = channel.id;
                    state
                        .store
                        .borrow_mut()
                        .apply(Event::ChannelUpsert(channel));
                    state.open_channel(channel_id);
                }
                Err(error) => state.toast(&format!("Couldn't open the DM: {error}")),
            }
        });
    }

    /// Open a text channel, or join a voice channel.
    pub fn go_to(self: &Rc<Self>, channel_id: Snowflake) {
        let is_voice = self
            .store
            .borrow()
            .channel(channel_id)
            .is_some_and(|c| c.kind == plume_core::models::ChannelKind::Voice);
        if is_voice {
            self.join_voice(channel_id);
        } else {
            self.open_channel(channel_id);
        }
    }

    /// A link to a message in the open channel, like Discord's "Copy Message Link".
    pub fn message_link(&self, message_id: Snowflake) -> Option<String> {
        let store = self.store.borrow();
        let channel_id = store.active_channel()?;
        let place = store
            .channel(channel_id)
            .and_then(|c| c.guild_id)
            .map_or_else(|| "@me".to_owned(), |g| g.to_string());
        Some(format!(
            "https://discord.com/channels/{place}/{channel_id}/{message_id}"
        ))
    }

    /// Which header buttons make sense for what's on screen.
    pub fn update_header_buttons(&self) {
        let ui = self.ui();
        let store = self.store.borrow();
        let channel = store.active_channel().and_then(|id| store.channel(id));
        let chat = !self.showing_friends.get() && channel.is_some();
        ui.call_button
            .set_visible(chat && channel.is_some_and(|c| c.guild_id.is_none()));
        ui.pins_button.set_visible(chat);
        ui.threads_button
            .set_visible(chat && channel.is_some_and(|c| c.guild_id.is_some()));
        ui.members_button.set_visible(chat);
        ui.search.set_visible(chat);
    }

    pub fn open_channel(self: &Rc<Self>, channel_id: Snowflake) {
        self.showing_friends.set(false);
        self.compose.set(Compose::New);
        self.loading_older.set(false);
        let (needs_history, guild_id, can_send) = {
            let mut store = self.store.borrow_mut();
            let unread = store.unread(channel_id).unread;
            self.unread_marker.set(
                store
                    .last_read(channel_id)
                    .filter(|_| unread)
                    .map(|read| (channel_id, read)),
            );
            let needs_history = store.open_channel(channel_id);
            if needs_history
                && let Some(cache) = &*self.cache.borrow()
                && let Ok(cached) = cache.load_messages(&store, channel_id)
            {
                // Show last session's messages while the newest ones load.
                store.insert_cached(channel_id, cached);
            }
            let guild_id = store.channel(channel_id).and_then(|c| c.guild_id);
            (needs_history, guild_id, store.can_send(channel_id))
        };
        // Jumping from a notification or the switcher may change servers.
        if self.selected_guild.get() != guild_id {
            self.selected_guild.set(guild_id);
        }
        self.read_channel(channel_id);
        if let (Some(guild_id), Some(gateway)) = (guild_id, &*self.gateway.borrow()) {
            gateway.subscribe_channel(guild_id, channel_id);
        }

        let ui = self.ui();
        ui.set_title(&self.store.borrow(), channel_id);
        ui.content.set_visible_child_name("chat");
        self.update_header_buttons();
        ui.composer.set_context(None);
        ui.composer.set_placeholder_enabled(can_send);
        ui.composer.text.grab_focus();
        ui.split.set_show_content(true);
        self.refresh_sidebar();
        ui.messages.sync(self);
        self.refresh_typing();
        ui.member_list.rebuild(self);

        if needs_history {
            self.fetch_history(channel_id);
        }
    }

    fn fetch_history(self: &Rc<Self>, channel_id: Snowflake) {
        let limit = self.store.borrow().profile().messages_per_channel.min(100) as u8;
        if self.options.borrow().demo {
            self.store
                .borrow_mut()
                .insert_history(channel_id, demo::history(channel_id));
            self.ui().messages.sync(self);
            return;
        }
        let Some(rest) = self.rest.clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.messages(channel_id, limit, None).await },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                match result {
                    Ok(history) => {
                        state.store.borrow_mut().insert_history(channel_id, history);
                        state.ui().messages.sync(&state);
                    }
                    Err(RestError::Forbidden) => {
                        state.toast("You don't have access to that channel")
                    }
                    Err(error) => state.toast(&format!("Couldn't load messages: {error}")),
                }
            },
        );
    }

    /// Scrolled to the top: fetch older messages.
    pub fn load_older(self: &Rc<Self>) {
        let (channel_id, before) = {
            let store = self.store.borrow();
            let Some(channel_id) = store.active_channel() else {
                return;
            };
            if self.loading_older.get() || !store.has_older_messages(channel_id) {
                return;
            }
            let Some(before) = store.oldest_message(channel_id) else {
                return;
            };
            (channel_id, before)
        };
        let Some(rest) = self.rest.clone() else {
            return;
        };
        self.loading_older.set(true);
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.messages(channel_id, OLDER_PAGE, Some(before)).await },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                state.loading_older.set(false);
                match result {
                    Ok(history) => {
                        let reached_start = history.len() < usize::from(OLDER_PAGE);
                        state
                            .store
                            .borrow_mut()
                            .insert_older(channel_id, history, reached_start);
                        state.ui().messages.sync(&state);
                    }
                    Err(error) => state.toast(&format!("Couldn't load older messages: {error}")),
                }
            },
        );
    }

    // ---- Read state ---------------------------------------------------------

    fn read_channel(&self, channel_id: Snowflake) {
        if let Some(message_id) = self.store.borrow_mut().mark_read(channel_id) {
            self.pending_ack.set(Some((channel_id, message_id)));
        }
        if self
            .last_ack
            .get()
            .is_none_or(|at| at.elapsed() >= ACK_INTERVAL)
        {
            self.flush_ack();
        }
    }

    fn flush_ack(&self) {
        let Some((channel_id, message_id)) = self.pending_ack.take() else {
            return;
        };
        let Some(rest) = self.rest.clone() else {
            return;
        };
        self.last_ack.set(Some(Instant::now()));
        bridge::spawn_detached(async move {
            if let Err(error) = rest.ack_message(channel_id, message_id).await {
                warn!("read receipt failed: {error}");
            }
        });
    }

    /// The window got focus: whatever's on screen has been read.
    pub fn window_focused(self: &Rc<Self>) {
        // Bound first: in an `if let`, the borrow would live through the block.
        let active = self.store.borrow().active_channel();
        if let Some(channel_id) = active {
            self.read_channel(channel_id);
        }
        self.refresh_sidebar();
    }

    // ---- Sending, editing, deleting, reacting -------------------------------

    pub fn submit(self: &Rc<Self>, text: String, files: Vec<PathBuf>) {
        let Some(channel_id) = self.store.borrow().active_channel() else {
            return;
        };
        let text = text.trim().to_owned();
        let compose = self.compose.replace(Compose::New);
        self.ui().composer.set_context(None);
        if text.is_empty() && files.is_empty() {
            return;
        }
        if text.chars().count() > MAX_MESSAGE_LENGTH {
            self.toast(&format!(
                "Message is too long ({}/{MAX_MESSAGE_LENGTH})",
                text.chars().count()
            ));
            self.ui().composer.set_text(&text);
            return;
        }

        if let Compose::Edit(message_id) = compose {
            self.edit(channel_id, message_id, text);
            return;
        }
        let reply_to = match compose {
            Compose::Reply(id) => Some(plume_rest::ReplyTo {
                message_id: id,
                ping: self.ui().composer.reply_ping(),
            }),
            _ => None,
        };
        self.last_typing_sent.set(None);
        self.ui().messages.scroll_to_bottom();

        if self.options.borrow().demo {
            self.demo_send(channel_id, text, reply_to.map(|r| r.message_id));
            return;
        }
        let Some(rest) = self.rest.clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move {
                if files.is_empty() {
                    return rest
                        .send_message(channel_id, &text, reply_to)
                        .await
                        .map_err(|e| (e, text));
                }
                let mut uploads = Vec::new();
                for path in &files {
                    let filename = path
                        .file_name()
                        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
                    match tokio::fs::read(path).await {
                        Ok(bytes) => uploads.push(Upload { filename, bytes }),
                        Err(error) => {
                            return Err((
                                RestError::Local(format!("can't read {filename}: {error}")),
                                text,
                            ));
                        }
                    }
                }
                rest.send_message_with_files(channel_id, &text, reply_to, &uploads)
                    .await
                    .map_err(|e| (e, text))
            },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                match result {
                    // Store it now; the gateway's copy of the same message is ignored.
                    Ok((message, author)) => {
                        state.on_event(Event::MessageCreate { message, author })
                    }
                    Err((error, text)) => {
                        state.toast(&format!("Couldn't send: {error}"));
                        if state.ui().composer.current_text().is_empty() {
                            state.ui().composer.set_text(&text);
                        }
                    }
                }
            },
        );
    }

    /// In demo mode, "sending" just adds the message locally.
    fn demo_send(
        self: &Rc<Self>,
        channel_id: Snowflake,
        text: String,
        reply_to: Option<Snowflake>,
    ) {
        let Some(me) = self.store.borrow().me().cloned() else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let message = Message {
            id: Snowflake::from_timestamp_ms(now),
            channel_id,
            author_id: me.id,
            content: text,
            edited: false,
            reply_to,
            mentions: Vec::new(),
            mention_everyone: false,
            attachments: Vec::new(),
            embeds: Vec::new(),
            reactions: Vec::new(),
            stickers: Vec::new(),
            pinned: false,
        };
        self.on_event(Event::MessageCreate {
            message,
            author: me,
        });
    }

    fn edit(self: &Rc<Self>, channel_id: Snowflake, message_id: Snowflake, text: String) {
        let update = move |state: &Rc<AppState>, content: String| {
            state.on_event(Event::MessageUpdate {
                id: message_id,
                channel_id,
                content: Some(content),
                embeds: None,
                edited: true,
            });
        };
        if self.options.borrow().demo {
            update(self, text);
            return;
        }
        let Some(rest) = self.rest.clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.edit_message(channel_id, message_id, &text).await },
            move |result| {
                let Some(state) = weak.upgrade() else { return };
                match result {
                    Ok((message, _)) => update(&state, message.content),
                    Err(error) => state.toast(&format!("Couldn't edit: {error}")),
                }
            },
        );
    }

    pub fn start_reply(&self, message_id: Snowflake) {
        let name = {
            let store = self.store.borrow();
            store
                .active_channel()
                .and_then(|c| store.message(c, message_id))
                .and_then(|m| store.user(m.author_id))
                .map_or_else(|| "a message".to_owned(), |u| u.display_name().to_owned())
        };
        self.compose.set(Compose::Reply(message_id));
        self.ui()
            .composer
            .set_context(Some(&format!("Replying to {name}")));
        self.ui().composer.show_reply_ping(true);
        self.ui().composer.text.grab_focus();
    }

    pub fn start_edit(&self, message_id: Snowflake) {
        let content = {
            let store = self.store.borrow();
            let Some(message) = store
                .active_channel()
                .and_then(|c| store.message(c, message_id))
            else {
                return;
            };
            if Some(message.author_id) != store.me().map(|u| u.id) {
                return;
            }
            message.content.clone()
        };
        self.compose.set(Compose::Edit(message_id));
        let composer = &self.ui().composer;
        composer.set_context(Some("Editing message · Enter to save · Esc to cancel"));
        composer.set_text(&content);
    }

    /// ↑ in an empty composer: edit your newest message, like Discord.
    pub fn edit_last_message(&self) {
        let last = {
            let store = self.store.borrow();
            let me = store.me().map(|u| u.id);
            store.active_channel().and_then(|c| {
                store
                    .messages(c)
                    .filter(|m| Some(m.author_id) == me)
                    .last()
                    .map(|m| m.id)
            })
        };
        if let Some(id) = last {
            self.start_edit(id);
        }
    }

    pub fn cancel_compose(&self) {
        if let Compose::Edit(_) = self.compose.replace(Compose::New) {
            self.ui().composer.set_text("");
        }
        self.ui().composer.set_context(None);
    }

    pub fn delete_message(self: &Rc<Self>, message_id: Snowflake) {
        let Some(channel_id) = self.store.borrow().active_channel() else {
            return;
        };
        // Remove it right away; the gateway's delete event is then a no-op.
        self.on_event(Event::MessageDelete {
            id: message_id,
            channel_id,
        });
        let Some(rest) = self.rest.clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        bridge::spawn(
            async move { rest.delete_message(channel_id, message_id).await },
            move |result| {
                if let (Err(error), Some(state)) = (result, weak.upgrade()) {
                    state.toast(&format!("Couldn't delete: {error}"));
                }
            },
        );
    }

    pub fn toggle_reaction(
        self: &Rc<Self>,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &ReactionEmoji,
    ) {
        let add = self
            .store
            .borrow_mut()
            .toggle_own_reaction(channel_id, message_id, emoji);
        self.ui().messages.refresh(message_id);
        let Some(rest) = self.rest.clone() else {
            return;
        };
        let (weak, emoji) = (Rc::downgrade(self), emoji.clone());
        let api_name = emoji.api_name();
        bridge::spawn(
            async move {
                rest.set_reaction(channel_id, message_id, &api_name, add)
                    .await
            },
            move |result| {
                let (Err(error), Some(state)) = (result, weak.upgrade()) else {
                    return;
                };
                // Undo what we showed.
                state
                    .store
                    .borrow_mut()
                    .toggle_own_reaction(channel_id, message_id, &emoji);
                state.ui().messages.refresh(message_id);
                state.toast(&format!("Couldn't react: {error}"));
            },
        );
    }

    pub fn message_text(&self, message_id: Snowflake) -> Option<String> {
        let store = self.store.borrow();
        store
            .active_channel()
            .and_then(|c| store.message(c, message_id))
            .map(|m| m.content.clone())
    }

    // ---- Typing -------------------------------------------------------------

    pub fn notify_typing(&self) {
        if self.options.borrow().silent_typing || matches!(self.compose.get(), Compose::Edit(_)) {
            return;
        }
        let Some(channel_id) = self.store.borrow().active_channel() else {
            return;
        };
        let recently = self
            .last_typing_sent
            .get()
            .is_some_and(|(channel, at)| channel == channel_id && at.elapsed() < TYPING_RESEND);
        if recently {
            return;
        }
        self.last_typing_sent
            .set(Some((channel_id, Instant::now())));
        let Some(rest) = self.rest.clone() else {
            return;
        };
        bridge::spawn_detached(async move {
            if let Err(error) = rest.trigger_typing(channel_id).await {
                warn!("typing indicator failed: {error}");
            }
        });
    }

    fn refresh_typing(&self) {
        let store = self.store.borrow();
        let text = store.active_channel().and_then(|channel_id| {
            let typing = self.typing.borrow();
            let names: Vec<String> = typing
                .get(&channel_id)?
                .iter()
                .filter_map(|(user, _)| store.user(*user))
                .map(|u| u.display_name().to_owned())
                .collect();
            match names.as_slice() {
                [] => None,
                [one] => Some(format!("{one} is typing…")),
                [a, b] => Some(format!("{a} and {b} are typing…")),
                _ => Some("Several people are typing…".to_owned()),
            }
        });
        self.ui()
            .composer
            .typing
            .set_text(text.as_deref().unwrap_or(""));
    }

    // ---- UI helpers ---------------------------------------------------------

    /// Is this person visible somewhere: friends, the member list, a DM, us?
    fn shows_person(&self, user_id: Snowflake) -> bool {
        let store = self.store.borrow();
        if store.me().is_some_and(|me| me.id == user_id) || store.relationship(user_id).is_some() {
            return true;
        }
        let in_list = self.selected_guild.get().is_some_and(|guild| {
            store
                .member_list(guild)
                .contains(&plume_core::models::MemberListItem::Member(user_id))
        });
        in_list
            || store
                .active_channel()
                .and_then(|c| store.channel(c))
                .is_some_and(|c| c.recipients.contains(&user_id))
    }

    /// Would this new message change the unread dots, badges or DM order?
    fn sidebar_changes_with(&self, message: &Message) -> bool {
        let store = self.store.borrow();
        let Some(channel) = store.channel(message.channel_id) else {
            return false;
        };
        if channel.guild_id.is_none() {
            // DMs re-sort by latest message.
            return true;
        }
        let me = store.me().map(|u| u.id);
        let mentions_me =
            me.is_some_and(|me| message.mentions_user(me)) || message.mention_everyone;
        let unread = store.unread(message.channel_id);
        let on_screen = store.active_channel() == Some(message.channel_id);
        mentions_me || (!unread.unread && !on_screen)
    }

    /// Redraw the sidebar once, a moment from now, however many events ask.
    pub fn refresh_sidebar_soon(self: &Rc<Self>) {
        if self.sidebar_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(150), move || {
            if let Some(state) = weak.upgrade() {
                state.sidebar_pending.set(false);
                state.refresh_sidebar();
            }
        });
    }

    pub fn refresh_sidebar(self: &Rc<Self>) {
        let ui = self.ui();
        ui.guild_rail.rebuild(self);
        ui.channels.rebuild(self);
        ui.user_panel.update(self);
    }

    pub fn toast(&self, text: &str) {
        if let Some(ui) = self.ui.get() {
            ui.toasts.add_toast(adw::Toast::new(text));
        }
    }

    /// Everything the quick switcher can jump to: (channel, label).
    pub fn jump_targets(&self) -> Vec<(Snowflake, String)> {
        let store = self.store.borrow();
        let mut targets: Vec<(Snowflake, String)> = navigation::sorted_dms(&store)
            .into_iter()
            .map(|id| (id, navigation::channel_label(&store, id)))
            .collect();
        for guild in navigation::sorted_guilds(&store) {
            for entry in navigation::guild_channels(&store, guild) {
                if let navigation::ChannelEntry::Channel(id) | navigation::ChannelEntry::Voice(id) =
                    entry
                {
                    targets.push((
                        id,
                        format!("{} · {}", navigation::channel_label(&store, id), guild.name),
                    ));
                }
            }
        }
        targets
    }
}
