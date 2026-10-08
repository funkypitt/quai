#!/bin/sh
# Removes Quai and gives COSMIC's dock back. Settings in ~/.config/quai are kept.
set -eu
systemctl --user disable --now quai.service 2>/dev/null || true
rm -f "$HOME/.config/systemd/user/quai.service" "$HOME/.local/bin/quai" \
      "$HOME/.local/share/applications/quai.desktop" "$HOME/.local/share/icons/hicolor/scalable/apps/quai.svg"
systemctl --user daemon-reload
entries="$HOME/.config/cosmic/com.system76.CosmicPanel/v1/entries"
if [ -f "$entries.before-quai" ]; then
    cp "$entries.before-quai" "$entries"
    echo "COSMIC dock restored."
fi
echo "Quai is removed."
