//! Retrait de la marge d'un marché 1X2.
//!
//! Les cotes d'un bookmaker contiennent sa marge : la somme β des probabilités implicites `π_i = 1/cote_i`
//! dépasse 1. Pour comparer une cote à une probabilité « vraie », on cherche trois probabilités `p_i` qui
//! somment à 1. Il n'existe pas de solution unique ; trois méthodes classiques sont implémentées et
//! départagées sur la calibration (voir le rapport `edge market report`).
//!
//! Sources : Štrumbelj (2014), « On determining probability forecasts from betting odds », *International
//! Journal of Forecasting* 30(4) ; Shin (1993), *The Economic Journal* 103(420) ; Clarke, Kovalchik & Ingram
//! (2017), « Adjusting bookmaker's odds to allow for overround », *American Journal of Sports Science*.

use serde::Deserialize;

use crate::domain::{Odds1x2, Probs1x2};

/// Méthode de retrait de la marge. Dans la configuration : `multiplicative`, `power` ou `shin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarginMethod {
    /// Normalisation proportionnelle : la marge est répartie au prorata des probabilités implicites.
    Multiplicative,
    /// Exposant commun : `p_i = π_i^k`. Retire proportionnellement plus de marge aux outsiders.
    Power,
    /// Modèle de Shin : la marge vient d'une fraction `z` de parieurs informés.
    Shin,
}

impl MarginMethod {
    /// Les trois méthodes.
    pub const ALL: [MarginMethod; 3] = [
        MarginMethod::Multiplicative,
        MarginMethod::Power,
        MarginMethod::Shin,
    ];

    /// Nom court (rapport, configuration).
    pub fn name(self) -> &'static str {
        match self {
            MarginMethod::Multiplicative => "multiplicative",
            MarginMethod::Power => "power",
            MarginMethod::Shin => "shin",
        }
    }
}

/// Retire la marge des cotes. Renvoie `None` si la méthode n'a pas de solution (Shin exige β ≥ 1 :
/// en dessous, les cotes offrent un arbitrage et le modèle n'a plus de sens).
pub fn remove_margin(odds: &Odds1x2, method: MarginMethod) -> Option<Probs1x2> {
    let pi = [
        1.0 / odds.home.get(),
        1.0 / odds.draw.get(),
        1.0 / odds.away.get(),
    ];
    let p = match method {
        MarginMethod::Multiplicative => multiplicative(pi),
        MarginMethod::Power => power(pi)?,
        MarginMethod::Shin => shin(pi)?,
    };
    Probs1x2::new(p[0], p[1], p[2])
}

/// `p_i = π_i / β`, avec `β = Σ π_j`.
fn multiplicative(pi: [f64; 3]) -> [f64; 3] {
    let beta: f64 = pi.iter().sum();
    pi.map(|x| x / beta)
}

/// `p_i = π_i^k` où `k` est l'unique solution de `Σ π_i^k = 1`.
///
/// Chaque `π_i < 1`, donc `g(k) = Σ π_i^k − 1` décroît strictement de `g(0) = 2` vers −1 : une seule racine,
/// trouvée par bissection. `k > 1` quand il y a une marge.
fn power(pi: [f64; 3]) -> Option<[f64; 3]> {
    let g = |k: f64| pi.iter().map(|x| x.powf(k)).sum::<f64>() - 1.0;
    let k = bisect(g, 0.0, 1000.0)?;
    Some(pi.map(|x| x.powf(k)))
}

/// Modèle de Shin : `p_i = ( √(z² + 4(1−z)·π_i²/β) − z ) / ( 2(1−z) )`, où `z ∈ [0, 1)` est la part de
/// parieurs informés, fixée par la condition `Σ p_i = 1`.
fn shin(pi: [f64; 3]) -> Option<[f64; 3]> {
    let beta: f64 = pi.iter().sum();
    if (beta - 1.0).abs() < 1e-12 {
        // Cotes sans marge : rien à retirer.
        return Some(pi);
    }
    if beta < 1.0 {
        return None;
    }
    let probs = |z: f64| {
        pi.map(|x| ((z * z + 4.0 * (1.0 - z) * x * x / beta).sqrt() - z) / (2.0 * (1.0 - z)))
    };
    // f(0) = √β − 1 > 0 et f(1⁻) = Σπ²/β − 1 < 0 : la racine est dans l'intervalle ; f décroît.
    let f = |z: f64| probs(z).iter().sum::<f64>() - 1.0;
    let z = bisect(f, 0.0, 1.0 - 1e-12)?;
    Some(probs(z))
}

/// Racine d'une fonction **décroissante** `f` sur `[lo, hi]`, par bissection (200 itérations : la précision
/// du `f64` est atteinte). `None` si `f` ne change pas de signe sur l'intervalle.
///
/// `f` est une fermeture reçue en paramètre : le même algorithme sert à `power` et à `shin`.
fn bisect(f: impl Fn(f64) -> f64, mut lo: f64, mut hi: f64) -> Option<f64> {
    if f(lo) < 0.0 || f(hi) > 0.0 {
        return None;
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if f(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((lo + hi) / 2.0)
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;
    use crate::domain::{Odds, Outcome};

    fn odds(home: f64, draw: f64, away: f64) -> Odds1x2 {
        Odds1x2 {
            home: Odds::new(home).unwrap(),
            draw: Odds::new(draw).unwrap(),
            away: Odds::new(away).unwrap(),
        }
    }

    fn probs(o: &Odds1x2, method: MarginMethod) -> [f64; 3] {
        remove_margin(o, method).unwrap().to_array()
    }

    /// Valeurs de référence calculées à part (programme Python indépendant, bissection à 300 itérations).
    #[test]
    fn matches_independent_reference_values() {
        // Winamax, Bournemouth–Sunderland, 2026-10-18.
        let o = odds(2.10, 3.30, 3.15);
        let m = probs(&o, MarginMethod::Multiplicative);
        assert_abs_diff_eq!(m[0], 0.4342105263, epsilon = 1e-9);
        assert_abs_diff_eq!(m[1], 0.2763157895, epsilon = 1e-9);
        let p = probs(&o, MarginMethod::Power);
        assert_abs_diff_eq!(p[0], 0.4441202000, epsilon = 1e-9);
        assert_abs_diff_eq!(p[1], 0.2708690119, epsilon = 1e-9);
        assert_abs_diff_eq!(p[2], 0.2850107881, epsilon = 1e-9);
        let s = probs(&o, MarginMethod::Shin);
        assert_abs_diff_eq!(s[0], 0.4413834705, epsilon = 1e-9);
        assert_abs_diff_eq!(s[1], 0.2722700837, epsilon = 1e-9);
        assert_abs_diff_eq!(s[2], 0.2863464458, epsilon = 1e-9);

        // Pinnacle, même match : marge plus faible, les méthodes se rapprochent.
        let o = odds(2.22, 3.38, 3.55);
        let s = probs(&o, MarginMethod::Shin);
        assert_abs_diff_eq!(s[0], 0.4403711309, epsilon = 1e-9);
        assert_abs_diff_eq!(s[2], 0.2727803436, epsilon = 1e-9);

        // Gros favori.
        let o = odds(1.25, 6.5, 12.0);
        let p = probs(&o, MarginMethod::Power);
        assert_abs_diff_eq!(p[0], 0.7897473568, epsilon = 1e-9);
        let s = probs(&o, MarginMethod::Shin);
        assert_abs_diff_eq!(s[0], 0.7834627575, epsilon = 1e-9);
    }

    #[test]
    fn every_method_sums_to_one_on_a_grid_of_realistic_odds() {
        let mut checked = 0;
        for home in [1.1, 1.5, 2.0, 3.0, 5.0, 9.0] {
            for draw in [2.9, 3.4, 4.5, 7.0] {
                for away in [1.4, 2.5, 4.0, 8.0, 15.0] {
                    let o = odds(home, draw, away);
                    if !(1.0..=1.3).contains(&o.implied_sum()) {
                        continue;
                    }
                    for method in MarginMethod::ALL {
                        let sum: f64 = probs(&o, method).iter().sum();
                        assert_abs_diff_eq!(sum, 1.0, epsilon = 1e-12);
                    }
                    checked += 1;
                }
            }
        }
        assert!(
            checked > 20,
            "la grille doit contenir assez de cas ({checked})"
        );
    }

    #[test]
    fn fair_odds_are_left_unchanged_by_every_method() {
        let o = odds(2.0, 4.0, 4.0);
        for method in MarginMethod::ALL {
            let p = probs(&o, method);
            assert_abs_diff_eq!(p[0], 0.5, epsilon = 1e-9);
            assert_abs_diff_eq!(p[1], 0.25, epsilon = 1e-9);
        }
    }

    #[test]
    fn equal_odds_give_equal_probabilities() {
        let o = odds(3.0, 3.0, 3.0);
        for method in MarginMethod::ALL {
            for p in probs(&o, method) {
                assert_abs_diff_eq!(p, 1.0 / 3.0, epsilon = 1e-12);
            }
        }
    }

    #[test]
    fn the_order_of_outcomes_is_preserved() {
        let o = odds(1.8, 3.6, 4.8);
        for method in MarginMethod::ALL {
            let p = probs(&o, method);
            assert!(p[0] > p[1] && p[1] > p[2], "{method:?} : {p:?}");
        }
    }

    #[test]
    fn power_and_shin_give_the_favourite_more_than_the_multiplicative_method() {
        // Biais favori / outsider : ces méthodes retirent davantage de marge aux outsiders.
        let o = odds(1.25, 6.5, 12.0);
        let m = probs(&o, MarginMethod::Multiplicative);
        for method in [MarginMethod::Power, MarginMethod::Shin] {
            let p = probs(&o, method);
            assert!(p[0] > m[0], "{method:?} doit donner plus au favori");
            assert!(p[2] < m[2], "{method:?} doit donner moins à l'outsider");
        }
    }

    #[test]
    fn shin_needs_a_margin_but_the_others_still_work_below_one() {
        // Cotes de « Max » (meilleure cote de chaque issue) : la somme peut passer sous 1.
        let o = odds(2.2, 3.6, 3.9);
        assert!(o.implied_sum() < 1.0);
        assert!(remove_margin(&o, MarginMethod::Shin).is_none());
        for method in [MarginMethod::Multiplicative, MarginMethod::Power] {
            let sum: f64 = probs(&o, method).iter().sum();
            assert_abs_diff_eq!(sum, 1.0, epsilon = 1e-12);
        }
    }

    #[test]
    fn probabilities_are_exposed_by_outcome() {
        let o = odds(2.10, 3.30, 3.15);
        let p = remove_margin(&o, MarginMethod::Multiplicative).unwrap();
        assert_abs_diff_eq!(p.get(Outcome::Home).get(), 0.4342105263, epsilon = 1e-9);
    }
}
