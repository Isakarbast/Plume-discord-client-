# 01 — Vision

## Why
The official client and Vesktop both ship a full Chromium. That means 300–700 MB RAM at idle, slow startup, and battery drain, just for a chat app.

## Goals
1. **Native UI.** Real toolkit widgets. No webview, no JS engine.
2. **Small footprint.** See [06-performance-budget.md](06-performance-budget.md). Target: about 1/5 of Vesktop's idle RAM.
3. **Covers daily use:** servers, channels, DMs, threads, replies, reactions, embeds, attachments, emoji, mentions, unread state, notifications.
4. **Fits the desktop.** Respects the system theme, fonts, notifications, tray, and keyring.
5. **Keyboard-first.** Quick switcher, vim-ish navigation if you want it.
6. **Doesn't look like a bot.** It behaves like a normal client on the wire (see [10-risks.md](10-risks.md)).

## Non-goals (at least for v1)
- Pixel-perfect copy of Discord's UI.
- Running Vencord/BetterDiscord JS plugins.
- Activities, Nitro shop, Quests, the game library, server boosting UI.
- Go-live/screen share (stretch goal after voice).

## Prior art to study
- **Abaddon** (C++/gtkmm). Native client with voice support. Good reference for the gateway and the voice pipeline.
- **Dissent / gtkcord4** (Go/GTK4). Clean GTK4 UI patterns.
- **Ripcord** (Qt, closed source). Shows how dense and fast a native client can be.
