#!/bin/sh
# Builds quai_<version>_<arch>.deb next to this script. Needs cargo, dpkg-deb and fakeroot.
# The package starts Quai with every COSMIC session (systemctl --global enable, in postinst).
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"; SRC="$HERE/.."
PATH="$HOME/.cargo/bin:$PATH"
VERSION=$(grep -m1 '^version = ' "$SRC/Cargo.toml" | cut -d'"' -f2)
ARCH=$(dpkg --print-architecture)
ROOT="$HERE/deb-root"; rm -rf "$ROOT"
(cd "$SRC" && cargo build --release)
install -Dm755 "$SRC/target/release/quai" "$ROOT/usr/bin/quai"
# the same unit as data/quai.service, pointing at the packaged binary
sed 's|ExecStart=%h/.local/bin/quai|ExecStart=/usr/bin/quai|' "$SRC/data/quai.service" > "$HERE/quai.service.tmp"
install -Dm644 "$HERE/quai.service.tmp" "$ROOT/usr/lib/systemd/user/quai.service"; rm -f "$HERE/quai.service.tmp"
install -Dm644 "$SRC/data/quai.desktop" "$ROOT/usr/share/applications/quai.desktop"
install -Dm644 "$SRC/data/quai.svg" "$ROOT/usr/share/icons/hicolor/scalable/apps/quai.svg"
install -Dm644 "$SRC/LICENSE" "$ROOT/usr/share/doc/quai/copyright"
install -Dm644 "$SRC/README.md" "$ROOT/usr/share/doc/quai/README.md"
install -Dm644 "$SRC/README.fr.md" "$ROOT/usr/share/doc/quai/README.fr.md"
mkdir -p "$ROOT/DEBIAN"
cat > "$ROOT/DEBIAN/control" <<CTRL
Package: quai
Version: $VERSION
Section: x11
Priority: optional
Architecture: $ARCH
Depends: libc6 (>= 2.34)
Recommends: cosmic-session
Maintainer: funkypitt <pierregallaz@gmail.com>
Homepage: https://github.com/funkypitt/quai
Description: Two-column Unity-style dock for the COSMIC desktop
 Pinned applications in the left column, open windows in the right one,
 every tile lit with the colour of its icon, as in Ubuntu's Unity. Mouse
 and touch. Starts with every COSMIC session and, unless told otherwise
 in ~/.config/quai/config.toml, switches COSMIC's own dock off.
CTRL
cat > "$ROOT/DEBIAN/postinst" <<'SH'
#!/bin/sh
set -e
# Started with every COSMIC session, for every user (next login).
if [ "$1" = "configure" ] && command -v systemctl >/dev/null 2>&1; then
    systemctl --global enable quai.service >/dev/null 2>&1 || true
fi
SH
cat > "$ROOT/DEBIAN/prerm" <<'SH'
#!/bin/sh
set -e
if [ "$1" = "remove" ] && command -v systemctl >/dev/null 2>&1; then
    systemctl --global disable quai.service >/dev/null 2>&1 || true
fi
SH
chmod 755 "$ROOT/DEBIAN/postinst" "$ROOT/DEBIAN/prerm"
fakeroot dpkg-deb --build "$ROOT" "$HERE/quai_${VERSION}_${ARCH}.deb"
rm -rf "$ROOT"
echo "built $HERE/quai_${VERSION}_${ARCH}.deb"
