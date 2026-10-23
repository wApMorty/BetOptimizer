//! Onglet « Loto Foot » (`/lotofoot`) : verdict du modèle, dernière séance (grilles à remplir et tickets), bilans,
//! vérification.
//!
//! Tout vient de [`crate::report::lotofoot_page::page`] ; ce module ajoute la présentation en bulletin : une ligne par
//! rencontre, une colonne par grille à remplir (simple ou multiple, `docs/sources.md` §39), les cases à cocher dans
//! chaque colonne ; puis le détail des tickets de la grille (espérance, règlement). Le bouton « Optimiser maintenant »
//! lance la séance comme `edge lotofoot recommend` ([`crate::lotofoot::live`]), avec la mise plafond saisie.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::Result;
use crate::lotofoot::bulletin::session_lines;
use crate::lotofoot::live::{self, Live, Notified};
use crate::lotofoot::session::Request;
use crate::report::lotofoot_page::{self, LineView, LotofootPage, RankView, TicketView};
use crate::store;
use crate::web::App;
use crate::web::form::Form;
use crate::web::pages::entry::Reply;
use crate::web::view::{Flash, Layout, render};

const TEMPLATE: &str = include_str!("../templates/lotofoot.html");

/// Une grille à remplir, en tête et en pied de sa colonne.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Column {
    price: String,
    /// Numéros des tickets contenus (« n° 12, 13 »).
    tickets: String,
    played: bool,
}

/// Un rang en pied de bulletin (« Sans faute ») : pour chaque grille à remplir, sa chance et son gain moyen.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct RankRow {
    label: &'static str,
    /// `None` si les entrées du modèle manquent pour cette grille.
    cells: Vec<Option<RankView>>,
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
    ranks: Vec<RankRow>,
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
    /// Mise plafond du formulaire « Optimiser maintenant » (`lotofoot_max_tickets`, 16 € gelés).
    max_stake: u32,
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
                ranks: rank_rows(&lines),
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
            max_stake: app.config.lotofoot_max_tickets,
        },
    )
}

/// Bouton « Optimiser maintenant » : mise plafond saisie (champ `mise`, en euros entiers), message Discord si la case
/// `discord` est cochée. Le résultat revient en message sur l'onglet, qui montre alors la nouvelle séance.
pub fn optimize(app: &App, form: &Form) -> Reply {
    let flash = match stake(form.get("mise"), app.config.lotofoot_max_tickets) {
        Err(message) => Flash::error(message),
        Ok(max_tickets) => match run_now(app, max_tickets, form.has("discord")) {
            Ok(message) => Flash::ok(message),
            Err(error) => Flash::error(format!("séance non faite : {error}")),
        },
    };
    Reply::Done {
        to: "/lotofoot",
        flash,
    }
}

/// Mise saisie : un nombre entier d'euros entre 1 et le plafond de la configuration.
fn stake(text: &str, cap: u32) -> std::result::Result<u32, String> {
    match text.trim().parse::<u32>() {
        Ok(euros) if (1..=cap).contains(&euros) => Ok(euros),
        _ => Err(format!(
            "mise attendue : un nombre entier d'euros entre 1 et {cap}"
        )),
    }
}

fn run_now(app: &App, max_tickets: u32, notify: bool) -> Result<String> {
    let mut conn = store::open_migrated(&app.paths.db_file)?;
    // Le bouton recalcule aussi les grilles déjà recommandées (pas encore jouées) : la répartition a pu bouger.
    let request = Request {
        max_tickets,
        replace: true,
        dry_run: false,
    };
    let live = live::recommend(&mut conn, &app.config, &app.aliases, request, notify)?;
    Ok(summary(&live))
}

/// Message de fin de séance : tickets et grilles à remplir, grilles écartées, relevé et Discord.
fn summary(live: &Live) -> String {
    let report = &live.report;
    let mut parts = Vec::new();
    match report.session_id {
        Some(id) => {
            let keyed: Vec<_> = report.chosen.iter().map(|c| (c.grid, &c.ticket)).collect();
            parts.push(format!(
                "Séance n°{id} : {} ticket(s), soit {} €, en {} grille(s) à remplir.",
                report.chosen.len(),
                report.chosen.len(),
                session_lines(&keyed).len()
            ));
        }
        None => parts.push("Aucun nouveau ticket : rien n'est écrit au journal.".to_string()),
    }
    if !report.skipped.is_empty() {
        let skipped: Vec<String> = report
            .skipped
            .iter()
            .map(|s| format!("{} ({})", s.grid, s.reason))
            .collect();
        parts.push(format!("Écartée(s) : {}.", skipped.join(" ; ")));
    }
    if !live.fetched.failures.is_empty() {
        parts.push(format!(
            "Relevé de prono-match incomplet : {} page(s) non lue(s).",
            live.fetched.failures.len()
        ));
    }
    match &live.notified {
        Notified::NotAsked => {}
        Notified::Sent => parts.push("Envoyée sur Discord.".to_string()),
        Notified::Failed(error) => parts.push(format!("Message Discord non envoyé : {error}.")),
    }
    parts.join(" ")
}

/// Lignes « Sans faute » et « Une erreur » du pied de bulletin, une case par grille à remplir.
fn rank_rows(lines: &[&LineView]) -> Vec<RankRow> {
    // Toutes les grilles d'une même formule ont le même nombre de rangs : la plus longue liste donne les libellés.
    let Some(longest) = lines.iter().map(|l| &l.ranks).max_by_key(|r| r.len()) else {
        return Vec::new();
    };
    longest
        .iter()
        .enumerate()
        .map(|(k, rank)| RankRow {
            label: rank.label,
            cells: lines.iter().map(|l| l.ranks.get(k).cloned()).collect(),
        })
        .collect()
}

fn column(line: &LineView) -> Column {
    let ids: Vec<String> = line.tickets.iter().map(i64::to_string).collect();
    Column {
        price: line.price.clone(),
        tickets: format!("n° {}", ids.join(", ")),
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
        // Le bouton est là même sans séance, avec la mise plafond de la configuration.
        assert!(html.contains("action=\"/lotofoot/optimiser\""), "{html}");
        assert!(html.contains("value=\"16\""), "{html}");
    }

    #[test]
    fn the_stake_must_be_whole_euros_within_the_cap() {
        assert_eq!(super::stake("8", 16), Ok(8));
        assert_eq!(super::stake(" 16 ", 16), Ok(16));
        for wrong in ["0", "17", "2,5", "", "huit"] {
            assert!(super::stake(wrong, 16).is_err(), "{wrong}");
        }
    }
}
