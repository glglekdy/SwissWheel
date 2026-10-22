//! The desktop menu (spec 2.2, 2.3): which tile or settings row is selected,
//! and what each key press or click on the bar asks the app to do.

use crate::settings::{CATEGORIES, Category, Dir, SettingItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Settings,
    Download,
    Plugins,
    Update,
    Restart,
}

/// The bar's tiles, left to right.
pub const TILES: [Tile; 5] = [
    Tile::Settings,
    Tile::Download,
    Tile::Plugins,
    Tile::Update,
    Tile::Restart,
];

impl Tile {
    pub fn label(self) -> &'static str {
        match self {
            Tile::Settings => "설정",
            Tile::Download => "다운로드",
            Tile::Plugins => "플러그인",
            Tile::Update => "업데이트",
            Tile::Restart => "재시작",
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
    /// A category tab of the settings panel.
    Tab(usize),
    /// A settings row, away from its ◀ / ▶ or keycap.
    Row(usize),
    /// The ◀ (`Back`) or ▶ (`Forward`) of a settings row.
    Step(usize, Dir),
    /// The key shown on a shortcut row; clicking it waits for a key.
    KeyCap(usize),
    /// Inside the bar or panel but on nothing that reacts (margins, the
    /// notice, the hint line).
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
    /// Wait for a key for the selected shortcut row: the app has the hook
    /// hand over the next key press.
    StartCapture,
    /// This key (a virtual-key code) was pressed for this shortcut row; the
    /// app checks it can be bound.
    Bind(SettingItem, u16),
    /// Stop waiting for a key.
    CancelCapture,
}

/// Where the keyboard is in the open settings panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tabs,
    Row(usize),
}

#[derive(Debug)]
pub struct MenuSession {
    tile: usize,
    /// Index into `CATEGORIES`; kept while the panel closes and reopens.
    category: usize,
    /// `None` while the panel is closed.
    panel: Option<Focus>,
    /// Waiting for a key for the focused shortcut row.
    capturing: bool,
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
            category: 0,
            panel: None,
            capturing: false,
            hovered: MenuHit::Outside,
        }
    }

    pub fn tile(&self) -> usize {
        self.tile
    }

    pub fn category(&self) -> Category {
        CATEGORIES[self.category]
    }

    pub fn focus(&self) -> Option<Focus> {
        self.panel
    }

    pub fn capturing(&self) -> bool {
        self.capturing
    }

    pub fn hovered(&self) -> MenuHit {
        self.hovered
    }

    /// The hint line under the rows.
    pub fn hint(&self) -> &'static str {
        match self.panel {
            _ if self.capturing => "바꿀 키를 누르세요 · 다른 곳 클릭 시 취소",
            None => "",
            Some(Focus::Tabs) => "←/→ 카테고리 · ↓ 항목",
            Some(Focus::Row(i)) if self.item(i).is_some_and(SettingItem::is_key) => {
                "Enter로 키 변경"
            }
            Some(Focus::Row(_)) => "←/→로 값 변경",
        }
    }

    pub fn key(&mut self, key: MenuKey) -> MenuAction {
        if self.capturing {
            return MenuAction::None; // the hook sends the key as MenuCapture
        }
        match (key, self.panel) {
            (MenuKey::Open, _) => MenuAction::Close,
            (MenuKey::Left, Some(Focus::Tabs)) if self.category > 0 => {
                self.set_category(self.category - 1)
            }
            (MenuKey::Right, Some(Focus::Tabs)) if self.category + 1 < CATEGORIES.len() => {
                self.set_category(self.category + 1)
            }
            (MenuKey::Left | MenuKey::Right, Some(Focus::Tabs)) => MenuAction::None,
            (MenuKey::Down | MenuKey::Enter, Some(Focus::Tabs)) => self.focus_row(0),
            (MenuKey::Up | MenuKey::Escape, Some(Focus::Tabs)) => {
                self.panel = None;
                MenuAction::Redraw
            }
            (MenuKey::Left | MenuKey::Right, Some(Focus::Row(i))) => {
                let dir = if key == MenuKey::Left {
                    Dir::Back
                } else {
                    Dir::Forward
                };
                match self.item(i) {
                    Some(item) if !item.is_key() => MenuAction::Change(item, dir),
                    _ => MenuAction::None,
                }
            }
            (MenuKey::Up, Some(Focus::Row(0))) | (MenuKey::Escape, Some(Focus::Row(_))) => {
                self.panel = Some(Focus::Tabs);
                MenuAction::Redraw
            }
            (MenuKey::Up, Some(Focus::Row(i))) => self.focus_row(i - 1),
            (MenuKey::Down, Some(Focus::Row(i))) => self.focus_row(i + 1),
            (MenuKey::Enter, Some(Focus::Row(i))) => self.start_capture(i),
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

    /// `true` if the bar needs redrawing. Hovering a tab only highlights it;
    /// hovering a tile selects it, except while waiting for a key.
    pub fn hover(&mut self, hit: MenuHit) -> bool {
        let mut changed = self.hovered != hit;
        self.hovered = hit;
        if let MenuHit::Tile(i) = hit
            && i != self.tile
            && !self.capturing
        {
            self.select_tile(i);
            changed = true;
        }
        changed
    }

    pub fn click(&mut self, hit: MenuHit) -> MenuAction {
        if self.capturing {
            if hit == MenuHit::Outside {
                return MenuAction::Close;
            }
            if hit == MenuHit::KeyCap(self.focused_row().unwrap_or(usize::MAX)) {
                return MenuAction::None;
            }
            self.capturing = false;
            return match self.click_idle(hit) {
                MenuAction::None | MenuAction::Redraw => MenuAction::CancelCapture,
                action => action,
            };
        }
        self.click_idle(hit)
    }

    fn click_idle(&mut self, hit: MenuHit) -> MenuAction {
        let panel_open = self.panel.is_some();
        match hit {
            MenuHit::Tile(i) => {
                self.select_tile(i);
                self.activate()
            }
            MenuHit::Tab(i) if panel_open && i < CATEGORIES.len() => {
                self.category = i;
                self.panel = Some(Focus::Tabs);
                MenuAction::Redraw
            }
            MenuHit::Row(i) if panel_open => self.focus_row(i),
            MenuHit::Step(i, dir) if panel_open => match self.item(i) {
                Some(item) if !item.is_key() => {
                    self.panel = Some(Focus::Row(i));
                    MenuAction::Change(item, dir)
                }
                _ => MenuAction::None,
            },
            MenuHit::KeyCap(i) if panel_open => self.start_capture(i),
            MenuHit::Outside => MenuAction::Close,
            _ => MenuAction::None,
        }
    }

    /// The mouse wheel over a settings row works like ← / →.
    pub fn scroll(&mut self, hit: MenuHit, dir: Dir) -> MenuAction {
        if self.capturing || self.panel.is_none() {
            return MenuAction::None;
        }
        match hit {
            MenuHit::Row(i) | MenuHit::Step(i, _) | MenuHit::KeyCap(i) => match self.item(i) {
                Some(item) if !item.is_key() => {
                    self.panel = Some(Focus::Row(i));
                    MenuAction::Change(item, dir)
                }
                _ => MenuAction::None,
            },
            _ => MenuAction::None,
        }
    }

    /// The key pressed while waiting: bind it to the focused row.
    pub fn capture(&mut self, vk: u16) -> MenuAction {
        if !self.capturing {
            return MenuAction::None;
        }
        self.capturing = false;
        match self.focused_row().and_then(|i| self.item(i)) {
            Some(item) => MenuAction::Bind(item, vk),
            None => MenuAction::Redraw,
        }
    }

    /// Keep waiting after the app turned a key down.
    pub fn rearm(&mut self) {
        if self.focused_row().is_some() {
            self.capturing = true;
        }
    }

    /// Stop waiting (the timeout). `true` if it was waiting.
    pub fn cancel_capture(&mut self) -> bool {
        std::mem::replace(&mut self.capturing, false)
    }

    fn item(&self, row: usize) -> Option<SettingItem> {
        self.category().items().get(row).copied()
    }

    fn focused_row(&self) -> Option<usize> {
        match self.panel {
            Some(Focus::Row(i)) => Some(i),
            _ => None,
        }
    }

    fn set_category(&mut self, i: usize) -> MenuAction {
        self.category = i;
        MenuAction::Redraw
    }

    fn select_tile(&mut self, i: usize) {
        if i < TILES.len() {
            self.tile = i;
            if TILES[i] != Tile::Settings {
                self.panel = None;
            }
        }
    }

    fn focus_row(&mut self, i: usize) -> MenuAction {
        if self.item(i).is_none() || self.panel == Some(Focus::Row(i)) {
            return MenuAction::None;
        }
        self.panel = Some(Focus::Row(i));
        MenuAction::Redraw
    }

    fn start_capture(&mut self, i: usize) -> MenuAction {
        match self.item(i) {
            Some(item) if item.is_key() => {
                self.panel = Some(Focus::Row(i));
                self.capturing = true;
                MenuAction::StartCapture
            }
            _ => MenuAction::None,
        }
    }

    fn activate(&mut self) -> MenuAction {
        match TILES[self.tile] {
            Tile::Settings if self.panel.is_some() => MenuAction::None,
            Tile::Settings => {
                self.panel = Some(Focus::Tabs);
                MenuAction::Redraw
            }
            tile => MenuAction::Notice(tile),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Category, Dir, SettingItem};

    fn panel() -> MenuSession {
        let mut m = MenuSession::new();
        assert_eq!(m.key(MenuKey::Enter), MenuAction::Redraw);
        m
    }

    #[test]
    fn starts_on_settings_and_stops_at_ends() {
        let mut m = MenuSession::new();
        assert_eq!((m.tile(), m.focus()), (0, None));
        assert_eq!(m.key(MenuKey::Left), MenuAction::None);
        for _ in 0..4 {
            assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
        }
        assert_eq!(m.key(MenuKey::Right), MenuAction::None);
        assert_eq!(m.tile(), 4);
        assert_eq!(m.hint(), "");
    }

    #[test]
    fn tile_order_and_labels() {
        assert_eq!(
            TILES,
            [
                Tile::Settings,
                Tile::Download,
                Tile::Plugins,
                Tile::Update,
                Tile::Restart
            ]
        );
        assert_eq!(Tile::Download.label(), "다운로드");
        assert_eq!(Tile::Restart.label(), "재시작");
    }

    #[test]
    fn enter_or_down_runs_the_tile() {
        let mut m = MenuSession::new();
        m.key(MenuKey::Right);
        assert_eq!(m.key(MenuKey::Enter), MenuAction::Notice(Tile::Download));
        m.key(MenuKey::Right);
        assert_eq!(m.key(MenuKey::Enter), MenuAction::Notice(Tile::Plugins));
        m.key(MenuKey::Right);
        assert_eq!(m.key(MenuKey::Down), MenuAction::Notice(Tile::Update));
        m.key(MenuKey::Right);
        assert_eq!(m.key(MenuKey::Enter), MenuAction::Notice(Tile::Restart));
        let mut m = MenuSession::new();
        assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
        assert_eq!(m.focus(), Some(Focus::Tabs));
    }

    #[test]
    fn enter_opens_panel_on_tabs_of_keys() {
        let m = panel();
        assert_eq!(
            (m.focus(), m.category()),
            (Some(Focus::Tabs), Category::Keys)
        );
        assert_eq!(m.hint(), "←/→ 카테고리 · ↓ 항목");
    }

    #[test]
    fn tabs_switch_and_stop_at_ends() {
        let mut m = panel();
        assert_eq!(m.key(MenuKey::Left), MenuAction::None);
        for _ in 0..3 {
            assert_eq!(m.key(MenuKey::Right), MenuAction::Redraw);
        }
        assert_eq!(m.key(MenuKey::Right), MenuAction::None);
        assert_eq!(m.category(), Category::General);
        assert_eq!(m.focus(), Some(Focus::Tabs));
    }

    #[test]
    fn rows_move_within_category_and_back_to_tabs() {
        let mut m = panel();
        m.key(MenuKey::Right); // 도구: 2 rows
        assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
        assert_eq!(m.key(MenuKey::Down), MenuAction::Redraw);
        assert_eq!(m.key(MenuKey::Down), MenuAction::None);
        assert_eq!(m.focus(), Some(Focus::Row(1)));
        assert_eq!(m.hint(), "←/→로 값 변경");
        assert_eq!(
            m.key(MenuKey::Right),
            MenuAction::Change(SettingItem::CompressVideo, Dir::Forward)
        );
        m.key(MenuKey::Up);
        assert_eq!(m.key(MenuKey::Up), MenuAction::Redraw);
        assert_eq!(m.focus(), Some(Focus::Tabs));
        m.key(MenuKey::Down);
        assert_eq!(m.key(MenuKey::Escape), MenuAction::Redraw);
        assert_eq!(m.focus(), Some(Focus::Tabs));
        assert_eq!(m.key(MenuKey::Escape), MenuAction::Redraw);
        assert_eq!(m.focus(), None);
        assert_eq!(m.key(MenuKey::Escape), MenuAction::Close);
    }

    #[test]
    fn category_is_kept_when_panel_reopens() {
        let mut m = panel();
        m.key(MenuKey::Right);
        m.key(MenuKey::Up); // closes the panel
        assert_eq!(m.focus(), None);
        m.key(MenuKey::Enter);
        assert_eq!(m.category(), Category::Tools);
    }

    #[test]
    fn open_key_closes_everything() {
        let mut m = panel();
        m.key(MenuKey::Down);
        assert_eq!(m.key(MenuKey::Open), MenuAction::Close);
    }

    #[test]
    fn key_rows_capture_on_enter_and_ignore_left_right() {
        let mut m = panel();
        m.key(MenuKey::Down);
        assert_eq!(m.hint(), "Enter로 키 변경");
        assert_eq!(m.key(MenuKey::Right), MenuAction::None);
        assert_eq!(m.key(MenuKey::Enter), MenuAction::StartCapture);
        assert!(m.capturing());
        assert_eq!(m.hint(), "바꿀 키를 누르세요 · 다른 곳 클릭 시 취소");
        assert_eq!(m.key(MenuKey::Open), MenuAction::None); // the hook sends MenuCapture instead
        assert_eq!(
            m.capture(0x74),
            MenuAction::Bind(SettingItem::OpenKey, 0x74)
        );
        assert!(!m.capturing());
        assert_eq!(m.capture(0x74), MenuAction::None);
    }

    #[test]
    fn rearm_and_cancel() {
        let mut m = panel();
        m.key(MenuKey::Down);
        m.key(MenuKey::Enter);
        m.capture(0x41);
        m.rearm();
        assert!(m.capturing());
        assert!(m.cancel_capture());
        assert!(!m.cancel_capture());
    }

    #[test]
    fn mouse_on_tabs_rows_and_keycaps() {
        let mut m = panel();
        assert!(m.hover(MenuHit::Tab(2)));
        assert_eq!(m.category(), Category::Keys); // hover only highlights
        assert_eq!(m.click(MenuHit::Tab(2)), MenuAction::Redraw);
        assert_eq!(
            (m.category(), m.focus()),
            (Category::Convert, Some(Focus::Tabs))
        );
        assert_eq!(
            m.click(MenuHit::Step(1, Dir::Back)),
            MenuAction::Change(SettingItem::WebpQuality, Dir::Back)
        );
        assert_eq!(m.focus(), Some(Focus::Row(1)));
        assert_eq!(
            m.scroll(MenuHit::Row(2), Dir::Forward),
            MenuAction::Change(SettingItem::Mp3Bitrate, Dir::Forward)
        );
        m.click(MenuHit::Tab(0));
        assert_eq!(m.scroll(MenuHit::Row(0), Dir::Forward), MenuAction::None);
        assert_eq!(m.click(MenuHit::KeyCap(2)), MenuAction::StartCapture);
        assert_eq!(m.focus(), Some(Focus::Row(2)));
    }

    #[test]
    fn clicks_while_capturing() {
        let mut m = panel();
        m.click(MenuHit::KeyCap(0));
        assert_eq!(m.click(MenuHit::KeyCap(0)), MenuAction::None);
        assert!(m.capturing());
        assert_eq!(m.click(MenuHit::Row(1)), MenuAction::CancelCapture);
        assert_eq!((m.capturing(), m.focus()), (false, Some(Focus::Row(1))));
        m.click(MenuHit::KeyCap(1));
        assert_eq!(m.click(MenuHit::KeyCap(2)), MenuAction::StartCapture);
        assert!(m.capturing());
        assert_eq!(m.click(MenuHit::Outside), MenuAction::Close);
    }

    #[test]
    fn hovering_tiles_while_capturing_keeps_the_panel() {
        let mut m = panel();
        m.click(MenuHit::KeyCap(0));
        m.hover(MenuHit::Tile(1));
        assert_eq!((m.tile(), m.capturing()), (0, true));
        assert_eq!(m.scroll(MenuHit::Row(0), Dir::Back), MenuAction::None);
    }

    #[test]
    fn mouse_hits() {
        let mut m = MenuSession::new();
        assert!(m.hover(MenuHit::Tile(2)));
        assert_eq!(m.tile(), 2);
        assert!(!m.hover(MenuHit::Tile(2)));
        assert_eq!(m.click(MenuHit::Tile(0)), MenuAction::Redraw); // opens the panel
        assert_eq!(m.focus(), Some(Focus::Tabs));
        assert_eq!(m.click(MenuHit::Row(1)), MenuAction::Redraw);
        assert_eq!(m.focus(), Some(Focus::Row(1)));
        assert_eq!(m.click(MenuHit::Row(2)), MenuAction::Redraw);
        assert_eq!(m.scroll(MenuHit::Tile(1), Dir::Forward), MenuAction::None);
        assert_eq!(m.click(MenuHit::Panel), MenuAction::None);
        assert_eq!(m.click(MenuHit::Tile(0)), MenuAction::None); // settings already open
        assert!(m.hover(MenuHit::Tile(1)));
        assert_eq!(m.focus(), None); // another tile closes the panel
        assert_eq!(m.click(MenuHit::Outside), MenuAction::Close);
    }
}
