//! Vraisemblance pondérée et pénalisée des modèles de scores, avec son gradient analytique, et ajustement
//! par L-BFGS (crate `argmin`).
//!
//! On **minimise** l'opposé de la log-vraisemblance :
//!
//! ```text
//! J(θ) = − Σ_k w_k · [ y_h·η_h − e^{η_h} + y_a·η_a − e^{η_a} + ln τ(g_h, g_a; λ, μ, ρ) ]  +  (κ/2) · Σ_i (a_i² + d_i²)
//! ```
//!
//! avec `η_h = base + avantage + a_dom + d_ext`, `η_a = base + a_ext + d_dom`, `λ = e^{η_h}`, `μ = e^{η_a}`.
//! Les termes `ln(y!)` ne dépendent pas des paramètres : ils sont omis. `y` est la cible de la partie Poisson
//! (les buts, ou des buts attendus non entiers : la forme `y·η − e^η` reste valable) ; `g` les buts réels,
//! seuls à entrer dans `τ`. Sources : Dixon & Coles (1997), *Applied Statistics* 46(2), pondération `w` de la
//! section 4 ; pénalité L2 = loi a priori gaussienne centrée sur les paramètres d'équipe.

use argmin::core::{CostFunction, Error as ArgminError, Executor, Gradient, State};
use argmin::solver::linesearch::MoreThuenteLineSearch;
use argmin::solver::quasinewton::LBFGS;

use super::{FittedModel, TrainingGame};
use crate::error::{Error, Result};

/// Borne de `|ρ|` : `ρ = RHO_MAX · tanh(φ)` où `φ` est le paramètre réellement optimisé (sans contrainte).
/// Les valeurs ajustées sont de l'ordre de −0,1 : la borne n'est jamais atteinte en pratique.
const RHO_MAX: f64 = 0.5;

/// Plancher de `τ` dans le logarithme (même principe que `penaltyblog`) : évite `ln(τ ≤ 0)`.
const TAU_FLOOR: f64 = 1e-10;

/// Itérations maximales de L-BFGS.
const MAX_ITERATIONS: u64 = 500;

/// Le problème d'optimisation d'une ligue : données, intensité de la pénalité, et `ρ` libre ou fixé à 0.
pub struct LikelihoodProblem<'a> {
    teams: usize,
    games: &'a [TrainingGame],
    ridge: f64,
    fit_rho: bool,
}

impl<'a> LikelihoodProblem<'a> {
    /// Définit le problème d'une ligue (réservé aux tests de référence).
    #[cfg(test)]
    pub(crate) fn new(teams: usize, games: &'a [TrainingGame], ridge: f64, fit_rho: bool) -> Self {
        LikelihoodProblem {
            teams,
            games,
            ridge,
            fit_rho,
        }
    }

    /// Nombre de paramètres : attaque et défense de chaque équipe, base, avantage, et `φ` si `ρ` est libre.
    fn parameter_count(&self) -> usize {
        2 * self.teams + 2 + usize::from(self.fit_rho)
    }

    /// Valeur de `J` et son gradient en `params`, calculés ensemble (les dérivées partagent leurs termes).
    pub fn evaluate(&self, params: &[f64]) -> (f64, Vec<f64>) {
        let n = self.teams;
        let (attack, defence) = (&params[..n], &params[n..2 * n]);
        let (base, advantage) = (params[2 * n], params[2 * n + 1]);
        let phi = if self.fit_rho { params[2 * n + 2] } else { 0.0 };
        let tanh = phi.tanh();
        let rho = if self.fit_rho { RHO_MAX * tanh } else { 0.0 };

        let mut value = 0.0;
        let mut gradient = vec![0.0; self.parameter_count()];
        let mut rho_gradient = 0.0;
        for game in self.games {
            let eta_home = base + advantage + attack[game.home] + defence[game.away];
            let eta_away = base + attack[game.away] + defence[game.home];
            let (home_mean, away_mean) = (eta_home.exp(), eta_away.exp());

            // Partie Poisson : f = y·η − e^η ; ∂f/∂η = y − e^η.
            let mut log_likelihood =
                game.target_home * eta_home - home_mean + game.target_away * eta_away - away_mean;
            let mut d_eta_home = game.target_home - home_mean;
            let mut d_eta_away = game.target_away - away_mean;

            // Partie corrélation des scores faibles : ln τ et ses dérivées par rapport à η_h, η_a et ρ.
            if self.fit_rho {
                let tau_terms =
                    low_score_terms(game.goals_home, game.goals_away, home_mean, away_mean, rho);
                if let Some((tau, d_home, d_away, d_rho)) = tau_terms {
                    if tau > TAU_FLOOR {
                        log_likelihood += tau.ln();
                        d_eta_home += d_home / tau;
                        d_eta_away += d_away / tau;
                        rho_gradient += game.weight * d_rho / tau;
                    } else {
                        log_likelihood += TAU_FLOOR.ln();
                    }
                }
            }

            let w = game.weight;
            value -= w * log_likelihood;
            gradient[game.home] -= w * d_eta_home;
            gradient[n + game.away] -= w * d_eta_home;
            gradient[game.away] -= w * d_eta_away;
            gradient[n + game.home] -= w * d_eta_away;
            gradient[2 * n] -= w * (d_eta_home + d_eta_away);
            gradient[2 * n + 1] -= w * d_eta_home;
        }

        // Pénalité L2 sur les forces d'équipe (ni la base ni l'avantage du terrain ne sont pénalisés).
        for i in 0..n {
            value += 0.5 * self.ridge * (attack[i].powi(2) + defence[i].powi(2));
            gradient[i] += self.ridge * attack[i];
            gradient[n + i] += self.ridge * defence[i];
        }
        if self.fit_rho {
            // ∂J/∂φ = (∂J/∂ρ) · RHO_MAX · (1 − tanh² φ), et ∂J/∂ρ = −Σ w · ∂ln τ/∂ρ.
            gradient[2 * n + 2] = -rho_gradient * RHO_MAX * (1.0 - tanh * tanh);
        }
        (value, gradient)
    }
}

/// `(τ, ∂τ/∂η_h, ∂τ/∂η_a, ∂τ/∂ρ)` pour les quatre scores corrigés ; `None` pour tous les autres (τ = 1).
///
/// Comme `λ = e^{η_h}`, `∂λ/∂η_h = λ` : par exemple `τ(0,0) = 1 − λμρ` donne `∂τ/∂η_h = −λμρ`.
fn low_score_terms(
    goals_home: u8,
    goals_away: u8,
    home_mean: f64,
    away_mean: f64,
    rho: f64,
) -> Option<(f64, f64, f64, f64)> {
    match (goals_home, goals_away) {
        (0, 0) => Some((
            1.0 - home_mean * away_mean * rho,
            -home_mean * away_mean * rho,
            -home_mean * away_mean * rho,
            -home_mean * away_mean,
        )),
        (0, 1) => Some((1.0 + home_mean * rho, home_mean * rho, 0.0, home_mean)),
        (1, 0) => Some((1.0 + away_mean * rho, 0.0, away_mean * rho, away_mean)),
        (1, 1) => Some((1.0 - rho, 0.0, 0.0, -1.0)),
        _ => None,
    }
}

// Les deux traits d'`argmin` : la fonction de coût et son gradient. `argmin` n'a pas besoin d'autre chose.
impl CostFunction for LikelihoodProblem<'_> {
    type Param = Vec<f64>;
    type Output = f64;

    fn cost(&self, params: &Vec<f64>) -> std::result::Result<f64, ArgminError> {
        Ok(self.evaluate(params).0)
    }
}

impl Gradient for LikelihoodProblem<'_> {
    type Param = Vec<f64>;
    type Gradient = Vec<f64>;

    fn gradient(&self, params: &Vec<f64>) -> std::result::Result<Vec<f64>, ArgminError> {
        Ok(self.evaluate(params).1)
    }
}

/// Ajuste un modèle (Poisson si `fit_rho` est faux, Dixon-Coles sinon) par L-BFGS.
pub fn fit_model(
    teams: usize,
    games: &[TrainingGame],
    ridge: f64,
    fit_rho: bool,
    start: Option<&FittedModel>,
) -> Result<FittedModel> {
    let problem = LikelihoodProblem {
        teams,
        games,
        ridge,
        fit_rho,
    };
    let initial = match start {
        Some(previous) if previous.attack.len() == teams => pack(previous, fit_rho),
        _ => pack(&initial_guess(teams, games), fit_rho),
    };

    let line_search: MoreThuenteLineSearch<Vec<f64>, Vec<f64>, f64> = MoreThuenteLineSearch::new();
    let solver = LBFGS::new(line_search, 10)
        .with_tolerance_grad(1e-5)
        .and_then(|solver| solver.with_tolerance_cost(1e-10))
        .map_err(|e| Error::Optimizer(e.to_string()))?;
    let result = Executor::new(problem, solver)
        .configure(|state| state.param(initial).max_iters(MAX_ITERATIONS))
        .run()
        .map_err(|e| Error::Optimizer(e.to_string()))?;
    let best = result
        .state()
        .get_best_param()
        .cloned()
        .ok_or_else(|| Error::Optimizer("aucun paramètre retenu".to_string()))?;
    if best.iter().any(|value| !value.is_finite()) {
        return Err(Error::Optimizer("paramètres non finis".to_string()));
    }
    Ok(unpack(&best, teams, fit_rho))
}

/// Point de départ neutre : équipes moyennes, base = log du nombre moyen de buts par équipe, avantage modéré.
fn initial_guess(teams: usize, games: &[TrainingGame]) -> FittedModel {
    let weight: f64 = games.iter().map(|g| g.weight).sum();
    let mean_target = if weight > 0.0 {
        games
            .iter()
            .map(|g| g.weight * (g.target_home + g.target_away))
            .sum::<f64>()
            / (2.0 * weight)
    } else {
        1.3
    };
    FittedModel {
        attack: vec![0.0; teams],
        defence: vec![0.0; teams],
        intercept: mean_target.max(0.1).ln(),
        home_advantage: 0.2,
        rho: -0.1,
    }
}

/// Paramètres du modèle → vecteur d'optimisation (`ρ` passe par `φ = atanh(ρ / RHO_MAX)`).
fn pack(model: &FittedModel, fit_rho: bool) -> Vec<f64> {
    let mut params = Vec::with_capacity(2 * model.attack.len() + 3);
    params.extend_from_slice(&model.attack);
    params.extend_from_slice(&model.defence);
    params.push(model.intercept);
    params.push(model.home_advantage);
    if fit_rho {
        params.push((model.rho / RHO_MAX).clamp(-0.999, 0.999).atanh());
    }
    params
}

/// Vecteur d'optimisation → paramètres du modèle.
fn unpack(params: &[f64], teams: usize, fit_rho: bool) -> FittedModel {
    FittedModel {
        attack: params[..teams].to_vec(),
        defence: params[teams..2 * teams].to_vec(),
        intercept: params[2 * teams],
        home_advantage: params[2 * teams + 1],
        rho: if fit_rho {
            RHO_MAX * params[2 * teams + 2].tanh()
        } else {
            0.0
        },
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;
    use crate::models::MatchModel;
    use crate::models::dixon_coles::DixonColes;
    use crate::models::grid::ScoreGrid;
    use crate::models::poisson::Poisson;

    /// Générateur congruentiel (Knuth, MMIX) : données de test déterministes, sans dépendance.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }
    }

    /// Tire un score dans une grille (inversion de la fonction de répartition sur les cellules).
    fn sample_score(grid: &ScoreGrid, rng: &mut Lcg) -> (u8, u8) {
        let mut remaining = rng.next();
        for x in 0..=10 {
            for y in 0..=10 {
                remaining -= grid.probability(x, y);
                if remaining <= 0.0 {
                    return (x as u8, y as u8);
                }
            }
        }
        (0, 0)
    }

    fn game(home: usize, away: usize, goals: (u8, u8), weight: f64) -> TrainingGame {
        TrainingGame {
            home,
            away,
            goals_home: goals.0,
            goals_away: goals.1,
            target_home: f64::from(goals.0),
            target_away: f64::from(goals.1),
            weight,
        }
    }

    /// Vérité terrain : 8 équipes aux forces connues, ρ connu.
    fn truth() -> FittedModel {
        FittedModel {
            attack: vec![0.35, 0.2, 0.1, 0.0, -0.05, -0.15, -0.2, -0.25],
            defence: vec![-0.3, -0.15, -0.05, 0.0, 0.05, 0.1, 0.2, 0.25],
            intercept: 0.1,
            home_advantage: 0.3,
            rho: -0.12,
        }
    }

    /// Simule `rounds` championnats aller-retour à 8 équipes depuis la vérité terrain.
    fn simulate(truth: &FittedModel, rounds: usize, seed: u64) -> Vec<TrainingGame> {
        let mut rng = Lcg(seed);
        let mut games = Vec::new();
        for _ in 0..rounds {
            for home in 0..8 {
                for away in 0..8 {
                    if home != away {
                        let grid = truth.grid(home, away).unwrap();
                        games.push(game(home, away, sample_score(&grid, &mut rng), 1.0));
                    }
                }
            }
        }
        games
    }

    #[test]
    fn the_analytic_gradient_matches_finite_differences() {
        // Matchs couvrant les quatre scores corrigés, des scores élevés, des cibles non entières et des poids variés.
        let mut games = vec![
            game(0, 1, (0, 0), 1.0),
            game(1, 2, (0, 1), 0.8),
            game(2, 0, (1, 0), 0.6),
            game(0, 2, (1, 1), 0.9),
            game(1, 0, (3, 2), 0.7),
            game(2, 1, (0, 4), 1.0),
        ];
        games[4].target_home = 2.37;
        games[4].target_away = 1.12;
        let problem = LikelihoodProblem {
            teams: 3,
            games: &games,
            ridge: 1.7,
            fit_rho: true,
        };
        let params = vec![0.2, -0.1, 0.05, -0.15, 0.1, 0.0, 0.1, 0.25, -0.3];

        let (_, analytic) = problem.evaluate(&params);

        for i in 0..params.len() {
            let h = 1e-6;
            let (mut plus, mut minus) = (params.clone(), params.clone());
            plus[i] += h;
            minus[i] -= h;
            let numeric = (problem.evaluate(&plus).0 - problem.evaluate(&minus).0) / (2.0 * h);
            assert_abs_diff_eq!(analytic[i], numeric, epsilon = 1e-5 * (1.0 + numeric.abs()));
        }
    }

    #[test]
    fn the_gradient_without_rho_has_one_fewer_component_and_still_matches() {
        let games = vec![
            game(0, 1, (2, 1), 1.0),
            game(1, 0, (0, 0), 0.5),
            game(0, 1, (1, 3), 0.9),
        ];
        let problem = LikelihoodProblem {
            teams: 2,
            games: &games,
            ridge: 0.5,
            fit_rho: false,
        };
        let params = vec![0.1, -0.1, 0.05, 0.0, 0.2, 0.15];

        let (_, analytic) = problem.evaluate(&params);

        assert_eq!(analytic.len(), 6);
        for i in 0..params.len() {
            let h = 1e-6;
            let (mut plus, mut minus) = (params.clone(), params.clone());
            plus[i] += h;
            minus[i] -= h;
            let numeric = (problem.evaluate(&plus).0 - problem.evaluate(&minus).0) / (2.0 * h);
            assert_abs_diff_eq!(analytic[i], numeric, epsilon = 1e-5 * (1.0 + numeric.abs()));
        }
    }

    #[test]
    fn poisson_fit_recovers_the_expected_goals_of_a_simulated_league() {
        let truth = FittedModel {
            rho: 0.0,
            ..truth()
        };
        let games = simulate(&truth, 100, 11); // 5 600 matchs

        let fitted = Poisson { ridge: 0.01 }.fit(8, &games, None).unwrap();

        // On compare les buts attendus (invariants au choix de normalisation), pas les paramètres bruts.
        let mut worst: f64 = 0.0;
        for home in 0..8 {
            for away in 0..8 {
                if home != away {
                    let (true_home, true_away) = truth.expectancies(home, away);
                    let (est_home, est_away) = fitted.expectancies(home, away);
                    worst = worst
                        .max((est_home / true_home - 1.0).abs())
                        .max((est_away / true_away - 1.0).abs());
                }
            }
        }
        assert!(worst < 0.10, "erreur relative maximale {worst}");
        assert_eq!(fitted.rho, 0.0);
    }

    #[test]
    fn dixon_coles_fit_recovers_rho_and_the_home_advantage() {
        let truth = truth();
        let games = simulate(&truth, 300, 7); // 16 800 matchs

        let fitted = DixonColes { ridge: 0.01 }.fit(8, &games, None).unwrap();

        assert_abs_diff_eq!(fitted.rho, truth.rho, epsilon = 0.04);
        assert_abs_diff_eq!(fitted.home_advantage, truth.home_advantage, epsilon = 0.06);
        assert!(
            fitted.rho < 0.0,
            "la corrélation des scores faibles est négative dans les données simulées"
        );
    }

    #[test]
    fn dixon_coles_finds_no_correlation_in_independent_poisson_data() {
        let truth = FittedModel {
            rho: 0.0,
            ..truth()
        };
        let games = simulate(&truth, 300, 21);

        let fitted = DixonColes { ridge: 0.01 }.fit(8, &games, None).unwrap();

        assert_abs_diff_eq!(fitted.rho, 0.0, epsilon = 0.04);
    }

    #[test]
    fn a_team_without_any_game_is_shrunk_to_the_league_average() {
        // 3 équipes jouent entre elles ; la 4e (indice 3) n'a aucun match : c'est une équipe promue.
        let mut games = Vec::new();
        for _ in 0..30 {
            games.push(game(0, 1, (3, 0), 1.0));
            games.push(game(1, 0, (0, 2), 1.0));
            games.push(game(0, 2, (2, 1), 1.0));
            games.push(game(2, 0, (1, 2), 1.0));
            games.push(game(1, 2, (1, 1), 1.0));
            games.push(game(2, 1, (1, 1), 1.0));
        }

        let fitted = DixonColes { ridge: 2.0 }.fit(4, &games, None).unwrap();

        assert_abs_diff_eq!(fitted.attack[3], 0.0, epsilon = 1e-6);
        assert_abs_diff_eq!(fitted.defence[3], 0.0, epsilon = 1e-6);
        assert!(
            fitted.attack[0].abs() > 0.1,
            "les équipes observées s'écartent de la moyenne"
        );
    }

    #[test]
    fn a_stronger_ridge_pulls_every_team_closer_to_the_average() {
        let games = simulate(&truth(), 5, 3);
        let spread = |model: &FittedModel| model.attack.iter().map(|a| a * a).sum::<f64>();

        let loose = Poisson { ridge: 0.1 }.fit(8, &games, None).unwrap();
        let tight = Poisson { ridge: 50.0 }.fit(8, &games, None).unwrap();

        assert!(spread(&tight) < spread(&loose));
    }

    #[test]
    fn a_game_with_zero_weight_is_ignored() {
        let base = simulate(&truth(), 10, 5);
        let mut with_noise = base.clone();
        with_noise.push(game(0, 1, (9, 9), 0.0));

        let a = Poisson { ridge: 0.5 }.fit(8, &base, None).unwrap();
        let b = Poisson { ridge: 0.5 }.fit(8, &with_noise, None).unwrap();

        for i in 0..8 {
            assert_abs_diff_eq!(a.attack[i], b.attack[i], epsilon = 1e-6);
        }
    }

    #[test]
    fn time_weights_make_the_model_follow_recent_form() {
        // L'équipe 0 marquait beaucoup (3 buts) dans le passé et peu (0 but) récemment.
        let mut old_and_new = Vec::new();
        for _ in 0..40 {
            for other in 1..4 {
                old_and_new.push(game(0, other, (3, 1), 1.0));
                old_and_new.push(game(other, 0, (1, 3), 1.0));
                old_and_new.push(game(other, (other % 3) + 1, (1, 1), 1.0));
            }
        }
        let recent: Vec<TrainingGame> = old_and_new
            .iter()
            .map(|g| TrainingGame { weight: 1.0, ..*g })
            .collect();
        let mut weighted = old_and_new.clone();
        for g in weighted.iter_mut() {
            g.weight = 0.02; // le passé pèse presque rien
        }
        for other in 1..4 {
            for _ in 0..40 {
                weighted.push(game(0, other, (0, 1), 1.0));
                weighted.push(game(other, 0, (1, 0), 1.0));
            }
        }

        let flat = Poisson { ridge: 0.5 }.fit(4, &recent, None).unwrap();
        let decayed = Poisson { ridge: 0.5 }.fit(4, &weighted, None).unwrap();

        assert!(
            decayed.attack[0] < flat.attack[0] - 0.3,
            "{} contre {}",
            decayed.attack[0],
            flat.attack[0]
        );
    }

    #[test]
    fn fitting_twice_gives_identical_parameters() {
        let games = simulate(&truth(), 20, 9);
        let model = DixonColes { ridge: 1.0 };
        assert_eq!(
            model.fit(8, &games, None).unwrap(),
            model.fit(8, &games, None).unwrap()
        );
    }

    #[test]
    fn a_warm_start_reaches_the_same_optimum() {
        let games = simulate(&truth(), 20, 13);
        let model = DixonColes { ridge: 1.0 };
        let cold = model.fit(8, &games, None).unwrap();

        let warm = model.fit(8, &games, Some(&cold)).unwrap();

        for i in 0..8 {
            assert_abs_diff_eq!(cold.attack[i], warm.attack[i], epsilon = 1e-4);
        }
        assert_abs_diff_eq!(cold.rho, warm.rho, epsilon = 1e-4);
    }
}
