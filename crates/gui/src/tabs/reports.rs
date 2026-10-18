//! Onglet « Rapports » : les rapports HTML générés par la ligne de commande, ouverts dans le navigateur.

use edge_core::format;
use eframe::egui::{Grid, RichText, ScrollArea, Ui};

use crate::actions::Action;
use crate::data::Data;

/// Dessine l'onglet.
pub fn show(ui: &mut Ui, data: &Data) -> Vec<Action> {
    let mut actions = Vec::new();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.heading("Rapports");
        ui.label("Générés par `edge market report`, `edge model report` et `edge backtest report` (ce dernier prend quelques secondes).");
        if data.reports.is_empty() {
            ui.label("Aucun rapport pour l'instant.");
            return;
        }
        Grid::new("reports").striped(true).num_columns(3).show(ui, |ui| {
            for title in ["Rapport", "Modifié", ""] {
                ui.label(RichText::new(title).strong());
            }
            ui.end_row();
            for report in &data.reports {
                ui.label(&report.name);
                ui.label(report.modified.map_or("—".to_string(), |t| {
                    let at: chrono::DateTime<chrono::Utc> = t.into();
                    format::age(at, data.now)
                }));
                if ui.button("Ouvrir").clicked() {
                    actions.push(Action::OpenReport(report.path.clone()));
                }
                ui.end_row();
            }
        });
    });
    actions
}
