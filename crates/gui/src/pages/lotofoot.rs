//! Écran « Loto Foot » : verdict du modèle, dernière séance (grilles et tickets à remplir), bilans, vérification.
//!
//! Tout vient de `edge_core::report::lotofoot_page::page`, la même photographie que la page `/lotofoot` du site ; le
//! bouton « J'ai joué la séance » fait ce que fait `edge lotofoot played --last-session`.

use edge_core::report::lotofoot_page::{GridView, LedgerView, LotofootPage, TicketView};
use eframe::egui::{RichText, Ui};

use super::common::page_header;
use super::{PageCtx, Request};
use crate::actions::Action;
use crate::data::Data;
use crate::design::icons::Icon;
use crate::design::motion::reveal;
use crate::design::style::Semibold;
use crate::design::tokens::space;
use crate::design::widgets::{
    ButtonKind, Tone, button, card, chip, empty_state, mono, pal, section_title, split, stat_tile,
    tile_columns,
};

/// Dessine l'écran.
pub fn show(ui: &mut Ui, ctx: &PageCtx, data: &Data) -> Vec<Request> {
    let mut requests = Vec::new();
    let motion = &ctx.motion;
    let page = &data.lotofoot;
    page_header(
        ui,
        motion,
        "Loto Foot",
        "LF7 et LF8, validés au point de vente : tickets simples à 1 €, joués si leur espérance défavorable atteint le seuil.",
    );
    ui.add_space(space::SM);

    reveal(ui, motion, 1, |ui| verdict(ui, page));
    ui.add_space(space::MD);

    let to_declare = page.tickets.iter().filter(|t| !t.played).count();
    reveal(ui, motion, 2, |ui| {
        let n = tile_columns(ui);
        ui.columns(n, |cols| {
            let session = if page.session.is_empty() {
                "aucune séance".to_string()
            } else {
                format!("séance du {}", page.session)
            };
            stat_tile(
                &mut cols[0],
                "lotofoot-grids",
                "Grilles",
                page.grids.len().to_string(),
                Tone::Neutral,
                |ui| {
                    ui.label(RichText::new(session).small().color(pal(ui).muted));
                },
            );
            let stake = if page.tickets.is_empty() {
                "0 €".to_string()
            } else {
                page.stake.clone()
            };
            stat_tile(
                &mut cols[1 % n],
                "lotofoot-tickets",
                "Tickets",
                page.tickets.len().to_string(),
                Tone::Accent,
                |ui| {
                    ui.label(
                        RichText::new(format!("mise de la séance : {stake}"))
                            .small()
                            .color(pal(ui).muted),
                    );
                },
            );
            stat_tile(
                &mut cols[2 % n],
                "lotofoot-to-declare",
                "À déclarer",
                to_declare.to_string(),
                if to_declare > 0 {
                    Tone::Warning
                } else {
                    Tone::Neutral
                },
                |ui| {
                    ui.label(
                        RichText::new("tickets pas encore déclarés joués")
                            .small()
                            .color(pal(ui).muted),
                    );
                },
            );
        });
    });

    ui.add_space(space::LG);
    let title = if page.session.is_empty() {
        "Dernière séance".to_string()
    } else {
        format!("Séance du {}", page.session)
    };
    reveal(ui, motion, 3, |ui| {
        section_title(ui, &title, None);
        ui.label(
            RichText::new("Les signes se lisent dans l'ordre des rencontres de la grille.")
                .small()
                .color(pal(ui).muted),
        );
        ui.add_space(space::XS);
        if to_declare > 0 {
            if button(ui, "J'ai joué la séance", ButtonKind::Primary)
                .on_hover_text(
                    "Déclare joués tous les tickets de la séance (comme `edge lotofoot played --last-session`). \
                     Un ticket déjà déclaré est ignoré, jamais doublé.",
                )
                .clicked()
            {
                requests.push(Request::Do(Action::LotofootPlayedSession));
            }
        } else if !page.tickets.is_empty() {
            chip(ui, "séance déclarée jouée", Tone::Positive);
        }
    });
    ui.add_space(space::SM);
    reveal(ui, motion, 4, |ui| session(ui, page));

    ui.add_space(space::LG);
    section_title(
        ui,
        "Bilan",
        Some("le rendement ne dit rien avant des années"),
    );
    ui.add_space(space::SM);
    card(ui, "lotofoot-ledgers", false, |ui| {
        for (index, ledger) in page.ledgers.iter().enumerate() {
            if index > 0 {
                ui.separator();
            }
            ledger_row(ui, ledger);
        }
    });

    ui.add_space(space::LG);
    section_title(
        ui,
        "Vérification du modèle",
        Some("critère figé à 26 et 52 semaines"),
    );
    ui.add_space(space::SM);
    verification(ui, page);
    requests
}

fn verdict_tone(class: &str) -> Tone {
    match class {
        "" => Tone::Positive,
        "bad" => Tone::Negative,
        _ => Tone::Warning,
    }
}

fn verdict(ui: &mut Ui, page: &LotofootPage) {
    card(ui, "lotofoot-verdict", false, |ui| {
        ui.horizontal_wrapped(|ui| {
            chip(ui, "Verdict", verdict_tone(page.verdict_class));
            ui.label(RichText::new(&page.verdict).color(pal(ui).text));
        });
    });
}

fn session(ui: &mut Ui, page: &LotofootPage) {
    let p = pal(ui);
    if page.tickets.is_empty() {
        card(ui, "lotofoot-empty", false, |ui| {
            if page.session.is_empty() {
                empty_state(
                    ui,
                    Icon::Grid,
                    "Aucune séance pour l'instant",
                    "Le relevé horaire archive les grilles ; la séance a lieu le samedi à midi.",
                );
            } else {
                empty_state(
                    ui,
                    Icon::Grid,
                    "Rien à jouer cette semaine",
                    "Aucun ticket ne passait le seuil.",
                );
            }
        });
    }
    for grid in &page.grids {
        grid_card(ui, page, grid);
        ui.add_space(space::XS);
    }
    for text in &page.skipped {
        ui.label(RichText::new(format!("Grille écartée : {text}")).color(p.warning));
    }
    if !page.unknown_names.is_empty() {
        ui.label(
            RichText::new(format!(
                "Noms absents de config/aliases.toml : {}",
                page.unknown_names.join(", ")
            ))
            .color(p.warning),
        );
    }
}

/// Une grille : ses rencontres numérotées, puis ses tickets.
fn grid_card(ui: &mut Ui, page: &LotofootPage, grid: &GridView) {
    let p = pal(ui);
    card(ui, ("lotofoot-grid", &grid.grid), false, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&grid.grid).semibold().size(16.0));
            chip(ui, &format!("clôture {}", grid.closes), Tone::Neutral);
        });
        ui.add_space(space::XS);
        for line in &grid.matches {
            ui.label(RichText::new(line).color(p.muted));
        }
        for ticket in page.tickets.iter().filter(|t| t.grid == grid.grid) {
            ui.separator();
            ticket_row(ui, ticket);
        }
    });
}

fn ticket_row(ui: &mut Ui, ticket: &TicketView) {
    let p = pal(ui);
    split(
        ui,
        170.0,
        |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(mono(&ticket.ticket).size(18.0).color(p.text));
                ui.label(
                    RichText::new(format!("n° {}", ticket.id))
                        .small()
                        .color(p.faint),
                );
            });
            ui.horizontal_wrapped(|ui| {
                chip(ui, &format!("espérance {}", ticket.ev), Tone::Accent);
                chip(ui, &format!("défavorable {}", ticket.ev_low), Tone::Neutral);
                chip(ui, &format!("P(payé) {}", ticket.paid), Tone::Neutral);
            });
        },
        |ui| {
            let tone = if ticket.status.starts_with("rang") {
                Tone::Positive
            } else if ticket.status.starts_with("perdu") {
                Tone::Negative
            } else {
                Tone::Neutral
            };
            chip(ui, &ticket.status, tone);
            if ticket.played {
                chip(ui, "joué", Tone::Positive);
            } else {
                chip(ui, "pas déclaré", Tone::Warning);
            }
        },
    );
}

fn ledger_row(ui: &mut Ui, ledger: &LedgerView) {
    let p = pal(ui);
    ui.label(RichText::new(ledger.label).semibold());
    ui.label(
        RichText::new(format!(
            "{} ticket(s), {} réglé(s) · mises {} · gains {} · rendement {} · payés {}",
            ledger.tickets, ledger.settled, ledger.staked, ledger.returned, ledger.roi, ledger.paid
        ))
        .small()
        .color(p.muted),
    );
}

fn verification(ui: &mut Ui, page: &LotofootPage) {
    let p = pal(ui);
    card(ui, "lotofoot-verification", false, |ui| {
        ui.label(RichText::new("Gain").semibold());
        ui.label(RichText::new(&page.gain).color(p.muted));
        ui.add_space(space::XS);
        ui.label(RichText::new("Fréquence").semibold());
        ui.label(RichText::new(&page.frequency).color(p.muted));
        ui.add_space(space::XS);
        ui.label(RichText::new("Calibration des concurrents").semibold());
        if page.bands.is_empty() {
            ui.label(
                RichText::new("Aucune grille réglée avec sa répartition finale pour l'instant.")
                    .color(p.muted),
            );
            return;
        }
        for band in &page.bands {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("λ {}", band.range)).color(p.text));
                ui.label(
                    RichText::new(format!(
                        "{} observation(s) · publiés / prédits {}",
                        band.observations, band.ratio
                    ))
                    .small()
                    .color(p.muted),
                );
                if band.failing {
                    chip(ui, "hors bornes", Tone::Negative);
                }
            });
        }
        ui.label(
            RichText::new("Bornes jugées : [0,8 ; 1,25] pour λ ≥ 10, à partir de 10 observations.")
                .small()
                .color(p.faint),
        );
    });
}
