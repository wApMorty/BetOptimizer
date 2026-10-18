//! Critère de Kelly : la fraction de bankroll qui maximise la croissance logarithmique à long terme.
//!
//! Source : Kelly (1956), « A new interpretation of information rate », *Bell System Technical Journal* 35(4) ;
//! Thorp (2006), « The Kelly criterion in blackjack, sports betting, and the stock market ».

use crate::domain::{Euros, Odds, Probability};

/// Plafond d'une fraction : miser toute la bankroll sur un seul pari ferait de la moindre erreur une ruine.
const MAX_FRACTION: f64 = 0.95;

/// Fraction de Kelly d'un pari seul : `f* = (p · cote − 1) / (cote − 1)`. Nulle sans avantage.
///
/// Si le pari gagne, la mise `f·B` rapporte `f·B·(cote − 1)` ; s'il perd, elle est perdue. La croissance attendue
/// `p·ln(1 + f(cote − 1)) + (1 − p)·ln(1 − f)` est maximale en `f*`.
pub fn kelly_fraction(probability: Probability, odds: Odds) -> f64 {
    let edge = probability.get() * odds.get() - 1.0;
    if edge <= 0.0 {
        return 0.0;
    }
    (edge / (odds.get() - 1.0)).min(MAX_FRACTION)
}

/// Mise en euros pour une fraction de bankroll, arrondie **à la baisse** au dixième d'euro, ou `None` si elle
/// tombe sous la mise minimale (un pari trop petit n'est pas placé).
pub fn stake(bankroll: Euros, fraction: f64, min_stake: Euros) -> Option<Euros> {
    let raw = bankroll.get() * fraction;
    let stake = Euros::floor_to_step(raw)?;
    (stake >= min_stake && stake.get() > 0.0).then_some(stake)
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

    /// Croissance logarithmique attendue d'un pari seul pour une fraction `f`.
    fn growth(p: f64, odds: f64, f: f64) -> f64 {
        p * (1.0 + f * (odds - 1.0)).ln() + (1.0 - p) * (1.0 - f).ln()
    }

    #[test]
    fn kelly_is_zero_without_edge() {
        assert_eq!(kelly_fraction(p(0.5), o(2.0)), 0.0);
        assert_eq!(kelly_fraction(p(0.4), o(2.0)), 0.0);
        assert_eq!(kelly_fraction(p(0.25), o(3.9)), 0.0);
    }

    #[test]
    fn kelly_matches_the_textbook_formula() {
        // p = 0,6, cote 2,0 : f* = (0,6×2 − 1)/(2 − 1) = 0,2.
        assert_abs_diff_eq!(kelly_fraction(p(0.6), o(2.0)), 0.2, epsilon = 1e-12);
        // p = 0,4, cote 3,0 : f* = (0,2)/(2) = 0,1.
        assert_abs_diff_eq!(kelly_fraction(p(0.4), o(3.0)), 0.1, epsilon = 1e-12);
    }

    #[test]
    fn kelly_maximises_the_expected_log_growth() {
        for (probability, odds) in [(0.6, 2.0), (0.4, 3.0), (0.25, 5.2), (0.55, 1.95)] {
            let best = kelly_fraction(p(probability), o(odds));
            let at_best = growth(probability, odds, best);
            // Toute autre fraction proche (en dessous comme au-dessus) fait croître moins vite.
            for factor in [0.5, 0.8, 1.2, 1.6] {
                assert!(
                    growth(probability, odds, best * factor) < at_best,
                    "p={probability} cote={odds} ×{factor}"
                );
            }
        }
    }

    #[test]
    fn a_fraction_is_capped_so_that_the_whole_bankroll_is_never_staked() {
        // Avantage énorme : (0,99 × 10 − 1)/9 ≈ 0,99 → plafonné.
        assert!(kelly_fraction(p(0.99), o(10.0)) <= 0.95);
    }

    #[test]
    fn stakes_are_floored_and_small_ones_are_not_placed() {
        let bankroll = Euros::new(1000.0).unwrap();
        let minimum = Euros::new(1.0).unwrap();
        assert_eq!(stake(bankroll, 0.0237, minimum).unwrap().get(), 23.7);
        assert_eq!(stake(bankroll, 0.0239, minimum).unwrap().get(), 23.9);
        // 0,0009 × 1000 = 0,90 € < mise minimale : ignoré.
        assert!(stake(bankroll, 0.0009, minimum).is_none());
        // Pile à la mise minimale : placé.
        assert_eq!(stake(bankroll, 0.001, minimum).unwrap().get(), 1.0);
        assert!(stake(bankroll, 0.0, minimum).is_none());
    }

    #[test]
    fn rounding_down_can_never_exceed_the_wanted_stake() {
        let bankroll = Euros::new(337.0).unwrap();
        for raw_fraction in [0.0123, 0.0456, 0.0789, 0.1111, 0.2222] {
            if let Some(placed) = stake(bankroll, raw_fraction, Euros::new(0.1).unwrap()) {
                assert!(placed.get() <= bankroll.get() * raw_fraction + 1e-9);
            }
        }
    }
}
