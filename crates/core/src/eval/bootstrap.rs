//! Intervalle de confiance d'une moyenne par bootstrap **par blocs**.
//!
//! Les matchs d'une même journée ne sont pas indépendants (conditions communes, calendrier, décisions des
//! bookmakers). On rééchantillonne donc des blocs entiers (une journée = un bloc), pas des matchs isolés :
//! sinon l'intervalle serait trop étroit et on prendrait du bruit pour un avantage.
//!
//! Le générateur pseudo-aléatoire est écrit à la main (SplitMix64) : déterministe, sans dépendance, et le
//! résultat est reproductible à graine égale (invariant 9 du projet).

/// Résultat : moyenne observée et bornes à 95 % (percentiles 2,5 % et 97,5 %).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct MeanCi {
    /// Moyenne sur l'échantillon réel.
    pub mean: f64,
    /// Borne basse à 95 %.
    pub low: f64,
    /// Borne haute à 95 %.
    pub high: f64,
}

impl MeanCi {
    /// `true` si zéro est hors de l'intervalle : la différence mesurée est distinguable du bruit.
    pub fn excludes_zero(&self) -> bool {
        self.low > 0.0 || self.high < 0.0
    }
}

/// Générateur SplitMix64 (Steele, Lea & Flood, 2014, « Fast splittable pseudorandom number generators »).
pub(crate) struct SplitMix64(u64);

impl SplitMix64 {
    /// Générateur initialisé par une graine : même graine, même suite.
    pub(crate) fn new(seed: u64) -> SplitMix64 {
        SplitMix64(seed)
    }

    /// Réel uniforme dans [0, 1) (53 bits aléatoires).
    pub(crate) fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Entier dans `0..n`. Le léger biais du modulo est négligeable pour n ≪ 2⁶⁴.
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Moyenne de `values` (couples `(bloc, valeur)`) et son intervalle de confiance à 95 %.
///
/// Chaque rééchantillon tire autant de blocs que l'original, avec remise, puis recalcule la moyenne sur
/// l'ensemble des valeurs des blocs tirés. Renvoie `None` si `values` est vide.
pub fn block_bootstrap_mean(values: &[(u32, f64)], resamples: usize, seed: u64) -> Option<MeanCi> {
    if values.is_empty() {
        return None;
    }
    // Un bloc = (somme des valeurs, nombre de valeurs). Le tri regroupe les valeurs d'un même bloc.
    let mut sorted = values.to_vec();
    sorted.sort_by_key(|(block, _)| *block);
    let mut blocks: Vec<(f64, f64)> = Vec::new();
    let mut previous = None;
    for (block, value) in sorted {
        if previous == Some(block) {
            // Le dernier bloc de la liste est celui en cours : on l'incrémente sur place.
            if let Some(last) = blocks.last_mut() {
                last.0 += value;
                last.1 += 1.0;
            }
        } else {
            blocks.push((value, 1.0));
            previous = Some(block);
        }
    }

    let total_sum: f64 = blocks.iter().map(|b| b.0).sum();
    let total_count: f64 = blocks.iter().map(|b| b.1).sum();
    let mean = total_sum / total_count;

    let mut rng = SplitMix64::new(seed);
    let mut means = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let (mut sum, mut count) = (0.0, 0.0);
        for _ in 0..blocks.len() {
            let block = blocks[rng.below(blocks.len())];
            sum += block.0;
            count += block.1;
        }
        means.push(sum / count);
    }
    means.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| means[((q * (resamples - 1) as f64).round() as usize).min(resamples - 1)];
    Some(MeanCi {
        mean,
        low: at(0.025),
        high: at(0.975),
    })
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    #[test]
    fn constant_values_give_a_degenerate_interval() {
        let values: Vec<(u32, f64)> = (0..50).map(|i| (i / 5, 0.7)).collect();
        let ci = block_bootstrap_mean(&values, 500, 1).unwrap();
        assert_abs_diff_eq!(ci.mean, 0.7, epsilon = 1e-12);
        assert_abs_diff_eq!(ci.low, 0.7, epsilon = 1e-12);
        assert_abs_diff_eq!(ci.high, 0.7, epsilon = 1e-12);
    }

    #[test]
    fn the_interval_contains_the_mean_and_is_ordered() {
        let values: Vec<(u32, f64)> = (0..200).map(|i| (i / 10, (i % 7) as f64)).collect();
        let ci = block_bootstrap_mean(&values, 1000, 42).unwrap();
        assert!(ci.low <= ci.mean && ci.mean <= ci.high, "{ci:?}");
        assert!(ci.high > ci.low);
    }

    #[test]
    fn the_same_seed_gives_the_same_result() {
        let values: Vec<(u32, f64)> = (0..100).map(|i| (i / 4, (i * 37 % 11) as f64)).collect();
        let a = block_bootstrap_mean(&values, 300, 7).unwrap();
        let b = block_bootstrap_mean(&values, 300, 7).unwrap();
        let c = block_bootstrap_mean(&values, 300, 8).unwrap();
        assert_eq!(a, b);
        assert_ne!(
            a, c,
            "une autre graine doit donner un autre échantillonnage"
        );
    }

    #[test]
    fn blocks_are_resampled_whole() {
        // Deux blocs de valeurs opposées : toute moyenne rééchantillonnée vaut -1, 0 ou +1
        // (jamais une valeur intermédiaire, qui trahirait un rééchantillonnage match par match).
        let values = vec![
            (0, 1.0),
            (0, 1.0),
            (0, 1.0),
            (1, -1.0),
            (1, -1.0),
            (1, -1.0),
        ];
        let ci = block_bootstrap_mean(&values, 400, 3).unwrap();
        assert_abs_diff_eq!(ci.mean, 0.0, epsilon = 1e-12);
        assert_abs_diff_eq!(ci.low, -1.0, epsilon = 1e-12);
        assert_abs_diff_eq!(ci.high, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn a_clear_shift_excludes_zero() {
        let values: Vec<(u32, f64)> = (0..400)
            .map(|i| (i / 8, 0.05 + 0.01 * ((i % 5) as f64)))
            .collect();
        let ci = block_bootstrap_mean(&values, 800, 9).unwrap();
        assert!(ci.excludes_zero());
    }

    #[test]
    fn empty_input_gives_none() {
        assert!(block_bootstrap_mean(&[], 100, 1).is_none());
    }
}
