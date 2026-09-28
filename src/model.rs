//! What the dock shows: two columns of tiles built from the pinned
//! applications and the open windows. Pure logic, no display code.

use crate::config::ClickActive;

pub type WinId = u32;

#[derive(Debug, Clone, PartialEq)]
pub struct Win {
    pub id: WinId,
    pub app_id: String,
    pub title: String,
    /// Application key: a desktop file id when one matches.
    pub key: String,
    pub active: bool,
    pub minimized: bool,
    /// Order of appearance.
    pub seq: u64,
    /// Grows each time the window takes the focus.
    pub last_active: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Applications,
    Workspaces,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TileId {
    Button(Button),
    /// One tile for an application and all its windows.
    App(String),
    /// One tile for a single window (right column, ungrouped).
    Window(WinId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub id: TileId,
    pub key: String,
    /// Windows of the tile, in order of appearance.
    pub wins: Vec<WinId>,
    pub active: bool,
    pub pinned: bool,
}

pub const LEFT: usize = 0;

/// Pinned applications on the left; on the right, what is open and not pinned.
pub fn build(pinned: &[String], wins: &[Win], group: bool) -> [Vec<Tile>; 2] {
    let mut sorted: Vec<&Win> = wins.iter().collect();
    sorted.sort_by_key(|w| w.seq);

    let left = pinned
        .iter()
        .map(|key| {
            let mine: Vec<&&Win> = sorted.iter().filter(|w| &w.key == key).collect();
            Tile {
                id: TileId::App(key.clone()),
                key: key.clone(),
                wins: mine.iter().map(|w| w.id).collect(),
                active: mine.iter().any(|w| w.active),
                pinned: true,
            }
        })
        .collect();

    // Applications in their order of arrival; the windows of one
    // application stay side by side, grouped or not.
    let mut unpinned: Vec<&&Win> = sorted.iter().filter(|w| !pinned.contains(&w.key)).collect();
    let first_seen = |key: &str| sorted.iter().find(|w| w.key == key).map_or(0, |w| w.seq);
    unpinned.sort_by_key(|w| (first_seen(&w.key), w.seq));

    let mut right: Vec<Tile> = Vec::new();
    for w in unpinned {
        if group {
            if let Some(t) = right.iter_mut().find(|t| t.key == w.key) {
                t.wins.push(w.id);
                t.active |= w.active;
                continue;
            }
            right.push(Tile {
                id: TileId::App(w.key.clone()),
                key: w.key.clone(),
                wins: vec![w.id],
                active: w.active,
                pinned: false,
            });
        } else {
            right.push(Tile {
                id: TileId::Window(w.id),
                key: w.key.clone(),
                wins: vec![w.id],
                active: w.active,
                pinned: false,
            });
        }
    }
    [left, right]
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Launch(String),
    Activate(WinId),
    Minimize(WinId),
    Press(Button),
}

/// What a plain click on a tile does.
pub fn click(tile: &Tile, wins: &[Win], on_active: ClickActive) -> Action {
    if let TileId::Button(b) = tile.id {
        return Action::Press(b);
    }
    let mine: Vec<&Win> = tile.wins.iter().filter_map(|id| wins.iter().find(|w| w.id == *id)).collect();
    if mine.is_empty() {
        return Action::Launch(tile.key.clone());
    }
    match mine.iter().position(|w| w.active) {
        // Not in use: back to the window used last.
        None => Action::Activate(mine.iter().max_by_key(|w| (w.last_active, w.seq)).unwrap().id),
        // In use with several windows: on to the next one.
        Some(i) if mine.len() > 1 => Action::Activate(mine[(i + 1) % mine.len()].id),
        Some(i) => match on_active {
            ClickActive::Minimize => Action::Minimize(mine[i].id),
            ClickActive::Cycle => Action::None,
        },
    }
}

/// Moves `key` to `index` among the pinned applications, pinning it if needed.
/// `index` counts positions in the list as it is shown before the move.
pub fn pin_at(pinned: &mut Vec<String>, key: &str, index: usize) {
    let mut index = index.min(pinned.len());
    if let Some(old) = pinned.iter().position(|k| k == key) {
        pinned.remove(old);
        if old < index {
            index -= 1;
        }
    }
    pinned.insert(index.min(pinned.len()), key.to_string());
}

pub fn unpin(pinned: &mut Vec<String>, key: &str) {
    pinned.retain(|k| k != key);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: WinId, key: &str, active: bool) -> Win {
        Win {
            id,
            app_id: key.into(),
            title: format!("{key} {id}"),
            key: key.into(),
            active,
            minimized: false,
            seq: id as u64,
            last_active: 0,
        }
    }

    fn keys(tiles: &[Tile]) -> Vec<&str> {
        tiles.iter().map(|t| t.key.as_str()).collect()
    }

    #[test]
    fn columns() {
        let pinned = vec!["mail".to_string(), "files".to_string()];
        let wins = vec![win(1, "term", false), win(2, "mail", true), win(3, "term", false), win(4, "edit", false)];

        let [left, right] = build(&pinned, &wins, true);
        assert_eq!(keys(&left), ["mail", "files"]);
        assert_eq!(left[0].wins, [2]);
        assert!(left[0].active && left[1].wins.is_empty());
        assert_eq!(keys(&right), ["term", "edit"]);
        assert_eq!(right[0].wins, [1, 3]);

        let [left, right] = build(&pinned, &wins, false);
        assert_eq!(left.len(), 2, "pinned applications keep a single tile");
        assert_eq!(keys(&right), ["term", "term", "edit"]);
        assert_eq!(right[1].id, TileId::Window(3));

        // A window opened later joins the others of its application.
        let mut wins = wins;
        wins.push(win(5, "term", false));
        let [_, right] = build(&pinned, &wins, false);
        assert_eq!(keys(&right), ["term", "term", "term", "edit"]);
    }

    #[test]
    fn a_pinned_application_never_shows_on_the_right() {
        let pinned = vec!["mail".to_string()];
        let wins = vec![win(1, "mail", false), win(2, "mail", false)];
        for group in [true, false] {
            let [left, right] = build(&pinned, &wins, group);
            assert_eq!(left[0].wins, [1, 2]);
            assert!(right.is_empty());
        }
    }

    #[test]
    fn clicks() {
        let mut wins = vec![win(1, "term", false), win(2, "term", false), win(3, "edit", true)];
        wins[0].last_active = 5;
        wins[1].last_active = 9;
        let [_, right] = build(&[], &wins, true);
        let (term, edit) = (&right[0], &right[1]);

        assert_eq!(click(term, &wins, ClickActive::Minimize), Action::Activate(2));
        assert_eq!(click(edit, &wins, ClickActive::Minimize), Action::Minimize(3));
        assert_eq!(click(edit, &wins, ClickActive::Cycle), Action::None);

        wins[1].active = true;
        let [_, right] = build(&[], &wins, true);
        assert_eq!(click(&right[0], &wins, ClickActive::Minimize), Action::Activate(1));

        let [left, _] = build(&["calc".to_string()], &wins, true);
        assert_eq!(click(&left[0], &wins, ClickActive::Minimize), Action::Launch("calc".into()));
    }

    #[test]
    fn pinning() {
        let mut p: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        pin_at(&mut p, "c", 0);
        assert_eq!(p, ["c", "a", "b"]);
        pin_at(&mut p, "c", 3);
        assert_eq!(p, ["a", "b", "c"]);
        pin_at(&mut p, "a", 1);
        assert_eq!(p, ["a", "b", "c"], "dropping a tile on its own place changes nothing");
        pin_at(&mut p, "a", 2);
        assert_eq!(p, ["b", "a", "c"]);
        pin_at(&mut p, "new", 99);
        assert_eq!(p, ["b", "a", "c", "new"]);
        unpin(&mut p, "a");
        assert_eq!(p, ["b", "c", "new"]);
    }
}
