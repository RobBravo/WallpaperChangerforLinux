#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

cargo build --release

mkdir -p "$HOME/.local/bin"
cp target/release/wallpaper-changer-daemon "$HOME/.local/bin/"
cp target/release/wallpaper-changer-gui "$HOME/.local/bin/"

mkdir -p "$HOME/.config/systemd/user"
cp packaging/wallpaper-changer-daemon.service "$HOME/.config/systemd/user/"

systemctl --user daemon-reload
systemctl --user enable --now wallpaper-changer-daemon

echo "Installed. Check status with: systemctl --user status wallpaper-changer-daemon"
