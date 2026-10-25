# 07 — Voice (phase 3)

This is the most complex subsystem, so it's isolated in `plume-voice` behind a cargo feature.

## Pipeline
```
mic → capture (cpal/PipeWire) → AEC/NS/AGC (webrtc-audio-processing) → Opus encode
    → DAVE E2EE frame encrypt → RTP + transport encryption (AEAD) → UDP
UDP → decrypt → DAVE decrypt → jitter buffer (per SSRC) → Opus decode → mix → playback
```

## Pieces
- **Voice gateway (`voice_gateway.rs`):** a separate WebSocket. IDENTIFY → READY (ssrc, ip, port, modes) → IP discovery over UDP → SELECT_PROTOCOL → SESSION_DESCRIPTION. Speaking events, heartbeats.
- **DAVE (`dave.rs`):** Discord's end-to-end encryption for voice, built on MLS. It's now required for most calls, so plan for it from day one. Implemented with the pure-Rust [`davey`](https://crates.io/crates/davey) crate (MLS via OpenMLS), which avoids building Discord's C++ `libdave`.
- **Audio I/O:** device selection, push-to-talk (needs a global hotkey: XDG GlobalShortcuts portal on Wayland), voice activity threshold, per-user volume.

## Out of scope at first
Video, screen share/Go Live, Krisp-style noise suppression, soundboard.
