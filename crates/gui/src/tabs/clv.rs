//! Onglet « CLV et calibration » : le CLV réel par groupe et par tranche d'avantage, et la fiabilité des probabilités annoncées.

use edge_core::eval::bootstrap::MeanCi;
use edge_core::format;
use eframe::egui::{Grid, RichText, ScrollArea, Ui};
use egui_plot::{Bar, BarChart, Line, Plot, PlotPoints, Points};

use crate::actions::Action;
use crate::data::Data;
use crate::theme;

/// Effectif minimal d'un groupe pour que son intervalle ne soit pas qu'indicatif.
const MIN_FOR_CONFIDENCE: usize = 30;

/// Dessine l'onglet.
pub fn show(ui: &mut Ui, data: &Data) -> Vec<Action> {
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.heading("CLV réel");
        ui.label("CLV = cote obtenue × probabilité de clôture de Pinnacle sans marge − 1 (intervalle à 95 %). La mesure principale du projet.");
        Grid::new("clv_groups").striped(true).num_columns(5).show(ui, |ui| {
            for title in ["Groupe", "Effectif", "Clôture connue", "CLV moyen", "Rendement à mise fixe"] {
                ui.label(RichText::new(title).strong());
            }
            ui.end_row();
            for group in &data.clv.groups {
                ui.label(group.label);
                ui.label(group.n.to_string());
                ui.label(format!("{} (réglés : {})", group.with_closing, group.settled));
                interval(ui, group.clv, group.with_closing);
                interval(ui, group.flat_roi, group.settled);
                ui.end_row();
            }
        });
        ui.add_space(10.0);

        ui.heading("CLV par tranche d'avantage annoncé");
        ui.label("Notifiées et journal fantôme confondus : si l'avantage annoncé est réel, le CLV monte avec lui.");
        let bars: Vec<Bar> = data
            .clv
            .buckets
            .iter()
            .enumerate()
            .filter_map(|(i, b)| b.mean_clv.map(|clv| Bar::new(i as f64, clv * 100.0).name(b.label)))
            .collect();
        if bars.is_empty() {
            ui.label("Aucune clôture relevée pour l'instant : le CLV apparaîtra après les premiers matchs suivis.");
        } else {
            let labels: Vec<&str> = data.clv.buckets.iter().map(|b| b.label).collect();
            Plot::new("clv_buckets")
                .height(240.0)
                .allow_zoom(false)
                .allow_drag(false)
                .x_axis_formatter(move |mark, _| {
                    let index = mark.value.round();
                    if (mark.value - index).abs() < 1e-6 && index >= 0.0 {
                        labels.get(index as usize).map_or(String::new(), |l| (*l).to_string())
                    } else {
                        String::new()
                    }
                })
                .y_axis_label("CLV moyen (%)")
                .show(ui, |plot| plot.bar_chart(BarChart::new("CLV moyen", bars)));
        }
        Grid::new("clv_buckets_table").striped(true).num_columns(4).show(ui, |ui| {
            for title in ["Tranche", "Effectif", "Clôture connue", "Avantage annoncé moyen"] {
                ui.label(RichText::new(title).strong());
            }
            ui.end_row();
            for b in &data.clv.buckets {
                ui.label(b.label);
                ui.label(b.n.to_string());
                ui.label(b.with_closing.to_string());
                ui.label(format!("{} %", format::signed(b.mean_edge * 100.0, 1)));
                ui.end_row();
            }
        });
        ui.add_space(10.0);

        ui.heading("Fiabilité des probabilités annoncées");
        ui.label("Recommandations réglées : la probabilité de Pinnacle annoncée contre la fréquence à laquelle l'issue s'est produite. Sur la diagonale, elle est fiable.");
        if data.reliability.is_empty() {
            ui.label("Aucune recommandation réglée pour l'instant.");
        } else {
            let observed: PlotPoints<'_> = data
                .reliability
                .iter()
                .map(|b| [b.mean_probability, b.frequency])
                .collect();
            Plot::new("reliability")
                .height(300.0)
                .data_aspect(1.0)
                .include_x(0.0)
                .include_x(1.0)
                .include_y(0.0)
                .include_y(1.0)
                .x_axis_label("probabilité annoncée")
                .y_axis_label("fréquence observée")
                .show(ui, |plot| {
                    plot.line(Line::new("fiabilité parfaite", PlotPoints::from(vec![[0.0, 0.0], [1.0, 1.0]])));
                    plot.points(Points::new("recommandations réglées", observed).radius(5.0));
                });
            Grid::new("reliability_table").striped(true).num_columns(4).show(ui, |ui| {
                for title in ["Tranche", "Effectif", "Annoncée (moyenne)", "Observée"] {
                    ui.label(RichText::new(title).strong());
                }
                ui.end_row();
                for b in &data.reliability {
                    ui.label(format!("{} – {} %", format::decimal(b.low * 100.0, 0), format::decimal(b.high * 100.0, 0)));
                    ui.label(b.n.to_string());
                    ui.label(format!("{} %", format::decimal(b.mean_probability * 100.0, 1)));
                    ui.label(format!("{} %", format::decimal(b.frequency * 100.0, 1)));
                    ui.end_row();
                }
            });
        }
    });
    Vec::new()
}

/// Moyenne et intervalle en pourcentage signé et coloré ; sous 30 éléments, la mention « indicatif ».
fn interval(ui: &mut Ui, ci: Option<MeanCi>, n: usize) {
    match ci {
        None => {
            ui.label("—");
        }
        Some(c) => {
            ui.horizontal(|ui| {
                ui.label(theme::signed_percent(ui, c.mean, 2));
                ui.label(format!(
                    "[{} ; {}]",
                    format::signed(c.low * 100.0, 1),
                    format::signed(c.high * 100.0, 1)
                ));
                if n < MIN_FOR_CONFIDENCE {
                    ui.label(
                        RichText::new("indicatif")
                            .italics()
                            .color(theme::warning(ui)),
                    );
                }
            });
        }
    }
}
