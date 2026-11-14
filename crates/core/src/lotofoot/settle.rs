//! Règlement des tickets à partir des résultats et des rapports officiels relevés.
//!
//! - Ticket **joué** : il touche le gain unitaire publié du rang atteint (la FDJ compte sa grille parmi les gagnants).
//! - Ticket **recommandé** (suivi du modèle, qu'il ait été joué ou non) : il aurait touché la masse du rang partagée avec
//!   les gagnants publiés, soit `gain unitaire × W / (W + 1)` ; s'il n'y a aucun gagnant publié, la masse entière,
//!   estimée à partir des mises estimées le jour de la séance et du pactole.
//!
//! Rien n'est stocké : le règlement se recalcule à la demande, à partir du journal immuable.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::Connection;

use super::model::GridModel;
use super::session::{budget_since, month_of};
use super::{Game, Ticket};
use crate::domain::{Euros, Outcome};
use crate::error::Result;
use crate::store::lotofoot::{self as store, PublishedRank, RecoRow};

/// Règlement d'un ticket.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settled {
    /// Bons pronostics.
    pub good: usize,
    /// Rang atteint (1 : sans-faute), `None` hors des rangs payés.
    pub rank: Option<u32>,
    /// Gain en euros (mise non comprise).
    pub gain: f64,
}

/// Rang atteint par un ticket : 1 pour le sans-faute, 2 pour une erreur, `None` au-delà des rangs payés.
fn rank_of(game: Game, good: usize) -> Option<u32> {
    let errors = game.matches().checked_sub(good)?;
    (errors < game.rank_shares().len()).then(|| u32::try_from(errors + 1).unwrap_or(u32::MAX))
}

/// Règlement d'un ticket **joué** ; `None` tant que les résultats ou les rapports ne sont pas connus.
pub fn played(
    game: Game,
    ticket: &Ticket,
    results: &[Outcome],
    ranks: &[PublishedRank],
) -> Option<Settled> {
    let good = ticket.good(results);
    let rank = rank_of(game, good);
    let gain = match rank {
        None => 0.0,
        Some(r) => ranks
            .iter()
            .find(|p| p.rank == r)?
            .unit_gain
            .map_or(0.0, Euros::get),
    };
    Some(Settled { good, rank, gain })
}

/// Règlement d'un ticket **recommandé**, comme s'il avait été joué seul. `None` tant que les rapports manquent.
pub fn paper(
    game: Game,
    ticket: &Ticket,
    results: &[Outcome],
    ranks: &[PublishedRank],
    stakes_estimate: f64,
    jackpot: Option<Euros>,
) -> Option<Settled> {
    let good = ticket.good(results);
    let rank = rank_of(game, good);
    let gain = match rank {
        None => 0.0,
        Some(r) => {
            let published = ranks.iter().find(|p| p.rank == r)?;
            match published.unit_gain {
                Some(unit) if published.winners > 0 => {
                    let w = f64::from(published.winners);
                    unit.get() * w / (w + 1.0)
                }
                _ => {
                    let masses = GridModel::masses_for(game, Euros::new(stakes_estimate)?, jackpot);
                    masses.get(usize::try_from(r - 1).ok()?)?.get()
                }
            }
        }
    };
    Some(Settled { good, rank, gain })
}

/// Bilan d'un ensemble de tickets.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ledger {
    /// Tickets de l'ensemble.
    pub tickets: usize,
    /// Tickets réglés (résultats et rapports connus).
    pub settled: usize,
    /// Mises des tickets réglés.
    pub staked: f64,
    /// Gains des tickets réglés.
    pub returned: f64,
    /// Tickets réglés qui ont atteint un rang payé.
    pub paid: usize,
    /// Somme des probabilités de gain annoncées des tickets réglés (nombre de tickets payés attendu).
    pub expected_paid: f64,
    /// Somme des espérances annoncées des tickets réglés (gains attendus, mises comprises).
    pub expected_return: f64,
}

impl Ledger {
    /// Rendement : gains / mises − 1 ; `None` sans ticket réglé.
    pub fn roi(&self) -> Option<f64> {
        (self.staked > 0.0).then(|| self.returned / self.staked - 1.0)
    }
}

/// Bilan des tickets recommandés (suivi du modèle).
pub fn recommended(conn: &Connection) -> Result<Ledger> {
    let mut ledger = Ledger::default();
    for reco in store::current_recos(conn)? {
        ledger.tickets += 1;
        let Some(results) = store::results(conn, reco.grid)? else {
            continue;
        };
        let ranks = store::ranks(conn, reco.grid)?;
        let jackpot = store::grid(conn, reco.grid)?.and_then(|g| g.jackpot);
        let Some(settled) = paper(
            reco.grid.game,
            &reco.ticket,
            &results,
            &ranks,
            reco.stakes_estimate,
            jackpot,
        ) else {
            continue;
        };
        ledger.settled += 1;
        ledger.staked += super::TICKET_PRICE;
        ledger.returned += settled.gain;
        ledger.paid += usize::from(settled.rank.is_some());
        ledger.expected_paid += reco.paid_probability;
        ledger.expected_return += reco.ev_base * super::TICKET_PRICE;
    }
    Ok(ledger)
}

/// Bilan des tickets réellement joués, séparé en tickets du modèle et tickets hors modèle (CLAUDE.md, invariant 7).
pub fn played_ledgers(conn: &Connection) -> Result<(Ledger, Ledger)> {
    let mut model = Ledger::default();
    let mut off = Ledger::default();
    for play in store::plays(conn)? {
        let ledger = if play.reco_id.is_some() {
            &mut model
        } else {
            &mut off
        };
        ledger.tickets += 1;
        let Some(results) = store::results(conn, play.grid)? else {
            continue;
        };
        let ranks = store::ranks(conn, play.grid)?;
        let Some(settled) = played(play.grid.game, &play.ticket, &results, &ranks) else {
            continue;
        };
        ledger.settled += 1;
        ledger.staked += play.stake;
        // Une grille jouée avec une mise unitaire de 2 € ou 5 € touche le gain unitaire autant de fois.
        ledger.returned += settled.gain * play.stake / super::TICKET_PRICE;
        ledger.paid += usize::from(settled.rank.is_some());
    }
    Ok((model, off))
}

/// Gains recyclés dans le budget de `month`, en euros entiers (§67) : ceux que connaît une séance de l'instant `at`.
///
/// Comptent les tickets du modèle déclarés joués avant `at`, dont la mise a compté dans le budget (`recos` : recommandations
/// en vigueur, écrites depuis [`budget_since`]), sur les grilles qui ferment ce mois-là. Un gain reste donc rattaché au mois
/// de sa mise, et les tickets hors modèle n'y entrent pas (CLAUDE.md, invariant 7). Une grille dont la composition a changé
/// après la déclaration n'est pas réglée automatiquement : elle ne recycle rien.
pub fn recycled_gains(
    conn: &Connection,
    recos: &[RecoRow],
    month: NaiveDate,
    at: DateTime<Utc>,
) -> Result<u32> {
    let since = budget_since();
    let mut total = 0.0;
    for play in store::plays(conn)? {
        let counted = recos
            .iter()
            .any(|r| Some(r.id) == play.reco_id && r.created_at >= since);
        if !counted || play.played_at > at {
            continue;
        }
        let Some(grid) = store::grid(conn, play.grid)? else {
            continue;
        };
        if month_of(grid.closes_at) != month
            || store::published_after(conn, play.grid, at)?
            || store::recomposition(conn, play.grid)?.is_some_and(|c| c.played_before)
        {
            continue;
        }
        let Some(results) = store::results(conn, play.grid)? else {
            continue;
        };
        let ranks = store::ranks(conn, play.grid)?;
        let Some(settled) = played(play.grid.game, &play.ticket, &results, &ranks) else {
            continue;
        };
        total += settled.gain * play.stake / super::TICKET_PRICE;
    }
    // Le budget se compte en tickets d'un euro : on arrondit vers le bas, et `as` ne peut pas dépasser les bornes de `u32`.
    Ok(total as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lotofoot::grid_of_2026;
    use crate::lotofoot::grid_page::{GridPage, OpenGrid};
    use crate::lotofoot::played::{declare, declare_manual};
    use crate::store::lotofoot::{
        NewReco, NewSession, Page, Source, record_fetch, record_grid_page, record_session,
    };
    use crate::store::migrate::migrate;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const OCTOBER: fn() -> NaiveDate = || NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

    /// Une grille LF7 qui ferme le 25 octobre, deux tickets recommandés à `session_at`.
    fn journal(session_at: DateTime<Utc>) -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let f = record_fetch(
            &conn,
            at("2026-10-23T09:00:00Z"),
            Source::PronoMatch,
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
                grid: grid_of_2026(Game::Lf7, 133),
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
            created_at: session_at,
            model_version: "lotofoot-3",
            params: "{}",
            notes: "{}",
        };
        record_session(&mut conn, &session, &recos).unwrap();
        conn
    }

    /// Résultats « 1N21N21 » et rapports relevés à `fetched` : 476 € au sans-faute, 29,70 € à une erreur.
    fn publish(conn: &Connection, fetched: DateTime<Utc>) {
        let f = record_fetch(conn, fetched, Source::PronoMatch, Page::Results, 200, None).unwrap();
        for (i, outcome) in ["H", "D", "A", "H", "D", "A", "H"].iter().enumerate() {
            conn.execute(
                "INSERT INTO lotofoot_result VALUES ('lf7', 2026, 133, ?1, ?2, ?3)",
                rusqlite::params![i as i64 + 1, outcome, f],
            )
            .unwrap();
        }
        for (rank, winners, gain) in [(1, 210, 476.0), (2, 3601, 29.7)] {
            conn.execute(
                "INSERT INTO lotofoot_rank VALUES ('lf7', 2026, 133, ?1, ?2, ?3, ?4)",
                rusqlite::params![rank, winners, gain, f],
            )
            .unwrap();
        }
    }

    fn euros(amount: f64) -> Euros {
        Euros::new(amount).unwrap()
    }

    /// Identifiant du ticket à une erreur, gagnant de 29,70 €.
    fn second_ticket(conn: &Connection) -> i64 {
        store::current_recos(conn)
            .unwrap()
            .iter()
            .find(|r| r.ticket.to_string() == "2N21N21")
            .unwrap()
            .id
    }

    const SESSION: &str = "2026-10-24T10:00:00Z";
    const PLAYED: &str = "2026-10-24T11:00:00Z";
    const PUBLISHED: &str = "2026-10-25T20:00:00Z";
    const LATER: &str = "2026-10-26T10:00:00Z";

    fn recycled(conn: &Connection, when: &str) -> u32 {
        let recos = store::current_recos(conn).unwrap();
        recycled_gains(conn, &recos, OCTOBER(), at(when)).unwrap()
    }

    #[test]
    fn a_winning_played_ticket_is_recycled_rounded_down() {
        let conn = journal(at(SESSION));
        publish(&conn, at(PUBLISHED));
        let id = second_ticket(&conn);
        declare(&conn, &[id], at(PLAYED), euros(1.0)).unwrap();
        assert_eq!(recycled(&conn, LATER), 29);
    }

    #[test]
    fn a_larger_stake_is_paid_as_many_times() {
        let conn = journal(at(SESSION));
        publish(&conn, at(PUBLISHED));
        let id = second_ticket(&conn);
        declare(&conn, &[id], at(PLAYED), euros(2.0)).unwrap();
        assert_eq!(recycled(&conn, LATER), 59);
    }

    #[test]
    fn nothing_is_recycled_without_a_declaration() {
        let conn = journal(at(SESSION));
        publish(&conn, at(PUBLISHED));
        assert_eq!(recycled(&conn, LATER), 0);
    }

    #[test]
    fn off_model_tickets_are_not_recycled() {
        let conn = journal(at(SESSION));
        publish(&conn, at(PUBLISHED));
        let ticket = Ticket::parse("1N21N22").unwrap();
        let grid = grid_of_2026(Game::Lf7, 133);
        declare_manual(&conn, grid, &ticket, at(PLAYED), euros(1.0)).unwrap();
        assert_eq!(recycled(&conn, LATER), 0);
    }

    #[test]
    fn tickets_older_than_the_budget_are_not_recycled() {
        let conn = journal(at("2026-10-23T10:00:00Z"));
        publish(&conn, at(PUBLISHED));
        let id = second_ticket(&conn);
        declare(&conn, &[id], at(PLAYED), euros(1.0)).unwrap();
        assert_eq!(recycled(&conn, LATER), 0);
    }

    #[test]
    fn a_session_does_not_see_a_report_or_a_play_from_its_future() {
        let conn = journal(at(SESSION));
        publish(&conn, at(PUBLISHED));
        let id = second_ticket(&conn);
        declare(&conn, &[id], at(PLAYED), euros(1.0)).unwrap();
        assert_eq!(recycled(&conn, "2026-10-25T19:59:59Z"), 0);
        let conn = journal(at(SESSION));
        publish(&conn, at(PUBLISHED));
        let id = second_ticket(&conn);
        declare(&conn, &[id], at("2026-10-26T09:00:00Z"), euros(1.0)).unwrap();
        assert_eq!(recycled(&conn, "2026-10-26T08:59:59Z"), 0);
    }

    #[test]
    fn a_gain_stays_in_the_month_of_its_grid() {
        let conn = journal(at(SESSION));
        publish(&conn, at(PUBLISHED));
        let id = second_ticket(&conn);
        declare(&conn, &[id], at(PLAYED), euros(1.0)).unwrap();
        let recos = store::current_recos(&conn).unwrap();
        let november = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let gains = recycled_gains(&conn, &recos, november, at("2026-11-02T10:00:00Z")).unwrap();
        assert_eq!(gains, 0);
    }

    fn ranks(w1: u32, g1: Option<f64>, w2: u32, g2: Option<f64>) -> Vec<PublishedRank> {
        vec![
            PublishedRank {
                rank: 1,
                winners: w1,
                unit_gain: g1.and_then(Euros::new),
            },
            PublishedRank {
                rank: 2,
                winners: w2,
                unit_gain: g2.and_then(Euros::new),
            },
        ]
    }

    const RESULTS: [Outcome; 7] = [
        Outcome::Home,
        Outcome::Draw,
        Outcome::Away,
        Outcome::Home,
        Outcome::Home,
        Outcome::Away,
        Outcome::Draw,
    ];

    #[test]
    fn a_played_ticket_receives_the_published_unit_gain() {
        let ranks = ranks(135, Some(578.0), 2393, Some(39.8));
        let perfect = Ticket::parse("1N2112N").unwrap();
        assert_eq!(
            played(Game::Lf7, &perfect, &RESULTS, &ranks).unwrap(),
            Settled {
                good: 7,
                rank: Some(1),
                gain: 578.0
            }
        );
        let one_error = Ticket::parse("1N2112N".replace('N', "1").as_str()).unwrap();
        let s = played(Game::Lf7, &one_error, &RESULTS, &ranks).unwrap();
        assert_eq!((s.good, s.rank), (5, None));
        let second = Ticket::parse("1N21121").unwrap();
        assert_eq!(
            played(Game::Lf7, &second, &RESULTS, &ranks).unwrap().gain,
            39.8
        );
    }

    #[test]
    fn a_paper_ticket_shares_the_mass_with_the_published_winners() {
        let ranks = ranks(1, Some(578.0), 2393, Some(39.8));
        let perfect = Ticket::parse("1N2112N").unwrap();
        // Un seul gagnant publié : avec nous, la masse se partage en deux.
        let s = paper(Game::Lf7, &perfect, &RESULTS, &ranks, 200_000.0, None).unwrap();
        assert!((s.gain - 289.0).abs() < 1e-9);
        // Aucun gagnant publié : nous aurions touché la masse entière (45 % de 70 % des mises, ou le pactole).
        let ranks = ranks_without_winner();
        let s = paper(
            Game::Lf7,
            &perfect,
            &RESULTS,
            &ranks,
            200_000.0,
            Euros::new(100_000.0),
        )
        .unwrap();
        assert!((s.gain - 100_000.0).abs() < 1e-9);
        let s = paper(
            Game::Lf7,
            &perfect,
            &RESULTS,
            &ranks,
            400_000.0,
            Euros::new(100_000.0),
        )
        .unwrap();
        assert!((s.gain - 0.45 * 0.7 * 400_000.0).abs() < 1e-6);
    }

    fn ranks_without_winner() -> Vec<PublishedRank> {
        ranks(0, None, 2393, Some(39.8))
    }

    #[test]
    fn unknown_ranks_leave_the_ticket_unsettled() {
        let perfect = Ticket::parse("1N2112N").unwrap();
        assert!(played(Game::Lf7, &perfect, &RESULTS, &[]).is_none());
        // Un ticket hors des rangs payés se règle sans rapport (gain nul).
        let lost = Ticket::parse("2121221").unwrap();
        assert_eq!(played(Game::Lf7, &lost, &RESULTS, &[]).unwrap().gain, 0.0);
    }
}
