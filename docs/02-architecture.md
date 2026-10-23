# 02 — Architecture

## Layers
```
┌──────────────────────── plume-gtk (UI thread) ────────────────────────┐
│  views/  widgets/  state/view_models  ← bound to GListModels          │
└───────────────▲──────────────────────────────────┬────────────────────┘
                │ UiEvent (channel)                │ Command (channel)
┌───────────────┴──────────── plume-core ──────────▼────────────────────┐
│  Store (single source of truth) · reducers · permissions · ids        │
└──────▲──────────────▲──────────────▲──────────────▲───────────────────┘
       │              │              │              │
 plume-gateway   plume-rest     plume-cache     plume-media
 (WebSocket)     (HTTP + rate   (SQLite on      (fetch/decode/
                  limits)        disk)           LRU images)
       │
 plume-voice (optional feature) · plume-auth · plume-notify · plume-markdown
```

## Crates
| Crate | Responsibility | Depends on |
|---|---|---|
| `plume-core` | Domain models (Guild, Channel, Message, User, Member, Role), the snowflake ID type, the in-memory `Store`, event reducers, permission math | — |
| `plume-gateway` | WebSocket connection, zlib-stream decompression, IDENTIFY/RESUME, heartbeat/ACK, reconnect with backoff, dispatch decoding into `core::events` | core |
| `plume-rest` | HTTP client, per-route and global rate-limit buckets, typed routes, multipart uploads, error mapping | core |
| `plume-auth` | QR remote-auth login, paste-token login, storing the token in the OS keyring | rest |
| `plume-cache` | SQLite persistence: recent messages per channel, guild/channel snapshot for instant startup | core |
| `plume-markdown` | Discord-flavored markdown → AST (bold, code blocks, spoilers, mentions, custom emoji, timestamps) | core |
| `plume-media` | Async fetch, image decode at the target size, animated GIF/WebP/APNG, memory LRU plus disk cache | — |
| `plume-voice` | Voice gateway, UDP/RTP, Opus, DAVE E2EE, audio device I/O. **Behind a cargo feature.** | core |
| `plume-notify` | Desktop notifications, unread/mention counts, tray icon | core |
| `plume-tui` | 🪶 Feather frontend: terminal UI (ratatui), text only | core, gateway, rest, auth, markdown |
| `plume-lite` | 🪶 Feather frontend: minimal GUI (Slint software renderer) | core, gateway, rest, auth, markdown |
| `plume-gtk` | 💬/🧩 Standard and Full frontend. The application: windows, widgets, view models, and the bridge between the async runtime and the GTK main loop | everything |

**Rule:** only the frontend crates (`plume-gtk`, `plume-tui`, `plume-lite`) know about a UI toolkit. Everything else is UI-agnostic, which is exactly what lets the three tiers share one core (see [11-usage-profiles.md](11-usage-profiles.md)).

## Threading model
- **GTK main thread.** Owns all widgets and never blocks.
- **Tokio runtime (multi-thread, 2–4 workers).** Gateway, REST, media, and cache I/O.
- **`state/bridge`.** Async channels (`async-channel`) carry `UiEvent`s to the GTK loop through `glib::MainContext::spawn_local`. `Command`s (send message, switch channel, …) go the other way.
- **Image decoding** runs on a small blocking pool, never on the UI thread.

## Data flow examples
**Incoming message:** gateway frame → decompress → deserialize `MESSAGE_CREATE` → `Store` reducer updates the channel ring buffer and unread state → emits `UiEvent::MessageAdded{channel}` → if that channel is visible, its `GListStore` gets one item appended → `ListView` recycles a row widget.

**Opening a channel:** UI sends `Command::OpenChannel` → Store serves cached messages immediately (from SQLite if they aren't in memory) → REST fetches the latest 50 in the background → diff and patch the list.

## State and memory policy
- Keep messages in memory only for the **N most recently viewed channels** (default 8, ring buffer of 200 each). Evict the rest to SQLite.
- Hold member lists lazily per guild, using the gateway's lazy member subscription (op 37 / "guild subscriptions").
- Store users once and reference them by ID everywhere (no duplicated author structs).

## Profiles
`plume-core::profile` holds the active tier's knobs (cache sizes, ring buffers, what to render). Cargo features (`media`, `voice`, `plugins`, `cache`, `animations`) compile whole subsystems out for Feather builds.
