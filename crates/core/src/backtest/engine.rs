//! Moteur séquentiel : lots de décision, mises de Kelly simultané, règlement, évolution de la bankroll.

use chrono::{Datelike, Days, NaiveDate};

use crate::betting::kelly::stake;
use crate::betting::simultaneous::{Bet, fractions};
use crate::domain::{Euros, Odds, Outcome, Probability};
use crate::error::Result;

/// Un pari candidat : l'issue retenue d'un match, avec le prix et la probabilité connus au moment de décider.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Identifiant du match en base.
    pub game_id: i64,
    /// Date du match.
    pub date: NaiveDate,
    /// Bloc de bootstrap (une ligue et une semaine).
    pub block: u32,
    /// Issue sur laquelle on parie.
    pub outcome: Outcome,
    /// Cote obtenue (après décote éventuelle).
    pub odds: Odds,
    /// Probabilité estimée de l'issue **au moment de la décision**.
    pub probability: Probability,
    /// Probabilité de clôture sans marge de la même issue : sert uniquement à mesurer le CLV, jamais à décider.
    pub close_fair: Option<Probability>,
    /// L'issue s'est-elle produite ?
    pub won: bool,
}

/// Date de décision d'un match, selon le rythme de collecte des cotes d'avant-match : le **vendredi** pour les
/// matchs du vendredi au lundi, le **mardi** pour ceux du mardi au jeudi.
pub fn decision_date(match_date: NaiveDate) -> NaiveDate {
    let back = match match_date.weekday().num_days_from_monday() {
        0 => 3, // lundi → vendredi précédent
        1 => 0, // mardi
        2 => 1, // mercredi → mardi
        3 => 2, // jeudi → mardi
        4 => 0, // vendredi
        5 => 1, // samedi → vendredi
        _ => 2, // dimanche → vendredi
    };
    match_date - Days::new(back)
}

/// Règle de mise du backtest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StakingRule {
    /// Fraction de Kelly appliquée (1 = Kelly complet, 0,5 = demi-Kelly…).
    pub kelly_fraction: f64,
    /// Bankroll de départ.
    pub initial_bankroll: Euros,
    /// Mise minimale : un pari plus petit n'est pas placé.
    pub min_stake: Euros,
}

impl StakingRule {
    /// Règle standard des rapports : bankroll de 1 000 €, mise minimale de 1 €, fraction de Kelly au choix.
    pub fn standard(kelly_fraction: f64) -> StakingRule {
        StakingRule {
            kelly_fraction,
            // Constantes finies et positives : `Euros::new` ne peut pas échouer ici.
            initial_bankroll: Euros::new(1000.0).expect("1000 est un montant valide"),
            min_stake: Euros::new(1.0).expect("1 est un montant valide"),
        }
    }
}

/// Un pari effectivement placé.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedBet {
    /// Indice du candidat dans la liste d'entrée.
    pub candidate: usize,
    /// Date de décision du lot.
    pub batch: NaiveDate,
    /// Mise.
    pub stake: Euros,
    /// Gain net (négatif si perdu : l'opposé de la mise).
    pub profit: f64,
}

/// Résultat d'un backtest.
#[derive(Debug, Clone, PartialEq)]
pub struct BacktestResult {
    /// Paris placés, dans l'ordre chronologique.
    pub bets: Vec<PlacedBet>,
    /// Bankroll après chaque lot qui a placé au moins un pari : `(date de décision, bankroll)`.
    pub path: Vec<(NaiveDate, f64)>,
    /// Bankroll de départ.
    pub initial: f64,
    /// Bankroll finale.
    pub final_bankroll: f64,
}

/// Rejoue les candidats lot par lot : à chaque date de décision, on dimensionne **ensemble** les paris du lot
/// (Kelly simultané, puis fraction de Kelly), on arrondit les mises, puis on règle le lot avant le suivant.
///
/// Règle d'or : seule la bankroll du moment, les cotes et les probabilités d'avant-match interviennent. Aucun résultat
/// futur ni aucune cote de clôture n'entre dans le calcul d'une mise.
pub fn run(candidates: &[Candidate], rule: &StakingRule) -> Result<BacktestResult> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|i| (decision_date(candidates[*i].date), candidates[*i].game_id));

    let mut bankroll = rule.initial_bankroll.get();
    let mut bets = Vec::new();
    let mut path = Vec::new();
    let mut start = 0;
    while start < order.len() {
        let batch_date = decision_date(candidates[order[start]].date);
        let end = order[start..]
            .iter()
            .position(|i| decision_date(candidates[*i].date) != batch_date)
            .map_or(order.len(), |offset| start + offset);
        let batch = &order[start..end];
        start = end;

        let Some(available) = Euros::new(bankroll) else {
            break;
        };
        let proposals: Vec<Bet> = batch
            .iter()
            .map(|i| Bet {
                probability: candidates[*i].probability,
                odds: candidates[*i].odds,
            })
            .collect();
        let shares = fractions(&proposals)?;

        let mut batch_profit = 0.0;
        let mut placed_any = false;
        for (index, share) in batch.iter().zip(shares) {
            let Some(amount) = stake(available, share * rule.kelly_fraction, rule.min_stake) else {
                continue;
            };
            let candidate = &candidates[*index];
            let profit = if candidate.won {
                amount.get() * (candidate.odds.get() - 1.0)
            } else {
                -amount.get()
            };
            batch_profit += profit;
            placed_any = true;
            bets.push(PlacedBet {
                candidate: *index,
                batch: batch_date,
                stake: amount,
                profit,
            });
        }
        if placed_any {
            bankroll += batch_profit;
            path.push((batch_date, bankroll));
        }
    }
    Ok(BacktestResult {
        bets,
        path,
        initial: rule.initial_bankroll.get(),
        final_bankroll: bankroll,
    })
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn candidate(
        game_id: i64,
        date: NaiveDate,
        probability: f64,
        odds: f64,
        won: bool,
    ) -> Candidate {
        Candidate {
            game_id,
            date,
            block: 0,
            outcome: Outcome::Home,
            odds: Odds::new(odds).unwrap(),
            probability: Probability::new(probability).unwrap(),
            close_fair: None,
            won,
        }
    }

    fn rule(fraction: f64) -> StakingRule {
        StakingRule {
            kelly_fraction: fraction,
            initial_bankroll: Euros::new(1000.0).unwrap(),
            min_stake: Euros::new(1.0).unwrap(),
        }
    }

    #[test]
    fn decision_dates_follow_the_friday_and_tuesday_collection_rhythm() {
        // Semaine du lundi 2025-08-11 au dimanche 2025-08-17, puis le lundi suivant.
        let expected = [
            (day(2025, 8, 11), day(2025, 8, 8)), // lundi → vendredi précédent
            (day(2025, 8, 12), day(2025, 8, 12)), // mardi
            (day(2025, 8, 13), day(2025, 8, 12)), // mercredi → mardi
            (day(2025, 8, 14), day(2025, 8, 12)), // jeudi → mardi
            (day(2025, 8, 15), day(2025, 8, 15)), // vendredi
            (day(2025, 8, 16), day(2025, 8, 15)), // samedi → vendredi
            (day(2025, 8, 17), day(2025, 8, 15)), // dimanche → vendredi
        ];
        for (match_date, decision) in expected {
            assert_eq!(decision_date(match_date), decision, "{match_date}");
        }
    }

    #[test]
    fn a_single_winning_bet_is_settled_by_hand() {
        // p = 0,6, cote 2,0 → Kelly 0,2 → 200 € sur 1 000 € ; gagné : +200 €.
        let result = run(
            &[candidate(1, day(2025, 8, 16), 0.6, 2.0, true)],
            &rule(1.0),
        )
        .unwrap();

        assert_eq!(result.bets.len(), 1);
        assert_abs_diff_eq!(result.bets[0].stake.get(), 200.0, epsilon = 0.05);
        assert_abs_diff_eq!(result.final_bankroll, 1200.0, epsilon = 0.1);
        assert_eq!(result.path.len(), 1);
    }

    #[test]
    fn a_single_losing_bet_loses_exactly_the_stake() {
        let result = run(
            &[candidate(1, day(2025, 8, 16), 0.6, 2.0, false)],
            &rule(1.0),
        )
        .unwrap();
        let stake = result.bets[0].stake.get();
        assert_abs_diff_eq!(result.final_bankroll, 1000.0 - stake, epsilon = 1e-9);
        assert_abs_diff_eq!(result.bets[0].profit, -stake, epsilon = 1e-9);
    }

    #[test]
    fn half_kelly_stakes_half_as_much() {
        let candidates = [candidate(1, day(2025, 8, 16), 0.6, 2.0, true)];
        let full = run(&candidates, &rule(1.0)).unwrap().bets[0].stake.get();
        let half = run(&candidates, &rule(0.5)).unwrap().bets[0].stake.get();
        assert_abs_diff_eq!(half, full / 2.0, epsilon = 0.11);
    }

    #[test]
    fn the_bankroll_compounds_from_one_batch_to_the_next() {
        // Deux lots : le samedi 16 (lot du vendredi 15) puis le samedi 23 (lot du vendredi 22).
        let candidates = [
            candidate(1, day(2025, 8, 16), 0.6, 2.0, true),
            candidate(2, day(2025, 8, 23), 0.6, 2.0, true),
        ];

        let result = run(&candidates, &rule(1.0)).unwrap();

        assert_eq!(result.path.len(), 2);
        // La seconde mise (20 % de la bankroll d'alors) est plus grosse que la première.
        assert!(result.bets[1].stake.get() > result.bets[0].stake.get() * 1.15);
        assert_abs_diff_eq!(result.path[1].1, result.final_bankroll, epsilon = 1e-9);
        assert_eq!(result.path[0].0, day(2025, 8, 15));
        assert_eq!(result.path[1].0, day(2025, 8, 22));
    }

    #[test]
    fn bets_of_one_batch_are_sized_together_and_never_exceed_the_bankroll() {
        // Dix matchs le même week-end, tous à forte valeur : la somme des mises doit rester sous la bankroll.
        let candidates: Vec<Candidate> = (0..10)
            .map(|i| candidate(i, day(2025, 8, 16), 0.62, 2.1, i % 2 == 0))
            .collect();

        let result = run(&candidates, &rule(1.0)).unwrap();

        let staked: f64 = result.bets.iter().map(|b| b.stake.get()).sum();
        assert_eq!(result.bets.len(), 10);
        assert!(staked < 1000.0, "{staked}");
        assert!(result.final_bankroll > 0.0);
    }

    #[test]
    fn a_bet_without_positive_expectation_is_never_placed() {
        let result = run(
            &[candidate(1, day(2025, 8, 16), 0.45, 2.0, true)],
            &rule(1.0),
        )
        .unwrap();
        assert!(result.bets.is_empty());
        assert_eq!(result.final_bankroll, 1000.0);
        assert!(result.path.is_empty());
    }

    #[test]
    fn bets_under_the_minimum_stake_are_ignored() {
        // Avantage minuscule : 0,0002 × 1 000 € = 0,20 € < mise minimale de 1 €.
        let result = run(
            &[candidate(1, day(2025, 8, 16), 0.5001, 2.0, true)],
            &rule(1.0),
        )
        .unwrap();
        assert!(result.bets.is_empty());
    }

    #[test]
    fn the_bankroll_can_never_go_negative_even_after_a_long_losing_streak() {
        let candidates: Vec<Candidate> = (0..40)
            .map(|i| {
                candidate(
                    i,
                    day(2025, 8, 16) + Days::new(7 * i as u64),
                    0.7,
                    2.0,
                    false,
                )
            })
            .collect();

        let result = run(&candidates, &rule(1.0)).unwrap();

        assert!(result.final_bankroll > 0.0);
        assert!(result.path.iter().all(|(_, bankroll)| *bankroll > 0.0));
    }

    #[test]
    fn the_result_does_not_depend_on_the_input_order() {
        let a = candidate(1, day(2025, 8, 16), 0.6, 2.0, true);
        let b = candidate(2, day(2025, 8, 23), 0.55, 2.2, false);
        let forward = run(&[a.clone(), b.clone()], &rule(1.0)).unwrap();
        let backward = run(&[b, a], &rule(1.0)).unwrap();
        assert_abs_diff_eq!(
            forward.final_bankroll,
            backward.final_bankroll,
            epsilon = 1e-9
        );
    }

    #[test]
    fn the_close_price_never_changes_the_stakes() {
        // Fuite de la clôture : modifier `close_fair` ne doit rien changer à ce qui est misé.
        let mut with_close = candidate(1, day(2025, 8, 16), 0.6, 2.0, true);
        let without_close = with_close.clone();
        with_close.close_fair = Probability::new(0.3);

        let a = run(&[with_close], &rule(1.0)).unwrap();
        let b = run(&[without_close], &rule(1.0)).unwrap();

        assert_eq!(a.bets[0].stake, b.bets[0].stake);
        assert_eq!(a.final_bankroll, b.final_bankroll);
    }
}
