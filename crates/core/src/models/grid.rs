//! Grille de scores : la probabilité de chaque score exact, d'où se lisent tous les marchés.

use statrs::distribution::{Discrete, Poisson};

use crate::domain::Probs1x2;

/// Nombre maximal de buts par équipe dans la grille (0 à 10). Au-delà, la masse est négligeable (de l'ordre de
/// 10⁻⁸ pour une équipe qui en marque 1,5 en moyenne) et la grille est renormalisée.
pub const MAX_GOALS: usize = 10;

const SIZE: usize = MAX_GOALS + 1;

/// Probabilités des scores exacts : `cell[x][y]` = probabilité que l'équipe à domicile marque `x` buts et celle
/// à l'extérieur `y`. La somme vaut 1.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreGrid {
    cells: [[f64; SIZE]; SIZE],
}

impl ScoreGrid {
    /// Grille de Dixon-Coles : deux lois de Poisson de moyennes `home_mean` et `away_mean`, corrigées pour les
    /// scores faibles par le facteur `τ` de paramètre `rho` ; `rho = 0` donne deux Poisson indépendantes.
    ///
    /// Forme (Dixon & Coles, 1997, *Applied Statistics* 46(2), éq. 4.2) :
    /// `P(x, y) = τ(x, y) · Pois(x; λ) · Pois(y; μ)` avec
    /// `τ(0,0) = 1 − λμρ`, `τ(0,1) = 1 + λρ`, `τ(1,0) = 1 + μρ`, `τ(1,1) = 1 − ρ`, et `τ = 1` ailleurs.
    ///
    /// Renvoie `None` si une moyenne n'est pas strictement positive, ou si `rho` rend un `τ` négatif (le
    /// « modèle » ne serait plus une loi de probabilité).
    pub fn dixon_coles(home_mean: f64, away_mean: f64, rho: f64) -> Option<ScoreGrid> {
        let home = Poisson::new(home_mean).ok()?;
        let away = Poisson::new(away_mean).ok()?;
        let mut cells = [[0.0; SIZE]; SIZE];
        let mut total = 0.0;
        for (x, row) in cells.iter_mut().enumerate() {
            for (y, cell) in row.iter_mut().enumerate() {
                let tau = low_score_factor(x, y, home_mean, away_mean, rho);
                if tau < 0.0 {
                    return None;
                }
                *cell = tau * home.pmf(x as u64) * away.pmf(y as u64);
                total += *cell;
            }
        }
        // Renormalisation : la grille tronquée à 10 buts et le facteur τ ne somment pas exactement à 1.
        for row in cells.iter_mut() {
            for cell in row.iter_mut() {
                *cell /= total;
            }
        }
        Some(ScoreGrid { cells })
    }

    /// Probabilité du score exact `x`–`y` (0 hors de la grille).
    pub fn probability(&self, home_goals: usize, away_goals: usize) -> f64 {
        self.cells
            .get(home_goals)
            .and_then(|row| row.get(away_goals))
            .copied()
            .unwrap_or(0.0)
    }

    /// Probabilités de victoire à domicile, de nul et de victoire à l'extérieur.
    pub fn home_draw_away(&self) -> Option<Probs1x2> {
        let (mut home, mut draw, mut away) = (0.0, 0.0, 0.0);
        for (x, row) in self.cells.iter().enumerate() {
            for (y, p) in row.iter().enumerate() {
                match x.cmp(&y) {
                    std::cmp::Ordering::Greater => home += p,
                    std::cmp::Ordering::Equal => draw += p,
                    std::cmp::Ordering::Less => away += p,
                }
            }
        }
        Probs1x2::new(home, draw, away)
    }

    /// Probabilité que le total de buts dépasse `line` (une ligne `x,5` : `2.5` signifie « 3 buts ou plus »).
    pub fn over(&self, line: f64) -> f64 {
        let minimum_goals = line.floor() as usize + 1;
        let mut total = 0.0;
        for (x, row) in self.cells.iter().enumerate() {
            for (y, p) in row.iter().enumerate() {
                if x + y >= minimum_goals {
                    total += p;
                }
            }
        }
        total
    }

    /// Probabilité que les deux équipes marquent au moins un but.
    pub fn both_teams_score(&self) -> f64 {
        let mut total = 0.0;
        for row in self.cells.iter().skip(1) {
            total += row.iter().skip(1).sum::<f64>();
        }
        total
    }
}

/// Facteur `τ` de Dixon-Coles pour le score `x`–`y`.
pub(crate) fn low_score_factor(
    x: usize,
    y: usize,
    home_mean: f64,
    away_mean: f64,
    rho: f64,
) -> f64 {
    match (x, y) {
        (0, 0) => 1.0 - home_mean * away_mean * rho,
        (0, 1) => 1.0 + home_mean * rho,
        (1, 0) => 1.0 + away_mean * rho,
        (1, 1) => 1.0 - rho,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn total(grid: &ScoreGrid) -> f64 {
        (0..=MAX_GOALS)
            .flat_map(|x| (0..=MAX_GOALS).map(move |y| (x, y)))
            .map(|(x, y)| grid.probability(x, y))
            .sum()
    }

    #[test]
    fn the_grid_sums_to_one() {
        for (lambda, mu, rho) in [
            (1.5, 1.1, 0.0),
            (1.2, 0.8, -0.1),
            (0.4, 0.3, -0.2),
            (3.5, 2.8, -0.05),
        ] {
            let grid = ScoreGrid::dixon_coles(lambda, mu, rho).unwrap();
            assert_abs_diff_eq!(total(&grid), 1.0, epsilon = 1e-12);
        }
    }

    #[test]
    fn without_correction_two_equal_teams_draw_like_the_poisson_theory() {
        // P(nul) = Σ Pois(k; 1)² = 0,308508… (calculé à part).
        let grid = ScoreGrid::dixon_coles(1.0, 1.0, 0.0).unwrap();
        let probs = grid.home_draw_away().unwrap();
        assert_abs_diff_eq!(
            probs.get(crate::domain::Outcome::Draw).get(),
            0.3085083,
            epsilon = 1e-6
        );
        // Symétrie : deux équipes égales ont la même chance de gagner.
        assert_abs_diff_eq!(
            probs.get(crate::domain::Outcome::Home).get(),
            probs.get(crate::domain::Outcome::Away).get(),
            epsilon = 1e-12
        );
    }

    #[test]
    fn the_dixon_coles_correction_matches_reference_values_computed_separately() {
        let grid = ScoreGrid::dixon_coles(1.2, 0.8, -0.1).unwrap();
        let probs = grid.home_draw_away().unwrap();
        assert_abs_diff_eq!(probs.to_array()[0], 0.4409585882, epsilon = 1e-8);
        assert_abs_diff_eq!(probs.to_array()[1], 0.3259562308, epsilon = 1e-8);
        assert_abs_diff_eq!(probs.to_array()[2], 0.2330851810, epsilon = 1e-8);
        assert_abs_diff_eq!(grid.over(2.5), 0.3233235410, epsilon = 1e-8);
        assert_abs_diff_eq!(grid.both_teams_score(), 0.3978042846, epsilon = 1e-8);
        assert_abs_diff_eq!(grid.probability(0, 0), 0.1483274798, epsilon = 1e-8);
        assert_abs_diff_eq!(grid.probability(1, 1), 0.1429140681, epsilon = 1e-8);
    }

    #[test]
    fn a_negative_rho_moves_mass_from_one_one_to_the_other_low_scores() {
        let plain = ScoreGrid::dixon_coles(1.2, 0.8, 0.0).unwrap();
        let corrected = ScoreGrid::dixon_coles(1.2, 0.8, -0.1).unwrap();
        // Avec ρ < 0 : plus de 0-0 et de 1-1, moins de 0-1 et de 1-0.
        assert!(corrected.probability(0, 0) > plain.probability(0, 0));
        assert!(corrected.probability(1, 1) > plain.probability(1, 1));
        assert!(corrected.probability(0, 1) < plain.probability(0, 1));
        assert!(corrected.probability(1, 0) < plain.probability(1, 0));
    }

    #[test]
    fn over_and_under_are_complementary_and_the_lines_nest() {
        let grid = ScoreGrid::dixon_coles(1.6, 1.2, -0.08).unwrap();
        // « Plus de 0,5 » = au moins un but = 1 − P(0-0).
        assert_abs_diff_eq!(
            grid.over(0.5),
            1.0 - grid.probability(0, 0),
            epsilon = 1e-12
        );
        assert!(grid.over(1.5) < grid.over(0.5));
        assert!(grid.over(2.5) < grid.over(1.5));
        assert!(grid.over(3.5) < grid.over(2.5));
    }

    #[test]
    fn both_teams_score_follows_the_inclusion_exclusion_identity() {
        let grid = ScoreGrid::dixon_coles(1.4, 1.0, -0.1).unwrap();
        // P(les deux marquent) = 1 − P(domicile 0) − P(extérieur 0) + P(0-0).
        let home_zero: f64 = (0..=MAX_GOALS).map(|y| grid.probability(0, y)).sum();
        let away_zero: f64 = (0..=MAX_GOALS).map(|x| grid.probability(x, 0)).sum();
        let expected = 1.0 - home_zero - away_zero + grid.probability(0, 0);
        assert_abs_diff_eq!(grid.both_teams_score(), expected, epsilon = 1e-12);
    }

    #[test]
    fn invalid_inputs_give_no_grid() {
        assert!(ScoreGrid::dixon_coles(0.0, 1.0, 0.0).is_none());
        assert!(ScoreGrid::dixon_coles(-1.0, 1.0, 0.0).is_none());
        assert!(ScoreGrid::dixon_coles(f64::NAN, 1.0, 0.0).is_none());
        // ρ trop grand : τ(0,0) = 1 − 3·3·0,5 < 0.
        assert!(ScoreGrid::dixon_coles(3.0, 3.0, 0.5).is_none());
    }

    #[test]
    fn scores_outside_the_grid_have_zero_probability() {
        let grid = ScoreGrid::dixon_coles(1.0, 1.0, 0.0).unwrap();
        assert_eq!(grid.probability(11, 0), 0.0);
    }
}
