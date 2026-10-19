//! Écran « Rapports » : les rapports HTML générés par la ligne de commande, et ce que l'archive contient.

use edge_core::format;
use eframe::egui::{Align, Layout, RichText, Ui};

use super::common::page_header;
use super::{PageCtx, Request};
use crate::actions::Action;
use crate::data::Data;
use crate::design::icons::Icon;
use crate::design::motion::reveal;
use crate::design::style::Semibold;
use crate::design::tokens::space;
use crate::design::widgets::{
    ButtonKind, Tone, button, card, chip, empty_state, pal, section_title,
};

/// Dessine l'écran.
pub fn show(ui: &mut Ui, ctx: &PageCtx, data: &Data) -> Vec<Request> {
    let mut requests = Vec::new();
    let motion = &ctx.motion;
    let p = pal(ui);
    page_header(
        ui,
        motion,
        "Rapports",
        "Générés par la ligne de commande : `edge backtest report`, `edge market report`, `edge model report`, `edge tennis report`, `edge nba report`.",
    );
    ui.add_space(space::SM);

    reveal(ui, motion, 1, |ui| {
        section_title(ui, "Rapports HTML", None);
        ui.add_space(space::XS);
        if data.reports.is_empty() {
            card(ui, "reports-empty", false, |ui| {
                empty_state(
                    ui,
                    Icon::Doc,
                    "Aucun rapport pour l'instant",
                    "Lance par exemple `edge backtest report` dans un terminal.",
                );
            });
        }
        for (index, report) in data.reports.iter().enumerate() {
            card(ui, ("report", index), true, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&report.name).semibold());
                        let age = report.modified.map(|t| {
                            let at: chrono::DateTime<chrono::Utc> = t.into();
                            format::age(at, data.now)
                        });
                        ui.label(
                            RichText::new(age.unwrap_or_else(|| "date inconnue".to_string()))
                                .small()
                                .color(p.muted),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if button(ui, "Ouvrir", ButtonKind::Secondary).clicked() {
                            requests.push(Request::Do(Action::OpenReport(report.path.clone())));
                        }
                    });
                });
            });
            ui.add_space(space::XS);
        }
    });

    ui.add_space(space::MD);
    reveal(ui, motion, 2, |ui| {
        section_title(
            ui,
            "Données archivées",
            Some("ce que les relevés ont déjà ramassé"),
        );
        ui.add_space(space::XS);
        card(ui, "feeds", false, |ui| {
            for feed in &data.feeds {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(feed.label).semibold());
                    if let Some(count) = feed.archived {
                        let unit = if feed.label == "Football" {
                            "matchs d'historique"
                        } else if feed.label == "Tennis" {
                            "matchs importés"
                        } else {
                            "matchs terminés"
                        };
                        chip(ui, &format!("{count} {unit}"), Tone::Neutral);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| match feed.last {
                        Some(at) => {
                            ui.label(
                                RichText::new(format!(
                                    "dernier relevé {}",
                                    format::age(at, data.now)
                                ))
                                .small()
                                .color(p.muted),
                            );
                        }
                        None => {
                            ui.label(RichText::new("aucun relevé").small().color(p.faint));
                        }
                    });
                });
            }
        });
    });
    requests
}
