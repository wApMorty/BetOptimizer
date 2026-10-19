//! Les écrans de l'application. Chacun reçoit les données (`&Data`) et l'horloge de mouvement, dessine, et renvoie des
//! [`Request`]s : il ne touche jamais la base ni la fenêtre, c'est `App` qui exécute.

pub mod bets;
pub mod common;
pub mod opportunities;
pub mod performance;
pub mod reports;
pub mod today;

use eframe::egui::Ui;

use crate::actions::Action;
use crate::data::Data;
use crate::design::icons::Icon;
use crate::design::motion::Motion;

/// Écran affiché.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Le verdict du jour.
    Today,
    /// Les prix qui dépassent la cote juste, et le journal.
    Opportunities,
    /// Mes paris.
    Bets,
    /// Bankroll, CLV, calibration.
    Performance,
    /// Rapports et données.
    Reports,
}

impl Tab {
    /// Tous les écrans, dans l'ordre de la barre latérale.
    pub const ALL: [Tab; 5] = [
        Tab::Today,
        Tab::Opportunities,
        Tab::Bets,
        Tab::Performance,
        Tab::Reports,
    ];

    /// Titre dans la barre latérale.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Today => "Aujourd'hui",
            Tab::Opportunities => "Opportunités",
            Tab::Bets => "Paris",
            Tab::Performance => "Performance",
            Tab::Reports => "Rapports",
        }
    }

    /// Icône dans la barre latérale.
    pub fn icon(self) -> Icon {
        match self {
            Tab::Today => Icon::Today,
            Tab::Opportunities => Icon::Bolt,
            Tab::Bets => Icon::Ticket,
            Tab::Performance => Icon::Chart,
            Tab::Reports => Icon::Doc,
        }
    }
}

/// Ce qu'un écran demande à l'application.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Une action sur le journal (ou l'ouverture d'un rapport).
    Do(Action),
    /// Aller à un autre écran.
    Go(Tab),
    /// Ouvrir le tiroir « J'ai parié » pour la recommandation donnée.
    Follow(i64),
    /// Ouvrir le tiroir d'un pari hors modèle.
    NewOffModelBet,
}

/// Contexte d'affichage d'un écran.
#[derive(Clone, Copy)]
pub struct PageCtx {
    /// Horloge de mouvement.
    pub motion: Motion,
}

/// État local des écrans.
#[derive(Default)]
pub struct PageStates {
    /// Écran d'accueil.
    pub today: today::State,
    /// Écran des opportunités.
    pub opportunities: opportunities::State,
    /// Écran de performance.
    pub performance: performance::State,
}

/// Dessine l'écran choisi.
pub fn show(
    tab: Tab,
    ui: &mut Ui,
    ctx: &PageCtx,
    data: &Data,
    states: &mut PageStates,
) -> Vec<Request> {
    match tab {
        Tab::Today => today::show(ui, ctx, data, &mut states.today),
        Tab::Opportunities => opportunities::show(ui, ctx, data, &mut states.opportunities),
        Tab::Bets => bets::show(ui, ctx, data),
        Tab::Performance => performance::show(ui, ctx, data, &mut states.performance),
        Tab::Reports => reports::show(ui, ctx, data),
    }
}
