//! Joining, leaving and controlling voice calls.
//!
//! Joining goes: ask the gateway → Discord replies with our voice session
//! (`VoiceStateUpdate`) and a server to use (`VoiceServerUpdate`) → once we
//! have both, `plume-voice` connects.

use std::collections::HashSet;
use std::rc::Rc;

use gtk::glib;
use plume_core::models::{ChannelKind, VoiceState};
use plume_core::{Event, Snowflake};
use plume_voice::{VoiceConnection, VoiceEvent, VoiceServer, VoiceSettings};
use tracing::info;

use super::AppState;
use crate::state::bridge;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceStatus {
    /// Waiting for Discord to tell us where to connect.
    Joining,
    Connecting,
    Connected {
        encrypted: bool,
    },
    /// `--demo`: shows the UI, no audio.
    Demo,
}

pub struct Call {
    pub guild_id: Option<Snowflake>,
    pub channel_id: Snowflake,
    pub status: VoiceStatus,
    pub speaking: HashSet<Snowflake>,
    pub privacy_code: Option<String>,
    session_id: Option<String>,
    /// Voice token and endpoint from `VoiceServerUpdate`.
    server: Option<(String, String)>,
    connection: Option<VoiceConnection>,
    /// Bumped for every connection so a stale one's events are ignored.
    generation: u32,
}

impl Call {
    fn new(guild_id: Option<Snowflake>, channel_id: Snowflake, status: VoiceStatus) -> Self {
        Call {
            guild_id,
            channel_id,
            status,
            speaking: HashSet::new(),
            privacy_code: None,
            session_id: None,
            server: None,
            connection: None,
            generation: 0,
        }
    }

    fn hang_up(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.disconnect();
        }
        self.speaking.clear();
    }
}

impl AppState {
    pub fn voice_muted(&self) -> bool {
        self.voice_muted.get() || self.voice_deafened.get()
    }

    pub fn voice_deafened(&self) -> bool {
        self.voice_deafened.get()
    }

    pub fn is_speaking(&self, user_id: Snowflake) -> bool {
        self.voice
            .borrow()
            .as_ref()
            .is_some_and(|call| call.speaking.contains(&user_id))
    }

    pub fn join_voice(self: &Rc<Self>, channel_id: Snowflake) {
        let (guild_id, allowed, is_voice) = {
            let store = self.store.borrow();
            let Some(channel) = store.channel(channel_id) else {
                return;
            };
            let is_voice = matches!(
                channel.kind,
                ChannelKind::Voice | ChannelKind::Dm | ChannelKind::GroupDm
            );
            let allowed = channel.guild_id.is_none() || store.can_connect(channel_id);
            (channel.guild_id, allowed, is_voice)
        };
        info!(
            "join voice requested: {channel_id} ({}), guild {guild_id:?}",
            self.channel_name(channel_id)
        );
        if !is_voice {
            return;
        }
        if !allowed {
            self.toast("You don't have permission to join this channel");
            return;
        }
        if self
            .voice
            .borrow()
            .as_ref()
            .is_some_and(|call| call.channel_id == channel_id)
        {
            return;
        }
        self.end_call();

        if self.options.borrow().demo {
            *self.voice.borrow_mut() = Some(Call::new(guild_id, channel_id, VoiceStatus::Demo));
            let me = self.store.borrow().me().map(|u| u.id);
            if let Some(me) = me {
                let state = self.voice_state(me, guild_id, Some(channel_id));
                self.store
                    .borrow_mut()
                    .apply(Event::VoiceStateUpdate(state));
            }
            self.refresh_voice();
            return;
        }

        let Some(gateway) = self.gateway.borrow().clone() else {
            self.toast("Not connected to Discord");
            return;
        };
        *self.voice.borrow_mut() = Some(Call::new(guild_id, channel_id, VoiceStatus::Joining));
        gateway.update_voice_state(
            guild_id,
            Some(channel_id),
            self.voice_muted(),
            self.voice_deafened(),
        );
        // In a DM, also make the other side's client ring.
        if guild_id.is_none()
            && let Some(rest) = self.rest.clone()
        {
            bridge::spawn_detached(async move {
                if let Err(error) = rest.ring_call(channel_id).await {
                    tracing::debug!("couldn't ring: {error}");
                }
            });
        }
        self.refresh_voice();
    }

    pub fn leave_voice(self: &Rc<Self>) {
        info!("leave voice clicked");
        self.end_call();
        self.refresh_voice();
    }

    /// Hang up and tell Discord, without refreshing the UI.
    pub(super) fn end_call(&self) {
        let Some(mut call) = self.voice.borrow_mut().take() else {
            return;
        };
        call.hang_up();
        if call.status == VoiceStatus::Demo {
            let me = self.store.borrow().me().map(|u| u.id);
            if let Some(me) = me {
                let state = self.voice_state(me, call.guild_id, None);
                self.store
                    .borrow_mut()
                    .apply(Event::VoiceStateUpdate(state));
            }
        } else if let Some(gateway) = &*self.gateway.borrow() {
            gateway.update_voice_state(call.guild_id, None, false, false);
        }
    }

    pub fn toggle_mute(self: &Rc<Self>) {
        if self.voice_deafened.get() {
            // Unmuting while deafened undeafens too, like Discord.
            self.voice_deafened.set(false);
            self.voice_muted.set(false);
        } else {
            self.voice_muted.set(!self.voice_muted.get());
        }
        self.voice_settings_changed();
    }

    pub fn toggle_deafen(self: &Rc<Self>) {
        self.voice_deafened.set(!self.voice_deafened.get());
        self.voice_settings_changed();
    }

    fn voice_settings_changed(self: &Rc<Self>) {
        let (muted, deafened) = (self.voice_muted(), self.voice_deafened());
        if let Some(call) = &*self.voice.borrow() {
            if let Some(connection) = &call.connection {
                connection.set_muted(muted);
                connection.set_deafened(deafened);
            }
            if call.status == VoiceStatus::Demo {
                let me = self.store.borrow().me().map(|u| u.id);
                if let Some(me) = me {
                    let state = self.voice_state(me, call.guild_id, Some(call.channel_id));
                    self.store
                        .borrow_mut()
                        .apply(Event::VoiceStateUpdate(state));
                }
            } else if let Some(gateway) = &*self.gateway.borrow() {
                // So others see the mute/deafen icons.
                gateway.update_voice_state(call.guild_id, Some(call.channel_id), muted, deafened);
            }
        }
        self.refresh_voice();
    }

    fn voice_state(
        &self,
        user_id: Snowflake,
        guild_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
    ) -> VoiceState {
        VoiceState {
            guild_id,
            channel_id,
            user_id,
            session_id: String::new(),
            self_mute: self.voice_muted(),
            self_deaf: self.voice_deafened(),
            mute: false,
            deaf: false,
            user: None,
        }
    }

    /// Gateway events that matter for our own call. Runs before the store
    /// sees the event.
    pub(super) fn on_voice_gateway_event(self: &Rc<Self>, event: &Event) {
        let me = self.store.borrow().me().map(|u| u.id);
        match event {
            Event::VoiceStateUpdate(state) if Some(state.user_id) == me => {
                let mut voice = self.voice.borrow_mut();
                let Some(call) = voice.as_mut() else {
                    info!(
                        "own voice state (no call in Plume): channel {:?}",
                        state.channel_id
                    );
                    return;
                };
                // Updates about us can also come from Discord on another
                // device, or arrive late from a channel we just left. Only
                // our own session counts; until we know it, only the channel
                // we asked for.
                let ours = match &call.session_id {
                    Some(session_id) => *session_id == state.session_id,
                    None => state.channel_id == Some(call.channel_id),
                };
                info!(
                    "own voice state: channel {:?} ({}), call is in {} ({}), from this session: {ours}",
                    state.channel_id,
                    state
                        .channel_id
                        .map(|c| self.channel_name(c))
                        .unwrap_or_default(),
                    call.channel_id,
                    self.channel_name(call.channel_id),
                );
                if !ours {
                    return;
                }
                match state.channel_id {
                    Some(channel_id) => {
                        if channel_id != call.channel_id {
                            // Moved by a moderator: a new server update follows.
                            call.hang_up();
                            call.channel_id = channel_id;
                            call.guild_id = state.guild_id;
                            call.server = None;
                            call.status = VoiceStatus::Joining;
                        }
                        call.session_id = Some(state.session_id.clone());
                    }
                    None => {
                        call.hang_up();
                        *voice = None;
                        drop(voice);
                        self.toast("Disconnected from voice");
                        self.refresh_voice();
                        return;
                    }
                }
                drop(voice);
                self.try_connect_voice();
            }
            Event::VoiceServerUpdate {
                guild_id,
                channel_id,
                token,
                endpoint,
            } => {
                let mut voice = self.voice.borrow_mut();
                let Some(call) = voice.as_mut() else { return };
                let ours = match call.guild_id {
                    Some(id) => *guild_id == Some(id),
                    None => *channel_id == Some(call.channel_id),
                };
                info!(
                    "voice server update: guild {guild_id:?}, has endpoint: {}, for our call: {ours}",
                    endpoint.is_some()
                );
                if !ours {
                    return;
                }
                // A new server (or none yet, while Discord moves us).
                call.hang_up();
                call.status = VoiceStatus::Connecting;
                call.server = endpoint.clone().map(|endpoint| (token.clone(), endpoint));
                drop(voice);
                self.try_connect_voice();
                self.refresh_voice();
            }
            _ => {}
        }
    }

    fn try_connect_voice(self: &Rc<Self>) {
        let Some(user_id) = self.store.borrow().me().map(|u| u.id) else {
            return;
        };
        let mut voice = self.voice.borrow_mut();
        let Some(call) = voice.as_mut() else { return };
        let (Some(session_id), Some((token, endpoint))) = (&call.session_id, &call.server) else {
            return;
        };
        if call.connection.is_some() {
            return;
        }
        let server = VoiceServer {
            guild_id: call.guild_id,
            channel_id: call.channel_id,
            user_id,
            session_id: session_id.clone(),
            token: token.clone(),
            endpoint: endpoint.clone(),
        };
        let settings = VoiceSettings {
            muted: self.voice_muted(),
            deafened: self.voice_deafened(),
            threshold_db: self.options.borrow().voice_threshold,
            input_gain_db: self.options.borrow().mic_gain,
            auto_gain: self.options.borrow().auto_gain,
            noise_suppression: self.options.borrow().noise_suppression,
            push_to_talk: self.options.borrow().push_to_talk,
            input_device: Some(self.options.borrow().input_device.clone())
                .filter(|d| !d.is_empty()),
            output_device: Some(self.options.borrow().output_device.clone())
                .filter(|d| !d.is_empty()),
            ..VoiceSettings::default()
        };
        info!(
            "connecting voice to {} ({})",
            call.channel_id,
            self.channel_name(call.channel_id)
        );
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let connection = {
            let _runtime = bridge::runtime().enter();
            VoiceConnection::connect(server, settings, events_tx)
        };
        for (&user, &percent) in &self.options.borrow().user_volumes {
            connection.set_user_volume(user, percent as f32 / 100.0);
        }
        call.generation += 1;
        call.status = VoiceStatus::Connecting;
        call.connection = Some(connection);
        let generation = call.generation;

        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Some(event) = events.recv().await {
                let Some(state) = weak.upgrade() else { break };
                state.on_voice_event(generation, event);
            }
        });
    }

    pub(super) fn apply_voice_options(&self, options: &crate::settings::Options) {
        if let Some(connection) = self
            .voice
            .borrow()
            .as_ref()
            .and_then(|c| c.connection.as_ref())
        {
            connection.set_threshold(options.voice_threshold);
            connection.set_gain(options.mic_gain, options.auto_gain);
            connection.set_noise_suppression(options.noise_suppression);
            connection.set_push_to_talk(options.push_to_talk, false);
        }
    }

    /// The push-to-talk key went down or up. Returns whether it was ours.
    pub fn push_to_talk_key(&self, key: gtk::gdk::Key, pressed: bool) -> bool {
        let options = self.options.borrow();
        if !options.push_to_talk || key.name().as_deref() != Some(options.push_to_talk_key.as_str())
        {
            return false;
        }
        let voice = self.voice.borrow();
        let Some(connection) = voice.as_ref().and_then(|c| c.connection.as_ref()) else {
            return false;
        };
        connection.set_push_to_talk(true, pressed);
        true
    }

    pub(super) fn apply_user_volume(&self, user_id: Snowflake, percent: u32) {
        if let Some(connection) = self
            .voice
            .borrow()
            .as_ref()
            .and_then(|c| c.connection.as_ref())
        {
            connection.set_user_volume(user_id, percent as f32 / 100.0);
        }
    }

    fn channel_name(&self, channel_id: Snowflake) -> String {
        self.store
            .borrow()
            .channel(channel_id)
            .and_then(|c| c.name.clone())
            .unwrap_or_else(|| "?".to_owned())
    }

    fn on_voice_event(self: &Rc<Self>, generation: u32, event: VoiceEvent) {
        if !matches!(event, VoiceEvent::Speaking { .. }) {
            info!("voice event (connection {generation}): {event:?}");
        }
        let mut voice = self.voice.borrow_mut();
        let Some(call) = voice.as_mut().filter(|c| c.generation == generation) else {
            return;
        };
        match event {
            VoiceEvent::Connected { encrypted } => {
                call.status = VoiceStatus::Connected { encrypted };
            }
            VoiceEvent::Devices { input, output } => {
                drop(voice);
                if !input {
                    self.toast("No microphone found. You can still listen.");
                }
                if !output {
                    self.toast("No speakers found");
                }
                return;
            }
            VoiceEvent::Speaking { user_id, speaking } => {
                let changed = if speaking {
                    call.speaking.insert(user_id)
                } else {
                    call.speaking.remove(&user_id)
                };
                drop(voice);
                if changed {
                    self.ui().channels.set_speaking(user_id, speaking);
                    self.ui().voice_panel.update(self);
                }
                return;
            }
            VoiceEvent::PrivacyCode(code) => call.privacy_code = code,
            VoiceEvent::Disconnected { reason } => {
                drop(voice);
                self.end_call();
                if let Some(reason) = reason {
                    self.toast(&format!("Voice: {reason}"));
                }
            }
        }
        self.refresh_voice_later();
    }

    /// Update the voice panel and the participants in the channel list.
    pub fn refresh_voice(self: &Rc<Self>) {
        let ui = self.ui();
        ui.voice_panel.update(self);
        ui.user_panel.update(self);
        self.refresh_sidebar();
    }

    /// Like [`Self::refresh_voice`], from places that may hold borrows.
    fn refresh_voice_later(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.refresh_voice();
            }
        });
    }
}
