use std::collections::HashMap;
use std::error::Error;
use std::io::stdout;
use std::time::{Duration, Instant};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
    Event as TermEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use futures_util::StreamExt;
use plume_cache::Cache;
use plume_core::models::{Message, ReactionEmoji, User};
use plume_core::{Event, Snowflake, Store, Tier, Token};
use plume_gateway::{Gateway, GatewayError, GatewayHandle};
use plume_rest::{RestClient, RestError};
use ratatui::widgets::ListState;
use tokio::sync::mpsc;
use tracing::warn;

use crate::Options;
use crate::emoji;
use crate::input::Input;
use crate::render;
use crate::sidebar::{self, Entry, Sidebar};

/// Discord's limit for one message (without Nitro).
const MAX_MESSAGE_LENGTH: usize = 2000;
/// Discord shows "is typing" for about 10 seconds per trigger.
const TYPING_TIMEOUT: Duration = Duration::from_secs(10);
/// So we don't re-send "typing" on every key press.
const TYPING_RESEND: Duration = Duration::from_secs(8);
const ERROR_TIMEOUT: Duration = Duration::from_secs(6);
/// Read receipts for a busy open channel are batched this far apart.
const ACK_INTERVAL: Duration = Duration::from_secs(3);
/// Older messages fetched per scroll-back.
const OLDER_PAGE: u8 = 50;
/// How often the Standard/Full tiers save their cache.
const CACHE_SAVE_EVERY_TICKS: u32 = 60;
/// Channels whose messages stay in the cache file.
const CACHED_CHANNELS_ON_DISK: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Messages,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Connecting,
    Online,
}

/// What pressing Enter in the input box will do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compose {
    New,
    Reply(Snowflake),
    Edit(Snowflake),
}

/// A question or small text field that takes over the keyboard.
pub enum Prompt {
    ConfirmDelete {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    React {
        channel_id: Snowflake,
        message_id: Snowflake,
        input: Input,
    },
}

/// The Ctrl+K popup.
pub struct Switcher {
    pub input: Input,
    /// (channel, label) pairs matching the query, best first.
    pub matches: Vec<(Snowflake, String)>,
    pub selected: usize,
    /// Every channel we could jump to: (channel, label, lowercase label).
    all: Vec<(Snowflake, String, String)>,
}

/// Results of background work (REST calls), sent back to the UI loop.
enum TaskResult {
    History {
        channel_id: Snowflake,
        result: Result<Vec<(Message, User)>, RestError>,
    },
    Older {
        channel_id: Snowflake,
        result: Result<Vec<(Message, User)>, RestError>,
    },
    Sent {
        text: String,
        result: Result<(Message, User), RestError>,
    },
    Edited {
        result: Result<(Message, User), RestError>,
    },
    /// A fire-and-forget call worked; nothing to do.
    Done,
    Failed {
        what: &'static str,
        error: RestError,
    },
    /// A reaction request failed, so undo what we showed.
    ReactionFailed {
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: ReactionEmoji,
        error: RestError,
    },
}

/// Everything the UI shows and remembers. `render.rs` only reads it (apart
/// from scroll bookkeeping).
pub struct App {
    pub store: Store,
    pub sidebar: Sidebar,
    pub sidebar_state: ListState,
    pub focus: Focus,
    pub input: Input,
    pub compose: Compose,
    pub prompt: Option<Prompt>,
    /// The message picked in the message list (Focus::Messages).
    pub selected: Option<Snowflake>,
    /// Set when the selection moved, so the renderer scrolls to it.
    pub scroll_to_selected: bool,
    /// Lines scrolled up from the newest message.
    pub scroll: usize,
    /// Set by the renderer so PgUp/PgDn can move by half a screen.
    pub messages_height: u16,
    /// Set by the renderer when the top of the history is on screen.
    pub at_top: bool,
    pub loading_history: bool,
    pub loading_older: bool,
    pub connection: Connection,
    pub error: Option<(String, Instant)>,
    pub memory_kb: Option<u64>,
    pub options_tier: Tier,
    pub switcher: Option<Switcher>,

    rest: RestClient,
    gateway: GatewayHandle,
    /// SQLite snapshot for instant startup (Standard and Full only).
    cache: Option<Cache>,
    tasks: mpsc::UnboundedSender<TaskResult>,
    /// Who's typing where, and when we last heard about it.
    typing: HashMap<Snowflake, Vec<(Snowflake, Instant)>>,
    last_typing_sent: Option<(Snowflake, Instant)>,
    /// A read receipt waiting to be sent, and when we last sent one.
    pending_ack: Option<(Snowflake, Snowflake)>,
    last_ack: Option<Instant>,
    /// Whether the terminal window has focus (for read state and notifications).
    terminal_focused: bool,
    silent_typing: bool,
    notifications: bool,
    history_limit: u8,
    ticks: u32,
    quit: bool,
    /// Something changed, so redraw before waiting for the next event.
    dirty: bool,
}

pub async fn run(token: Token, options: Options) -> Result<(), Box<dyn Error>> {
    let rest = RestClient::new(&token)?;
    let (events_tx, mut gateway_events) = mpsc::channel(256);
    let gateway = Gateway::new(token, events_tx);
    let handle = gateway.handle();
    let mut gateway_task = tokio::spawn(gateway.run());
    let (tasks_tx, mut task_results) = mpsc::unbounded_channel();

    let mut app = App::new(rest, handle, tasks_tx, &options);
    if options.profile.tier != Tier::Feather {
        app.open_cache();
    }

    // From here on the terminal is ours; `ratatui::restore` must always run.
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(stdout(), EnableBracketedPaste, EnableFocusChange);
    let mut terminal_events = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));

    let result: Result<(), Box<dyn Error>> = loop {
        if app.dirty {
            app.dirty = false;
            app.sidebar.rebuild_if_dirty(&app.store);
            // Reading /proc is a few microseconds, so the number shown is always current.
            app.memory_kb = resident_memory_kb();
            if let Err(error) = terminal.draw(|frame| render::draw(frame, &mut app)) {
                break Err(error.into());
            }
            app.maybe_load_older();
        }

        tokio::select! {
            Some(event) = terminal_events.next() => match event {
                Ok(event) => app.on_terminal(event),
                Err(error) => break Err(error.into()),
            },
            Some(event) = gateway_events.recv() => app.on_gateway(event),
            Some(result) = task_results.recv() => app.on_task(result),
            _ = ticker.tick() => app.on_tick(),
            finished = &mut gateway_task => break match finished {
                Ok(Ok(())) => Ok(()),
                Ok(Err(GatewayError::AuthenticationFailed)) => {
                    Err("Discord rejected the token. Run `plume-tui logout`, then `plume-tui login`.".into())
                }
                Ok(Err(error)) => Err(error.into()),
                Err(error) => Err(error.into()),
            },
        }

        if app.quit {
            break Ok(());
        }
    };

    app.flush_ack();
    app.save_cache();
    let _ = crossterm::execute!(stdout(), DisableBracketedPaste, DisableFocusChange);
    ratatui::restore();
    result
}

impl App {
    fn new(
        rest: RestClient,
        gateway: GatewayHandle,
        tasks: mpsc::UnboundedSender<TaskResult>,
        options: &Options,
    ) -> Self {
        App {
            store: Store::new(options.profile),
            sidebar: Sidebar::default(),
            sidebar_state: ListState::default(),
            focus: Focus::Sidebar,
            input: Input::default(),
            compose: Compose::New,
            prompt: None,
            selected: None,
            scroll_to_selected: false,
            scroll: 0,
            messages_height: 0,
            at_top: false,
            loading_history: false,
            loading_older: false,
            connection: Connection::Connecting,
            error: None,
            memory_kb: resident_memory_kb(),
            options_tier: options.profile.tier,
            switcher: None,
            rest,
            gateway,
            cache: None,
            tasks,
            typing: HashMap::new(),
            last_typing_sent: None,
            pending_ack: None,
            last_ack: None,
            terminal_focused: true,
            silent_typing: options.silent_typing,
            notifications: options.notifications,
            history_limit: options.profile.messages_per_channel.min(100) as u8,
            ticks: 0,
            quit: false,
            dirty: true,
        }
    }

    // ---- Events -------------------------------------------------------------

    fn on_terminal(&mut self, event: TermEvent) {
        match event {
            TermEvent::Key(key) if key.kind == KeyEventKind::Press => self.on_key(key),
            // A paste arrives in one piece, so its newlines can't trigger "send".
            TermEvent::Paste(text) => self.paste(&text),
            TermEvent::FocusGained => {
                self.terminal_focused = true;
                if let Some(channel_id) = self.store.active_channel() {
                    self.read_channel(channel_id);
                }
            }
            TermEvent::FocusLost => {
                self.terminal_focused = false;
                return;
            }
            TermEvent::Resize(..) => {}
            _ => return,
        }
        self.dirty = true;
    }

    fn paste(&mut self, text: &str) {
        let target = if let Some(switcher) = &mut self.switcher {
            &mut switcher.input
        } else if let Some(Prompt::React { input, .. }) = &mut self.prompt {
            input
        } else {
            self.focus = Focus::Input;
            &mut self.input
        };
        for c in text.chars() {
            target.insert(if c == '\n' || c == '\r' { ' ' } else { c });
        }
        if self.switcher.is_some() {
            self.refilter_switcher();
        }
    }

    fn on_gateway(&mut self, event: Event) {
        let active = self.store.active_channel();
        let mut redraw = true;
        let mut read_after = None;

        match &event {
            Event::Ready { .. } | Event::Resumed => {
                self.connection = Connection::Online;
                self.sidebar.mark_dirty();
            }
            Event::GuildCreate { .. }
            | Event::GuildDelete { .. }
            | Event::GuildRoleUpsert { .. }
            | Event::GuildRoleDelete { .. }
            | Event::GuildMemberUpdate(_)
            | Event::GuildSettingsUpdate(_)
            | Event::ChannelUpsert(_)
            | Event::ChannelDelete { .. }
            | Event::MessageAck { .. } => self.sidebar.mark_dirty(),
            Event::MessageCreate { message, author } => {
                // They sent it, so they're not typing any more.
                if let Some(typers) = self.typing.get_mut(&message.channel_id) {
                    typers.retain(|(user, _)| *user != message.author_id);
                }
                let watching = active == Some(message.channel_id) && self.terminal_focused;
                if self.notifications
                    && let Some(notification) =
                        plume_notify::notification_for(&self.store, message, author, watching)
                {
                    tokio::task::spawn_blocking(move || {
                        if let Err(error) = plume_notify::show(&notification) {
                            warn!("couldn't show a notification: {error}");
                        }
                    });
                }
                if watching {
                    read_after = Some(message.channel_id);
                }
                self.sidebar.mark_dirty(); // unread badges and DM order change
            }
            Event::TypingStart {
                channel_id,
                user_id,
            } => {
                if Some(*user_id) == self.store.me().map(|u| u.id) {
                    return;
                }
                let typers = self.typing.entry(*channel_id).or_default();
                typers.retain(|(user, _)| user != user_id);
                typers.push((*user_id, Instant::now()));
                redraw = active == Some(*channel_id);
            }
            Event::MessageUpdate { channel_id, .. }
            | Event::MessageDelete { channel_id, .. }
            | Event::ReactionAdd { channel_id, .. }
            | Event::ReactionRemove { channel_id, .. }
            | Event::ReactionClear { channel_id, .. } => redraw = active == Some(*channel_id),
            // Presence updates and the like: nothing to redraw.
            Event::VoiceStateUpdate(_)
            | Event::VoiceServerUpdate { .. }
            | Event::Relationships(_)
            | Event::RelationshipAdd { .. }
            | Event::RelationshipRemove { .. }
            | Event::Presences(_)
            | Event::PresenceUpdate { .. }
            | Event::MemberListUpdate { .. }
            | Event::Other(_) => redraw = false,
        }

        if let Event::MessageDelete { id, .. } = &event
            && self.selected == Some(*id)
        {
            self.selected = None;
        }
        self.store.apply(event);
        if let Some(channel_id) = read_after {
            self.read_channel(channel_id);
        }
        self.dirty |= redraw;
    }

    fn on_task(&mut self, result: TaskResult) {
        match result {
            TaskResult::History { channel_id, result } => {
                if Some(channel_id) == self.store.active_channel() {
                    self.loading_history = false;
                }
                match result {
                    Ok(history) => self.store.insert_history(channel_id, history),
                    Err(RestError::Forbidden) => {
                        self.show_error("You don't have access to that channel")
                    }
                    Err(error) => self.show_error(format!("Couldn't load messages: {error}")),
                }
            }
            TaskResult::Older { channel_id, result } => {
                self.loading_older = false;
                match result {
                    Ok(history) => {
                        let reached_start = history.len() < usize::from(OLDER_PAGE);
                        self.store.insert_older(channel_id, history, reached_start);
                    }
                    Err(error) => self.show_error(format!("Couldn't load older messages: {error}")),
                }
            }
            TaskResult::Sent { text, result } => match result {
                // Store it now; the gateway's copy of the same message is ignored.
                Ok((message, author)) => self.store.apply(Event::MessageCreate { message, author }),
                Err(error) => {
                    self.show_error(format!("Couldn't send: {error}"));
                    if self.input.is_empty() {
                        self.input.set(text); // don't lose what they typed
                    }
                }
            },
            TaskResult::Edited { result } => match result {
                Ok((message, _)) => self.store.apply(Event::MessageUpdate {
                    id: message.id,
                    channel_id: message.channel_id,
                    content: Some(message.content),
                    embeds: Some(message.embeds),
                    edited: true,
                }),
                Err(error) => self.show_error(format!("Couldn't edit: {error}")),
            },
            TaskResult::Done => return,
            TaskResult::Failed { what, error } => {
                self.show_error(format!("Couldn't {what}: {error}"))
            }
            TaskResult::ReactionFailed {
                channel_id,
                message_id,
                emoji,
                error,
            } => {
                self.store
                    .toggle_own_reaction(channel_id, message_id, &emoji);
                self.show_error(format!("Couldn't react: {error}"));
            }
        }
        self.dirty = true;
    }

    fn on_tick(&mut self) {
        self.ticks += 1;
        let now = Instant::now();

        let active = self.store.active_channel();
        for (channel_id, typers) in &mut self.typing {
            let before = typers.len();
            typers.retain(|(_, since)| now - *since < TYPING_TIMEOUT);
            if typers.len() != before && Some(*channel_id) == active {
                self.dirty = true;
            }
        }
        self.typing.retain(|_, typers| !typers.is_empty());

        if self
            .error
            .as_ref()
            .is_some_and(|(_, since)| now - *since > ERROR_TIMEOUT)
        {
            self.error = None;
            self.dirty = true;
        }
        if self.last_ack.is_none_or(|at| now - at >= ACK_INTERVAL) {
            self.flush_ack();
        }
        if self.ticks.is_multiple_of(CACHE_SAVE_EVERY_TICKS) {
            self.save_cache();
        }
        // While idle, check every 2 s and redraw if the shown value (0.1 MB) changed.
        if self.ticks.is_multiple_of(2)
            && shown_mb(resident_memory_kb()) != shown_mb(self.memory_kb)
        {
            self.dirty = true;
        }
    }

    // ---- Keys ---------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c' | 'q') if ctrl => {
                self.quit = true;
                return;
            }
            KeyCode::Char('k') if ctrl && self.prompt.is_none() => {
                self.toggle_switcher();
                return;
            }
            _ => {}
        }
        if self.prompt.is_some() {
            self.prompt_key(key);
            return;
        }
        if self.switcher.is_some() {
            self.switcher_key(key);
            return;
        }

        let half_page = usize::from(self.messages_height / 2).max(1);
        match key.code {
            KeyCode::PageUp => self.scroll += half_page,
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(half_page),
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            _ => match self.focus {
                Focus::Sidebar => self.sidebar_key(key),
                Focus::Messages => self.messages_key(key),
                Focus::Input => self.input_key(key),
            },
        }
    }

    /// Tab: sidebar → messages → input → sidebar (messages only with a channel open).
    fn cycle_focus(&mut self, forward: bool) {
        let has_channel = self.store.active_channel().is_some();
        let order = [Focus::Sidebar, Focus::Messages, Focus::Input];
        let mut index = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        loop {
            index = if forward {
                (index + 1) % 3
            } else {
                (index + 2) % 3
            };
            if order[index] != Focus::Messages || has_channel {
                break;
            }
        }
        self.focus = order[index];
        if self.focus == Focus::Messages && self.selected.is_none() {
            self.select_newest();
        }
    }

    fn sidebar_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.sidebar.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.sidebar.move_by(1),
            KeyCode::Home | KeyCode::Char('g') => self.sidebar.select_first(),
            KeyCode::End | KeyCode::Char('G') => self.sidebar.select_last(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(entry) = self.sidebar.selected() {
                    self.activate(entry);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => match self.sidebar.selected() {
                Some(Entry::Guild(id)) if self.sidebar.is_expanded(id) => {
                    self.sidebar.toggle_guild(id)
                }
                Some(Entry::Channel(id) | Entry::Category(id)) => {
                    if let Some(guild_id) = self.store.channel(id).and_then(|c| c.guild_id) {
                        self.sidebar.collapse_guild(guild_id);
                    }
                }
                _ => {}
            },
            KeyCode::Char('i' | '/') => self.focus = Focus::Input,
            _ => {}
        }
    }

    fn activate(&mut self, entry: Entry) {
        match entry {
            Entry::DmHeader => self.sidebar.toggle_dms(),
            Entry::Guild(id) => self.sidebar.toggle_guild(id),
            Entry::Dm(id) | Entry::Channel(id) => self.open_channel(id),
            Entry::ServerHeader | Entry::Category(_) => {}
        }
    }

    fn messages_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = self.message_ids().first().copied();
                self.scroll_to_selected = true;
            }
            KeyCode::End | KeyCode::Char('G') => self.select_newest(),
            KeyCode::Char('r') | KeyCode::Enter => self.start_reply(),
            KeyCode::Char('e') => self.start_edit(),
            KeyCode::Char('d') | KeyCode::Delete => self.ask_delete(),
            KeyCode::Char('+') => self.ask_react(),
            KeyCode::Esc | KeyCode::Char('i') => {
                self.selected = None;
                self.focus = Focus::Input;
            }
            _ => {}
        }
    }

    fn input_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => self.escape_input(),
            KeyCode::Enter => self.submit(),
            // Like Discord: Up in an empty box goes to the messages.
            KeyCode::Up if self.input.is_empty() && self.store.active_channel().is_some() => {
                self.focus = Focus::Messages;
                self.select_newest();
            }
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Delete => self.input.delete(),
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            // With nothing typed, End jumps back to the newest messages.
            KeyCode::End if self.input.is_empty() => self.scroll = 0,
            KeyCode::End => self.input.end(),
            KeyCode::Char('u') if ctrl => {
                self.input.take();
            }
            KeyCode::Char('w') if ctrl => self.input.delete_word(),
            KeyCode::Char(c) if !ctrl && !alt => {
                self.input.insert(c);
                self.notify_typing();
            }
            _ => {}
        }
    }

    /// Esc cancels a reply/edit first; a second Esc goes to the sidebar.
    fn escape_input(&mut self) {
        match self.compose {
            Compose::Edit(_) => {
                self.compose = Compose::New;
                self.input.take();
            }
            Compose::Reply(_) => self.compose = Compose::New,
            Compose::New => self.focus = Focus::Sidebar,
        }
    }

    fn prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match prompt {
            Prompt::ConfirmDelete {
                channel_id,
                message_id,
            } => {
                let (channel_id, message_id) = (*channel_id, *message_id);
                self.prompt = None;
                if matches!(key.code, KeyCode::Char('y' | 'Y') | KeyCode::Enter) {
                    self.delete_message(channel_id, message_id);
                }
            }
            Prompt::React {
                channel_id,
                message_id,
                input,
            } => match key.code {
                KeyCode::Esc => self.prompt = None,
                KeyCode::Enter => {
                    let (channel_id, message_id) = (*channel_id, *message_id);
                    let text = input.take();
                    self.prompt = None;
                    self.react(channel_id, message_id, &text);
                }
                KeyCode::Backspace => input.backspace(),
                KeyCode::Left => input.left(),
                KeyCode::Right => input.right(),
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.insert(c)
                }
                _ => {}
            },
        }
    }

    // ---- Message selection --------------------------------------------------

    fn message_ids(&self) -> Vec<Snowflake> {
        match self.store.active_channel() {
            Some(channel_id) => self.store.messages(channel_id).map(|m| m.id).collect(),
            None => Vec::new(),
        }
    }

    fn select_newest(&mut self) {
        self.selected = self.message_ids().last().copied();
        self.scroll_to_selected = true;
    }

    fn move_selection(&mut self, delta: isize) {
        let ids = self.message_ids();
        let Some(current) = self
            .selected
            .and_then(|id| ids.iter().position(|&m| m == id))
        else {
            self.select_newest();
            return;
        };
        let next = current.saturating_add_signed(delta);
        if next >= ids.len() {
            // Past the newest message: back to typing.
            self.selected = None;
            self.focus = Focus::Input;
            self.scroll = 0;
            return;
        }
        self.selected = Some(ids[next]);
        self.scroll_to_selected = true;
        if next == 0 {
            self.at_top = true; // load older ones
        }
    }

    fn selected_message(&self) -> Option<&Message> {
        let channel_id = self.store.active_channel()?;
        self.store.message(channel_id, self.selected?)
    }

    fn is_mine(&self, message: &Message) -> bool {
        Some(message.author_id) == self.store.me().map(|u| u.id)
    }

    fn start_reply(&mut self) {
        if let Some(message) = self.selected_message() {
            self.compose = Compose::Reply(message.id);
            self.focus = Focus::Input;
            self.selected = None;
        }
    }

    fn start_edit(&mut self) {
        let Some(message) = self.selected_message() else {
            return;
        };
        if !self.is_mine(message) {
            self.show_error("You can only edit your own messages");
            return;
        }
        let (id, content) = (message.id, message.content.clone());
        self.compose = Compose::Edit(id);
        self.input.set(content);
        self.focus = Focus::Input;
        self.selected = None;
    }

    fn ask_delete(&mut self) {
        let Some(message) = self.selected_message() else {
            return;
        };
        if !self.is_mine(message) {
            self.show_error("You can only delete your own messages");
            return;
        }
        self.prompt = Some(Prompt::ConfirmDelete {
            channel_id: message.channel_id,
            message_id: message.id,
        });
    }

    fn ask_react(&mut self) {
        let Some(message) = self.selected_message() else {
            return;
        };
        self.prompt = Some(Prompt::React {
            channel_id: message.channel_id,
            message_id: message.id,
            input: Input::default(),
        });
    }

    // ---- Actions ------------------------------------------------------------

    fn open_channel(&mut self, channel_id: Snowflake) {
        self.scroll = 0;
        self.selected = None;
        self.compose = Compose::New;
        self.loading_older = false;
        self.focus = Focus::Input;
        let needs_history = self.store.open_channel(channel_id);
        if needs_history && let Some(cache) = &self.cache {
            // Show what we saw last time while the newest messages load.
            match cache.load_messages(&self.store, channel_id) {
                Ok(cached) => self.store.insert_cached(channel_id, cached),
                Err(error) => warn!("couldn't read cached messages: {error}"),
            }
        }
        self.sidebar.reveal_channel(&self.store, channel_id);
        self.read_channel(channel_id);

        if let Some(guild_id) = self.store.channel(channel_id).and_then(|c| c.guild_id) {
            self.gateway.subscribe_channel(guild_id, channel_id);
        }
        if needs_history {
            self.loading_history = true;
            let limit = self.history_limit;
            self.spawn(move |rest| async move {
                let result = rest.messages(channel_id, limit, None).await;
                TaskResult::History { channel_id, result }
            });
        }
    }

    /// Opens the SQLite cache and shows the last session's servers and
    /// channels right away. Any problem just means starting without it.
    fn open_cache(&mut self) {
        let Some(path) = crate::cache_path() else {
            return;
        };
        match Cache::open(&path) {
            Ok(cache) => self.use_cache(cache),
            Err(error) => warn!("cache unavailable: {error}"),
        }
    }

    fn use_cache(&mut self, cache: Cache) {
        match cache.load_snapshot() {
            Ok(Some(snapshot)) => {
                self.store.apply(snapshot.into_event());
                self.sidebar.mark_dirty();
            }
            Ok(None) => {}
            Err(error) => warn!("couldn't read the cache: {error}"),
        }
        self.cache = Some(cache);
    }

    fn save_cache(&mut self) {
        // Only once logged in: a snapshot from before READY could be stale.
        if self.connection != Connection::Online {
            return;
        }
        let Some(cache) = &mut self.cache else { return };
        let per_channel = self.store.profile().messages_per_channel;
        let result = cache
            .save_snapshot(&self.store)
            .and_then(|()| cache.save_messages(&self.store, per_channel))
            .and_then(|()| cache.prune_messages(CACHED_CHANNELS_ON_DISK));
        if let Err(error) = result {
            warn!("couldn't save the cache: {error}");
        }
    }

    /// Marks a channel read and queues the read receipt.
    fn read_channel(&mut self, channel_id: Snowflake) {
        if let Some(message_id) = self.store.mark_read(channel_id) {
            self.pending_ack = Some((channel_id, message_id));
            self.sidebar.mark_dirty();
        }
        // Opening a channel acks right away; a busy channel is batched by the ticker.
        if self.last_ack.is_none_or(|at| at.elapsed() >= ACK_INTERVAL) {
            self.flush_ack();
        }
    }

    fn flush_ack(&mut self) {
        let Some((channel_id, message_id)) = self.pending_ack.take() else {
            return;
        };
        self.last_ack = Some(Instant::now());
        let rest = self.rest.clone();
        tokio::spawn(async move {
            if let Err(error) = rest.ack_message(channel_id, message_id).await {
                warn!("read receipt failed: {error}");
            }
        });
    }

    /// Loads older messages once the top of the history is on screen.
    fn maybe_load_older(&mut self) {
        if !self.at_top || self.loading_older || self.loading_history {
            return;
        }
        self.at_top = false;
        let Some(channel_id) = self.store.active_channel() else {
            return;
        };
        if !self.store.has_older_messages(channel_id) {
            return;
        }
        let Some(before) = self.store.oldest_message(channel_id) else {
            return;
        };
        self.loading_older = true;
        self.dirty = true;
        self.spawn(move |rest| async move {
            let result = rest.messages(channel_id, OLDER_PAGE, Some(before)).await;
            TaskResult::Older { channel_id, result }
        });
    }

    fn submit(&mut self) {
        let Some(channel_id) = self.store.active_channel() else {
            self.show_error("Open a channel first");
            return;
        };
        let text = self.input.text().trim().to_owned();
        if text.is_empty() {
            if let Compose::Edit(_) = self.compose {
                self.show_error("A message can't be empty. Select it and press d to delete it");
            }
            return;
        }
        let length = text.chars().count();
        if length > MAX_MESSAGE_LENGTH {
            self.show_error(format!(
                "Message is too long ({length}/{MAX_MESSAGE_LENGTH})"
            ));
            return;
        }

        let compose = std::mem::replace(&mut self.compose, Compose::New);
        if let Compose::Edit(message_id) = compose {
            self.input.take();
            self.spawn(move |rest| async move {
                let result = rest.edit_message(channel_id, message_id, &text).await;
                TaskResult::Edited { result }
            });
            return;
        }
        if !self.store.can_send(channel_id) {
            self.compose = compose;
            self.show_error("You can't send messages in this channel");
            return;
        }

        self.input.take();
        self.scroll = 0;
        self.last_typing_sent = None;
        let reply_to = match compose {
            Compose::Reply(id) => Some(plume_rest::ReplyTo {
                message_id: id,
                ping: true,
            }),
            _ => None,
        };
        self.spawn(move |rest| async move {
            let result = rest.send_message(channel_id, &text, reply_to).await;
            TaskResult::Sent { text, result }
        });
    }

    fn delete_message(&mut self, channel_id: Snowflake, message_id: Snowflake) {
        // Remove it right away; the gateway's delete event will be a no-op.
        self.store.apply(Event::MessageDelete {
            id: message_id,
            channel_id,
        });
        self.selected = None;
        self.spawn(move |rest| async move {
            match rest.delete_message(channel_id, message_id).await {
                Ok(()) => TaskResult::Done,
                Err(error) => TaskResult::Failed {
                    what: "delete",
                    error,
                },
            }
        });
    }

    fn react(&mut self, channel_id: Snowflake, message_id: Snowflake, text: &str) {
        let guild_id = self.store.channel(channel_id).and_then(|c| c.guild_id);
        let Some(emoji) = emoji::resolve(text, &self.store, guild_id) else {
            self.show_error(format!("Unknown emoji \"{}\"", text.trim()));
            return;
        };
        let add = self
            .store
            .toggle_own_reaction(channel_id, message_id, &emoji);
        let api_name = emoji.api_name();
        self.spawn(move |rest| async move {
            match rest
                .set_reaction(channel_id, message_id, &api_name, add)
                .await
            {
                Ok(()) => TaskResult::Done,
                Err(error) => TaskResult::ReactionFailed {
                    channel_id,
                    message_id,
                    emoji,
                    error,
                },
            }
        });
    }

    /// Runs a REST call in the background and sends its result to the UI loop.
    fn spawn<F, Fut>(&self, job: F)
    where
        F: FnOnce(RestClient) -> Fut,
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        let tasks = self.tasks.clone();
        let future = job(self.rest.clone());
        tokio::spawn(async move {
            let _ = tasks.send(future.await);
        });
    }

    /// Tells the channel we're typing, at most once every few seconds.
    fn notify_typing(&mut self) {
        if self.silent_typing || matches!(self.compose, Compose::Edit(_)) {
            return;
        }
        let Some(channel_id) = self.store.active_channel() else {
            return;
        };
        let recently = self
            .last_typing_sent
            .is_some_and(|(channel, at)| channel == channel_id && at.elapsed() < TYPING_RESEND);
        if recently {
            return;
        }
        self.last_typing_sent = Some((channel_id, Instant::now()));
        let rest = self.rest.clone();
        tokio::spawn(async move {
            if let Err(error) = rest.trigger_typing(channel_id).await {
                warn!("typing indicator failed: {error}");
            }
        });
    }

    fn show_error(&mut self, message: impl Into<String>) {
        self.error = Some((message.into(), Instant::now()));
    }

    /// "alice is typing…" for the open channel, if anyone is.
    pub fn typing_text(&self) -> Option<String> {
        let channel_id = self.store.active_channel()?;
        let names: Vec<&str> = self
            .typing
            .get(&channel_id)?
            .iter()
            .filter_map(|(user, _)| self.store.user(*user))
            .map(|u| u.display_name())
            .collect();
        match names.as_slice() {
            [] => None,
            [one] => Some(format!("{one} is typing…")),
            [a, b] => Some(format!("{a} and {b} are typing…")),
            _ => Some("Several people are typing…".to_owned()),
        }
    }

    /// "Replying to Alice" / "Editing your message", shown above the input.
    pub fn compose_text(&self) -> Option<String> {
        match self.compose {
            Compose::New => None,
            Compose::Edit(_) => {
                Some("Editing your message · Enter to save · Esc to cancel".to_owned())
            }
            Compose::Reply(id) => {
                let name = self
                    .store
                    .active_channel()
                    .and_then(|c| self.store.message(c, id))
                    .and_then(|m| self.store.user(m.author_id))
                    .map_or("a message", |u| u.display_name());
                Some(format!("Replying to {name} · Esc to cancel"))
            }
        }
    }

    // ---- Quick switcher -----------------------------------------------------

    fn toggle_switcher(&mut self) {
        if self.switcher.take().is_some() {
            return;
        }
        let mut all = Vec::new();
        for channel_id in sidebar::sorted_dms(&self.store) {
            all.push((channel_id, sidebar::channel_label(&self.store, channel_id)));
        }
        for guild in sidebar::sorted_guilds(&self.store) {
            for entry in sidebar::guild_channels(&self.store, guild) {
                if let Entry::Channel(id) = entry {
                    let label = format!(
                        "{} · {}",
                        sidebar::channel_label(&self.store, id),
                        guild.name
                    );
                    all.push((id, label));
                }
            }
        }
        let all = all.into_iter().map(|(id, label)| {
            let lower = label.to_lowercase();
            (id, label, lower)
        });
        self.switcher = Some(Switcher {
            input: Input::default(),
            matches: Vec::new(),
            selected: 0,
            all: all.collect(),
        });
        self.refilter_switcher();
    }

    fn switcher_key(&mut self, key: KeyEvent) {
        let Some(switcher) = &mut self.switcher else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.switcher = None,
            KeyCode::Enter => {
                let target = switcher.matches.get(switcher.selected).map(|(id, _)| *id);
                self.switcher = None;
                if let Some(channel_id) = target {
                    self.open_channel(channel_id);
                }
            }
            KeyCode::Up => switcher.selected = switcher.selected.saturating_sub(1),
            KeyCode::Down => {
                switcher.selected =
                    (switcher.selected + 1).min(switcher.matches.len().saturating_sub(1));
            }
            KeyCode::Backspace => {
                switcher.input.backspace();
                self.refilter_switcher();
            }
            KeyCode::Left => switcher.input.left(),
            KeyCode::Right => switcher.input.right(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                switcher.input.insert(c);
                self.refilter_switcher();
            }
            _ => {}
        }
    }

    /// Every word typed must appear in the label; labels starting with the
    /// first word come first.
    fn refilter_switcher(&mut self) {
        let Some(switcher) = &mut self.switcher else {
            return;
        };
        let query = switcher.input.text().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let first = words.first().copied().unwrap_or("");

        let mut matches: Vec<(bool, &(Snowflake, String, String))> = switcher
            .all
            .iter()
            .filter(|(_, _, lower)| words.iter().all(|w| lower.contains(w)))
            .map(|entry| {
                // Skip the "#" or "@" when checking "starts with".
                let starts = entry.2.get(1..).is_some_and(|rest| rest.starts_with(first));
                (!starts, entry)
            })
            .collect();
        matches.sort_by_key(|(not_prefix, _)| *not_prefix); // stable: keeps sidebar order
        switcher.matches = matches
            .into_iter()
            .take(200)
            .map(|(_, (id, label, _))| (*id, label.clone()))
            .collect();
        switcher.selected = 0;
    }
}

/// Memory rounded the way the status bar shows it (tenths of a MB).
fn shown_mb(kb: Option<u64>) -> Option<u64> {
    kb.map(|kb| (kb as f64 / 1024.0 * 10.0).round() as u64)
}

/// This process's resident memory in KB (Linux only), to show in the status bar.
fn resident_memory_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use plume_core::Permissions;
    use plume_core::models::{
        Attachment, Channel, ChannelKind, Embed, Guild, Overwrite, OverwriteKind, Reaction,
        ReadState, Role,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const ME: u64 = 1;

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
    ) -> Channel {
        Channel {
            id: Snowflake(id),
            guild_id: guild.map(Snowflake),
            name: name.map(str::to_owned),
            kind,
            parent_id: parent.map(Snowflake),
            position: 0,
            recipients: if guild.is_none() {
                vec![Snowflake(3)]
            } else {
                Vec::new()
            },
            last_message_id: None,
            overwrites: Vec::new(),
            thread: None,
        }
    }

    fn message(seconds: u64, author: u64, content: &str) -> Message {
        Message {
            id: Snowflake::from_timestamp_ms(1_760_000_000_000 + seconds * 1000),
            channel_id: Snowflake(100),
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

    /// An app with one server (plus a hidden channel), a DM, and a few
    /// messages, without any network.
    fn demo_app() -> App {
        let options = Options {
            profile: plume_core::Profile::FEATHER,
            silent_typing: true,
            notifications: false,
        };
        let token = Token::new("test");
        let (events, _) = mpsc::channel(1);
        let handle = Gateway::new(token.clone(), events).handle();
        let (tasks, _) = mpsc::unbounded_channel();
        let mut app = App::new(RestClient::new(&token).unwrap(), handle, tasks, &options);

        let mut secret = channel(
            102,
            Some(10),
            Some("mods-only"),
            ChannelKind::Text,
            Some(99),
        );
        secret.overwrites = vec![Overwrite {
            id: Snowflake(10),
            kind: OverwriteKind::Role,
            allow: 0,
            deny: Permissions::VIEW_CHANNEL.0,
        }];
        let mut help = channel(101, Some(10), Some("help"), ChannelKind::Text, Some(99));
        help.last_message_id = Some(Snowflake(500));

        app.on_gateway(Event::Ready {
            me: user(ME, "Me"),
            guilds: vec![Guild {
                id: Snowflake(10),
                name: "Rust Hangout".into(),
                icon: None,
                channels: vec![
                    Snowflake(99),
                    Snowflake(100),
                    Snowflake(101),
                    Snowflake(102),
                ],
                owner_id: Some(Snowflake(999)),
                roles: vec![Role {
                    id: Snowflake(10),
                    permissions: Permissions::VIEW_CHANNEL.0 | Permissions::SEND_MESSAGES.0,
                    ..Default::default()
                }],
                my_roles: Vec::new(),
                voice_states: Vec::new(),
                stickers: Vec::new(),
                emojis: Vec::new(),
            }],
            channels: vec![
                channel(
                    99,
                    Some(10),
                    Some("Text Channels"),
                    ChannelKind::Category,
                    None,
                ),
                channel(100, Some(10), Some("general"), ChannelKind::Text, Some(99)),
                help,
                secret,
                channel(200, None, None, ChannelKind::Dm, None),
            ],
            users: vec![user(2, "Alice"), user(3, "Bob")],
            members: Vec::new(),
            read_states: vec![ReadState {
                channel_id: Snowflake(101),
                last_read: Some(Snowflake(400)),
                mentions: 1,
            }],
            settings: Vec::new(),
        });

        app.store.open_channel(Snowflake(100));
        app.sidebar.reveal_channel(&app.store, Snowflake(100));
        let mut with_file = message(30, 2, "and a picture");
        with_file.attachments.push(Attachment {
            filename: "crab.png".into(),
            url: "https://cdn.example/crab.png".into(),
            size: 48_000,
            width: None,
            height: None,
        });
        with_file.embeds.push(Embed {
            provider: Some("Wikipedia".into()),
            title: Some("Rust (programming language)".into()),
            url: Some("https://en.wikipedia.org/wiki/Rust".into()),
            description: Some("A memory-safe systems language.".into()),
            ..Embed::default()
        });
        with_file.reactions.push(Reaction {
            emoji: ReactionEmoji::unicode("🦀"),
            count: 3,
            me: true,
        });
        let mut ping = message(90, 3, "hey <@1>, try `cargo run` ||it works||");
        ping.edited = true;
        let mine = message(120, ME, "my own message");
        app.store.insert_history(
            Snowflake(100),
            vec![
                (message(0, 2, "**hello** everyone, this is a *long* message that should wrap onto the next line nicely"), user(2, "Alice")),
                (with_file, user(2, "Alice")),
                (ping, user(3, "Bob")),
                (mine, user(ME, "Me")),
            ],
        );
        app
    }

    fn render(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        app.sidebar.rebuild_if_dirty(&app.store);
        terminal.draw(|frame| render::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[tokio::test]
    async fn draws_sidebar_messages_embeds_reactions_and_status() {
        let mut app = demo_app();
        let screen = render(&mut app, 100, 30);
        println!("{screen}");

        assert!(screen.contains("Rust Hangout"));
        assert!(screen.contains("# general"));
        assert!(screen.contains("@ Bob"), "DM listed by recipient");
        assert!(!screen.contains("mods-only"), "hidden channel stays hidden");
        assert!(
            screen.contains("# help  1"),
            "unread with a mention from read state"
        );
        assert!(screen.contains("crab.png (46.9 KB)"));
        assert!(
            screen.contains("Wikipedia") && screen.contains("Rust (programming language)"),
            "embed"
        );
        assert!(screen.contains("🦀"), "reaction chip");
        assert!(screen.contains("@Me"), "mention resolved to a name");
        assert!(screen.contains("(edited)"));
        assert!(screen.contains("online"));
        assert!(
            !screen.contains("**hello**"),
            "markdown is rendered, not shown raw"
        );
    }

    #[tokio::test]
    async fn selecting_replying_and_editing() {
        let mut app = demo_app();
        app.focus = Focus::Input;
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);

        // ↑ in the empty box selects the newest message (ours).
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.focus, Focus::Messages);
        let mine = app.selected.unwrap();
        assert_eq!(
            app.store.message(Snowflake(100), mine).unwrap().content,
            "my own message"
        );

        // e edits it: the box is filled in.
        app.on_key(key(KeyCode::Char('e')));
        assert_eq!(app.compose, Compose::Edit(mine));
        assert_eq!(app.input.text(), "my own message");
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("Editing your message"));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.compose, Compose::New);
        assert!(app.input.is_empty(), "cancelling an edit clears the box");

        // Select Bob's message and reply.
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Char('r')));
        assert!(matches!(app.compose, Compose::Reply(_)));
        let screen = render(&mut app, 100, 30);
        assert!(screen.contains("Replying to Bob"));

        // Someone else's message can't be edited or deleted.
        app.focus = Focus::Messages;
        app.select_newest();
        app.on_key(key(KeyCode::Up)); // Bob's
        app.on_key(key(KeyCode::Char('d')));
        assert!(app.prompt.is_none());
        assert!(app.error.is_some());
    }

    #[tokio::test]
    async fn delete_asks_first() {
        let mut app = demo_app();
        app.focus = Focus::Messages;
        app.select_newest();
        app.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(matches!(app.prompt, Some(Prompt::ConfirmDelete { .. })));
        assert!(render(&mut app, 100, 30).contains("Delete this message?"));
        app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(app.prompt.is_none());
        assert_eq!(
            app.store.messages(Snowflake(100)).count(),
            4,
            "nothing deleted"
        );
    }

    #[tokio::test]
    async fn cache_restores_servers_and_messages_on_the_next_start() {
        let path =
            std::env::temp_dir().join(format!("plume-tui-cache-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);

        // First run: online, with history in #general. Saving happens on exit.
        let mut first = demo_app();
        first.connection = Connection::Online;
        first.cache = Some(Cache::open(&path).unwrap());
        first.save_cache();

        // Second run: before Discord answers, everything is already there.
        let mut second = App::new(
            first.rest.clone(),
            first.gateway.clone(),
            first.tasks.clone(),
            &Options {
                profile: plume_core::Profile::STANDARD,
                silent_typing: true,
                notifications: false,
            },
        );
        second.use_cache(Cache::open(&path).unwrap());
        assert!(second.store.guild(Snowflake(10)).is_some());
        assert_eq!(
            second.connection,
            Connection::Connecting,
            "still waiting for Discord"
        );

        second.store.open_channel(Snowflake(100));
        let cached = second
            .cache
            .as_ref()
            .unwrap()
            .load_messages(&second.store, Snowflake(100))
            .unwrap();
        assert_eq!(cached.len(), 4);
        let screen = {
            second.store.insert_cached(Snowflake(100), cached);
            render(&mut second, 100, 30)
        };
        assert!(screen.contains("my own message"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn tab_cycles_focus() {
        let mut app = demo_app();
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        app.on_key(tab);
        assert_eq!(app.focus, Focus::Messages);
        app.on_key(tab);
        assert_eq!(app.focus, Focus::Input);
        app.on_key(tab);
        assert_eq!(app.focus, Focus::Sidebar);
    }

    #[tokio::test]
    async fn narrow_terminal_shows_one_pane_at_a_time() {
        let mut app = demo_app();
        app.focus = Focus::Input;
        let chat = render(&mut app, 50, 20);
        assert!(!chat.contains("SERVERS"));

        app.focus = Focus::Sidebar;
        let side = render(&mut app, 50, 20);
        assert!(side.contains("SERVERS"));
    }

    #[tokio::test]
    async fn switcher_filters_by_every_word_and_skips_hidden() {
        let mut app = demo_app();
        app.toggle_switcher();
        for c in "hang".chars() {
            app.switcher.as_mut().unwrap().input.insert(c);
        }
        app.refilter_switcher();
        let labels: Vec<&str> = app
            .switcher
            .as_ref()
            .unwrap()
            .matches
            .iter()
            .map(|(_, l)| l.as_str())
            .collect();
        assert_eq!(labels, ["#general · Rust Hangout", "#help · Rust Hangout"]);
    }

    #[tokio::test]
    async fn tiny_terminal_does_not_panic() {
        let mut app = demo_app();
        for (w, h) in [(1, 1), (5, 3), (20, 5), (59, 10), (61, 4)] {
            for focus in [Focus::Sidebar, Focus::Messages, Focus::Input] {
                app.focus = focus;
                app.select_newest();
                render(&mut app, w, h);
            }
        }
        app.toggle_switcher();
        render(&mut app, 10, 4);
    }
}
