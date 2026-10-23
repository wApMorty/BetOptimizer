//! Règles des jeux, telles qu'homologuées par l'ANJ : probabilités des rangs, prix, clés de répartition.
//!
//! Sources (lues le 2026-10-23, `docs/sources.md` §44) : règlement Loto, décision ANJ n° 2026-PR-023 (articles 5 et 8) ;
//! règlement EuroMillions, décision ANJ n° 2025-PR-054 (articles 4.2.2 à 4.2.5). Une EuroMillions modifiée est autorisée
//! pour 2027 (décision 2026-136, « retour à l'ancienne matrice de jeu ») : ces constantes seront à revoir à son entrée en
//! vigueur, et [`crate::loto::alert`] refuse de conclure si les gagnants publiés ne collent plus à la matrice.

/// Numéros du Loto : 5 parmi 49.
pub const LOTO_BALLS: u8 = 49;
/// Numéros chance du Loto : 1 parmi 10.
pub const LOTO_CHANCES: u8 = 10;

/// Prix d'une grille de Grand Loto, en euros (art. 5 : « 5 € »).
pub const GRAND_LOTO_PRICE: f64 = 5.0;
/// Gain fixe du rang 9 au Grand Loto, en euros (art. 8.1.1.3) : la mise est rendue.
pub const GRAND_LOTO_RANK9_GAIN: f64 = 5.0;
/// Part des mises dévolue aux gagnants du Loto (art. 8.1.1.3), définie globalement et non tirage par tirage.
pub const LOTO_WINNERS_SHARE: f64 = 0.5485;
/// Part des mises d'un Grand Loto qui finance le rang 1 (art. 8.3.1 bis 1).
pub const GRAND_LOTO_JACKPOT_SHARE: f64 = 0.055;
/// Part des mises d'un Grand Loto versée au Fonds de Super Cagnotte (art. 8.3.1 bis 1).
pub const GRAND_LOTO_SUPER_FUND_SHARE: f64 = 0.0285;
/// Clé de répartition des rangs 2 à 8 d'un Grand Loto (art. 8.1.1.3, colonne « tirage Grand Loto ») : `[0]` est le
/// rang 2. Elle s'applique au reliquat des mises, après le rang 1, le Fonds de Super Cagnotte et les gains du rang 9.
pub const GRAND_LOTO_LOWER_KEYS: [f64; 7] =
    [0.0506, 0.0309, 0.1114, 0.0532, 0.2395, 0.1118, 0.4026];
/// Part du jackpot non gagné d'un Grand Loto affectée à chaque rang inférieur (art. 8.3.4) : `[0]` est le rang 2,
/// `[7]` le rang 9. Elle se partage à parts égales entre les gagnants du rang.
pub const GRAND_LOTO_REDISTRIBUTION: [f64; 8] = [
    0.0128, 0.0077, 0.0348, 0.0131, 0.0596, 0.1123, 0.5105, 0.2492,
];

/// Numéros de l'EuroMillions : 5 parmi 50.
pub const EURO_BALLS: u8 = 50;
/// Étoiles de l'EuroMillions : 2 parmi 12.
pub const EURO_STARS: u8 = 12;
/// Prix d'une grille d'EuroMillions en France, en euros (My Million compris).
pub const EURO_PRICE: f64 = 2.50;
/// Part de la mise dévolue aux gagnants, par grille et par pays, en euros (art. 4.2.2.1).
pub const EURO_WINNERS_PER_GRID: f64 = 1.10;
/// Plafond du rang 1, en euros (art. 4.2.5.5 et 4.2.5.7 : 200 M€ en 2020, plus 10 M€ par cycle, au plus 250 M€).
pub const EURO_CAP: f64 = 250_000_000.0;
/// Tirage consécutif au plafond où, sans gagnant, le jackpot va au rang inférieur (art. 4.2.5.6).
pub const EURO_ROLLDOWN_DRAW: usize = 5;
/// Clé de répartition des rangs 2 à 13 de l'EuroMillions (art. 4.2.4.1) : `[0]` est le rang 2. Elle s'applique à la
/// part des mises dévolue aux gagnants (1,10 € par grille).
pub const EURO_RANK_SHARES: [f64; 12] = [
    0.0261, 0.0061, 0.0019, 0.0035, 0.0037, 0.0026, 0.0130, 0.0145, 0.0270, 0.0327, 0.1030, 0.1659,
];

/// Rangs de l'EuroMillions dans l'ordre : (numéros trouvés, étoiles trouvées). `[0]` est le rang 1.
const EURO_RANKS: [(u8, u8); 13] = [
    (5, 2),
    (5, 1),
    (5, 0),
    (4, 2),
    (4, 1),
    (3, 2),
    (4, 0),
    (2, 2),
    (3, 1),
    (3, 0),
    (1, 2),
    (2, 1),
    (2, 0),
];

/// Coefficient binomial `C(n, k)` en flottant (les combinaisons du jeu dépassent 10⁸ mais restent exactes en `f64`).
pub fn choose(n: u32, k: u32) -> f64 {
    if k > n {
        return 0.0;
    }
    // Produit (n − k + 1)…n / k! calculé terme à terme : chaque quotient partiel est un entier exact.
    (1..=k).fold(1.0, |acc, i| acc * f64::from(n - k + i) / f64::from(i))
}

/// Probabilité qu'une grille de Loto trouve exactement `good` numéros parmi les 5 tirés (loi hypergéométrique,
/// `C(5, good) · C(44, 5 − good) / C(49, 5)`).
pub fn loto_good_probability(good: u32) -> f64 {
    if good > 5 {
        return 0.0;
    }
    choose(5, good) * choose(44, 5 - good) / choose(49, 5)
}

/// Rang Loto (1 à 9) d'une grille qui trouve `good` numéros et, si `chance`, le numéro chance (art. 8.1.1.2).
pub fn loto_rank(good: usize, chance: bool) -> Option<usize> {
    match (good, chance) {
        (5, true) => Some(1),
        (5, false) => Some(2),
        (4, true) => Some(3),
        (4, false) => Some(4),
        (3, true) => Some(5),
        (3, false) => Some(6),
        (2, true) => Some(7),
        (2, false) => Some(8),
        (0 | 1, true) => Some(9),
        _ => None,
    }
}

/// Probabilité qu'une grille de Loto gagne au rang `rank` (1 à 9) : `P(numéros) · P(chance)`, avec `P(chance) = 1/10`
/// ou `9/10`. Le rang 9 réunit 0 et 1 numéro avec la chance. Vaut 0 hors de 1 à 9.
pub fn loto_rank_probability(rank: usize) -> f64 {
    let with_chance = 1.0 / f64::from(LOTO_CHANCES);
    let without = 1.0 - with_chance;
    match rank {
        1 => loto_good_probability(5) * with_chance,
        2 => loto_good_probability(5) * without,
        3 => loto_good_probability(4) * with_chance,
        4 => loto_good_probability(4) * without,
        5 => loto_good_probability(3) * with_chance,
        6 => loto_good_probability(3) * without,
        7 => loto_good_probability(2) * with_chance,
        8 => loto_good_probability(2) * without,
        9 => (loto_good_probability(1) + loto_good_probability(0)) * with_chance,
        _ => 0.0,
    }
}

/// Probabilité qu'une grille d'EuroMillions gagne au rang `rank` (1 à 13) :
/// `C(5, g)·C(45, 5 − g) · C(2, s)·C(10, 2 − s) / (C(50, 5)·C(12, 2))`. Vaut 0 hors de 1 à 13.
pub fn euro_rank_probability(rank: usize) -> f64 {
    let Some(&(good, stars)) = rank.checked_sub(1).and_then(|i| EURO_RANKS.get(i)) else {
        return 0.0;
    };
    let (good, stars) = (u32::from(good), u32::from(stars));
    choose(5, good) * choose(45, 5 - good) * choose(2, stars) * choose(10, 2 - stars)
        / (choose(50, 5) * choose(12, 2))
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    #[test]
    fn loto_rank_odds_match_the_rules_table() {
        // Art. 8.1.1.2 : « 1 chance de gagner sur », arrondi à l'entier.
        let odds = [
            19_068_840.0,
            2_118_760.0,
            86_677.0,
            9_631.0,
            2_016.0,
            224.0,
            144.0,
            16.0,
        ];
        for (i, expected) in odds.iter().enumerate() {
            let one_in = 1.0 / loto_rank_probability(i + 1);
            assert_relative_eq!(one_in.round(), *expected, max_relative = 1e-9);
        }
        // Rang 9 : 1 sur 28 (un numéro) et 1 sur 18 (aucun numéro), réunis.
        let total: f64 = (1..=9).map(loto_rank_probability).sum();
        assert_relative_eq!(1.0 / total, 5.99, epsilon = 0.005);
    }

    #[test]
    fn hypergeometric_probabilities_sum_to_one() {
        let total: f64 = (0..=5).map(loto_good_probability).sum();
        assert_relative_eq!(total, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn grand_loto_keys_sum_to_one() {
        assert_relative_eq!(
            GRAND_LOTO_LOWER_KEYS.iter().sum::<f64>(),
            1.0,
            epsilon = 1e-9
        );
        assert_relative_eq!(
            GRAND_LOTO_REDISTRIBUTION.iter().sum::<f64>(),
            1.0,
            epsilon = 1e-9
        );
    }

    #[test]
    fn euromillions_odds_match_the_known_values() {
        assert_relative_eq!(
            1.0 / euro_rank_probability(1),
            139_838_160.0,
            max_relative = 1e-9
        );
        assert_relative_eq!(
            1.0 / euro_rank_probability(2),
            6_991_908.0,
            max_relative = 1e-9
        );
        assert_eq!(euro_rank_probability(0), 0.0);
        assert_eq!(euro_rank_probability(14), 0.0);
        // Environ une grille sur 13 gagne quelque chose.
        let total: f64 = (1..=13).map(euro_rank_probability).sum();
        assert_relative_eq!(1.0 / total, 13.0, epsilon = 0.05);
    }

    #[test]
    fn euromillions_shares_leave_the_rest_to_the_jackpot_and_the_fund() {
        // Les rangs 2 à 13 reçoivent 40 % de la part des gagnants ; les 60 % restants vont au rang 1 et au Fonds.
        assert_relative_eq!(EURO_RANK_SHARES.iter().sum::<f64>(), 0.40, epsilon = 1e-9);
    }
}
