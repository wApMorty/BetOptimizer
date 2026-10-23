//! Alerte « nouvelle grille » : prévenir Paul sur Discord de chaque grille LF7 ou LF8 qui sort, avec son heure de clôture,
//! sans aucun ticket (il est trop tôt : la répartition des autres joueurs n'est pas encore représentative, §47).
//!
//! Une grille est à signaler tant qu'elle n'est pas close et que son alerte n'est pas notée au journal
//! (`lotofoot_alert`). L'alerte n'est notée qu'après l'envoi réussi : un échec est retenté au relevé suivant, et peu importe
//! quel relevé (horaire, séance, bouton) a vu la grille en premier.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::Game;
use crate::error::Result;
use crate::store::lotofoot as store;

/// Une grille à signaler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Formule.
    pub game: Game,
    /// Numéro.
    pub number: u32,
    /// Fin de validation, en UTC.
    pub closes_at: DateTime<Utc>,
    /// Pactole annoncé, en euros entiers.
    pub jackpot: Option<u32>,
    /// La répartition FDJ est déjà publiée (sinon la grille est seulement annoncée, ou vient d'ouvrir).
    pub split_published: bool,
}

/// Grilles encore ouvertes à `now` dont l'alerte n'a pas été envoyée, dans l'ordre de clôture.
pub fn pending(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<Listed>> {
    let mut listed = Vec::new();
    for row in store::grids_closing_after(conn, now)? {
        if store::alerted(conn, row.game, row.number)? {
            continue;
        }
        listed.push(Listed {
            game: row.game,
            number: row.number,
            closes_at: row.closes_at,
            // Un pactole est un montant rond en euros : l'arrondi ne perd rien.
            jackpot: row.jackpot.map(|j| j.get().round() as u32),
            split_published: store::latest_split(conn, row.game, row.number, now)?.is_some(),
        });
    }
    Ok(listed)
}

/// Note les alertes comme envoyées (à appeler seulement après un envoi réussi).
pub fn record(conn: &Connection, listed: &[Listed], sent_at: DateTime<Utc>) -> Result<()> {
    for grid in listed {
        store::record_alert(conn, grid.game, grid.number, sent_at)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Euros;
    use crate::lotofoot::pronomatch::{GridPage, OpenGrid};
    use crate::store::lotofoot::{Page, record_fetch, record_grid_page};
    use crate::store::migrate::migrate;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Une grille annoncée, sans répartition.
    fn announce(conn: &mut Connection, game: Game, number: u32, closes: &str) {
        let fetch = record_fetch(
            conn,
            at("2026-10-24T08:07:00Z"),
            Page::Grid(game),
            200,
            None,
        )
        .unwrap();
        let page = GridPage {
            open: Some(OpenGrid {
                game,
                accepting: false,
                number,
                closes_at: at(closes),
                jackpot: Euros::new(150_000.0),
                matches: (1..=game.matches())
                    .map(|i| (format!("H{i}"), format!("A{i}")))
                    .collect(),
                split: None,
                split_at: None,
            }),
            history: vec![],
        };
        record_grid_page(conn, fetch, game, &page).unwrap();
    }

    #[test]
    fn an_open_grid_is_signalled_once_and_a_closed_one_never() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        announce(&mut conn, Game::Lf8, 147, "2026-10-26T17:55:00Z");
        announce(&mut conn, Game::Lf7, 132, "2026-10-22T18:00:00Z");
        let now = at("2026-10-24T09:00:00Z");
        let first = pending(&conn, now).unwrap();
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(
            (first[0].game, first[0].number, first[0].jackpot),
            (Game::Lf8, 147, Some(150_000))
        );
        assert!(!first[0].split_published);
        // Rien n'est noté tant que l'envoi n'a pas réussi : la grille reste à signaler.
        assert_eq!(pending(&conn, now).unwrap(), first);
        record(&conn, &first, now).unwrap();
        assert!(pending(&conn, now).unwrap().is_empty());
        // Noter deux fois la même alerte ne fait rien.
        record(&conn, &first, now).unwrap();
    }
}
