# 11 — Usage profiles (tiers)

You choose how heavy Plume is. There are three tiers, from "runs on a potato" to "everything on."

| | 🪶 **Feather** | 💬 **Standard** | 🧩 **Full** |
|---|---|---|---|
| RAM target | **5–30 MB** | **50–100 MB** | **100–250 MB** |
| Frontend | `plume-tui` (terminal, 5–15 MB) or `plume-lite` (minimal GUI, 15–30 MB) | `plume-gtk` | `plume-gtk` |
| Text chat, DMs, replies, mentions | ✅ | ✅ | ✅ |
| Markdown | Basic (bold/italic/code/mentions) | Full | Full |
| Images / attachments | Links only, or one preview at a time (lite GUI), opened in the system viewer | Inline, decoded to display size | Inline, bigger cache |
| Avatars | ❌ (coloured initials) | ✅ static | ✅ animated |
| Custom emoji | `:name:` text | Static | Animated |
| Embeds | Title + URL only | ✅ | ✅ |
| Reactions | Text counts | ✅ | ✅ |
| Member list | ❌ (on request only) | ✅ lazy | ✅ |
| Notifications | ✅ (libnotify via D-Bus, no tray) | ✅ + tray | ✅ + tray |
| Voice | ❌ | Optional toggle | ✅ |
| Plugins / user themes | ❌ | Themes only | ✅ plugins + themes |
| Disk cache (SQLite) | Off or tiny (guild/channel tree only) | 200 MB | 500 MB+ |
| Messages kept in RAM | 2 channels × 50 | 8 × 200 | 16 × 500 |

## Why Feather needs its own frontend
GTK4 with its GPU renderer uses about 30–50 MB before any Discord data is loaded, so **5–30 MB isn't reachable with GTK**. Because the core crates don't depend on any toolkit, Feather can reuse all the networking and state code under a much smaller frontend:
- **`plume-tui`:** terminal UI built on `ratatui` + `crossterm`. No GPU, no fonts, no image decoding. Target 5–15 MB.
- **`plume-lite`:** a minimal windowed app using **Slint with its software renderer** (no GL context, no GTK). Target 15–30 MB. FLTK is the fallback. *Validate with a spike before committing* (see roadmap M1.5).

## How we hit the Feather numbers
The UI isn't the biggest problem. **Discord's data is.** The READY payload for an account with many servers can be several MB of JSON, and naively parsing it into structs uses tens of MB.
1. **Stream-parse READY** (`plume-gateway/src/ready_filter.rs`): deserialize with a visitor and drop fields Feather never uses (presences, member lists, emoji/sticker catalogs, role colours if disabled, activities) *while parsing*, so they never become structs.
2. **Gateway capabilities and intents:** ask for lazy guilds and skip presence and typing events where the protocol allows it.
3. **Compact models:** IDs as `u64`, interned strings for names, `Box<str>` instead of `String`, and no `serde_json::Value` kept around.
4. **Small allocator option:** build with `mimalloc` (or the system allocator) and measure. Call `malloc_trim` after READY.
5. **No image pipeline at all** in the TUI. In `plume-lite`, keep exactly one decoded image in memory and free it when it's closed.
6. **Single-threaded tokio runtime** (`current_thread`) and default-features-off reqwest/rustls.
7. Release profile: `opt-level = "z"` or `"s"`, LTO, `panic = "abort"`, strip. Binary target < 8 MB.

## Switching profiles
- **Compile time:** cargo features gate whole subsystems (`media`, `voice`, `plugins`, `cache`, `animations`). A Feather build doesn't even link them, which keeps the binary small.
- **Run time:** `plume-core/src/profile.rs` reads `config/profiles/<name>.toml` and sets the knobs in the table above (cache sizes, ring buffer sizes, what to render). Standard ⇄ Full can switch at run time in the GTK settings. Going to Feather means launching `plume-tui` or `plume-lite`, which share the same keyring login and config directory.
- A settings page shows **live RSS** so users can see what each toggle costs.

## Measuring
Each tier gets an RSS check in CI (see [06-performance-budget.md](06-performance-budget.md)) using a recorded READY fixture of a "heavy" account (~100 servers), so we know the numbers hold for real users and not just for empty accounts.
