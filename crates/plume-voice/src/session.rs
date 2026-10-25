//! The voice connection's main loop: signalling, sending and receiving.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use plume_core::Snowflake;
use serde_json::Value;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{Interval, MissedTickBehavior, interval};
use tracing::{debug, info, warn};

use crate::audio_io::{self, AudioIo};
use crate::dave::Dave;
use crate::opus::{MicProcessor, OpusDecoder, OpusEncoder, SILENCE_FRAME, VoiceGate};
use crate::udp::{self, ENCRYPTION_MODE, Transport};
use crate::voice_gateway::{self as vg, Incoming, Outgoing, VoiceSocket, op};
use crate::{Control, VoiceError, VoiceEvent, VoiceServer, VoiceSettings};

/// Someone counts as speaking until this long after their last packet.
const SPEAKING_TIMEOUT: Duration = Duration::from_millis(250);
const KEEPALIVE_EVERY: Duration = Duration::from_secs(5);
const MAX_RESUMES: u32 = 5;
/// Sent after we stop talking so receivers' decoders fade out cleanly.
const TRAILING_SILENCE_FRAMES: u8 = 5;

pub(crate) async fn run(
    server: VoiceServer,
    settings: VoiceSettings,
    control: mpsc::UnboundedReceiver<Control>,
    events: mpsc::UnboundedSender<VoiceEvent>,
) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let reason = match Session::start(server, settings, control, events.clone()).await {
        Ok(mut session) => session.run().await,
        Err(error) => Some(error.to_string()),
    };
    if let Some(reason) = &reason {
        warn!("voice disconnected: {reason}");
    }
    let _ = events.send(VoiceEvent::Disconnected { reason });
}

enum End {
    /// We hung up (or the app dropped the connection).
    Left,
}

struct Remote {
    user_id: Option<u64>,
    decoder: Option<OpusDecoder>,
    last_sequence: Option<u16>,
    last_heard: Option<Instant>,
}

impl Remote {
    fn new() -> Self {
        Remote {
            user_id: None,
            decoder: None,
            last_sequence: None,
            last_heard: None,
        }
    }
}

struct Session {
    server: VoiceServer,
    settings: VoiceSettings,
    control: mpsc::UnboundedReceiver<Control>,
    events: mpsc::UnboundedSender<VoiceEvent>,

    socket: Option<VoiceSocket>,
    heartbeat: Option<Interval>,
    seq_ack: Option<u64>,

    ssrc: u32,
    udp: Option<UdpSocket>,
    transport: Option<Transport>,
    dave: Dave,
    privacy_code: Option<String>,

    audio: AudioIo,
    frames: mpsc::Receiver<Vec<f32>>,
    encoder: OpusEncoder,
    gate: VoiceGate,
    /// Push-to-talk: whether the key is down, and a short release delay.
    ptt_held: bool,
    ptt: crate::opus::Hold,
    mic: MicProcessor,
    transmitting: bool,
    silence_left: u8,
    remotes: HashMap<u32, Remote>,
    /// Per-person volume, applied once we know their SSRC.
    user_volumes: HashMap<u64, f32>,
}

impl Session {
    async fn start(
        server: VoiceServer,
        settings: VoiceSettings,
        control: mpsc::UnboundedReceiver<Control>,
        events: mpsc::UnboundedSender<VoiceEvent>,
    ) -> Result<Self, VoiceError> {
        let (frame_tx, frames) = mpsc::channel(8);
        let audio = audio_io::start(
            frame_tx,
            settings.input_device.clone(),
            settings.output_device.clone(),
        )
        .await;
        audio.mixer.set_deafened(settings.deafened);
        let _ = events.send(VoiceEvent::Devices {
            input: audio.has_input,
            output: audio.has_output,
        });
        Ok(Session {
            dave: Dave::new(server.user_id.0, server.channel_id.0),
            server,
            control,
            events,
            socket: None,
            heartbeat: None,
            seq_ack: None,
            ssrc: 0,
            udp: None,
            transport: None,
            privacy_code: None,
            audio,
            frames,
            encoder: OpusEncoder::new(settings.bitrate)?,
            gate: VoiceGate::new(settings.threshold_db),
            ptt_held: false,
            ptt: crate::opus::Hold::default(),
            mic: {
                let mut mic = MicProcessor::new(settings.input_gain_db, settings.auto_gain);
                mic.set_noise_suppression(settings.noise_suppression);
                mic
            },
            transmitting: false,
            silence_left: 0,
            remotes: HashMap::new(),
            user_volumes: HashMap::new(),
            settings,
        })
    }

    /// Returns why we disconnected, or `None` if we left on purpose.
    async fn run(&mut self) -> Option<String> {
        let mut resumes = 0;
        let mut resume = false;
        loop {
            match self.connection(resume).await {
                Ok(End::Left) => {
                    if let Some(socket) = &mut self.socket {
                        socket.close().await;
                    }
                    return None;
                }
                Err(VoiceError::Closed { code, reason })
                    if vg::is_resumable(code)
                        && self.transport.is_some()
                        && resumes < MAX_RESUMES =>
                {
                    resumes += 1;
                    info!("voice connection dropped ({code:?} {reason}); resuming");
                    tokio::time::sleep(Duration::from_secs(u64::from(resumes))).await;
                    resume = true;
                }
                Err(error) => return Some(error.to_string()),
            }
        }
    }

    /// One WebSocket connection, until it closes.
    async fn connection(&mut self, resume: bool) -> Result<End, VoiceError> {
        self.heartbeat = None;
        let mut socket = VoiceSocket::connect(&self.server.endpoint).await?;
        let hello = if resume {
            vg::resume(&self.server, self.seq_ack)
        } else {
            vg::identify(&self.server, Dave::max_version())
        };
        socket.send(hello).await?;
        self.socket = Some(socket);

        let mut tick = interval(Duration::from_millis(100));
        let mut last_keepalive = Instant::now();
        let mut keepalive_count = 0u32;
        let mut buf = vec![0u8; 2048];

        loop {
            let socket = self.socket.as_mut().expect("socket is set above");
            tokio::select! {
                message = socket.next() => self.handle_message(message?).await?,

                _ = tick_opt(&mut self.heartbeat) => {
                    let nonce = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as u64);
                    self.send(vg::heartbeat(nonce, self.seq_ack)).await?;
                }

                Some(mut frame) = self.frames.recv() => self.send_audio(&mut frame).await?,

                received = recv_opt(self.udp.as_ref(), &mut buf) => match received {
                    Ok(len) => self.receive_audio(&buf[..len]),
                    // e.g. a stray ICMP "port unreachable"; the next packet may be fine.
                    Err(error) => debug!("voice UDP receive failed: {error}"),
                },

                command = self.control.recv() => match command {
                    Some(Control::Mute(muted)) => self.settings.muted = muted,
                    Some(Control::Deafen(deafened)) => {
                        self.settings.deafened = deafened;
                        self.audio.mixer.set_deafened(deafened);
                    }
                    Some(Control::Threshold(db)) => {
                        self.settings.threshold_db = db;
                        self.gate.set_threshold(db);
                    }
                    Some(Control::Gain { db, auto }) => self.mic.set_gain(db, auto),
                    Some(Control::NoiseSuppression(on)) => self.mic.set_noise_suppression(on),
                    Some(Control::PushToTalk { enabled, held }) => {
                        self.settings.push_to_talk = enabled;
                        self.ptt_held = held;
                    }
                    Some(Control::UserVolume { user_id, gain }) => {
                        self.user_volumes.insert(user_id, gain);
                        for (&ssrc, remote) in &self.remotes {
                            if remote.user_id == Some(user_id) {
                                self.audio.mixer.set_gain(ssrc, gain);
                            }
                        }
                    }
                    Some(Control::Disconnect) | None => return Ok(End::Left),
                },

                _ = tick.tick() => {
                    self.expire_speakers();
                    if let Some(udp) = &self.udp
                        && last_keepalive.elapsed() >= KEEPALIVE_EVERY
                    {
                        last_keepalive = Instant::now();
                        keepalive_count = keepalive_count.wrapping_add(1);
                        udp.send(&udp::keepalive(keepalive_count)).await?;
                    }
                }
            }
        }
    }

    async fn send(&mut self, message: Outgoing) -> Result<(), VoiceError> {
        match &mut self.socket {
            Some(socket) => socket.send(message).await,
            None => Ok(()),
        }
    }

    async fn send_all(&mut self, messages: Vec<Outgoing>) -> Result<(), VoiceError> {
        for message in messages {
            self.send(message).await?;
        }
        self.check_privacy_code();
        Ok(())
    }

    async fn handle_message(&mut self, message: Incoming) -> Result<(), VoiceError> {
        match message {
            Incoming::Json { op, data, seq } => {
                if seq.is_some() {
                    self.seq_ack = seq;
                }
                self.handle_json(op, data).await
            }
            Incoming::Binary { seq, op, payload } => {
                self.seq_ack = Some(u64::from(seq));
                let replies = self.dave.handle_binary(op, &payload);
                self.send_all(replies).await
            }
        }
    }

    async fn handle_json(&mut self, opcode: u8, data: Value) -> Result<(), VoiceError> {
        match opcode {
            op::HELLO => {
                let millis = data["heartbeat_interval"].as_f64().unwrap_or(13_750.0);
                let mut heartbeat = interval(Duration::from_secs_f64(millis / 1000.0));
                heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
                self.heartbeat = Some(heartbeat);
            }
            op::READY => self.ready(&data).await?,
            op::SESSION_DESCRIPTION => {
                let key: Vec<u8> = data["secret_key"]
                    .as_array()
                    .map(|bytes| {
                        bytes
                            .iter()
                            .filter_map(|b| b.as_u64())
                            .map(|b| b as u8)
                            .collect()
                    })
                    .unwrap_or_default();
                self.transport = Some(Transport::new(&key, self.ssrc)?);
                let version = data["dave_protocol_version"].as_u64().unwrap_or(0) as u16;
                let replies = self.dave.start(version);
                self.send_all(replies).await?;
                info!("voice connected (end-to-end encryption: {})", version > 0);
                let _ = self.events.send(VoiceEvent::Connected {
                    encrypted: version > 0,
                });
            }
            op::RESUMED => info!("voice connection resumed"),
            op::HEARTBEAT_ACK => {}
            op::SPEAKING => {
                if let (Some(ssrc), Some(user_id)) =
                    (data["ssrc"].as_u64(), snowflake(&data["user_id"]))
                {
                    self.remote(ssrc as u32).user_id = Some(user_id);
                    if let Some(&gain) = self.user_volumes.get(&user_id) {
                        self.audio.mixer.set_gain(ssrc as u32, gain);
                    }
                }
            }
            op::CLIENTS_CONNECT => {
                let ids = data["user_ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(snowflake);
                self.dave.clients_connected(ids);
            }
            op::CLIENT_DISCONNECT => {
                if let Some(user_id) = snowflake(&data["user_id"]) {
                    self.dave.client_disconnected(user_id);
                    let gone: Vec<u32> = self
                        .remotes
                        .iter()
                        .filter(|(_, r)| r.user_id == Some(user_id))
                        .map(|(&ssrc, _)| ssrc)
                        .collect();
                    for ssrc in gone {
                        self.remotes.remove(&ssrc);
                        self.audio.mixer.remove(ssrc);
                    }
                    self.speaking(user_id, false);
                }
            }
            op::DAVE_PREPARE_TRANSITION | op::DAVE_EXECUTE_TRANSITION | op::DAVE_PREPARE_EPOCH => {
                let replies = self.dave.handle_json(opcode, &data);
                self.send_all(replies).await?;
            }
            other => debug!("ignoring voice op {other}"),
        }
        Ok(())
    }

    /// READY: open UDP, learn our public address and pick encryption.
    async fn ready(&mut self, data: &Value) -> Result<(), VoiceError> {
        self.ssrc = data["ssrc"].as_u64().unwrap_or(0) as u32;
        let ip = data["ip"].as_str().unwrap_or_default();
        let port = data["port"].as_u64().unwrap_or(0) as u16;
        let offers_mode = data["modes"]
            .as_array()
            .is_some_and(|modes| modes.iter().any(|m| m.as_str() == Some(ENCRYPTION_MODE)));
        if !offers_mode {
            return Err(VoiceError::Protocol(
                "the voice server doesn't support our encryption mode".into(),
            ));
        }

        let socket = udp::connect(ip, port).await?;
        let (my_ip, my_port) = udp::discover_ip(&socket, self.ssrc).await?;
        debug!("voice UDP ready, ssrc {}", self.ssrc);
        self.udp = Some(socket);
        self.send(vg::select_protocol(&my_ip, my_port, ENCRYPTION_MODE))
            .await
    }

    async fn send_audio(&mut self, pcm: &mut [f32]) -> Result<(), VoiceError> {
        if self.transport.is_none() {
            return Ok(());
        }
        self.mic.clean(pcm);
        let open = !self.settings.muted
            && if self.settings.push_to_talk {
                self.ptt.process(self.ptt_held)
            } else {
                self.gate.process(pcm)
            };
        if !open {
            if self.settings.muted {
                self.gate.close();
            }
            return self.wind_down().await;
        }

        if !self.transmitting {
            self.transmitting = true;
            self.send(vg::speaking(self.ssrc, true)).await?;
            self.speaking(self.server.user_id.0, true);
        }
        let loud = crate::opus::level_db(pcm) >= self.settings.threshold_db;
        self.mic.amplify(pcm, loud);
        let opus = self.encoder.encode(pcm)?.to_vec();
        if let Some(frame) = self.dave.encrypt(&opus) {
            self.send_packet(&frame).await?;
        }
        self.silence_left = TRAILING_SILENCE_FRAMES;
        Ok(())
    }

    /// After we stop talking: a few silence frames, then "not speaking".
    async fn wind_down(&mut self) -> Result<(), VoiceError> {
        if !self.transmitting {
            return Ok(());
        }
        if self.silence_left > 0 {
            self.silence_left -= 1;
            return self.send_packet(&SILENCE_FRAME).await;
        }
        self.transmitting = false;
        self.speaking(self.server.user_id.0, false);
        self.send(vg::speaking(self.ssrc, false)).await
    }

    async fn send_packet(&mut self, frame: &[u8]) -> Result<(), VoiceError> {
        if let (Some(transport), Some(udp)) = (&mut self.transport, &self.udp) {
            let packet = transport.seal(frame)?;
            udp.send(&packet).await?;
        }
        Ok(())
    }

    fn receive_audio(&mut self, packet: &[u8]) {
        let Some(incoming) = self.transport.as_ref().and_then(|t| t.open(packet)) else {
            return;
        };
        if incoming.payload == SILENCE_FRAME || self.settings.deafened {
            return;
        }

        let remote = self.remote(incoming.ssrc);
        // Late packets would only make things sound worse.
        if let Some(last) = remote.last_sequence
            && (incoming.sequence.wrapping_sub(last) as i16) <= 0
        {
            return;
        }
        remote.last_sequence = Some(incoming.sequence);
        let user_id = remote.user_id;

        let Some(opus) = self.dave.decrypt(user_id, &incoming.payload) else {
            return;
        };
        let remote = self
            .remotes
            .entry(incoming.ssrc)
            .or_insert_with(Remote::new);
        if remote.decoder.is_none() {
            remote.decoder = OpusDecoder::new().ok();
        }
        let Some(decoder) = &mut remote.decoder else {
            return;
        };
        match decoder.decode(&opus) {
            Ok(pcm) => self.audio.mixer.push(incoming.ssrc, pcm),
            Err(error) => debug!("dropping undecodable packet: {error}"),
        }

        let remote = self.remote(incoming.ssrc);
        let started = remote.last_heard.is_none();
        remote.last_heard = Some(Instant::now());
        if started && let Some(user_id) = user_id {
            self.speaking(user_id, true);
        }
    }

    fn remote(&mut self, ssrc: u32) -> &mut Remote {
        self.remotes.entry(ssrc).or_insert_with(Remote::new)
    }

    fn expire_speakers(&mut self) {
        let mut stopped = Vec::new();
        for remote in self.remotes.values_mut() {
            if remote
                .last_heard
                .is_some_and(|t| t.elapsed() > SPEAKING_TIMEOUT)
            {
                remote.last_heard = None;
                stopped.extend(remote.user_id);
            }
        }
        for user_id in stopped {
            self.speaking(user_id, false);
        }
    }

    fn speaking(&self, user_id: u64, speaking: bool) {
        let _ = self.events.send(VoiceEvent::Speaking {
            user_id: Snowflake(user_id),
            speaking,
        });
    }

    fn check_privacy_code(&mut self) {
        let code = self.dave.privacy_code();
        if code != self.privacy_code {
            self.privacy_code = code.clone();
            let _ = self.events.send(VoiceEvent::PrivacyCode(code));
        }
    }
}

fn snowflake(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_u64())
}

async fn tick_opt(interval: &mut Option<Interval>) {
    match interval {
        Some(interval) => {
            interval.tick().await;
        }
        None => std::future::pending().await,
    }
}

async fn recv_opt(socket: Option<&UdpSocket>, buf: &mut [u8]) -> std::io::Result<usize> {
    match socket {
        Some(socket) => socket.recv(buf).await,
        None => std::future::pending().await,
    }
}
