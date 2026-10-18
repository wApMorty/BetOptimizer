//! Les onglets de l'application. Chacun reçoit les données (`&Data`) et son état local, dessine, et renvoie les
//! [`Action`]s demandées par l'utilisateur : il ne touche jamais la base.

pub mod bankroll;
pub mod bets;
pub mod clv;
pub mod recommendations;
pub mod reports;

use crate::actions::Action;
use crate::data::Data;
use eframe::egui::Ui;

/// Onglet affiché.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Prix actuels et journal des recommandations.
    Recommendations,
    /// Paris réels.
    Bets,
    /// Courbe de bankroll.
    Bankroll,
    /// CLV réel et calibration.
    Clv,
    /// Rapports HTML.
    Reports,
}

impl Tab {
    /// Tous les onglets, dans l'ordre d'affichage.
    pub const ALL: [Tab; 5] = [
        Tab::Recommendations,
        Tab::Bets,
        Tab::Bankroll,
        Tab::Clv,
        Tab::Reports,
    ];

    /// Titre de l'onglet.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Recommendations => "Recommandations",
            Tab::Bets => "Paris",
            Tab::Bankroll => "Bankroll",
            Tab::Clv => "CLV et calibration",
            Tab::Reports => "Rapports",
        }
    }
}

/// État local de chaque onglet (formulaires ouverts, filtres).
#[derive(Default)]
pub struct TabStates {
    /// Onglet des recommandations.
    pub recommendations: recommendations::State,
    /// Onglet des paris.
    pub bets: bets::State,
}

/// Dessine l'onglet choisi et renvoie les actions demandées.
pub fn show(tab: Tab, ui: &mut Ui, data: &Data, states: &mut TabStates) -> Vec<Action> {
    match tab {
        Tab::Recommendations => recommendations::show(ui, data, &mut states.recommendations),
        Tab::Bets => bets::show(ui, data, &mut states.bets),
        Tab::Bankroll => bankroll::show(ui, data),
        Tab::Clv => clv::show(ui, data),
        Tab::Reports => reports::show(ui, data),
    }
}
