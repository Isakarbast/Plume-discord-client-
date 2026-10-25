//! Voice chat: joining a call, sending your microphone and playing everyone
//! else.
//!
//! Joining takes two steps. First the app asks the main gateway to join
//! (`GatewayHandle::update_voice_state`). Discord answers with a
//! `VoiceStateUpdate` (our `session_id`) and a `VoiceServerUpdate` (where to
//! connect). Put those into a [`VoiceServer`] and call
//! [`VoiceConnection::connect`].
//!
//! Inside: a WebSocket for signalling ([`voice_gateway`]), UDP for the
//! audio ([`udp`]), Opus ([`opus`]), DAVE end-to-end encryption ([`dave`])
//! and the sound card ([`audio_io`]).

pub mod audio_io;
pub mod dave;
pub mod opus;
mod session;
pub mod udp;
pub mod voice_gateway;

use plume_core::Snowflake;
use tokio::sync::mpsc;

/// Everything needed to connect to a voice server.
#[derive(Clone)]
pub struct VoiceServer {
    /// `None` for DM and group calls.
    pub guild_id: Option<Snowflake>,
    pub channel_id: Snowflake,
    pub user_id: Snowflake,
    /// From our own `VoiceStateUpdate`.
    pub session_id: String,
    /// From `VoiceServerUpdate`. A voice token, not the account token.
    pub token: String,
    pub endpoint: String,
}

impl std::fmt::Debug for VoiceServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceServer")
            .field("guild_id", &self.guild_id)
            .field("channel_id", &self.channel_id)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct VoiceSettings {
    pub muted: bool,
    pub deafened: bool,
    /// Microphone level (dBFS) above which we transmit.
    pub threshold_db: f32,
    /// Extra microphone gain in dB (0 = as is).
    pub input_gain_db: f32,
    /// Bring quiet microphones up to a normal level automatically.
    pub auto_gain: bool,
    /// RNNoise noise suppression.
    pub noise_suppression: bool,
    /// Only send while the push-to-talk key is held.
    pub push_to_talk: bool,
    /// Device names from [`audio_io::input_devices`]; `None` = system default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub bitrate: i32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        VoiceSettings {
            muted: false,
            deafened: false,
            threshold_db: -50.0,
            input_gain_db: 0.0,
            auto_gain: true,
            noise_suppression: true,
            push_to_talk: false,
            input_device: None,
            output_device: None,
            bitrate: 64_000,
        }
    }
}

/// What the UI hears about a voice connection.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceEvent {
    /// Audio can flow. `encrypted` says whether DAVE end-to-end encryption
    /// is on (Discord turns it on for most calls).
    Connected {
        encrypted: bool,
    },
    /// Which audio devices opened.
    Devices {
        input: bool,
        output: bool,
    },
    Speaking {
        user_id: Snowflake,
        speaking: bool,
    },
    /// The end-to-end encryption code changed (`None`: not encrypted).
    PrivacyCode(Option<String>),
    /// The connection is gone. `reason: None` means we left on purpose.
    Disconnected {
        reason: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("websocket: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("network: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad voice message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Protocol(String),
    #[error("opus: {0}")]
    Codec(String),
    #[error("{}", voice_gateway::describe_close(*code, reason))]
    Closed { code: Option<u16>, reason: String },
}

enum Control {
    Mute(bool),
    Deafen(bool),
    Threshold(f32),
    Gain { db: f32, auto: bool },
    NoiseSuppression(bool),
    PushToTalk { enabled: bool, held: bool },
    UserVolume { user_id: u64, gain: f32 },
    Disconnect,
}

/// A running voice connection. Dropping it hangs up.
pub struct VoiceConnection {
    control: mpsc::UnboundedSender<Control>,
}

impl VoiceConnection {
    /// Start connecting on the current Tokio runtime. Progress and errors
    /// arrive on `events`.
    pub fn connect(
        server: VoiceServer,
        settings: VoiceSettings,
        events: mpsc::UnboundedSender<VoiceEvent>,
    ) -> Self {
        let (control, commands) = mpsc::unbounded_channel();
        tokio::spawn(session::run(server, settings, commands, events));
        VoiceConnection { control }
    }

    pub fn set_muted(&self, muted: bool) {
        let _ = self.control.send(Control::Mute(muted));
    }

    pub fn set_deafened(&self, deafened: bool) {
        let _ = self.control.send(Control::Deafen(deafened));
    }

    pub fn set_threshold(&self, threshold_db: f32) {
        let _ = self.control.send(Control::Threshold(threshold_db));
    }

    pub fn set_gain(&self, db: f32, auto: bool) {
        let _ = self.control.send(Control::Gain { db, auto });
    }

    pub fn set_noise_suppression(&self, on: bool) {
        let _ = self.control.send(Control::NoiseSuppression(on));
    }

    /// Push-to-talk mode, and whether the key is held right now.
    pub fn set_push_to_talk(&self, enabled: bool, held: bool) {
        let _ = self.control.send(Control::PushToTalk { enabled, held });
    }

    /// How loud one person is for us (1.0 = normal, 2.0 = double).
    pub fn set_user_volume(&self, user_id: Snowflake, gain: f32) {
        let _ = self.control.send(Control::UserVolume {
            user_id: user_id.0,
            gain,
        });
    }

    pub fn disconnect(&self) {
        let _ = self.control.send(Control::Disconnect);
    }
}
