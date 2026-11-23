#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Accounts,
    Security,
    MailHandling,
    Updates,
    About,
}

impl Section {
    pub const ALL: [Section; 6] =
        [Section::General, Section::Accounts, Section::Security, Section::MailHandling, Section::Updates, Section::About];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Accounts => "accounts",
            Section::Security => "security",
            Section::MailHandling => "mail handling",
            Section::Updates => "updates",
            Section::About => "about",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Servers,
    Sync,
    Notifications,
    Danger,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Overview, Tab::Servers, Tab::Sync, Tab::Notifications, Tab::Danger];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "overview",
            Tab::Servers => "servers",
            Tab::Sync => "sync and storage",
            Tab::Notifications => "notifications",
            Tab::Danger => "danger zone",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Sections,
    Page,
    Account,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Enter,
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Leave,
}

#[derive(Clone, Debug)]
pub struct PageState {
    pub section: Section,
    pub level: Level,
    pub account: usize,
    pub tab: Tab,
    pub narrow: bool,
    pub accounts: usize,
}

impl PageState {
    pub fn new() -> Self {
        PageState { section: Section::General, level: Level::Sections, account: 0, tab: Tab::Overview, narrow: false, accounts: 0 }
    }

    pub fn showing_sections(&self) -> bool {
        !self.narrow || self.level == Level::Sections
    }

    pub fn showing_account_list(&self) -> bool {
        self.section == Section::Accounts && (!self.narrow || self.level == Level::Page)
    }

    pub fn showing_account(&self) -> bool {
        self.section == Section::Accounts && self.accounts > 0 && (!self.narrow || self.level == Level::Account)
    }

    pub fn open_section(&mut self, section: Section) {
        self.section = section;
        self.level = Level::Page;
    }

    pub fn select_account(&mut self, index: usize) {
        self.account = index.min(self.accounts.saturating_sub(1));
        self.level = Level::Account;
        self.tab = Tab::Overview;
    }

    pub fn press(&mut self, nav: Nav) -> Outcome {
        match nav {
            Nav::Back => self.back(),
            Nav::Enter => {
                self.enter();
                Outcome::Stay
            }
            Nav::Up => {
                self.step(-1);
                Outcome::Stay
            }
            Nav::Down => {
                self.step(1);
                Outcome::Stay
            }
        }
    }

    fn back(&mut self) -> Outcome {
        match self.level {
            Level::Account => {
                self.level = Level::Page;
                Outcome::Stay
            }
            Level::Page if self.narrow => {
                self.level = Level::Sections;
                Outcome::Stay
            }
            Level::Page => Outcome::Leave,
            Level::Sections => Outcome::Leave,
        }
    }

    fn enter(&mut self) {
        match self.level {
            Level::Sections => self.level = Level::Page,
            Level::Page if self.section == Section::Accounts && self.accounts > 0 => self.select_account(self.account),
            _ => {}
        }
    }

    fn step(&mut self, delta: i32) {
        let bump = |i: usize, len: usize| (i as i32 + delta).clamp(0, len.max(1) as i32 - 1) as usize;
        match self.level {
            Level::Sections => {
                let at = Section::ALL.iter().position(|s| *s == self.section).unwrap_or(0);
                self.section = Section::ALL[bump(at, Section::ALL.len())];
            }
            Level::Page if self.section == Section::Accounts => self.account = bump(self.account, self.accounts),
            _ => {}
        }
    }
}

impl Default for PageState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn move_item<T>(items: &mut [T], from: usize, delta: i32) -> Option<usize> {
    let to = from as i32 + delta;
    if to < 0 || to as usize >= items.len() || from >= items.len() {
        return None;
    }
    items.swap(from, to as usize);
    Some(to as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide() -> PageState {
        PageState { accounts: 3, ..PageState::new() }
    }

    fn phone() -> PageState {
        PageState { narrow: true, accounts: 3, ..PageState::new() }
    }

    #[test]
    fn down_and_up_walk_the_sections_in_order_and_stop_at_the_ends() {
        let mut s = wide();
        s.press(Nav::Up);
        assert_eq!(s.section, Section::General);
        for want in [Section::Accounts, Section::Security, Section::MailHandling, Section::Updates, Section::About, Section::About] {
            s.press(Nav::Down);
            assert_eq!(s.section, want);
        }
    }

    #[test]
    fn enter_opens_the_section_and_escape_goes_back_one_level_on_a_phone() {
        let mut s = phone();
        s.press(Nav::Down);
        s.press(Nav::Enter);
        assert_eq!(s.level, Level::Page);
        s.press(Nav::Enter);
        assert_eq!(s.level, Level::Account);
        assert_eq!(s.press(Nav::Back), Outcome::Stay);
        assert_eq!(s.level, Level::Page);
        assert_eq!(s.press(Nav::Back), Outcome::Stay);
        assert_eq!(s.level, Level::Sections);
        assert_eq!(s.press(Nav::Back), Outcome::Leave);
    }

    #[test]
    fn escape_on_desktop_steps_from_account_to_list_then_leaves() {
        let mut s = wide();
        s.open_section(Section::Accounts);
        s.select_account(1);
        assert_eq!(s.press(Nav::Back), Outcome::Stay);
        assert_eq!(s.level, Level::Page);
        assert_eq!(s.press(Nav::Back), Outcome::Leave);
    }

    #[test]
    fn up_and_down_move_through_accounts_on_the_list() {
        let mut s = wide();
        s.open_section(Section::Accounts);
        s.press(Nav::Down);
        s.press(Nav::Down);
        s.press(Nav::Down);
        assert_eq!(s.account, 2);
        s.press(Nav::Up);
        assert_eq!(s.account, 1);
    }

    #[test]
    fn moving_an_item_swaps_it_and_refuses_past_the_ends() {
        let mut v = vec!["a", "b", "c"];
        assert_eq!(move_item(&mut v, 2, -1), Some(1));
        assert_eq!(v, ["a", "c", "b"]);
        assert_eq!(move_item(&mut v, 0, -1), None);
        assert_eq!(move_item(&mut v, 2, 1), None);
    }
}
