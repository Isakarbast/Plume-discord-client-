# 06 — Performance budget

These are the success criteria per tier (see [11-usage-profiles.md](11-usage-profiles.md)). CI should eventually measure them (`tools/` will hold the benchmark harness). "Heavy account" means a recorded fixture with ~100 servers.

| Metric | 🪶 Feather | 💬 Standard | 🧩 Full | Vesktop (rough reference) |
|---|---|---|---|---|
| Idle RSS, heavy account | **5–30 MB** | **50–100 MB** | **100–250 MB** | 400–700 MB |
| Cold start to usable UI | < 200 ms | < 500 ms | < 800 ms | 3–6 s |
| Channel switch (cached) | < 16 ms | < 16 ms | < 16 ms | 100–300 ms |
| Idle CPU | ~0% | ~0% | ~0% | 1–3% |
| Stripped binary | < 8 MB | < 25 MB | < 40 MB | ~150 MB+ |
| Scroll 10k messages | flat memory | 60 fps, flat memory | 60 fps | — |

## Techniques
- **Virtualize everything:** message list, member list, emoji picker.
- **Decode images at their display size,** not at full resolution. Animate only while visible (optionally only on hover).
- **Bounded caches** sized by the active profile.
- **Startup from snapshot:** render the guild and channel tree from SQLite before the gateway's READY arrives (Standard/Full).
- **Stream-filter READY** so unused data is never allocated (essential for Feather).
- **Lazy loading:** member lists, guilds you haven't opened, the emoji catalog.
- **No idle wakeups:** timers only for heartbeats and visible relative timestamps.
- Profile with `heaptrack`, `perf`, and the GTK inspector. Track RSS per tier on every release.
