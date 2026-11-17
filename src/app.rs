use crate::kit::Draft;
use crate::{Concept, Screen};
use egui_bench::prelude::*;

pub struct State {
    pub concept: Concept,
    pub screen: Screen,
    pub draft: Draft,
    pub filter: Filter,
    pub selected: usize,
    pub reply: String,
    pub command: String,
    pub search: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Unread,
    Flagged,
}

impl State {
    pub fn new(concept: Concept, screen: Screen) -> State {
        State {
            concept,
            screen,
            draft: Draft::sample(),
            filter: Filter::All,
            selected: 0,
            reply: "Thanks, leave the panel edge unpopulated. Paying today.".into(),
            command: String::new(),
            search: String::new(),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        match self.concept {
            Concept::A => crate::concept_a::show(ui, self),
            Concept::B => crate::concept_b::show(ui, self),
            Concept::C => crate::concept_c::show(ui, self),
        }
    }
}

pub struct App {
    pub state: State,
}

impl App {
    pub fn new(ctx: &egui::Context) -> App {
        crate::kit::setup(ctx);
        App { state: State::new(Concept::A, Screen::Inbox) }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("review")
            .frame(egui::Frame::NONE.fill(WELL).inner_margin(egui::Margin::symmetric(12, 4)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(legend("concept"));
                    let c = &mut self.state.concept;
                    let opts: Vec<(Concept, &str)> = Concept::ALL.iter().map(|c| (*c, c.title())).collect();
                    tabs(ui, c, &opts);
                    ui.add_space(20.0);
                    ui.label(legend("screen"));
                    let s = &mut self.state.screen;
                    let opts: Vec<(Screen, &str)> = Screen::ALL.iter().map(|s| (*s, s.title())).collect();
                    tabs(ui, s, &opts);
                });
            });
        egui::CentralPanel::no_frame().show(ui, |ui| self.state.show(ui));
    }
}
