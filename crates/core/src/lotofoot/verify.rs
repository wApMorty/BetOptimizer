//! Vérification du modèle en réel, selon un critère **figé avant le premier ticket** (2026-10-22, `docs/sources.md` §38).
//!
//! Trois mesures, sur les tickets recommandés et les grilles relevées :
//! 1. **calibration des concurrents** : gagnants publiés contre gagnants prédits (`μ`), à l'issue réelle, avec la
//!    répartition finale de chaque grille, par tranche de `λ` ;
//! 2. **fréquence** : tickets recommandés qui ont atteint un rang payé, contre la somme de leurs probabilités annoncées
//!    (test de Poisson unilatéral) ;
//! 3. **gain** : gain réalisé des tickets recommandés contre la loi de ce gain simulée par le modèle (issues tirées selon
//!    le marché enregistré le jour de la séance).
//!
//! Verdict aux points de contrôle de 26 et 52 semaines après la première séance : le modèle est **en défaut** si une
//! tranche `λ ≥ 10` d'au moins 10 observations sort de [0,8 ; 1,25], ou si le gain réalisé tombe sous le 5e centile de
//! sa loi simulée. Le ROI est affiché mais ne décide de rien : il est trop bruité (`docs/sources.md` §37).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use statrs::distribution::{DiscreteCDF, Poisson};

use super::model::{FROZEN, GridModel, public_split};
use super::{Game, Ticket};
use crate::domain::{Euros, Outcome};
use crate::error::Result;
use crate::store::lotofoot as store;

/// Points de contrôle, en semaines après la première séance.
pub const CHECKPOINTS_WEEKS: [i64; 2] = [26, 52];
/// Bornes acceptables du rapport gagnants publiés / prédits.
pub const CALIBRATION_BOUNDS: (f64, f64) = (0.8, 1.25);
/// Tranches de `λ` jugées (en dessous, trop peu de gagnants par grille pour juger un rapport).
pub const CALIBRATION_MIN_LAMBDA: f64 = 10.0;
/// Observations minimales d'une tranche jugée.
pub const CALIBRATION_MIN_OBS: usize = 10;
/// Centile du gain simulé sous lequel le modèle est en défaut.
pub const GAIN_PERCENTILE_FLOOR: f64 = 0.05;
/// Tirages de la simulation du gain.
pub const SIMULATIONS: usize = 20_000;
/// Graine de la simulation (reproductible).
pub const SEED: u64 = 20_261_022;

/// Une tranche de `λ` : gagnants publiés contre prédits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// Borne basse de `λ`.
    pub low: f64,
    /// Borne haute de `λ` (exclue).
    pub high: f64,
    /// Observations (un rang d'une grille).
    pub observations: usize,
    /// Gagnants publiés.
    pub observed: f64,
    /// Gagnants prédits (`μ`).
    pub predicted: f64,
}

impl Band {
    /// Rapport publiés / prédits.
    pub fn ratio(&self) -> Option<f64> {
        (self.predicted > 0.0).then(|| self.observed / self.predicted)
    }

    /// `true` si la tranche est jugée et sort des bornes.
    pub fn failing(&self) -> bool {
        self.low >= CALIBRATION_MIN_LAMBDA
            && self.observations >= CALIBRATION_MIN_OBS
            && self
                .ratio()
                .is_some_and(|r| r < CALIBRATION_BOUNDS.0 || r > CALIBRATION_BOUNDS.1)
    }
}

const BAND_EDGES: [f64; 7] = [0.0, 1.0, 10.0, 100.0, 1_000.0, 10_000.0, f64::INFINITY];

/// Calibration des concurrents sur toutes les grilles réglées dont la répartition finale a été relevée.
pub fn calibration(conn: &Connection) -> Result<Vec<Band>> {
    let mut bands: Vec<Band> = BAND_EDGES
        .windows(2)
        .map(|w| Band {
            low: w[0],
            high: w[1],
            observations: 0,
            observed: 0.0,
            predicted: 0.0,
        })
        .collect();
    for (game, number) in store::settled_grids(conn)? {
        let (Some(results), Some(split)) = (
            store::results(conn, game, number)?,
            store::final_split(conn, game, number)?,
        ) else {
            continue;
        };
        let ranks = store::ranks(conn, game, number)?;
        let Some(stakes) = actual_stakes(game, &ranks) else {
            continue;
        };
        let Some(q) = public_split(&split) else {
            continue;
        };
        // Le marché ne sert pas ici : seule la répartition du public entre dans λ. `q` tient lieu de `p`.
        let masses = GridModel::masses_for(game, stakes, None);
        let Some(model) = GridModel::new(&q, &q, stakes, &masses) else {
            continue;
        };
        for rank in &ranks {
            let errors = usize::try_from(rank.rank)
                .unwrap_or(usize::MAX)
                .saturating_sub(1);
            let lambda = model.public_winners(&results, errors);
            let predicted = FROZEN.expected(lambda, stakes.get(), game.matches());
            if let Some(band) = bands
                .iter_mut()
                .find(|b| lambda >= b.low && lambda < b.high)
            {
                band.observations += 1;
                band.observed += f64::from(rank.winners);
                band.predicted += predicted;
            }
        }
    }
    Ok(bands)
}

/// Mises réelles d'une grille réglée : masse du rang 2 publiée divisée par sa part des mises.
fn actual_stakes(game: Game, ranks: &[store::PublishedRank]) -> Option<Euros> {
    let second = ranks.iter().find(|r| r.rank == 2)?;
    let mass = f64::from(second.winners) * second.unit_gain?.get();
    Euros::new(mass / (game.rank_shares()[1] * super::PAYOUT_RATE))
}

/// Fréquence des tickets payés.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frequency {
    /// Tickets recommandés réglés.
    pub settled: usize,
    /// Tickets payés observés.
    pub observed: usize,
    /// Tickets payés attendus (somme des probabilités annoncées).
    pub expected: f64,
    /// Probabilité d'en observer autant ou moins si le modèle est juste (Poisson).
    pub p_low: f64,
}

/// Gain réalisé contre sa loi simulée.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainCheck {
    /// Tickets réglés pris en compte.
    pub settled: usize,
    /// Gain réalisé (règlement « papier » des tickets recommandés).
    pub realized: f64,
    /// Moyenne du gain simulé.
    pub simulated_mean: f64,
    /// Part des simulations dont le gain est inférieur ou égal au réalisé.
    pub percentile: f64,
}

/// Verdict à la date du bilan.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Aucune séance enregistrée.
    NoSession,
    /// Avant le premier point de contrôle.
    Pending {
        /// Semaines écoulées depuis la première séance.
        weeks: i64,
        /// Prochain point de contrôle.
        next: i64,
    },
    /// Au dernier point de contrôle passé, le modèle tient.
    Consistent {
        /// Point de contrôle (semaines).
        checkpoint: i64,
    },
    /// Le modèle est en défaut.
    Failed {
        /// Point de contrôle (semaines).
        checkpoint: i64,
        /// Raison lisible.
        reason: String,
    },
}

/// Bilan complet.
#[derive(Debug, Clone, PartialEq)]
pub struct Verification {
    /// Première séance.
    pub first_session: Option<DateTime<Utc>>,
    /// Calibration des concurrents.
    pub bands: Vec<Band>,
    /// Fréquence des tickets payés.
    pub frequency: Frequency,
    /// Gain réalisé contre simulé.
    pub gain: GainCheck,
    /// Verdict.
    pub verdict: Verdict,
}

/// Établit le bilan à l'instant `now`.
pub fn verify(conn: &Connection, now: DateTime<Utc>) -> Result<Verification> {
    let bands = calibration(conn)?;
    let recos = store::current_recos(conn)?;
    let first_session = recos.iter().map(|r| r.created_at).min();

    // Tickets réglés, regroupés par grille (la simulation tire une seule issue par grille et par tirage).
    let mut by_grid: BTreeMap<(Game, u32), (GridModel, Vec<Ticket>)> = BTreeMap::new();
    let mut frequency = Frequency {
        settled: 0,
        observed: 0,
        expected: 0.0,
        p_low: 1.0,
    };
    let mut realized = 0.0;
    for reco in &recos {
        let Some(results) = store::results(conn, reco.game, reco.number)? else {
            continue;
        };
        let ranks = store::ranks(conn, reco.game, reco.number)?;
        let jackpot = store::grid(conn, reco.game, reco.number)?.and_then(|g| g.jackpot);
        let Some(settled) = super::settle::paper(
            reco.game,
            &reco.ticket,
            &results,
            &ranks,
            reco.stakes_estimate,
            jackpot,
        ) else {
            continue;
        };
        let Some(model) = GridModel::from_journal(reco.game, &reco.inputs) else {
            continue;
        };
        frequency.settled += 1;
        frequency.observed += usize::from(settled.rank.is_some());
        frequency.expected += reco.paid_probability;
        realized += settled.gain;
        by_grid
            .entry((reco.game, reco.number))
            .or_insert_with(|| (model, Vec::new()))
            .1
            .push(reco.ticket.clone());
    }
    if frequency.expected > 0.0
        && let Ok(poisson) = Poisson::new(frequency.expected)
    {
        frequency.p_low = poisson.cdf(u64::try_from(frequency.observed).unwrap_or(u64::MAX));
    }
    let gain = simulate(&by_grid, realized, frequency.settled);

    let verdict = match first_session {
        None => Verdict::NoSession,
        Some(first) => {
            let weeks = (now - first).num_weeks();
            match CHECKPOINTS_WEEKS.iter().rev().find(|c| weeks >= **c) {
                None => Verdict::Pending {
                    weeks,
                    next: CHECKPOINTS_WEEKS[0],
                },
                Some(&checkpoint) => {
                    if let Some(band) = bands.iter().find(|b| b.failing()) {
                        Verdict::Failed {
                            checkpoint,
                            reason: format!(
                                "gagnants publiés / prédits = {:.2} pour λ entre {} et {}",
                                band.ratio().unwrap_or(f64::NAN),
                                band.low,
                                band.high
                            ),
                        }
                    } else if gain.percentile < GAIN_PERCENTILE_FLOOR {
                        Verdict::Failed {
                            checkpoint,
                            reason: format!(
                                "gain réalisé au centile {:.3} de sa loi simulée",
                                gain.percentile
                            ),
                        }
                    } else {
                        Verdict::Consistent { checkpoint }
                    }
                }
            }
        }
    };
    Ok(Verification {
        first_session,
        bands,
        frequency,
        gain,
        verdict,
    })
}

/// Loi du gain total des tickets réglés : pour chaque tirage, une issue par grille tirée selon le marché, et le gain
/// espéré de chaque ticket sachant cette issue.
fn simulate(
    by_grid: &BTreeMap<(Game, u32), (GridModel, Vec<Ticket>)>,
    realized: f64,
    settled: usize,
) -> GainCheck {
    let mut rng = SplitMix64(SEED);
    let mut below = 0usize;
    let mut total = 0.0;
    for _ in 0..SIMULATIONS {
        let mut gain = 0.0;
        for (model, tickets) in by_grid.values() {
            let outcome: Vec<Outcome> = (0..model.len())
                .map(|i| draw(model.market(i).unwrap_or([1.0, 0.0, 0.0]), rng.next_f64()))
                .collect();
            gain += tickets
                .iter()
                .map(|t| model.outcome_gain(t, &outcome, &FROZEN))
                .sum::<f64>();
        }
        total += gain;
        below += usize::from(gain <= realized);
    }
    let n = SIMULATIONS as f64;
    GainCheck {
        settled,
        realized,
        simulated_mean: total / n,
        percentile: if by_grid.is_empty() {
            1.0
        } else {
            below as f64 / n
        },
    }
}

/// Issue tirée selon les probabilités `p` à partir d'un nombre uniforme `u` de [0, 1[.
fn draw(p: [f64; 3], u: f64) -> Outcome {
    if u < p[0] {
        Outcome::Home
    } else if u < p[0] + p[1] {
        Outcome::Draw
    } else {
        Outcome::Away
    }
}

/// Générateur pseudo-aléatoire SplitMix64 (Steele, Lea & Flood, « Fast splittable pseudorandom number generators »,
/// OOPSLA 2014) : une addition et deux multiplications par tirage, assez bon pour une simulation, et reproductible à
/// partir de la graine. Écrit ici pour ne pas ajouter de dépendance.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Nombre uniforme de [0, 1[ : les 53 bits de poids fort, la précision d'un `f64`.
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_reproducible_and_uniform_enough() {
        let mut a = SplitMix64(1);
        let mut b = SplitMix64(1);
        assert_eq!(a.next_u64(), b.next_u64());
        let mut rng = SplitMix64(SEED);
        let mean: f64 = (0..100_000).map(|_| rng.next_f64()).sum::<f64>() / 100_000.0;
        assert!((mean - 0.5).abs() < 0.005, "moyenne {mean}");
    }

    #[test]
    fn draws_follow_the_probabilities() {
        assert_eq!(draw([0.5, 0.3, 0.2], 0.49), Outcome::Home);
        assert_eq!(draw([0.5, 0.3, 0.2], 0.79), Outcome::Draw);
        assert_eq!(draw([0.5, 0.3, 0.2], 0.81), Outcome::Away);
    }

    #[test]
    fn a_band_is_judged_only_with_enough_large_lambda_observations() {
        let mut band = Band {
            low: 10.0,
            high: 100.0,
            observations: 12,
            observed: 150.0,
            predicted: 100.0,
        };
        assert!(band.failing());
        band.observations = 5;
        assert!(!band.failing());
        let low = Band {
            low: 1.0,
            high: 10.0,
            observations: 50,
            observed: 300.0,
            predicted: 100.0,
        };
        assert!(!low.failing());
    }

    /// Une grille relevée (répartition avant la clôture), puis ses résultats et rapports : ses deux rangs entrent dans les
    /// tranches de λ, et le gagnant prédit au rang 2 vaut le λ corrigé par le modèle gelé.
    #[test]
    fn a_settled_grid_feeds_the_calibration() {
        use crate::lotofoot::pronomatch::{GridPage, GridResult, OpenGrid, RankLine};
        use crate::store::lotofoot::{Page, record_fetch, record_grid_page, record_results};
        let mut conn = Connection::open_in_memory().unwrap();
        crate::store::migrate::migrate(&mut conn).unwrap();
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let f = record_fetch(
            &conn,
            at("2026-10-25T15:00:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        let page = GridPage {
            open: Some(OpenGrid {
                game: Game::Lf7,
                accepting: true,
                number: 133,
                closes_at: at("2026-10-25T16:10:00Z"),
                jackpot: None,
                matches: (1..=7)
                    .map(|i| (format!("H{i}"), format!("A{i}")))
                    .collect(),
                split: Some(vec![[50, 30, 20]; 7]),
                split_at: None,
            }),
            history: vec![],
        };
        record_grid_page(&mut conn, f, Game::Lf7, &page).unwrap();
        let r = record_fetch(&conn, at("2026-10-25T21:00:00Z"), Page::Results, 200, None).unwrap();
        let result = GridResult {
            game: Some(Game::Lf7),
            number: 133,
            date: None,
            matches: (1..=7)
                .map(|i| (format!("H{i}"), format!("A{i}"), Outcome::Home))
                .collect(),
            ranks: vec![
                RankLine {
                    good: Some(7),
                    winners: 1500,
                    unit_gain: Euros::new(40.0),
                },
                RankLine {
                    good: Some(6),
                    winners: 15000,
                    unit_gain: Euros::new(5.0),
                },
            ],
        };
        record_results(&mut conn, r, &[result]).unwrap();
        let bands = calibration(&conn).unwrap();
        assert_eq!(bands.iter().map(|b| b.observations).sum::<usize>(), 2);
        // Mises déduites du rang 2 : 15 000 × 5 / 0,385 ; λ du rang 2 = S · 7 · 0,5^6 · 0,5 (tous les favoris sont sortis).
        let stakes = 15_000.0 * 5.0 / 0.385;
        let lambda = stakes * 7.0 * 0.5f64.powi(7);
        let band = bands
            .iter()
            .find(|b| lambda >= b.low && lambda < b.high)
            .unwrap();
        let expected = FROZEN.expected(lambda, stakes, 7);
        assert!(band.predicted >= expected - 1e-6);
    }

    #[test]
    fn an_empty_journal_has_no_session_verdict() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::store::migrate::migrate(&mut conn).unwrap();
        let v = verify(&conn, Utc::now()).unwrap();
        assert_eq!(v.verdict, Verdict::NoSession);
        assert_eq!(v.frequency.settled, 0);
    }
}
