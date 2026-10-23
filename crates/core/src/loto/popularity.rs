//! Préférences du public pour les numéros du Loto, mesurées sur le nombre de gagnants de chaque rang.
//!
//! **Modèle produit.** Chaque numéro `i` a un poids `a_i > 0` et chaque numéro chance `j` une probabilité `β_j`. Un joueur
//! choisit la combinaison `S` avec une probabilité proportionnelle à `Π_{i∈S} a_i`, et sa chance indépendamment. Au tirage
//! `D` (chance `c`), la part des joueurs qui trouvent exactement `m` numéros est
//!
//! ```text
//! π_m(D) = e_m(a_D) · e_{5−m}(a_{D̄}) / e_5(a)
//! ```
//!
//! où `e_r(·)` est le polynôme symétrique élémentaire de degré `r` des poids d'un ensemble de numéros, `a_D` les poids des
//! numéros tirés et `a_{D̄}` ceux des 44 autres (on choisit `m` numéros parmi les tirés et `5 − m` parmi les autres). Le
//! rang `(m, chance)` reçoit `π_m(D) · β_c`, le rang `(m, sans chance)` `π_m(D) · (1 − β_c)`. Avec des poids égaux, on
//! retrouve les probabilités du règlement.
//!
//! **Vraisemblance.** Le nombre de grilles `N_d` de chaque tirage est inconnu ; à `N_d` maximisé, la vraisemblance de
//! Poisson des gagnants `n_dk` devient celle d'une loi multinomiale sur les rangs gagnants (le dénominateur `e_5(a)`
//! s'y simplifie) :
//!
//! ```text
//! ℓ_d = Σ_k n_dk · ln q_dk − n_d· · ln Σ_k q_dk,    q_dk = u_{m_k}(D) · γ_k,   u_m(D) = e_m(a_D) · e_{5−m}(a_{D̄})
//! ```
//!
//! On maximise `Σ_d ℓ_d` moins une pénalité L2 (loi a priori normale d'écart type [`PRIOR_SD`] sur `ln a_i` et sur les
//! logits de `β`), par L-BFGS avec gradient analytique, comme Dixon-Coles (`models::likelihood`).
//!
//! **Limites.** Baker & McHale (2011, *IMA J. Manag. Math.* 22(3)) montrent qu'un modèle par numéro seul colle mal au
//! nombre de gagnants : les joueurs choisissent aussi des motifs (suites, colonnes, dates). D'où la validation hors
//! échantillon ([`walk_forward`]) avant tout usage : le critère est fixé dans [`Verdict`]. Roger & Broihanne (2006,
//! *Revue de statistique appliquée* 54(3)) montrent que les joueurs français ne choisissent pas au hasard.

use argmin::core::{CostFunction, Error as ArgminError, Executor, Gradient, State};
use argmin::solver::linesearch::MoreThuenteLineSearch;
use argmin::solver::quasinewton::LBFGS;
use chrono::Datelike;

use super::{Draw, Game, Grid, rules};
use crate::error::{Error, Result};
use crate::eval::bootstrap::{MeanCi, SplitMix64, block_bootstrap_mean};

/// Nombre de numéros (49) et de numéros chance (10), en `usize` pour indexer les tableaux.
const BALLS: usize = rules::LOTO_BALLS as usize;
const CHANCES: usize = rules::LOTO_CHANCES as usize;

/// Écart type a priori des paramètres (`ln a_i`, logits de `β`) : 0,5, soit des poids entre ⅓ et 3 à deux écarts types.
/// Fixé d'avance, jamais réglé sur le test.
pub const PRIOR_SD: f64 = 0.5;

/// Itérations maximales de L-BFGS.
const MAX_ITERATIONS: u64 = 500;

/// Première année de test du walk-forward : l'entraînement commence en mars 2017 (premières archives à neuf rangs).
pub const FIRST_TEST_YEAR: i32 = 2019;

/// Borne basse minimale de la pente de calibration (critère fixé d'avance, `docs/sources.md` §44) : les écarts prédits
/// doivent se retrouver au moins à moitié dans les gagnants observés.
pub const MIN_SLOPE_LOW: f64 = 0.5;

/// Préférences du public.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Preferences {
    /// Poids des numéros 1 à 49 (`balls[0]` : numéro 1), de moyenne géométrique 1.
    pub balls: Vec<f64>,
    /// Probabilités des numéros chance 1 à 10 (`chances[0]` : chance 1), de somme 1.
    pub chances: Vec<f64>,
}

impl Preferences {
    /// Choix au hasard : tous les poids égaux. C'est aussi le choix « Flash ».
    pub fn uniform() -> Preferences {
        Preferences {
            balls: vec![1.0; BALLS],
            chances: vec![1.0 / CHANCES as f64; CHANCES],
        }
    }

    /// Préférences à partir des paramètres optimisés : `a_i = e^{θ_i}`, `β = softmax(φ)`.
    fn from_params(params: &[f64]) -> Preferences {
        let balls = params[..BALLS].iter().map(|t| t.exp()).collect();
        Preferences {
            balls,
            chances: softmax(&params[BALLS..]),
        }
    }

    /// Part des grilles des autres joueurs qui ont la chance `chance` (1 à 10).
    pub fn chance_share(&self, chance: u8) -> f64 {
        self.chances[usize::from(chance) - 1]
    }

    /// Popularité relative d'une grille : sa fréquence chez les joueurs divisée par celle d'une grille au hasard (1 pour
    /// une grille moyenne, 2 pour une grille deux fois plus jouée).
    pub fn grid_popularity(&self, grid: &Grid) -> f64 {
        let product: f64 = grid
            .balls
            .iter()
            .map(|&b| self.balls[usize::from(b) - 1])
            .product();
        let all = elementary(&self.balls)[5];
        product / all * rules::choose(49, 5) * self.chance_share(grid.chance) * CHANCES as f64
    }
}

/// `softmax(φ)_j = e^{φ_j} / Σ_k e^{φ_k}`, calculé après soustraction du maximum (évite un dépassement de `exp`).
fn softmax(phi: &[f64]) -> Vec<f64> {
    let max = phi.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = phi.iter().map(|p| (p - max).exp()).collect();
    let total: f64 = exps.iter().sum();
    exps.iter().map(|e| e / total).collect()
}

/// Polynômes symétriques élémentaires `e_0` à `e_5` des poids `weights` : `e_r` est la somme des produits de `r` poids
/// distincts. Récurrence classique : on ajoute les poids un à un, `e_r ← e_r + w · e_{r−1}` (de `r = 5` à `1`, pour
/// ne pas réutiliser le poids ajouté).
pub fn elementary(weights: &[f64]) -> [f64; 6] {
    let mut e = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    for &w in weights {
        for r in (1..=5).rev() {
            e[r] += w * e[r - 1];
        }
    }
    e
}

/// `e_r` de l'ensemble privé d'un poids `w` : `g_0 = 1`, `g_r = e_r − w · g_{r−1}` (la récurrence ci-dessus, inversée).
fn excluding(e: &[f64; 6], w: f64) -> [f64; 6] {
    let mut g = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    for r in 1..=5 {
        g[r] = e[r] - w * g[r - 1];
    }
    g
}

/// Pour chaque rang 1 à 9 : (nombre de numéros trouvés `m`, chance trouvée). Le rang 9 réunit `m = 0` et `m = 1`.
const RANK_SHAPES: [(usize, bool); 8] = [
    (5, true),
    (5, false),
    (4, true),
    (4, false),
    (3, true),
    (3, false),
    (2, true),
    (2, false),
];

/// Les termes d'un tirage pour des préférences données.
struct DrawTerms {
    /// `u_m(D) = e_m(a_D) · e_{5−m}(a_{D̄})`, `m = 0..5`.
    u: [f64; 6],
    /// `e_r(a_D)`.
    drawn: [f64; 6],
    /// `e_r(a_{D̄})`.
    others: [f64; 6],
    /// `β_c`, part des joueurs qui ont la chance tirée.
    chance_share: f64,
}

impl DrawTerms {
    fn new(balls: &[f64], chances: &[f64], draw: &Draw) -> Option<DrawTerms> {
        let drawn_index: Vec<usize> = draw.balls.iter().map(|&b| usize::from(b) - 1).collect();
        let chance = usize::from(*draw.extra.first()?) - 1;
        let drawn_weights: Vec<f64> = drawn_index.iter().map(|&i| balls[i]).collect();
        let other_weights: Vec<f64> = (0..BALLS)
            .filter(|i| !drawn_index.contains(i))
            .map(|i| balls[i])
            .collect();
        let drawn = elementary(&drawn_weights);
        let others = elementary(&other_weights);
        let mut u = [0.0; 6];
        for (m, slot) in u.iter_mut().enumerate() {
            *slot = drawn[m] * others[5 - m];
        }
        Some(DrawTerms {
            u,
            drawn,
            others,
            chance_share: chances[chance],
        })
    }

    /// `q_k` des rangs 1 à 9 (`q[0]` : rang 1).
    fn rank_weights(&self) -> [f64; 9] {
        let (with, without) = (self.chance_share, 1.0 - self.chance_share);
        let mut q = [0.0; 9];
        for (k, &(m, chance)) in RANK_SHAPES.iter().enumerate() {
            q[k] = self.u[m] * if chance { with } else { without };
        }
        q[8] = (self.u[1] + self.u[0]) * with;
        q
    }
}

/// Log-vraisemblance multinomiale `ℓ_d` d'un tirage du Loto (formule en tête du module). `None` si le tirage n'a pas ses
/// neuf rangs ou aucun gagnant.
pub fn log_likelihood(prefs: &Preferences, draw: &Draw) -> Option<f64> {
    let terms = DrawTerms::new(&prefs.balls, &prefs.chances, draw)?;
    let counts = counts(draw)?;
    Some(multinomial(&counts, &terms.rank_weights()))
}

/// Gagnants des rangs 1 à 9, en flottants. `None` si un rang manque ou si personne n'a gagné.
fn counts(draw: &Draw) -> Option<[f64; 9]> {
    if draw.ranks.len() != 9 {
        return None;
    }
    let mut n = [0.0; 9];
    for (slot, rank) in n.iter_mut().zip(&draw.ranks) {
        *slot = rank.winners as f64;
    }
    (n.iter().sum::<f64>() > 0.0).then_some(n)
}

/// `Σ_k n_k ln q_k − n· ln Σ_k q_k` (un rang sans gagnant ne compte que par la somme).
fn multinomial(n: &[f64; 9], q: &[f64; 9]) -> f64 {
    let total_n: f64 = n.iter().sum();
    let total_q: f64 = q.iter().sum();
    let mut value = -total_n * total_q.ln();
    for (count, weight) in n.iter().zip(q) {
        if *count > 0.0 {
            value += count * weight.ln();
        }
    }
    value
}

/// Le problème d'optimisation : tirages, pénalité, et normalisation par le nombre total de gagnants.
struct Problem<'a> {
    draws: Vec<(&'a Draw, [f64; 9])>,
    winners: f64,
}

impl Problem<'_> {
    /// Valeur de `J(θ, φ) = (−Σ_d ℓ_d + (Σθ² + Σφ²) / (2σ²)) / T` et son gradient. `T`, le nombre total de gagnants,
    /// ramène `J` à l'ordre de grandeur 1 (sans changer le minimum) : les tolérances de L-BFGS restent lisibles.
    fn evaluate(&self, params: &[f64]) -> (f64, Vec<f64>) {
        let prefs = Preferences::from_params(params);
        let mut value = 0.0;
        let mut gradient = vec![0.0; BALLS + CHANCES];
        for &(draw, n) in &self.draws {
            let Some(terms) = DrawTerms::new(&prefs.balls, &prefs.chances, draw) else {
                continue;
            };
            let q = terms.rank_weights();
            value -= multinomial(&n, &q);
            add_draw_gradient(&mut gradient, &prefs, draw, &terms, &n, &q);
        }
        let precision = 1.0 / (PRIOR_SD * PRIOR_SD);
        for (g, p) in gradient.iter_mut().zip(params) {
            value += 0.5 * precision * p * p;
            *g += precision * p;
        }
        let scale = 1.0 / self.winners;
        (
            value * scale,
            gradient.into_iter().map(|g| g * scale).collect(),
        )
    }
}

/// Ajoute à `gradient` la dérivée de `−ℓ_d` par rapport à `θ` (49 premiers) et `φ` (10 derniers).
///
/// `∂ℓ/∂q_k = w_k = n_k / q_k − n· / Σq`. Ensuite, par la règle de dérivation en chaîne :
/// `∂ℓ/∂u_m = Σ_{k de forme m} w_k · γ_k` ; `∂u_m/∂θ_i = a_i · e_{m−1}(a_{D∖i}) · e_{5−m}(a_{D̄})` si `i` est tiré,
/// `e_m(a_D) · a_i · e_{4−m}(a_{D̄∖i})` sinon ; `∂ℓ/∂β_c = Σ_{avec chance} w_k u_{m_k} − Σ_{sans chance} w_k u_{m_k}`, et
/// `∂β_c/∂φ_j = β_c (δ_cj − β_j)` (dérivée du softmax).
fn add_draw_gradient(
    gradient: &mut [f64],
    prefs: &Preferences,
    draw: &Draw,
    terms: &DrawTerms,
    n: &[f64; 9],
    q: &[f64; 9],
) {
    let total_n: f64 = n.iter().sum();
    let total_q: f64 = q.iter().sum();
    let w: Vec<f64> = n
        .iter()
        .zip(q)
        .map(|(count, weight)| count / weight - total_n / total_q)
        .collect();
    let (with, without) = (terms.chance_share, 1.0 - terms.chance_share);

    let mut d_u = [0.0; 6];
    let (mut d_with, mut d_without) = (0.0, 0.0);
    for (k, &(m, chance)) in RANK_SHAPES.iter().enumerate() {
        if chance {
            d_u[m] += w[k] * with;
            d_with += w[k] * terms.u[m];
        } else {
            d_u[m] += w[k] * without;
            d_without += w[k] * terms.u[m];
        }
    }
    d_u[1] += w[8] * with;
    d_u[0] += w[8] * with;
    d_with += w[8] * (terms.u[1] + terms.u[0]);

    // `zip` parcourt ensemble la dérivée de chaque numéro et son poids ; `enumerate` donne le numéro (`i + 1`).
    for (i, (slot, &a)) in gradient.iter_mut().zip(&prefs.balls).enumerate() {
        let ball = u8::try_from(i + 1).unwrap_or(u8::MAX);
        let mut d_theta = 0.0;
        if draw.balls.contains(&ball) {
            let g = excluding(&terms.drawn, a);
            for m in 1..=5 {
                d_theta += d_u[m] * a * g[m - 1] * terms.others[5 - m];
            }
        } else {
            let g = excluding(&terms.others, a);
            for m in 0..=4 {
                d_theta += d_u[m] * terms.drawn[m] * a * g[4 - m];
            }
        }
        // On minimise −ℓ : le gradient change de signe.
        *slot -= d_theta;
    }

    let Some(&chance) = draw.extra.first() else {
        return;
    };
    let c = usize::from(chance) - 1;
    let d_beta_c = d_with - d_without;
    for j in 0..CHANCES {
        let delta = if j == c { 1.0 } else { 0.0 };
        let d_phi = d_beta_c * prefs.chances[c] * (delta - prefs.chances[j]);
        gradient[BALLS + j] -= d_phi;
    }
}

// Les deux traits d'`argmin` : la fonction de coût et son gradient.
impl CostFunction for Problem<'_> {
    type Param = Vec<f64>;
    type Output = f64;

    fn cost(&self, params: &Vec<f64>) -> std::result::Result<f64, ArgminError> {
        Ok(self.evaluate(params).0)
    }
}

impl Gradient for Problem<'_> {
    type Param = Vec<f64>;
    type Gradient = Vec<f64>;

    fn gradient(&self, params: &Vec<f64>) -> std::result::Result<Vec<f64>, ArgminError> {
        Ok(self.evaluate(params).1)
    }
}

/// Ajuste les préférences sur des tirages du Loto (déterministe : même données, même résultat). Les tirages sans neuf
/// rangs sont ignorés.
pub fn fit(draws: &[Draw]) -> Result<Preferences> {
    let draws: Vec<(&Draw, [f64; 9])> = draws
        .iter()
        .filter(|d| d.game == Game::Loto && d.extra.len() == 1)
        .filter_map(|d| counts(d).map(|n| (d, n)))
        .collect();
    if draws.is_empty() {
        return Err(Error::Data(
            "aucun tirage du Loto à neuf rangs pour mesurer les préférences".to_string(),
        ));
    }
    let winners = draws.iter().map(|(_, n)| n.iter().sum::<f64>()).sum();
    let problem = Problem { draws, winners };
    let line_search: MoreThuenteLineSearch<Vec<f64>, Vec<f64>, f64> = MoreThuenteLineSearch::new();
    let solver = LBFGS::new(line_search, 10)
        .with_tolerance_grad(1e-10)
        .and_then(|solver| solver.with_tolerance_cost(1e-14))
        .map_err(|e| Error::Optimizer(e.to_string()))?;
    let result = Executor::new(problem, solver)
        .configure(|state| {
            state
                .param(vec![0.0; BALLS + CHANCES])
                .max_iters(MAX_ITERATIONS)
        })
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
    Ok(Preferences::from_params(&best))
}

/// Résultat d'une année de test du walk-forward.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct YearResult {
    /// Année testée.
    pub year: i32,
    /// Tirages d'entraînement (tous ceux d'avant le 1er janvier).
    pub trained_on: usize,
    /// Tirages testés.
    pub tested: usize,
    /// Gain moyen de log-vraisemblance par tirage, contre le choix au hasard.
    pub mean_gain: f64,
}

/// Pente de calibration et son intervalle à 95 %.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Slope {
    /// Pente des moindres carrés.
    pub slope: f64,
    /// Borne basse.
    pub low: f64,
    /// Borne haute.
    pub high: f64,
}

/// Verdict du critère fixé d'avance (`docs/sources.md` §44) : les préférences servent au choix des grilles seulement si,
/// hors échantillon, (1) elles prédisent mieux les gagnants qu'un choix au hasard (gain de log-vraisemblance par tirage
/// dont l'intervalle à 95 % exclut 0) et (2) les écarts prédits se retrouvent dans les gagnants observés (pente de
/// calibration du rang 8 dont la borne basse dépasse [`MIN_SLOPE_LOW`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Verdict {
    /// Les deux conditions sont remplies.
    Validated,
    /// Au moins une condition manque : on joue au hasard.
    Rejected,
}

/// Validation walk-forward des préférences.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WalkForward {
    /// Une ligne par année de test.
    pub years: Vec<YearResult>,
    /// Gain de log-vraisemblance par tirage sur toutes les années de test, intervalle par blocs (une semaine = un bloc).
    pub gain: MeanCi,
    /// Pente de calibration du rang 8 (deux numéros sans la chance) : observé contre prédit, en écart au hasard. `None`
    /// si les préférences ajustées ne prédisent aucun écart (choix au hasard) : la pente n'existe pas.
    pub slope: Option<Slope>,
    /// Contrôle, hors critère : pente de calibration par nombre de numéros trouvés (`m` = 2, 3, 4 ; rangs 7-8, 5-6 et
    /// 3-4 réunis). Le gain d'une grille délaissée vient surtout des rangs 3 à 6 : une pente nettement sous 1 y dirait
    /// que le modèle exagère ce gain.
    pub level_slopes: Vec<(usize, Option<Slope>)>,
    /// Verdict du critère.
    pub verdict: Verdict,
}

/// Rééchantillonnages du bootstrap.
const RESAMPLES: usize = 2000;
/// Graine du bootstrap (invariant 9 : résultat reproductible).
const SEED: u64 = 16;

/// Walk-forward annuel : pour chaque année à partir de [`FIRST_TEST_YEAR`], ajuste sur les tirages **antérieurs** au
/// 1er janvier et mesure sur l'année. Aucun tirage testé ne sert à l'ajustement qui le prédit.
pub fn walk_forward(draws: &[Draw]) -> Result<WalkForward> {
    let loto: Vec<&Draw> = draws
        .iter()
        .filter(|d| d.game == Game::Loto && counts(d).is_some())
        .collect();
    let last_year = loto
        .iter()
        .map(|d| d.date.year())
        .max()
        .unwrap_or(FIRST_TEST_YEAR);
    let mut years = Vec::new();
    let mut gains = Vec::new();
    let mut pairs = Vec::new();
    let mut level_pairs: Vec<Vec<(u32, f64, f64)>> = vec![Vec::new(); LEVELS.len()];
    for year in FIRST_TEST_YEAR..=last_year {
        let train: Vec<Draw> = loto
            .iter()
            .filter(|d| d.date.year() < year)
            .map(|d| (*d).clone())
            .collect();
        let test: Vec<&Draw> = loto
            .iter()
            .copied()
            .filter(|d| d.date.year() == year)
            .collect();
        if train.is_empty() || test.is_empty() {
            continue;
        }
        let prefs = fit(&train)?;
        let uniform = Preferences::uniform();
        let mut total = 0.0;
        for draw in &test {
            let (Some(fitted), Some(base)) =
                (log_likelihood(&prefs, draw), log_likelihood(&uniform, draw))
            else {
                continue;
            };
            let block = week_block(draw);
            gains.push((block, fitted - base));
            total += fitted - base;
            if let Some(pair) = calibration_pair(&prefs, draw, &[7]) {
                pairs.push((block, pair.0, pair.1));
            }
            for (slot, (_, ranks)) in level_pairs.iter_mut().zip(LEVELS) {
                if let Some(pair) = calibration_pair(&prefs, draw, ranks) {
                    slot.push((block, pair.0, pair.1));
                }
            }
        }
        years.push(YearResult {
            year,
            trained_on: train.len(),
            tested: test.len(),
            mean_gain: total / test.len() as f64,
        });
    }
    let gain = block_bootstrap_mean(&gains, RESAMPLES, SEED).ok_or_else(|| {
        Error::Data("aucun tirage de test pour valider les préférences".to_string())
    })?;
    let slope = block_bootstrap_slope(&pairs, RESAMPLES, SEED);
    let level_slopes = LEVELS
        .iter()
        .zip(&level_pairs)
        .map(|((m, _), points)| (*m, block_bootstrap_slope(points, RESAMPLES, SEED)))
        .collect();
    let calibrated = slope.is_some_and(|s| s.low > MIN_SLOPE_LOW);
    let verdict = if gain.low > 0.0 && calibrated {
        Verdict::Validated
    } else {
        Verdict::Rejected
    };
    Ok(WalkForward {
        years,
        gain,
        slope,
        level_slopes,
        verdict,
    })
}

/// Bloc du bootstrap : la semaine ISO du tirage (trois tirages par semaine, conditions de jeu communes).
fn week_block(draw: &Draw) -> u32 {
    let week = draw.date.iso_week();
    u32::try_from(week.year()).unwrap_or(0) * 100 + week.week()
}

/// Niveaux contrôlés : (numéros trouvés, indices des rangs réunis).
const LEVELS: [(usize, &[usize]); 3] = [(2, &[6, 7]), (3, &[4, 5]), (4, &[2, 3])];

/// Rangs `ranks` (indices, `7` : rang 8) réunis : écart au hasard de leur part parmi les gagnants, prédit `x` et observé
/// `y`, en logarithme.
fn calibration_pair(prefs: &Preferences, draw: &Draw, ranks: &[usize]) -> Option<(f64, f64)> {
    let n = counts(draw)?;
    let total_n: f64 = n.iter().sum();
    let observed: f64 = ranks.iter().map(|&k| n[k]).sum();
    if observed <= 0.0 {
        return None;
    }
    let q = DrawTerms::new(&prefs.balls, &prefs.chances, draw)?.rank_weights();
    let uniform = DrawTerms::new(
        &Preferences::uniform().balls,
        &Preferences::uniform().chances,
        draw,
    )?
    .rank_weights();
    let share = |w: &[f64; 9]| ranks.iter().map(|&k| w[k]).sum::<f64>() / w.iter().sum::<f64>();
    let base = share(&uniform).ln();
    Some((share(&q).ln() - base, (observed / total_n).ln() - base))
}

/// Pente des moindres carrés de `y` sur `x` et son intervalle à 95 % par bootstrap par blocs.
fn block_bootstrap_slope(pairs: &[(u32, f64, f64)], resamples: usize, seed: u64) -> Option<Slope> {
    let slope_of = |points: &[(f64, f64)]| -> Option<f64> {
        let n = points.len() as f64;
        let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
        let my = points.iter().map(|p| p.1).sum::<f64>() / n;
        let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
        let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        (sxx > 0.0).then_some(sxy / sxx)
    };
    let mut blocks: Vec<(u32, Vec<(f64, f64)>)> = Vec::new();
    for &(block, x, y) in pairs {
        match blocks.last_mut() {
            Some((last, points)) if *last == block => points.push((x, y)),
            _ => blocks.push((block, vec![(x, y)])),
        }
    }
    let all: Vec<(f64, f64)> = pairs.iter().map(|p| (p.1, p.2)).collect();
    let slope = slope_of(&all)?;
    let mut rng = SplitMix64::new(seed);
    let mut slopes = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut sample = Vec::with_capacity(all.len());
        for _ in 0..blocks.len() {
            let i = ((rng.next_f64() * blocks.len() as f64) as usize).min(blocks.len() - 1);
            sample.extend_from_slice(&blocks[i].1);
        }
        if let Some(s) = slope_of(&sample) {
            slopes.push(s);
        }
    }
    if slopes.is_empty() {
        return None;
    }
    slopes.sort_by(|a, b| a.total_cmp(b));
    let at =
        |q: f64| slopes[((q * (slopes.len() - 1) as f64).round() as usize).min(slopes.len() - 1)];
    Some(Slope {
        slope,
        low: at(0.025),
        high: at(0.975),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use approx::assert_relative_eq;
    use chrono::NaiveDate;

    use super::*;
    use crate::loto::RankResult;

    /// Tirage au hasard (cinq numéros distincts, une chance) avec le générateur du projet.
    fn random_draw(rng: &mut SplitMix64) -> ([u8; 5], u8) {
        let mut balls = Vec::new();
        while balls.len() < 5 {
            let b = 1 + (rng.next_f64() * 49.0) as u8;
            if !balls.contains(&b) {
                balls.push(b);
            }
        }
        let chance = 1 + (rng.next_f64() * 10.0) as u8;
        let mut balls: [u8; 5] = balls.try_into().unwrap();
        balls.sort_unstable();
        (balls, chance)
    }

    /// Préférences « plantées » : les dates (1 à 31) plus jouées, 7 et 13 très joués, la chance 7 favorite.
    pub(crate) fn planted() -> Preferences {
        let mut balls: Vec<f64> = (1..=49).map(|b| if b <= 31 { 1.25 } else { 0.7 }).collect();
        balls[6] = 1.8;
        balls[12] = 1.6;
        let mut chances = vec![1.0; 10];
        chances[6] = 1.6;
        let total: f64 = chances.iter().sum();
        Preferences {
            balls,
            chances: chances.into_iter().map(|c| c / total).collect(),
        }
    }

    /// Tirages simulés : gagnants égaux à leur espérance `N · π_k(D)` (sans bruit), pour `n` tirages au hasard.
    pub(crate) fn simulated(prefs: &Preferences, n: usize, seed: u64) -> Vec<Draw> {
        let mut rng = SplitMix64::new(seed);
        let all = elementary(&prefs.balls)[5];
        let start = NaiveDate::from_ymd_opt(2017, 3, 6).unwrap();
        (0..n)
            .map(|i| {
                let (balls, chance) = random_draw(&mut rng);
                let mut draw = Draw {
                    game: Game::Loto,
                    date: start + chrono::Days::new(2 * i as u64),
                    balls,
                    extra: vec![chance],
                    ranks: Vec::new(),
                    codes: None,
                };
                let q = DrawTerms::new(&prefs.balls, &prefs.chances, &draw)
                    .unwrap()
                    .rank_weights();
                draw.ranks = q
                    .iter()
                    .map(|w| RankResult {
                        winners: (5_000_000.0 * w / all).round() as u64,
                        unit_gain: 1.0,
                    })
                    .collect();
                draw
            })
            .collect()
    }

    #[test]
    fn elementary_polynomials_match_their_definition() {
        let e = elementary(&[1.0, 2.0, 3.0]);
        assert_eq!(e, [1.0, 6.0, 11.0, 6.0, 0.0, 0.0]);
        let g = excluding(&e, 2.0);
        assert_eq!(g, elementary(&[1.0, 3.0]));
    }

    #[test]
    fn uniform_preferences_give_the_rules_probabilities() {
        let prefs = Preferences::uniform();
        let draw = &simulated(&prefs, 1, 3)[0];
        let terms = DrawTerms::new(&prefs.balls, &prefs.chances, draw).unwrap();
        let all = elementary(&prefs.balls)[5];
        for (k, weight) in terms.rank_weights().iter().enumerate() {
            assert_relative_eq!(
                weight / all,
                rules::loto_rank_probability(k + 1),
                max_relative = 1e-12
            );
        }
        assert_relative_eq!(
            prefs.grid_popularity(&Grid::parse("1-2-3-4-5+6").unwrap()),
            1.0,
            max_relative = 1e-12
        );
    }

    #[test]
    fn the_analytic_gradient_matches_finite_differences() {
        let draws = simulated(&planted(), 20, 5);
        let problem = Problem {
            draws: draws.iter().map(|d| (d, counts(d).unwrap())).collect(),
            winners: 1.0e6,
        };
        let mut params: Vec<f64> = (0..BALLS + CHANCES)
            .map(|i| 0.01 * ((i * 7 % 11) as f64 - 5.0))
            .collect();
        let (_, gradient) = problem.evaluate(&params);
        for i in [0, 6, 30, 48, 49, 55, 58] {
            let h = 1e-6;
            params[i] += h;
            let up = problem.evaluate(&params).0;
            params[i] -= 2.0 * h;
            let down = problem.evaluate(&params).0;
            params[i] += h;
            assert_relative_eq!(
                gradient[i],
                (up - down) / (2.0 * h),
                epsilon = 1e-6,
                max_relative = 1e-4
            );
        }
    }

    #[test]
    fn the_fit_recovers_planted_preferences() {
        let truth = planted();
        let fitted = fit(&simulated(&truth, 400, 11)).unwrap();
        // Comparaison en logarithme, après centrage (l'échelle des poids n'est pas identifiable).
        let centred = |w: &[f64]| {
            let logs: Vec<f64> = w.iter().map(|x| x.ln()).collect();
            let mean = logs.iter().sum::<f64>() / logs.len() as f64;
            logs.into_iter().map(|x| x - mean).collect::<Vec<f64>>()
        };
        for (a, b) in centred(&truth.balls).iter().zip(centred(&fitted.balls)) {
            assert!((a - b).abs() < 0.05, "poids {a} retrouvé à {b}");
        }
        for (a, b) in truth.chances.iter().zip(&fitted.chances) {
            assert!((a - b).abs() < 0.005, "chance {a} retrouvée à {b}");
        }
    }

    #[test]
    fn the_walk_forward_validates_real_preferences_and_never_trains_on_its_test_year() {
        let draws = simulated(&planted(), 600, 21);
        let real = walk_forward(&draws).unwrap();
        assert_eq!(real.verdict, Verdict::Validated, "{real:?}");
        assert!(!real.years.is_empty());
        for year in &real.years {
            let before = draws.iter().filter(|d| d.date.year() < year.year).count();
            assert_eq!(year.trained_on, before, "aucune fuite du futur");
        }
    }

    #[test]
    fn the_walk_forward_rejects_absent_preferences() {
        let none = walk_forward(&simulated(&Preferences::uniform(), 600, 22)).unwrap();
        assert_eq!(none.verdict, Verdict::Rejected, "{none:?}");
    }
}
