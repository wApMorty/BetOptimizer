//! Prix dynamique de l'euro (`docs/sources.md` §69 et §70) : la valeur marginale qu'un ticket doit atteindre, selon le temps
//! et le budget qui restent dans le mois. La table est calculée une fois pour toutes sur l'entraînement.
//!
//! Le budget se saisit dans l'application ; le seuil n'est pas un réglage, il découle de ce qui reste à dépenser.

use crate::error::{Error, Result};

/// Version de la table, enregistrée avec chaque séance.
pub const VERSION: &str = "prix-2026-10-31";

/// Budget mensuel minimal et maximal acceptés (euros).
pub const MIN_BUDGET: u32 = 20;
/// Voir [`MIN_BUDGET`].
pub const MAX_BUDGET: u32 = 400;

/// Nombre de pas d'un mois dans la récurrence de la table.
pub const MONTH_STEPS: f64 = 120.0;

/// La table gelée, embarquée dans le binaire : `scripts/lotofoot/net_price.py`, tirée des 319 grilles d'entraînement, pour un
/// plafond de 32 tickets par grille.
const TABLE: &str = include_str!("../../data/lotofoot_price.csv");

/// Refuse un budget mensuel hors des bornes.
pub fn check_budget(monthly_budget: u32) -> Result<()> {
    if (MIN_BUDGET..=MAX_BUDGET).contains(&monthly_budget) {
        Ok(())
    } else {
        Err(Error::Input(format!(
            "budget mensuel de {monthly_budget} € refusé : de {MIN_BUDGET} à {MAX_BUDGET} €"
        )))
    }
}

/// Seuils de valeur marginale.
///
/// Récurrence (Papastavrou, Rajagopalan & Kleywegt 1996 ; Lueker 1998), `b` euros restants, `t` pas restants :
/// `W_t(b) = (1 − π) W_{t−1}(b) + π · E_g max_{k ≤ b} [C_g(k) − k + W_{t−1}(b − k)]`, `W_0 = 0`.
/// Le seuil d'un ticket vaut `1 + W_t(b) − W_t(b − 1)` : il doit rendre son euro, et ce que cet euro aurait rapporté plus
/// tard. `W` ne dépend pas du budget mensuel, seulement de ce qui reste : une seule table sert à tous les budgets.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceTable {
    /// Pas restants de chaque ligne, croissants.
    steps: Vec<f64>,
    /// Euros restants de chaque colonne, croissants.
    budgets: Vec<u32>,
    /// Seuils, une ligne par valeur de `steps`, une colonne par valeur de `budgets`.
    rows: Vec<Vec<f64>>,
}

impl PriceTable {
    /// La table embarquée.
    pub fn frozen() -> PriceTable {
        // Le fichier fait partie du binaire et sa lecture est vérifiée par les tests : une erreur ici serait une erreur de
        // construction, pas une situation à gérer en séance.
        PriceTable::parse(TABLE)
            .expect("table embarquée, lue par le test `the_frozen_table_is_well_formed`")
    }

    /// Lit la forme CSV : commentaires `#`, en-tête `t,b1,b2,…`, puis une ligne par valeur de `t`.
    fn parse(text: &str) -> Option<PriceTable> {
        let mut lines = text.lines().filter(|line| !line.starts_with('#'));
        let budgets: Vec<u32> = lines
            .next()?
            .split(',')
            .skip(1)
            .map(|cell| cell.parse().ok())
            .collect::<Option<_>>()?;
        let mut steps = Vec::new();
        let mut rows = Vec::new();
        for line in lines {
            let mut cells = line.split(',').map(|cell| cell.parse::<f64>().ok());
            steps.push(cells.next()??);
            let row: Vec<f64> = cells.collect::<Option<_>>()?;
            if row.len() != budgets.len() {
                return None;
            }
            rows.push(row);
        }
        (!rows.is_empty() && !budgets.is_empty()).then_some(PriceTable {
            steps,
            budgets,
            rows,
        })
    }

    /// Seuil à `steps_left` pas de la fin du mois avec `budget_left` euros restants, un euro au moins.
    ///
    /// Interpolation linéaire en `t` ; en `b`, le point mesuré juste en dessous, dont le seuil est un peu plus haut, donc
    /// prudent (écart d'au plus 3 € par an à 400 € : `net_price.py`). Au-delà de la table : son dernier point.
    pub fn threshold(&self, steps_left: f64, budget_left: u32) -> f64 {
        let column = self
            .budgets
            .iter()
            .rposition(|&b| b <= budget_left)
            .unwrap_or(0);
        let last = self.steps.len() - 1;
        let upper = self
            .steps
            .iter()
            .position(|&t| t >= steps_left)
            .unwrap_or(last);
        if upper == 0 {
            return self.rows[0][column];
        }
        let (t0, t1) = (self.steps[upper - 1], self.steps[upper]);
        let weight = ((steps_left - t0) / (t1 - t0)).clamp(0.0, 1.0);
        (1.0 - weight) * self.rows[upper - 1][column] + weight * self.rows[upper][column]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    #[test]
    fn the_frozen_table_is_well_formed() {
        let table = PriceTable::frozen();
        assert_eq!(table.steps.len(), 25);
        assert_eq!(table.budgets.len(), 260);
        assert_eq!(table.budgets[199], 200);
        assert_eq!(table.budgets[259], 800);
    }

    /// Valeurs relues dans la table complète de l'étude (`net_price.py`).
    #[test]
    fn measured_points_give_their_frozen_threshold() {
        let table = PriceTable::frozen();
        assert_abs_diff_eq!(table.threshold(0.0, 100), 1.0, epsilon = 1e-9);
        let start = table.threshold(120.0, 100);
        assert!((2.0..2.4).contains(&start), "{start}");
    }

    #[test]
    fn the_threshold_falls_with_the_budget_and_rises_with_the_time_left() {
        let table = PriceTable::frozen();
        for t in [0.0, 3.0, 17.5, 60.0, 119.0, 120.0] {
            let bars: Vec<f64> = (1..=900).map(|b| table.threshold(t, b)).collect();
            assert!(bars.windows(2).all(|w| w[1] <= w[0] + 1e-9));
            assert!(bars.iter().all(|&bar| bar >= 1.0));
        }
        for b in [1, 10, 100, 250, 800] {
            let bars: Vec<f64> = (0..=120)
                .map(|t| table.threshold(f64::from(t), b))
                .collect();
            assert!(bars.windows(2).all(|w| w[1] >= w[0] - 1e-3));
        }
    }

    #[test]
    fn the_time_is_interpolated_and_the_budget_takes_the_point_below() {
        let table = PriceTable::frozen();
        let middle = table.threshold(62.5, 100);
        assert_abs_diff_eq!(
            middle,
            (table.threshold(60.0, 100) + table.threshold(65.0, 100)) / 2.0,
            epsilon = 1e-9
        );
        assert_eq!(table.threshold(60.0, 215), table.threshold(60.0, 210));
    }

    #[test]
    fn outside_the_table_the_last_point_holds() {
        let table = PriceTable::frozen();
        assert_eq!(table.threshold(60.0, 5_000), table.threshold(60.0, 800));
        assert_eq!(table.threshold(500.0, 100), table.threshold(120.0, 100));
        assert_eq!(table.threshold(-1.0, 100), table.threshold(0.0, 100));
    }

    #[test]
    fn budgets_outside_the_bounds_are_refused() {
        assert!(check_budget(MIN_BUDGET).is_ok() && check_budget(MAX_BUDGET).is_ok());
        assert!(check_budget(19).is_err());
        assert!(check_budget(401).is_err());
        assert!(check_budget(0).is_err());
    }

    #[test]
    fn a_broken_table_is_refused() {
        assert!(PriceTable::parse("").is_none());
        assert!(PriceTable::parse("t,1,2\n0,1.0\n").is_none());
        assert!(PriceTable::parse("t,1,2\n0,1.0,x\n").is_none());
    }
}
