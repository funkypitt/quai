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

The small arrows are Unity's: on the outer edge, one per open window (three
at most); in the middle, the one of the current application.

## Installation

Rust is needed (`rustup`); there is no other dependency to install.

```sh
./install.sh                     # builds, installs, starts with the session
./install.sh --cosmic-dock off   # also switches COSMIC's dock off
./uninstall.sh                   # removes Quai and gives COSMIC's dock back
```

Quai runs as a service of the session (`systemctl --user status quai`),
restarted if it fails.

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

## Known limits

- The windows of all workspaces are shown.
- COSMIC's `zcosmic_*` protocols are not stable: an update of COSMIC may
  require adapting Quai (the version of the library is pinned in
  `Cargo.toml`).
- Tried on a single screen, at 100 % scale.

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
