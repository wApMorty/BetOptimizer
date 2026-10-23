//! Nombre de grilles jouées à un tirage, espérance réalisée d'un Grand Loto passé, et prévision du nombre de grilles
//! d'une édition à venir.
//!
//! La FDJ ne publie pas le nombre de grilles, mais elle publie le nombre de gagnants de chaque rang. Si chaque grille
//! gagne au rang `k` avec la probabilité `p_k`, le nombre de gagnants suit une loi de Poisson de moyenne `N · p_k`, et
//! l'estimateur du maximum de vraisemblance de `N` sur plusieurs rangs est
//!
//! ```text
//! N̂ = Σ_k n_k / Σ_k p_k
//! ```
//!
//! On le calcule sur les rangs fréquents (5 à 9 au Loto, 9 à 13 à l'EuroMillions), qui comptent des dizaines de milliers
//! de gagnants. Biais connu : un tirage de numéros populaires a plus de gagnants que prévu, donc un `N̂` trop haut
//! (l'écart entre les estimations par un seul rang le montre). `docs/sources.md` §43.

use chrono::{Datelike, NaiveDate};
use statrs::distribution::{ContinuousCDF, Normal, StudentsT};

use super::{Draw, Game, rules};
use crate::error::{Error, Result};

/// Estimation du nombre de grilles d'un tirage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grids {
    /// Estimation sur tous les rangs fréquents.
    pub n: f64,
    /// Plus petite estimation par un seul rang (rangs 8 et 9 au Loto, 12 et 13 à l'EuroMillions).
    pub low: f64,
    /// Plus grande estimation par un seul rang.
    pub high: f64,
}

/// Rangs utilisés pour estimer `N` : les plus fréquents.
fn frequent_ranks(game: Game) -> std::ops::RangeInclusive<usize> {
    match game {
        Game::Loto | Game::GrandLoto => 5..=9,
        Game::EuroMillions => 9..=13,
    }
}

/// Probabilité de gagner au rang `rank` du jeu `game`.
pub fn rank_probability(game: Game, rank: usize) -> f64 {
    match game {
        Game::Loto | Game::GrandLoto => rules::loto_rank_probability(rank),
        Game::EuroMillions => rules::euro_rank_probability(rank),
    }
}

/// Nombre de grilles du tirage `draw`. `None` si les rangs fréquents n'ont aucun gagnant (archive incomplète).
pub fn grids(draw: &Draw) -> Option<Grids> {
    let ranks = frequent_ranks(draw.game);
    let winners = |k: usize| draw.ranks.get(k - 1).map(|r| r.winners as f64);
    let mut count = 0.0;
    let mut probability = 0.0;
    for k in ranks.clone() {
        count += winners(k)?;
        probability += rank_probability(draw.game, k);
    }
    if count <= 0.0 {
        return None;
    }
    let (a, b) = (*ranks.end() - 1, *ranks.end());
    let single = |k: usize| winners(k).map(|n| n / rank_probability(draw.game, k));
    let (first, second) = (single(a)?, single(b)?);
    Some(Grids {
        n: count / probability,
        low: first.min(second),
        high: first.max(second),
    })
}

/// Bilan d'un Grand Loto passé.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditionOutcome {
    /// Date du tirage.
    pub date: NaiveDate,
    /// Jackpot versé (au rang 1, ou réparti sur les rangs inférieurs), en euros.
    pub jackpot: f64,
    /// Montant des codes gagnants, en euros.
    pub codes: f64,
    /// Nombre de grilles estimé.
    pub grids: Grids,
    /// Total versé (rangs et codes), en euros.
    pub paid: f64,
    /// `true` si le jackpot a été gagné au rang 1.
    pub jackpot_won: bool,
}

impl EditionOutcome {
    /// Espérance réalisée d'une grille, en euros : le jackpot étant toujours versé, c'est `versé / N`.
    pub fn return_per_grid(&self) -> f64 {
        self.paid / self.grids.n
    }

    /// Fourchette de l'espérance selon l'estimation de `N` par un seul rang : `(basse, haute)`.
    pub fn return_range(&self) -> (f64, f64) {
        (self.paid / self.grids.high, self.paid / self.grids.low)
    }

    /// Reversement des rangs inférieurs hors jackpot et codes, par grille : `r = (versé − J − C) / N`.
    pub fn lower_ranks_per_grid(&self) -> f64 {
        (self.paid - self.jackpot - self.codes) / self.grids.n
    }

    /// Nombre de grilles au-delà duquel une grille de `price` euros perd en moyenne : `N* = (J + C) / (prix − r)`.
    pub fn break_even_grids(&self, price: f64) -> f64 {
        (self.jackpot + self.codes) / (price - self.lower_ranks_per_grid())
    }
}

/// Bilan d'un tirage Grand Loto de l'historique. `None` si ses rangs fréquents sont vides.
pub fn edition_outcome(draw: &Draw) -> Option<EditionOutcome> {
    let grids = grids(draw)?;
    let first = draw.ranks.first()?;
    // Sans gagnant, la colonne du rang 1 donne le jackpot annoncé, réparti sur les rangs inférieurs (art. 8.3.4).
    let jackpot_won = first.winners > 0;
    let jackpot = if jackpot_won {
        first.winners as f64 * first.unit_gain
    } else {
        first.unit_gain
    };
    Some(EditionOutcome {
        date: draw.date,
        jackpot,
        codes: draw.paid_to_codes(),
        grids,
        paid: draw.paid_to_ranks() + draw.paid_to_codes(),
        jackpot_won,
    })
}

/// Saison d'une édition : le public n'est pas le même l'été et à Noël (`docs/sources.md` §43 : deux à trois fois moins de
/// grilles l'été).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Season {
    /// Juin à août.
    Summer,
    /// Décembre.
    Christmas,
    /// Autre mois.
    Other,
}

impl Season {
    /// Saison d'une date.
    pub fn of(date: NaiveDate) -> Season {
        match date.month() {
            6..=8 => Season::Summer,
            12 => Season::Christmas,
            _ => Season::Other,
        }
    }

    /// Nom en français.
    pub fn label(self) -> &'static str {
        match self {
            Season::Summer => "été",
            Season::Christmas => "Noël",
            Season::Other => "autre",
        }
    }
}

/// Nombre d'éditions passées de la même saison prises en compte (les plus récentes).
pub const FORECAST_EDITIONS: usize = 5;

/// Écart type minimal du logarithme du nombre de grilles. Les éditions de Noël 2017 à 2025 donnent 0,11 : avec
/// deux ou trois éditions, l'écart type observé peut être bien plus petit que le vrai par hasard.
pub const MIN_LOG_SD: f64 = 0.15;

/// Écart type supposé quand une seule édition est connue (aucune mesure possible) : 0,30, le double du plancher.
pub const SINGLE_EDITION_LOG_SD: f64 = 0.30;

/// Prévision du nombre de grilles d'une édition.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Forecast {
    /// Saison de l'édition.
    pub season: Season,
    /// Éditions utilisées (dates).
    pub editions: Vec<NaiveDate>,
    /// Nombre de grilles central (médiane de la loi prédictive).
    pub central: f64,
    /// Nombre de grilles pessimiste (quantile `quantile` : plus de grilles, moins d'espérance).
    pub pessimistic: f64,
    /// Quantile retenu.
    pub quantile: f64,
}

/// Prévoit le nombre de grilles de l'édition du `date` à partir des éditions **antérieures** (aucune fuite du futur) de la
/// même saison, ou de toutes les éditions antérieures s'il n'y en a aucune.
///
/// Sur `x = ln N` des `n` éditions retenues, de moyenne `m` et d'écart type `s` (au moins [`MIN_LOG_SD`]), la loi
/// prédictive d'une nouvelle valeur est une loi de Student à `n − 1` degrés de liberté, centrée sur `m`, d'échelle
/// `s · √(1 + 1/n)` (intervalle de prédiction d'une loi normale à variance inconnue). Le nombre pessimiste est
/// `exp(m + t_q · s · √(1 + 1/n))`. Avec une seule édition, `s` vaut [`SINGLE_EDITION_LOG_SD`] et `t` devient une loi normale.
pub fn forecast(editions: &[EditionOutcome], date: NaiveDate, quantile: f64) -> Result<Forecast> {
    if !(0.5..1.0).contains(&quantile) {
        return Err(Error::InvalidConfig(
            "le quantile pessimiste doit être dans [0,5 ; 1[".to_string(),
        ));
    }
    let season = Season::of(date);
    let mut past: Vec<&EditionOutcome> = editions.iter().filter(|e| e.date < date).collect();
    past.sort_by_key(|e| e.date);
    let same: Vec<&EditionOutcome> = past
        .iter()
        .copied()
        .filter(|e| Season::of(e.date) == season)
        .collect();
    let chosen = if same.is_empty() { past } else { same };
    // Les plus récentes : `skip` saute les plus anciennes au-delà de FORECAST_EDITIONS.
    let chosen: Vec<&EditionOutcome> = chosen
        .iter()
        .copied()
        .skip(chosen.len().saturating_sub(FORECAST_EDITIONS))
        .collect();
    if chosen.is_empty() {
        return Err(Error::Data(
            "aucune édition passée du Grand Loto en base : lance d'abord `edge loto fetch`"
                .to_string(),
        ));
    }
    let logs: Vec<f64> = chosen.iter().map(|e| e.grids.n.ln()).collect();
    let n = logs.len() as f64;
    let mean = logs.iter().sum::<f64>() / n;
    let upper = if logs.len() == 1 {
        let normal = Normal::new(0.0, 1.0).map_err(|e| Error::Data(e.to_string()))?;
        normal.inverse_cdf(quantile) * SINGLE_EDITION_LOG_SD * 2f64.sqrt()
    } else {
        let variance = logs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let sd = variance.sqrt().max(MIN_LOG_SD);
        let student = StudentsT::new(0.0, 1.0, n - 1.0).map_err(|e| Error::Data(e.to_string()))?;
        student.inverse_cdf(quantile) * sd * (1.0 + 1.0 / n).sqrt()
    };
    Ok(Forecast {
        season,
        editions: chosen.iter().map(|e| e.date).collect(),
        central: mean.exp(),
        pessimistic: (mean + upper).exp(),
        quantile,
    })
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;
    use crate::loto::RankResult;
    use crate::loto::fdj::{self, tests::GRAND_LOTO_CSV};

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Tirage dont chaque rang a exactement `N · p_k` gagnants.
    fn exact_draw(game: Game, n: f64) -> Draw {
        Draw {
            game,
            date: day(2026, 1, 1),
            balls: [1, 2, 3, 4, 5],
            extra: vec![1],
            ranks: (1..=game.rank_count())
                .map(|k| RankResult {
                    winners: (n * rank_probability(game, k)).round() as u64,
                    unit_gain: 1.0,
                })
                .collect(),
            codes: None,
        }
    }

    #[test]
    fn the_estimator_recovers_the_number_of_grids() {
        for game in [Game::Loto, Game::EuroMillions] {
            let grids = grids(&exact_draw(game, 7_000_000.0)).unwrap();
            assert_relative_eq!(grids.n, 7_000_000.0, max_relative = 1e-3);
            assert!(grids.low <= grids.n * 1.001 && grids.high >= grids.n * 0.999);
        }
    }

    #[test]
    fn christmas_2025_reproduces_the_study() {
        let draw = &fdj::parse_csv(GRAND_LOTO_CSV, Game::GrandLoto).unwrap()[0];
        let outcome = edition_outcome(draw).unwrap();
        assert!(!outcome.jackpot_won);
        assert_eq!(outcome.jackpot, 20_000_000.0);
        assert_eq!(outcome.codes, 2_000_000.0);
        // §43 : 6,84 millions de grilles, +16 %, r = 2,56 €, N* = 9,01 millions.
        assert_relative_eq!(outcome.grids.n / 1e6, 6.84, epsilon = 0.01);
        assert_relative_eq!(outcome.return_per_grid() / 5.0 - 1.0, 0.16, epsilon = 0.01);
        assert_relative_eq!(outcome.lower_ranks_per_grid(), 2.56, epsilon = 0.01);
        assert_relative_eq!(outcome.break_even_grids(5.0) / 1e6, 9.01, epsilon = 0.01);
    }

    fn edition(date: NaiveDate, millions: f64) -> EditionOutcome {
        EditionOutcome {
            date,
            jackpot: 20e6,
            codes: 2e6,
            grids: Grids {
                n: millions * 1e6,
                low: millions * 1e6,
                high: millions * 1e6,
            },
            paid: 0.0,
            jackpot_won: false,
        }
    }

    #[test]
    fn the_forecast_uses_past_editions_of_the_same_season_only() {
        let editions = [
            edition(day(2024, 7, 26), 2.86),
            edition(day(2024, 12, 24), 8.40),
            edition(day(2025, 7, 4), 4.94),
            edition(day(2025, 12, 25), 6.84),
            edition(day(2026, 12, 25), 100.0), // l'édition prévue elle-même : jamais utilisée
        ];
        let christmas = forecast(&editions, day(2026, 12, 25), 0.8).unwrap();
        assert_eq!(christmas.season, Season::Christmas);
        assert_eq!(
            christmas.editions,
            vec![day(2024, 12, 24), day(2025, 12, 25)]
        );
        assert_relative_eq!(
            christmas.central / 1e6,
            (8.40f64 * 6.84).sqrt(),
            epsilon = 1e-9
        );
        assert!(christmas.pessimistic > christmas.central);

        let summer = forecast(&editions, day(2026, 6, 26), 0.8).unwrap();
        assert_eq!(summer.editions, vec![day(2024, 7, 26), day(2025, 7, 4)]);

        let only_one = forecast(&editions[..1], day(2025, 7, 1), 0.8).unwrap();
        // Une édition : N pessimiste = N · exp(z_0,8 · 0,30 · √2).
        assert_relative_eq!(
            only_one.pessimistic / only_one.central,
            (0.8416 * 0.30 * 2f64.sqrt()).exp(),
            epsilon = 1e-3
        );

        assert!(
            forecast(&editions, day(2017, 1, 1), 0.8).is_err(),
            "aucune édition antérieure"
        );
        assert!(forecast(&editions, day(2026, 12, 25), 1.0).is_err());
    }

    #[test]
    fn a_higher_quantile_is_more_pessimistic() {
        let editions = [
            edition(day(2024, 12, 24), 8.40),
            edition(day(2025, 12, 25), 6.84),
        ];
        let at = |q| {
            forecast(&editions, day(2026, 12, 25), q)
                .unwrap()
                .pessimistic
        };
        assert!(at(0.9) > at(0.8) && at(0.8) > at(0.6));
    }
}
