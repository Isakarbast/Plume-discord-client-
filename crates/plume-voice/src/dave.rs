//! DAVE: Discord's end-to-end encryption for voice.
//!
//! The voice server coordinates an MLS group of everyone in the call; the
//! [`davey`] crate does the cryptography. This module only follows the
//! protocol's opcodes. See <https://daveprotocol.com>.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU16;

use davey::{DaveSession, MediaType, ProposalsOperationType};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::voice_gateway::{Outgoing, op};

/// How long unencrypted audio is still accepted while a call switches
/// encryption on, in seconds.
const TRANSITION_EXPIRY: u32 = 10;

pub struct Dave {
    user_id: u64,
    channel_id: u64,
    /// 0 means the call isn't end-to-end encrypted.
    protocol_version: u16,
    session: Option<DaveSession>,
    /// Transitions announced but not executed yet: id → protocol version.
    pending: HashMap<u16, u16>,
    /// Everyone the voice server says is connected, including us.
    connected: HashSet<u64>,
}

impl Dave {
    pub fn new(user_id: u64, channel_id: u64) -> Self {
        Dave {
            user_id,
            channel_id,
            protocol_version: 0,
            session: None,
            pending: HashMap::new(),
            connected: HashSet::from([user_id]),
        }
    }

    pub fn max_version() -> u16 {
        davey::DAVE_PROTOCOL_VERSION
    }

    /// Whether audio is end-to-end encrypted right now.
    pub fn is_active(&self) -> bool {
        self.protocol_version > 0 && self.session.as_ref().is_some_and(DaveSession::is_ready)
    }

    /// The code users can compare to check nobody is listening in.
    pub fn privacy_code(&self) -> Option<String> {
        if self.protocol_version == 0 {
            return None;
        }
        self.session
            .as_ref()
            .and_then(DaveSession::voice_privacy_code)
            .map(str::to_owned)
    }

    pub fn clients_connected(&mut self, user_ids: impl IntoIterator<Item = u64>) {
        self.connected.extend(user_ids);
    }

    pub fn client_disconnected(&mut self, user_id: u64) {
        self.connected.remove(&user_id);
    }

    /// From SESSION_DESCRIPTION: the version the call starts with.
    pub fn start(&mut self, protocol_version: u16) -> Vec<Outgoing> {
        self.protocol_version = protocol_version;
        self.reinit()
    }

    /// Start over with a fresh MLS group (or none, for version 0).
    fn reinit(&mut self) -> Vec<Outgoing> {
        let Some(version) = NonZeroU16::new(self.protocol_version) else {
            if let Some(session) = &mut self.session {
                if let Err(error) = session.reset() {
                    warn!("DAVE reset failed: {error}");
                }
                session.set_passthrough_mode(true, None);
            }
            return Vec::new();
        };

        let result = match &mut self.session {
            Some(session) => session
                .reinit(version, self.user_id, self.channel_id, None)
                .map_err(|e| e.to_string()),
            None => DaveSession::new(version, self.user_id, self.channel_id, None)
                .map(|session| self.session = Some(session))
                .map_err(|e| e.to_string()),
        };
        if let Err(error) = result {
            warn!("couldn't start DAVE: {error}");
            return Vec::new();
        }
        self.key_package().into_iter().collect()
    }

    fn key_package(&mut self) -> Option<Outgoing> {
        let session = self.session.as_mut()?;
        match session.create_key_package() {
            Ok(package) => Some(Outgoing::binary(op::DAVE_MLS_KEY_PACKAGE, &package)),
            Err(error) => {
                warn!("couldn't create DAVE key package: {error}");
                None
            }
        }
    }

    /// Handle a JSON DAVE opcode.
    pub fn handle_json(&mut self, opcode: u8, data: &Value) -> Vec<Outgoing> {
        let transition_id = data["transition_id"].as_u64().unwrap_or(0) as u16;
        let protocol_version = data["protocol_version"].as_u64().unwrap_or(0) as u16;
        match opcode {
            op::DAVE_PREPARE_TRANSITION => {
                self.pending.insert(transition_id, protocol_version);
                if transition_id == 0 {
                    self.execute(transition_id);
                    Vec::new()
                } else {
                    if protocol_version == 0
                        && let Some(session) = &mut self.session
                    {
                        // Others may stop encrypting before the switch.
                        session.set_passthrough_mode(true, None);
                    }
                    vec![transition_ready(transition_id)]
                }
            }
            op::DAVE_EXECUTE_TRANSITION => {
                self.execute(transition_id);
                Vec::new()
            }
            op::DAVE_PREPARE_EPOCH => {
                // Epoch 1 means a brand new group: rejoin from scratch.
                if data["epoch"].as_u64() == Some(1) {
                    self.protocol_version = protocol_version;
                    self.reinit()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn execute(&mut self, transition_id: u16) {
        let Some(version) = self.pending.remove(&transition_id) else {
            debug!("unknown DAVE transition {transition_id}");
            return;
        };
        let upgrading = self.protocol_version == 0 && version > 0;
        self.protocol_version = version;
        if let Some(session) = &mut self.session {
            if version == 0 {
                session.set_passthrough_mode(true, None);
            } else if upgrading {
                session.set_passthrough_mode(false, Some(TRANSITION_EXPIRY));
            }
        }
        debug!("DAVE transition {transition_id} executed, version {version}");
    }

    /// Handle a binary DAVE opcode (MLS messages).
    pub fn handle_binary(&mut self, opcode: u8, payload: &[u8]) -> Vec<Outgoing> {
        let Some(session) = &mut self.session else {
            return Vec::new();
        };
        match opcode {
            op::DAVE_MLS_EXTERNAL_SENDER => {
                if let Err(error) = session.set_external_sender(payload) {
                    warn!("DAVE external sender rejected: {error}");
                }
                Vec::new()
            }
            op::DAVE_MLS_PROPOSALS => {
                let Some((&kind, proposals)) = payload.split_first() else {
                    return Vec::new();
                };
                let kind = if kind == 0 {
                    ProposalsOperationType::APPEND
                } else {
                    ProposalsOperationType::REVOKE
                };
                let expected: Vec<u64> = self.connected.iter().copied().collect();
                match session.process_proposals(kind, proposals, Some(&expected)) {
                    Ok(Some(commit)) => {
                        let mut message = commit.commit;
                        message.extend(commit.welcome.unwrap_or_default());
                        vec![Outgoing::binary(op::DAVE_MLS_COMMIT_WELCOME, &message)]
                    }
                    Ok(None) => Vec::new(),
                    Err(error) => {
                        warn!("DAVE proposals failed: {error}");
                        Vec::new()
                    }
                }
            }
            op::DAVE_MLS_ANNOUNCE_COMMIT_TRANSITION | op::DAVE_MLS_WELCOME => {
                if payload.len() < 2 {
                    return Vec::new();
                }
                let transition_id = u16::from_be_bytes([payload[0], payload[1]]);
                let message = &payload[2..];
                let result = if opcode == op::DAVE_MLS_WELCOME {
                    session.process_welcome(message).map_err(|e| e.to_string())
                } else {
                    session.process_commit(message).map_err(|e| e.to_string())
                };
                match result {
                    Ok(()) => {
                        if transition_id == 0 {
                            Vec::new()
                        } else {
                            self.pending.insert(transition_id, self.protocol_version);
                            vec![transition_ready(transition_id)]
                        }
                    }
                    Err(error) => {
                        warn!("DAVE group update failed ({error}); rejoining");
                        let mut out = vec![Outgoing::json(
                            op::DAVE_MLS_INVALID_COMMIT_WELCOME,
                            json!({ "transition_id": transition_id }),
                        )];
                        out.extend(self.reinit());
                        out
                    }
                }
            }
            _ => Vec::new(),
        }
    }

    /// Encrypt our Opus frame, if the call is encrypted. `None` means drop it.
    pub fn encrypt(&mut self, opus: &[u8]) -> Option<Vec<u8>> {
        if self.protocol_version == 0 {
            return Some(opus.to_vec());
        }
        let session = self.session.as_mut()?;
        if !session.is_ready() {
            return None;
        }
        session
            .encrypt_opus(opus)
            .ok()
            .map(|frame| frame.into_owned())
    }

    /// Decrypt someone's Opus frame. `None` means drop it.
    pub fn decrypt(&mut self, user_id: Option<u64>, frame: &[u8]) -> Option<Vec<u8>> {
        let ready = self.session.as_ref().is_some_and(DaveSession::is_ready);
        match (user_id, &mut self.session) {
            (Some(user_id), Some(session)) if ready => {
                match session.decrypt(user_id, MediaType::AUDIO, frame) {
                    Ok(plain) => Some(plain),
                    // Not in the group (yet): only fine if the call isn't encrypted.
                    Err(_) if self.protocol_version == 0 => Some(frame.to_vec()),
                    Err(_) => None,
                }
            }
            _ if self.protocol_version == 0 => Some(frame.to_vec()),
            _ => None,
        }
    }
}

fn transition_ready(transition_id: u16) -> Outgoing {
    Outgoing::json(
        op::DAVE_TRANSITION_READY,
        json!({ "transition_id": transition_id }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unencrypted_calls_pass_audio_through() {
        let mut dave = Dave::new(1, 2);
        assert!(dave.start(0).is_empty());
        assert!(!dave.is_active());
        assert_eq!(dave.encrypt(b"opus").unwrap(), b"opus");
        assert_eq!(dave.decrypt(Some(5), b"opus").unwrap(), b"opus");
        assert_eq!(dave.privacy_code(), None);
    }

    #[test]
    fn encrypted_calls_send_a_key_package_and_wait() {
        let mut dave = Dave::new(1, 2);
        let out = dave.start(1);
        assert!(
            matches!(&out[..], [Outgoing::Binary(data)] if data[0] == op::DAVE_MLS_KEY_PACKAGE)
        );
        // No group yet: nothing may be sent or played unencrypted.
        assert!(dave.encrypt(b"opus").is_none());
        assert!(dave.decrypt(Some(5), b"opus").is_none());
    }

    #[test]
    fn transitions_wait_for_execute() {
        let mut dave = Dave::new(1, 2);
        dave.start(1);
        let out = dave.handle_json(
            op::DAVE_PREPARE_TRANSITION,
            &json!({ "transition_id": 7, "protocol_version": 0 }),
        );
        assert!(matches!(&out[..], [Outgoing::Json(_)]));
        assert!(dave.encrypt(b"x").is_none());

        dave.handle_json(op::DAVE_EXECUTE_TRANSITION, &json!({ "transition_id": 7 }));
        assert_eq!(dave.encrypt(b"x").unwrap(), b"x");
    }
}
