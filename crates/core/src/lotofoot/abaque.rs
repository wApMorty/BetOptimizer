//! Abaque du seuil τ selon le budget mensuel (`docs/sources.md` §49) : la table gelée des τ* mesurés sur l'entraînement, et
//! la lecture d'un budget saisi au palier inférieur.
//!
//! Le budget se saisit dans l'application ; le seuil n'est plus un réglage, il découle du budget.

use crate::error::{Error, Result};

/// Version de l'abaque, enregistrée avec chaque séance.
pub const VERSION: &str = "abaque-2026-10-24";

/// Budget mensuel minimal et maximal mesurés (euros).
pub const MIN_BUDGET: u32 = 20;
/// Voir [`MIN_BUDGET`].
pub const MAX_BUDGET: u32 = 400;

/// τ* de chaque budget mesuré (pas de 20 €), sur les grilles d'entraînement (`scripts/lotofoot/budget_tau.py`, §49) :
/// le τ qui maximise l'espérance S3 par an, règle séquentielle, plafond par mois calendaire.
const TABLE: [(u32, f64); 20] = [
    (20, 2.0),
    (40, 1.9),
    (60, 1.8),
    (80, 1.8),
    (100, 1.7),
    (120, 1.7),
    (140, 1.6),
    (160, 1.6),
    (180, 1.5),
    (200, 1.5),
    (220, 1.5),
    (240, 1.5),
    (260, 1.4),
    (280, 1.4),
    (300, 1.4),
    (320, 1.3),
    (340, 1.3),
    (360, 1.3),
    (380, 1.3),
    (400, 1.2),
];

/// Seuil τ d'un budget mensuel : celui du point mesuré juste en dessous (70 € → τ* de 60 €). Ce palier donne un τ un peu
/// plus haut, donc prudent, et aucune valeur non mesurée. Un budget hors de [`MIN_BUDGET`, `MAX_BUDGET`] est refusé.
pub fn threshold(monthly_budget: u32) -> Result<f64> {
    if !(MIN_BUDGET..=MAX_BUDGET).contains(&monthly_budget) {
        return Err(Error::Input(format!(
            "budget mensuel de {monthly_budget} € hors de l'abaque : de {MIN_BUDGET} à {MAX_BUDGET} €"
        )));
    }
    // La table est triée par budget croissant et commence à MIN_BUDGET : le dernier point ≤ budget existe toujours.
    let tau = TABLE
        .iter()
        .take_while(|(budget, _)| *budget <= monthly_budget)
        .last()
        .map(|(_, tau)| *tau)
        .expect("la table commence à MIN_BUDGET, qui est ≤ au budget vérifié");
    Ok(tau)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_points_give_their_frozen_tau() {
        assert_eq!(threshold(20).unwrap(), 2.0);
        assert_eq!(threshold(60).unwrap(), 1.8);
        assert_eq!(threshold(100).unwrap(), 1.7);
        assert_eq!(threshold(400).unwrap(), 1.2);
    }

    #[test]
    fn a_budget_between_two_points_takes_the_lower_point() {
        assert_eq!(threshold(70).unwrap(), 1.8);
        assert_eq!(threshold(139).unwrap(), 1.7);
        assert_eq!(threshold(399).unwrap(), 1.3);
    }

    #[test]
    fn budgets_outside_the_abaque_are_refused() {
        assert!(threshold(19).is_err());
        assert!(threshold(401).is_err());
        assert!(threshold(0).is_err());
    }

    #[test]
    fn tau_never_rises_with_the_budget() {
        let taus: Vec<f64> = (MIN_BUDGET..=MAX_BUDGET)
            .map(|b| threshold(b).unwrap())
            .collect();
        assert!(taus.windows(2).all(|w| w[1] <= w[0]));
    }
}
