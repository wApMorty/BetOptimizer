//! Valeur d'un pari : l'espérance de gain d'une mise d'un euro.

use crate::domain::{Odds, Odds1x2, Outcome, Probability, Probs1x2};

/// Espérance de gain d'une mise de 1 € : `p · cote − 1`.
///
/// Positive : le prix est supérieur à la cote « juste » `1/p`, on parle de valeur (EV+). Elle ne vaut que par la
/// qualité de `p` : une probabilité fausse produit une valeur illusoire, d'autant plus grande que l'erreur l'est.
pub fn expected_value(probability: Probability, odds: Odds) -> f64 {
    probability.get() * odds.get() - 1.0
}

/// Cote minimale en dessous de laquelle l'espérance tombe sous `min_edge` : `(1 + min_edge) / p`.
///
/// C'est la cote affichée avec chaque recommandation : si le prix a baissé entre-temps, on ne parie plus.
pub fn minimal_odds(probability: Probability, min_edge: f64) -> f64 {
    (1.0 + min_edge) / probability.get()
}

/// Une issue retenue pour un match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    /// Issue sur laquelle parier.
    pub outcome: Outcome,
    /// Cote proposée.
    pub odds: Odds,
    /// Probabilité estimée de l'issue.
    pub probability: Probability,
    /// Espérance de gain d'une mise de 1 € (`p · cote − 1`).
    pub edge: f64,
}

/// Meilleure issue d'un match : celle de plus forte espérance, si elle atteint `min_edge`.
///
/// Au plus **une** sélection par match (règle du projet) : jouer deux issues du même match se neutralise en partie
/// et double la commission du bookmaker.
pub fn best_selection(probs: &Probs1x2, odds: &Odds1x2, min_edge: f64) -> Option<Selection> {
    Outcome::ALL
        .into_iter()
        .map(|outcome| {
            let probability = probs.get(outcome);
            let price = odds.get(outcome);
            Selection {
                outcome,
                odds: price,
                probability,
                edge: expected_value(probability, price),
            }
        })
        .max_by(|a, b| a.edge.total_cmp(&b.edge))
        .filter(|selection| selection.edge >= min_edge)
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn p(x: f64) -> Probability {
        Probability::new(x).unwrap()
    }

    fn o(x: f64) -> Odds {
        Odds::new(x).unwrap()
    }

    #[test]
    fn expected_value_is_probability_times_odds_minus_one() {
        assert_abs_diff_eq!(expected_value(p(0.5), o(2.0)), 0.0, epsilon = 1e-12);
        assert_abs_diff_eq!(expected_value(p(0.5), o(2.2)), 0.10, epsilon = 1e-12);
        assert_abs_diff_eq!(expected_value(p(0.25), o(3.0)), -0.25, epsilon = 1e-12);
    }

    #[test]
    fn the_minimal_odds_are_exactly_the_odds_that_give_the_required_edge() {
        let minimal = minimal_odds(p(0.4), 0.05);
        assert_abs_diff_eq!(minimal, 2.625, epsilon = 1e-12);
        assert_abs_diff_eq!(expected_value(p(0.4), o(minimal)), 0.05, epsilon = 1e-12);
        // Sans exigence d'avantage, c'est la cote juste 1/p.
        assert_abs_diff_eq!(minimal_odds(p(0.4), 0.0), 2.5, epsilon = 1e-12);
    }

    #[test]
    fn only_the_best_outcome_of_a_match_is_selected() {
        let probs = Probs1x2::new(0.50, 0.28, 0.22).unwrap();
        let odds = Odds1x2 {
            home: o(2.10),
            draw: o(3.90),
            away: o(4.80),
        };
        // EV : domicile +5 %, nul +9,2 %, extérieur +5,6 % : seule le nul est retenu.

        let selection = best_selection(&probs, &odds, 0.0).unwrap();

        assert_eq!(selection.outcome, Outcome::Draw);
        assert_abs_diff_eq!(selection.edge, 0.28 * 3.90 - 1.0, epsilon = 1e-12);
    }

    #[test]
    fn nothing_is_selected_below_the_required_edge() {
        let probs = Probs1x2::new(0.50, 0.28, 0.22).unwrap();
        let odds = Odds1x2 {
            home: o(2.10),
            draw: o(3.90),
            away: o(4.80),
        };
        assert!(best_selection(&probs, &odds, 0.10).is_none());
        // Cotes toutes inférieures aux cotes justes : jamais de sélection, même sans exigence.
        let tight = Odds1x2 {
            home: o(1.90),
            draw: o(3.40),
            away: o(4.40),
        };
        assert!(best_selection(&probs, &tight, 0.0).is_none());
    }
}
