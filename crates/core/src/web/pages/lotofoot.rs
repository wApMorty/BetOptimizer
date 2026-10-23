//! Onglet « Loto Foot » (`/lotofoot`) : verdict du modèle, dernière séance (grilles à remplir et tickets), bilans,
//! vérification.
//!
//! Tout vient de [`crate::report::lotofoot_page::page`] ; ce module ajoute la présentation en bulletin : une ligne par
//! rencontre, une colonne par grille à remplir (simple ou multiple, `docs/sources.md` §39), les cases à cocher dans
//! chaque colonne ; puis le détail des tickets de la grille (espérance, règlement).

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::Result;
use crate::report::lotofoot_page::{self, LineView, LotofootPage, RankView, TicketView};
use crate::store;
use crate::web::App;
use crate::web::view::{Flash, Layout, render};

const TEMPLATE: &str = include_str!("../templates/lotofoot.html");

/// Une grille à remplir, en tête et en pied de sa colonne.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Column {
    price: String,
    /// Numéros des tickets contenus (« n° 12, 13 »).
    tickets: String,
    /// Chance que la grille paie (« 1 sur 40 ») et gain moyen quand elle paie.
    chance: String,
    gain: String,
    played: bool,
}

/// Une rencontre et les cases à cocher dans chaque grille à remplir (une case par signe : deux pour un double).
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Row {
    fixture: String,
    cells: Vec<Vec<String>>,
}

/// Un ticket de la grille, pour le détail.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Detail {
    id: i64,
    ticket: String,
    played: bool,
    status: String,
    /// Tonalité du règlement (`t-pos` pour un rang payé, `t-neg` pour perdu).
    tone: &'static str,
    ev: String,
    ev_low: String,
    /// Chance d'être payé, tous rangs confondus.
    chance: String,
    /// Chance et gain moyen de chaque rang.
    ranks: Vec<RankView>,
}

/// Une grille du Loto Foot présentée en bulletin.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Bulletin {
    grid: String,
    closes: String,
    columns: Vec<Column>,
    rows: Vec<Row>,
    details: Vec<Detail>,
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
            let lines: Vec<&LineView> = lotofoot
                .lines
                .iter()
                .filter(|l| l.grid == grid.grid)
                .collect();
            Bulletin {
                grid: grid.grid.clone(),
                closes: grid.closes.clone(),
                columns: lines.iter().map(|l| column(l)).collect(),
                rows: grid
                    .matches
                    .iter()
                    .enumerate()
                    .map(|(index, fixture)| Row {
                        fixture: fixture.clone(),
                        cells: lines
                            .iter()
                            .map(|l| l.signs.get(index).cloned().unwrap_or_default())
                            .collect(),
                    })
                    .collect(),
                details: lotofoot
                    .tickets
                    .iter()
                    .filter(|t| t.grid == grid.grid)
                    .map(detail)
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

fn column(line: &LineView) -> Column {
    let ids: Vec<String> = line.tickets.iter().map(i64::to_string).collect();
    Column {
        price: line.price.clone(),
        tickets: format!("n° {}", ids.join(", ")),
        chance: line.chance.clone(),
        gain: line.gain.clone(),
        played: line.played,
    }
}

fn detail(ticket: &TicketView) -> Detail {
    let tone = if ticket.status.starts_with("rang") {
        "t-pos"
    } else if ticket.status.starts_with("perdu") {
        "t-neg"
    } else {
        ""
    };
    Detail {
        id: ticket.id,
        ticket: ticket.ticket.clone(),
        played: ticket.played,
        status: ticket.status.clone(),
        tone,
        ev: ticket.ev.clone(),
        ev_low: ticket.ev_low.clone(),
        chance: ticket.chance.clone(),
        ranks: ticket.ranks.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::web::testing::{empty, sample};

    #[test]
    fn the_session_is_shown_as_a_bulletin_with_one_column_per_grid_to_fill() {
        let html = sample().html("/lotofoot");
        assert!(html.contains("LF7 n°133"), "{html}");
        // Deux tickets, `1N21N21` et `2N21N21`, qui ne diffèrent que sur la première rencontre : une seule grille à
        // remplir, un double 12 puis six simples, soit huit cases cochées pour sept rencontres.
        assert_eq!(html.matches("class=\"sign s-").count(), 8, "{html}");
        assert!(html.contains("2 €"), "{html}");
        // Le détail garde chaque ticket.
        assert!(
            html.contains("1N21N21") && html.contains("2N21N21"),
            "{html}"
        );
        assert!(html.contains("J'ai joué la séance"));
        let html = empty().html("/lotofoot");
        assert!(html.contains("Aucune séance pour l'instant"));
    }
}
