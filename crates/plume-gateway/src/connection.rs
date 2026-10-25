use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use plume_core::{Event, Snowflake, Token};
use serde::Deserialize;
use serde_json::{json, value::RawValue};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{Instant, interval_at};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{debug, info, warn};

use crate::compression::ZlibStream;
use crate::dispatch;
use crate::error::GatewayError;
use crate::heartbeat::{Heartbeat, jitter};
use crate::opcodes;
use crate::resume::{Backoff, ResumeState};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub url: String,
    /// Use `zlib-stream` transport compression. Cuts bandwidth ~5–10x.
    pub compress: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        GatewayConfig {
            url: "wss://gateway.discord.gg".to_owned(),
            compress: true,
        }
    }
}

/// Things the app can ask the gateway to do while it runs.
#[derive(Debug, Clone)]
pub enum Command {
    /// Ask for live events from a channel the user is looking at. Discord
    /// doesn't send user accounts events from large servers otherwise.
    SubscribeChannel {
        guild_id: Snowflake,
        channel_id: Snowflake,
    },
    /// Our online status for this session (`Offline` = invisible), with an
    /// optional custom status line.
    SetStatus(plume_core::models::Status, Option<String>),
    /// Join a voice channel (`channel_id: Some`) or leave voice (`None`).
    /// `guild_id` is `None` for DM calls.
    UpdateVoiceState {
        guild_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
        self_mute: bool,
        self_deaf: bool,
    },
}

/// A cheap, cloneable way to send [`Command`]s to a running [`Gateway`].
#[derive(Clone)]
pub struct GatewayHandle {
    commands: mpsc::UnboundedSender<Command>,
}

impl GatewayHandle {
    pub fn subscribe_channel(&self, guild_id: Snowflake, channel_id: Snowflake) {
        // If the gateway has stopped there's nothing to subscribe to anyway.
        let _ = self.commands.send(Command::SubscribeChannel {
            guild_id,
            channel_id,
        });
    }

    pub fn set_status(&self, status: plume_core::models::Status, custom: Option<String>) {
        let _ = self.commands.send(Command::SetStatus(status, custom));
    }

    /// Join, move or leave voice. Discord answers with `VoiceStateUpdate`
    /// and `VoiceServerUpdate`, which `plume-voice` needs to connect.
    pub fn update_voice_state(
        &self,
        guild_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
        self_mute: bool,
        self_deaf: bool,
    ) {
        let _ = self.commands.send(Command::UpdateVoiceState {
            guild_id,
            channel_id,
            self_mute,
            self_deaf,
        });
    }
}

/// One gateway message: `{"op": …, "d": …, "s": …, "t": …}`.
/// `d` stays raw JSON until we know what type it should become.
#[derive(Deserialize)]
struct Payload<'a> {
    op: u8,
    #[serde(borrow)]
    d: Option<&'a RawValue>,
    s: Option<u64>,
    t: Option<&'a str>,
}

/// Why a single connection ended without an error.
enum SessionEnd {
    /// Reconnect right away (Discord asked, or the connection went zombie).
    Reconnect,
    /// Whoever was receiving our events is gone, so there's no point running.
    ReceiverDropped,
}

pub struct Gateway {
    token: Token,
    config: GatewayConfig,
    events: mpsc::Sender<Event>,
    resume: Option<ResumeState>,
    /// Last sequence number seen. Survives reconnects so we can resume.
    sequence: Option<u64>,
    backoff: Backoff,
    commands: mpsc::UnboundedReceiver<Command>,
    command_sender: mpsc::UnboundedSender<Command>,
    /// The channel we watch in each server, re-sent after reconnecting.
    subscriptions: HashMap<Snowflake, Snowflake>,
    /// Re-sent after every (re)connect.
    status: Option<(plume_core::models::Status, Option<String>)>,
}

impl Gateway {
    pub fn new(token: Token, events: mpsc::Sender<Event>) -> Self {
        Gateway::with_config(token, GatewayConfig::default(), events)
    }

    pub fn with_config(token: Token, config: GatewayConfig, events: mpsc::Sender<Event>) -> Self {
        let (command_sender, commands) = mpsc::unbounded_channel();
        Gateway {
            token,
            config,
            events,
            resume: None,
            sequence: None,
            backoff: Backoff::default(),
            commands,
            command_sender,
            subscriptions: HashMap::new(),
            status: None,
        }
    }

    /// Get this before calling [`Gateway::run`] to send it commands later.
    pub fn handle(&self) -> GatewayHandle {
        GatewayHandle {
            commands: self.command_sender.clone(),
        }
    }

    /// Runs until a fatal error (like a bad token) or until the event
    /// receiver is dropped. Everything else triggers a reconnect.
    pub async fn run(mut self) -> Result<(), GatewayError> {
        loop {
            match self.session().await {
                Ok(SessionEnd::ReceiverDropped) => return Ok(()),
                Ok(SessionEnd::Reconnect) => info!("reconnecting"),
                Err(error) if error.is_fatal() => return Err(error),
                Err(error) => {
                    if let GatewayError::Closed {
                        code: Some(code), ..
                    } = &error
                        && opcodes::invalidates_session(*code)
                    {
                        self.forget_session();
                    }
                    let delay = self.backoff.next_delay();
                    warn!("gateway connection lost ({error}); retrying in {delay:.1?}");
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// One connection, from connect to disconnect.
    async fn session(&mut self) -> Result<SessionEnd, GatewayError> {
        let base = self
            .resume
            .as_ref()
            .map_or(self.config.url.as_str(), |r| r.resume_url.as_str());
        let mut url = format!("{base}/?v=10&encoding=json");
        if self.config.compress {
            url.push_str("&compress=zlib-stream");
        }

        info!("connecting to {base}");
        let (mut socket, _) = connect_async(url).await?;
        let mut zlib = ZlibStream::new();

        // 1. HELLO tells us how often to heartbeat.
        let hello = loop {
            match next_frame(&mut socket, &mut zlib).await? {
                Frame::Text(text) => break text,
                Frame::Skip => continue,
            }
        };
        let payload: Payload = serde_json::from_str(&hello)?;
        if payload.op != opcodes::HELLO {
            return Err(GatewayError::NoHello(payload.op));
        }
        let interval = parse_hello(payload.d)?;
        let mut heartbeat = Heartbeat::new(interval);
        let mut ticker = interval_at(Instant::now() + heartbeat.first_delay(), interval);

        // 2. Log in: resume the old session if we have one, else start fresh.
        if let (Some(resume), Some(seq)) = (&self.resume, self.sequence) {
            debug!("resuming session at sequence {seq}");
            let resume = json!({
                "op": opcodes::RESUME,
                "d": { "token": self.token.expose(), "session_id": resume.session_id, "seq": seq }
            });
            send_json(&mut socket, &resume).await?;
        } else {
            self.forget_session();
            send_json(&mut socket, &identify_payload(&self.token)).await?;
        }

        // 3. Main loop: heartbeat on the timer, handle commands and whatever Discord sends.
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if heartbeat.is_zombie() {
                        warn!("no heartbeat ACK from Discord; reconnecting");
                        close_for_resume(&mut socket).await;
                        return Ok(SessionEnd::Reconnect);
                    }
                    self.send_heartbeat(&mut socket, &mut heartbeat).await?;
                }

                Some(command) = self.commands.recv() => {
                    match command {
                        Command::SubscribeChannel { guild_id, channel_id } => {
                            self.subscriptions.insert(guild_id, channel_id);
                            send_json(&mut socket, &subscribe_payload(guild_id, channel_id)).await?;
                        }
                        Command::SetStatus(status, custom) => {
                            send_json(&mut socket, &status_payload(status, custom.as_deref())).await?;
                            self.status = Some((status, custom));
                        }
                        Command::UpdateVoiceState { guild_id, channel_id, self_mute, self_deaf } => {
                            let payload = json!({
                                "op": opcodes::VOICE_STATE_UPDATE,
                                "d": {
                                    "guild_id": guild_id,
                                    "channel_id": channel_id,
                                    "self_mute": self_mute,
                                    "self_deaf": self_deaf,
                                    "self_video": false,
                                }
                            });
                            send_json(&mut socket, &payload).await?;
                        }
                    }
                }

                frame = next_frame(&mut socket, &mut zlib) => {
                    let Frame::Text(text) = frame? else { continue };
                    let payload: Payload = serde_json::from_str(&text)?;
                    if let Some(seq) = payload.s {
                        self.sequence = Some(seq);
                    }

                    match payload.op {
                        opcodes::DISPATCH => {
                            let name = payload.t.unwrap_or_default();
                            if !self.handle_dispatch(name, payload.d).await? {
                                return Ok(SessionEnd::ReceiverDropped);
                            }
                            if (name == "READY" || name == "RESUMED")
                                && let Some((status, custom)) = &self.status
                            {
                                send_json(&mut socket, &status_payload(*status, custom.as_deref())).await?;
                            }
                            if name == "READY" || name == "RESUMED" {
                                for (&guild_id, &channel_id) in &self.subscriptions {
                                    send_json(&mut socket, &subscribe_payload(guild_id, channel_id)).await?;
                                }
                            }
                        }
                        opcodes::HEARTBEAT => self.send_heartbeat(&mut socket, &mut heartbeat).await?,
                        opcodes::HEARTBEAT_ACK => {
                            heartbeat.acked();
                            debug!("heartbeat ACK ({:?})", heartbeat.latency);
                        }
                        opcodes::RECONNECT => {
                            info!("Discord asked us to reconnect");
                            close_for_resume(&mut socket).await;
                            return Ok(SessionEnd::Reconnect);
                        }
                        opcodes::INVALID_SESSION => {
                            let resumable = payload.d.is_some_and(|d| d.get() == "true");
                            if !resumable {
                                self.forget_session();
                            }
                            // Discord asks for a random 1–5 second wait here.
                            let wait = Duration::from_secs_f64(1.0 + 4.0 * jitter());
                            warn!("invalid session (resumable: {resumable}); waiting {wait:.1?}");
                            tokio::time::sleep(wait).await;
                            return Ok(SessionEnd::Reconnect);
                        }
                        other => debug!("ignoring op {other}"),
                    }
                }
            }
        }
    }

    /// Decodes an event and passes it on. Returns `false` if nobody's listening.
    async fn handle_dispatch(
        &mut self,
        name: &str,
        data: Option<&RawValue>,
    ) -> Result<bool, GatewayError> {
        let Some(data) = data else {
            return Ok(true);
        };
        let decoded = match dispatch::decode(name, data) {
            Ok(decoded) => decoded,
            Err(error) => {
                // One odd event shouldn't take the whole connection down.
                warn!("couldn't parse {name}: {error}");
                return Ok(true);
            }
        };

        if let Some(session) = decoded.session {
            info!("logged in (session ready)");
            self.resume = Some(session);
            self.backoff.reset();
        }
        if matches!(decoded.event, Event::Resumed) {
            info!("session resumed");
            self.backoff.reset();
        }

        if self.events.send(decoded.event).await.is_err() {
            return Ok(false);
        }
        for event in decoded.extra {
            if self.events.send(event).await.is_err() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn send_heartbeat(
        &self,
        socket: &mut Socket,
        heartbeat: &mut Heartbeat,
    ) -> Result<(), GatewayError> {
        send_json(
            socket,
            &json!({ "op": opcodes::HEARTBEAT, "d": self.sequence }),
        )
        .await?;
        heartbeat.sent();
        Ok(())
    }

    fn forget_session(&mut self) {
        self.resume = None;
        self.sequence = None;
    }
}

/// IDENTIFY. This contains the token, so it must never be logged.
fn identify_payload(token: &Token) -> serde_json::Value {
    json!({
        "op": opcodes::IDENTIFY,
        "d": {
            "token": token.expose(),
            "properties": {
                "os": "Linux",
                "browser": "Discord Client",
                "device": ""
            },
            "compress": false
        }
    })
}

fn status_payload(status: plume_core::models::Status, custom: Option<&str>) -> serde_json::Value {
    use plume_core::models::Status;
    let status = match status {
        Status::Online => "online",
        Status::Idle => "idle",
        Status::Dnd => "dnd",
        Status::Offline => "invisible",
    };
    let activities = match custom.filter(|text| !text.trim().is_empty()) {
        Some(text) => json!([{ "type": 4, "name": "Custom Status", "state": text }]),
        None => json!([]),
    };
    json!({
        "op": opcodes::PRESENCE_UPDATE,
        "d": { "since": 0, "activities": activities, "status": status, "afk": false }
    })
}

/// "Lazy guild" request (op 14): live events for one channel of a server,
/// plus typing and thread updates. This is what the official client sends
/// when you open a channel.
fn subscribe_payload(guild_id: Snowflake, channel_id: Snowflake) -> serde_json::Value {
    json!({
        "op": opcodes::GUILD_SUBSCRIBE,
        "d": {
            "guild_id": guild_id,
            "typing": true,
            "threads": true,
            "activities": true,
            "members": [],
            "channels": { channel_id.to_string(): [[0, 99]] },
            "thread_member_lists": []
        }
    })
}

fn parse_hello(data: Option<&RawValue>) -> Result<Duration, GatewayError> {
    #[derive(Deserialize)]
    struct Hello {
        heartbeat_interval: u64,
    }
    let data = data.ok_or(GatewayError::NoHello(opcodes::HELLO))?;
    let hello: Hello = serde_json::from_str(data.get())?;
    Ok(Duration::from_millis(hello.heartbeat_interval))
}

enum Frame {
    Text(String),
    /// Pings, pongs, or the first half of a split compressed payload.
    Skip,
}

async fn next_frame(socket: &mut Socket, zlib: &mut ZlibStream) -> Result<Frame, GatewayError> {
    let Some(message) = socket.next().await else {
        return Err(GatewayError::Closed {
            code: None,
            reason: "connection ended".to_owned(),
        });
    };
    match message? {
        Message::Text(text) => Ok(Frame::Text(text.to_string())),
        Message::Binary(data) => Ok(zlib.push(&data)?.map_or(Frame::Skip, Frame::Text)),
        Message::Close(frame) => {
            let code = frame.as_ref().map(|f| u16::from(f.code));
            let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
            if code == Some(4004) {
                return Err(GatewayError::AuthenticationFailed);
            }
            Err(GatewayError::Closed { code, reason })
        }
        _ => Ok(Frame::Skip),
    }
}

async fn send_json(socket: &mut Socket, value: &serde_json::Value) -> Result<(), GatewayError> {
    socket.send(Message::text(value.to_string())).await?;
    Ok(())
}

/// Closing with 1000/1001 would end the session on Discord's side, so we use
/// a 4000-range code that keeps it resumable. Errors don't matter here.
async fn close_for_resume(socket: &mut Socket) {
    let frame = CloseFrame {
        code: CloseCode::Library(4000),
        reason: "reconnecting".into(),
    };
    let _ = socket.close(Some(frame)).await;
}
