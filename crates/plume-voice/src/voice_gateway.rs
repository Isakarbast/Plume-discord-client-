//! The voice WebSocket (version 8): signalling for one voice connection.
//!
//! See <https://discord.com/developers/docs/topics/voice-connections>.

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::{VoiceError, VoiceServer};

/// Voice opcodes. 21 and up belong to DAVE (end-to-end encryption).
pub mod op {
    pub const IDENTIFY: u8 = 0;
    pub const SELECT_PROTOCOL: u8 = 1;
    pub const READY: u8 = 2;
    pub const HEARTBEAT: u8 = 3;
    pub const SESSION_DESCRIPTION: u8 = 4;
    pub const SPEAKING: u8 = 5;
    pub const HEARTBEAT_ACK: u8 = 6;
    pub const RESUME: u8 = 7;
    pub const HELLO: u8 = 8;
    pub const RESUMED: u8 = 9;
    pub const CLIENTS_CONNECT: u8 = 11;
    pub const CLIENT_DISCONNECT: u8 = 13;
    pub const DAVE_PREPARE_TRANSITION: u8 = 21;
    pub const DAVE_EXECUTE_TRANSITION: u8 = 22;
    pub const DAVE_TRANSITION_READY: u8 = 23;
    pub const DAVE_PREPARE_EPOCH: u8 = 24;
    pub const DAVE_MLS_EXTERNAL_SENDER: u8 = 25;
    pub const DAVE_MLS_KEY_PACKAGE: u8 = 26;
    pub const DAVE_MLS_PROPOSALS: u8 = 27;
    pub const DAVE_MLS_COMMIT_WELCOME: u8 = 28;
    pub const DAVE_MLS_ANNOUNCE_COMMIT_TRANSITION: u8 = 29;
    pub const DAVE_MLS_WELCOME: u8 = 30;
    pub const DAVE_MLS_INVALID_COMMIT_WELCOME: u8 = 31;
}

/// Something to send to the voice server.
#[derive(Debug)]
pub enum Outgoing {
    Json(Value),
    /// `[opcode][payload…]`, used for DAVE's MLS messages.
    Binary(Vec<u8>),
}

impl Outgoing {
    pub fn json(opcode: u8, data: Value) -> Self {
        Outgoing::Json(json!({ "op": opcode, "d": data }))
    }

    pub fn binary(opcode: u8, payload: &[u8]) -> Self {
        let mut data = Vec::with_capacity(payload.len() + 1);
        data.push(opcode);
        data.extend_from_slice(payload);
        Outgoing::Binary(data)
    }
}

/// Something the voice server sent.
#[derive(Debug)]
pub enum Incoming {
    Json {
        op: u8,
        data: Value,
        seq: Option<u64>,
    },
    /// `[seq: u16][opcode][payload…]`.
    Binary { seq: u16, op: u8, payload: Vec<u8> },
}

#[derive(Deserialize)]
struct Payload {
    op: u8,
    #[serde(default)]
    d: Value,
    seq: Option<u64>,
}

pub struct VoiceSocket {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

impl VoiceSocket {
    pub async fn connect(endpoint: &str) -> Result<Self, VoiceError> {
        let host = endpoint.trim_start_matches("wss://").trim_end_matches('/');
        let (ws, _) = connect_async(format!("wss://{host}/?v=8")).await?;
        Ok(VoiceSocket { ws })
    }

    pub async fn send(&mut self, message: Outgoing) -> Result<(), VoiceError> {
        let message = match message {
            Outgoing::Json(value) => Message::text(value.to_string()),
            Outgoing::Binary(data) => Message::binary(data),
        };
        self.ws.send(message).await?;
        Ok(())
    }

    /// The next message; skips pings and other noise.
    pub async fn next(&mut self) -> Result<Incoming, VoiceError> {
        loop {
            let Some(message) = self.ws.next().await else {
                return Err(VoiceError::Closed {
                    code: None,
                    reason: "connection ended".into(),
                });
            };
            match message? {
                Message::Text(text) => {
                    let payload: Payload = serde_json::from_str(&text)?;
                    return Ok(Incoming::Json {
                        op: payload.op,
                        data: payload.d,
                        seq: payload.seq,
                    });
                }
                Message::Binary(data) if data.len() >= 3 => {
                    return Ok(Incoming::Binary {
                        seq: u16::from_be_bytes([data[0], data[1]]),
                        op: data[2],
                        payload: data[3..].to_vec(),
                    });
                }
                Message::Close(frame) => {
                    return Err(VoiceError::Closed {
                        code: frame.as_ref().map(|f| u16::from(f.code)),
                        reason: frame.map(|f| f.reason.to_string()).unwrap_or_default(),
                    });
                }
                _ => {}
            }
        }
    }

    pub async fn close(&mut self) {
        let frame = CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        };
        let _ = self.ws.close(Some(frame)).await;
    }
}

/// Close codes after which resuming may work.
pub fn is_resumable(code: Option<u16>) -> bool {
    matches!(
        code,
        None // network trouble
        | Some(1001) // going away
        | Some(1006) // abnormal closure
        | Some(4015) // voice server crashed
    )
}

/// A friendlier message for the close codes people actually hit.
pub fn describe_close(code: Option<u16>, reason: &str) -> String {
    let what = match code {
        Some(4004) => "voice authentication failed",
        Some(4006) => "voice session expired",
        Some(4009) => "voice session timed out",
        Some(4011) => "voice server not found",
        Some(4014) => "disconnected from the voice channel",
        Some(4016) => "unsupported voice encryption",
        Some(4017) => "this call requires end-to-end encryption",
        Some(4021) => "rate limited by Discord",
        Some(4022) => "the call ended",
        _ if reason.is_empty() => "voice connection closed",
        _ => reason,
    };
    match code {
        Some(code) => format!("{what} ({code})"),
        None => what.to_owned(),
    }
}

/// DM calls identify with the channel ID instead of a server ID.
fn server_id(server: &VoiceServer) -> String {
    server.guild_id.unwrap_or(server.channel_id).to_string()
}

pub fn identify(server: &VoiceServer, max_dave_version: u16) -> Outgoing {
    Outgoing::json(
        op::IDENTIFY,
        json!({
            "server_id": server_id(server),
            "user_id": server.user_id.to_string(),
            "session_id": server.session_id,
            "token": server.token,
            "max_dave_protocol_version": max_dave_version,
        }),
    )
}

pub fn resume(server: &VoiceServer, seq_ack: Option<u64>) -> Outgoing {
    Outgoing::json(
        op::RESUME,
        json!({
            "server_id": server_id(server),
            "session_id": server.session_id,
            "token": server.token,
            "seq_ack": seq_ack.map_or(-1, |s| s as i64),
        }),
    )
}

pub fn heartbeat(nonce: u64, seq_ack: Option<u64>) -> Outgoing {
    Outgoing::json(
        op::HEARTBEAT,
        json!({ "t": nonce, "seq_ack": seq_ack.map_or(-1, |s| s as i64) }),
    )
}

pub fn select_protocol(address: &str, port: u16, mode: &str) -> Outgoing {
    Outgoing::json(
        op::SELECT_PROTOCOL,
        json!({
            "protocol": "udp",
            "data": { "address": address, "port": port, "mode": mode },
        }),
    )
}

pub fn speaking(ssrc: u32, speaking: bool) -> Outgoing {
    // Flag 1 = microphone.
    Outgoing::json(
        op::SPEAKING,
        json!({ "speaking": u8::from(speaking), "delay": 0, "ssrc": ssrc }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use plume_core::Snowflake;

    fn server(guild: Option<u64>) -> VoiceServer {
        VoiceServer {
            guild_id: guild.map(Snowflake),
            channel_id: Snowflake(5),
            user_id: Snowflake(9),
            session_id: "sess".into(),
            token: "tok".into(),
            endpoint: "example.discord.media:443".into(),
        }
    }

    #[test]
    fn identify_uses_the_channel_for_dm_calls() {
        let Outgoing::Json(dm) = identify(&server(None), 1) else {
            panic!()
        };
        assert_eq!(dm["d"]["server_id"], "5");
        let Outgoing::Json(guild) = identify(&server(Some(3)), 1) else {
            panic!()
        };
        assert_eq!(guild["d"]["server_id"], "3");
        assert_eq!(guild["d"]["max_dave_protocol_version"], 1);
    }

    #[test]
    fn binary_messages_lead_with_the_opcode() {
        let Outgoing::Binary(data) = Outgoing::binary(26, &[1, 2]) else {
            panic!()
        };
        assert_eq!(data, [26, 1, 2]);
    }

    #[test]
    fn close_codes() {
        assert!(is_resumable(Some(4015)));
        assert!(!is_resumable(Some(4014)));
        assert_eq!(
            describe_close(Some(4014), ""),
            "disconnected from the voice channel (4014)"
        );
    }
}
