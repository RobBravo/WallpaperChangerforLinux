#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# --locked so an install builds exactly what the committed Cargo.lock pins.
cargo build --release --locked

# Stop the running daemon first: overwriting a binary that is currently executing
# fails with ETXTBSY, which under `set -e` would abort the whole install.
systemctl --user stop wallpaper-changer-daemon 2>/dev/null || true

# `install` replaces the destination via a fresh inode instead of truncating it in
# place, so it is safe even if something still holds the old binary open.
install -Dm755 target/release/wallpaper-changer-daemon "$HOME/.local/bin/wallpaper-changer-daemon"
install -Dm755 target/release/wallpaper-changer-gui "$HOME/.local/bin/wallpaper-changer-gui"

mkdir -p "$HOME/.config/systemd/user"
cp packaging/wallpaper-changer-daemon.service "$HOME/.config/systemd/user/"

mkdir -p "$HOME/.local/share/applications"
sed "s|@BINDIR@|$HOME/.local/bin|" packaging/wallpaper-changer-gui.desktop \
    > "$HOME/.local/share/applications/wallpaper-changer-gui.desktop"

systemctl --user daemon-reload
systemctl --user enable --now wallpaper-changer-daemon
# `enable --now` is a no-op on an already-enabled unit, so restart explicitly to make
# sure the process that ends up running is the binary we just installed. Restarting a
# unit that was not running yet simply starts it.
systemctl --user restart wallpaper-changer-daemon

echo "Installed. Check status with: systemctl --user status wallpaper-changer-daemon"
