#!/bin/sh
# Builds Quai and installs it for the current user, started with the COSMIC session.
#   ./install.sh                 build, install, start
#   ./install.sh --cosmic-dock off   also switch COSMIC's own dock off (on: back on)
set -eu
cd "$(dirname "$0")"
PATH="$HOME/.cargo/bin:$PATH"

entries="$HOME/.config/cosmic/com.system76.CosmicPanel/v1/entries"
cosmic_dock() {
    [ -f "$entries" ] || { echo "COSMIC panel settings not found: $entries" >&2; return 1; }
    [ -f "$entries.before-quai" ] || cp "$entries" "$entries.before-quai"
    case "$1" in
        off) printf '[\n    "Panel",\n]' > "$entries"; echo "COSMIC dock switched off" ;;
        on)  printf '[\n    "Panel",\n    "Dock",\n]' > "$entries"; echo "COSMIC dock switched on" ;;
        *)   echo "--cosmic-dock takes on or off" >&2; return 1 ;;
    esac
}

cargo build --release
install -Dm755 target/release/quai "$HOME/.local/bin/quai"
install -Dm644 data/quai.service "$HOME/.config/systemd/user/quai.service"
systemctl --user daemon-reload
systemctl --user enable quai.service
systemctl --user restart quai.service
echo "Quai is installed and running."

if [ "${1:-}" = "--cosmic-dock" ]; then
    cosmic_dock "${2:-}"
fi
