//! Déclaration des tickets réellement joués (CLAUDE.md, invariant 7) : par numéro de recommandation, toute la dernière
//! séance, ou un ticket saisi à la main. Partagée par `edge lotofoot played` et l'écran Loto Foot de l'application.
//!
//! Un ticket recommandé n'est déclaré qu'une fois : le redéclarer est ignoré, jamais doublé. Un ticket saisi à la main est
//! rattaché à la recommandation de mêmes grille et signes s'il y en a une ; sinon il est hors modèle.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::{Game, Ticket};
use crate::domain::Euros;
use crate::error::{Error, Result};
use crate::store::lotofoot::{self as store, NewPlay, RecoRow};

/// Bilan d'une déclaration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    /// Recommandations déclarées jouées par cet appel.
    pub written: Vec<i64>,
    /// Recommandations déjà déclarées auparavant, ignorées.
    pub already: Vec<i64>,
}

/// Déclare jouées les recommandations données (numéros du journal). Un numéro inconnu est refusé avant toute écriture.
pub fn declare(
    conn: &Connection,
    ids: &[i64],
    now: DateTime<Utc>,
    stake: Euros,
) -> Result<Declared> {
    let recos = store::current_recos(conn)?;
    let chosen = ids
        .iter()
        .map(|id| {
            recos
                .iter()
                .find(|r| r.id == *id)
                .ok_or_else(|| Error::Input(format!("ticket recommandé n°{id} introuvable")))
        })
        .collect::<Result<Vec<&RecoRow>>>()?;
    write(conn, &chosen, now, stake)
}

/// Déclare joués tous les tickets en vigueur de la semaine de la dernière séance ([`store::week_recos`]).
pub fn declare_last_session(
    conn: &Connection,
    now: DateTime<Utc>,
    stake: Euros,
) -> Result<Declared> {
    if store::last_session(conn)?.is_none() {
        return Err(Error::Input("aucune séance au journal".to_string()));
    }
    // Les tickets en vigueur de la semaine : ceux du bouton du matin et du timer de midi se jouent ensemble.
    let recos = store::week_recos(conn)?;
    let chosen: Vec<&RecoRow> = recos.iter().collect();
    write(conn, &chosen, now, stake)
}

/// Déclare joués les tickets en vigueur d'une grille (bouton « J'ai joué cette grille »).
pub fn declare_grid(
    conn: &Connection,
    game: Game,
    number: u32,
    now: DateTime<Utc>,
    stake: Euros,
) -> Result<Declared> {
    let recos = store::current_recos(conn)?;
    let chosen: Vec<&RecoRow> = recos
        .iter()
        .filter(|r| (r.game, r.number) == (game, number))
        .collect();
    if chosen.is_empty() {
        return Err(Error::Input(format!(
            "aucun ticket en vigueur pour la grille {game} n°{number}"
        )));
    }
    write(conn, &chosen, now, stake)
}

fn write(
    conn: &Connection,
    chosen: &[&RecoRow],
    now: DateTime<Utc>,
    stake: Euros,
) -> Result<Declared> {
    let mut already: Vec<i64> = store::plays(conn)?
        .iter()
        .filter_map(|p| p.reco_id)
        .collect();
    let mut declared = Declared::default();
    for reco in chosen {
        if already.contains(&reco.id) {
            declared.already.push(reco.id);
            continue;
        }
        store::record_play(
            conn,
            &NewPlay {
                played_at: now,
                game: reco.game,
                number: reco.number,
                ticket: &reco.ticket,
                stake,
                reco_id: Some(reco.id),
            },
        )?;
        // Le même numéro passé deux fois dans un appel n'est écrit qu'une fois.
        already.push(reco.id);
        declared.written.push(reco.id);
    }
    Ok(declared)
}

/// Issue de la déclaration d'un ticket saisi à la main.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manual {
    /// Enregistré, rattaché à une recommandation (`Some`) ou hors modèle (`None`).
    Recorded(Option<i64>),
    /// Il correspond à une recommandation déjà déclarée jouée : ignoré.
    AlreadyDeclared(i64),
}

/// Déclare joué un ticket saisi à la main sur une grille relevée.
pub fn declare_manual(
    conn: &Connection,
    game: Game,
    number: u32,
    ticket: &Ticket,
    now: DateTime<Utc>,
    stake: Euros,
) -> Result<Manual> {
    if ticket.len() != game.matches() {
        return Err(Error::Input(format!(
            "{game} : {} signes attendus, {} donnés",
            game.matches(),
            ticket.len()
        )));
    }
    if store::grid(conn, game, number)?.is_none() {
        return Err(Error::Input(format!(
            "{game} n°{number} inconnue : elle n'a jamais été relevée"
        )));
    }
    let reco_id = store::current_recos(conn)?
        .iter()
        .find(|r| r.game == game && r.number == number && r.ticket == *ticket)
        .map(|r| r.id);
    if let Some(id) = reco_id
        && store::plays(conn)?.iter().any(|p| p.reco_id == Some(id))
    {
        return Ok(Manual::AlreadyDeclared(id));
    }
    store::record_play(
        conn,
        &NewPlay {
            played_at: now,
            game,
            number,
            ticket,
            stake,
            reco_id,
        },
    )?;
    Ok(Manual::Recorded(reco_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lotofoot::pronomatch::{GridPage, OpenGrid};
    use crate::store::lotofoot::{
        NewReco, NewSession, Page, record_fetch, record_grid_page, record_session,
    };
    use crate::store::migrate::migrate;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// Une grille LF7 relevée et une séance de deux tickets.
    fn journal() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        let page = GridPage {
            open: Some(OpenGrid {
                game: Game::Lf7,
                accepting: true,
                number: 133,
                closes_at: at("2026-10-25T16:10:00Z"),
                jackpot: None,
                matches: (1..=7)
                    .map(|i| (format!("H{i}"), format!("A{i}")))
                    .collect(),
                split: Some(vec![[50, 30, 20]; 7]),
                split_at: None,
            }),
            history: vec![],
        };
        record_grid_page(&mut conn, f, Game::Lf7, &page).unwrap();
        let tickets = [
            Ticket::parse("1N21N21").unwrap(),
            Ticket::parse("2N21N21").unwrap(),
        ];
        let recos: Vec<NewReco<'_>> = tickets
            .iter()
            .enumerate()
            .map(|(i, ticket)| NewReco {
                game: Game::Lf7,
                number: 133,
                ticket,
                position: i + 1,
                ev_base: 1.8,
                ev_pessimistic: 1.4,
                paid_probability: 0.02,
                stakes_estimate: 200_000.0,
                split_fetch_id: f,
                inputs: "{}",
            })
            .collect();
        let session = NewSession {
            created_at: at("2026-10-24T10:00:00Z"),
            model_version: "lotofoot-1",
            params: "{}",
            notes: "{}",
        };
        record_session(&mut conn, &session, &recos).unwrap();
        conn
    }

    fn one() -> Euros {
        Euros::new(1.0).unwrap()
    }

    #[test]
    fn a_session_is_declared_once_even_if_declared_again() {
        let conn = journal();
        let now = at("2026-10-24T14:00:00Z");
        let first = declare_last_session(&conn, now, one()).unwrap();
        assert_eq!(first.written.len(), 2);
        let again = declare_last_session(&conn, now, one()).unwrap();
        assert!(again.written.is_empty());
        assert_eq!(again.already.len(), 2);
        assert_eq!(store::plays(&conn).unwrap().len(), 2);
    }

    #[test]
    fn unknown_numbers_are_refused_before_writing() {
        let conn = journal();
        let ids: Vec<i64> = store::current_recos(&conn)
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        assert!(declare(&conn, &[ids[0], 999], at("2026-10-24T14:00:00Z"), one()).is_err());
        assert!(store::plays(&conn).unwrap().is_empty());
        // Le même numéro deux fois : une seule écriture.
        let d = declare(&conn, &[ids[0], ids[0]], at("2026-10-24T14:00:00Z"), one()).unwrap();
        assert_eq!((d.written.len(), d.already.len()), (1, 1));
    }

    #[test]
    fn a_manual_ticket_is_linked_or_off_model() {
        let conn = journal();
        let now = at("2026-10-24T14:00:00Z");
        let recommended = Ticket::parse("1N21N21").unwrap();
        let Manual::Recorded(Some(id)) =
            declare_manual(&conn, Game::Lf7, 133, &recommended, now, one()).unwrap()
        else {
            panic!("le ticket recommandé doit être rattaché");
        };
        assert_eq!(
            declare_manual(&conn, Game::Lf7, 133, &recommended, now, one()).unwrap(),
            Manual::AlreadyDeclared(id)
        );
        let other = Ticket::parse("1111111").unwrap();
        assert_eq!(
            declare_manual(&conn, Game::Lf7, 133, &other, now, one()).unwrap(),
            Manual::Recorded(None)
        );
        assert!(declare_manual(&conn, Game::Lf7, 999, &other, now, one()).is_err());
        assert!(declare_manual(&conn, Game::Lf8, 133, &other, now, one()).is_err());
    }
}
