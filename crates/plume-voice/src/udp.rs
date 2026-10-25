//! The UDP side of voice: finding our public address, and wrapping audio in
//! encrypted RTP packets.
//!
//! See <https://discord.com/developers/docs/topics/voice-connections>.

use std::net::SocketAddr;

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use tokio::net::UdpSocket;

use crate::VoiceError;

/// The only transport encryption we offer. Discord supports it everywhere
/// and `ring` (already used for TLS) implements it with hardware AES.
pub const ENCRYPTION_MODE: &str = "aead_aes256_gcm_rtpsize";

/// RTP payload type Discord uses for Opus.
const PAYLOAD_TYPE: u8 = 0x78;
const RTP_HEADER_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// The 32-bit nonce counter goes on the end of every packet.
const NONCE_SUFFIX_LEN: usize = 4;

const DISCOVERY_LEN: usize = 74;

/// Ask Discord's voice server what our public IP and port look like from
/// outside, so it knows where to send audio.
pub async fn discover_ip(socket: &UdpSocket, ssrc: u32) -> Result<(String, u16), VoiceError> {
    socket.send(&discovery_request(ssrc)).await?;
    let mut buf = [0u8; DISCOVERY_LEN];
    let read = tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv(&mut buf))
        .await
        .map_err(|_| VoiceError::Protocol("IP discovery timed out".into()))??;
    parse_discovery(&buf[..read])
        .ok_or_else(|| VoiceError::Protocol("bad IP discovery reply".into()))
}

pub async fn connect(ip: &str, port: u16) -> Result<UdpSocket, VoiceError> {
    let remote: SocketAddr = format!("{ip}:{port}")
        .parse()
        .map_err(|_| VoiceError::Protocol(format!("bad voice server address {ip}:{port}")))?;
    let local: SocketAddr = if remote.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };
    let socket = UdpSocket::bind(local).await?;
    socket.connect(remote).await?;
    Ok(socket)
}

fn discovery_request(ssrc: u32) -> [u8; DISCOVERY_LEN] {
    let mut packet = [0u8; DISCOVERY_LEN];
    packet[0..2].copy_from_slice(&1u16.to_be_bytes()); // type: request
    packet[2..4].copy_from_slice(&70u16.to_be_bytes()); // length of what follows
    packet[4..8].copy_from_slice(&ssrc.to_be_bytes());
    packet
}

fn parse_discovery(packet: &[u8]) -> Option<(String, u16)> {
    if packet.len() < DISCOVERY_LEN || packet[0..2] != 2u16.to_be_bytes() {
        return None;
    }
    let address = &packet[8..72];
    let end = address
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(address.len());
    let ip = std::str::from_utf8(&address[..end]).ok()?.to_owned();
    let port = u16::from_be_bytes([packet[72], packet[73]]);
    Some((ip, port))
}

/// Encrypts our packets and decrypts everyone else's.
pub struct Transport {
    key: LessSafeKey,
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
    nonce: u32,
}

/// An incoming audio packet after transport decryption.
pub struct Incoming {
    pub ssrc: u32,
    pub sequence: u16,
    /// Opus data, possibly still DAVE-encrypted.
    pub payload: Vec<u8>,
}

impl Transport {
    pub fn new(secret_key: &[u8], ssrc: u32) -> Result<Self, VoiceError> {
        let key = UnboundKey::new(&AES_256_GCM, secret_key)
            .map_err(|_| VoiceError::Protocol("voice key has the wrong length".into()))?;
        Ok(Transport {
            key: LessSafeKey::new(key),
            ssrc,
            sequence: 0,
            timestamp: 0,
            nonce: 0,
        })
    }

    /// Wrap one 20 ms Opus frame in an encrypted RTP packet.
    pub fn seal(&mut self, opus: &[u8]) -> Result<Vec<u8>, VoiceError> {
        let mut packet =
            Vec::with_capacity(RTP_HEADER_LEN + opus.len() + TAG_LEN + NONCE_SUFFIX_LEN);
        packet.extend_from_slice(&[0x80, PAYLOAD_TYPE]);
        packet.extend_from_slice(&self.sequence.to_be_bytes());
        packet.extend_from_slice(&self.timestamp.to_be_bytes());
        packet.extend_from_slice(&self.ssrc.to_be_bytes());
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self
            .timestamp
            .wrapping_add(crate::opus::FRAME_SAMPLES as u32);

        let counter = self.nonce;
        self.nonce = self.nonce.wrapping_add(1);

        let mut body = opus.to_vec();
        let tag = self
            .key
            .seal_in_place_separate_tag(nonce(counter), Aad::from(&packet[..]), &mut body)
            .map_err(|_| VoiceError::Protocol("encryption failed".into()))?;
        packet.extend_from_slice(&body);
        packet.extend_from_slice(tag.as_ref());
        packet.extend_from_slice(&counter.to_be_bytes());
        Ok(packet)
    }

    /// Unwrap someone's packet. Returns `None` for anything that isn't
    /// valid audio (RTCP, keep-alives, tampered packets).
    pub fn open(&self, packet: &[u8]) -> Option<Incoming> {
        if packet.len() < RTP_HEADER_LEN + TAG_LEN + NONCE_SUFFIX_LEN
            || packet[0] >> 6 != 2
            || packet[1] & 0x7F != PAYLOAD_TYPE
        {
            return None;
        }
        let has_extension = packet[0] & 0x10 != 0;
        let csrc_count = usize::from(packet[0] & 0x0F);
        let sequence = u16::from_be_bytes([packet[2], packet[3]]);
        let ssrc = u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]);

        // With the `rtpsize` modes the fixed header, CSRCs and the 4-byte
        // extension header are authenticated but not encrypted. The
        // extension's body is encrypted along with the audio.
        let mut header_len = RTP_HEADER_LEN + 4 * csrc_count;
        let mut extension_words = 0;
        if has_extension {
            let ext = packet.get(header_len..header_len + 4)?;
            extension_words = usize::from(u16::from_be_bytes([ext[2], ext[3]]));
            header_len += 4;
        }
        let nonce_start = packet.len() - NONCE_SUFFIX_LEN;
        if nonce_start < header_len + TAG_LEN {
            return None;
        }
        let counter = u32::from_be_bytes(packet[nonce_start..].try_into().ok()?);

        let mut body = packet[header_len..nonce_start].to_vec();
        let plain = self
            .key
            .open_in_place(nonce(counter), Aad::from(&packet[..header_len]), &mut body)
            .ok()?;
        let payload = plain.get(extension_words * 4..)?.to_vec();
        Some(Incoming {
            ssrc,
            sequence,
            payload,
        })
    }
}

/// AES-GCM wants 12 bytes; Discord puts the counter in the first four.
fn nonce(counter: u32) -> Nonce {
    let mut bytes = [0u8; NONCE_LEN];
    bytes[..4].copy_from_slice(&counter.to_be_bytes());
    Nonce::assume_unique_for_key(bytes)
}

/// UDP keep-alive. Discord drops idle voice connections without them.
pub fn keepalive(counter: u32) -> [u8; 8] {
    let mut packet = [0u8; 8];
    packet[..4].copy_from_slice(&counter.to_le_bytes());
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn sealed_packets_open_again() {
        let mut ours = Transport::new(&KEY, 1234).unwrap();
        let theirs = Transport::new(&KEY, 99).unwrap();
        for frame in [&b"hello"[..], b"second frame"] {
            let packet = ours.seal(frame).unwrap();
            let incoming = theirs.open(&packet).unwrap();
            assert_eq!(incoming.ssrc, 1234);
            assert_eq!(incoming.payload, frame);
        }
    }

    #[test]
    fn tampered_packets_are_dropped() {
        let mut ours = Transport::new(&KEY, 1).unwrap();
        let mut packet = ours.seal(b"audio").unwrap();
        packet[RTP_HEADER_LEN] ^= 1;
        assert!(Transport::new(&KEY, 2).unwrap().open(&packet).is_none());
    }

    #[test]
    fn header_extension_is_stripped() {
        // Build a packet the way Discord does: one-word extension, encrypted
        // together with the audio.
        let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &KEY).unwrap());
        let mut packet = vec![0x90, PAYLOAD_TYPE, 0, 5, 0, 0, 0, 0, 0, 0, 0, 42];
        packet.extend_from_slice(&[0xBE, 0xDE, 0, 1]);
        let mut body = vec![1, 2, 3, 4];
        body.extend_from_slice(b"opus");
        let tag = key
            .seal_in_place_separate_tag(nonce(9), Aad::from(&packet[..]), &mut body)
            .unwrap();
        packet.extend_from_slice(&body);
        packet.extend_from_slice(tag.as_ref());
        packet.extend_from_slice(&9u32.to_be_bytes());

        let incoming = Transport::new(&KEY, 1).unwrap().open(&packet).unwrap();
        assert_eq!(incoming.ssrc, 42);
        assert_eq!(incoming.sequence, 5);
        assert_eq!(incoming.payload, b"opus");
    }

    #[test]
    fn rtcp_is_ignored() {
        let transport = Transport::new(&KEY, 1).unwrap();
        let mut packet = vec![0x80, 200];
        packet.resize(40, 0);
        assert!(transport.open(&packet).is_none());
    }

    #[test]
    fn discovery_round_trip() {
        let request = discovery_request(77);
        assert_eq!(&request[4..8], &77u32.to_be_bytes());

        let mut reply = [0u8; DISCOVERY_LEN];
        reply[0..2].copy_from_slice(&2u16.to_be_bytes());
        reply[8..19].copy_from_slice(b"203.0.113.9");
        reply[72..74].copy_from_slice(&50004u16.to_be_bytes());
        assert_eq!(parse_discovery(&reply), Some(("203.0.113.9".into(), 50004)));
    }
}
