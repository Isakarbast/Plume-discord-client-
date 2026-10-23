# ADR 0003 — Tiered usage profiles with a separate Feather frontend

**Status:** Accepted (planning)

**Context:** Users want to choose anything from an ultra-light 5–30 MB chat client up to a full 100–250 MB client with voice and plugins. GTK4 alone uses about 30–50 MB, so the lowest tier can't be a "mode" of the GTK app.

**Decision:** Three tiers: Feather, Standard, Full. Feather ships as separate frontends (`plume-tui`, `plume-lite`) on the same core crates. Subsystems are gated by cargo features (compile time) and `profile.rs` + `config/profiles/*.toml` (run time).

**Consequences:** Three frontends to maintain, so Feather is intentionally kept text-first and small. The core has to stay strictly toolkit-free. READY stream-filtering becomes a core requirement, not an optimisation.
