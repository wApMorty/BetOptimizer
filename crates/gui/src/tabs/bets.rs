//! Onglet « Paris » : tes paris réels, leur règlement, et le formulaire des paris hors modèle.

use edge_core::domain::Outcome;
use edge_core::format;
use edge_core::store::journal::Settlement;
use eframe::egui::{ComboBox, Grid, RichText, ScrollArea, Ui};

use crate::actions::Action;
use crate::data::Data;
use crate::forms::OffModelForm;
use crate::theme;

/// État local de l'onglet.
#[derive(Default)]
pub struct State {
    /// Formulaire du pari hors modèle.
    pub form: OffModelForm,
    /// Dernière erreur de saisie.
    pub error: Option<String>,
}

/// Dessine l'onglet.
pub fn show(ui: &mut Ui, data: &Data, state: &mut State) -> Vec<Action> {
    let mut actions = Vec::new();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Mes paris");
            if ui
                .button("Régler depuis l'historique")
                .on_hover_text("Lit les résultats de football-data déjà importés (`edge fd fetch` puis `edge fd import`).")
                .clicked()
            {
                actions.push(Action::SettleAuto);
            }
        });
        ui.label(format!(
            "Bankroll disponible : {} € (départ {} € · gains réglés {} · mises ouvertes {} €)",
            format::decimal(data.curve.available, 2),
            format::decimal(data.curve.initial, 0),
            format::signed(data.curve.settled_profit, 2),
            format::decimal(data.curve.open_stakes, 2),
        ));
        if data.bets.is_empty() {
            ui.label("Aucun pari enregistré. « J'ai parié » (onglet Recommandations) ou le formulaire ci-dessous en crée un.");
        } else {
            table(ui, data, &mut actions);
        }
        ui.add_space(10.0);
        off_model_form(ui, data, state, &mut actions);
    });
    actions
}

fn table(ui: &mut Ui, data: &Data, actions: &mut Vec<Action>) {
    ScrollArea::horizontal().id_salt("bets").show(ui, |ui| {
        Grid::new("bets")
            .striped(true)
            .num_columns(10)
            .show(ui, |ui| {
                for title in [
                    "N°",
                    "Coup d'envoi",
                    "Match",
                    "Bookmaker",
                    "Issue",
                    "Cote",
                    "Mise",
                    "État",
                    "Origine",
                    "Régler",
                ] {
                    ui.label(RichText::new(title).strong());
                }
                ui.end_row();
                for stored in &data.bets {
                    let b = &stored.bet;
                    ui.label(stored.id.to_string());
                    ui.label(format::kickoff_label(b.kickoff));
                    ui.label(format!("{} – {}", b.home_team, b.away_team));
                    ui.label(&b.bookmaker);
                    ui.label(format::outcome_label(b.outcome));
                    ui.label(format::decimal(b.odds.get(), 2));
                    ui.label(format!("{} €", format::decimal(b.stake.get(), 1)));
                    match &stored.result {
                        None => {
                            ui.label("ouvert");
                        }
                        Some(r) => match r.settlement {
                            Settlement::Won => {
                                ui.label(theme::signed_euros(ui, r.profit));
                            }
                            Settlement::Lost => {
                                ui.label(theme::signed_euros(ui, r.profit));
                            }
                            Settlement::Void => {
                                ui.label("remboursé");
                            }
                        },
                    }
                    ui.label(b.recommendation_id.map_or("hors modèle".to_string(), |id| {
                        format!("recommandation n° {id}")
                    }));
                    if stored.result.is_none() {
                        ui.horizontal(|ui| {
                            for (label, settlement) in [
                                ("Gagné", Settlement::Won),
                                ("Perdu", Settlement::Lost),
                                ("Remboursé", Settlement::Void),
                            ] {
                                if ui.small_button(label).clicked() {
                                    actions.push(Action::SettleManual {
                                        bet_id: stored.id,
                                        settlement,
                                    });
                                }
                            }
                        });
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
    });
}

fn off_model_form(ui: &mut Ui, data: &Data, state: &mut State, actions: &mut Vec<Action>) {
    ui.heading("Pari hors modèle");
    ui.label("Un pari qui ne suit aucune recommandation. Il est rapporté à part, et exclu des métriques du modèle.");
    let selected = state
        .form
        .event_id
        .as_ref()
        .and_then(|id| data.upcoming.iter().find(|e| &e.event_id == id))
        .map_or("choisir un match…".to_string(), |e| {
            format!(
                "{} · {} – {}",
                format::kickoff_label(e.kickoff),
                e.home_team,
                e.away_team
            )
        });
    Grid::new("off_model").num_columns(2).show(ui, |ui| {
        ui.label("Match");
        ComboBox::from_id_salt("off_model_event")
            .width(360.0)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for event in &data.upcoming {
                    let label = format!(
                        "{} · {} – {}",
                        format::kickoff_label(event.kickoff),
                        event.home_team,
                        event.away_team
                    );
                    ui.selectable_value(
                        &mut state.form.event_id,
                        Some(event.event_id.clone()),
                        label,
                    );
                }
            });
        ui.end_row();

        ui.label("Issue");
        ui.horizontal(|ui| {
            for outcome in Outcome::ALL {
                ui.selectable_value(
                    &mut state.form.outcome,
                    Some(outcome),
                    format::outcome_label(outcome),
                );
            }
        });
        ui.end_row();

        ui.label("Bookmaker");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut state.form.bookmaker);
            ComboBox::from_id_salt("off_model_bookmaker")
                .selected_text("liste")
                .show_ui(ui, |ui| {
                    for name in &data.config.bookmakers {
                        if ui.selectable_label(false, name).clicked() {
                            state.form.bookmaker = name.clone();
                        }
                    }
                });
        });
        ui.end_row();

        ui.label("Cote obtenue");
        ui.text_edit_singleline(&mut state.form.odds);
        ui.end_row();
        ui.label("Mise (€)");
        ui.text_edit_singleline(&mut state.form.stake);
        ui.end_row();
    });
    if let Some(message) = &state.error {
        ui.label(RichText::new(message).color(theme::negative(ui)));
    }
    if ui.button("Enregistrer le pari hors modèle").clicked() {
        match state.form.to_action() {
            Ok(action) => {
                actions.push(action);
                state.form = OffModelForm::default();
                state.error = None;
            }
            Err(message) => state.error = Some(message),
        }
    }
}
