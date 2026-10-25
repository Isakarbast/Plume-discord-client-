# 09 — Roadmap

## M0 — Skeleton ✅
Cargo workspace, CI (fmt, clippy, test), logging with token redaction, a GTK window that opens.

## M1 — Read-only client ✅ (except the SQLite startup snapshot)
QR and token login → keyring. Gateway connect/heartbeat/resume. Guild rail + channel tree. Message list with plain text and avatars. SQLite snapshot for instant startup.

## M1.5 — Feather spike ✅ (TUI done: 7.9 MB release binary, ~5 MB before login; `plume-lite` not started)
Build `plume-tui` against the M1 core. Add the READY stream filter. Measure RSS with the heavy-account fixture and prove that 5–30 MB holds. Prototype `plume-lite` on the Slint software renderer and compare it with FLTK.

## M2 — Daily driver (text) ✅ in the TUI (threads, emoji picker and SQLite cache still open)
Send/edit/delete, replies, the full markdown renderer, embeds, attachments (view + upload), reactions, emoji picker, typing indicators, unread/mention state + ack, notifications, DMs and group DMs, threads, quick switcher, settings page.

## M3 — Voice ✅ in the GTK app (push-to-talk, per-user volume and TUI voice still open)
Join/leave a voice channel, audio in/out, DAVE, mute/deafen, push-to-talk, per-user volume.

## M4 — Polish and distribution ✅ mostly (GTK app, packaging, CI; Flathub submission, AppImage and Windows/macOS builds open)
Themes and the built-in feature toggles, a profile switcher with a live RSS readout, `plume-lite` reaching parity with the TUI, Flatpak (Flathub), RPM/COPR (Fedora), AUR, AppImage. Windows/macOS builds if GTK behaves well there.

## M5 — Stretch
Plugin API, screen share/video, multiple accounts, search UI.

## Progress notes
- **M2 in the TUI:** sending, replies, edits, deletes, reactions (add/remove, live updates), embeds (text), attachments (as links), mentions, typing indicators both ways, unread and mention counts synced with Discord (read state, acks, `MESSAGE_ACK`), muted servers and channels, hidden channels via permission math, desktop notifications for DMs and mentions, older history on scroll-up, the quick switcher.
- **Still open:** threads and forums, an emoji picker (the TUI resolves names instead), the SQLite startup cache (`plume-cache`, planned for the GTK round), mute expiry times (a timed mute is treated as muted), and member lists.
- **Size:** `cargo build --profile feather` gives a 5.3 MB TUI binary (opt-level "s", panic = abort); `--release` is 8.5 MB.
- READY is parsed straight from borrowed JSON into typed structs, so fields Plume doesn't use are skipped without being allocated. That covers most of what `ready_filter.rs` was planned for; measure with a big account before doing more.
- **GTK app (round 2):** server bar with icons, channel/DM list with unread and mention badges, virtualized message list with avatars, inline images (via Discord's media proxy at display size), link previews, reaction buttons, code blocks, replies, right-click menu (reply, react, copy, edit, delete), file uploads, emoji chooser, Ctrl+K, toasts for errors, notifications that open the channel, QR/token login window, adaptive layout for narrow windows, SQLite cache, `--demo` mode. Release binary 11.1 MB, ~66 MB RSS in demo mode (software rendering).
- **Still open in GTK:** member list, threads, custom emoji images inline, animated avatars (Full tier), a settings dialog (settings are a file for now), tray icon.
- **Voice (round 3):** new `plume-voice` crate. Voice gateway v8 (identify, resume, heartbeats with `seq_ack`), UDP IP discovery, RTP with `aead_aes256_gcm_rtpsize` via `ring`, Opus via `opus` (libopus built from source, so no system package), DAVE end-to-end encryption with the pure-Rust `davey` crate instead of C++ `libdave`, `cpal` for the microphone and speakers (resampling for devices that aren't 48 kHz), a voice-activity gate, jitter prebuffering and mixing. GTK: voice channels and who's in them (with speaking highlights and mute icons) in the channel list, a call bar with mute/deafen/hang up and the privacy code, DM calls that ring, joining from Ctrl+K, `voice_threshold` setting. `mic_test` example checks devices without Discord. Not yet tested against a real call.
- **Discord look (round 4):** Discord-style dark theme, member list (GUILD_MEMBER_LIST_UPDATE via op 14, hoisted role groups, presences and activities), status dots, user panel with status picker (op 3), Friends page (relationships, accept/ignore/remove, add by username), pins and search popovers, User Settings dialog (saved to the settings file), composer autocomplete (@user, #channel, :emoji:), role colours and nicknames, jumbo and server emoji images, server/channel/message context menus (mark read, mute server, copy ID/link), per-person voice volume. Not done: video and screen share, threads and forums, stickers and GIF picker, server settings and moderation, Nitro features, global push-to-talk.
- **Round 5 (frugal pass + features):** fewer redraws (sidebar only when unread state changes, debounced; presence redraws only for people on screen), texture cache capped by bytes (24/64 MB), images at the screen's real density, glibc arenas capped and freed memory returned after READY and every minute, optional software rendering. Features: hover toolbar, NEW divider and Jump to Present, reply @ping toggle, custom status, threads (sidebar + Threads button), stickers and image/GIF embeds with colour bars, GIF and sticker picker, RNNoise noise suppression, device choice, push-to-talk (window focused), moderation (timeout/kick/ban), pin/unpin, delete with Manage Messages, per-server notification level, join by invite, display name and avatar editing. Still open: video, screen share, server folders, server and role settings, Nitro features, global push-to-talk.
