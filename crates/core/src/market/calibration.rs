//! Calibration d'un pronostiqueur probabiliste : diagramme de fiabilité et tranches de cote.
//!
//! Un pronostiqueur est calibré si, parmi les événements auxquels il donne 30 %, environ 30 % se produisent.
//! La calibration prime sur la précision (invariant 3 du projet). Ces fonctions sont pures : elles reçoivent
//! des paires (probabilité prévue, événement réalisé) et rendent des tableaux.

use serde::Serialize;

/// Quantile 97,5 % de la loi normale centrée réduite (intervalles à 95 %).
const Z_95: f64 = 1.959_963_984_540_054;

/// Intervalle de Wilson à 95 % d'une proportion observée : `k` succès sur `n` essais.
///
/// Forme : centre `(p̂ + z²/2n) / (1 + z²/n)`, demi-largeur `z·√(p̂(1−p̂)/n + z²/4n²) / (1 + z²/n)`.
/// Meilleur que l'intervalle normal pour les petits effectifs et les proportions proches de 0 ou 1.
/// Source : Wilson (1927), « Probable inference, the law of succession, and statistical inference »,
/// *JASA* 22(158).
pub fn wilson_interval(successes: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let n = n as f64;
    let p = successes as f64 / n;
    let denominator = 1.0 + Z_95 * Z_95 / n;
    let center = (p + Z_95 * Z_95 / (2.0 * n)) / denominator;
    let half = Z_95 * (p * (1.0 - p) / n + Z_95 * Z_95 / (4.0 * n * n)).sqrt() / denominator;
    ((center - half).max(0.0), (center + half).min(1.0))
}

/// Une classe de probabilités prévues et ce qui s'est réellement produit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bin {
    /// Borne basse de la classe.
    pub lower: f64,
    /// Borne haute de la classe.
    pub upper: f64,
    /// Nombre de prévisions dans la classe.
    pub n: usize,
    /// Probabilité moyenne prévue.
    pub mean_predicted: f64,
    /// Fréquence observée de l'événement.
    pub observed: f64,
    /// Borne basse de l'intervalle de Wilson à 95 % de la fréquence observée.
    pub wilson_low: f64,
    /// Borne haute de l'intervalle de Wilson à 95 %.
    pub wilson_high: f64,
}

/// Diagramme de fiabilité : regroupe les paires `(probabilité prévue, s'est produit)` en classes de largeur
/// `bin_width` et compare la probabilité moyenne prévue à la fréquence observée. Les classes vides sont omises.
pub fn reliability(pairs: &[(f64, bool)], bin_width: f64) -> Vec<Bin> {
    let bin_count = (1.0 / bin_width).round() as usize;
    let mut sums = vec![(0usize, 0.0f64, 0usize); bin_count]; // (effectif, somme des prévisions, succès)
    for &(p, happened) in pairs {
        // p = 1,0 appartient à la dernière classe (sinon il tomberait dans une classe inexistante).
        let index = ((p / bin_width).floor() as usize).min(bin_count - 1);
        sums[index].0 += 1;
        sums[index].1 += p;
        sums[index].2 += usize::from(happened);
    }
    sums.iter()
        .enumerate()
        .filter(|(_, (n, _, _))| *n > 0)
        .map(|(index, &(n, sum_p, successes))| {
            let (wilson_low, wilson_high) = wilson_interval(successes, n);
            Bin {
                lower: index as f64 * bin_width,
                upper: (index + 1) as f64 * bin_width,
                n,
                mean_predicted: sum_p / n as f64,
                observed: successes as f64 / n as f64,
                wilson_low,
                wilson_high,
            }
        })
        .collect()
}

/// Bornes des tranches de cote « juste » (inverse de la probabilité sans marge).
pub const BUCKET_EDGES: [f64; 7] = [1.0, 1.5, 2.0, 3.0, 5.0, 10.0, f64::INFINITY];

/// Une sélection (un match, une issue) pour l'analyse par tranche de cote.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucketInput {
    /// Probabilité sans marge de l'issue.
    pub fair_probability: f64,
    /// Cote brute du bookmaker (marge comprise) : celle à laquelle on parierait.
    pub raw_odds: f64,
    /// L'issue s'est-elle produite ?
    pub won: bool,
}

/// Résultat d'une tranche de cote.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OddsBucket {
    /// Libellé lisible (`2,00 – 3,00`).
    pub label: String,
    /// Nombre de sélections.
    pub n: usize,
    /// Probabilité sans marge moyenne.
    pub mean_fair_probability: f64,
    /// Fréquence observée.
    pub observed: f64,
    /// Rendement moyen d'une mise de 1 sur chaque sélection, à la cote brute : `moyenne(gagné·cote − 1)`.
    pub roi: f64,
    /// Erreur-type du rendement (écart-type / √n), indépendance supposée : indicatif seulement, car les
    /// matchs d'une même journée ne sont pas indépendants.
    pub roi_standard_error: f64,
}

/// Regroupe les sélections par tranche de cote juste et calcule fréquence observée et rendement.
///
/// Sert à voir le biais favori / outsider : si les outsiders gagnent moins souvent que ne le dit le marché,
/// leur rendement est nettement plus négatif que celui des favoris.
pub fn odds_buckets(rows: &[BucketInput]) -> Vec<OddsBucket> {
    let mut buckets = Vec::new();
    for window in BUCKET_EDGES.windows(2) {
        let (low, high) = (window[0], window[1]);
        let selected: Vec<&BucketInput> = rows
            .iter()
            .filter(|row| {
                let fair_odds = 1.0 / row.fair_probability;
                fair_odds >= low && fair_odds < high
            })
            .collect();
        if selected.is_empty() {
            continue;
        }
        let n = selected.len() as f64;
        let returns: Vec<f64> = selected
            .iter()
            .map(|row| if row.won { row.raw_odds - 1.0 } else { -1.0 })
            .collect();
        let roi = returns.iter().sum::<f64>() / n;
        let variance = if selected.len() > 1 {
            returns.iter().map(|r| (r - roi).powi(2)).sum::<f64>() / (n - 1.0)
        } else {
            0.0
        };
        buckets.push(OddsBucket {
            label: bucket_label(low, high),
            n: selected.len(),
            mean_fair_probability: selected.iter().map(|r| r.fair_probability).sum::<f64>() / n,
            observed: selected.iter().filter(|r| r.won).count() as f64 / n,
            roi,
            roi_standard_error: (variance / n).sqrt(),
        });
    }
    buckets
}

fn bucket_label(low: f64, high: f64) -> String {
    let comma = |x: f64| format!("{x:.2}").replace('.', ",");
    if low <= 1.0 {
        format!("< {}", comma(high))
    } else if high.is_infinite() {
        format!("≥ {}", comma(low))
    } else {
        format!("{} – {}", comma(low), comma(high))
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    #[test]
    fn wilson_matches_reference_values_computed_separately() {
        let (low, high) = wilson_interval(50, 100);
        assert_abs_diff_eq!(low, 0.4038315304, epsilon = 1e-9);
        assert_abs_diff_eq!(high, 0.5961684696, epsilon = 1e-9);
        let (low, high) = wilson_interval(0, 10);
        assert_abs_diff_eq!(low, 0.0, epsilon = 1e-12);
        assert_abs_diff_eq!(high, 0.2775327999, epsilon = 1e-9);
        let (low, high) = wilson_interval(10, 10);
        assert_abs_diff_eq!(low, 0.7224672001, epsilon = 1e-9);
        assert_abs_diff_eq!(high, 1.0, epsilon = 1e-9);
        let (low, high) = wilson_interval(30, 200);
        assert_abs_diff_eq!(low, 0.1071359356, epsilon = 1e-9);
        assert_abs_diff_eq!(high, 0.2060557928, epsilon = 1e-9);
    }

    #[test]
    fn wilson_without_data_is_the_whole_interval() {
        assert_eq!(wilson_interval(0, 0), (0.0, 1.0));
    }

    #[test]
    fn reliability_groups_pairs_and_skips_empty_bins() {
        // 100 prévisions à 0,22 (classe 0,20–0,25) dont 22 réalisées ; 50 à 0,81 dont 40 réalisées.
        let mut pairs: Vec<(f64, bool)> = (0..100).map(|i| (0.22, i < 22)).collect();
        pairs.extend((0..50).map(|i| (0.81, i < 40)));

        let bins = reliability(&pairs, 0.05);

        assert_eq!(bins.len(), 2);
        assert_abs_diff_eq!(bins[0].lower, 0.20, epsilon = 1e-12);
        assert_abs_diff_eq!(bins[0].upper, 0.25, epsilon = 1e-12);
        assert_eq!(bins[0].n, 100);
        assert_abs_diff_eq!(bins[0].mean_predicted, 0.22, epsilon = 1e-12);
        assert_abs_diff_eq!(bins[0].observed, 0.22, epsilon = 1e-12);
        assert!(bins[0].wilson_low < 0.22 && 0.22 < bins[0].wilson_high);
        assert_eq!(bins[1].n, 50);
        assert_abs_diff_eq!(bins[1].observed, 0.80, epsilon = 1e-12);
    }

    #[test]
    fn a_probability_of_exactly_one_falls_in_the_last_bin() {
        let bins = reliability(&[(1.0, true), (0.0, false)], 0.1);
        assert_eq!(bins.len(), 2);
        assert_abs_diff_eq!(bins[0].lower, 0.0);
        assert_abs_diff_eq!(bins[1].upper, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn bin_counts_add_up_to_the_input() {
        let pairs: Vec<(f64, bool)> = (0..1000)
            .map(|i| (f64::from(i) / 1000.0, i % 3 == 0))
            .collect();
        let total: usize = reliability(&pairs, 0.05).iter().map(|b| b.n).sum();
        assert_eq!(total, 1000);
    }

    #[test]
    fn odds_buckets_compute_frequency_and_flat_stake_return_by_hand() {
        // Quatre sélections de cote juste 2,5 (probabilité 0,4), cote brute 2,4, une seule gagnante.
        let row = |won| BucketInput {
            fair_probability: 0.4,
            raw_odds: 2.4,
            won,
        };
        let rows = [row(true), row(false), row(false), row(false)];

        let buckets = odds_buckets(&rows);

        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].label, "2,00 – 3,00");
        assert_eq!(buckets[0].n, 4);
        assert_abs_diff_eq!(buckets[0].observed, 0.25, epsilon = 1e-12);
        // (1,4 − 1 − 1 − 1) / 4 = −0,40
        assert_abs_diff_eq!(buckets[0].roi, -0.40, epsilon = 1e-12);
        assert!(buckets[0].roi_standard_error > 0.0);
    }

    #[test]
    fn odds_buckets_split_favourites_from_longshots() {
        let rows = [
            BucketInput {
                fair_probability: 0.80,
                raw_odds: 1.2,
                won: true,
            }, // cote juste 1,25
            BucketInput {
                fair_probability: 0.05,
                raw_odds: 18.0,
                won: false,
            }, // cote juste 20
        ];
        let buckets = odds_buckets(&rows);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].label, "< 1,50");
        assert_eq!(buckets[1].label, "≥ 10,00");
    }
}
