//! Walk-forward hebdomadaire : chaque semaine, on réajuste le modèle sur le passé et on prédit la semaine.
//!
//! C'est la seule validation autorisée (invariant 1) : jamais de validation croisée aléatoire, qui ferait servir
//! l'avenir à prédire le passé. Pour la semaine commençant le lundi `D`, le modèle n'a vu que des matchs
//! **strictement antérieurs à D** : les résultats du mardi ne servent pas à prédire le samedi suivant (on perd un
//! peu d'information, on ne triche jamais).

use std::thread;

use chrono::{Datelike, Days, NaiveDate};

use super::data::{HistoricalGame, LeagueData, training_games};
use super::dixon_coles::DixonColes;
use super::poisson::Poisson;
use super::xg_proxy::{ShotObservation, XgProxy};
use super::{FittedModel, MatchModel};
use crate::error::{Error, Result};

/// Version du code des modèles, stockée avec chaque prédiction (invariant 9). À incrémenter dès que le calcul
/// d'une prédiction change : les anciennes prédictions restent consultables sous leur ancienne version.
pub const CODE_VERSION: &str = "v1";

/// Nombre minimal de matchs d'entraînement pour qu'une prédiction soit tentée. En dessous, le modèle est
/// dominé par la pénalité : mieux vaut ne rien prédire.
const MIN_TRAINING_GAMES: usize = 300;

/// Famille de modèle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// Poisson indépendant.
    Poisson,
    /// Dixon-Coles.
    DixonColes,
}

/// Hyperparamètres d'une configuration : ce que le réglage sur la validation choisit, puis gèle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelConfig {
    /// Famille de modèle.
    pub kind: ModelKind,
    /// Décroissance temporelle `ξ` (par jour) : un match vieux de `t` jours pèse `exp(−ξ t)`.
    pub decay: f64,
    /// Intensité `κ` de la régularisation L2 des forces d'équipe.
    pub ridge: f64,
    /// Part des buts attendus (estimés à partir des tirs) dans la cible : 0 = buts seuls, 1 = tirs seuls.
    pub blend: f64,
}

impl ModelConfig {
    /// Nom du modèle (`poisson`, `dixon_coles`).
    pub fn model_name(&self) -> &'static str {
        match self.kind {
            ModelKind::Poisson => "poisson",
            ModelKind::DixonColes => "dixon_coles",
        }
    }

    /// Identifiant de version stocké en base : famille, hyperparamètres et version du code.
    pub fn version(&self) -> String {
        format!(
            "{}|xi={}|ridge={}|blend={}|grille={}|{}",
            self.model_name(),
            self.decay,
            self.ridge,
            self.blend,
            super::grid::MAX_GOALS,
            CODE_VERSION
        )
    }

    fn model(&self) -> Box<dyn MatchModel> {
        match self.kind {
            ModelKind::Poisson => Box::new(Poisson { ridge: self.ridge }),
            ModelKind::DixonColes => Box::new(DixonColes { ridge: self.ridge }),
        }
    }
}

/// Période évaluée : les matchs dont la date est dans `[first, last]` sont prédits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// Premier jour (inclus).
    pub first: NaiveDate,
    /// Dernier jour (inclus).
    pub last: NaiveDate,
}

/// Prédiction d'un match.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// Identifiant du match en base.
    pub game_id: i64,
    /// Date d'ajustement : le modèle n'a vu que des matchs strictement antérieurs.
    pub as_of: NaiveDate,
    /// Buts attendus de l'équipe à domicile.
    pub home_mean: f64,
    /// Buts attendus de l'équipe à l'extérieur.
    pub away_mean: f64,
    /// Probabilité de victoire à domicile.
    pub p_home: f64,
    /// Probabilité de nul.
    pub p_draw: f64,
    /// Probabilité de victoire à l'extérieur.
    pub p_away: f64,
    /// Probabilité de plus de 2,5 buts.
    pub p_over25: f64,
    /// Probabilité que les deux équipes marquent.
    pub p_btts: f64,
}

/// Résultat d'un walk-forward.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WalkForwardReport {
    /// Prédictions, dans l'ordre (ligue, semaine).
    pub predictions: Vec<Prediction>,
    /// Matchs non prédits faute d'historique suffisant.
    pub skipped_for_lack_of_history: usize,
    /// Matchs non prédits parce que les paramètres ne donnaient pas une loi de probabilité valide.
    pub skipped_invalid: usize,
    /// Prédictions pour lesquelles `ρ` a dû être ramené dans le domaine où `τ` reste positif.
    pub rho_clamped: usize,
}

/// Lundi de la semaine d'une date.
fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Days::new(u64::from(date.weekday().num_days_from_monday()))
}

/// Lance le walk-forward sur toutes les ligues, une par fil d'exécution (les ligues sont indépendantes).
///
/// Le résultat ne dépend pas du nombre de fils : chaque ligue est calculée séquentiellement et de façon
/// déterministe ; on réassemble simplement dans l'ordre des ligues.
pub fn run(
    leagues: &[LeagueData],
    config: &ModelConfig,
    window: Window,
) -> Result<WalkForwardReport> {
    let model = config.model();
    let model_ref: &dyn MatchModel = model.as_ref();
    let results: Vec<Result<WalkForwardReport>> = thread::scope(|scope| {
        let handles: Vec<_> = leagues
            .iter()
            .map(|league| scope.spawn(move || run_league(league, config, model_ref, window)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(Error::Optimizer("un fil d'exécution a paniqué".to_string()))
                })
            })
            .collect()
    });
    let mut merged = WalkForwardReport::default();
    for result in results {
        let report = result?;
        merged.predictions.extend(report.predictions);
        merged.skipped_for_lack_of_history += report.skipped_for_lack_of_history;
        merged.skipped_invalid += report.skipped_invalid;
        merged.rho_clamped += report.rho_clamped;
    }
    Ok(merged)
}

/// Walk-forward d'une ligue.
pub fn run_league(
    league: &LeagueData,
    config: &ModelConfig,
    model: &dyn MatchModel,
    window: Window,
) -> Result<WalkForwardReport> {
    let mut report = WalkForwardReport::default();
    let mut previous: Option<FittedModel> = None;
    let mut week_start = monday_of(window.first);
    while week_start <= window.last {
        let week_end = week_start + Days::new(7);
        let week_games: Vec<&HistoricalGame> = league
            .games
            .iter()
            .filter(|g| {
                g.date >= week_start
                    && g.date < week_end
                    && g.date >= window.first
                    && g.date <= window.last
            })
            .collect();
        if !week_games.is_empty() {
            let proxy = if config.blend > 0.0 {
                fit_proxy(league, week_start)
            } else {
                None
            };
            let training = training_games(
                league,
                week_start,
                config.decay,
                config.blend,
                proxy.as_ref(),
            );
            if training.len() < MIN_TRAINING_GAMES {
                report.skipped_for_lack_of_history += week_games.len();
            } else {
                // Point de départ = ajustement de la semaine précédente : les données ne diffèrent que d'une semaine.
                let fitted = model.fit(league.teams.len(), &training, previous.as_ref())?;
                for game in week_games {
                    match predict(&fitted, game, week_start) {
                        Some((prediction, rho_clamped)) => {
                            report.rho_clamped += usize::from(rho_clamped);
                            report.predictions.push(prediction);
                        }
                        None => report.skipped_invalid += 1,
                    }
                }
                previous = Some(fitted);
            }
        }
        week_start = week_end;
    }
    Ok(report)
}

/// Coefficients buts/tirs estimés sur les matchs strictement antérieurs à `as_of`.
pub(crate) fn fit_proxy(league: &LeagueData, as_of: NaiveDate) -> Option<XgProxy> {
    let mut observations = Vec::new();
    for game in league.games.iter().filter(|g| g.date < as_of) {
        if let Some([(shots_home, on_home), (shots_away, on_away)]) = game.shots {
            observations.push(ShotObservation {
                on_target: on_home,
                shots: shots_home,
                goals: game.goals_home,
            });
            observations.push(ShotObservation {
                on_target: on_away,
                shots: shots_away,
                goals: game.goals_away,
            });
        }
    }
    XgProxy::fit(&observations)
}

fn predict(
    fitted: &FittedModel,
    game: &HistoricalGame,
    as_of: NaiveDate,
) -> Option<(Prediction, bool)> {
    let (home_mean, away_mean) = fitted.expectancies(game.home, game.away);
    let (grid, rho_clamped) = fitted.grid_checked(game.home, game.away)?;
    let probs = grid.home_draw_away()?.to_array();
    let prediction = Prediction {
        game_id: game.game_id,
        as_of,
        home_mean,
        away_mean,
        p_home: probs[0],
        p_draw: probs[1],
        p_away: probs[2],
        p_over25: grid.over(2.5),
        p_btts: grid.both_teams_score(),
    };
    Some((prediction, rho_clamped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Outcome, Probs1x2};
    use crate::eval::metrics::log_loss;
    use crate::models::testing::{simulated_league, truth};

    fn dixon_coles() -> ModelConfig {
        ModelConfig {
            kind: ModelKind::DixonColes,
            decay: 0.002,
            ridge: 1.0,
            blend: 0.0,
        }
    }

    /// Fenêtre : les saisons 7 et 8 simulées ; les six premières (336 matchs) servent de passé.
    fn window(league: &LeagueData) -> Window {
        let first = league.games.iter().map(|g| g.date).min().unwrap() + Days::new(6 * 365);
        Window {
            first,
            last: first + Days::new(2 * 365),
        }
    }

    fn run_one(league: &LeagueData, config: &ModelConfig, window: Window) -> WalkForwardReport {
        run(std::slice::from_ref(league), config, window).unwrap()
    }

    #[test]
    fn every_game_of_the_window_is_predicted_once_with_a_past_only_date() {
        let league = simulated_league(8, 1);
        let window = window(&league);

        let report = run_one(&league, &dixon_coles(), window);

        let expected: Vec<i64> = league
            .games
            .iter()
            .filter(|g| g.date >= window.first && g.date <= window.last)
            .map(|g| g.game_id)
            .collect();
        assert!(!expected.is_empty());
        let mut predicted: Vec<i64> = report.predictions.iter().map(|p| p.game_id).collect();
        predicted.sort_unstable();
        let mut expected_sorted = expected.clone();
        expected_sorted.sort_unstable();
        assert_eq!(predicted, expected_sorted);
        assert_eq!(report.skipped_invalid, 0);
        // as_of est le lundi de la semaine du match : jamais après le match.
        for prediction in &report.predictions {
            let game = league
                .games
                .iter()
                .find(|g| g.game_id == prediction.game_id)
                .unwrap();
            assert_eq!(prediction.as_of, monday_of(game.date));
            assert!(prediction.as_of <= game.date);
        }
    }

    #[test]
    fn changing_future_results_never_changes_an_earlier_prediction() {
        // Test d'absence de fuite du futur (invariant 1) : on réécrit tous les matchs à partir d'une date de
        // coupure ; les prédictions faites AVANT cette date doivent être exactement identiques, au bit près.
        let league = simulated_league(8, 2);
        let window = window(&league);
        let cutoff = window.first + Days::new(200);
        let mut altered = league.clone();
        for game in altered.games.iter_mut().filter(|g| g.date >= cutoff) {
            game.goals_home = 7;
            game.goals_away = 0;
            game.shots = Some([(40, 20), (2, 0)]);
        }

        let original = run_one(&league, &dixon_coles(), window);
        let rewritten = run_one(&altered, &dixon_coles(), window);

        let before_cutoff = |report: &WalkForwardReport| -> Vec<Prediction> {
            report
                .predictions
                .iter()
                .filter(|p| p.as_of < cutoff)
                .cloned()
                .collect()
        };
        let (a, b) = (before_cutoff(&original), before_cutoff(&rewritten));
        assert!(
            a.len() > 20,
            "le test doit couvrir de nombreuses prédictions ({})",
            a.len()
        );
        assert_eq!(a, b);
        // Et le test est bien sensible : après la coupure, les prédictions, elles, changent.
        let after = |report: &WalkForwardReport| -> Vec<Prediction> {
            report
                .predictions
                .iter()
                .filter(|p| p.as_of >= cutoff + Days::new(14))
                .cloned()
                .collect()
        };
        assert_ne!(after(&original), after(&rewritten));
    }

    #[test]
    fn the_same_week_results_are_not_used_to_predict_the_same_week() {
        // Un match du mardi ne sert pas à prédire le samedi de la même semaine : as_of est le lundi.
        let league = simulated_league(8, 3);
        let window = window(&league);
        let report = run_one(&league, &dixon_coles(), window);
        for prediction in &report.predictions {
            let same_week_or_later = league
                .games
                .iter()
                .filter(|g| g.date >= prediction.as_of)
                .count();
            assert!(same_week_or_later > 0);
            assert_eq!(prediction.as_of.weekday().num_days_from_monday(), 0);
        }
    }

    #[test]
    fn walk_forward_is_deterministic() {
        let league = simulated_league(8, 4);
        let window = window(&league);
        let leagues = [league.clone(), simulated_league(8, 5)];

        let a = run(&leagues, &dixon_coles(), window).unwrap();
        let b = run(&leagues, &dixon_coles(), window).unwrap();

        assert_eq!(a, b);
    }

    #[test]
    fn predictions_are_valid_probabilities() {
        let league = simulated_league(8, 6);
        let report = run_one(&league, &dixon_coles(), window(&league));
        for p in &report.predictions {
            assert!((p.p_home + p.p_draw + p.p_away - 1.0).abs() < 1e-9);
            assert!(p.p_over25 > 0.0 && p.p_over25 < 1.0);
            assert!(p.p_btts > 0.0 && p.p_btts < 1.0);
            assert!(p.home_mean > 0.0 && p.away_mean > 0.0);
        }
    }

    #[test]
    fn games_outside_the_window_are_not_predicted() {
        let league = simulated_league(8, 7);
        let window = window(&league);
        let report = run_one(&league, &dixon_coles(), window);
        for p in &report.predictions {
            let game = league
                .games
                .iter()
                .find(|g| g.game_id == p.game_id)
                .unwrap();
            assert!(game.date >= window.first && game.date <= window.last);
        }
    }

    #[test]
    fn too_little_history_means_no_prediction_rather_than_a_bad_one() {
        let league = simulated_league(2, 8); // 112 matchs au total : jamais 300 d'entraînement
        let first = league.games.iter().map(|g| g.date).min().unwrap() + Days::new(365);
        let report = run_one(
            &league,
            &dixon_coles(),
            Window {
                first,
                last: first + Days::new(300),
            },
        );
        assert!(report.predictions.is_empty());
        assert!(report.skipped_for_lack_of_history > 0);
    }

    #[test]
    fn dixon_coles_and_poisson_differ_and_the_shot_blend_changes_predictions() {
        let league = simulated_league(8, 9);
        let window = window(&league);
        let dc = run_one(&league, &dixon_coles(), window);
        let poisson = run_one(
            &league,
            &ModelConfig {
                kind: ModelKind::Poisson,
                ..dixon_coles()
            },
            window,
        );
        let blended = run_one(
            &league,
            &ModelConfig {
                blend: 0.5,
                ..dixon_coles()
            },
            window,
        );

        assert_ne!(dc.predictions[0].p_draw, poisson.predictions[0].p_draw);
        assert_ne!(dc.predictions[0].p_home, blended.predictions[0].p_home);
    }

    #[test]
    fn on_data_simulated_from_known_strengths_the_walk_forward_is_close_to_the_true_model() {
        // Qualité de bout en bout : le walk-forward, qui ne connaît pas la vérité, doit se rapprocher du
        // modèle qui la connaît, et faire nettement mieux qu'une prévision uniforme (log loss de ln 3 ≈ 1,0986).
        let league = simulated_league(10, 10);
        let first = league.games.iter().map(|g| g.date).min().unwrap() + Days::new(6 * 365);
        let window = Window {
            first,
            last: first + Days::new(4 * 365),
        };
        let report = run_one(&league, &dixon_coles(), window);
        let truth = truth();

        let mut ours = 0.0;
        let mut oracle = 0.0;
        for p in &report.predictions {
            let game = league
                .games
                .iter()
                .find(|g| g.game_id == p.game_id)
                .unwrap();
            let outcome = match game.goals_home.cmp(&game.goals_away) {
                std::cmp::Ordering::Greater => Outcome::Home,
                std::cmp::Ordering::Equal => Outcome::Draw,
                std::cmp::Ordering::Less => Outcome::Away,
            };
            ours += log_loss(
                &Probs1x2::new(p.p_home, p.p_draw, p.p_away).unwrap(),
                outcome,
            );
            oracle += log_loss(
                &truth
                    .grid(game.home, game.away)
                    .unwrap()
                    .home_draw_away()
                    .unwrap(),
                outcome,
            );
        }
        let n = report.predictions.len() as f64;
        assert!(n > 100.0);
        let (ours, oracle) = (ours / n, oracle / n);
        assert!(ours < 3.0_f64.ln() - 0.03, "{ours}");
        assert!(
            ours < oracle + 0.03,
            "notre log loss {ours} contre {oracle} pour le vrai modèle"
        );
    }
}
