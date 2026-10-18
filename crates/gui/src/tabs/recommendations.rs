//! Onglet « Recommandations » : les prix actuels qui dépassent la cote juste de Pinnacle, et le journal.

use edge_core::betting::live::Opportunity;
use edge_core::format;
use edge_core::store::journal::{StoredRecommendation, Tier};
use eframe::egui::{self, Grid, RichText, ScrollArea, Ui};

use crate::actions::Action;
use crate::data::Data;
use crate::forms::FollowForm;
use crate::theme;

/// Nombre de lignes du journal affichées.
const JOURNAL_ROWS: usize = 60;

/// État local de l'onglet.
#[derive(Default)]
pub struct State {
    /// Afficher aussi les prix sous le seuil (journal fantôme).
    pub show_below: bool,
    /// Formulaire « J'ai parié » ouvert.
    pub follow: Option<FollowForm>,
    /// Dernière erreur de saisie du formulaire.
    pub follow_error: Option<String>,
}

/// Dessine l'onglet.
pub fn show(ui: &mut Ui, data: &Data, state: &mut State) -> Vec<Action> {
    let mut actions = Vec::new();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        theme::experimental_banner(ui);
        ui.add_space(6.0);
        ui.heading("Prix actuels");
        ui.label(format!(
            "{} match(s) à venir dans un relevé récent · {} sans cotes complètes de Pinnacle (ignorés) · {} prix aberrant(s) écarté(s) · mises sur {} € (Kelly ×{})",
            data.scan.events,
            data.scan.without_reference,
            data.scan.implausible_quotes,
            format::decimal(data.curve.available, 0),
            format::decimal(data.config.kelly_fraction, 2)
        ));
        if data.scan.events == 0 {
            ui.label("Aucun relevé de moins de 30 h : `edge snapshot` (ou le timer de 14 h) en crée un.");
        } else if data.notified.is_empty() {
            ui.label("Aucun prix n'atteint le seuil de notification (3 %).");
        } else {
            opportunities(ui, "notified", &data.notified, true);
        }
        ui.checkbox(
            &mut state.show_below,
            format!("Afficher aussi les {} prix sous le seuil (journal fantôme, sans mise)", data.below.len()),
        );
        if state.show_below && !data.below.is_empty() {
            opportunities(ui, "below", &data.below, false);
        }

        ui.add_space(10.0);
        ui.heading("Journal des recommandations");
        if data.recommendations.is_empty() {
            ui.label("Aucune recommandation enregistrée : `edge recommend` (ou le timer de 14 h 10) en écrit.");
        } else {
            journal(ui, data, state, &mut actions);
        }
    });
    follow_window(ui, state, &mut actions);
    actions
}

fn opportunities(ui: &mut Ui, id: &str, list: &[Opportunity], with_stake: bool) {
    ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
        Grid::new(id).striped(true).num_columns(9).show(ui, |ui| {
            for title in [
                "Coup d'envoi",
                "Match",
                "Bookmaker",
                "Issue",
                "Cote",
                "Cote juste",
                "Avantage",
                "Cote min",
                "Mise",
            ] {
                ui.label(RichText::new(title).strong());
            }
            ui.end_row();
            for found in list {
                let s = &found.selection;
                ui.label(format::kickoff_label(found.kickoff));
                ui.label(format!("{} – {}", found.home_team, found.away_team));
                ui.label(&found.bookmaker);
                ui.label(format::outcome_label(s.outcome));
                ui.label(format::decimal(s.odds.get(), 2));
                ui.label(format::decimal(1.0 / s.probability.get(), 2));
                ui.label(theme::signed_percent(ui, s.edge, 1));
                ui.label(format::decimal(found.minimal_odds, 2));
                ui.label(match (with_stake, found.stake) {
                    (true, Some(stake)) => format!("{} €", format::decimal(stake.get(), 1)),
                    (true, None) => "sous le minimum".to_string(),
                    (false, _) => "—".to_string(),
                });
                ui.end_row();
            }
        });
    });
}

fn journal(ui: &mut Ui, data: &Data, state: &mut State, actions: &mut Vec<Action>) {
    ScrollArea::horizontal().id_salt("journal").show(ui, |ui| {
        Grid::new("journal")
            .striped(true)
            .num_columns(11)
            .show(ui, |ui| {
                for title in [
                    "N°",
                    "Palier",
                    "Coup d'envoi",
                    "Match",
                    "Bookmaker",
                    "Issue",
                    "Cote",
                    "Avantage",
                    "Cote min",
                    "Mise",
                    "",
                ] {
                    ui.label(RichText::new(title).strong());
                }
                ui.end_row();
                for found in data.recommendations.iter().take(JOURNAL_ROWS) {
                    let r = &found.record;
                    ui.label(found.id.to_string());
                    ui.label(match r.tier {
                        Tier::Recommended => "notifiée",
                        Tier::Shadow => "fantôme",
                    });
                    ui.label(format::kickoff_label(r.kickoff));
                    ui.label(format!("{} – {}", r.home_team, r.away_team));
                    ui.label(&r.bookmaker);
                    ui.label(format::outcome_label(r.outcome));
                    ui.label(format::decimal(r.odds.get(), 2));
                    ui.label(theme::signed_percent(ui, r.edge, 1));
                    ui.label(format::decimal(r.minimal_odds, 2));
                    ui.label(r.stake.map_or("—".to_string(), |s| {
                        format!("{} €", format::decimal(s.get(), 1))
                    }));
                    follow_button(ui, data, found, state, actions);
                    ui.end_row();
                }
            });
    });
}

/// Le bouton « J'ai parié » n'existe que pour une recommandation notifiée, à venir, sans pari déjà enregistré.
fn follow_button(
    ui: &mut Ui,
    data: &Data,
    found: &StoredRecommendation,
    state: &mut State,
    _actions: &mut Vec<Action>,
) {
    let followed = data
        .bets
        .iter()
        .any(|b| b.bet.recommendation_id == Some(found.id));
    if followed {
        ui.label("pari enregistré");
    } else if found.record.tier == Tier::Recommended && found.record.kickoff > data.now {
        if ui.button("J'ai parié").clicked() {
            state.follow = Some(FollowForm::for_recommendation(found));
            state.follow_error = None;
        }
    } else {
        ui.label("");
    }
}

/// La fenêtre du formulaire « J'ai parié ».
fn follow_window(ui: &mut Ui, state: &mut State, actions: &mut Vec<Action>) {
    let Some(form) = state.follow.as_mut() else {
        return;
    };
    let mut open = true;
    let mut submit = false;
    let mut cancel = false;
    egui::Window::new(format!("J'ai parié — recommandation n° {}", form.recommendation_id))
        .open(&mut open)
        .collapsible(false)
        .show(ui.ctx(), |ui| {
            ui.label("Saisis ce que tu as réellement obtenu ; une cote sous la cote minimale est refusée (pari hors modèle).");
            Grid::new("follow_form").num_columns(2).show(ui, |ui| {
                ui.label("Cote obtenue");
                ui.text_edit_singleline(&mut form.odds);
                ui.end_row();
                ui.label("Mise (€)");
                ui.text_edit_singleline(&mut form.stake);
                ui.end_row();
                ui.label("Bookmaker");
                ui.text_edit_singleline(&mut form.bookmaker);
                ui.end_row();
            });
            if let Some(message) = &state.follow_error {
                ui.label(RichText::new(message).color(theme::negative(ui)));
            }
            ui.horizontal(|ui| {
                submit = ui.button("Enregistrer").clicked();
                cancel = ui.button("Annuler").clicked();
            });
        });
    if submit {
        match form.to_action() {
            Ok(action) => {
                actions.push(action);
                state.follow = None;
                state.follow_error = None;
            }
            Err(message) => state.follow_error = Some(message),
        }
    } else if cancel || !open {
        state.follow = None;
        state.follow_error = None;
    }
}
