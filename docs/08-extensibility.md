# 08 — Themes and plugins

There's no browser, so Vencord/BetterDiscord plugins and CSS **cannot** work. Planned alternatives:

## Themes
- GTK CSS at `~/.config/plume/style.css`, hot-reloaded.
- Ship a few presets in `assets/themes/` (e.g. a Discord-like dark theme, AMOLED, and a theme that follows the system).
- Document the stable CSS class names for each widget (`.message-row`, `.guild-rail`, …) and treat them as an API.

## Built-in "Vencord-style" features (native, toggleable in settings)
These cover the most popular reasons people use client mods, without needing a plugin runtime:
- Show hidden channels (names only, read-only)
- Message logger (keep deleted/edited messages locally, opt-in)
- Silent typing, which doesn't send typing indicators
- Better timestamps, compact mode, and "jump to reply"
- Image zoom / open in system viewer

## Plugin system (later, optional)
- Lua (`mlua`) or WASM (`wasmtime`) scripts with a **narrow, capability-based API**: listen to events, add message context-menu actions, add slash-style local commands.
- Don't expose raw REST/gateway access to plugins (account safety, see [10-risks.md](10-risks.md)).
