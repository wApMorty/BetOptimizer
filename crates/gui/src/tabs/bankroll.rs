//! Onglet « Bankroll » : la courbe réalisée et ses indicateurs.

use chrono::DateTime;
use edge_core::format;
use eframe::egui::{Grid, ScrollArea, Ui};
use egui_plot::{HLine, Line, Plot, PlotPoints};

use crate::actions::Action;
use crate::data::Data;
use crate::theme;

/// Secondes par jour : l'abscisse de la courbe est un nombre de jours depuis l'époque Unix.
const DAY: f64 = 86_400.0;

/// Dessine l'onglet.
pub fn show(ui: &mut Ui, data: &Data) -> Vec<Action> {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let curve = &data.curve;
            ui.heading("Bankroll");
            Grid::new("bankroll_summary").num_columns(2).show(ui, |ui| {
                ui.label("Départ");
                ui.label(format!("{} €", format::decimal(curve.initial, 2)));
                ui.end_row();
                ui.label("Gains nets réglés");
                ui.label(theme::signed_euros(ui, curve.settled_profit));
                ui.end_row();
                ui.label("Mises ouvertes");
                ui.label(format!("{} €", format::decimal(curve.open_stakes, 2)));
                ui.end_row();
                ui.label("Disponible");
                ui.label(format!("{} €", format::decimal(curve.available, 2)));
                ui.end_row();
                ui.label("Plus forte baisse depuis un sommet");
                ui.label(format!(
                    "{} %",
                    format::decimal(curve.max_drawdown * 100.0, 1)
                ));
                ui.end_row();
            });
            ui.add_space(8.0);
            if curve.points.is_empty() {
                ui.label("Aucun pari réglé : la courbe apparaîtra au premier règlement.");
                return;
            }
            let points: PlotPoints<'_> = curve
                .points
                .iter()
                .map(|p| [p.at.timestamp() as f64 / DAY, p.bankroll])
                .collect();
            Plot::new("bankroll_curve")
                .height(320.0)
                .allow_zoom(false)
                .x_axis_formatter(|mark, _| {
                    DateTime::from_timestamp((mark.value * DAY) as i64, 0)
                        .map_or(String::new(), |t| t.format("%d/%m").to_string())
                })
                .y_axis_label("€")
                .show(ui, |plot| {
                    plot.hline(HLine::new("départ", curve.initial));
                    plot.line(Line::new("bankroll réalisée", points));
                });
        });
    Vec::new()
}
