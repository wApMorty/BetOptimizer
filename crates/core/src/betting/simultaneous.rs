//! Kelly **simultané** : dimensionner ensemble les paris d'un même lot de décision.
//!
//! Plusieurs paris indépendants (un par match, voir la règle « une sélection par match ») se jouent en même temps :
//! la bonne mise de chacun dépend des autres, car le risque total partage la même bankroll. On maximise la
//! croissance logarithmique attendue sur **toutes** les combinaisons d'issues :
//!
//! ```text
//! G(f) = Σ_{S ⊆ {1..n}} P(S) · ln( 1 − Σ_i f_i + Σ_{i ∈ S} f_i·cote_i ),   f_i ≥ 0,  Σ_i f_i < 1
//! ```
//!
//! où `S` est l'ensemble des paris gagnants et `P(S) = Π_{i∈S} p_i · Π_{i∉S} (1 − p_i)`.
//!
//! - jusqu'à [`EXACT_LIMIT`] paris : énumération exacte des `2ⁿ` combinaisons, optimisée par L-BFGS (`argmin`) ;
//! - au-delà : approximation du second ordre, en forme fermée (voir [`approximate_fractions`]).

use argmin::core::{CostFunction, Error as ArgminError, Executor, Gradient, State};
use argmin::solver::linesearch::MoreThuenteLineSearch;
use argmin::solver::quasinewton::LBFGS;

use super::kelly::kelly_fraction;
use crate::domain::{Odds, Probability};
use crate::error::{Error, Result};

/// Nombre maximal de paris traités par énumération exacte (2¹² = 4 096 combinaisons).
pub const EXACT_LIMIT: usize = 12;

/// Part maximale de la bankroll engagée sur un lot. L'optimum exact reste toujours sous 1 (sinon la perte de tous les
/// paris ruinerait la bankroll et `ln` vaudrait −∞) ; l'approximation, elle, doit être ramenée sous ce plafond.
const MAX_TOTAL: f64 = 0.95;

/// Un pari candidat : probabilité estimée et cote proposée.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bet {
    /// Probabilité estimée que le pari gagne.
    pub probability: Probability,
    /// Cote proposée.
    pub odds: Odds,
}

impl Bet {
    fn edge(&self) -> f64 {
        self.probability.get() * self.odds.get() - 1.0
    }
}

/// Fractions de bankroll à miser sur chaque pari (même ordre que `bets`), de somme strictement inférieure à 1.
/// Un pari sans espérance positive reçoit 0.
pub fn fractions(bets: &[Bet]) -> Result<Vec<f64>> {
    let positive: Vec<usize> = (0..bets.len()).filter(|i| bets[*i].edge() > 0.0).collect();
    let selected: Vec<Bet> = positive.iter().map(|i| bets[*i]).collect();
    let optimum = if selected.len() <= EXACT_LIMIT {
        exact_fractions(&selected)?
    } else {
        cap_total(approximate_fractions(&selected))
    };
    let mut result = vec![0.0; bets.len()];
    for (slot, fraction) in positive.iter().zip(optimum) {
        result[*slot] = fraction;
    }
    Ok(result)
}

/// Croissance logarithmique attendue `G(f)` d'un lot **déjà dimensionné** : les mises `f` sont données, et les
/// probabilités de `bets` sont celles que l'on tient pour vraies (pour le backtest, la clôture de Pinnacle sans marge),
/// qui ne sont pas forcément celles qui ont servi à calculer `f`.
///
/// Même formule que l'optimisation ; énumération exacte jusqu'à [`EXACT_LIMIT`] paris, au-delà le développement du
/// second ordre `G ≈ Σ f_i μ_i − ½ Σ f_i² σ_i² − ½ (Σ f_i μ_i)²` (voir [`approximate_fractions`]), très précis pour
/// les petites mises qu'un petit avantage produit.
pub fn expected_log_growth(bets: &[Bet], f: &[f64]) -> f64 {
    if bets.len() <= EXACT_LIMIT {
        return growth_and_gradient(bets, f).0;
    }
    let mut mean = 0.0;
    let mut variance = 0.0;
    for (bet, fraction) in bets.iter().zip(f) {
        let p = bet.probability.get();
        mean += fraction * bet.edge();
        variance += fraction * fraction * bet.odds.get().powi(2) * p * (1.0 - p);
    }
    mean - 0.5 * variance - 0.5 * mean * mean
}

/// Croissance logarithmique attendue `G(f)` par énumération des `2ⁿ` combinaisons d'issues, et son gradient.
fn growth_and_gradient(bets: &[Bet], f: &[f64]) -> (f64, Vec<f64>) {
    let n = bets.len();
    let total: f64 = f.iter().sum();
    let mut growth = 0.0;
    let mut gradient = vec![0.0; n];
    for mask in 0u32..(1u32 << n) {
        let mut probability = 1.0;
        let mut wealth = 1.0 - total;
        for (i, bet) in bets.iter().enumerate() {
            if mask & (1 << i) != 0 {
                probability *= bet.probability.get();
                wealth += f[i] * bet.odds.get();
            } else {
                probability *= 1.0 - bet.probability.get();
            }
        }
        if probability == 0.0 {
            continue;
        }
        growth += probability * wealth.ln();
        for (i, bet) in bets.iter().enumerate() {
            let payoff = if mask & (1 << i) != 0 {
                bet.odds.get() - 1.0
            } else {
                -1.0
            };
            gradient[i] += probability * payoff / wealth;
        }
    }
    (growth, gradient)
}

/// Le problème d'optimisation. Les fractions sont paramétrées par `f_i = e^{x_i} / (1 + Σ_k e^{x_k})` : elles sont
/// automatiquement positives et de somme < 1, donc la richesse reste strictement positive dans toutes les
/// combinaisons et aucune contrainte n'est à gérer. Un pari à abandonner correspond à `x_i → −∞`.
struct SimultaneousProblem<'a> {
    bets: &'a [Bet],
}

impl SimultaneousProblem<'_> {
    fn fractions_of(x: &[f64]) -> Vec<f64> {
        let denominator = 1.0 + x.iter().map(|v| v.exp()).sum::<f64>();
        x.iter().map(|v| v.exp() / denominator).collect()
    }

    /// `−G` et son gradient par rapport à `x` : `∂G/∂x_k = f_k · (g_k − Σ_i g_i f_i)` avec `g = ∂G/∂f`.
    fn evaluate(&self, x: &[f64]) -> (f64, Vec<f64>) {
        let f = Self::fractions_of(x);
        let (growth, g) = growth_and_gradient(self.bets, &f);
        let weighted: f64 = g.iter().zip(&f).map(|(gi, fi)| gi * fi).sum();
        let gradient_x: Vec<f64> = f
            .iter()
            .zip(&g)
            .map(|(fk, gk)| -(fk * (gk - weighted)))
            .collect();
        (-growth, gradient_x)
    }
}

impl CostFunction for SimultaneousProblem<'_> {
    type Param = Vec<f64>;
    type Output = f64;

    fn cost(&self, x: &Vec<f64>) -> std::result::Result<f64, ArgminError> {
        Ok(self.evaluate(x).0)
    }
}

impl Gradient for SimultaneousProblem<'_> {
    type Param = Vec<f64>;
    type Gradient = Vec<f64>;

    fn gradient(&self, x: &Vec<f64>) -> std::result::Result<Vec<f64>, ArgminError> {
        Ok(self.evaluate(x).1)
    }
}

/// Optimum exact par énumération (au plus [`EXACT_LIMIT`] paris, tous d'espérance positive).
pub(crate) fn exact_fractions(bets: &[Bet]) -> Result<Vec<f64>> {
    if bets.is_empty() {
        return Ok(Vec::new());
    }
    // Départ : fractions de Kelly de chaque pari seul, ramenées à une somme de 0,9 au plus.
    let singles: Vec<f64> = bets
        .iter()
        .map(|b| kelly_fraction(b.probability, b.odds).max(1e-4))
        .collect();
    let total: f64 = singles.iter().sum();
    let scale = if total > 0.9 { 0.9 / total } else { 1.0 };
    let start: Vec<f64> = singles.iter().map(|f| f * scale).collect();
    let slack = 1.0 - start.iter().sum::<f64>();
    let initial: Vec<f64> = start.iter().map(|f| (f / slack).ln()).collect();

    let problem = SimultaneousProblem { bets };
    let line_search: MoreThuenteLineSearch<Vec<f64>, Vec<f64>, f64> = MoreThuenteLineSearch::new();
    let solver = LBFGS::new(line_search, 8)
        .with_tolerance_grad(1e-9)
        .and_then(|solver| solver.with_tolerance_cost(1e-14))
        .map_err(|e| Error::Optimizer(e.to_string()))?;
    let result = Executor::new(problem, solver)
        .configure(|state| state.param(initial).max_iters(300))
        .run()
        .map_err(|e| Error::Optimizer(e.to_string()))?;
    let best = result
        .state()
        .get_best_param()
        .cloned()
        .ok_or_else(|| Error::Optimizer("aucun paramètre retenu".to_string()))?;
    let fractions = SimultaneousProblem::fractions_of(&best);
    if fractions.iter().any(|f| !f.is_finite()) {
        return Err(Error::Optimizer("fractions non finies".to_string()));
    }
    // Un pari abandonné par l'optimum (fraction infime) est ramené à 0.
    Ok(fractions
        .into_iter()
        .map(|f| if f < 1e-6 { 0.0 } else { f })
        .collect())
}

/// Approximation du second ordre pour plus de [`EXACT_LIMIT`] paris : `ln W ≈ X − X²/2` avec `X = Σ f_i·r_i`
/// (`r_i` : gain net d'un euro misé sur le pari `i`).
///
/// Avec `μ_i = p_i·cote_i − 1` et `σ_i² = cote_i²·p_i·(1 − p_i)` (moyenne et variance de `r_i`, paris indépendants),
/// `G ≈ Σ f_i μ_i − ½ Σ f_i² σ_i² − ½ (Σ f_i μ_i)²`, maximal en
///
/// ```text
/// f_i = (μ_i / σ_i²) / (1 + Q),     Q = Σ_k μ_k² / σ_k²
/// ```
///
/// Le premier facteur est la mise de Kelly approchée du pari seul ; le diviseur `1 + Q` la réduit d'autant plus que
/// l'avantage total du lot est grand (le risque est partagé). Rien ne garantit toutefois que la somme reste sous 1
/// quand le lot est grand et que les avantages annoncés sont élevés : [`cap_total`] s'en charge.
pub(crate) fn approximate_fractions(bets: &[Bet]) -> Vec<f64> {
    let moments: Vec<(f64, f64)> = bets
        .iter()
        .map(|b| {
            let p = b.probability.get();
            (b.edge(), b.odds.get().powi(2) * p * (1.0 - p))
        })
        .collect();
    let q: f64 = moments
        .iter()
        .map(|(mu, variance)| mu * mu / variance)
        .sum();
    moments
        .iter()
        .map(|(mu, variance)| (mu / variance) / (1.0 + q))
        .collect()
}

/// Ramène la somme des fractions à [`MAX_TOTAL`] au plus, proportionnellement (les rapports entre mises sont conservés).
fn cap_total(mut fractions: Vec<f64>) -> Vec<f64> {
    let total: f64 = fractions.iter().sum();
    if total > MAX_TOTAL {
        let scale = MAX_TOTAL / total;
        for fraction in &mut fractions {
            *fraction *= scale;
        }
    }
    fractions
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn bet(p: f64, odds: f64) -> Bet {
        Bet {
            probability: Probability::new(p).unwrap(),
            odds: Odds::new(odds).unwrap(),
        }
    }

    fn growth(bets: &[Bet], f: &[f64]) -> f64 {
        growth_and_gradient(bets, f).0
    }

    #[test]
    fn the_expected_growth_of_given_stakes_follows_the_truth_and_both_regimes_agree() {
        // Un pari : f = 10 % à la cote 2,0 ; si p = 0,5 (pas d'avantage), G = ½ ln 1,1 + ½ ln 0,9 < 0.
        let fair = [bet(0.5, 2.0)];
        assert_abs_diff_eq!(
            expected_log_growth(&fair, &[0.1]),
            0.5 * 1.1_f64.ln() + 0.5 * 0.9_f64.ln(),
            epsilon = 1e-12
        );
        // Aucune mise : aucune croissance.
        assert_eq!(expected_log_growth(&fair, &[0.0]), 0.0);
        // Au-delà de la limite exacte, l'approximation reste proche de l'énumération (petites mises).
        let many: Vec<Bet> = (0..EXACT_LIMIT + 1).map(|_| bet(0.52, 2.0)).collect();
        let stakes = vec![0.01; many.len()];
        let exact = growth_and_gradient(&many, &stakes).0;
        assert_abs_diff_eq!(expected_log_growth(&many, &stakes), exact, epsilon = 1e-5);
    }

    #[test]
    fn one_bet_gives_the_single_bet_kelly_fraction() {
        for (p, odds) in [(0.6, 2.0), (0.4, 3.0), (0.25, 5.2)] {
            let simultaneous = fractions(&[bet(p, odds)]).unwrap()[0];
            let single = kelly_fraction(Probability::new(p).unwrap(), Odds::new(odds).unwrap());
            assert_abs_diff_eq!(simultaneous, single, epsilon = 1e-5);
        }
    }

    #[test]
    fn two_identical_bets_share_the_risk_symmetrically() {
        let bets = [bet(0.6, 2.0), bet(0.6, 2.0)];

        let f = fractions(&bets).unwrap();

        assert_abs_diff_eq!(f[0], f[1], epsilon = 1e-6);
        // Chacun est un peu moins misé que seul (0,2) : le risque total est partagé.
        assert!(f[0] < 0.2 && f[0] > 0.15, "{f:?}");
        assert!(f.iter().sum::<f64>() < 1.0);
        // Et c'est bien un optimum : mieux que deux fois la fraction du pari seul.
        assert!(growth(&bets, &f) > growth(&bets, &[0.2, 0.2]));
    }

    #[test]
    fn the_exact_optimum_beats_the_independent_kelly_stakes() {
        let bets = [
            bet(0.55, 2.0),
            bet(0.40, 2.9),
            bet(0.30, 4.0),
            bet(0.62, 1.8),
            bet(0.22, 5.5),
        ];
        let independent: Vec<f64> = bets
            .iter()
            .map(|b| kelly_fraction(b.probability, b.odds))
            .collect();

        let f = fractions(&bets).unwrap();

        assert!(f.iter().sum::<f64>() < 1.0);
        assert!(growth(&bets, &f) >= growth(&bets, &independent) - 1e-12);
        // Un gradient nul à l'optimum : aucune petite variation ne fait mieux.
        for i in 0..bets.len() {
            let mut nudged = f.clone();
            nudged[i] += 1e-4;
            assert!(
                growth(&bets, &nudged) <= growth(&bets, &f) + 1e-9,
                "pari {i}"
            );
        }
    }

    #[test]
    fn bets_without_positive_expectation_get_nothing() {
        let f = fractions(&[bet(0.5, 1.9), bet(0.6, 2.0), bet(0.3, 3.0)]).unwrap();
        assert_eq!(f[0], 0.0);
        assert_eq!(f[2], 0.0);
        assert!(f[1] > 0.0);
        assert_eq!(fractions(&[]).unwrap(), Vec::<f64>::new());
    }

    #[test]
    fn the_analytic_gradient_matches_finite_differences() {
        let bets = [bet(0.55, 2.0), bet(0.40, 2.9), bet(0.30, 4.0)];
        let problem = SimultaneousProblem { bets: &bets };
        let x = vec![-2.3, -2.8, -3.1];

        let (_, analytic) = problem.evaluate(&x);

        for i in 0..x.len() {
            let h = 1e-6;
            let (mut plus, mut minus) = (x.clone(), x.clone());
            plus[i] += h;
            minus[i] -= h;
            let numeric = (problem.evaluate(&plus).0 - problem.evaluate(&minus).0) / (2.0 * h);
            assert_abs_diff_eq!(analytic[i], numeric, epsilon = 1e-6);
        }
    }

    #[test]
    fn the_approximation_is_close_to_the_exact_solution_for_small_edges() {
        // Avantages de 2 % à 5 % : le cas réaliste.
        let bets: Vec<Bet> = (0..8)
            .map(|i| {
                let odds = 1.8 + 0.25 * f64::from(i);
                bet((1.0 + 0.02 + 0.005 * f64::from(i)) / odds, odds)
            })
            .collect();

        let exact = exact_fractions(&bets).unwrap();
        let approx = approximate_fractions(&bets);

        for (e, a) in exact.iter().zip(&approx) {
            assert!((e - a).abs() / e < 0.10, "exact {e}, approché {a}");
        }
        // Et la croissance obtenue est quasi la même.
        assert!(growth(&bets, &exact) >= growth(&bets, &approx));
        assert!(growth(&bets, &exact) - growth(&bets, &approx) < 1e-4);
    }

    #[test]
    fn beyond_the_exact_limit_the_approximation_is_used_and_stays_feasible() {
        let bets: Vec<Bet> = (0..15)
            .map(|i| bet(0.45 + 0.01 * f64::from(i % 3), 2.4))
            .collect();

        let f = fractions(&bets).unwrap();

        assert_eq!(f.len(), 15);
        assert!(f.iter().all(|x| *x > 0.0));
        assert!(f.iter().sum::<f64>() < 1.0);
        // Même lot que l'approximation plafonnée, calculée directement.
        let direct = cap_total(approximate_fractions(&bets));
        for (a, b) in f.iter().zip(&direct) {
            assert_abs_diff_eq!(a, b, epsilon = 1e-12);
        }
    }

    #[test]
    fn a_large_batch_with_large_edges_is_capped_below_the_whole_bankroll() {
        // Quarante paris à fort avantage : l'approximation brute dépasse 1 (≈ 2,8), le plafond la ramène à 0,95.
        let bets: Vec<Bet> = (0..40).map(|_| bet(0.55, 2.2)).collect();
        let raw: f64 = approximate_fractions(&bets).iter().sum();
        assert!(
            raw > 1.0,
            "le cas de test doit dépasser 1 sans plafond : {raw}"
        );

        let f = fractions(&bets).unwrap();

        assert_abs_diff_eq!(f.iter().sum::<f64>(), MAX_TOTAL, epsilon = 1e-12);
        assert!(
            f.iter().all(|x| (*x - f[0]).abs() < 1e-12),
            "mises égales conservées"
        );
    }

    #[test]
    fn the_total_staked_never_reaches_the_whole_bankroll() {
        // Douze paris à très gros avantage : la somme doit rester sous 1 sans plafond explicite.
        let bets: Vec<Bet> = (0..12).map(|_| bet(0.7, 2.5)).collect();
        let f = fractions(&bets).unwrap();
        assert!(f.iter().sum::<f64>() < 1.0, "{}", f.iter().sum::<f64>());
        assert!(f.iter().all(|x| *x >= 0.0));
    }
}
