# 03 — Tech stack

| Concern | Choice | Alternatives considered |
|---|---|---|
| Language | **Rust** | C++ (Abaddon does this, but memory safety matters for parsing untrusted network data), Go (GC pauses, bigger RSS) |
| UI toolkit | **GTK4 + libadwaita** via gtk-rs | Qt6 (nicer on Windows/macOS, but Rust bindings are awkward), iced/Slint (pure Rust, but rich text and virtualized lists are still immature) |
| Feather frontends | **ratatui + crossterm** (TUI); **Slint, software renderer** (lite GUI) | FLTK (tiny, but weak Unicode/emoji), egui (needs GL, uses more RAM) |
| Async | tokio (`current_thread` in Feather) | async-std (smaller ecosystem) |
| WebSocket | tokio-tungstenite + rustls | — |
| HTTP | reqwest (rustls, http2) | hyper directly (later, if size matters) |
| JSON | serde + serde_json (simd-json later if profiling says so) | — |
| Compression | flate2 (zlib-stream); zstd-stream when the gateway supports it | — |
| Storage | rusqlite (bundled SQLite, WAL) | sled/redb |
| Keyring | `keyring` crate → Secret Service / Windows Credential Manager / macOS Keychain | — |
| Images | `image` crate + `gdk::Texture`; `webp-animation` / `gif` for animation | gdk-pixbuf loaders |
| Audio (voice) | cpal (or PipeWire directly on Linux), `opus` bindings, webrtc-audio-processing for echo cancellation and noise suppression | — |
| Emoji font | System Noto Color Emoji, Twemoji SVGs bundled as a fallback | — |

## Why GTK4 specifically
- `GtkListView` / `GtkColumnView` recycle widgets. That's essential for message lists with 10k+ items.
- It's native on Linux (your main target), uses GPU rendering (the GSK Vulkan/GL renderer), and follows the system dark mode and accent color through libadwaita.
- Downside: on Windows and macOS it works but feels foreign. That's acceptable for v1. Because of the crate split, a Qt frontend could come later.

See [adr/0001-rust-gtk4.md](adr/0001-rust-gtk4.md).
