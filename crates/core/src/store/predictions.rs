//! Journal des prédictions des modèles et des sessions de réglage (tables immuables, migration 4).

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::models::walk_forward::{ModelConfig, ModelKind, Prediction, WalkForwardReport, Window};

fn date_text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Identifiant d'une exécution déjà enregistrée pour cette configuration et cette période.
pub fn find_run(conn: &Connection, config: &ModelConfig, window: Window) -> Result<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM model_run WHERE version = ?1 AND window_start = ?2 AND window_end = ?3",
            params![
                config.version(),
                date_text(window.first),
                date_text(window.last)
            ],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id)
}

/// Enregistre une exécution et toutes ses prédictions, en une seule transaction. Renvoie l'identifiant de l'exécution.
pub fn insert_run(
    conn: &mut Connection,
    config: &ModelConfig,
    window: Window,
    report: &WalkForwardReport,
    created_at: DateTime<Utc>,
) -> Result<i64> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO model_run (model, version, window_start, window_end, created_at,
                                skipped_for_lack_of_history, skipped_invalid, rho_clamped)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            config.model_name(),
            config.version(),
            date_text(window.first),
            date_text(window.last),
            created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            report.skipped_for_lack_of_history as i64,
            report.skipped_invalid as i64,
            report.rho_clamped as i64,
        ],
    )?;
    let run_id = tx.last_insert_rowid();
    {
        let mut insert = tx.prepare_cached(
            "INSERT INTO prediction (run_id, game_id, as_of, home_mean, away_mean, p_home, p_draw, p_away, p_over25, p_btts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for p in &report.predictions {
            insert.execute(params![
                run_id,
                p.game_id,
                date_text(p.as_of),
                p.home_mean,
                p.away_mean,
                p.p_home,
                p.p_draw,
                p.p_away,
                p.p_over25,
                p.p_btts,
            ])?;
        }
    }
    tx.commit()?;
    Ok(run_id)
}

/// Prédictions d'une exécution, par identifiant de match.
pub fn load_predictions(conn: &Connection, run_id: i64) -> Result<Vec<Prediction>> {
    let mut statement = conn.prepare(
        "SELECT game_id, as_of, home_mean, away_mean, p_home, p_draw, p_away, p_over25, p_btts
           FROM prediction WHERE run_id = ?1 ORDER BY game_id",
    )?;
    let rows = statement.query_map([run_id], |row| {
        let as_of: String = row.get(1)?;
        Ok(Prediction {
            game_id: row.get(0)?,
            as_of: NaiveDate::parse_from_str(&as_of, "%Y-%m-%d").unwrap_or_default(),
            home_mean: row.get(2)?,
            away_mean: row.get(3)?,
            p_home: row.get(4)?,
            p_draw: row.get(5)?,
            p_away: row.get(6)?,
            p_over25: row.get(7)?,
            p_btts: row.get(8)?,
        })
    })?;
    let predictions = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(predictions)
}

/// Une session de réglage : la meilleure configuration d'un modèle, figée avant le test.
///
/// `label` identifie ce qui est figé : `poisson`, `dixon_coles` (mélange de tirs libre) ou `dixon_coles_goals`
/// (Dixon-Coles à buts seuls, la variante témoin pour mesurer ce que les tirs apportent).
#[derive(Debug, Clone, PartialEq)]
pub struct TuningRecord {
    /// Étiquette de la configuration figée.
    pub label: String,
    /// Instant du réglage.
    pub created_at: DateTime<Utc>,
    /// Période de validation utilisée.
    pub window: Window,
    /// Configurations essayées pendant cette session, tous modèles confondus.
    pub configs_tried: usize,
    /// Meilleure configuration trouvée pour ce modèle.
    pub config: ModelConfig,
    /// Log loss de cette configuration sur la validation.
    pub validation_log_loss: f64,
}

/// Enregistre le meilleur réglage d'un modèle.
pub fn insert_tuning(conn: &Connection, record: &TuningRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO tuning_session (created_at, window_start, window_end, configs_tried, model, decay, ridge, blend,
                                     validation_log_loss)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            record.created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
            date_text(record.window.first),
            date_text(record.window.last),
            record.configs_tried as i64,
            record.label,
            record.config.decay,
            record.config.ridge,
            record.config.blend,
            record.validation_log_loss,
        ],
    )?;
    Ok(())
}

/// Dernier réglage figé sous une étiquette (`poisson`, `dixon_coles`, `dixon_coles_goals`).
pub fn latest_tuning(conn: &Connection, label: &str) -> Result<Option<TuningRecord>> {
    let record = conn
        .query_row(
            "SELECT created_at, window_start, window_end, configs_tried, decay, ridge, blend, validation_log_loss
               FROM tuning_session WHERE model = ?1 ORDER BY id DESC LIMIT 1",
            [label],
            |row| {
                let created: String = row.get(0)?;
                let (start, end): (String, String) = (row.get(1)?, row.get(2)?);
                let kind = if label == "poisson" { ModelKind::Poisson } else { ModelKind::DixonColes };
                Ok(TuningRecord {
                    label: label.to_string(),
                    created_at: created.parse().unwrap_or_default(),
                    window: Window {
                        first: NaiveDate::parse_from_str(&start, "%Y-%m-%d").unwrap_or_default(),
                        last: NaiveDate::parse_from_str(&end, "%Y-%m-%d").unwrap_or_default(),
                    },
                    configs_tried: row.get::<_, i64>(3)? as usize,
                    config: ModelConfig { kind, decay: row.get(4)?, ridge: row.get(5)?, blend: row.get(6)? },
                    validation_log_loss: row.get(7)?,
                })
            },
        )
        .optional()?;
    Ok(record)
}

/// Nombre de sessions de réglage enregistrées (une seule est l'idéal : chaque réglage supplémentaire est une
/// occasion de s'adapter au hasard).
pub fn tuning_session_count(conn: &Connection) -> Result<u32> {
    let count = conn.query_row(
        "SELECT COUNT(DISTINCT created_at) FROM tuning_session",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'A'), (2, 'B');
             INSERT INTO game (id, div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result) VALUES
               (1, 'E0', '2025/26', '2025-08-16', 1, 2, 1, 0, 'H'),
               (2, 'E0', '2025/26', '2025-08-23', 2, 1, 0, 0, 'D');",
        )
        .unwrap();
        conn
    }

    fn config() -> ModelConfig {
        ModelConfig {
            kind: ModelKind::DixonColes,
            decay: 0.002,
            ridge: 2.0,
            blend: 0.5,
        }
    }

    fn window() -> Window {
        Window {
            first: NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
            last: NaiveDate::from_ymd_opt(2026, 5, 31).unwrap(),
        }
    }

    fn report() -> WalkForwardReport {
        let prediction = |game_id| Prediction {
            game_id,
            as_of: NaiveDate::from_ymd_opt(2025, 8, 11).unwrap(),
            home_mean: 1.5,
            away_mean: 1.1,
            p_home: 0.45,
            p_draw: 0.27,
            p_away: 0.28,
            p_over25: 0.52,
            p_btts: 0.5,
        };
        WalkForwardReport {
            predictions: vec![prediction(1), prediction(2)],
            skipped_for_lack_of_history: 3,
            skipped_invalid: 1,
            rho_clamped: 2,
        }
    }

    #[test]
    fn a_run_and_its_predictions_round_trip() {
        let mut conn = database();
        let report = report();

        let run_id = insert_run(&mut conn, &config(), window(), &report, Utc::now()).unwrap();

        assert_eq!(find_run(&conn, &config(), window()).unwrap(), Some(run_id));
        assert_eq!(load_predictions(&conn, run_id).unwrap(), report.predictions);
        let (skipped, clamped): (i64, i64) = conn
            .query_row(
                "SELECT skipped_for_lack_of_history, rho_clamped FROM model_run",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((skipped, clamped), (3, 2));
    }

    #[test]
    fn another_configuration_or_window_is_another_run_and_a_rerun_is_refused() {
        let mut conn = database();
        insert_run(&mut conn, &config(), window(), &report(), Utc::now()).unwrap();
        let other = ModelConfig {
            ridge: 8.0,
            ..config()
        };
        assert_eq!(find_run(&conn, &other, window()).unwrap(), None);

        // Même version et même période : la contrainte d'unicité interdit de réécrire l'historique.
        assert!(insert_run(&mut conn, &config(), window(), &report(), Utc::now()).is_err());
        // L'échec ne laisse aucune prédiction orpheline.
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM prediction", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn predictions_runs_and_tuning_sessions_are_immutable() {
        let mut conn = database();
        insert_run(&mut conn, &config(), window(), &report(), Utc::now()).unwrap();
        insert_tuning(
            &conn,
            &TuningRecord {
                label: "dixon_coles".to_string(),
                created_at: Utc::now(),
                window: window(),
                configs_tried: 64,
                config: config(),
                validation_log_loss: 0.99,
            },
        )
        .unwrap();

        for statement in [
            "UPDATE prediction SET p_home = 0.9",
            "DELETE FROM prediction",
            "UPDATE model_run SET model = 'x'",
            "DELETE FROM model_run",
            "UPDATE tuning_session SET blend = 1",
            "DELETE FROM tuning_session",
        ] {
            let err = conn.execute(statement, []).unwrap_err();
            assert!(
                err.to_string().contains("journal immuable"),
                "{statement} : {err}"
            );
        }
    }

    #[test]
    fn the_latest_tuning_of_a_model_is_found_and_sessions_are_counted() {
        let conn = database();
        assert_eq!(latest_tuning(&conn, "dixon_coles").unwrap(), None);
        let first = TuningRecord {
            label: "dixon_coles".to_string(),
            created_at: "2026-10-18T10:00:00Z".parse().unwrap(),
            window: window(),
            configs_tried: 64,
            config: config(),
            validation_log_loss: 0.99,
        };
        let second = TuningRecord {
            created_at: "2026-10-19T10:00:00Z".parse().unwrap(),
            config: ModelConfig {
                decay: 0.004,
                ..config()
            },
            validation_log_loss: 0.98,
            ..first.clone()
        };
        insert_tuning(&conn, &first).unwrap();
        insert_tuning(&conn, &second).unwrap();

        let latest = latest_tuning(&conn, "dixon_coles").unwrap().unwrap();

        assert_eq!(latest, second);
        assert_eq!(tuning_session_count(&conn).unwrap(), 2);
        assert_eq!(latest_tuning(&conn, "poisson").unwrap(), None);
    }
}
