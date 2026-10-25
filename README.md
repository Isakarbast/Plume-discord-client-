# Plume (working name)

A lightweight, **native** Discord client with selectable weight: 🪶 Feather (5–30 MB), 💬 Standard (50–100 MB), 🧩 Full with voice and plugins (100–250 MB). Same idea as Vesktop — a nicer desktop client — but without Electron or any embedded browser.

**Status:** text chat works in both the 💬 desktop app (`plume`, GTK4 + libadwaita) and the 🪶 terminal client (`plume-tui`). Voice calls (with Discord's end-to-end encryption) work in the desktop app. See the [roadmap](docs/09-roadmap.md).

> ⚠️ Third-party clients break Discord's Terms of Service. Use at your own risk. See [10-risks.md](docs/10-risks.md).

## Quick start

### Desktop app

```bash
sudo dnf install gtk4-devel libadwaita-devel alsa-lib-devel cmake   # Fedora; see packaging/ for others
./packaging/install.sh                          # builds and installs to ~/.local
plume                                           # log in with a QR code on first start
plume --demo                                    # or try it with made-up data, no login
```

It looks like Discord's dark theme by default (`theme = system` for GNOME's look), with the member list on the right (roles, statuses, activities), your status and mute/deafen bottom-left, a Friends page (online/all/pending/blocked, add by username), pinned messages, search, @/#/: autocomplete, role colours and nicknames, big emoji, and right-click menus on servers, channels, messages and people in voice (per-person volume). User Settings: Ctrl+, or the gear. Click a voice channel (or the phone button in a DM) to join a call; the bar under the channel list has mute, deafen and hang up, and a lock when the call is end-to-end encrypted (hover it for the privacy code). Test your mic first with `cargo run -p plume-voice --example mic_test` (use headphones). Right-click a message to reply, react, copy, edit or delete. Enter sends, Shift+Enter adds a line, ↑ in an empty box edits your last message, Ctrl+K jumps anywhere. Settings go in `~/.config/plume/settings` (`profile = full`, `notifications = false`, `silent_typing = true`, `voice_threshold = -50`) and your own CSS in `~/.config/plume/style.css`.

### Terminal client

```bash
cargo build --profile feather -p plume-tui   # smallest build (~5 MB); --release works too
./target/feather/plume-tui login             # scan the QR code with the Discord app
./target/feather/plume-tui
```

If QR login says a captcha is required, log in with a token instead (it's checked with Discord, then kept in your system keyring):

```bash
read -rs DISCORD_TOKEN && export DISCORD_TOKEN
./target/feather/plume-tui login --token
```

### Keys

| Where | Keys |
|---|---|
| Anywhere | `Tab` sidebar → messages → input · `Ctrl+K` jump to a channel · `PgUp`/`PgDn` scroll (scroll to the top for older messages) · `Ctrl+C` quit |
| Sidebar | `↑`/`↓` or `j`/`k` move · `Enter`/`→` open or expand · `←` collapse · `i` start typing |
| Messages | `↑`/`↓` select · `r`/`Enter` reply · `e` edit · `d` delete · `+` react · `Esc` back to typing |
| Input | `Enter` send · `↑` (empty box) select messages · `Esc` cancel reply/edit, then sidebar · `End` (empty box) jump to newest · `Ctrl+W` delete word · `Ctrl+U` clear |

Reactions take an emoji (`🔥`), a server emoji (`:pepe:`), or a common name (`thumbsup`, `fire`, `heart`…). Unread state syncs with Discord both ways, muted servers and channels stay quiet, channels you can't see are hidden, and DMs and mentions show desktop notifications.

Options: `--profile feather|standard|full` (how much to keep in memory), `--silent-typing`, `--no-notifications`. Logs go to `~/.local/state/plume/plume-tui.log`.

## Crates

| Crate | What it does |
|---|---|
| `plume-core` | Data types, the in-memory store (read state, mutes, reactions, scrollback), permissions, profiles, the redacting `Token` type |
| `plume-gateway` | The live WebSocket connection: login, heartbeats, resume, reconnect, compression |
| `plume-rest` | HTTP API with rate limiting: history, sending, typing |
| `plume-auth` | QR login, token login, keyring storage |
| `plume-markdown` | Discord's markdown → syntax tree → styled spans |
| `plume-notify` | When to notify (DMs, mentions, mutes) and desktop notifications |
| `plume-cache` | SQLite snapshot for instant startup (Standard/Full) |
| `plume-media` | Avatars and images: fetched at display size, memory + disk cache, animated GIF/WebP |
| `plume-voice` | Voice calls: voice gateway, UDP/RTP with AES-GCM, Opus, DAVE end-to-end encryption (`davey`), microphone and speakers (`cpal`) |
| `plume-tui` | 🪶 Feather frontend (ratatui) |
| `plume-gtk` | 💬/🧩 Standard and Full frontend (GTK4 + libadwaita), binary `plume` |

## Packaging

`packaging/` has a user install script, a desktop entry and AppStream metainfo, a Flatpak manifest, a Fedora/COPR spec and an AUR `PKGBUILD`. CI (`.github/workflows/ci.yml`) runs rustfmt, clippy, the tests, and checks the Feather binary stays under 8 MB.

## Docs

| Doc | What it covers |
|---|---|
| [01-vision.md](docs/01-vision.md) | Goals, non-goals, and what "lightweight" means here |
| [02-architecture.md](docs/02-architecture.md) | Crates, data flow, threading |
| [03-tech-stack.md](docs/03-tech-stack.md) | Language and toolkit choices, and why |
| [04-discord-protocol.md](docs/04-discord-protocol.md) | Gateway, REST, rate limits, login |
| [05-ui-design.md](docs/05-ui-design.md) | Layout, widgets, keyboard UX, theming |
| [06-performance-budget.md](docs/06-performance-budget.md) | Hard numbers we hold ourselves to |
| [07-voice.md](docs/07-voice.md) | Voice/video plan (the hardest part) |
| [08-extensibility.md](docs/08-extensibility.md) | Themes and plugins without a JS runtime |
| [09-roadmap.md](docs/09-roadmap.md) | Milestones in order |
| [10-risks.md](docs/10-risks.md) | ToS, account safety, API drift |
| [11-usage-profiles.md](docs/11-usage-profiles.md) | Feather (5–30 MB) / Standard / Full tiers |
| [adr/](docs/adr/) | Architecture decision records |
