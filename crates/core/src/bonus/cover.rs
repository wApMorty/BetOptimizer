//! Mises de couverture d'un bonus : le calcul pur, sans base ni archive.
//!
//! On mise `A` (le bonus) sur une issue à la cote `o`, puis une couverture `c_i` en argent sur chaque autre issue `i`, à la
//! cote `o_i`, de sorte que le gain net soit le même quelle que soit l'issue.

use super::OfferKind;
use crate::domain::{Euros, Odds};
use crate::error::{Error, Result};

/// Résultat d'un calcul de couverture, mises arrondies au dixième d'euro.
#[derive(Debug, Clone, PartialEq)]
pub struct Cover {
    /// Mise de couverture sur chaque autre issue, dans l'ordre des cotes reçues.
    pub stakes: Vec<Euros>,
    /// Gain net de l'opération si l'issue du bonus passe.
    pub if_bonus_wins: f64,
    /// Gain net si l'autre issue `i` passe, dans le même ordre que `stakes`.
    pub if_other_wins: Vec<f64>,
    /// Le plus petit des gains nets : ce que l'opération rapporte au pire, mises arrondies comprises.
    pub guaranteed: f64,
    /// Argent réellement sorti : la mise du bonus si elle est en argent, plus les couvertures. Il revient après le match.
    pub cash_out: f64,
}

/// Calcule les mises de couverture d'un bonus de montant `amount` joué à la cote `odds`, couvert aux cotes `others`.
///
/// Forme mathématique (calcul élémentaire, `docs/bonus-bienvenue.md` §4). Gain du pari bonus s'il passe : `W = A·(o − 1)`.
/// Valeur `L` du pari bonus s'il perd :
/// - remboursé en argent : `L = 0` (la mise revient) ;
/// - remboursé en freebet de valeur `g` par euro : `L = −A + g·A = −A·(1 − g)` ;
/// - freebet : `L = 0` (rien n'était misé en argent).
///
/// En égalisant le gain `W − Σc` (le bonus passe) et `L + c_i·o_i − Σc` (l'issue `i` passe) :
/// `c_i = (W − L) / o_i`, et le gain garanti avant arrondi vaut `W − (W − L)·Σ 1/o_i`.
///
/// `freebet_value` (`g`) n'est lu que pour un premier pari remboursé en freebet : le gain n'est alors garanti que si le
/// freebet reçu est ensuite converti à cette valeur au moins.
pub fn cover(
    kind: OfferKind,
    amount: Euros,
    odds: Odds,
    freebet_value: f64,
    others: &[Odds],
) -> Result<Cover> {
    if amount.get() <= 0.0 {
        return Err(Error::Input("le montant du bonus doit être positif".into()));
    }
    if others.is_empty() {
        return Err(Error::Input(
            "il faut au moins une cote de couverture (les autres issues du match)".into(),
        ));
    }
    if kind == OfferKind::FreebetRefund && !(freebet_value > 0.0 && freebet_value < 1.0) {
        return Err(Error::Input(format!(
            "la valeur d'un euro de freebet doit être dans ]0 ; 1[ (reçu {freebet_value})"
        )));
    }
    let a = amount.get();
    let win = a * (odds.get() - 1.0);
    let lose = match kind {
        OfferKind::CashRefund | OfferKind::Freebet => 0.0,
        OfferKind::FreebetRefund => -a * (1.0 - freebet_value),
    };

    let stakes: Vec<Euros> = others
        .iter()
        .map(|o| {
            Euros::round_to_step((win - lose) / o.get())
                .expect("W − L est positif et fini : W ≥ 0 et L ≤ 0")
        })
        .collect();
    let total: f64 = stakes.iter().map(|s| s.get()).sum();

    // Les gains se recalculent sur les mises arrondies : c'est ce que Paul touchera vraiment.
    let if_bonus_wins = win - total;
    let if_other_wins: Vec<f64> = stakes
        .iter()
        .zip(others)
        .map(|(stake, o)| lose + stake.get() * o.get() - total)
        .collect();
    let guaranteed = if_other_wins.iter().copied().fold(if_bonus_wins, f64::min);
    let cash_out = total + if kind.bonus_stake_is_cash() { a } else { 0.0 };
    Ok(Cover {
        stakes,
        if_bonus_wins,
        if_other_wins,
        guaranteed,
        cash_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    fn odds(value: f64) -> Odds {
        Odds::new(value).unwrap()
    }

    fn euros(value: f64) -> Euros {
        Euros::new(value).unwrap()
    }

    /// Écart maximal entre deux issues dû à l'arrondi : 0,05 € par mise, multiplié par la plus grosse cote.
    fn rounding_slack(others: &[Odds]) -> f64 {
        0.05 * others.iter().map(|o| o.get()).fold(1.0, f64::max) * others.len() as f64
    }

    #[test]
    fn a_covered_freebet_pays_the_same_whatever_the_outcome() {
        let others = [odds(1.85), odds(3.90)];
        let result = cover(OfferKind::Freebet, euros(100.0), odds(4.2), 0.0, &others).unwrap();
        // Avant arrondi : c_i = 320 / o_i ; garanti = 320·(1 − 1/1,85 − 1/3,90) = 64,92 €.
        assert_abs_diff_eq!(result.stakes[0].get(), 173.0);
        assert_abs_diff_eq!(result.stakes[1].get(), 82.1);
        for gain in &result.if_other_wins {
            assert_abs_diff_eq!(
                *gain,
                result.if_bonus_wins,
                epsilon = rounding_slack(&others)
            );
        }
        assert_abs_diff_eq!(result.guaranteed, 64.9, epsilon = 0.3);
        // Le freebet ne coûte rien en argent : seules les couvertures sortent.
        assert_abs_diff_eq!(result.cash_out, 173.0 + 82.1);
    }

    #[test]
    fn a_lost_freebet_does_not_cost_its_stake() {
        let others = [odds(1.5)];
        let result = cover(OfferKind::Freebet, euros(10.0), odds(3.0), 0.0, &others).unwrap();
        // Si l'issue du freebet perd, la couverture (20 / 1,5 = 13,3 €) gagne 13,3 × 0,5 = 6,65 € ; le freebet perdu ne
        // retire rien. Si le freebet passe : 20 − 13,3 = 6,7 €.
        assert_abs_diff_eq!(result.if_other_wins[0], 13.3 * 1.5 - 13.3, epsilon = 1e-9);
        assert_abs_diff_eq!(result.if_bonus_wins, 20.0 - 13.3, epsilon = 1e-9);
    }

    #[test]
    fn a_cash_refund_is_covered_like_a_freebet_but_costs_its_stake_in_cash() {
        let others = [odds(1.85), odds(3.90)];
        let freebet = cover(OfferKind::Freebet, euros(100.0), odds(4.2), 0.0, &others).unwrap();
        let refund = cover(OfferKind::CashRefund, euros(100.0), odds(4.2), 0.0, &others).unwrap();
        assert_eq!(freebet.stakes, refund.stakes);
        assert_abs_diff_eq!(freebet.guaranteed, refund.guaranteed);
        assert_abs_diff_eq!(refund.cash_out, freebet.cash_out + 100.0);
    }

    #[test]
    fn a_freebet_refund_matches_the_closed_form() {
        let others = [odds(2.10), odds(3.40)];
        let g = 0.55;
        let result = cover(
            OfferKind::FreebetRefund,
            euros(100.0),
            odds(3.1),
            g,
            &others,
        )
        .unwrap();
        // Forme fermée : A·[(o − 1) − (o − g)·Σ 1/o_i] = 100·[2,1 − 2,55·(1/2,1 + 1/3,4)] = 13,57 €.
        let closed = 100.0 * (2.1 - 2.55 * (1.0 / 2.1 + 1.0 / 3.4));
        assert_abs_diff_eq!(result.guaranteed, closed, epsilon = rounding_slack(&others));
        for gain in &result.if_other_wins {
            assert_abs_diff_eq!(
                *gain,
                result.if_bonus_wins,
                epsilon = rounding_slack(&others)
            );
        }
    }

    #[test]
    fn a_fair_two_way_market_converts_a_freebet_at_one_minus_the_implied_probability() {
        // Marché juste à deux issues (cotes 2 et 2) : garanti = F·(o − 1)·(1 − 1/2) = F/2.
        let result = cover(
            OfferKind::Freebet,
            euros(50.0),
            odds(2.0),
            0.0,
            &[odds(2.0)],
        )
        .unwrap();
        assert_abs_diff_eq!(result.guaranteed, 25.0, epsilon = 1e-9);
    }

    #[test]
    fn impossible_inputs_are_refused() {
        let others = [odds(2.0)];
        assert!(cover(OfferKind::Freebet, euros(0.0), odds(2.0), 0.0, &others).is_err());
        assert!(cover(OfferKind::Freebet, euros(10.0), odds(2.0), 0.0, &[]).is_err());
        assert!(
            cover(
                OfferKind::FreebetRefund,
                euros(10.0),
                odds(2.0),
                1.0,
                &others
            )
            .is_err()
        );
        assert!(
            cover(
                OfferKind::FreebetRefund,
                euros(10.0),
                odds(2.0),
                f64::NAN,
                &others
            )
            .is_err()
        );
        // La valeur du freebet n'est pas lue pour les autres formes.
        assert!(cover(OfferKind::CashRefund, euros(10.0), odds(2.0), 7.0, &others).is_ok());
    }
}
