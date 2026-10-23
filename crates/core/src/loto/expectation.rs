//! Espérance d'une grille de Grand Loto avant le tirage, selon les clés du règlement (art. 8.1.1.3 et 8.3.4), le nombre
//! de grilles jouées et les préférences des autres joueurs.
//!
//! Une grille qui gagne au rang `k` partage la somme du rang avec les autres gagnants. Si `M` autres grilles gagnent
//! aussi, et que `M` suit une loi de Poisson de moyenne `μ`, la part moyenne est
//!
//! ```text
//! E[1 / (1 + M)] = (1 − e^{−μ}) / μ
//! ```
//!
//! `μ = N · P(une autre grille est au même rang | la nôtre y est)`. Au hasard, `μ = N · p_k` ; une grille de numéros
//! délaissés a un `μ` plus petit, donc une plus grosse part. Les sommes des rangs :
//!
//! - rangs 2 à 8 : une clé du reliquat `R = N · prix · (54,85 % − 5,5 % − 2,85 %) − 5 € · N · p_9` ;
//! - rang 9 : 5 € fixes (la mise rendue) ;
//! - jackpot `J` : au rang 1 s'il a un gagnant, sinon réparti sur les rangs 2 à 9 (51,05 % au rang 8, 24,92 % au rang 9…) ;
//! - codes : `C` euros tirés au sort parmi les `N` grilles, soit `C / N` par grille.
//!
//! Approximations (documentées dans `docs/sources.md` §44) : la probabilité qu'aucun autre joueur ne gagne le jackpot est
//! celle du hasard, `e^{−N p_1}` ; les reports d'un rang sans gagnant vers le rang inférieur sont ignorés (au plus 0,1 %
//! de l'espérance) ; la part réelle des mises reversée est « définie globalement » et varie d'une édition à l'autre
//! (2,07 à 2,56 € observés, contre 2,33 € par le règlement).

use super::popularity::{Preferences, elementary};
use super::{Grid, rules};
use crate::eval::bootstrap::SplitMix64;

/// Une édition du Grand Loto : ce que la FDJ annonce avant le tirage.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Edition {
    /// Jackpot garanti, en euros.
    pub jackpot: f64,
    /// Prix d'une grille, en euros.
    pub price: f64,
    /// Nombre de codes gagnants.
    pub codes: u32,
    /// Gain d'un code, en euros.
    pub code_gain: f64,
}

/// Part moyenne d'une somme partagée avec `M ~ Poisson(μ)` autres gagnants : `(1 − e^{−μ}) / μ` (1 si `μ` est nul).
fn share(mu: f64) -> f64 {
    if mu < 1e-12 {
        1.0
    } else {
        -(-mu).exp_m1() / mu
    }
}

/// Loi hypergéométrique : probabilité de `x` succès en `n` tirages sans remise dans une population de `total` éléments
/// dont `successes` succès, `C(K, x) · C(T − K, n − x) / C(T, n)`.
fn hypergeometric(x: u32, total: u32, successes: u32, n: u32) -> f64 {
    if x > successes || x > n || n - x > total - successes {
        return 0.0;
    }
    rules::choose(successes, x) * rules::choose(total - successes, n - x) / rules::choose(total, n)
}

/// `P(|S' ∩ D| = t)` pour une grille `S'` qui partage `j` numéros avec la nôtre, quand le tirage `D` partage `m` numéros
/// avec la nôtre (tirage au hasard sous cette condition). `D = T ∪ U`, `T` : `m` de nos 5 numéros, `U` : `5 − m` des 44
/// autres ; `S'` en a `j` parmi les nôtres et `5 − j` parmi les autres. Les deux intersections sont indépendantes :
///
/// ```text
/// h(t | m, j) = Σ_x Hyp(x ; 5, j, m) · Hyp(t − x ; 44, 5 − j, 5 − m)
/// ```
fn overlap(t: u32, m: u32, j: u32) -> f64 {
    (0..=t.min(5))
        .map(|x| hypergeometric(x, 5, j, m) * hypergeometric(t - x, 44, 5 - j, 5 - m))
        .sum()
}

/// Ce qu'une grille partage avec les grilles des autres joueurs.
struct Overlaps {
    /// `F_j` : part des autres grilles qui ont exactement `j` de nos numéros (`j = 0..5`).
    with_ours: [f64; 6],
    /// Part des autres grilles qui ont notre numéro chance.
    chance: f64,
}

impl Overlaps {
    /// `F_j = e_j(a_G) · e_{5−j}(a_{Ḡ}) / e_5(a)` (même raisonnement que `popularity`, appliqué à notre grille `G`).
    fn new(prefs: &Preferences, grid: &Grid) -> Overlaps {
        let ours: Vec<usize> = grid.balls.iter().map(|&b| usize::from(b) - 1).collect();
        let mine: Vec<f64> = ours.iter().map(|&i| prefs.balls[i]).collect();
        let rest: Vec<f64> = (0..prefs.balls.len())
            .filter(|i| !ours.contains(i))
            .map(|i| prefs.balls[i])
            .collect();
        let (e_mine, e_rest) = (elementary(&mine), elementary(&rest));
        let all = elementary(&prefs.balls)[5];
        let mut with_ours = [0.0; 6];
        for (j, slot) in with_ours.iter_mut().enumerate() {
            *slot = e_mine[j] * e_rest[5 - j] / all;
        }
        Overlaps {
            with_ours,
            chance: prefs.chance_share(grid.chance),
        }
    }

    /// Part des autres grilles qui ont exactement `t` numéros du tirage, quand le nôtre en a `m`.
    fn numbers(&self, t: u32, m: u32) -> f64 {
        (0..=5u32)
            .map(|j| self.with_ours[j as usize] * overlap(t, m, j))
            .sum()
    }

    /// Part des autres grilles au même état de chance que la nôtre. Avec la chance (`χ = 1`), c'est la part qui a notre
    /// chance ; sans (`χ = 0`), la chance tirée est l'une des 9 autres, au hasard : `1 − (1 − β_g) / 9`.
    fn same_chance(&self, with_chance: bool) -> f64 {
        if with_chance {
            self.chance
        } else {
            1.0 - (1.0 - self.chance) / 9.0
        }
    }
}

/// Euros rendus en moyenne par une grille `grid` de l'édition `edition`, si `grids` grilles sont jouées et que les
/// autres joueurs suivent `prefs`. Les gains des codes y sont compris.
pub fn grid_value(prefs: &Preferences, grid: &Grid, edition: &Edition, grids: f64) -> f64 {
    let overlaps = Overlaps::new(prefs, grid);
    let p = |k| rules::loto_rank_probability(k);
    let reliquat = grids
        * edition.price
        * (rules::LOTO_WINNERS_SHARE
            - rules::GRAND_LOTO_JACKPOT_SHARE
            - rules::GRAND_LOTO_SUPER_FUND_SHARE)
        - rules::GRAND_LOTO_RANK9_GAIN * grids * p(9);
    let no_other_jackpot = (-grids * p(1)).exp();
    let chance_probability = |with: bool| {
        let one = 1.0 / f64::from(rules::LOTO_CHANCES);
        if with { one } else { 1.0 - one }
    };

    // Rang 1 : le jackpot, partagé avec les grilles identiques à la nôtre.
    let mu_jackpot = grids * overlaps.numbers(5, 5) * overlaps.same_chance(true);
    let mut value = p(1) * edition.jackpot * share(mu_jackpot);

    // Rangs 2 à 8 : (m numéros, chance), dans l'ordre du règlement.
    let shapes = [
        (5, false),
        (4, true),
        (4, false),
        (3, true),
        (3, false),
        (2, true),
        (2, false),
    ];
    for (i, &(m, with)) in shapes.iter().enumerate() {
        let pool = rules::GRAND_LOTO_LOWER_KEYS[i] * reliquat
            + no_other_jackpot * rules::GRAND_LOTO_REDISTRIBUTION[i] * edition.jackpot;
        let mu = grids * overlaps.numbers(m, m) * overlaps.same_chance(with);
        value += rules::loto_good_probability(m) * chance_probability(with) * pool * share(mu);
    }

    // Rang 9 (0 ou 1 numéro, avec la chance) : 5 € fixes, plus sa part du jackpot non gagné.
    let rank9_pool = no_other_jackpot * rules::GRAND_LOTO_REDISTRIBUTION[7] * edition.jackpot;
    for m in 0..=1 {
        let others = overlaps.numbers(0, m) + overlaps.numbers(1, m);
        let mu = grids * others * overlaps.same_chance(true);
        value += rules::loto_good_probability(m)
            * chance_probability(true)
            * (rules::GRAND_LOTO_RANK9_GAIN + rank9_pool * share(mu));
    }

    value + f64::from(edition.codes) * edition.code_gain / grids
}

/// Forme fermée de l'espérance d'une grille au hasard : `(J + C) / N + prix · (54,85 % − 5,5 % − 2,85 %)`. Tout le
/// jackpot est versé, et les rangs 2 à 9 reçoivent leur part des mises. Sert de contrôle à [`grid_value`].
pub fn closed_form_value(edition: &Edition, grids: f64) -> f64 {
    let codes = f64::from(edition.codes) * edition.code_gain;
    (edition.jackpot + codes) / grids
        + edition.price
            * (rules::LOTO_WINNERS_SHARE
                - rules::GRAND_LOTO_JACKPOT_SHARE
                - rules::GRAND_LOTO_SUPER_FUND_SHARE)
}

/// Numéros parmi lesquels on cherche les grilles : les plus délaissés (15 numéros, soit 3 003 combinaisons).
const CANDIDATE_BALLS: usize = 15;
/// Numéros chance essayés : les plus délaissés.
const CANDIDATE_CHANCES: usize = 3;

/// Grille à motif (au moins trois numéros consécutifs, ou cinq numéros à écart constant). Le modèle par numéro ne voit
/// pas ces motifs, que le public joue beaucoup (Baker & McHale, 2011) : on les écarte par prudence.
fn has_pattern(grid: &Grid) -> bool {
    let b = grid.balls;
    let consecutive = b.windows(3).any(|w| w[1] == w[0] + 1 && w[2] == w[1] + 1);
    let step = b[1] - b[0];
    let constant = b.windows(2).all(|w| w[1] - w[0] == step);
    consecutive || constant
}

/// Nombre maximal de numéros communs à deux grilles recommandées : à espérance presque égale, des grilles qui couvrent
/// plus de numéros gagnent moins souvent ensemble, ce qui réduit la variance du total.
const MAX_SHARED_BALLS: usize = 2;

/// Les `count` grilles de meilleure espérance pour `grids` grilles jouées, parmi les combinaisons des numéros les plus
/// délaissés, motifs écartés. Choix glouton : la meilleure grille, puis la meilleure qui partage au plus
/// [`MAX_SHARED_BALLS`] numéros avec chacune des précédentes, etc. Ordre : meilleure espérance d'abord ; à égalité, ordre
/// des grilles (déterministe).
pub fn best_grids(
    prefs: &Preferences,
    edition: &Edition,
    grids: f64,
    count: usize,
) -> Vec<(Grid, f64)> {
    let mut balls: Vec<u8> = (1..=rules::LOTO_BALLS).collect();
    balls.sort_by(|a, b| {
        prefs.balls[usize::from(*a) - 1].total_cmp(&prefs.balls[usize::from(*b) - 1])
    });
    let pool: Vec<u8> = balls.into_iter().take(CANDIDATE_BALLS).collect();
    let mut chances: Vec<u8> = (1..=rules::LOTO_CHANCES).collect();
    chances.sort_by(|a, b| prefs.chance_share(*a).total_cmp(&prefs.chance_share(*b)));
    chances.truncate(CANDIDATE_CHANCES);

    let mut scored = Vec::new();
    for combination in combinations(&pool, 5) {
        let Ok(chosen) = <[u8; 5]>::try_from(combination) else {
            continue;
        };
        for &chance in &chances {
            let Some(grid) = Grid::new(chosen, chance) else {
                continue;
            };
            if has_pattern(&grid) {
                continue;
            }
            scored.push((grid, grid_value(prefs, &grid, edition, grids)));
        }
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut chosen: Vec<(Grid, f64)> = Vec::new();
    for (grid, value) in scored {
        if chosen.len() == count {
            break;
        }
        let shared = |other: &Grid| {
            grid.balls
                .iter()
                .filter(|b| other.balls.contains(b))
                .count()
        };
        if chosen
            .iter()
            .all(|(other, _)| shared(other) <= MAX_SHARED_BALLS)
        {
            chosen.push((grid, value));
        }
    }
    chosen
}

/// Toutes les combinaisons de `k` éléments de `items`, dans l'ordre lexicographique des positions.
fn combinations(items: &[u8], k: usize) -> Vec<Vec<u8>> {
    if k == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for (i, &first) in items.iter().enumerate() {
        for mut rest in combinations(&items[i + 1..], k - 1) {
            rest.insert(0, first);
            out.push(rest);
        }
    }
    out
}

/// `count` grilles tirées au hasard (comme le « Flash »), reproductibles : même graine, mêmes grilles (invariant 9).
pub fn random_grids(seed: u64, count: usize) -> Vec<Grid> {
    let mut rng = SplitMix64::new(seed);
    let mut grids: Vec<Grid> = Vec::new();
    while grids.len() < count {
        let mut balls = Vec::new();
        while balls.len() < 5 {
            let b = 1 + ((rng.next_f64() * 49.0) as u8).min(48);
            if !balls.contains(&b) {
                balls.push(b);
            }
        }
        let chance = 1 + ((rng.next_f64() * 10.0) as u8).min(9);
        let Ok(chosen) = <[u8; 5]>::try_from(balls) else {
            continue;
        };
        if let Some(grid) = Grid::new(chosen, chance)
            && !grids.contains(&grid)
        {
            grids.push(grid);
        }
    }
    grids
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;
    use crate::loto::popularity::tests::planted;

    fn christmas() -> Edition {
        Edition {
            jackpot: 20_000_000.0,
            price: 5.0,
            codes: 100,
            code_gain: 20_000.0,
        }
    }

    #[test]
    fn hypergeometric_and_overlap_are_probability_laws() {
        let total: f64 = (0..=5).map(|x| hypergeometric(x, 49, 5, 5)).sum();
        assert_relative_eq!(total, 1.0, epsilon = 1e-12);
        for m in 0..=5 {
            for j in 0..=5 {
                let total: f64 = (0..=5).map(|t| overlap(t, m, j)).sum();
                assert_relative_eq!(total, 1.0, epsilon = 1e-12);
            }
        }
        // Une grille identique à la nôtre (j = 5) a autant de numéros du tirage que nous.
        assert_relative_eq!(overlap(3, 3, 5), 1.0, epsilon = 1e-12);
    }

    #[test]
    fn a_random_grid_matches_the_closed_form() {
        let grid = Grid::parse("4-17-23-38-45+3").unwrap();
        for grids in [3e6, 7e6, 9e6] {
            let value = grid_value(&Preferences::uniform(), &grid, &christmas(), grids);
            assert_relative_eq!(
                value,
                closed_form_value(&christmas(), grids),
                max_relative = 0.005
            );
        }
        // §43 : à 6,84 millions de grilles, environ +11 % par le règlement (+16 % réalisés : r plus haut ce soir-là).
        assert_relative_eq!(
            closed_form_value(&christmas(), 6.84e6) / 5.0,
            1.108,
            epsilon = 0.002
        );
    }

    #[test]
    fn an_unpopular_grid_is_worth_more_than_a_popular_one() {
        let prefs = planted();
        let popular = Grid::parse("7-13-21-25-29+7").unwrap();
        let unpopular = Grid::parse("33-38-41-44-47+2").unwrap();
        let value = |g: &Grid| grid_value(&prefs, g, &christmas(), 7e6);
        assert!(value(&unpopular) > value(&popular));
        assert!(prefs.grid_popularity(&unpopular) < 1.0 && prefs.grid_popularity(&popular) > 1.0);
    }

    #[test]
    fn the_best_grids_avoid_popular_numbers_and_patterns() {
        let prefs = planted();
        let best = best_grids(&prefs, &christmas(), 7e6, 4);
        assert_eq!(best.len(), 4);
        for (grid, value) in &best {
            assert!(grid.balls.iter().all(|&b| b > 31), "{grid}");
            assert!(!has_pattern(grid));
            assert!(*value > closed_form_value(&christmas(), 7e6));
        }
        assert!(best.windows(2).all(|w| w[0].1 >= w[1].1));
        for (i, (a, _)) in best.iter().enumerate() {
            for (b, _) in &best[..i] {
                let shared = a.balls.iter().filter(|x| b.balls.contains(x)).count();
                assert!(shared <= MAX_SHARED_BALLS, "{a} et {b}");
            }
        }
    }

    #[test]
    fn patterns_are_recognised() {
        assert!(has_pattern(&Grid::parse("1-2-3-20-40+1").unwrap()));
        assert!(has_pattern(&Grid::parse("5-10-15-20-25+1").unwrap()));
        assert!(!has_pattern(&Grid::parse("5-10-15-20-26+1").unwrap()));
    }

    #[test]
    fn random_grids_are_reproducible_and_distinct() {
        let a = random_grids(20261225, 4);
        assert_eq!(a, random_grids(20261225, 4));
        assert_ne!(a, random_grids(20261226, 4));
        assert_eq!(a.len(), 4);
        assert!(a.iter().enumerate().all(|(i, g)| !a[..i].contains(g)));
    }
}
