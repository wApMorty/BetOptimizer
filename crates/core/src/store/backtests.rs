//! Journal des backtests : seuils gelés, exécutions et paris (tables immuables, migration 6).

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::backtest::candidates::Period;
use crate::backtest::engine::{BacktestResult, Candidate};
use crate::backtest::study::Criterion;
use crate::domain::Outcome;
use crate::error::Result;

fn period_text(period: Period) -> &'static str {
    match period {
        Period::Train => "train",
        Period::Validation => "validation",
        Period::Test => "test",
    }
}

fn outcome_code(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Home => "H",
        Outcome::Draw => "D",
        Outcome::Away => "A",
    }
}

/// Un seuil gelé.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenThreshold {
    /// Source (`soft:B365`, `model:dixon_coles`).
    pub source: String,
    /// Avantage minimal gelé.
    pub threshold: f64,
    /// CLV moyen sur la période de réglage.
    pub selection_clv: Option<f64>,
    /// Paris de la période de réglage.
    pub selection_bets: usize,
    /// Configurations essayées pendant la session.
    pub configs_tried: usize,
    /// Critère qui a choisi le seuil.
    pub criterion: Criterion,
    /// Croissance logarithmique totale attendue sur la période de réglage (critère `expected_growth` seulement).
    pub selection_growth: Option<f64>,
}

/// Enregistre un seuil gelé.
pub fn insert_selection(
    conn: &Connection,
    record: &FrozenThreshold,
    created_at: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO backtest_selection (created_at, source, threshold, selection_clv, selection_bets, configs_tried,
                                         criterion, selection_growth)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            record.source,
            record.threshold,
            record.selection_clv,
            record.selection_bets as i64,
            record.configs_tried as i64,
            record.criterion.as_str(),
            record.selection_growth,
        ],
    )?;
    Ok(())
}

/// Dernier seuil gelé d'une source.
pub fn latest_selection(conn: &Connection, source: &str) -> Result<Option<FrozenThreshold>> {
    let record = conn
        .query_row(
            "SELECT threshold, selection_clv, selection_bets, configs_tried, criterion, selection_growth
               FROM backtest_selection WHERE source = ?1 ORDER BY id DESC LIMIT 1",
            [source],
            |row| {
                Ok(FrozenThreshold {
                    source: source.to_string(),
                    threshold: row.get(0)?,
                    selection_clv: row.get(1)?,
                    selection_bets: row.get::<_, i64>(2)? as usize,
                    configs_tried: row.get::<_, i64>(3)? as usize,
                    // La contrainte CHECK de la migration 11 n'admet que ces deux valeurs.
                    criterion: if row.get::<_, String>(4)? == "expected_growth" {
                        Criterion::ExpectedGrowth
                    } else {
                        Criterion::MeanClv
                    },
                    selection_growth: row.get(5)?,
                })
            },
        )
        .optional()?;
    Ok(record)
}

/// Nombre de sessions de réglage de seuils.
pub fn selection_session_count(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row(
        "SELECT COUNT(DISTINCT created_at) FROM backtest_selection",
        [],
        |row| row.get(0),
    )?)
}

/// Configurations essayées, toutes sessions confondues : chaque session compte une fois (ses lignes portent toutes le
/// même total), d'où le `DISTINCT` sur la date de la session.
pub fn configs_tried_all_sessions(conn: &Connection) -> Result<usize> {
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(configs_tried), 0) FROM
           (SELECT created_at, MAX(configs_tried) AS configs_tried FROM backtest_selection GROUP BY created_at)",
        [],
        |row| row.get(0),
    )?;
    Ok(total as usize)
}

/// Description d'une exécution à enregistrer.
#[derive(Debug, Clone, PartialEq)]
pub struct RunKey {
    /// Source.
    pub source: String,
    /// Période.
    pub period: Period,
    /// Seuil d'avantage.
    pub threshold: f64,
    /// Fraction de Kelly.
    pub kelly_fraction: f64,
    /// Décote appliquée aux cotes.
    pub price_factor: f64,
}

/// Enregistre une exécution et ses paris. Une exécution déjà enregistrée n'est pas réécrite : renvoie `None`.
pub fn insert_run(
    conn: &mut Connection,
    key: &RunKey,
    candidates: &[Candidate],
    result: &BacktestResult,
    created_at: DateTime<Utc>,
) -> Result<Option<i64>> {
    let tx = conn.transaction()?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM backtest_run WHERE source = ?1 AND period = ?2 AND threshold = ?3
                AND kelly_fraction = ?4 AND price_factor = ?5",
            params![
                key.source,
                period_text(key.period),
                key.threshold,
                key.kelly_fraction,
                key.price_factor
            ],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Ok(None);
    }
    tx.execute(
        "INSERT INTO backtest_run (created_at, source, period, threshold, kelly_fraction, price_factor, bets, final_bankroll)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            key.source,
            period_text(key.period),
            key.threshold,
            key.kelly_fraction,
            key.price_factor,
            result.bets.len() as i64,
            result.final_bankroll,
        ],
    )?;
    let run_id = tx.last_insert_rowid();
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO backtest_bet (run_id, game_id, batch_date, outcome, odds, probability, close_fair, stake, profit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for bet in &result.bets {
            let candidate = &candidates[bet.candidate];
            insert.execute(params![
                run_id,
                candidate.game_id,
                bet.batch.format("%Y-%m-%d").to_string(),
                outcome_code(candidate.outcome),
                candidate.odds.get(),
                candidate.probability.get(),
                candidate.close_fair.map(|p| p.get()),
                bet.stake.get(),
                bet.profit,
            ])?;
        }
    }
    tx.commit()?;
    Ok(Some(run_id))
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::backtest::engine::{PlacedBet, StakingRule, run};
    use crate::domain::{Euros, Odds, Probability};
    use crate::store::migrate;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'A'), (2, 'B');
             INSERT INTO game (id, div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
             VALUES (1, 'E0', '2025/26', '2025-08-16', 1, 2, 1, 0, 'H');",
        )
        .unwrap();
        conn
    }

    fn candidates() -> Vec<Candidate> {
        vec![Candidate {
            game_id: 1,
            date: NaiveDate::from_ymd_opt(2025, 8, 16).unwrap(),
            block: 0,
            outcome: Outcome::Home,
            odds: Odds::new(2.0).unwrap(),
            probability: Probability::new(0.6).unwrap(),
            close_fair: Probability::new(0.55),
            won: true,
        }]
    }

    fn key() -> RunKey {
        RunKey {
            source: "soft:B365".into(),
            period: Period::Test,
            threshold: 0.03,
            kelly_fraction: 0.5,
            price_factor: 1.0,
        }
    }

    #[test]
    fn a_run_and_its_bets_are_stored_once_and_cannot_be_rewritten() {
        let mut conn = database();
        let candidates = candidates();
        let rule = StakingRule {
            kelly_fraction: 0.5,
            initial_bankroll: Euros::new(1000.0).unwrap(),
            min_stake: Euros::new(1.0).unwrap(),
        };
        let result = run(&candidates, &rule).unwrap();

        let first = insert_run(&mut conn, &key(), &candidates, &result, Utc::now()).unwrap();
        let second = insert_run(&mut conn, &key(), &candidates, &result, Utc::now()).unwrap();

        assert!(first.is_some());
        assert_eq!(second, None, "relancer la même exécution ne réécrit rien");
        let (bets, stake): (i64, f64) = conn
            .query_row("SELECT COUNT(*), SUM(stake) FROM backtest_bet", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(bets, 1);
        assert!((stake - result.bets[0].stake.get()).abs() < 1e-12);
        let _: &PlacedBet = &result.bets[0];
        // Les triggers ne se déclenchent que sur des lignes existantes : on vérifie donc après l'insertion.
        for statement in [
            "UPDATE backtest_run SET bets = 0",
            "DELETE FROM backtest_run",
            "UPDATE backtest_bet SET stake = 0",
            "DELETE FROM backtest_bet",
        ] {
            let err = conn.execute(statement, []).unwrap_err();
            assert!(err.to_string().contains("journal immuable"), "{statement}");
        }
    }

    #[test]
    fn frozen_thresholds_are_found_counted_and_immutable() {
        let conn = database();
        assert_eq!(latest_selection(&conn, "soft:B365").unwrap(), None);
        let record = FrozenThreshold {
            source: "soft:B365".into(),
            threshold: 0.03,
            selection_clv: Some(0.004),
            selection_bets: 410,
            configs_tried: 54,
            criterion: Criterion::MeanClv,
            selection_growth: None,
        };
        insert_selection(&conn, &record, "2026-10-19T10:00:00Z".parse().unwrap()).unwrap();
        let newer = FrozenThreshold {
            threshold: 0.005,
            configs_tried: 648,
            criterion: Criterion::ExpectedGrowth,
            selection_growth: Some(0.21),
            ..record.clone()
        };
        insert_selection(&conn, &newer, "2026-10-20T10:00:00Z".parse().unwrap()).unwrap();

        assert_eq!(
            latest_selection(&conn, "soft:B365").unwrap().unwrap(),
            newer
        );
        assert_eq!(selection_session_count(&conn).unwrap(), 2);
        assert_eq!(configs_tried_all_sessions(&conn).unwrap(), 54 + 648);
        for statement in [
            "UPDATE backtest_selection SET threshold = 0",
            "DELETE FROM backtest_selection",
        ] {
            let err = conn.execute(statement, []).unwrap_err();
            assert!(err.to_string().contains("journal immuable"), "{statement}");
        }
    }
}
