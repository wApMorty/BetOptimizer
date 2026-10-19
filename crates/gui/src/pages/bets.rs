//! Écran « Paris » : tes paris réels, leur état, et le règlement.

use edge_core::format;
use edge_core::store::journal::{Settlement, StoredBet};
use eframe::egui::{RichText, Sense, Ui, vec2};

use super::common::{euros, fixture, league_name, page_header, signed_euros, sport_icon, when};
use super::{PageCtx, Request};
use crate::actions::Action;
use crate::data::Data;
use crate::design::icons;
use crate::design::motion::reveal;
use crate::design::style::Semibold;
use crate::design::tokens::space;
use crate::design::widgets::{ButtonKind, Tone, button, card, chip, empty_state, pal, split};

/// Dessine l'écran.
pub fn show(ui: &mut Ui, ctx: &PageCtx, data: &Data) -> Vec<Request> {
    let mut requests = Vec::new();
    let motion = &ctx.motion;
    let open = data.bets.iter().filter(|b| b.result.is_none()).count();
    let subtitle = format!(
        "{} pari(s) · {} ouvert(s) · bankroll disponible {}",
        data.bets.len(),
        open,
        euros(data.curve.available, 0)
    );
    page_header(ui, motion, "Paris", &subtitle);
    ui.add_space(space::SM);

    reveal(ui, motion, 1, |ui| {
        ui.horizontal(|ui| {
            if button(ui, "Pari hors modèle", ButtonKind::Primary).clicked() {
                requests.push(Request::NewOffModelBet);
            }
            if button(ui, "Régler depuis l'historique", ButtonKind::Secondary)
                .on_hover_text("Lit les résultats de football-data déjà importés (`edge fd fetch` puis `edge fd import`).")
                .clicked()
            {
                requests.push(Request::Do(Action::SettleAuto));
            }
        });
    });
    ui.add_space(space::SM);

    if data.bets.is_empty() {
        reveal(ui, motion, 2, |ui| {
            card(ui, "bets-empty", false, |ui| {
                empty_state(
                    ui,
                    icons::Icon::Ticket,
                    "Aucun pari enregistré",
                    "« J'ai parié » sur une opportunité, ou « Pari hors modèle », en crée un.",
                );
            });
        });
    }
    for (index, stored) in data.bets.iter().enumerate() {
        reveal(ui, motion, 2 + index, |ui| {
            if let Some(request) = bet_card(ui, data, stored) {
                requests.push(request);
            }
        });
        ui.add_space(space::XS);
    }
    requests
}

/// Couleur et texte de l'état d'un pari.
fn state(stored: &StoredBet) -> (Tone, &'static str) {
    match &stored.result {
        None => (Tone::Accent, "ouvert"),
        Some(r) => match r.settlement {
            Settlement::Won => (Tone::Positive, "gagné"),
            Settlement::Lost => (Tone::Negative, "perdu"),
            Settlement::Void => (Tone::Neutral, "remboursé"),
        },
    }
}

fn bet_card(ui: &mut Ui, data: &Data, stored: &StoredBet) -> Option<Request> {
    let p = pal(ui);
    let b = &stored.bet;
    let (tone, label) = state(stored);
    let (stripe, _) = tone.colors(p);
    let mut request = None;
    card(ui, ("bet", stored.id), true, |ui| {
        let settled = stored
            .result
            .as_ref()
            .is_some_and(|r| r.settlement != Settlement::Void);
        // Pari ouvert : trois boutons de règlement (assez larges pour passer sous le texte en fenêtre étroite).
        let right_width = if stored.result.is_none() {
            290.0
        } else {
            120.0
        };
        split(
            ui,
            right_width,
            |ui| {
                ui.horizontal(|ui| {
                    let (bar, _) = ui.allocate_exact_size(vec2(4.0, 54.0), Sense::hover());
                    ui.painter().rect_filled(bar, 2.0, stripe);
                    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    icons::draw(ui.painter(), rect, sport_icon(&b.sport_key), p.muted);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(fixture(&b.home_team, &b.away_team))
                                .semibold()
                                .size(16.0),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                league_name(&b.sport_key),
                                when(b.kickoff, data.now)
                            ))
                            .small()
                            .color(p.muted),
                        );
                        ui.horizontal_wrapped(|ui| {
                            chip(ui, &b.bookmaker, Tone::Neutral);
                            chip(
                                ui,
                                &format!(
                                    "{} à {}",
                                    format::outcome_label(b.outcome),
                                    format::decimal(b.odds.get(), 2)
                                ),
                                Tone::Accent,
                            );
                            chip(
                                ui,
                                &format!("mise {}", euros(b.stake.get(), 1)),
                                Tone::Neutral,
                            );
                            match b.recommendation_id {
                                Some(id) => {
                                    chip(ui, &format!("recommandation n° {id}"), Tone::Positive)
                                }
                                None => chip(ui, "hors modèle", Tone::Warning),
                            };
                        });
                    });
                });
            },
            |ui| match &stored.result {
                Some(r) if settled => {
                    ui.label(
                        RichText::new(signed_euros(r.profit))
                            .semibold()
                            .size(20.0)
                            .color(tone.colors(p).0),
                    );
                    ui.label(RichText::new(label).small().color(p.muted));
                }
                Some(_) => {
                    chip(ui, label, tone);
                }
                None => {
                    ui.label(RichText::new("Régler à la main").small().color(p.muted));
                    ui.horizontal(|ui| {
                        // Dans un bloc aligné à droite, `horizontal` range les boutons de droite à gauche : on inverse pour garder l'ordre.
                        let mut choices = [
                            ("Gagné", Settlement::Won),
                            ("Perdu", Settlement::Lost),
                            ("Remboursé", Settlement::Void),
                        ];
                        if ui.layout().prefer_right_to_left() {
                            choices.reverse();
                        }
                        for (text, settlement) in choices {
                            if button(ui, text, ButtonKind::Secondary).clicked() {
                                request = Some(Request::Do(Action::SettleManual {
                                    bet_id: stored.id,
                                    settlement,
                                }));
                            }
                        }
                    });
                }
            },
        );
    });
    request
}
