//! Abaque des seuils : pour chaque avantage minimal de 0 à 8 % (pas de 0,1 point), ce que retient une règle « prix mou
//! contre Pinnacle » : nombre de paris, CLV moyen et **croissance logarithmique totale attendue** de la bankroll.
//!
//! Tracée sur l'entraînement et la validation seulement : le test reste aveugle jusqu'au gel (invariant 8). Le seuil gelé
//! est celui qui maximise la croissance totale attendue en demi-Kelly ([`FREEZE_KELLY`]), et non plus le CLV moyen.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::Connection;
use serde::Serialize;

use super::candidates::{GameInputs, Generator, Period, SOFT_BOOKMAKERS, in_period, soft_vs_sharp};
use super::engine::{Candidate, decision_date};
use super::metrics::{RESAMPLES, SEED, clv_ci};
use super::study::{Criterion, KELLY_FRACTIONS, MIN_SELECTION_BETS};
use crate::betting::simultaneous::{Bet, expected_log_growth, fractions};
use crate::betting::value::best_selection;
use crate::domain::{Moment, Outcome};
use crate::error::{Error, Result};
use crate::eval::bootstrap::{MeanCi, block_bootstrap_mean};
use crate::ingest::football_data::Season;
use crate::market::margin::{MarginMethod, remove_margin};
use crate::store::backtests::{self, FrozenThreshold};

/// Pas de l'abaque : 0,1 point d'avantage.
pub const STEP: f64 = 0.001;

/// Nombre de pas : l'abaque va de 0 à `STEPS × STEP` = 8 %.
pub const STEPS: u32 = 80;

/// Périodes sur lesquelles l'abaque est tracée et le seuil choisi. Jamais le test.
pub const TUNING_PERIODS: [Period; 2] = [Period::Train, Period::Validation];

/// Position, dans [`KELLY_FRACTIONS`] (×1, ½, ¼), de la fraction de Kelly qui choisit le seuil gelé : ½.
///
/// Décision du 2026-10-20 : en Kelly complet, la croissance de Bet365 est négative à **tous** les seuils (l'avantage annoncé
/// vaut environ trois fois l'avantage réel, Kelly complet mise donc trop) ; son maximum ne désignerait qu'un « moins mauvais ».
pub const FREEZE_KELLY_INDEX: usize = 1;

/// Fraction de Kelly du critère de gel.
pub const FREEZE_KELLY: f64 = KELLY_FRACTIONS[FREEZE_KELLY_INDEX];

/// Seuils auxquels le détail par issue (domicile, nul, extérieur) est donné.
pub const OUTCOME_THRESHOLDS: [f64; 5] = [0.0, 0.005, 0.01, 0.02, 0.03];

/// Les seuils de l'abaque, de 0 à 8 %.
pub fn thresholds() -> Vec<f64> {
    (0..=STEPS).map(|k| f64::from(k) * STEP).collect()
}

/// Un point de l'abaque.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CurvePoint {
    /// Avantage minimal exigé.
    pub threshold: f64,
    /// Paris retenus (au plus un par match).
    pub bets: usize,
    /// CLV moyen par pari, avec intervalle à 95 % (bootstrap par blocs ligue × semaine).
    pub clv: Option<MeanCi>,
    /// Rendement moyen d'un euro misé à plat : très bruité, donné pour mémoire.
    pub flat_roi: Option<f64>,
    /// Croissance logarithmique totale attendue sur la période, une valeur par fraction de Kelly (×1, ½, ¼).
    pub growth: Vec<f64>,
    /// Intervalle à 95 % de la croissance totale en demi-Kelly, la fraction du gel (bootstrap par lots de décision).
    pub growth_ci: Option<(f64, f64)>,
}

/// Nombre de paris et CLV moyen d'une issue.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct OutcomeCell {
    /// Paris retenus sur cette issue.
    pub bets: usize,
    /// CLV moyen.
    pub clv: Option<f64>,
}

/// Détail par issue à un seuil donné : le nul est-il surreprésenté, et son CLV est-il différent ?
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeSplit {
    /// Avantage minimal.
    pub threshold: f64,
    /// Victoire de l'équipe à domicile.
    pub home: OutcomeCell,
    /// Nul.
    pub draw: OutcomeCell,
    /// Victoire de l'équipe à l'extérieur.
    pub away: OutcomeCell,
}

/// Imprécision de l'avantage due à la méthode de retrait de marge : pour la même sélection, l'écart absolu entre
/// l'avantage calculé avec la méthode de la configuration et avec chacune des deux autres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MarginSpread {
    /// Comparaisons faites (deux par sélection).
    pub compared: usize,
    /// Écart médian, en points d'avantage (0,002 = 0,2 point).
    pub median: f64,
    /// 90e centile de l'écart.
    pub p90: f64,
}

/// L'abaque d'une source.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceCurve {
    /// Source (`soft:B365`…).
    pub label: String,
    /// Lots de décision (vendredis et mardis) qui ont au moins un pari à 0 %.
    pub batches: usize,
    /// Saisons couvertes par ces paris.
    pub seasons: usize,
    /// Un point par seuil.
    pub points: Vec<CurvePoint>,
    /// Seuil de plus forte croissance attendue en demi-Kelly ([`FREEZE_KELLY`]), parmi ceux qui gardent au moins
    /// [`MIN_SELECTION_BETS`] paris. `None` si aucun ne les garde.
    pub best: Option<f64>,
    /// Détail par issue aux seuils [`OUTCOME_THRESHOLDS`].
    pub by_outcome: Vec<OutcomeSplit>,
    /// Sensibilité de l'avantage à la méthode de retrait de marge.
    pub margin_spread: Option<MarginSpread>,
}

impl SourceCurve {
    /// Le point de plus forte croissance (celui du seuil [`SourceCurve::best`]).
    pub fn best_point(&self) -> Option<&CurvePoint> {
        self.best
            .and_then(|best| self.points.iter().find(|p| p.threshold == best))
    }
}

/// Avantage annoncé d'un candidat : `p × cote − 1`.
fn edge(candidate: &Candidate) -> f64 {
    candidate.probability.get() * candidate.odds.get() - 1.0
}

/// Un lot de décision, ses candidats triés par avantage décroissant.
///
/// Pourquoi trier : un seuil `t` retient les paris d'avantage `≥ t`, c'est-à-dire **les `k` premiers** du lot trié. Les
/// mises de Kelly simultané d'un lot ne dépendent que de ses paris ; il suffit donc de calculer la croissance de
/// chaque préfixe une fois (au plus la taille du lot) au lieu d'une fois par seuil (81).
struct Batch {
    members: Vec<Candidate>,
    /// `growth[k][j]` : croissance attendue du lot réduit à ses `k` meilleurs paris, en fraction de Kelly
    /// `KELLY_FRACTIONS[j]`. `growth[0]` vaut zéro (aucun pari).
    growth: Vec<Vec<f64>>,
}

/// Croissance attendue d'un lot pour chaque fraction de Kelly.
///
/// Les mises sont calculées comme en production, sur les probabilités **annoncées** (Pinnacle d'avant-match sans
/// marge) ; la croissance est évaluée sous la **clôture** sans marge, tenue pour vraie (à défaut de clôture, la
/// probabilité annoncée, comme dans le Monte Carlo de [`super::metrics`]) :
///
/// ```text
/// G = Σ_S Q(S) · ln(1 − Σ_i f_i + Σ_{i∈S} f_i·cote_i),   f = fraction × Kelly_simultané(p_annoncées)
/// ```
fn lot_growth(members: &[Candidate]) -> Result<Vec<f64>> {
    let announced: Vec<Bet> = members
        .iter()
        .map(|c| Bet {
            probability: c.probability,
            odds: c.odds,
        })
        .collect();
    let truth: Vec<Bet> = members
        .iter()
        .map(|c| Bet {
            probability: c.close_fair.unwrap_or(c.probability),
            odds: c.odds,
        })
        .collect();
    let shares = fractions(&announced)?;
    Ok(KELLY_FRACTIONS
        .iter()
        .map(|kelly| {
            let stakes: Vec<f64> = shares.iter().map(|share| share * kelly).collect();
            expected_log_growth(&truth, &stakes)
        })
        .collect())
}

/// Regroupe les candidats par date de décision et calcule la croissance de chaque préfixe.
///
/// `candidates` est pris par valeur : chaque candidat est déplacé dans son lot, sans copie.
fn into_batches(candidates: Vec<Candidate>) -> Result<Vec<Batch>> {
    let mut by_date: BTreeMap<NaiveDate, Vec<Candidate>> = BTreeMap::new();
    for candidate in candidates {
        by_date
            .entry(decision_date(candidate.date))
            .or_default()
            .push(candidate);
    }
    let mut batches = Vec::with_capacity(by_date.len());
    for mut members in by_date.into_values() {
        // Départage par identifiant de match : même entrée, même ordre (invariant 9).
        members.sort_by(|a, b| edge(b).total_cmp(&edge(a)).then(a.game_id.cmp(&b.game_id)));
        let mut growth = vec![vec![0.0; KELLY_FRACTIONS.len()]];
        for k in 1..=members.len() {
            growth.push(lot_growth(&members[..k])?);
        }
        batches.push(Batch { members, growth });
    }
    Ok(batches)
}

/// Le point de l'abaque au seuil `threshold`.
fn point(batches: &[Batch], threshold: f64) -> CurvePoint {
    // Clonés : les mesures (`clv_ci`) attendent une tranche contiguë de candidats, or ceux-ci sont répartis dans les lots.
    let mut selected: Vec<Candidate> = Vec::new();
    let mut growth = vec![0.0; KELLY_FRACTIONS.len()];
    let mut per_batch: Vec<(u32, f64)> = Vec::with_capacity(batches.len());
    for (index, batch) in batches.iter().enumerate() {
        let k = batch
            .members
            .iter()
            .take_while(|c| edge(c) >= threshold)
            .count();
        selected.extend_from_slice(&batch.members[..k]);
        for (total, value) in growth.iter_mut().zip(&batch.growth[k]) {
            *total += value;
        }
        per_batch.push((index as u32, batch.growth[k][FREEZE_KELLY_INDEX]));
    }
    // Intervalle du total = nombre de lots × intervalle de la moyenne par lot (les lots sont rééchantillonnés entiers).
    let lots = batches.len() as f64;
    let growth_ci =
        block_bootstrap_mean(&per_batch, RESAMPLES, SEED).map(|ci| (ci.low * lots, ci.high * lots));
    let flat_roi = (!selected.is_empty()).then(|| {
        selected
            .iter()
            .map(|c| if c.won { c.odds.get() - 1.0 } else { -1.0 })
            .sum::<f64>()
            / selected.len() as f64
    });
    CurvePoint {
        threshold,
        bets: selected.len(),
        clv: clv_ci(&selected),
        flat_roi,
        growth,
        growth_ci,
    }
}

fn outcome_cell(candidates: &[&Candidate], outcome: Outcome) -> OutcomeCell {
    let clvs: Vec<f64> = candidates
        .iter()
        .filter(|c| c.outcome == outcome)
        .filter_map(|c| c.close_fair.map(|close| c.odds.get() * close.get() - 1.0))
        .collect();
    OutcomeCell {
        bets: candidates.iter().filter(|c| c.outcome == outcome).count(),
        clv: (!clvs.is_empty()).then(|| clvs.iter().sum::<f64>() / clvs.len() as f64),
    }
}

fn outcome_splits(batches: &[Batch]) -> Vec<OutcomeSplit> {
    OUTCOME_THRESHOLDS
        .iter()
        .map(|threshold| {
            let selected: Vec<&Candidate> = batches
                .iter()
                .flat_map(|b| &b.members)
                .filter(|c| edge(c) >= *threshold)
                .collect();
            OutcomeSplit {
                threshold: *threshold,
                home: outcome_cell(&selected, Outcome::Home),
                draw: outcome_cell(&selected, Outcome::Draw),
                away: outcome_cell(&selected, Outcome::Away),
            }
        })
        .collect()
}

/// Pour chaque sélection à 0 % (méthode de la configuration), l'avantage de la même issue avec les deux autres méthodes.
fn margin_spread(games: &[&GameInputs], book: &str, method: MarginMethod) -> Option<MarginSpread> {
    let mut gaps: Vec<f64> = Vec::new();
    for game in games {
        let (Some(sharp), Some(soft)) = (game.reference_pre(), game.price(book, Moment::Pre))
        else {
            continue;
        };
        let Some(selection) =
            remove_margin(sharp, method).and_then(|probs| best_selection(&probs, soft, 0.0))
        else {
            continue;
        };
        for other in MarginMethod::ALL.into_iter().filter(|m| *m != method) {
            if let Some(probs) = remove_margin(sharp, other) {
                let other_edge = probs.get(selection.outcome).get() * selection.odds.get() - 1.0;
                gaps.push((other_edge - selection.edge).abs());
            }
        }
    }
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_by(f64::total_cmp);
    let at = |q: f64| gaps[((q * (gaps.len() - 1) as f64).round() as usize).min(gaps.len() - 1)];
    Some(MarginSpread {
        compared: gaps.len(),
        median: at(0.5),
        p90: at(0.9),
    })
}

/// L'abaque d'un bookmaker « mou » sur les matchs donnés.
pub fn source_curve(
    games: &[&GameInputs],
    book: &str,
    method: MarginMethod,
) -> Result<SourceCurve> {
    let generator = Generator {
        min_edge: 0.0,
        price_factor: 1.0,
        method,
    };
    let candidates = soft_vs_sharp(games, book, &generator);
    let seasons: BTreeSet<Season> = candidates
        .iter()
        .map(|c| Season::containing(c.date))
        .collect();
    let batches = into_batches(candidates)?;
    let points: Vec<CurvePoint> = thresholds()
        .into_iter()
        .map(|t| point(&batches, t))
        .collect();
    let best = points
        .iter()
        .filter(|p| p.bets >= MIN_SELECTION_BETS)
        .max_by(|a, b| a.growth[FREEZE_KELLY_INDEX].total_cmp(&b.growth[FREEZE_KELLY_INDEX]))
        .map(|p| p.threshold);
    Ok(SourceCurve {
        label: format!("soft:{book}"),
        batches: batches.len(),
        seasons: seasons.len(),
        best,
        by_outcome: outcome_splits(&batches),
        margin_spread: margin_spread(games, book, method),
        points,
    })
}

/// Les abaques des huit bookmakers « mous », sur l'entraînement et la validation.
pub fn curves(all: &[GameInputs], method: MarginMethod) -> Result<Vec<SourceCurve>> {
    let games: Vec<&GameInputs> = TUNING_PERIODS
        .iter()
        .flat_map(|period| in_period(all, *period))
        .collect();
    SOFT_BOOKMAKERS
        .iter()
        .map(|book| source_curve(&games, book, method))
        .collect()
}

/// Configurations essayées par une session de l'abaque : chaque seuil de chaque source.
pub fn configs_tried() -> usize {
    SOFT_BOOKMAKERS.len() * (STEPS as usize + 1)
}

/// Gèle, pour chaque source qui en a un, le seuil de plus forte croissance attendue. C'est une **nouvelle session de
/// réglage** : elle est comptée dans le rapport. Refuse si un gel à ce critère existe déjà, pour qu'une relance par
/// mégarde ne multiplie pas les sessions.
pub fn freeze(
    conn: &Connection,
    curves: &[SourceCurve],
    now: DateTime<Utc>,
) -> Result<Vec<FrozenThreshold>> {
    for curve in curves {
        if backtests::latest_selection(conn, &curve.label)?
            .is_some_and(|f| f.criterion == Criterion::ExpectedGrowth)
        {
            return Err(Error::Data(format!(
                "{} a déjà un seuil gelé sur la croissance attendue : une nouvelle session ne se lance pas par mégarde",
                curve.label
            )));
        }
    }
    let mut frozen = Vec::new();
    for curve in curves {
        let Some(point) = curve.best_point() else {
            continue;
        };
        let record = FrozenThreshold {
            source: curve.label.clone(),
            threshold: point.threshold,
            selection_clv: point.clv.map(|c| c.mean),
            selection_bets: point.bets,
            configs_tried: configs_tried(),
            criterion: Criterion::ExpectedGrowth,
            selection_growth: Some(point.growth[FREEZE_KELLY_INDEX]),
            selection_kelly: Some(FREEZE_KELLY),
        };
        backtests::insert_selection(conn, &record, now)?;
        frozen.push(record);
    }
    Ok(frozen)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use approx::assert_abs_diff_eq;
    use chrono::Days;

    use super::*;
    use crate::domain::{Odds, Odds1x2};
    use crate::models::testing::Lcg;
    use crate::store::migrate;

    fn odds(p: [f64; 3], margin: f64) -> Odds1x2 {
        let price = |q: f64| Odds::new(1.0 / (q * (1.0 + margin))).unwrap();
        Odds1x2 {
            home: price(p[0]),
            draw: price(p[1]),
            away: price(p[2]),
        }
    }

    /// Monde simulé : Pinnacle dit vrai et clôture à son prix d'avant-match ; Bet365 bruite ce prix de ± `lag`.
    /// Les matchs commencent le `start` et se suivent tous les deux jours.
    fn world(games: usize, lag: f64, seed: u64, start: NaiveDate) -> Vec<GameInputs> {
        let mut rng = Lcg(seed);
        (0..games)
            .map(|i| {
                let home = 0.25 + 0.45 * rng.next();
                let draw = (1.0 - home) * (0.25 + 0.2 * rng.next());
                let p = [home, draw, 1.0 - home - draw];
                let roll = rng.next();
                let result = if roll < p[0] {
                    Outcome::Home
                } else if roll < p[0] + p[1] {
                    Outcome::Draw
                } else {
                    Outcome::Away
                };
                let sharp = odds(p, 0.02);
                let jitter = |rng: &mut Lcg| 1.0 + lag * (2.0 * rng.next() - 1.0);
                let soft = Odds1x2 {
                    home: Odds::new(sharp.home.get() * jitter(&mut rng)).unwrap(),
                    draw: Odds::new(sharp.draw.get() * jitter(&mut rng)).unwrap(),
                    away: Odds::new(sharp.away.get() * jitter(&mut rng)).unwrap(),
                };
                let mut prices = HashMap::new();
                prices.insert(("PS".to_string(), Moment::Pre), sharp);
                prices.insert(("PS".to_string(), Moment::Close), sharp);
                prices.insert(("B365".to_string(), Moment::Pre), soft);
                GameInputs {
                    game_id: i as i64 + 1,
                    date: start + Days::new(2 * i as u64),
                    block: (i / 4) as u32,
                    result,
                    odds: prices,
                }
            })
            .collect()
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn b365(all: &[GameInputs]) -> SourceCurve {
        curves(all, MarginMethod::Multiplicative)
            .unwrap()
            .into_iter()
            .find(|c| c.label == "soft:B365")
            .unwrap()
    }

    #[test]
    fn the_curve_has_one_point_per_tenth_of_a_point_and_fewer_bets_as_the_threshold_rises() {
        let curve = b365(&world(1500, 0.08, 1, day(2021, 8, 6)));

        assert_eq!(curve.points.len(), 81);
        assert_abs_diff_eq!(curve.points[80].threshold, 0.08, epsilon = 1e-12);
        let bets: Vec<usize> = curve.points.iter().map(|p| p.bets).collect();
        assert!(bets.windows(2).all(|w| w[0] >= w[1]), "{bets:?}");
        assert!(bets[0] > bets[80]);
    }

    #[test]
    fn each_point_matches_a_direct_selection_at_its_threshold() {
        // Le raccourci par préfixes doit donner exactement ce que donnerait le générateur au seuil demandé.
        let all = world(800, 0.08, 2, day(2021, 8, 6));
        let curve = b365(&all);
        // Le monde déborde sur le test (800 matchs × 2 jours) : la comparaison porte sur les périodes de réglage.
        let games: Vec<&GameInputs> = all
            .iter()
            .filter(|g| Period::of(g.date) != Period::Test)
            .collect();
        for point in curve.points.iter().step_by(10) {
            let generator = Generator {
                min_edge: point.threshold,
                price_factor: 1.0,
                method: MarginMethod::Multiplicative,
            };
            let direct = soft_vs_sharp(&games, "B365", &generator);
            assert_eq!(point.bets, direct.len(), "seuil {}", point.threshold);
            assert_eq!(point.clv, clv_ci(&direct));
        }
    }

    #[test]
    fn a_real_edge_gives_positive_growth_and_no_edge_gives_none() {
        // Bookmaker lent (± 8 %) contre une vérité connue : la croissance attendue est positive.
        let lagging = b365(&world(1500, 0.08, 3, day(2021, 8, 6)));
        assert!(lagging.points[0].growth[0] > 0.0, "{:?}", lagging.points[0]);
        // Bookmaker identique à Pinnacle (même marge de 2 %) : aucune espérance positive, donc aucun pari ni croissance.
        let identical = b365(&world(1500, 0.0, 3, day(2021, 8, 6)));
        assert_eq!(identical.points[0].bets, 0);
        assert_eq!(identical.points[0].growth, vec![0.0; 3]);
        assert_eq!(identical.best, None);
    }

    #[test]
    fn the_growth_of_a_lot_is_zero_without_edge_and_kelly_fractions_are_ordered() {
        let lagging = b365(&world(1500, 0.08, 4, day(2021, 8, 6)));
        let point = &lagging.points[0];
        // Avantage réel et petites mises : ×1 > ×½ > ×¼ > 0 (on reste sous le double du Kelly optimal).
        assert!(point.growth[0] > point.growth[1], "{:?}", point.growth);
        assert!(point.growth[1] > point.growth[2]);
        assert!(point.growth[2] > 0.0);
        let (low, high) = point.growth_ci.unwrap();
        let half = point.growth[FREEZE_KELLY_INDEX];
        assert!(low <= half && half <= high);
    }

    #[test]
    fn test_period_games_never_enter_the_curve() {
        // Mêmes matchs d'entraînement ; on ajoute des matchs de test très « mous » : l'abaque ne doit pas bouger.
        let train = world(600, 0.08, 5, day(2021, 8, 6));
        let mut with_test = train.clone();
        with_test.extend(
            world(600, 0.30, 6, day(2025, 8, 1))
                .into_iter()
                .map(|mut g| {
                    g.game_id += 10_000;
                    g
                }),
        );

        assert_eq!(b365(&train), b365(&with_test));
    }

    #[test]
    fn the_outcome_split_adds_up_and_the_margin_spread_is_zero_for_a_single_method_world() {
        let curve = b365(&world(1500, 0.08, 7, day(2021, 8, 6)));
        let split = &curve.by_outcome[0];
        assert_eq!(
            split.home.bets + split.draw.bets + split.away.bets,
            curve.points[0].bets
        );
        // La marge de 2 % est proportionnelle : les trois méthodes diffèrent peu, mais pas exactement (Shin, puissance).
        let spread = curve.margin_spread.unwrap();
        assert!(spread.median < 0.01, "{spread:?}");
        assert!(spread.p90 >= spread.median);
    }

    #[test]
    fn the_freeze_records_the_growth_criterion_once_and_refuses_a_second_session() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        let curves = curves(
            &world(3000, 0.08, 8, day(2021, 8, 6)),
            MarginMethod::Multiplicative,
        )
        .unwrap();

        let frozen = freeze(&conn, &curves, Utc::now()).unwrap();

        assert_eq!(frozen.len(), 1, "seul Bet365 a des cotes dans ce monde");
        let stored = backtests::latest_selection(&conn, "soft:B365")
            .unwrap()
            .unwrap();
        assert_eq!(stored.criterion, Criterion::ExpectedGrowth);
        assert_eq!(stored.configs_tried, 8 * 81);
        assert_eq!(Some(stored.threshold), curves[0].best);
        assert!(stored.selection_growth.unwrap() > 0.0);
        assert_eq!(stored.selection_kelly, Some(0.5));
        assert!(freeze(&conn, &curves, Utc::now()).is_err());
        assert_eq!(backtests::selection_session_count(&conn).unwrap(), 1);
    }
}
