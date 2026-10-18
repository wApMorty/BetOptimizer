//! Réglage des hyperparamètres sur la **validation**, gel de la meilleure configuration, puis évaluation unique
//! sur le **test** (invariants 3 et 8 : sélection sur le log loss, seuils gelés avant la saison de test, nombre de
//! configurations essayées rapporté).
//!
//! Découpage walk-forward (décidé avec l'utilisateur) : entraînement 2016/17–2023/24, validation 2024/25, test
//! 2025/26 puis 2026/27 en avant. Chaque configuration est un walk-forward complet sur la validation ; ses
//! prédictions sont stockées (journal immuable), ce qui rend le réglage reprenable après une interruption.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::Connection;

use super::data::LeagueData;
use super::walk_forward::{self, ModelConfig, ModelKind, Prediction, Window};
use crate::domain::{Outcome, Probs1x2};
use crate::error::{Error, Result};
use crate::eval::metrics::log_loss;
use crate::store::predictions::{self, TuningRecord};

/// Validation : saison 2024/25.
pub fn validation_window() -> Window {
    Window {
        first: NaiveDate::from_ymd_opt(2024, 7, 1).unwrap_or_default(),
        last: NaiveDate::from_ymd_opt(2025, 6, 30).unwrap_or_default(),
    }
}

/// Test : de la saison 2025/26 jusqu'à la dernière date disponible (donc 2026/27 « en avant »).
pub fn test_window(last: NaiveDate) -> Window {
    Window {
        first: NaiveDate::from_ymd_opt(2025, 7, 1).unwrap_or_default(),
        last,
    }
}

/// Les configurations essayées : Poisson (16, buts seuls) et Dixon-Coles (48, avec 3 mélanges de tirs).
///
/// `ξ` (par jour) : 0,0005 / 0,001 / 0,002 / 0,004, soit des demi-vies de 1 386 / 693 / 347 / 173 jours.
/// `κ` : 0,5 / 2 / 8 / 32. Mélange `w` avec les buts attendus par les tirs : 0 / 0,5 / 1.
pub fn candidate_grid() -> Vec<ModelConfig> {
    let decays = [0.0005, 0.001, 0.002, 0.004];
    let ridges = [0.5, 2.0, 8.0, 32.0];
    let mut grid = Vec::new();
    for decay in decays {
        for ridge in ridges {
            grid.push(ModelConfig {
                kind: ModelKind::Poisson,
                decay,
                ridge,
                blend: 0.0,
            });
        }
    }
    for decay in decays {
        for ridge in ridges {
            for blend in [0.0, 0.5, 1.0] {
                grid.push(ModelConfig {
                    kind: ModelKind::DixonColes,
                    decay,
                    ridge,
                    blend,
                });
            }
        }
    }
    grid
}

/// Issue réelle de chaque match, par identifiant.
pub fn outcomes(leagues: &[LeagueData]) -> HashMap<i64, Outcome> {
    leagues
        .iter()
        .flat_map(|league| league.games.iter())
        .map(|game| {
            let outcome = match game.goals_home.cmp(&game.goals_away) {
                std::cmp::Ordering::Greater => Outcome::Home,
                std::cmp::Ordering::Equal => Outcome::Draw,
                std::cmp::Ordering::Less => Outcome::Away,
            };
            (game.game_id, outcome)
        })
        .collect()
}

/// Log loss moyen du 1X2 d'une série de prédictions. `None` sans prédiction exploitable.
pub fn mean_log_loss(predictions: &[Prediction], outcomes: &HashMap<i64, Outcome>) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0usize;
    for prediction in predictions {
        let outcome = outcomes.get(&prediction.game_id)?;
        let probs = Probs1x2::new(prediction.p_home, prediction.p_draw, prediction.p_away)?;
        total += log_loss(&probs, *outcome);
        count += 1;
    }
    (count > 0).then(|| total / count as f64)
}

/// Résultat d'une configuration sur la validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigResult {
    /// Configuration.
    pub config: ModelConfig,
    /// Log loss moyen du 1X2 sur la validation.
    pub log_loss: f64,
    /// Nombre de matchs prédits.
    pub games: usize,
    /// Prédictions où `ρ` a dû être ramené dans le domaine faisable.
    pub rho_clamped: usize,
}

/// Résultat d'une session de réglage.
#[derive(Debug, Clone, PartialEq)]
pub struct TuningOutcome {
    /// Toutes les configurations essayées, dans l'ordre de la grille.
    pub results: Vec<ConfigResult>,
    /// Meilleure configuration Poisson.
    pub best_poisson: ConfigResult,
    /// Meilleure configuration Dixon-Coles (mélange de tirs libre).
    pub best_dixon_coles: ConfigResult,
    /// Meilleure configuration Dixon-Coles à buts seuls : la variante témoin qui mesure l'apport des tirs.
    pub best_goals_only: ConfigResult,
}

/// Meilleure configuration d'une famille : log loss le plus bas, parmi celles qui prédisent presque autant de
/// matchs que les autres (un modèle qui s'abstient sur les matchs difficiles ne doit pas gagner par défaut).
pub fn select_best(results: &[ConfigResult], kind: ModelKind) -> Option<&ConfigResult> {
    select_best_where(results, |config| config.kind == kind)
}

/// Comme [`select_best`], restreint aux configurations qui satisfont `keep`.
pub fn select_best_where(
    results: &[ConfigResult],
    keep: impl Fn(&ModelConfig) -> bool,
) -> Option<&ConfigResult> {
    let most_games = results.iter().map(|r| r.games).max()?;
    results
        .iter()
        .filter(|r| keep(&r.config) && (r.games as f64) >= 0.98 * most_games as f64)
        .min_by(|a, b| a.log_loss.total_cmp(&b.log_loss))
}

/// Régle les deux familles sur la validation et fige la meilleure configuration de chacune.
///
/// `progress(i, n, résultat)` est appelée après chaque configuration. Une configuration déjà calculée (même
/// version, même période) n'est pas recalculée : on relit ses prédictions.
pub fn tune(
    conn: &mut Connection,
    leagues: &[LeagueData],
    grid: &[ModelConfig],
    window: Window,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(usize, usize, &ConfigResult),
) -> Result<TuningOutcome> {
    let outcomes = outcomes(leagues);
    let mut results = Vec::with_capacity(grid.len());
    for (index, config) in grid.iter().enumerate() {
        let (predictions, rho_clamped) = match predictions::find_run(conn, config, window)? {
            Some(run_id) => (predictions::load_predictions(conn, run_id)?, 0),
            None => {
                let report = walk_forward::run(leagues, config, window)?;
                predictions::insert_run(conn, config, window, &report, now)?;
                (report.predictions, report.rho_clamped)
            }
        };
        let log_loss = mean_log_loss(&predictions, &outcomes)
            .ok_or_else(|| Error::Data(format!("aucune prédiction pour {}", config.version())))?;
        let result = ConfigResult {
            config: *config,
            log_loss,
            games: predictions.len(),
            rho_clamped,
        };
        progress(index + 1, grid.len(), &result);
        results.push(result);
    }

    let best = |kind| {
        select_best(&results, kind)
            .cloned()
            .ok_or_else(|| Error::Data("aucune configuration retenable".to_string()))
    };
    let best_poisson = best(ModelKind::Poisson)?;
    let best_dixon_coles = best(ModelKind::DixonColes)?;
    let best_goals_only = select_best_where(&results, |c| {
        c.kind == ModelKind::DixonColes && c.blend == 0.0
    })
    .cloned()
    .ok_or_else(|| Error::Data("aucune configuration à buts seuls".to_string()))?;
    for (label, chosen) in [
        ("poisson", &best_poisson),
        ("dixon_coles", &best_dixon_coles),
        ("dixon_coles_goals", &best_goals_only),
    ] {
        predictions::insert_tuning(
            conn,
            &TuningRecord {
                label: label.to_string(),
                created_at: now,
                window,
                configs_tried: grid.len(),
                config: chosen.config,
                validation_log_loss: chosen.log_loss,
            },
        )?;
    }
    Ok(TuningOutcome {
        results,
        best_poisson,
        best_dixon_coles,
        best_goals_only,
    })
}

/// Une exécution de test.
#[derive(Debug, Clone, PartialEq)]
pub struct TestRun {
    /// Étiquette de la configuration figée (`poisson`, `dixon_coles`, `dixon_coles_goals`).
    pub label: &'static str,
    /// Configuration figée.
    pub config: ModelConfig,
    /// Identifiant de l'exécution en base.
    pub run_id: i64,
    /// Nombre de matchs prédits.
    pub games: usize,
}

/// Évalue les configurations **figées** (dernier réglage de chaque famille) sur la période de test, une seule fois.
/// Refuse de s'exécuter sans réglage : l'évaluation n'a pas de configuration à elle.
pub fn evaluate(
    conn: &mut Connection,
    leagues: &[LeagueData],
    window: Window,
    now: DateTime<Utc>,
) -> Result<Vec<TestRun>> {
    let mut runs = Vec::new();
    for label in ["poisson", "dixon_coles", "dixon_coles_goals"] {
        let frozen = predictions::latest_tuning(conn, label)?.ok_or_else(|| {
            Error::Data("aucun réglage figé : lance d'abord `edge model tune`".to_string())
        })?;
        let config = frozen.config;
        let (run_id, games) = match predictions::find_run(conn, &config, window)? {
            Some(run_id) => (run_id, predictions::load_predictions(conn, run_id)?.len()),
            None => {
                let report = walk_forward::run(leagues, &config, window)?;
                let games = report.predictions.len();
                (
                    predictions::insert_run(conn, &config, window, &report, now)?,
                    games,
                )
            }
        };
        runs.push(TestRun {
            label,
            config,
            run_id,
            games,
        });
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use chrono::Days;

    use super::*;
    use crate::models::testing::{insert_league, simulated_league};
    use crate::store::migrate;

    fn result(kind: ModelKind, decay: f64, log_loss: f64, games: usize) -> ConfigResult {
        ConfigResult {
            config: ModelConfig {
                kind,
                decay,
                ridge: 1.0,
                blend: 0.0,
            },
            log_loss,
            games,
            rho_clamped: 0,
        }
    }

    #[test]
    fn the_grid_has_sixty_four_distinct_configurations() {
        let grid = candidate_grid();
        assert_eq!(grid.len(), 64);
        assert_eq!(
            grid.iter().filter(|c| c.kind == ModelKind::Poisson).count(),
            16
        );
        let versions: std::collections::HashSet<String> =
            grid.iter().map(|c| c.version()).collect();
        assert_eq!(
            versions.len(),
            64,
            "chaque configuration a une version distincte"
        );
        // Poisson : buts seuls.
        assert!(
            grid.iter()
                .filter(|c| c.kind == ModelKind::Poisson)
                .all(|c| c.blend == 0.0)
        );
    }

    #[test]
    fn the_validation_and_test_windows_do_not_overlap() {
        let validation = validation_window();
        let test = test_window(NaiveDate::from_ymd_opt(2026, 10, 12).unwrap());
        assert!(validation.last < test.first);
        assert_eq!(test.first, NaiveDate::from_ymd_opt(2025, 7, 1).unwrap());
    }

    #[test]
    fn the_best_configuration_has_the_lowest_log_loss_among_full_coverage_runs() {
        let results = vec![
            result(ModelKind::DixonColes, 0.001, 0.990, 1000),
            result(ModelKind::DixonColes, 0.002, 0.985, 1000),
            // Meilleur log loss, mais il s'abstient sur 10 % des matchs : écarté.
            result(ModelKind::DixonColes, 0.004, 0.970, 900),
            result(ModelKind::Poisson, 0.001, 0.995, 1000),
        ];

        let best = select_best(&results, ModelKind::DixonColes).unwrap();

        assert_eq!(best.config.decay, 0.002);
        assert_eq!(
            select_best(&results, ModelKind::Poisson).unwrap().log_loss,
            0.995
        );
    }

    #[test]
    fn mean_log_loss_matches_a_hand_calculation() {
        let prediction = |game_id, p_home| Prediction {
            game_id,
            as_of: NaiveDate::from_ymd_opt(2025, 8, 11).unwrap(),
            home_mean: 1.5,
            away_mean: 1.0,
            p_home,
            p_draw: (1.0 - p_home) / 2.0,
            p_away: (1.0 - p_home) / 2.0,
            p_over25: 0.5,
            p_btts: 0.5,
        };
        let outcomes: HashMap<i64, Outcome> = [(1, Outcome::Home), (2, Outcome::Away)]
            .into_iter()
            .collect();

        let mean = mean_log_loss(&[prediction(1, 0.5), prediction(2, 0.5)], &outcomes).unwrap();

        // (−ln 0,5 − ln 0,25) / 2
        assert!((mean - (0.5_f64.ln().abs() + 0.25_f64.ln().abs()) / 2.0).abs() < 1e-12);
        assert!(mean_log_loss(&[], &outcomes).is_none());
    }

    fn setup() -> (Connection, Vec<LeagueData>, Window) {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        let league = simulated_league(8, 77);
        insert_league(&conn, &league);
        let first = league.games.iter().map(|g| g.date).min().unwrap() + Days::new(6 * 365);
        let window = Window {
            first,
            last: first + Days::new(365),
        };
        (conn, vec![league], window)
    }

    fn small_grid() -> Vec<ModelConfig> {
        let mut grid = Vec::new();
        for decay in [0.001, 0.004] {
            grid.push(ModelConfig {
                kind: ModelKind::Poisson,
                decay,
                ridge: 2.0,
                blend: 0.0,
            });
            grid.push(ModelConfig {
                kind: ModelKind::DixonColes,
                decay,
                ridge: 2.0,
                blend: 0.0,
            });
        }
        grid
    }

    #[test]
    fn tuning_stores_every_run_freezes_one_configuration_per_family_and_counts_the_grid() {
        let (mut conn, leagues, window) = setup();
        let mut seen = Vec::new();

        let outcome = tune(
            &mut conn,
            &leagues,
            &small_grid(),
            window,
            Utc::now(),
            &mut |i, n, r| {
                seen.push((i, n, r.config.version()));
            },
        )
        .unwrap();

        assert_eq!(outcome.results.len(), 4);
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[3].0, 4);
        let runs: u32 = conn
            .query_row("SELECT COUNT(*) FROM model_run", [], |r| r.get(0))
            .unwrap();
        assert_eq!(runs, 4);
        // Trois lignes figées (Poisson, Dixon-Coles, Dixon-Coles à buts seuls), chacune notant 4 configurations.
        let frozen: Vec<(String, i64)> = conn
            .prepare("SELECT model, configs_tried FROM tuning_session ORDER BY model")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(
            frozen,
            vec![
                ("dixon_coles".to_string(), 4),
                ("dixon_coles_goals".to_string(), 4),
                ("poisson".to_string(), 4)
            ]
        );
        // Dans la petite grille, tous les Dixon-Coles sont à buts seuls : le témoin est une configuration à mélange nul.
        assert_eq!(outcome.best_goals_only.config.blend, 0.0);
        // Le meilleur choisi est bien le plus bas de sa famille.
        let lowest_dc = outcome
            .results
            .iter()
            .filter(|r| r.config.kind == ModelKind::DixonColes)
            .map(|r| r.log_loss)
            .fold(f64::INFINITY, f64::min);
        assert_eq!(outcome.best_dixon_coles.log_loss, lowest_dc);
    }

    #[test]
    fn tuning_can_be_resumed_without_recomputing_and_gives_the_same_result() {
        let (mut conn, leagues, window) = setup();
        let first = tune(
            &mut conn,
            &leagues,
            &small_grid(),
            window,
            Utc::now(),
            &mut |_, _, _| {},
        )
        .unwrap();

        let second = tune(
            &mut conn,
            &leagues,
            &small_grid(),
            window,
            Utc::now(),
            &mut |_, _, _| {},
        )
        .unwrap();

        let runs: u32 = conn
            .query_row("SELECT COUNT(*) FROM model_run", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            runs, 4,
            "aucune exécution supplémentaire : les prédictions sont relues"
        );
        for (a, b) in first.results.iter().zip(&second.results) {
            assert_eq!(a.log_loss, b.log_loss);
            assert_eq!(a.games, b.games);
        }
    }

    #[test]
    fn evaluation_refuses_to_run_without_a_frozen_configuration() {
        let (mut conn, leagues, window) = setup();
        let err = evaluate(&mut conn, &leagues, window, Utc::now()).unwrap_err();
        assert!(err.to_string().contains("edge model tune"));
    }

    #[test]
    fn evaluation_runs_the_frozen_configurations_once_and_reuses_them() {
        let (mut conn, leagues, window) = setup();
        // Réglage sur une première période, évaluation sur la suivante.
        let validation = Window {
            first: window.first,
            last: window.first + Days::new(180),
        };
        let test = Window {
            first: validation.last + Days::new(1),
            last: window.last,
        };
        tune(
            &mut conn,
            &leagues,
            &small_grid(),
            validation,
            Utc::now(),
            &mut |_, _, _| {},
        )
        .unwrap();

        let runs = evaluate(&mut conn, &leagues, test, Utc::now()).unwrap();
        let again = evaluate(&mut conn, &leagues, test, Utc::now()).unwrap();

        assert_eq!(runs.len(), 3);
        assert!(runs.iter().all(|r| r.games > 0));
        // Deux étiquettes figeant la même configuration partagent la même exécution (aucun calcul en double).
        assert_eq!(runs[1].run_id, runs[2].run_id);
        assert_eq!(
            runs, again,
            "la seconde évaluation relit les mêmes exécutions"
        );
        let total: u32 = conn
            .query_row("SELECT COUNT(*) FROM model_run", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            total, 6,
            "4 réglages + 2 évaluations distinctes (dixon_coles et dixon_coles_goals partagent la leur)"
        );
    }
}
