//! Scores d'une prévision 1X2 face à l'issue réellement observée.

use crate::domain::{Outcome, Probs1x2};

/// Log loss (entropie croisée) : `−ln p_obs`, où `p_obs` est la probabilité donnée à l'issue réalisée.
///
/// Une prévision certaine et fausse (`p_obs = 0`) donne `+∞` : c'est mathématiquement correct et voulu,
/// on ne « corrige » pas. Référence : Gneiting & Raftery (2007), « Strictly proper scoring rules,
/// prediction, and estimation », *JASA* 102(477).
pub fn log_loss(forecast: &Probs1x2, actual: Outcome) -> f64 {
    -forecast.get(actual).get().ln()
}

/// Score de Brier multiclasse : `Σ_i (p_i − o_i)²`, avec `o_i = 1` pour l'issue réalisée et 0 sinon.
/// Va de 0 (parfait) à 2. Référence : Brier (1950), *Monthly Weather Review* 78(1).
pub fn brier(forecast: &Probs1x2, actual: Outcome) -> f64 {
    let p = forecast.to_array();
    let observed = observed(actual);
    (0..3).map(|i| (p[i] - observed[i]).powi(2)).sum()
}

/// RPS (*ranked probability score*) : `1/(r−1) · Σ_{k=1}^{r−1} ( Σ_{i≤k} (p_i − o_i) )²`, avec `r = 3`.
///
/// Contrairement au Brier, il tient compte de l'ordre des issues (domicile < nul < extérieur) : prédire
/// « victoire à domicile » quand le match est nul est moins grave que quand l'extérieur gagne. Va de 0 à 1.
/// Références : Epstein (1969), *Journal of Applied Meteorology* 8(6) ; Constantinou & Fenton (2012),
/// *Journal of Quantitative Analysis in Sports* 8(1).
pub fn rps(forecast: &Probs1x2, actual: Outcome) -> f64 {
    let p = forecast.to_array();
    let observed = observed(actual);
    let mut cumulative_gap = 0.0;
    let mut sum = 0.0;
    // Les deux premières sommes cumulées suffisent : la troisième vaut toujours 0 (les deux vecteurs somment à 1).
    for i in 0..2 {
        cumulative_gap += p[i] - observed[i];
        sum += cumulative_gap.powi(2);
    }
    sum / 2.0
}

/// Vecteur « one-hot » de l'issue réalisée, dans l'ordre [`Outcome::ALL`].
fn observed(actual: Outcome) -> [f64; 3] {
    let mut v = [0.0; 3];
    v[actual.index()] = 1.0;
    v
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn probs(home: f64, draw: f64, away: f64) -> Probs1x2 {
        Probs1x2::new(home, draw, away).unwrap()
    }

    #[test]
    fn a_perfect_forecast_scores_zero() {
        let p = probs(1.0, 0.0, 0.0);
        assert_abs_diff_eq!(log_loss(&p, Outcome::Home), 0.0);
        assert_abs_diff_eq!(brier(&p, Outcome::Home), 0.0);
        assert_abs_diff_eq!(rps(&p, Outcome::Home), 0.0);
    }

    #[test]
    fn a_uniform_forecast_has_known_scores() {
        let p = probs(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0);
        for outcome in Outcome::ALL {
            assert_abs_diff_eq!(log_loss(&p, outcome), 3.0_f64.ln(), epsilon = 1e-12);
            // (2/3)² + 2·(1/3)² = 6/9
            assert_abs_diff_eq!(brier(&p, outcome), 6.0 / 9.0, epsilon = 1e-12);
        }
        // Calculs à la main : victoire dom. [(1/3−1)² + (2/3−1)²]/2 = 5/18 ; nul [(1/3)² + (2/3−1)²]/2 = 1/9.
        assert_abs_diff_eq!(rps(&p, Outcome::Home), 5.0 / 18.0, epsilon = 1e-12);
        assert_abs_diff_eq!(rps(&p, Outcome::Draw), 1.0 / 9.0, epsilon = 1e-12);
        assert_abs_diff_eq!(rps(&p, Outcome::Away), 5.0 / 18.0, epsilon = 1e-12);
    }

    #[test]
    fn log_loss_follows_the_probability_of_the_realised_outcome() {
        let p = probs(0.5, 0.3, 0.2);
        assert_abs_diff_eq!(
            log_loss(&p, Outcome::Home),
            -(0.5_f64.ln()),
            epsilon = 1e-12
        );
        assert_abs_diff_eq!(
            log_loss(&p, Outcome::Away),
            -(0.2_f64.ln()),
            epsilon = 1e-12
        );
    }

    #[test]
    fn a_certain_and_wrong_forecast_has_infinite_log_loss() {
        let p = probs(1.0, 0.0, 0.0);
        assert!(log_loss(&p, Outcome::Away).is_infinite());
    }

    #[test]
    fn brier_example_computed_by_hand() {
        // p = (0.5, 0.3, 0.2), nul réalisé : 0.25 + 0.49 + 0.04 = 0.78
        assert_abs_diff_eq!(
            brier(&probs(0.5, 0.3, 0.2), Outcome::Draw),
            0.78,
            epsilon = 1e-12
        );
    }

    #[test]
    fn rps_penalises_a_far_miss_more_than_a_near_miss() {
        // Même prévision (forte confiance en la victoire à domicile) : le nul est plus proche que la défaite.
        let p = probs(0.8, 0.1, 0.1);
        assert!(rps(&p, Outcome::Draw) < rps(&p, Outcome::Away));
        // Brier, lui, ne distingue pas les deux cas.
        assert_abs_diff_eq!(
            brier(&p, Outcome::Draw),
            brier(&p, Outcome::Away),
            epsilon = 1e-12
        );
    }

    #[test]
    fn rps_example_computed_by_hand() {
        // p = (0.5, 0.3, 0.2), extérieur réalisé : o = (0,0,1)
        // k=1 : 0.5 ; k=2 : 0.8 ; RPS = (0.25 + 0.64)/2 = 0.445
        assert_abs_diff_eq!(
            rps(&probs(0.5, 0.3, 0.2), Outcome::Away),
            0.445,
            epsilon = 1e-12
        );
    }
}
