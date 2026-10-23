//! Grilles de Grand Loto réellement jouées : déclaration (`edge loto played`) et règlement d'après l'archive FDJ.
//!
//! Une grille qui ne correspond à aucune recommandation de son édition est marquée hors modèle (invariant 7) et comptée
//! à part. Les gains des codes gagnants ne sont pas suivis : le code d'un reçu n'est pas connu de l'outil.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::Connection;

use super::{Game, Grid};
use crate::error::{Error, Result};
use crate::store::loto::{self as store, NewPlay, PlayRow};

/// Déclare jouées les grilles recommandées `ids` (une déclaration par grille). Renvoie les identifiants écrits.
pub fn declare_recos(conn: &Connection, ids: &[i64], now: DateTime<Utc>) -> Result<Vec<i64>> {
    let recos = store::recos(conn)?;
    let plays = store::plays(conn)?;
    let mut written = Vec::new();
    for &id in ids {
        let reco = recos
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| Error::Input(format!("aucune grille recommandée n°{id}")))?;
        if plays.iter().any(|p| p.reco_id == Some(id)) {
            return Err(Error::Input(format!(
                "la grille recommandée n°{id} est déjà déclarée jouée"
            )));
        }
        written.push(store::record_play(
            conn,
            &NewPlay {
                played_at: now,
                draw_date: reco.draw_date,
                grid: reco.grid,
                stake: reco.stake,
                reco_id: Some(id),
            },
        )?);
    }
    Ok(written)
}

/// Déclare jouées toutes les grilles de la dernière séance qui ne le sont pas encore.
pub fn declare_last_session(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<i64>> {
    let Some(last) = store::sessions(conn)?.pop() else {
        return Err(Error::Input("aucune séance du Grand Loto".to_string()));
    };
    let plays = store::plays(conn)?;
    let ids: Vec<i64> = store::recos(conn)?
        .iter()
        .filter(|r| r.session_id == last.id)
        .filter(|r| !plays.iter().any(|p| p.reco_id == Some(r.id)))
        .map(|r| r.id)
        .collect();
    if ids.is_empty() {
        return Err(Error::Input(format!(
            "la séance du Grand Loto du {} n'a aucune grille à déclarer",
            last.draw_date
        )));
    }
    declare_recos(conn, &ids, now)
}

/// Déclare une grille saisie à la main pour l'édition du `date`. Si elle correspond à une grille recommandée pour cette
/// édition et pas encore déclarée, elle lui est rattachée ; sinon elle est hors modèle. Renvoie l'identifiant écrit et
/// `true` si la grille est hors modèle.
pub fn declare_manual(
    conn: &Connection,
    date: NaiveDate,
    grid: Grid,
    stake: f64,
    now: DateTime<Utc>,
) -> Result<(i64, bool)> {
    if !(stake.is_finite() && stake > 0.0) {
        return Err(Error::Input(
            "la mise doit être strictement positive".to_string(),
        ));
    }
    let plays = store::plays(conn)?;
    let reco_id = store::recos(conn)?
        .iter()
        .find(|r| {
            r.draw_date == date && r.grid == grid && !plays.iter().any(|p| p.reco_id == Some(r.id))
        })
        .map(|r| r.id);
    let id = store::record_play(
        conn,
        &NewPlay {
            played_at: now,
            draw_date: date,
            grid,
            stake,
            reco_id,
        },
    )?;
    Ok((id, reco_id.is_none()))
}

/// Une grille jouée et son règlement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settled {
    /// La déclaration.
    pub play: PlayRow,
    /// Rang gagné (`None` : perdante, ou tirage pas encore connu).
    pub rank: Option<usize>,
    /// Gain, en euros (`None` : tirage pas encore dans l'archive).
    pub gain: Option<f64>,
}

/// Bilan des grilles jouées.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ledger {
    /// Toutes les grilles, dans l'ordre de déclaration.
    pub rows: Vec<Settled>,
}

impl Ledger {
    /// (mises, gains, grilles réglées) des grilles réglées, recommandées (`off_model = false`) ou hors modèle.
    pub fn totals(&self, off_model: bool) -> (f64, f64, usize) {
        self.rows
            .iter()
            .filter(|r| r.play.reco_id.is_none() == off_model)
            .filter_map(|r| r.gain.map(|gain| (r.play.stake, gain)))
            .fold((0.0, 0.0, 0), |(s, g, n), (stake, gain)| {
                (s + stake, g + gain, n + 1)
            })
    }
}

/// Règle les grilles jouées dont le tirage est dans l'archive.
pub fn ledger(conn: &Connection) -> Result<Ledger> {
    let rows = store::plays(conn)?
        .into_iter()
        .map(|play| {
            let draw = store::draw_on(conn, Game::GrandLoto, play.draw_date)?;
            Ok(match draw {
                None => Settled {
                    play,
                    rank: None,
                    gain: None,
                },
                Some(draw) => {
                    let rank = play.grid.rank_in(&draw);
                    let gain = rank
                        .and_then(|k| draw.ranks.get(k - 1))
                        .map(|r| r.unit_gain)
                        .unwrap_or(0.0);
                    Settled {
                        play,
                        rank,
                        gain: Some(gain),
                    }
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Ledger { rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loto::fdj::{self, tests::GRAND_LOTO_CSV};
    use crate::store::loto::{NewReco, NewSession};
    use crate::store::migrate::migrate;

    fn now() -> DateTime<Utc> {
        "2025-12-24T10:00:00Z".parse().unwrap()
    }

    fn christmas() -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 12, 25).unwrap()
    }

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let empty = serde_json::json!({});
        let recos = [
            NewReco {
                grid: Grid::parse("2-3-31-40-41+9").unwrap(),
                stake: 5.0,
                ev: 5.6,
                ev_pessimistic: 5.3,
            },
            NewReco {
                grid: Grid::parse("6-7-8-9-10+1").unwrap(),
                stake: 5.0,
                ev: 5.6,
                ev_pessimistic: 5.3,
            },
        ];
        store::record_session(
            &mut conn,
            &NewSession {
                created_at: now(),
                draw_date: christmas(),
                model_version: "grandloto-1",
                params: &empty,
                notes: &empty,
                recos: &recos,
            },
        )
        .unwrap();
        conn
    }

    #[test]
    fn the_last_session_is_declared_once() {
        let conn = database();
        assert_eq!(declare_last_session(&conn, now()).unwrap().len(), 2);
        assert!(
            declare_last_session(&conn, now()).is_err(),
            "rien de plus à déclarer"
        );
        assert!(declare_recos(&conn, &[1], now()).is_err(), "déjà déclarée");
        assert!(declare_recos(&conn, &[99], now()).is_err(), "inconnue");
    }

    #[test]
    fn a_manual_grid_is_matched_or_marked_off_model() {
        let conn = database();
        let (_, off) = declare_manual(
            &conn,
            christmas(),
            Grid::parse("2-3-31-40-41+9").unwrap(),
            5.0,
            now(),
        )
        .unwrap();
        assert!(!off, "grille recommandée");
        let (_, off) = declare_manual(
            &conn,
            christmas(),
            Grid::parse("1-2-3-4-5+6").unwrap(),
            5.0,
            now(),
        )
        .unwrap();
        assert!(off, "grille hors modèle");
        assert!(
            declare_manual(
                &conn,
                christmas(),
                Grid::parse("1-2-3-4-5+6").unwrap(),
                0.0,
                now()
            )
            .is_err()
        );
    }

    #[test]
    fn grids_are_settled_from_the_archive() {
        let mut conn = database();
        declare_last_session(&conn, now()).unwrap();
        let before = ledger(&conn).unwrap();
        assert!(
            before.rows.iter().all(|r| r.gain.is_none()),
            "tirage pas encore connu"
        );

        let fetch = store::record_fetch(&conn, now(), "test", 200, 1, None).unwrap();
        let draws = fdj::parse_csv(GRAND_LOTO_CSV, Game::GrandLoto).unwrap();
        store::record_draws(&mut conn, fetch, &draws).unwrap();
        let after = ledger(&conn).unwrap();
        // Tirage 2-3-30-35-44+10 : la première grille a deux numéros (rang 8, 36,20 €), la seconde rien.
        assert_eq!(after.rows[0].rank, Some(8));
        assert_eq!(after.rows[0].gain, Some(36.2));
        assert_eq!(after.rows[1].gain, Some(0.0));
        assert_eq!(after.totals(false), (10.0, 36.2, 2));
        assert_eq!(after.totals(true), (0.0, 0.0, 0));
    }
}
