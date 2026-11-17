pub mod app;
pub mod concept_a;
pub mod concept_b;
pub mod concept_c;
pub mod data;
pub mod kit;
pub mod store;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Concept {
    A,
    B,
    C,
}

impl Concept {
    pub const ALL: [Concept; 3] = [Concept::A, Concept::B, Concept::C];

    pub fn slug(self) -> &'static str {
        match self {
            Concept::A => "a-three-pane",
            Concept::B => "b-conversations",
            Concept::C => "c-triage",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Concept::A => "a three pane",
            Concept::B => "b conversations",
            Concept::C => "c triage",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Inbox,
    Reader,
    Composer,
    Empty,
    Error,
}

impl Screen {
    pub const ALL: [Screen; 5] =
        [Screen::Inbox, Screen::Reader, Screen::Composer, Screen::Empty, Screen::Error];

    pub fn slug(self) -> &'static str {
        match self {
            Screen::Inbox => "inbox",
            Screen::Reader => "reader",
            Screen::Composer => "composer",
            Screen::Empty => "empty",
            Screen::Error => "error",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Screen::Inbox => "inbox",
            Screen::Reader => "reader",
            Screen::Composer => "composer",
            Screen::Empty => "empty",
            Screen::Error => "error",
        }
    }
}
