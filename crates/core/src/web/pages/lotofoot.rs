//! Onglet « Loto Foot » (`/lotofoot`) : verdict du modèle, dernière séance (grilles et tickets à remplir), bilans,
//! vérification.
//!
//! Tout vient de [`crate::report::lotofoot_page::page`] ; ce module ajoute la présentation en bulletin : une ligne par
//! rencontre, une colonne par ticket, le signe à cocher dans chaque case.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::Result;
use crate::report::lotofoot_page::{self, LotofootPage, TicketView};
use crate::store;
use crate::web::App;
use crate::web::view::{Flash, Layout, render};

const TEMPLATE: &str = include_str!("../templates/lotofoot.html");

/// Un ticket, en tête de sa colonne.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Column {
    id: i64,
    played: bool,
    status: String,
    /// Tonalité du règlement (`t-pos` pour un rang payé, `t-neg` pour perdu).
    tone: &'static str,
    ev: String,
    ev_low: String,
    paid: String,
}

/// Une rencontre et le signe de chaque ticket.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Line {
    fixture: String,
    signs: Vec<String>,
}

/// Une grille présentée en bulletin.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Bulletin {
    grid: String,
    closes: String,
    columns: Vec<Column>,
    lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Page {
    #[serde(flatten)]
    layout: Layout,
    #[serde(flatten)]
    lotofoot: LotofootPage,
    bulletins: Vec<Bulletin>,
    to_declare: usize,
}

/// La page.
pub fn page(app: &App, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let lotofoot = lotofoot_page::page(&conn, now)?;
    let bulletins = lotofoot
        .grids
        .iter()
        .map(|grid| {
            let tickets: Vec<&TicketView> = lotofoot
                .tickets
                .iter()
                .filter(|t| t.grid == grid.grid)
                .collect();
            Bulletin {
                grid: grid.grid.clone(),
                closes: grid.closes.clone(),
                columns: tickets.iter().map(|t| column(t)).collect(),
                lines: grid
                    .matches
                    .iter()
                    .enumerate()
                    .map(|(index, fixture)| Line {
                        fixture: fixture.clone(),
                        // Le i-ième caractère du ticket est le signe de la i-ième rencontre (`1`, `N` ou `2`).
                        signs: tickets
                            .iter()
                            .map(|t| {
                                t.ticket
                                    .chars()
                                    .nth(index)
                                    .map_or(String::new(), String::from)
                            })
                            .collect(),
                    })
                    .collect(),
            }
        })
        .collect();
    let to_declare = lotofoot.tickets.iter().filter(|t| !t.played).count();
    render(
        "lotofoot.html",
        TEMPLATE,
        &Page {
            layout: Layout::new("lotofoot", now, flash),
            lotofoot,
            bulletins,
            to_declare,
        },
    )
}

fn column(ticket: &TicketView) -> Column {
    let tone = if ticket.status.starts_with("rang") {
        "t-pos"
    } else if ticket.status.starts_with("perdu") {
        "t-neg"
    } else {
        ""
    };
    Column {
        id: ticket.id,
        played: ticket.played,
        status: ticket.status.clone(),
        tone,
        ev: ticket.ev.clone(),
        ev_low: ticket.ev_low.clone(),
        paid: ticket.paid.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::web::testing::{empty, sample};

    #[test]
    fn the_session_is_shown_as_a_bulletin_with_one_column_per_ticket() {
        let html = sample().html("/lotofoot");
        assert!(html.contains("LF7 n°133"), "{html}");
        // Deux tickets : deux colonnes ; sept rencontres : sept lignes de deux cases.
        assert_eq!(html.matches("class=\"sign s-").count(), 14, "{html}");
        assert!(html.contains("J'ai joué la séance"));
        let html = empty().html("/lotofoot");
        assert!(html.contains("Aucune séance pour l'instant"));
    }
}
