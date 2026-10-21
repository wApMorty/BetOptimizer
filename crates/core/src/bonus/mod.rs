//! Bonus de bienvenue couverts : calcul des mises de couverture, choix du match sur le dernier relevé, suivi des offres.
//!
//! La stratégie et ses mesures sont dans `docs/bonus-bienvenue.md` : un bonus se joue en une *opération* (le pari bonus et
//! une couverture en argent sur chaque autre issue, chez d'autres opérateurs), de sorte que le gain soit le même quel que
//! soit le résultat. Rien ici n'est un modèle : la valeur vient de l'offre de l'opérateur, pas d'une prévision.

pub mod cover;
pub mod pick;
pub mod tracking;

use crate::error::{Error, Result};

/// Valeur par défaut d'un euro de freebet converti par couverture : 0,53 à 0,56 mesuré sur l'archive (meilleur match du
/// jour, `docs/bonus-bienvenue.md` §4). On prend le bas de la fourchette, pour ne pas surestimer le gain d'un premier pari
/// remboursé en freebet. À remplacer par la valeur réellement obtenue quand quelques freebets auront été joués.
pub const DEFAULT_FREEBET_VALUE: f64 = 0.53;

/// Nombre d'opérations proposées par opérateur.
pub const PICKS_PER_BOOKMAKER: usize = 5;

/// Forme d'une offre, ou d'une opération.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferKind {
    /// Premier pari remboursé en argent s'il est perdu.
    CashRefund,
    /// Premier pari remboursé en freebet s'il est perdu.
    FreebetRefund,
    /// Freebet : la mise est offerte, mais n'est pas rendue si le pari gagne.
    Freebet,
}

impl OfferKind {
    /// Les trois formes, dans l'ordre d'affichage.
    pub const ALL: [OfferKind; 3] = [
        OfferKind::CashRefund,
        OfferKind::FreebetRefund,
        OfferKind::Freebet,
    ];

    /// Texte stocké en base et accepté par la CLI.
    pub fn as_str(self) -> &'static str {
        match self {
            OfferKind::CashRefund => "cash_refund",
            OfferKind::FreebetRefund => "freebet_refund",
            OfferKind::Freebet => "freebet",
        }
    }

    /// Relit une forme stockée en base.
    pub fn parse(text: &str) -> Result<OfferKind> {
        OfferKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text)
            .ok_or_else(|| Error::Data(format!("forme d'offre inconnue : {text}")))
    }

    /// Nom affiché.
    pub fn label(self) -> &'static str {
        match self {
            OfferKind::CashRefund => "remboursé en argent",
            OfferKind::FreebetRefund => "remboursé en freebet",
            OfferKind::Freebet => "freebet",
        }
    }

    /// Vrai si le pari bonus est payé en argent (il l'est sauf pour un freebet).
    pub fn bonus_stake_is_cash(self) -> bool {
        self != OfferKind::Freebet
    }
}

/// Un opérateur agréé par l'ANJ : clé et nom affiché.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operator {
    /// Clé : celle de The Odds API quand elle existe, sinon une clé choisie sur le même modèle.
    pub key: &'static str,
    /// Nom affiché.
    pub name: &'static str,
    /// Vrai si The Odds API publie ses cotes (région `fr`) : seuls ceux-là ont des prix dans l'archive.
    pub in_archive: bool,
}

/// Opérateurs de paris sportifs agréés par l'ANJ (liste lue sur anj.fr le 2026-10-20, `docs/bonus-bienvenue.md` §7).
/// Parions Sport et ZEbet n'y figuraient pas ; ils restent listés, à revérifier avant de les écarter.
pub const OPERATORS: &[Operator] = &[
    Operator {
        key: "betclic_fr",
        name: "Betclic",
        in_archive: true,
    },
    Operator {
        key: "unibet_fr",
        name: "Unibet",
        in_archive: true,
    },
    Operator {
        key: "pmu_fr",
        name: "PMU",
        in_archive: true,
    },
    Operator {
        key: "winamax_fr",
        name: "Winamax",
        in_archive: true,
    },
    Operator {
        key: "netbet_fr",
        name: "NetBet",
        in_archive: true,
    },
    Operator {
        key: "bet365_fr",
        name: "bet365",
        in_archive: false,
    },
    Operator {
        key: "betsson_fr",
        name: "Betsson",
        in_archive: false,
    },
    Operator {
        key: "bwin_fr",
        name: "bwin",
        in_archive: false,
    },
    Operator {
        key: "pokerstars_fr",
        name: "PokerStars Sports",
        in_archive: false,
    },
    Operator {
        key: "genybet_fr",
        name: "Genybet",
        in_archive: false,
    },
    Operator {
        key: "vbet_fr",
        name: "Vbet",
        in_archive: false,
    },
    Operator {
        key: "feelingbet_fr",
        name: "Feelingbet",
        in_archive: false,
    },
    Operator {
        key: "olybet_fr",
        name: "Olybet",
        in_archive: false,
    },
    Operator {
        key: "circus_fr",
        name: "Circus",
        in_archive: false,
    },
    Operator {
        key: "daznbet_fr",
        name: "DAZN Bet",
        in_archive: false,
    },
    Operator {
        key: "yesorno_fr",
        name: "Yesorno",
        in_archive: false,
    },
    Operator {
        key: "zeturf_fr",
        name: "ZEturf",
        in_archive: false,
    },
    Operator {
        key: "parionssport_fr",
        name: "Parions Sport",
        in_archive: false,
    },
    Operator {
        key: "zebet_fr",
        name: "ZEbet",
        in_archive: false,
    },
];

/// Nom affiché d'un opérateur ; la clé elle-même s'il n'est pas dans [`OPERATORS`].
pub fn operator_name(key: &str) -> &str {
    OPERATORS
        .iter()
        .find(|operator| operator.key == key)
        .map_or(key, |operator| operator.name)
}

/// Vrai si `key` a la forme d'une clé d'opérateur : minuscules, chiffres et `_`, comme celles de The Odds API.
pub fn is_operator_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_kinds_round_trip_through_their_stored_text() {
        for kind in OfferKind::ALL {
            assert_eq!(OfferKind::parse(kind.as_str()).unwrap(), kind);
        }
        assert!(OfferKind::parse("double").is_err());
    }

    #[test]
    fn operator_keys_are_unique_and_well_formed() {
        for (index, operator) in OPERATORS.iter().enumerate() {
            assert!(is_operator_key(operator.key), "{}", operator.key);
            assert!(
                OPERATORS[index + 1..].iter().all(|o| o.key != operator.key),
                "{} en double",
                operator.key
            );
        }
        assert_eq!(operator_name("betclic_fr"), "Betclic");
        assert_eq!(operator_name("inconnu_fr"), "inconnu_fr");
        assert!(!is_operator_key("Betclic"));
        assert!(!is_operator_key(""));
    }
}
