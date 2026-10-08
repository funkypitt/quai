#!/bin/sh
# Builds Quai and installs it for the current user: started with the COSMIC session, and
# listed among the applications. Run it as yourself, never with sudo.
#   ./install.sh                     build, install, start
#   ./install.sh --cosmic-dock off   also switch COSMIC's own dock off (on: back on)
set -eu
cd "$(dirname "$0")"
PATH="$HOME/.cargo/bin:$PATH"

if [ "$(id -u)" = 0 ]; then
    echo "Run install.sh as yourself, without sudo: Quai is installed in your own home and session." >&2
    exit 1
fi
case "${XDG_CURRENT_DESKTOP:-}" in
    *COSMIC*) ;;
    *) echo "Warning: this session is not COSMIC (XDG_CURRENT_DESKTOP=${XDG_CURRENT_DESKTOP:-unset}); Quai only runs on COSMIC." >&2 ;;
esac

cargo build --release
install -Dm755 target/release/quai "$HOME/.local/bin/quai"
install -Dm644 data/quai.service "$HOME/.config/systemd/user/quai.service"
install -Dm644 data/quai.desktop "$HOME/.local/share/applications/quai.desktop"
install -Dm644 data/quai.svg "$HOME/.local/share/icons/hicolor/scalable/apps/quai.svg"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
systemctl --user daemon-reload
systemctl --user enable quai.service
systemctl --user restart quai.service
sleep 1
if [ "$(systemctl --user is-active quai.service)" = active ]; then
    echo "Quai is installed and running."
else
    echo "Quai is installed but not running: journalctl --user -u quai" >&2
fi
if ! systemctl --user list-units --all --no-legend cosmic-session.target 2>/dev/null | grep -q cosmic-session.target; then
    echo "Warning: no cosmic-session.target in this session; Quai will not start by itself at login." >&2
    echo "         Start it from the applications (Quai) or with: systemctl --user start quai" >&2
fi
"$HOME/.local/bin/quai" --doctor

if [ "${1:-}" = "--cosmic-dock" ]; then
    "$HOME/.local/bin/quai" --cosmic-dock "${2:-}"
fi
