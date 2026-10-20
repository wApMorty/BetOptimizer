//! Écran « Aujourd'hui » : le verdict du jour en une phrase, trois indicateurs, la santé des relevés, ce qui vient.
//!
//! Il répond, sans rien lire d'autre, à quatre questions : y a-t-il quelque chose à parier ? Est-ce que ça marche ? Combien
//! ai-je ? Tout tourne-t-il ? Le détail est un clic plus loin.

use edge_core::betting::recommend::RECOMMEND_MIN_EDGE;
use edge_core::format;
use eframe::egui::{RichText, Sense, Ui, vec2};

use super::common::{euros, fixture, league_name, page_header, signed_pct, sport_icon, when};
use super::{PageCtx, Request, Tab};
use crate::data::Data;
use crate::design::icons::{self, Icon};
use crate::design::motion::{DRAW_SECS, reveal};
use crate::design::style::Semibold;
use crate::design::tokens::space;
use crate::design::widgets::{
    ButtonKind, Tone, button, card, chip, counted, gauge, pal, section_title, sparkline, split,
    stat_tile, status_chip, tile_columns,
};

/// Clôtures nécessaires avant de lire le CLV autrement que comme un indice.
const CLOSINGS_NEEDED: usize = 30;

/// État local de l'écran.
#[derive(Default)]
pub struct State {
    /// Le bandeau « aucune règle validée » est-il déplié ?
    pub why_open: bool,
}

/// Dessine l'écran.
pub fn show(ui: &mut Ui, ctx: &PageCtx, data: &Data, state: &mut State) -> Vec<Request> {
    let mut requests = Vec::new();
    let motion = &ctx.motion;
    let p = pal(ui);

    page_header(ui, motion, &verdict_title(data), &verdict_subtitle(data));
    ui.add_space(space::SM);

    // Verdict : une carte avec l'action principale.
    reveal(ui, motion, 1, |ui| {
        card(ui, "verdict", false, |ui| {
            split(
                ui,
                190.0,
                |ui| {
                    ui.horizontal(|ui| {
                let (bar, _) = ui.allocate_exact_size(vec2(4.0, 56.0), Sense::hover());
                let tone = if data.notified.is_empty() { p.border } else { p.accent };
                ui.painter().rect_filled(bar, 2.0, tone);
                ui.vertical(|ui| match best(data) {
                    Some(top) => {
                        ui.label(RichText::new(format!("Meilleur avantage {}", signed_pct(top.selection.edge, 1))).color(p.positive).semibold());
                        ui.label(RichText::new(format!(
                            "{} · {} · {} à {}",
                            fixture(&top.home_team, &top.away_team),
                            top.bookmaker,
                            format::outcome_label(top.selection.outcome),
                            format::decimal(top.selection.odds.get(), 2)
                        )).color(p.muted));
                        ui.label(RichText::new(when(top.kickoff, data.now)).small().color(p.faint));
                    }
                    None => {
                        ui.label(RichText::new("Le programme compare chaque jour les prix des opérateurs à ceux de Pinnacle.").color(p.muted));
                        ui.label(RichText::new(format!(
                            "Un prix n'est signalé que s'il dépasse la cote juste d'au moins {} %.",
                            format::decimal(RECOMMEND_MIN_EDGE * 100.0, 0)
                        )).small().color(p.faint));
                    }
                });
            });
                },
                |ui| {
                    let label = if data.notified.is_empty() {
                        "Voir les prix"
                    } else {
                        "Voir les opportunités"
                    };
                    if button(ui, label, ButtonKind::Primary).clicked() {
                        requests.push(Request::Go(Tab::Opportunities));
                    }
                },
            );
        });
    });

    // Trois indicateurs.
    ui.add_space(space::SM);
    let n = tile_columns(ui);
    ui.columns(n, |cols| {
        reveal(&mut cols[0], motion, 2, |ui| bankroll_tile(ui, ctx, data));
        reveal(&mut cols[1 % n], motion, 3, |ui| clv_tile(ui, ctx, data));
        reveal(&mut cols[2 % n], motion, 4, |ui| {
            credits_tile(ui, ctx, data)
        });
    });

    // Santé des relevés.
    ui.add_space(space::MD);
    reveal(ui, motion, 5, |ui| {
        section_title(
            ui,
            "Ça tourne ?",
            Some("dernier relevé réussi de chaque sport"),
        );
        ui.add_space(space::XS);
        ui.horizontal_wrapped(|ui| {
            for feed in &data.feeds {
                let (tone, text) = match feed.last {
                    Some(at) => {
                        let hours = (data.now - at).num_hours();
                        let tone = if hours < 30 {
                            Tone::Positive
                        } else if hours < 72 {
                            Tone::Warning
                        } else {
                            Tone::Negative
                        };
                        (
                            tone,
                            format!("{} · {}", feed.label, format::age(at, data.now)),
                        )
                    }
                    None => (Tone::Neutral, format!("{} · aucun relevé", feed.label)),
                };
                status_chip(ui, &text, tone, tone == Tone::Positive);
            }
        });
    });

    // Prochains paris ouverts.
    let upcoming: Vec<_> = {
        let mut open: Vec<_> = data.bets.iter().filter(|b| b.result.is_none()).collect();
        open.sort_by_key(|b| b.bet.kickoff);
        open.into_iter().take(3).collect()
    };
    ui.add_space(space::MD);
    reveal(ui, motion, 6, |ui| {
        section_title(ui, "Paris ouverts", None);
        ui.add_space(space::XS);
        if upcoming.is_empty() {
            ui.label(RichText::new("Aucun pari ouvert.").color(p.muted));
        }
        for stored in &upcoming {
            let b = &stored.bet;
            card(ui, ("open-bet", stored.id), true, |ui| {
                split(
                    ui,
                    150.0,
                    |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                            icons::draw(ui.painter(), rect, sport_icon(&b.sport_key), p.muted);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(fixture(&b.home_team, &b.away_team)).semibold(),
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
                            });
                        });
                    },
                    |ui| {
                        ui.label(RichText::new(euros(b.stake.get(), 1)).semibold());
                        let side = format!(
                            "{} à {}",
                            format::outcome_label(b.outcome),
                            format::decimal(b.odds.get(), 2)
                        );
                        chip(ui, &side, Tone::Neutral);
                    },
                );
            });
            ui.add_space(space::XS);
        }
    });

    // Rappel permanent, calme : aucune règle validée.
    ui.add_space(space::MD);
    reveal(ui, motion, 7, |ui| {
        let (fg, bg) = Tone::Warning.colors(p);
        eframe::egui::Frame::new()
            .fill(bg)
            .corner_radius(eframe::egui::CornerRadius::same(crate::design::tokens::radius::CARD))
            .inner_margin(eframe::egui::Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                split(ui, 110.0, |ui| { ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    icons::draw(ui.painter(), rect, Icon::Info, fg);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Aucune règle n'est validée hors échantillon : les recommandations sont expérimentales.").color(fg));
                    });
                }); }, |ui| {
                    let label = if state.why_open { "Masquer" } else { "Pourquoi ?" };
                    if button(ui, label, ButtonKind::Ghost).clicked() {
                        state.why_open = !state.why_open;
                    }
                });
                let open = ui.ctx().animate_bool_with_time(ui.id().with("why"), state.why_open, 0.2);
                if open > 0.0 {
                    ui.scope(|ui| {
                        ui.set_opacity(open);
                        ui.add_space(space::SM);
                        ui.label(RichText::new("Les mesures disponibles (rapports Backtest, Modèles et Tennis) ne montrent pas d'avantage : les modèles font moins bien que le marché, et les prix « mous » contre Pinnacle n'ont presque aucun pari de test. Le seul vrai test est le CLV réel, qui s'accumule dans le journal (écran Performance). Vérifie toujours la cote sur le site de l'opérateur.").color(p.text));
                    });
                }
            });
    });
    requests
}

/// La meilleure opportunité notifiée.
fn best(data: &Data) -> Option<&edge_core::betting::live::Opportunity> {
    data.notified
        .iter()
        .max_by(|a, b| a.selection.edge.total_cmp(&b.selection.edge))
}

/// Titre de l'écran : le verdict.
pub fn verdict_title(data: &Data) -> String {
    match data.notified.len() {
        0 => "Rien à parier pour l'instant".to_string(),
        1 => "1 opportunité".to_string(),
        n => format!("{n} opportunités"),
    }
}

/// Phrase sous le verdict.
pub fn verdict_subtitle(data: &Data) -> String {
    let date = format::long_date(data.now);
    if data.scan.events == 0 {
        return format!("{date} · aucun relevé récent : le prochain arrive à 14 h");
    }
    let below = data.below.len();
    if data.notified.is_empty() {
        format!(
            "{date} · {} matchs examinés, aucun prix n'atteint le seuil (c'est le cas la plupart des jours)",
            data.scan.events
        )
    } else {
        format!(
            "{date} · {} matchs examinés · {below} autre(s) prix positif(s) sous le seuil",
            data.scan.events
        )
    }
}

fn bankroll_tile(ui: &mut Ui, ctx: &PageCtx, data: &Data) {
    let p = pal(ui);
    let value = counted(data.curve.available, &ctx.motion, 0.15);
    let mut series: Vec<f64> = vec![data.curve.initial];
    series.extend(data.curve.points.iter().map(|point| point.bankroll));
    let tone = Tone::Neutral;
    stat_tile(
        ui,
        "tile-bankroll",
        "Bankroll disponible",
        euros(value, 0),
        tone,
        |ui| {
            sparkline(
                ui,
                &series,
                vec2(ui.available_width(), 38.0),
                Tone::of_sign(data.curve.settled_profit),
                ctx.motion.linear(0.2, DRAW_SECS),
            );
            ui.add_space(space::XS);
            ui.label(
                RichText::new(format!(
                    "gains réglés {}",
                    super::common::signed_euros(data.curve.settled_profit)
                ))
                .small()
                .color(p.muted),
            );
        },
    );
}

fn clv_tile(ui: &mut Ui, ctx: &PageCtx, data: &Data) {
    let p = pal(ui);
    // Le CLV du journal : les recommandations notifiées si elles ont des clôtures, sinon le journal fantôme.
    let group = data
        .clv
        .groups
        .iter()
        .take(2)
        .max_by_key(|g| g.with_closing)
        .filter(|g| g.with_closing > 0);
    let (figure_text, tone, closings) = match group {
        Some(g) => match g.clv {
            // Vert ou rouge seulement si l'intervalle exclut zéro : sinon la différence n'est pas distinguable du bruit.
            Some(ci) => (
                signed_pct(ci.mean, 2),
                if ci.excludes_zero() {
                    Tone::of_sign(ci.mean)
                } else {
                    Tone::Neutral
                },
                g.with_closing,
            ),
            None => ("—".to_string(), Tone::Neutral, g.with_closing),
        },
        None => ("—".to_string(), Tone::Neutral, 0),
    };
    stat_tile(ui, "tile-clv", "CLV réel", figure_text, tone, |ui| {
        gauge(
            ui,
            closings as f32 / CLOSINGS_NEEDED as f32,
            ctx.motion.progress(0.3, 0.6),
            Tone::Accent,
        );
        ui.add_space(space::XS);
        let note = if closings >= CLOSINGS_NEEDED {
            match group.and_then(|g| g.clv) {
                Some(ci) if ci.excludes_zero() => format!("{closings} clôtures : différence nette"),
                _ => format!("{closings} clôtures : pas distinct de zéro"),
            }
        } else {
            format!("{closings} / {CLOSINGS_NEEDED} clôtures : trop peu")
        };
        ui.label(RichText::new(note).small().color(p.muted));
    });
}

fn credits_tile(ui: &mut Ui, ctx: &PageCtx, data: &Data) {
    let p = pal(ui);
    match data.credits {
        Some(c) => {
            let tone = if c.used * 10 >= c.cap * 9 {
                Tone::Negative
            } else if c.used * 10 >= c.cap * 7 {
                Tone::Warning
            } else {
                Tone::Neutral
            };
            stat_tile(
                ui,
                "tile-credits",
                "Crédits API ce mois",
                format!("{} / {}", c.used, c.cap),
                Tone::Neutral,
                |ui| {
                    gauge(
                        ui,
                        c.used as f32 / c.cap as f32,
                        ctx.motion.progress(0.3, 0.6),
                        tone,
                    );
                    ui.add_space(space::XS);
                    ui.label(
                        RichText::new(format!("{} avant le plafond", c.cap.saturating_sub(c.used)))
                            .small()
                            .color(p.muted),
                    );
                },
            );
        }
        None => {
            stat_tile(
                ui,
                "tile-credits",
                "Crédits API ce mois",
                "—".to_string(),
                Tone::Neutral,
                |ui| {
                    ui.label(
                        RichText::new("pas encore lus : le premier relevé les lit")
                            .small()
                            .color(p.muted),
                    );
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn the_verdict_counts_the_opportunities_or_says_there_is_nothing() {
        let mut data = demo::data("2026-10-19T12:00:00Z".parse().unwrap());
        assert_eq!(verdict_title(&data), "3 opportunités");
        data.notified.truncate(1);
        assert_eq!(verdict_title(&data), "1 opportunité");
        data.notified.clear();
        assert_eq!(verdict_title(&data), "Rien à parier pour l'instant");
        assert!(verdict_subtitle(&data).contains("aucun prix n'atteint le seuil"));
        data.scan.events = 0;
        assert!(verdict_subtitle(&data).contains("aucun relevé récent"));
    }
}
