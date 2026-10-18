//! Mesures d'un backtest : CLV, rendement avec intervalle de confiance, croissance, drawdown, risque de ruine.

use super::engine::{BacktestResult, Candidate, StakingRule};
use crate::betting::simultaneous::{Bet, fractions};
use crate::domain::Probability;
use crate::error::Result;
use crate::eval::bootstrap::{MeanCi, SplitMix64, block_bootstrap_mean};

const RESAMPLES: usize = 2000;
const SEED: u64 = 20_261_020;

/// Résumé chiffré d'un backtest.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// Paris placés.
    pub bets: usize,
    /// Total misé.
    pub staked: f64,
    /// Gain net total.
    pub profit: f64,
    /// Gain net rapporté au total misé.
    pub roi_on_stakes: f64,
    /// Bankroll finale.
    pub final_bankroll: f64,
    /// Croissance logarithmique moyenne par lot : `ln(bankroll finale / initiale) / nombre de lots`.
    pub log_growth_per_batch: f64,
    /// Plus forte baisse relative depuis un sommet de bankroll (0,3 = −30 %).
    pub max_drawdown: f64,
}

/// Résume un backtest.
pub fn summarize(result: &BacktestResult) -> Summary {
    let staked: f64 = result.bets.iter().map(|b| b.stake.get()).sum();
    let profit: f64 = result.bets.iter().map(|b| b.profit).sum();
    let batches = result.path.len();
    let log_growth_per_batch = if batches > 0 && result.final_bankroll > 0.0 && result.initial > 0.0
    {
        (result.final_bankroll / result.initial).ln() / batches as f64
    } else {
        0.0
    };
    Summary {
        bets: result.bets.len(),
        staked,
        profit,
        roi_on_stakes: if staked > 0.0 { profit / staked } else { 0.0 },
        final_bankroll: result.final_bankroll,
        log_growth_per_batch,
        max_drawdown: max_drawdown(result.initial, result.path.iter().map(|(_, b)| *b)),
    }
}

/// Plus forte baisse relative d'une série de bankrolls, la bankroll initiale comprise comme premier sommet.
pub fn max_drawdown(initial: f64, path: impl Iterator<Item = f64>) -> f64 {
    let mut peak = initial;
    let mut worst: f64 = 0.0;
    for value in path {
        peak = peak.max(value);
        worst = worst.max((peak - value) / peak);
    }
    worst
}

/// Rendement d'un euro misé à plat sur chaque candidat, avec intervalle de confiance par blocs de journées.
pub fn flat_roi_ci(candidates: &[Candidate]) -> Option<MeanCi> {
    let values: Vec<(u32, f64)> = candidates
        .iter()
        .map(|c| (c.block, if c.won { c.odds.get() - 1.0 } else { -1.0 }))
        .collect();
    block_bootstrap_mean(&values, RESAMPLES, SEED)
}

/// CLV : `cote obtenue × probabilité de clôture sans marge − 1`, c'est-à-dire l'espérance du pari si la
/// clôture du marché dit vrai. C'est la mesure principale : sa variance est bien plus faible que celle du rendement,
/// elle se prononce avec des centaines de paris là où le rendement en demande des dizaines de milliers.
pub fn clv_ci(candidates: &[Candidate]) -> Option<MeanCi> {
    let values: Vec<(u32, f64)> = candidates
        .iter()
        .filter_map(|c| {
            c.close_fair
                .map(|close| (c.block, c.odds.get() * close.get() - 1.0))
        })
        .collect();
    block_bootstrap_mean(&values, RESAMPLES, SEED)
}

/// Les paris retenus gagnent-ils à la fréquence annoncée ? Le diagnostic central d'une règle de sélection.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionCalibration {
    /// Nombre de paris retenus.
    pub n: usize,
    /// Probabilité moyenne annoncée par la règle.
    pub announced: f64,
    /// Fréquence de victoire réellement observée.
    pub realized: f64,
    /// Probabilité moyenne de clôture sans marge (le marché final).
    pub close_fair: Option<f64>,
    /// Espérance moyenne annoncée (`p × cote − 1`).
    pub announced_edge: f64,
}

/// Compare ce que la règle annonce à ce qui se produit.
pub fn selection_calibration(candidates: &[Candidate]) -> Option<SelectionCalibration> {
    if candidates.is_empty() {
        return None;
    }
    let n = candidates.len() as f64;
    let closes: Vec<f64> = candidates
        .iter()
        .filter_map(|c| c.close_fair.map(Probability::get))
        .collect();
    Some(SelectionCalibration {
        n: candidates.len(),
        announced: candidates.iter().map(|c| c.probability.get()).sum::<f64>() / n,
        realized: candidates.iter().filter(|c| c.won).count() as f64 / n,
        close_fair: (!closes.is_empty()).then(|| closes.iter().sum::<f64>() / closes.len() as f64),
        announced_edge: candidates
            .iter()
            .map(|c| c.probability.get() * c.odds.get() - 1.0)
            .sum::<f64>()
            / n,
    })
}

/// Probabilités qui servent à simuler les issues dans le Monte Carlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// Les probabilités annoncées par la règle sont justes : le meilleur cas.
    Announced,
    /// La clôture du marché dit vrai : le cas où l'avantage supposé n'existe pas.
    CloseFair,
}

/// Risque d'une règle de mise, estimé par simulation.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskReport {
    /// Part des trajectoires qui passent sous 10 % de la bankroll initiale (« ruine »).
    pub ruin_probability: f64,
    /// Bankroll finale médiane, en multiples de la bankroll initiale.
    pub median_final: f64,
    /// 95e centile du drawdown maximal.
    pub p95_drawdown: f64,
}

/// Simule `paths` trajectoires de la **même séquence de paris et de mises**, en tirant les issues selon le scénario.
///
/// Les fractions de bankroll de chaque lot sont calculées une fois (Kelly simultané × fraction de Kelly) ; on
/// ignore donc les arrondis, ce qui change peu le résultat. Renvoie `None` sans candidat.
pub fn monte_carlo_risk(
    candidates: &[Candidate],
    rule: &StakingRule,
    scenario: Scenario,
    paths: usize,
) -> Result<Option<RiskReport>> {
    if candidates.is_empty() || paths == 0 {
        return Ok(None);
    }
    // Lots, dans l'ordre chronologique, avec leurs fractions de mise.
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|i| {
        (
            super::engine::decision_date(candidates[*i].date),
            candidates[*i].game_id,
        )
    });
    let mut batches: Vec<Vec<(f64, f64, f64)>> = Vec::new(); // (fraction, cote, probabilité vraie)
    let mut current_date = None;
    for index in order {
        let c = &candidates[index];
        let date = super::engine::decision_date(c.date);
        if current_date != Some(date) {
            batches.push(Vec::new());
            current_date = Some(date);
        }
        let truth = match scenario {
            Scenario::Announced => c.probability.get(),
            Scenario::CloseFair => c.close_fair.unwrap_or(c.probability).get(),
        };
        if let Some(batch) = batches.last_mut() {
            batch.push((0.0, c.odds.get(), truth));
        }
    }
    // Fractions de mise (calculées sur les probabilités ANNONCÉES : c'est ce que la règle croit au moment de miser).
    let mut cursor = 0;
    let mut sorted_candidates: Vec<&Candidate> = candidates.iter().collect();
    sorted_candidates.sort_by_key(|c| (super::engine::decision_date(c.date), c.game_id));
    for batch in batches.iter_mut() {
        let proposals: Vec<Bet> = sorted_candidates[cursor..cursor + batch.len()]
            .iter()
            .map(|c| Bet {
                probability: c.probability,
                odds: c.odds,
            })
            .collect();
        let shares = fractions(&proposals)?;
        for (slot, share) in batch.iter_mut().zip(shares) {
            slot.0 = share * rule.kelly_fraction;
        }
        cursor += batch.len();
    }

    let mut rng = SplitMix64::new(SEED);
    let mut finals = Vec::with_capacity(paths);
    let mut drawdowns = Vec::with_capacity(paths);
    let mut ruined = 0usize;
    for _ in 0..paths {
        let mut bankroll = 1.0;
        let mut peak: f64 = 1.0;
        let mut worst: f64 = 0.0;
        let mut hit_ruin = false;
        for batch in &batches {
            let staked: f64 = batch.iter().map(|(f, _, _)| f).sum();
            let mut wealth = 1.0 - staked;
            for (fraction, odds, truth) in batch {
                if rng.next_f64() < *truth {
                    wealth += fraction * odds;
                }
            }
            bankroll *= wealth;
            peak = peak.max(bankroll);
            worst = worst.max((peak - bankroll) / peak);
            if bankroll < 0.10 {
                hit_ruin = true;
            }
        }
        ruined += usize::from(hit_ruin);
        finals.push(bankroll);
        drawdowns.push(worst);
    }
    finals.sort_by(|a, b| a.total_cmp(b));
    drawdowns.sort_by(|a, b| a.total_cmp(b));
    let at = |sorted: &[f64], q: f64| {
        sorted[((q * (sorted.len() - 1) as f64).round() as usize).min(sorted.len() - 1)]
    };
    Ok(Some(RiskReport {
        ruin_probability: ruined as f64 / paths as f64,
        median_final: at(&finals, 0.5),
        p95_drawdown: at(&drawdowns, 0.95),
    }))
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;
    use chrono::NaiveDate;

    use super::*;
    use crate::backtest::engine::{PlacedBet, run};
    use crate::domain::{Euros, Odds, Outcome};

    fn candidate(
        block: u32,
        probability: f64,
        odds: f64,
        close: Option<f64>,
        won: bool,
    ) -> Candidate {
        Candidate {
            game_id: i64::from(block),
            date: NaiveDate::from_ymd_opt(2025, 8, 16).unwrap()
                + chrono::Days::new(u64::from(block) * 7),
            block,
            outcome: Outcome::Home,
            odds: Odds::new(odds).unwrap(),
            probability: Probability::new(probability).unwrap(),
            close_fair: close.and_then(Probability::new),
            won,
        }
    }

    #[test]
    fn drawdown_is_the_worst_fall_from_a_peak() {
        // 1000 → 1200 (sommet) → 900 (−25 %) → 1100 → 800 (−33,3 % depuis 1200).
        let worst = max_drawdown(1000.0, [1200.0, 900.0, 1100.0, 800.0].into_iter());
        assert_abs_diff_eq!(worst, 1.0 - 800.0 / 1200.0, epsilon = 1e-12);
        // Une bankroll qui ne fait que monter n'a aucun drawdown.
        assert_eq!(max_drawdown(1000.0, [1100.0, 1200.0].into_iter()), 0.0);
        // Le premier sommet est la bankroll initiale.
        assert_abs_diff_eq!(
            max_drawdown(1000.0, [900.0].into_iter()),
            0.1,
            epsilon = 1e-12
        );
    }

    #[test]
    fn the_summary_matches_a_hand_computation() {
        let result = BacktestResult {
            bets: vec![
                PlacedBet {
                    candidate: 0,
                    batch: NaiveDate::from_ymd_opt(2025, 8, 15).unwrap(),
                    stake: Euros::new(100.0).unwrap(),
                    profit: 100.0,
                },
                PlacedBet {
                    candidate: 1,
                    batch: NaiveDate::from_ymd_opt(2025, 8, 22).unwrap(),
                    stake: Euros::new(100.0).unwrap(),
                    profit: -100.0,
                },
            ],
            path: vec![
                (NaiveDate::from_ymd_opt(2025, 8, 15).unwrap(), 1100.0),
                (NaiveDate::from_ymd_opt(2025, 8, 22).unwrap(), 1000.0),
            ],
            initial: 1000.0,
            final_bankroll: 1000.0,
        };

        let summary = summarize(&result);

        assert_eq!(summary.bets, 2);
        assert_abs_diff_eq!(summary.staked, 200.0);
        assert_abs_diff_eq!(summary.profit, 0.0);
        assert_abs_diff_eq!(summary.roi_on_stakes, 0.0);
        assert_abs_diff_eq!(summary.log_growth_per_batch, 0.0, epsilon = 1e-12);
        assert_abs_diff_eq!(summary.max_drawdown, 100.0 / 1100.0, epsilon = 1e-12);
    }

    #[test]
    fn flat_roi_and_clv_are_computed_per_bet() {
        // Cote 2,0 gagnée : +1 ; cote 3,0 perdue : −1 ; moyenne 0.
        let candidates = [
            candidate(0, 0.55, 2.0, Some(0.52), true),
            candidate(1, 0.40, 3.0, Some(0.35), false),
        ];

        let roi = flat_roi_ci(&candidates).unwrap();
        let clv = clv_ci(&candidates).unwrap();

        assert_abs_diff_eq!(roi.mean, 0.0, epsilon = 1e-12);
        // CLV : (2,0 × 0,52 − 1 = +0,04) et (3,0 × 0,35 − 1 = +0,05) → moyenne +0,045.
        assert_abs_diff_eq!(clv.mean, 0.045, epsilon = 1e-12);
    }

    #[test]
    fn clv_ignores_bets_without_a_closing_price() {
        let candidates = [
            candidate(0, 0.55, 2.0, None, true),
            candidate(1, 0.5, 2.2, Some(0.5), false),
        ];
        let clv = clv_ci(&candidates).unwrap();
        assert_abs_diff_eq!(clv.mean, 2.2 * 0.5 - 1.0, epsilon = 1e-12);
        assert!(clv_ci(&[candidate(0, 0.55, 2.0, None, true)]).is_none());
    }

    #[test]
    fn selection_calibration_contrasts_what_is_announced_and_what_happens() {
        let candidates = [
            candidate(0, 0.40, 3.0, Some(0.30), true),
            candidate(1, 0.40, 3.0, Some(0.30), false),
            candidate(2, 0.40, 3.0, Some(0.30), false),
            candidate(3, 0.40, 3.0, Some(0.30), false),
        ];

        let calibration = selection_calibration(&candidates).unwrap();

        assert_eq!(calibration.n, 4);
        assert_abs_diff_eq!(calibration.announced, 0.40, epsilon = 1e-12);
        assert_abs_diff_eq!(calibration.realized, 0.25, epsilon = 1e-12);
        assert_abs_diff_eq!(calibration.close_fair.unwrap(), 0.30, epsilon = 1e-12);
        assert_abs_diff_eq!(
            calibration.announced_edge,
            0.40 * 3.0 - 1.0,
            epsilon = 1e-12
        );
        assert!(selection_calibration(&[]).is_none());
    }

    fn series(probability: f64, odds: f64, close: f64, count: u32) -> Vec<Candidate> {
        (0..count)
            .map(|i| candidate(i, probability, odds, Some(close), i % 2 == 0))
            .collect()
    }

    #[test]
    fn a_real_edge_that_the_rule_estimates_correctly_has_almost_no_ruin_risk() {
        // p = 0,6 à la cote 2,2 (avantage 32 %) : si c'est vrai, la ruine est quasi impossible à demi-Kelly.
        let candidates = series(0.6, 2.2, 0.6, 40);
        let rule = StakingRule::standard(0.5);

        let risk = monte_carlo_risk(&candidates, &rule, Scenario::Announced, 500)
            .unwrap()
            .unwrap();

        assert!(risk.ruin_probability < 0.02, "{risk:?}");
        assert!(risk.median_final > 1.0);
    }

    #[test]
    fn if_the_assumed_edge_does_not_exist_full_kelly_is_very_risky() {
        // La règle croit à p = 0,6 mais la clôture du marché dit 0,45 : l'avantage est illusoire.
        let candidates = series(0.6, 2.2, 0.45, 60);
        let full = StakingRule::standard(1.0);
        let quarter = StakingRule::standard(0.25);

        let full_risk = monte_carlo_risk(&candidates, &full, Scenario::CloseFair, 500)
            .unwrap()
            .unwrap();
        let quarter_risk = monte_carlo_risk(&candidates, &quarter, Scenario::CloseFair, 500)
            .unwrap()
            .unwrap();

        assert!(full_risk.ruin_probability > 0.5, "{full_risk:?}");
        assert!(quarter_risk.ruin_probability < full_risk.ruin_probability);
        assert!(quarter_risk.median_final > full_risk.median_final);
        assert!(quarter_risk.p95_drawdown < full_risk.p95_drawdown);
    }

    #[test]
    fn the_simulation_is_reproducible_and_empty_input_gives_nothing() {
        let candidates = series(0.55, 2.1, 0.5, 20);
        let rule = StakingRule::standard(1.0);
        let a = monte_carlo_risk(&candidates, &rule, Scenario::Announced, 200).unwrap();
        let b = monte_carlo_risk(&candidates, &rule, Scenario::Announced, 200).unwrap();
        assert_eq!(a, b);
        assert!(
            monte_carlo_risk(&[], &rule, Scenario::Announced, 100)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_backtest_and_the_simulation_agree_on_the_staking_fractions() {
        // Sans arrondi ni aléa : une trajectoire entièrement gagnante du backtest multiplie la bankroll comme la simulation.
        let candidates = series(0.6, 2.2, 0.6, 3)
            .into_iter()
            .map(|mut c| {
                c.won = true;
                c
            })
            .collect::<Vec<_>>();
        let result = run(&candidates, &StakingRule::standard(1.0)).unwrap();
        assert!(result.final_bankroll > 1000.0);
        assert!(summarize(&result).max_drawdown < 1e-9);
    }
}
