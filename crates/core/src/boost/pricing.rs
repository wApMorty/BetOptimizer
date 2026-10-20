//! Tarification d'une cote boostée contre le marché.
//!
//! Le marché ne publie pas le prix de « Lyon gagne et les deux équipes marquent ». On reconstruit donc une grille de
//! scores **calée sur le marché** : la grille de Dixon-Coles dont les probabilités de victoire à domicile, de
//! victoire à l'extérieur et de plus de 2,5 buts sont celles de Pinnacle sans marge. Trois contraintes, trois
//! paramètres (λ, μ, ρ) : l'ajustement est en général exact. Tout pari sur le score final se lit ensuite sur la grille.

use argmin::core::{CostFunction, Error as ArgminError, Executor, State};
use argmin::solver::neldermead::NelderMead;

use super::selection::Selection;
use crate::domain::{Outcome, Probability, Probs1x2};
use crate::models::grid::{MAX_GOALS, ScoreGrid};

/// Écart maximal toléré entre une probabilité cible et celle de la grille ajustée quand ρ est ajusté (trois paramètres
/// pour trois cibles : l'ajustement doit être exact, un écart signale un échec numérique).
pub const MAX_RESIDUAL: f64 = 0.005;

/// Écart toléré quand ρ est fixé : deux paramètres ne peuvent pas reproduire exactement trois cibles (le nul surtout),
/// l'écart est le prix du modèle et non un échec. Au-delà, la grille est refusée.
pub const MAX_RESIDUAL_FIXED_RHO: f64 = 0.05;

/// Coût renvoyé pour des paramètres hors du domaine (ρ qui rendrait une probabilité négative) : l'optimiseur s'en
/// écarte.
const OUT_OF_DOMAIN: f64 = 1e3;

/// Probabilités du marché, sans marge, que la grille doit reproduire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketTargets {
    /// Victoire à domicile, nul, victoire à l'extérieur.
    pub result: Probs1x2,
    /// Plus de 2,5 buts.
    pub over_2_5: Probability,
}

/// Paramètre ρ de la grille (correction des petits scores de Dixon-Coles).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rho {
    /// Ajusté avec λ et μ : trois paramètres pour trois cibles, ajustement exact.
    Fitted,
    /// Fixé : λ et μ seuls, ajustés au mieux (moindres carrés) sur les trois cibles.
    Fixed(f64),
}

/// Problème d'ajustement : paramètres `x = (ln λ, ln μ)` suivis de `ρ` s'il est ajusté ; coût = somme des carrés des
/// écarts aux trois cibles. Le logarithme garantit des moyennes de buts positives sans contrainte.
struct FitProblem {
    targets: MarketTargets,
    rho: Rho,
}

impl FitProblem {
    fn grid(&self, x: &[f64]) -> Option<ScoreGrid> {
        let (log_home, log_away) = (*x.first()?, *x.get(1)?);
        let rho = match self.rho {
            Rho::Fitted => *x.get(2)?,
            Rho::Fixed(rho) => rho,
        };
        ScoreGrid::dixon_coles(log_home.exp(), log_away.exp(), rho)
    }

    /// Plus grand écart absolu entre la grille et les cibles.
    fn residual(&self, grid: &ScoreGrid) -> Option<f64> {
        let result = grid.home_draw_away()?;
        Some(
            self.errors(&result, grid)
                .iter()
                .fold(0.0_f64, |worst, e| worst.max(e.abs())),
        )
    }

    /// Écarts entre la grille et les trois cibles : victoire à domicile, victoire à l'extérieur, plus de 2,5 buts.
    fn errors(&self, result: &Probs1x2, grid: &ScoreGrid) -> [f64; 3] {
        let target = &self.targets.result;
        [
            result.get(Outcome::Home).get() - target.get(Outcome::Home).get(),
            result.get(Outcome::Away).get() - target.get(Outcome::Away).get(),
            grid.over(2.5) - self.targets.over_2_5.get(),
        ]
    }
}

impl CostFunction for FitProblem {
    type Param = Vec<f64>;
    type Output = f64;

    fn cost(&self, x: &Vec<f64>) -> std::result::Result<f64, ArgminError> {
        let Some(grid) = self.grid(x) else {
            return Ok(OUT_OF_DOMAIN);
        };
        let Some(result) = grid.home_draw_away() else {
            return Ok(OUT_OF_DOMAIN);
        };
        Ok(self.errors(&result, &grid).iter().map(|e| e * e).sum())
    }
}

/// Grille de scores calée sur le marché, ou `None` si l'ajustement échoue ou s'écarte des cibles de plus de
/// [`MAX_RESIDUAL`] (ρ ajusté) ou de [`MAX_RESIDUAL_FIXED_RHO`] (ρ fixé).
///
/// Optimiseur de Nelder & Mead (1965, *The Computer Journal* 7(4)) : sans gradient, robuste pour trois paramètres.
/// Départ déterministe (λ = 1,5, μ = 1,2, ρ = −0,05), donc résultat reproductible.
pub fn market_grid(targets: MarketTargets, rho: Rho) -> Option<ScoreGrid> {
    let mut start = vec![1.5_f64.ln(), 1.2_f64.ln()];
    if rho == Rho::Fitted {
        start.push(-0.05);
    }
    // Simplexe initial : le départ et un pas sur chaque axe.
    let mut simplex = vec![start.clone()];
    for axis in 0..start.len() {
        let mut vertex = start.clone();
        vertex[axis] += 0.2;
        simplex.push(vertex);
    }
    let solver = NelderMead::new(simplex).with_sd_tolerance(1e-12).ok()?;
    let result = Executor::new(FitProblem { targets, rho }, solver)
        .configure(|state| state.max_iters(2000))
        .run()
        .ok()?;
    // L'exécuteur a consommé le problème : on en reconstruit un (deux champs `Copy`) pour lire la grille optimale.
    let problem = FitProblem { targets, rho };
    let grid = problem.grid(result.state().get_best_param()?)?;
    let residual = problem.residual(&grid)?;
    let tolerance = match rho {
        Rho::Fitted => MAX_RESIDUAL,
        Rho::Fixed(_) => MAX_RESIDUAL_FIXED_RHO,
    };
    (residual <= tolerance).then_some(grid)
}

/// Probabilité que le pari soit gagné : somme des cases de la grille où toutes ses conditions sont remplies.
pub fn probability(grid: &ScoreGrid, selection: &Selection) -> Probability {
    let mut total = 0.0;
    for home in 0..=MAX_GOALS {
        for away in 0..=MAX_GOALS {
            if selection.holds(home as u32, away as u32) {
                total += grid.probability(home, away);
            }
        }
    }
    // La grille somme à 1 ; `min` absorbe un dépassement d'arrondi de l'ordre de 1e-16.
    Probability::new(total.min(1.0)).expect("somme de probabilités d'une grille normalisée")
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;
    use crate::boost::selection;

    fn targets(home: f64, draw: f64, away: f64, over: f64) -> MarketTargets {
        MarketTargets {
            result: Probs1x2::new(home, draw, away).unwrap(),
            over_2_5: Probability::new(over).unwrap(),
        }
    }

    #[test]
    fn the_fitted_grid_reproduces_the_market() {
        for t in [
            targets(0.45, 0.27, 0.28, 0.52),
            targets(0.75, 0.16, 0.09, 0.65),
            targets(0.20, 0.28, 0.52, 0.45),
            targets(0.36, 0.32, 0.32, 0.40),
        ] {
            let grid = market_grid(t, Rho::Fitted).unwrap();
            let result = grid.home_draw_away().unwrap();
            for outcome in [Outcome::Home, Outcome::Draw, Outcome::Away] {
                assert_abs_diff_eq!(
                    result.get(outcome).get(),
                    t.result.get(outcome).get(),
                    epsilon = 1e-3
                );
            }
            assert_abs_diff_eq!(grid.over(2.5), t.over_2_5.get(), epsilon = 1e-3);
        }
    }

    #[test]
    fn a_grid_sums_to_one_over_complementary_bets() {
        let grid = market_grid(targets(0.45, 0.27, 0.28, 0.52), Rho::Fitted).unwrap();
        let p = |text: &str| probability(&grid, &selection::parse(text, "A", "B").unwrap()).get();
        // Victoire, nul et défaite de A couvrent tous les scores.
        assert_abs_diff_eq!(
            p("A gagne") + p("match nul") + p("B gagne"),
            1.0,
            epsilon = 1e-9
        );
        assert_abs_diff_eq!(
            p("plus de 2,5 buts") + p("moins de 2,5 buts"),
            1.0,
            epsilon = 1e-9
        );
        // Une conjonction ne peut pas être plus probable que chacune de ses conditions.
        assert!(p("A gagne et les deux équipes marquent") < p("A gagne"));
        // La victoire du marché se relit sur la grille.
        assert_abs_diff_eq!(p("A gagne"), 0.45, epsilon = 1e-3);
    }
}
