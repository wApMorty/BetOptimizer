//! Écran « Bonus » : offres de bienvenue à jouer, opérations couvertes proposées sur le dernier relevé, calculateur, suivi.
//!
//! Tout vient de `edge_core::bonus::tracking::board` (le même tableau que `edge bonus status` et la page `/bonus` du site).

use edge_core::bonus::OfferKind;
use edge_core::bonus::operator_name;
use edge_core::bonus::pick::Candidate;
use edge_core::bonus::tracking::{self, OfferState, OperatorStatus, Plan};
use edge_core::format;
use edge_core::store::bonus::{Eligibility, MatchResult, Role, StoredOperation};
use eframe::egui::{RichText, Ui};

use super::common::{euros, fixture, league_name, page_header, signed_euros, when};
use super::{PageCtx, Request};
use crate::actions::Action;
use crate::data::Data;
use crate::design::icons::Icon;
use crate::design::motion::reveal;
use crate::design::style::Semibold;
use crate::design::tokens::space;
use crate::design::widgets::{
    ButtonKind, Tone, button, card, chip, empty_state, field, pal, section_title, segmented, split,
    stat_tile, tile_columns,
};
use crate::forms::{BonusForm, CalcForm};

/// Opérations proposées par offre à l'écran (la CLI `edge bonus pick` en montre davantage).
const SHOWN_PER_PLAN: usize = 3;

/// Libellés courts des formes d'offre, dans l'ordre de [`OfferKind::ALL`], pour les contrôles segmentés : les libellés
/// complets ne tiennent pas côte à côte dans une fenêtre tuilée.
pub const KIND_LABELS: [&str; 3] = ["Remb. argent", "Remb. freebet", "Freebet"];

/// État local de l'écran : le calculateur.
#[derive(Default)]
pub struct State {
    /// Valeurs tapées dans le calculateur.
    pub calc: CalcForm,
}

/// Dessine l'écran.
pub fn show(ui: &mut Ui, ctx: &PageCtx, data: &Data, state: &mut State) -> Vec<Request> {
    let mut requests = Vec::new();
    let motion = &ctx.motion;
    let board = &data.bonus;
    let covers: Vec<&str> = board
        .cover_accounts
        .iter()
        .map(|k| operator_name(k))
        .collect();
    page_header(
        ui,
        motion,
        "Bonus",
        "Bonus de bienvenue couverts : le gain est le même quel que soit le résultat du match.",
    );
    ui.add_space(space::SM);

    reveal(ui, motion, 1, |ui| {
        let n = tile_columns(ui);
        let to_play = board.plans.len();
        ui.columns(n, |cols| {
            stat_tile(
                &mut cols[0],
                "bonus-realized",
                "Réalisé",
                signed_euros(board.realized),
                Tone::of_sign(board.realized),
                |ui| {
                    ui.label(
                        RichText::new("opérations réglées, en argent")
                            .small()
                            .color(pal(ui).muted),
                    );
                },
            );
            stat_tile(
                &mut cols[1 % n],
                "bonus-open",
                "Garanti en cours",
                signed_euros(board.open_guaranteed),
                Tone::Neutral,
                |ui| {
                    ui.label(
                        RichText::new("opérations pas encore réglées")
                            .small()
                            .color(pal(ui).muted),
                    );
                },
            );
            stat_tile(
                &mut cols[2 % n],
                "bonus-left",
                "À jouer",
                to_play.to_string(),
                Tone::Accent,
                |ui| {
                    let text = if covers.is_empty() {
                        "aucun compte de couverture".to_string()
                    } else {
                        format!("couverture : {}", covers.join(", "))
                    };
                    ui.label(RichText::new(text).small().color(pal(ui).muted));
                },
            );
        });
    });

    ui.add_space(space::LG);
    reveal(ui, motion, 2, |ui| {
        section_title(
            ui,
            "À jouer",
            Some("dernier relevé · recalcule avant de parier"),
        );
    });
    ui.add_space(space::SM);
    if board.plans.is_empty() {
        reveal(ui, motion, 3, |ui| {
            card(ui, "bonus-empty", false, |ui| {
                empty_state(
                    ui,
                    Icon::Gift,
                    "Aucune offre à jouer",
                    "Lis le règlement d'un opérateur, puis « Enregistrer l'offre » dans la liste ci-dessous.",
                );
            });
        });
    }
    for (index, plan) in board.plans.iter().enumerate() {
        reveal(ui, motion, 3 + index, |ui| {
            if let Some(request) = plan_card(ui, data, plan) {
                requests.push(request);
            }
        });
        ui.add_space(space::XS);
    }

    ui.add_space(space::LG);
    section_title(ui, "Calculateur", Some("rien n'est enregistré"));
    ui.add_space(space::SM);
    calculator(ui, &mut state.calc, data.config.bonus_freebet_value);

    ui.add_space(space::LG);
    section_title(ui, "Offres", Some("une offre prise sert ensuite à couvrir"));
    ui.add_space(space::SM);
    card(ui, "bonus-offers", false, |ui| {
        for status in &board.operators {
            if let Some(request) = operator_row(ui, status) {
                requests.push(request);
            }
            ui.add_space(space::XS);
        }
    });

    if !board.operations.is_empty() {
        ui.add_space(space::LG);
        ui.horizontal(|ui| {
            section_title(ui, "Opérations", None);
            if button(ui, "Régler depuis l'historique", ButtonKind::Secondary)
                .on_hover_text("Lit les résultats de football-data déjà importés (`edge fd fetch` puis `edge fd import`).")
                .clicked()
            {
                requests.push(Request::Do(Action::SettleBonusAuto));
            }
        });
        ui.add_space(space::SM);
        // Les plus récentes d'abord.
        for stored in board.operations.iter().rev() {
            if let Some(request) = operation_card(ui, data, stored) {
                requests.push(request);
            }
            ui.add_space(space::XS);
        }
    }
    requests
}

fn plan_card(ui: &mut Ui, data: &Data, plan: &Plan) -> Option<Request> {
    let p = pal(ui);
    let mut request = None;
    card(
        ui,
        (
            "bonus-plan",
            &plan.target.bookmaker,
            plan.target.kind.as_str(),
        ),
        false,
        |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&plan.name).semibold().size(16.0));
                chip(ui, plan.target.kind.label(), Tone::Accent);
                chip(ui, &euros(plan.target.amount.get(), 2), Tone::Neutral);
                if let Some(min) = plan.target.min_odds {
                    chip(
                        ui,
                        &format!("cote ≥ {}", format::decimal(min.get(), 2)),
                        Tone::Neutral,
                    );
                }
            });
            ui.add_space(space::XS);
            if !plan.in_archive {
                ui.label(
                RichText::new("Cotes absentes de l'archive : compare les cotes affichées et utilise le calculateur.")
                    .color(p.muted),
            );
                return;
            }
            if plan.candidates.is_empty() {
                ui.label(
                RichText::new("Aucune opération rentable dans le dernier relevé avec les comptes de couverture actuels.")
                    .color(p.muted),
            );
                return;
            }
            for candidate in plan.candidates.iter().take(SHOWN_PER_PLAN) {
                ui.separator();
                if candidate_row(ui, data, candidate) {
                    request = Some(Request::OpenBonus(BonusForm::from_candidate(
                        &plan.target,
                        candidate,
                    )));
                }
            }
        },
    );
    request
}

/// Une opération proposée ; renvoie `true` si « J'ai joué » est cliqué.
fn candidate_row(ui: &mut Ui, data: &Data, candidate: &Candidate) -> bool {
    let p = pal(ui);
    let mut clicked = false;
    split(
        ui,
        150.0,
        |ui| {
            let event = &candidate.event;
            ui.label(RichText::new(fixture(&event.home_team, &event.away_team)).semibold());
            ui.label(
                RichText::new(format!(
                    "{} · {} · cotes {}",
                    league_name(&event.sport_key),
                    when(event.kickoff, data.now),
                    format::age(candidate.priced_at, data.now)
                ))
                .small()
                .color(p.muted),
            );
            ui.horizontal_wrapped(|ui| {
                for leg in &candidate.legs {
                    let tone = if leg.role == Role::Bonus {
                        Tone::Accent
                    } else {
                        Tone::Neutral
                    };
                    chip(
                        ui,
                        &format!(
                            "{} · {} {} · {}",
                            operator_name(&leg.bookmaker),
                            format::outcome_label(leg.outcome),
                            format::decimal(leg.odds.get(), 2),
                            euros(leg.stake.get(), 2)
                        ),
                        tone,
                    );
                }
            });
        },
        |ui| {
            ui.label(
                RichText::new(signed_euros(candidate.cover.guaranteed))
                    .semibold()
                    .size(18.0)
                    .color(Tone::of_sign(candidate.cover.guaranteed).colors(p).0),
            );
            ui.label(
                RichText::new(format!("sorti {}", euros(candidate.cover.cash_out, 0)))
                    .small()
                    .color(p.muted),
            );
            clicked = button(ui, "J'ai joué", ButtonKind::Primary).clicked();
        },
    );
    clicked
}

fn calculator(ui: &mut Ui, form: &mut CalcForm, freebet_value: f64) {
    let p = pal(ui);
    card(ui, "bonus-calc", false, |ui| {
        segmented(ui, "bonus-calc-kind", &KIND_LABELS, &mut form.kind);
        ui.add_space(space::SM);
        ui.columns(2, |cols| {
            field(
                &mut cols[0],
                "Montant du bonus (€)",
                &mut form.amount,
                "100",
            );
            field(&mut cols[1], "Cote du pari bonus", &mut form.odds, "3,10");
            field(
                &mut cols[0],
                "Cote couverture 1",
                &mut form.covers[0],
                "2,10",
            );
            field(
                &mut cols[1],
                "Cote couverture 2 (vide : deux issues)",
                &mut form.covers[1],
                "3,40",
            );
        });
        if form.odds.trim().is_empty() {
            ui.label(
                RichText::new("Tape la cote du pari bonus et celles des autres issues.")
                    .color(p.muted),
            );
            return;
        }
        match form.compute(freebet_value) {
            Ok(result) => {
                let stakes: Vec<String> = result.stakes.iter().map(|s| euros(s.get(), 2)).collect();
                ui.label(
                    RichText::new(format!("Mises de couverture : {}", stakes.join(" · ")))
                        .semibold(),
                );
                ui.label(
                    RichText::new(format!(
                        "Gain garanti {} · argent sorti {}{}",
                        signed_euros(result.guaranteed),
                        euros(result.cash_out, 2),
                        if form.kind() == OfferKind::FreebetRefund {
                            format!(
                                " · freebet compté {} par euro",
                                format::decimal(freebet_value, 2)
                            )
                        } else {
                            String::new()
                        }
                    ))
                    .color(Tone::of_sign(result.guaranteed).colors(p).0),
                );
            }
            Err(message) => {
                ui.label(RichText::new(message).color(p.negative));
            }
        }
    });
}

fn state_tone(state: OfferState) -> Tone {
    match state {
        OfferState::Available => Tone::Positive,
        OfferState::Consumed => Tone::Accent,
        OfferState::Ineligible => Tone::Warning,
        OfferState::ToCheck => Tone::Neutral,
    }
}

fn operator_row(ui: &mut Ui, status: &OperatorStatus) -> Option<Request> {
    let p = pal(ui);
    let mut request = None;
    split(
        ui,
        150.0,
        |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&status.name).semibold());
                chip(ui, status.state.label(), state_tone(status.state));
                if let Some(stored) = &status.offer {
                    if let Eligibility::Eligible { kind, amount, .. } = &stored.offer.eligibility {
                        chip(
                            ui,
                            &format!("{} · {}", kind.label(), euros(amount.get(), 0)),
                            Tone::Neutral,
                        );
                    }
                    ui.label(
                        RichText::new(format!(
                            "vérifié le {}",
                            stored.offer.checked_on.format("%d/%m/%Y")
                        ))
                        .small()
                        .color(p.faint),
                    )
                    .on_hover_text(stored.offer.source.as_str());
                }
                if let Some(freebet) = status.pending_freebet {
                    chip(
                        ui,
                        &format!("freebet {} à jouer", euros(freebet.get(), 2)),
                        Tone::Positive,
                    );
                }
                if status.realized != 0.0 {
                    chip(
                        ui,
                        &format!("réalisé {}", signed_euros(status.realized)),
                        Tone::of_sign(status.realized),
                    );
                }
            });
        },
        |ui| {
            let text = if status.offer.is_some() {
                "Corriger l'offre"
            } else {
                "Enregistrer l'offre"
            };
            if button(ui, text, ButtonKind::Ghost).clicked() {
                request = Some(Request::NewOffer(status.key.clone()));
            }
        },
    );
    request
}

fn operation_card(ui: &mut Ui, data: &Data, stored: &StoredOperation) -> Option<Request> {
    let p = pal(ui);
    let op = &stored.operation;
    let mut request = None;
    card(ui, ("bonus-operation", stored.id), true, |ui| {
        split(
            ui,
            300.0,
            |ui| {
                ui.label(
                    RichText::new(fixture(&op.event.home_team, &op.event.away_team)).semibold(),
                );
                ui.label(
                    RichText::new(format!(
                        "n° {} · {} chez {} · {}",
                        stored.id,
                        op.kind.label(),
                        operator_name(&stored.bonus_leg().bookmaker),
                        when(op.event.kickoff, data.now)
                    ))
                    .small()
                    .color(p.muted),
                );
                ui.horizontal_wrapped(|ui| {
                    for leg in &op.legs {
                        let tone = if leg.role == Role::Bonus {
                            Tone::Accent
                        } else {
                            Tone::Neutral
                        };
                        chip(
                            ui,
                            &format!(
                                "{} · {} {} · {}",
                                operator_name(&leg.bookmaker),
                                format::outcome_label(leg.outcome),
                                format::decimal(leg.odds.get(), 2),
                                euros(leg.stake.get(), 2)
                            ),
                            tone,
                        );
                    }
                });
            },
            |ui| match (&stored.settled, tracking::realized_gain(stored)) {
                (Some(settled), Some(gain)) => {
                    ui.label(
                        RichText::new(signed_euros(gain))
                            .semibold()
                            .size(18.0)
                            .color(Tone::of_sign(gain).colors(p).0),
                    );
                    let result = match settled.result {
                        MatchResult::Played(outcome) => format::outcome_label(outcome),
                        MatchResult::Void => "annulé",
                    };
                    ui.label(
                        RichText::new(format!(
                            "réglée : {result} · garanti {}",
                            signed_euros(op.guaranteed)
                        ))
                        .small()
                        .color(p.muted),
                    );
                }
                _ => {
                    ui.label(
                        RichText::new(format!("garanti {}", signed_euros(op.guaranteed)))
                            .semibold(),
                    );
                    ui.label(RichText::new("Résultat du match").small().color(p.muted));
                    ui.horizontal(|ui| {
                        let mut choices = [
                            (
                                "Dom.",
                                MatchResult::Played(edge_core::domain::Outcome::Home),
                            ),
                            ("Nul", MatchResult::Played(edge_core::domain::Outcome::Draw)),
                            (
                                "Ext.",
                                MatchResult::Played(edge_core::domain::Outcome::Away),
                            ),
                            ("Annulé", MatchResult::Void),
                        ];
                        // Dans un bloc aligné à droite, `horizontal` range de droite à gauche : on inverse pour garder l'ordre.
                        if ui.layout().prefer_right_to_left() {
                            choices.reverse();
                        }
                        for (text, result) in choices {
                            if button(ui, text, ButtonKind::Secondary).clicked() {
                                request = Some(Request::Do(Action::SettleBonusManual {
                                    operation_id: stored.id,
                                    result,
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
