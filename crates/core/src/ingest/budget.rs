//! Garde de budget : refuse tout appel qui ferait dépasser le plafond de crédits.
//!
//! C'est une garde de coût, pas de comportement : elle ne décide jamais *quoi* récupérer, seulement si
//! l'on a encore le droit de le payer. Elle se fonde sur `x-requests-used`, la valeur de l'API, et non
//! sur un compteur local ni sur le calendrier : la date de réinitialisation du quota gratuit n'est pas documentée.

use crate::error::{Error, Result};

/// Coût d'un appel `/odds` : un crédit par marché et par groupe de 10 bookmakers.
///
/// Règle de la documentation (« every group of 10 bookmakers is the equivalent of 1 region »),
/// vérifiée le 2026-10-18 pour 1 marché et 3 bookmakers (1 crédit) ; 11 bookmakers ou plus n'ont pas
/// été éprouvés. Voir `docs/sources.md`.
pub fn odds_cost(markets: usize, bookmakers: usize) -> u32 {
    let regions = bookmakers.div_ceil(10);
    // Une valeur démesurée est ramenée au maximum : la garde la refusera, ce qui est le bon réflexe.
    u32::try_from(markets * regions).unwrap_or(u32::MAX)
}

/// Autorise ou refuse un appel de coût `cost`, sachant `used` crédits déjà consommés et un plafond `cap`.
///
/// - quota inconnu (`used` à `None`) : refus, il faut d'abord le lire (appel gratuit à `/sports`) ;
/// - `used + cost` strictement supérieur au plafond : refus ; égal au plafond : accepté.
pub fn check(used: Option<u32>, cost: u32, cap: u32) -> Result<()> {
    let used = used.ok_or(Error::QuotaUnknown)?;
    // `saturating_add` : plafonne au lieu de déborder, donc ne peut jamais « boucler » vers un petit nombre.
    if used.saturating_add(cost) > cap {
        return Err(Error::BudgetExceeded { used, cost, cap });
    }
    Ok(())
}

/// `true` si le quota semble avoir été réinitialisé : la consommation lue est inférieure à la précédente.
pub fn reset_detected(previous: Option<u32>, current: u32) -> bool {
    previous.is_some_and(|before| current < before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_is_markets_times_groups_of_ten_bookmakers() {
        assert_eq!(odds_cost(1, 3), 1);
        assert_eq!(odds_cost(1, 6), 1);
        assert_eq!(odds_cost(1, 10), 1);
        assert_eq!(odds_cost(1, 11), 2);
        assert_eq!(odds_cost(2, 10), 2);
        assert_eq!(odds_cost(2, 11), 4);
    }

    #[test]
    fn call_within_the_cap_is_allowed_including_at_equality() {
        assert!(check(Some(100), 1, 450).is_ok());
        assert!(check(Some(449), 1, 450).is_ok());
    }

    #[test]
    fn call_over_the_cap_is_refused_with_the_numbers() {
        let err = check(Some(450), 1, 450).unwrap_err();
        assert!(matches!(
            err,
            Error::BudgetExceeded {
                used: 450,
                cost: 1,
                cap: 450
            }
        ));
        assert!(err.to_string().contains("450"));
    }

    #[test]
    fn unknown_quota_is_refused() {
        assert!(matches!(
            check(None, 1, 450).unwrap_err(),
            Error::QuotaUnknown
        ));
    }

    #[test]
    fn huge_cost_cannot_wrap_around() {
        assert!(check(Some(10), u32::MAX, 450).is_err());
    }

    #[test]
    fn reset_is_detected_only_when_usage_goes_down() {
        assert!(reset_detected(Some(430), 3));
        assert!(!reset_detected(Some(8), 8));
        assert!(!reset_detected(Some(8), 14));
        assert!(!reset_detected(None, 3));
    }
}
