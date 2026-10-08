# Quai

English · [Français](README.fr.md)

A two-column side dock for the COSMIC desktop, in the style of Unity's
launcher (Ubuntu): each tile is lit with the colour of its icon.

<img src="docs/quai.png" width="550" alt="Quai: pinned applications on the left, open ones on the right, a tooltip and a tile's menu">

- **Left column**: the pinned applications.
- **Right column**: the open applications that are not pinned, grouped per
  application or one tile per window, as you choose.

Fixed width of 130 px: two Unity launchers side by side, in its default
proportions (54 px tiles, 48 px icons, 5 px gaps).

## Usage

| Gesture | Effect |
|---|---|
| Click | starts the application, or returns to its last window |
| Click on the current application | goes to its next window; if it has only one, minimizes it |
| Middle click | opens a new window |
| Right click on a tile | windows, application actions, pin or unpin, close |
| Right click elsewhere | dock settings |
| Drag into the left column | pins, or changes the order |
| Drag out of the left column | unpins |
| Wheel | scrolls a column that is too long |
| Tap (touchscreen) | as a click |
| Finger held on a tile | as a right click: the menu opens under the finger |
| Finger dragged | as a drag with the mouse |

The small arrows are Unity's: on the outer edge, one per open window (three
at most); in the middle, the one of the current application.

## Installation

On Pop!_OS and Ubuntu, from the same apt repository as the Reader's desktop
apps ([funkypitt.github.io/apt-repo](https://funkypitt.github.io/apt-repo)):

```sh
sudo apt install quai
```

The package starts Quai with every COSMIC session, for every user, from the
next login on; the `.deb` is also attached to each
[release](https://github.com/funkypitt/quai/releases). On Arch and Manjaro,
`packaging/PKGBUILD`.

From the source, Rust is needed (`rustup`); there is no other dependency. Run the
script as yourself, never with `sudo` (it installs in your home and your session):

```sh
./install.sh                     # builds, installs for this user, starts with the session
./uninstall.sh                   # removes Quai and gives COSMIC's dock back
```

Quai is then listed among the applications (to start it by hand), and
`quai --doctor` says in one screen whether the session is COSMIC, whether the
service is enabled and running, and where the settings are.

Quai runs as a service of the session (`systemctl --user status quai`),
restarted if it fails. When it starts it switches COSMIC's own dock off, so
that only one dock shows (`hide_cosmic_dock = false` in the settings keeps
both); `quai --cosmic-dock on` gives COSMIC's dock back at any time.

## Settings

`~/.config/quai/config.toml`, read again as soon as it changes. The same
choices are offered by a right click on the dock. On first start, the
applications pinned in COSMIC's dock are taken over.

| Key | Values | Role |
|---|---|---|
| `pinned` | list | pinned applications, top to bottom |
| `group_windows` | `true`, `false` | on the right: one tile per application or per window |
| `backlight` | `"always"`, `"running"` | every tile lit (Unity), or only the open applications |
| `click_active` | `"minimize"`, `"cycle"` | effect of a click on the current application |
| `buttons` | `true`, `false` | Applications and Workspaces buttons |
| `opacity` | 0.0 to 1.0 | dock background |
| `tint` | `"wallpaper"`, `"none"`, `"#rrggbb"` | colour of the dock: drawn from the wallpaper (Unity), near black, or chosen |
| `blur` | `true`, `false` | blur behind the dock |
| `output` | `"all"` or an output name | screens where the dock shows |
| `icon_theme` | name | empty: the desktop's theme |
| `hide_cosmic_dock` | `true`, `false` | switch COSMIC's own dock off when Quai starts |

## Known limits

- The windows of all workspaces are shown.
- COSMIC's `zcosmic_*` protocols are not stable: an update of COSMIC may
  require adapting Quai (the version of the library is pinned in
  `Cargo.toml`).
- Tried on a single screen, at 100 % scale. Touch is wired through the same
  path as the mouse, but has not been tried on a touchscreen yet.

## Development

```sh
cargo test
cargo run -- --preview preview.png     # draws the dock into an image, without a display
QUAI_LOG=debug cargo run               # detailed log
cargo run -- --test-panel 10           # fake panel, to try the placement
cargo run -- --windows                 # open windows, application recognised, position
```

Two particularities of COSMIC that the code allows for:

- it serves panels in their order of arrival: the dock moves when a panel
  arrives after it (`check_placement`);
- it only tightens maximized windows when a panel changes size, not when it
  arrives: the dock is born one pixel narrower, then takes its width
  (`settle`).

Licence: GPL-3.0-or-later. The wallpaper in the picture is Adwaita, by
Jakub Steiner, from the GNOME backgrounds (CC BY-SA 3.0).
