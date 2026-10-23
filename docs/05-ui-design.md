# 05 — UI design

## Layout (libadwaita `NavigationSplitView`, collapses on narrow windows)
```
┌────┬──────────────┬───────────────────────────────────┬────────────┐
│ G  │ Server name ▾│ # channel-name · topic        🔍 👥│            │
│ u  ├──────────────┼───────────────────────────────────┤  Members   │
│ i  │ ▾ TEXT       │                                   │  (toggle)  │
│ l  │   # general  │   message list (GtkListView,      │            │
│ d  │   # memes  ● │   virtualized, grouped by author) │            │
│    │ ▾ VOICE      │                                   │            │
│ r  │   🔊 Lounge  │                                   │            │
│ a  ├──────────────┼───────────────────────────────────┤            │
│ i  │ me · 🎤 🎧 ⚙ │ [+] composer…            😀 GIF ➤ │            │
│ l  │              │ typing indicator                  │            │
└────┴──────────────┴───────────────────────────────────┴────────────┘
```

## Widgets (`crates/plume-gtk/src/widgets/`)
| Widget | Notes |
|---|---|
| `guild_rail` | Icons, folders, unread pill, mention badge, drag to reorder |
| `channel_tree` | `GtkTreeListModel`; categories collapse; hides channels you can't view |
| `message_list` | `GtkListView`, bottom-anchored, loads older pages on scroll-up, "jump to present" |
| `message_row` | Avatar, name, timestamp, rich text (Pango markup from the markdown AST), reply preview, attachments, embeds, reactions. **Compact mode** variant |
| `composer` | Multiline `GtkTextView`, autocomplete for @, #, :emoji:, and /commands; paste or drag images to upload; edit last message with ↑ |
| `embed` | Coloured side bar, title, fields grid, thumbnail |
| `emoji_picker` | Search, categories, recent, guild emoji; lazy-loaded images |
| `member_list` | Virtualized; grouped by role/status; uses lazy guild subscriptions |
| `avatar` | Circular texture, shared from the media LRU |

## Keyboard
- `Ctrl+K` quick switcher (fuzzy search over channels, DMs, and guilds)
- `Alt+↑/↓` previous/next channel; `Alt+Shift+↑/↓` previous/next unread
- `Esc` marks the channel read; `Ctrl+E` emoji picker; `Ctrl+,` settings

## Theming
- Follow the system light/dark mode and accent colour through libadwaita by default.
- User CSS from `~/.config/plume/style.css` loads last (GTK CSS, not web CSS). See [08-extensibility.md](08-extensibility.md).
- Settings: compact mode, font scale, show/hide avatars, animate emoji (always / hover / never), timestamps 12h/24h.

## Accessibility
Use GTK's accessible roles on every custom widget, make everything reachable by keyboard, and respect the system's reduced-motion setting.
