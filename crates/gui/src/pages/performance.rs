//! Écran « Performance » : bankroll, CLV réel, calibration. Un contrôle segmenté choisit la vue ; les courbes se tracent à l'arrivée.

use chrono::DateTime;
use edge_core::eval::bootstrap::MeanCi;
use edge_core::format;
use eframe::egui::{Color32, RichText, Ui};
use egui_plot::{Bar, BarChart, HLine, Line, Plot, PlotPoints, Points};

use super::common::{euros, page_header, signed_euros, signed_pct};
use super::{PageCtx, Request};
use crate::data::Data;
use crate::design::icons::Icon;
use crate::design::motion::{DRAW_SECS, reveal};
use crate::design::style::Semibold;
use crate::design::tokens::space;
use crate::design::widgets::{
    Tone, card, chip, counted, empty_state, pal, revealed_points, section_title, segmented, split,
    stat_tile, tile_columns,
};

/// Effectif minimal d'un groupe pour que son intervalle ne soit pas qu'indicatif.
const MIN_FOR_CONFIDENCE: usize = 30;

/// Secondes par jour : l'abscisse de la courbe est un nombre de jours depuis l'époque Unix.
const DAY: f64 = 86_400.0;

/// État local de l'écran.
#[derive(Default)]
pub struct State {
    /// Vue choisie : 0 bankroll, 1 CLV, 2 calibration.
    pub segment: usize,
}

/// Dessine l'écran.
pub fn show(ui: &mut Ui, ctx: &PageCtx, data: &Data, state: &mut State) -> Vec<Request> {
    let motion = &ctx.motion;
    page_header(
        ui,
        motion,
        "Performance",
        "Ce que valent vraiment les recommandations : l'argent, puis la mesure principale (le CLV), puis la fiabilité des probabilités.",
    );
    ui.add_space(space::SM);
    reveal(ui, motion, 1, |ui| {
        segmented(
            ui,
            "perf-segment",
            &["Bankroll", "CLV", "Calibration"],
            &mut state.segment,
        );
    });
    ui.add_space(space::MD);
    match state.segment {
        0 => bankroll(ui, ctx, data),
        1 => clv(ui, ctx, data),
        _ => calibration(ui, ctx, data),
    }
    Vec::new()
}

fn bankroll(ui: &mut Ui, ctx: &PageCtx, data: &Data) {
    let motion = &ctx.motion;
    let p = pal(ui);
    let curve = &data.curve;
    reveal(ui, motion, 2, |ui| {
        let n = tile_columns(ui);
        ui.columns(n, |cols| {
            stat_tile(
                &mut cols[0],
                "perf-available",
                "Disponible",
                euros(counted(curve.available, motion, 0.1), 0),
                Tone::Neutral,
                |ui| {
                    ui.label(
                        RichText::new(format!("départ {}", euros(curve.initial, 0)))
                            .small()
                            .color(p.muted),
                    );
                },
            );
            stat_tile(
                &mut cols[1 % n],
                "perf-profit",
                "Gains nets réglés",
                signed_euros(counted(curve.settled_profit, motion, 0.15)),
                Tone::of_sign(curve.settled_profit),
                |ui| {
                    ui.label(
                        RichText::new(format!("mises ouvertes {}", euros(curve.open_stakes, 0)))
                            .small()
                            .color(p.muted),
                    );
                },
            );
            stat_tile(
                &mut cols[2 % n],
                "perf-drawdown",
                "Plus forte baisse",
                format!("{} %", format::decimal(curve.max_drawdown * 100.0, 1)),
                Tone::Neutral,
                |ui| {
                    ui.label(
                        RichText::new("depuis un sommet de la bankroll réalisée")
                            .small()
                            .color(p.muted),
                    );
                },
            );
        });
    });
    ui.add_space(space::MD);
    reveal(ui, motion, 3, |ui| {
        card(ui, "perf-bankroll-chart", false, |ui| {
            section_title(
                ui,
                "Bankroll réalisée",
                Some("un point à chaque pari réglé"),
            );
            ui.add_space(space::SM);
            if curve.points.is_empty() {
                empty_state(
                    ui,
                    Icon::Chart,
                    "Aucun pari réglé",
                    "La courbe apparaît au premier règlement.",
                );
                return;
            }
            let points: Vec<[f64; 2]> = curve
                .points
                .iter()
                .map(|point| [point.at.timestamp() as f64 / DAY, point.bankroll])
                .collect();
            // La courbe se trace de gauche à droite à l'arrivée de l'écran.
            let reveal_fraction = motion.linear(0.2, DRAW_SECS);
            let screen_like: Vec<eframe::egui::Pos2> = points
                .iter()
                .map(|[x, y]| eframe::egui::pos2(*x as f32, *y as f32))
                .collect();
            let visible: Vec<[f64; 2]> = revealed_points(
                &screen_like,
                crate::design::motion::ease_out(reveal_fraction),
            )
            .into_iter()
            .map(|pt| [f64::from(pt.x), f64::from(pt.y)])
            .collect();
            let first = points.first().map_or(0.0, |pt| pt[0]);
            let last = points.last().map_or(1.0, |pt| pt[0]);
            Plot::new("bankroll_curve")
                .height(300.0)
                .show_grid([false, true])
                .show_background(false)
                .grid_spacing(80.0..=400.0)
                .grid_color(p.muted)
                .allow_zoom(false)
                .allow_drag(false)
                .allow_scroll(false)
                .include_x(first)
                .include_x(last)
                .include_y(curve.initial)
                .x_axis_formatter(|mark, _| {
                    DateTime::from_timestamp((mark.value * DAY) as i64, 0)
                        .map_or(String::new(), |t| t.format("%d/%m").to_string())
                })
                .y_axis_formatter(|mark, _| format!("{:.0}", mark.value))
                .show(ui, |plot| {
                    plot.hline(
                        HLine::new("départ", curve.initial)
                            .color(p.faint)
                            .width(1.0),
                    );
                    plot.line(
                        Line::new("bankroll réalisée", PlotPoints::from(visible))
                            .color(p.accent)
                            .width(2.5),
                    );
                });
        });
    });
}

fn interval_text(ci: Option<MeanCi>) -> String {
    match ci {
        Some(c) => format!(
            "[{} ; {}]",
            format::signed(c.low * 100.0, 1),
            format::signed(c.high * 100.0, 1)
        ),
        None => "pas de mesure".to_string(),
    }
}

fn clv(ui: &mut Ui, ctx: &PageCtx, data: &Data) {
    let motion = &ctx.motion;
    let p = pal(ui);
    reveal(ui, motion, 2, |ui| {
        ui.label(RichText::new("CLV = cote obtenue × probabilité de clôture de Pinnacle sans marge − 1. Positif : on a eu un meilleur prix que le marché final. Intervalle à 95 %.").color(p.muted));
    });
    ui.add_space(space::SM);
    for (index, group) in data.clv.groups.iter().enumerate() {
        reveal(ui, motion, 3 + index, |ui| {
            card(ui, ("clv-group", index), true, |ui| {
                split(
                    ui,
                    170.0,
                    |ui| {
                        ui.label(RichText::new(group.label).semibold());
                        ui.label(
                            RichText::new(format!(
                                "{} élément(s) · clôture connue {} · réglés {}",
                                group.n, group.with_closing, group.settled
                            ))
                            .small()
                            .color(p.muted),
                        );
                    },
                    |ui| {
                        match group.clv {
                            Some(c) => {
                                let tone = if c.excludes_zero() {
                                    Tone::of_sign(c.mean)
                                } else {
                                    Tone::Neutral
                                };
                                let color = if tone == Tone::Neutral {
                                    p.text
                                } else {
                                    tone.colors(p).0
                                };
                                ui.label(
                                    RichText::new(signed_pct(c.mean, 2))
                                        .semibold()
                                        .size(20.0)
                                        .color(color),
                                );
                                ui.label(
                                    RichText::new(interval_text(group.clv))
                                        .small()
                                        .color(p.muted),
                                );
                            }
                            None => {
                                ui.label(RichText::new("—").semibold().size(20.0).color(p.faint));
                                ui.label(
                                    RichText::new("pas encore de clôture")
                                        .small()
                                        .color(p.muted),
                                );
                            }
                        };
                        if group.with_closing < MIN_FOR_CONFIDENCE {
                            chip(ui, "indicatif", Tone::Warning);
                        }
                    },
                );
            });
        });
        ui.add_space(space::XS);
    }

    ui.add_space(space::MD);
    reveal(ui, motion, 8, |ui| {
        card(ui, "clv-buckets", false, |ui| {
            section_title(
                ui,
                "CLV par tranche d'avantage annoncé",
                Some("si l'avantage est réel, le CLV monte avec lui"),
            );
            ui.add_space(space::SM);
            let growth = motion.progress(0.3, 0.7);
            let bars: Vec<Bar> = data
                .clv
                .buckets
                .iter()
                .enumerate()
                .filter_map(|(i, b)| {
                    b.mean_clv.map(|clv| {
                        let color: Color32 = if clv >= 0.0 { p.positive } else { p.negative };
                        Bar::new(i as f64, clv * 100.0 * f64::from(growth))
                            .fill(color)
                            .width(0.55)
                            .name(b.label)
                    })
                })
                .collect();
            if bars.is_empty() {
                empty_state(
                    ui,
                    Icon::Chart,
                    "Aucune clôture pour l'instant",
                    "Le CLV apparaît après les premiers matchs suivis.",
                );
                return;
            }
            let labels: Vec<&str> = data.clv.buckets.iter().map(|b| b.label).collect();
            Plot::new("clv_buckets")
                .height(240.0)
                .show_grid([false, true])
                .show_background(false)
                .grid_spacing(80.0..=400.0)
                .grid_color(p.muted)
                .allow_zoom(false)
                .allow_drag(false)
                .allow_scroll(false)
                .x_axis_formatter(move |mark, _| {
                    let index = mark.value.round();
                    if (mark.value - index).abs() < 1e-6 && index >= 0.0 {
                        labels
                            .get(index as usize)
                            .map_or(String::new(), |l| (*l).to_string())
                    } else {
                        String::new()
                    }
                })
                .y_axis_formatter(|mark, _| format!("{:+.1} %", mark.value))
                .show(ui, |plot| {
                    plot.hline(HLine::new("zéro", 0.0).color(p.faint).width(1.0));
                    plot.bar_chart(BarChart::new("CLV moyen", bars));
                });
        });
    });
}

fn calibration(ui: &mut Ui, ctx: &PageCtx, data: &Data) {
    let motion = &ctx.motion;
    let p = pal(ui);
    reveal(ui, motion, 2, |ui| {
        ui.label(RichText::new("Recommandations réglées : la probabilité de Pinnacle annoncée contre la fréquence à laquelle l'issue s'est produite. Sur la diagonale, elle est fiable.").color(p.muted));
    });
    ui.add_space(space::SM);
    reveal(ui, motion, 3, |ui| {
        card(ui, "calibration", false, |ui| {
            if data.reliability.is_empty() {
                empty_state(
                    ui,
                    Icon::Chart,
                    "Aucune recommandation réglée",
                    "Le diagramme se construit au fil des règlements.",
                );
                return;
            }
            let observed: PlotPoints<'_> = data
                .reliability
                .iter()
                .map(|b| {
                    [
                        b.mean_probability,
                        b.frequency * f64::from(motion.progress(0.2, 0.7)),
                    ]
                })
                .collect();
            Plot::new("reliability")
                .height(320.0)
                .show_background(false)
                .grid_spacing(45.0..=400.0)
                .grid_color(p.muted)
                .data_aspect(1.0)
                .allow_zoom(false)
                .allow_drag(false)
                .allow_scroll(false)
                .include_x(0.0)
                .include_x(1.0)
                .include_y(0.0)
                .include_y(1.0)
                .x_axis_label("probabilité annoncée")
                .y_axis_label("fréquence observée")
                .show(ui, |plot| {
                    plot.line(
                        Line::new(
                            "fiabilité parfaite",
                            PlotPoints::from(vec![[0.0, 0.0], [1.0, 1.0]]),
                        )
                        .color(p.faint)
                        .width(1.0),
                    );
                    plot.points(
                        Points::new("recommandations réglées", observed)
                            .radius(6.0)
                            .color(p.accent),
                    );
                });
            ui.add_space(space::SM);
            for bin in &data.reliability {
                ui.horizontal(|ui| {
                    chip(
                        ui,
                        &format!(
                            "{} – {} %",
                            format::decimal(bin.low * 100.0, 0),
                            format::decimal(bin.high * 100.0, 0)
                        ),
                        Tone::Neutral,
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} réglées · annoncé {} % · observé {} %",
                            bin.n,
                            format::decimal(bin.mean_probability * 100.0, 1),
                            format::decimal(bin.frequency * 100.0, 1)
                        ))
                        .color(p.muted),
                    );
                });
            }
        });
    });
}
