# ADR 0001 — Rust + GTK4/libadwaita

**Status:** Accepted (planning)

**Context:** We need a native, low-memory, cross-platform-capable UI with good virtualized lists and rich text. The primary platform is Linux (Fedora).

**Decision:** Rust for all crates and GTK4 + libadwaita (gtk-rs) for the first frontend. Keep the core toolkit-agnostic.

**Consequences:** Excellent on Linux; acceptable on Windows/macOS. Rust's compile times are a cost. A Qt frontend stays possible because `plume-core` and the crates below it have no GTK dependency.
