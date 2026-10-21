//! The desktop menu (spec 2.2, 2.3): which tile or settings row is selected,
//! and what each key press or click on the bar asks the app to do.

use crate::settings::{Dir, ITEMS, SettingItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Settings,
    Plugins,
    Update,
}

/// The bar's tiles, left to right.
pub const TILES: [Tile; 3] = [Tile::Settings, Tile::Plugins, Tile::Update];

impl Tile {
    pub fn label(self) -> &'static str {
        match self {
            Tile::Settings => "설정",
            Tile::Plugins => "플러그인",
            Tile::Update => "업데이트",
        }
    }
}

/// Keys the menu reacts to. `Open` is whatever `open_key` currently is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKey {
    Left,
    Right,
    Up,
    Down,
    Enter,
    Escape,
    Open,
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuHit {
    Tile(usize),
    /// A settings row, away from its ◀ / ▶.
    Row(usize),
    /// The ◀ (`Back`) or ▶ (`Forward`) of a settings row.
    Step(usize, Dir),
    /// Inside the bar or panel but on no tile or row (margins, the notice).
    Panel,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    None,
    Redraw,
    /// Move the setting one step; the app applies and saves it.
    Change(SettingItem, Dir),
    /// A tile whose feature isn't there yet.
    Notice(Tile),
    Close,
}

#[derive(Debug)]
pub struct MenuSession {
    tile: usize,
    /// The selected settings row; `None` while the panel is closed.
    row: Option<usize>,
    hovered: MenuHit,
}

impl Default for MenuSession {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuSession {
    pub fn new() -> Self {
        MenuSession {
            tile: 0,
            row: None,
            hovered: MenuHit::Outside,
        }
    }

    pub fn tile(&self) -> usize {
        self.tile
    }

    pub fn row(&self) -> Option<usize> {
        self.row
    }

    pub fn hovered(&self) -> MenuHit {
        self.hovered
    }

    pub fn key(&mut self, key: MenuKey) -> MenuAction {
        match (key, self.row) {
            (MenuKey::Open, _) => MenuAction::Close,
            (MenuKey::Left, Some(row)) => MenuAction::Change(ITEMS[row], Dir::Back),
            (MenuKey::Right, Some(row)) => MenuAction::Change(ITEMS[row], Dir::Forward),
            (MenuKey::Up, Some(0)) | (MenuKey::Escape, Some(_)) => {
                self.row = None;
                MenuAction::Redraw
            }
            (MenuKey::Up, Some(row)) => self.select_row(row - 1),
            (MenuKey::Down, Some(row)) if row + 1 < ITEMS.len() => self.select_row(row + 1),
            (MenuKey::Down | MenuKey::Enter, Some(_)) => MenuAction::None,
            (MenuKey::Left, None) if self.tile > 0 => {
                self.tile -= 1;
                MenuAction::Redraw
            }
            (MenuKey::Right, None) if self.tile + 1 < TILES.len() => {
                self.tile += 1;
                MenuAction::Redraw
            }
            (MenuKey::Left | MenuKey::Right | MenuKey::Up, None) => MenuAction::None,
            (MenuKey::Down | MenuKey::Enter, None) => self.activate(),
            (MenuKey::Escape, None) => MenuAction::Close,
        }
    }

    /// `true` if the bar needs redrawing.
    pub fn hover(&mut self, hit: MenuHit) -> bool {
        let mut changed = self.hovered != hit;
        self.hovered = hit;
        if let MenuHit::Tile(i) = hit
            && i != self.tile
        {
            self.select_tile(i);
            changed = true;
        }
        changed
    }

    pub fn click(&mut self, hit: MenuHit) -> MenuAction {
        match hit {
            MenuHit::Tile(i) => {
                self.select_tile(i);
                self.activate()
            }
            MenuHit::Row(i) if self.row.is_some() => self.select_row(i),
            MenuHit::Step(i, dir) if self.row.is_some() => {
                self.row = Some(i);
                MenuAction::Change(ITEMS[i], dir)
            }
            MenuHit::Outside => MenuAction::Close,
            _ => MenuAction::None,
        }
    }

    /// The mouse wheel over a settings row works like ← / →.
    pub fn scroll(&mut self, hit: MenuHit, dir: Dir) -> MenuAction {
        match hit {
            MenuHit::Row(i) | MenuHit::Step(i, _) if self.row.is_some() => {
                self.row = Some(i);
                MenuAction::Change(ITEMS[i], dir)
            }
            _ => MenuAction::None,
        }
    }

    fn select_tile(&mut self, i: usize) {
        if i < TILES.len() {
            self.tile = i;
            if TILES[i] != Tile::Settings {
                self.row = None;
            }
        }
    }

    fn select_row(&mut self, i: usize) -> MenuAction {
        if i >= ITEMS.len() || self.row == Some(i) {
            return MenuAction::None;
        }
        self.row = Some(i);
        MenuAction::Redraw
    }

    fn activate(&mut self) -> MenuAction {
        match TILES[self.tile] {
            Tile::Settings if self.row.is_some() => MenuAction::None,
            Tile::Settings => {
                self.row = Some(0);
                MenuAction::Redraw
            }
            tile => MenuAction::Notice(tile),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Dir, SettingItem};

    #[test]
    fn starts_on_settings_and_stops_at_ends() {
        let mut m = MenuSession::new();
        assert_eq!((m.tile(), m.row()), (0, None));
        assert_eq!(m.key(MenuKey::Left), MenuAction::None);
        assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
        assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
        assert_eq!(m.key(MenuKey::Right), MenuAction::None);
        assert_eq!(m.tile(), 2);
    }

    #[test]
    fn enter_or_down_runs_the_tile() {
        let mut m = MenuSession::new();
        m.key(MenuKey::Right);
        assert_eq!(m.key(MenuKey::Enter), MenuAction::Notice(Tile::Plugins));
        m.key(MenuKey::Right);
        assert_eq!(m.key(MenuKey::Down), MenuAction::Notice(Tile::Update));
        let mut m = MenuSession::new();
        assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
        assert_eq!(m.row(), Some(0));
    }

    #[test]
    fn panel_rows_change_values_and_close_step_by_step() {
        let mut m = MenuSession::new();
        m.key(MenuKey::Enter);
        assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
        assert_eq!(
            m.key(MenuKey::Right),
            MenuAction::Change(SettingItem::CycleKey, Dir::Forward)
        );
        assert_eq!(
            m.key(MenuKey::Left),
            MenuAction::Change(SettingItem::CycleKey, Dir::Back)
        );
        for _ in 0..20 {
            m.key(MenuKey::Down);
        }
        assert_eq!(m.row(), Some(8)); // stops at the last row
        assert_eq!(m.key(MenuKey::Escape), MenuAction::Redraw);
        assert_eq!(m.row(), None);
        assert_eq!(m.key(MenuKey::Escape), MenuAction::Close);
    }

    #[test]
    fn up_on_first_row_returns_to_bar_and_open_closes_everything() {
        let mut m = MenuSession::new();
        m.key(MenuKey::Enter);
        assert_eq!(m.key(MenuKey::Up), MenuAction::Redraw);
        assert_eq!(m.row(), None);
        m.key(MenuKey::Enter);
        assert_eq!(m.key(MenuKey::Open), MenuAction::Close);
    }

    #[test]
    fn left_right_on_bar_with_panel_closed_only_moves_tiles() {
        let mut m = MenuSession::new();
        m.key(MenuKey::Enter);
        m.key(MenuKey::Escape);
        assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
        assert_eq!(m.tile(), 1);
    }

    #[test]
    fn mouse_hits() {
        let mut m = MenuSession::new();
        assert!(m.hover(MenuHit::Tile(2)));
        assert_eq!(m.tile(), 2);
        assert!(!m.hover(MenuHit::Tile(2)));
        assert_eq!(m.click(MenuHit::Tile(0)), MenuAction::Redraw); // opens the panel
        assert_eq!(m.row(), Some(0));
        assert_eq!(m.click(MenuHit::Row(4)), MenuAction::Redraw);
        assert_eq!(m.row(), Some(4));
        assert_eq!(
            m.click(MenuHit::Step(5, Dir::Back)),
            MenuAction::Change(SettingItem::WebpQuality, Dir::Back)
        );
        assert_eq!(m.row(), Some(5));
        assert_eq!(
            m.scroll(MenuHit::Row(6), Dir::Forward),
            MenuAction::Change(SettingItem::Mp3Bitrate, Dir::Forward)
        );
        assert_eq!(m.scroll(MenuHit::Tile(1), Dir::Forward), MenuAction::None);
        assert_eq!(m.click(MenuHit::Panel), MenuAction::None);
        assert_eq!(m.click(MenuHit::Outside), MenuAction::Close);
        assert_eq!(m.click(MenuHit::Tile(0)), MenuAction::None); // settings already open
    }
}
