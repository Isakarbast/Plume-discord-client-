#!/bin/sh
# Installs Plume for the current user (no root needed):
#   ./packaging/install.sh            -> ~/.local
#   PREFIX=/usr/local sudo -E ./packaging/install.sh
set -eu
PREFIX="${PREFIX:-$HOME/.local}"
cd "$(dirname "$0")/.."

cargo build --release -p plume-gtk -p plume-tui
cargo build --profile feather -p plume-tui

install -Dm755 target/release/plume "$PREFIX/bin/plume"
install -Dm755 target/feather/plume-tui "$PREFIX/bin/plume-tui"
install -Dm644 packaging/dev.plume.Plume.desktop "$PREFIX/share/applications/dev.plume.Plume.desktop"
install -Dm644 packaging/dev.plume.Plume.metainfo.xml "$PREFIX/share/metainfo/dev.plume.Plume.metainfo.xml"
install -Dm644 assets/icons/dev.plume.Plume.svg "$PREFIX/share/icons/hicolor/scalable/apps/dev.plume.Plume.svg"
echo "Installed to $PREFIX. Run 'plume' (desktop) or 'plume-tui' (terminal)."
