//! Écran « Opportunités » : les prix qui dépassent la cote juste de Pinnacle, en cartes, puis le journal récent.
//!
//! Une carte dit tout ce qu'il faut pour décider : le match et le moment, l'opérateur et l'issue, le prix contre la cote juste,
//! la taille de l'avantage, la cote en dessous de laquelle on ne parie plus, la mise suggérée. Une seule action : « J'ai parié ».

use edge_core::betting::kelly::to_units;
use edge_core::betting::live::Opportunity;
use edge_core::betting::recommend::RECOMMEND_MIN_EDGE;
use edge_core::format;
use edge_core::store::journal::{StoredRecommendation, Tier};
use eframe::egui::{RichText, Sense, Ui, vec2};

use super::common::{euros, fixture, league_name, page_header, signed_pct, sport_icon, when};
use super::{PageCtx, Request};
use crate::data::Data;
use crate::design::icons;
use crate::design::motion::reveal;
use crate::design::style::Semibold;
use crate::design::tokens::space;
use crate::design::widgets::{
    ButtonKind, Tone, button, card, chip, edge_meter, empty_state, mono, pal, section_title, split,
};

/// Avantage au bout de la jauge.
const METER_MAX: f64 = 0.08;

/// Cartes d'opportunité affichées au plus.
const MAX_CARDS: usize = 24;

/// Lignes du journal affichées.
const JOURNAL_ROWS: usize = 10;

/// État local de l'écran.
#[derive(Default)]
pub struct State {
    /// Choix du filtre : 0 = notifiées (≥ 3 %), 1 = toutes (≥ 0 %).
    pub filter: usize,
}

/// Dessine l'écran.
pub fn show(ui: &mut Ui, ctx: &PageCtx, data: &Data, state: &mut State) -> Vec<Request> {
    let mut requests = Vec::new();
    let motion = &ctx.motion;
    let p = pal(ui);
    page_header(
        ui,
        motion,
        "Opportunités",
        "Les prix qui dépassent la cote juste de Pinnacle (sa probabilité sans marge). Le prix est celui du dernier relevé.",
    );
    ui.add_space(space::SM);

    reveal(ui, motion, 1, |ui| {
        let labels = [
            format!(
                "Notifiées · ≥ {} %",
                format::decimal(RECOMMEND_MIN_EDGE * 100.0, 0)
            ),
            format!(
                "Toutes · ≥ 0 % ({})",
                data.notified.len() + data.below.len()
            ),
        ];
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        crate::design::widgets::segmented(ui, "opp-filter", &refs, &mut state.filter);
    });
    ui.add_space(space::SM);

    let mut shown: Vec<(&Opportunity, bool)> = data.notified.iter().map(|o| (o, true)).collect();
    if state.filter == 1 {
        shown.extend(data.below.iter().map(|o| (o, false)));
    }
    shown.sort_by(|a, b| b.0.selection.edge.total_cmp(&a.0.selection.edge));
    shown.truncate(MAX_CARDS);

    if shown.is_empty() {
        reveal(ui, motion, 2, |ui| {
            card(ui, "opp-empty", false, |ui| {
                empty_state(
                    ui,
                    icons::Icon::Bolt,
                    "Aucun prix n'atteint le seuil",
                    "C'est le cas la plupart des jours : les opérateurs français prennent 7 à 14 % de marge.",
                );
            });
        });
    }
    for (index, (found, notified)) in shown.iter().enumerate() {
        reveal(ui, motion, 2 + index, |ui| {
            if let Some(request) = opportunity_card(ui, ctx, data, found, *notified, index) {
                requests.push(request);
            }
        });
        ui.add_space(space::XS);
    }

    ui.add_space(space::LG);
    reveal(ui, motion, 3 + shown.len().min(6), |ui| {
        section_title(
            ui,
            "Journal récent",
            Some("chaque ligne est écrite une seule fois, jamais modifiée"),
        );
        ui.add_space(space::XS);
        if data.recommendations.is_empty() {
            ui.label(
                RichText::new("Le journal est vide : le programme y écrit à 14 h 10.")
                    .color(p.muted),
            );
        } else {
            card(ui, "journal", false, |ui| {
                for found in data.recommendations.iter().take(JOURNAL_ROWS) {
                    journal_row(ui, data, found);
                }
            });
        }
    });
    requests
}

fn opportunity_card(
    ui: &mut Ui,
    ctx: &PageCtx,
    data: &Data,
    found: &Opportunity,
    notified: bool,
    index: usize,
) -> Option<Request> {
    let p = pal(ui);
    let s = &found.selection;
    // La recommandation du journal qui correspond à ce prix : c'est elle qu'on suit avec « J'ai parié ».
    let recorded = data
        .recommendations
        .iter()
        .find(|r| r.record.event_id == found.event_id && r.record.tier == Tier::Recommended);
    let followed = recorded.is_some_and(|r| {
        data.bets
            .iter()
            .any(|b| b.bet.recommendation_id == Some(r.id))
    });
    let mut request = None;

    card(ui, ("opp", index), true, |ui| {
        // Ligne 1 : le match à gauche, la cote proposée à droite.
        split(
            ui,
            120.0,
            |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    icons::draw(ui.painter(), rect, sport_icon(&found.sport_key), p.muted);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(fixture(&found.home_team, &found.away_team))
                                .semibold()
                                .size(16.0),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                league_name(&found.sport_key),
                                when(found.kickoff, data.now)
                            ))
                            .small()
                            .color(p.muted),
                        );
                    });
                });
            },
            |ui| {
                ui.label(mono(format::decimal(s.odds.get(), 2)).size(28.0).semibold());
                ui.label(
                    RichText::new(format!(
                        "cote juste {}",
                        format::decimal(1.0 / s.probability.get(), 2)
                    ))
                    .small()
                    .color(p.muted),
                );
            },
        );

        // Ligne 2 : l'avantage, sur une jauge qui marque le seuil de 3 %.
        ui.add_space(space::SM);
        split(
            ui,
            150.0,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(signed_pct(s.edge, 1))
                            .semibold()
                            .color(if notified { p.positive } else { p.warning }),
                    );
                    ui.label(RichText::new("d'avantage").small().color(p.muted));
                });
            },
            |ui| {
                ui.label(
                    RichText::new(format!(
                        "cote minimale {}",
                        format::decimal(found.minimal_odds, 2)
                    ))
                    .small()
                    .color(p.faint),
                );
            },
        );
        let meter_progress = ctx.motion.progress(0.25 + 0.05 * index as f32, 0.6);
        edge_meter(
            ui,
            s.edge,
            RECOMMEND_MIN_EDGE,
            METER_MAX,
            meter_progress,
            ui.available_width(),
        );

        // Ligne 3 : d'où vient le prix, et ce qu'on en fait.
        ui.add_space(space::SM);
        split(
            ui,
            240.0,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    chip(ui, &found.bookmaker, Tone::Neutral);
                    chip(ui, format::outcome_label(s.outcome), Tone::Accent);
                    match (recorded, notified) {
                        (Some(r), _) => {
                            chip(ui, &format!("au journal n° {}", r.id), Tone::Positive)
                        }
                        (None, true) => chip(ui, "pas encore au journal", Tone::Neutral),
                        (None, false) => chip(ui, "journal fantôme", Tone::Neutral),
                    };
                });
            },
            |ui| {
                match found.stake {
                    Some(stake) => {
                        // L'unité (1 % de la bankroll) se calcule avec la bankroll du jour de la recommandation, si elle existe.
                        let bankroll =
                            recorded.map_or(data.curve.available, |r| r.record.bankroll.get());
                        let in_units = format::units(to_units(stake.get() / bankroll));
                        ui.label(
                            RichText::new(format!(
                                "Mise suggérée {} ({})",
                                in_units,
                                euros(stake.get(), 1)
                            ))
                            .semibold(),
                        );
                    }
                    None if notified => {
                        ui.label(RichText::new("Mise sous le minimum").small().color(p.muted));
                    }
                    None => {
                        ui.label(RichText::new("Observation").small().color(p.muted));
                    }
                }
                match recorded {
                    Some(r) if !followed => {
                        if button(ui, "J'ai parié", ButtonKind::Primary).clicked() {
                            request = Some(Request::Follow(r.id));
                        }
                    }
                    Some(_) => {
                        chip(ui, "pari enregistré", Tone::Positive);
                    }
                    None => {}
                }
            },
        );
    });
    request
}

fn journal_row(ui: &mut Ui, data: &Data, found: &StoredRecommendation) {
    let p = pal(ui);
    let r = &found.record;
    let followed = data
        .bets
        .iter()
        .any(|b| b.bet.recommendation_id == Some(found.id));
    // Que des composants simples : seuls eux passent à la ligne quand la fenêtre est étroite.
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("n° {}", found.id))
                .small()
                .color(p.faint),
        );
        match r.tier {
            Tier::Recommended => chip(ui, "notifiée", Tone::Accent),
            Tier::Shadow => chip(ui, "fantôme", Tone::Neutral),
        };
        ui.label(RichText::new(fixture(&r.home_team, &r.away_team)).semibold());
        ui.label(
            RichText::new(format!(
                "{} · {} à {}",
                r.bookmaker,
                format::outcome_label(r.outcome),
                format::decimal(r.odds.get(), 2)
            ))
            .color(p.muted),
        );
        let edge_color = if r.tier == Tier::Recommended {
            p.positive
        } else {
            p.warning
        };
        ui.label(RichText::new(signed_pct(r.edge, 1)).color(edge_color));
        if followed {
            chip(ui, "parié", Tone::Positive);
        }
        ui.label(
            RichText::new(format::kickoff_label(r.kickoff))
                .small()
                .color(p.faint),
        );
    });
}
