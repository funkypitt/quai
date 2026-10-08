#!/bin/sh
# Builds Quai and installs it for the current user, started with the COSMIC session.
#   ./install.sh                 build, install, start
#   ./install.sh --cosmic-dock off   also switch COSMIC's own dock off (on: back on)
set -eu
cd "$(dirname "$0")"
PATH="$HOME/.cargo/bin:$PATH"

cargo build --release
install -Dm755 target/release/quai "$HOME/.local/bin/quai"
install -Dm644 data/quai.service "$HOME/.config/systemd/user/quai.service"
systemctl --user daemon-reload
systemctl --user enable quai.service
systemctl --user restart quai.service
echo "Quai is installed and running."

if [ "${1:-}" = "--cosmic-dock" ]; then
    "$HOME/.local/bin/quai" --cosmic-dock "${2:-}"
fi
