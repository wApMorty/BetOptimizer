//! Grandeurs et entités du domaine, typées pour que le compilateur interdise de les confondre.

use chrono::{DateTime, Utc};

/// Cote décimale d'un bookmaker : gain total pour 1 € misé, mise comprise (2,10 → 2,10 € rendus).
///
/// Newtype *validé* : le champ est privé, donc la seule façon d'obtenir une `Odds` est [`Odds::new`],
/// qui refuse les valeurs impossibles. Une fonction qui reçoit une `Odds` n'a jamais à revérifier.
/// `Copy` : le type pèse 8 octets, on le passe par valeur sans emprunt ni `clone()`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Odds(f64);

impl Odds {
    /// Crée une cote. Renvoie `None` si la valeur n'est pas un nombre fini strictement supérieur à 1
    /// (une cote de 1,00 ne rapporte rien ; en dessous, c'est une valeur erronée).
    pub fn new(value: f64) -> Option<Odds> {
        if value.is_finite() && value > 1.0 {
            Some(Odds(value))
        } else {
            None
        }
    }

    /// Valeur décimale de la cote.
    pub fn get(self) -> f64 {
        self.0
    }
}

/// Compteurs de crédits lus dans les en-têtes d'une réponse de l'API.
///
/// Chaque champ est un `Option` : les réponses 401 (clé invalide) n'ont aucun de ces en-têtes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quota {
    /// Coût du dernier appel (`x-requests-last`).
    pub cost: Option<u32>,
    /// Crédits consommés depuis la dernière réinitialisation (`x-requests-used`).
    pub used: Option<u32>,
    /// Crédits restants (`x-requests-remaining`).
    pub remaining: Option<u32>,
}

impl Quota {
    /// Construit le quota à partir des valeurs textuelles des trois en-têtes.
    /// Une valeur absente ou qui n'est pas un entier devient `None` : on ne devine rien.
    pub fn from_header_values(
        cost: Option<&str>,
        used: Option<&str>,
        remaining: Option<&str>,
    ) -> Quota {
        // `and_then` enchaîne deux opérations qui peuvent échouer (absent, puis non numérique).
        let number = |value: Option<&str>| value.and_then(|text| text.trim().parse().ok());
        Quota {
            cost: number(cost),
            used: number(used),
            remaining: number(remaining),
        }
    }
}

/// Une cote d'un bookmaker pour une issue d'un marché, telle que relevée.
#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    /// Clé du bookmaker (`winamax_fr`).
    pub bookmaker_key: String,
    /// Clé du marché (`h2h`).
    pub market_key: String,
    /// Dernière mise à jour du marché chez l'API (UTC).
    pub updated_at: DateTime<Utc>,
    /// Nom brut de l'issue : le nom d'une équipe, ou `Draw`.
    pub outcome: String,
    /// Ligne du marché (totaux, écarts) ; `None` en 1X2.
    pub point: Option<f64>,
    /// Cote décimale.
    pub price: Odds,
}

/// Un match vu dans une réponse, avec toutes ses cotes relevées.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Identifiant The Odds API.
    pub id: String,
    /// Clé de la compétition (`soccer_epl`).
    pub sport_key: String,
    /// Heure du coup d'envoi (UTC).
    pub commence_time: DateTime<Utc>,
    /// Équipe à domicile, nom brut.
    pub home_team: String,
    /// Équipe à l'extérieur, nom brut.
    pub away_team: String,
    /// Cotes de tous les bookmakers et marchés relevés.
    pub quotes: Vec<Quote>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_odds_are_accepted() {
        assert_eq!(Odds::new(1.01).unwrap().get(), 1.01);
        assert_eq!(Odds::new(11.44).unwrap().get(), 11.44);
    }

    #[test]
    fn impossible_values_are_refused() {
        for value in [1.0, 0.99, 0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert!(Odds::new(value).is_none(), "{value} doit être refusée");
        }
    }

    #[test]
    fn quota_reads_the_three_headers() {
        let quota = Quota::from_header_values(Some("1"), Some("8"), Some("492"));
        assert_eq!(
            quota,
            Quota {
                cost: Some(1),
                used: Some(8),
                remaining: Some(492)
            }
        );
    }

    #[test]
    fn missing_or_garbled_headers_become_none() {
        let quota = Quota::from_header_values(None, Some("abc"), Some(" 7 "));
        assert_eq!(
            quota,
            Quota {
                cost: None,
                used: None,
                remaining: Some(7)
            }
        );
    }
}
