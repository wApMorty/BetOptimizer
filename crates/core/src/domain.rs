//! Grandeurs et entités du domaine, typées pour que le compilateur interdise de les confondre.

use chrono::{DateTime, NaiveDate, Utc};

/// Montant en euros, positif ou nul. Newtype : un montant ne se confond pas avec une probabilité ou une cote.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Euros(f64);

impl Euros {
    /// Pas des mises : les opérateurs acceptent des mises au dixième d'euro.
    pub const STEP: f64 = 0.10;

    /// Crée un montant. Renvoie `None` s'il est négatif ou non fini.
    pub fn new(amount: f64) -> Option<Euros> {
        (amount.is_finite() && amount >= 0.0).then_some(Euros(amount))
    }

    /// Arrondit un montant **à la baisse** au dixième d'euro (jamais plus que ce que l'on voulait miser).
    /// Les mises sont arrondies par défaut pour que la somme des mises ne dépasse jamais la bankroll.
    pub fn floor_to_step(amount: f64) -> Option<Euros> {
        if !amount.is_finite() || amount < 0.0 {
            return None;
        }
        // Le petit epsilon évite qu'un 2,30 stocké comme 2,2999999 soit arrondi à 2,20.
        let tenths = (amount / Euros::STEP + 1e-9).floor();
        // `÷ 10` plutôt que `× 0,1` : 237 ÷ 10 donne exactement 23,7 ; 237 × 0,1 donne 23,700000000000003.
        Euros::new(tenths / 10.0)
    }

    /// Valeur en euros.
    pub fn get(self) -> f64 {
        self.0
    }
}

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

/// Probabilité : nombre de l'intervalle [0, 1].
///
/// Newtype distinct de [`Odds`] : le compilateur interdit de passer une cote là où l'on attend une
/// probabilité (confusion classique : 2,0 est une cote, 0,5 est la probabilité correspondante).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Probability(f64);

impl Probability {
    /// Crée une probabilité. Renvoie `None` hors de [0, 1] ou pour `NaN`.
    pub fn new(value: f64) -> Option<Probability> {
        // `contains` renvoie `false` pour NaN : il est donc refusé sans test supplémentaire.
        if (0.0..=1.0).contains(&value) {
            Some(Probability(value))
        } else {
            None
        }
    }

    /// Valeur numérique.
    pub fn get(self) -> f64 {
        self.0
    }
}

/// Issue d'un match de football, dans l'ordre ordinal victoire domicile < nul < victoire extérieur.
/// Cet ordre sert au RPS (voir `eval`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Victoire de l'équipe à domicile.
    Home,
    /// Match nul.
    Draw,
    /// Victoire de l'équipe à l'extérieur.
    Away,
}

impl Outcome {
    /// Les trois issues, dans l'ordre ordinal.
    pub const ALL: [Outcome; 3] = [Outcome::Home, Outcome::Draw, Outcome::Away];

    /// Position de l'issue (0, 1, 2) dans [`Outcome::ALL`].
    pub fn index(self) -> usize {
        match self {
            Outcome::Home => 0,
            Outcome::Draw => 1,
            Outcome::Away => 2,
        }
    }
}

/// Les trois cotes d'un marché 1X2 d'un même bookmaker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Odds1x2 {
    /// Victoire à domicile.
    pub home: Odds,
    /// Match nul.
    pub draw: Odds,
    /// Victoire à l'extérieur.
    pub away: Odds,
}

impl Odds1x2 {
    /// Cote de l'issue donnée.
    pub fn get(&self, outcome: Outcome) -> Odds {
        match outcome {
            Outcome::Home => self.home,
            Outcome::Draw => self.draw,
            Outcome::Away => self.away,
        }
    }

    /// Somme des probabilités implicites `1/cote`. Elle dépasse 1 de la marge du bookmaker.
    pub fn implied_sum(&self) -> f64 {
        1.0 / self.home.get() + 1.0 / self.draw.get() + 1.0 / self.away.get()
    }
}

/// Probabilités d'un 1X2 : trois probabilités dont la somme vaut 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probs1x2 {
    home: Probability,
    draw: Probability,
    away: Probability,
}

impl Probs1x2 {
    /// Écart toléré entre la somme des trois probabilités et 1 (erreurs d'arrondi des calculs).
    const SUM_TOLERANCE: f64 = 1e-9;

    /// Crée un 1X2. Renvoie `None` si une valeur n'est pas une probabilité ou si la somme n'est pas 1.
    pub fn new(home: f64, draw: f64, away: f64) -> Option<Probs1x2> {
        if (home + draw + away - 1.0).abs() > Self::SUM_TOLERANCE {
            return None;
        }
        Some(Probs1x2 {
            home: Probability::new(home)?,
            draw: Probability::new(draw)?,
            away: Probability::new(away)?,
        })
    }

    /// Probabilité de l'issue donnée.
    pub fn get(&self, outcome: Outcome) -> Probability {
        match outcome {
            Outcome::Home => self.home,
            Outcome::Draw => self.draw,
            Outcome::Away => self.away,
        }
    }

    /// Les trois probabilités sous forme de nombres, dans l'ordre [`Outcome::ALL`].
    pub fn to_array(&self) -> [f64; 3] {
        [self.home.get(), self.draw.get(), self.away.get()]
    }
}

/// Moment où un bookmaker a relevé ses cotes, dans les données historiques de football-data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Moment {
    /// Relevé d'avant-match : le vendredi après-midi (week-end) ou le mardi (semaine), donc 1 à 3 jours
    /// avant le coup d'envoi. C'est le moment où l'on pourrait réellement parier.
    Pre,
    /// Cotes de clôture, juste avant le coup d'envoi : la référence la plus fine du marché.
    Close,
}

impl Moment {
    /// Valeur stockée en base (`game_odds.moment`).
    pub fn as_str(self) -> &'static str {
        match self {
            Moment::Pre => "pre",
            Moment::Close => "close",
        }
    }
}

/// Les cotes 1X2 d'un bookmaker pour un match, à un moment donné.
#[derive(Debug, Clone, PartialEq)]
pub struct BookOdds {
    /// Préfixe de colonne football-data : `PS` (Pinnacle), `B365`, `Avg` (moyenne du marché), `Max`…
    pub bookmaker: String,
    /// Relevé d'avant-match ou clôture.
    pub moment: Moment,
    /// Cotes de l'issue domicile, nul, extérieur.
    pub odds: Odds1x2,
}

/// Cotes plus/moins de buts d'un bookmaker pour une ligne, à un moment donné.
#[derive(Debug, Clone, PartialEq)]
pub struct TotalsOdds {
    /// Préfixe de colonne football-data (`PS` pour Pinnacle, `Avg`, `B365`, `Max`).
    pub bookmaker: String,
    /// Relevé d'avant-match ou clôture.
    pub moment: Moment,
    /// Ligne du marché : 2,5 (football-data ne publie que celle-ci).
    pub line: f64,
    /// Cote « plus de `line` buts ».
    pub over: Odds,
    /// Cote « moins de `line` buts ».
    pub under: Odds,
}

/// Statistiques d'un match publiées par football-data. Chaque valeur peut manquer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MatchStats {
    /// Tirs de l'équipe à domicile.
    pub shots_home: Option<u16>,
    /// Tirs de l'équipe à l'extérieur.
    pub shots_away: Option<u16>,
    /// Tirs cadrés à domicile.
    pub shots_on_target_home: Option<u16>,
    /// Tirs cadrés à l'extérieur.
    pub shots_on_target_away: Option<u16>,
    /// Corners à domicile.
    pub corners_home: Option<u16>,
    /// Corners à l'extérieur.
    pub corners_away: Option<u16>,
    /// Fautes à domicile.
    pub fouls_home: Option<u16>,
    /// Fautes à l'extérieur.
    pub fouls_away: Option<u16>,
    /// Cartons jaunes à domicile.
    pub yellow_home: Option<u16>,
    /// Cartons jaunes à l'extérieur.
    pub yellow_away: Option<u16>,
    /// Cartons rouges à domicile.
    pub red_home: Option<u16>,
    /// Cartons rouges à l'extérieur.
    pub red_away: Option<u16>,
}

/// Un match terminé tel que publié par football-data, noms d'équipes bruts.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchRecord {
    /// Date du match (date locale publiée).
    pub date: NaiveDate,
    /// Coup d'envoi en UTC ; `None` quand l'heure n'est pas publiée (saisons 2016/17 à 2018/19).
    pub kickoff_utc: Option<DateTime<Utc>>,
    /// Équipe à domicile, nom brut.
    pub home_team: String,
    /// Équipe à l'extérieur, nom brut.
    pub away_team: String,
    /// Buts à domicile à la fin du match.
    pub ft_home: u8,
    /// Buts à l'extérieur à la fin du match.
    pub ft_away: u8,
    /// Résultat tel que publié (contrôlé contre les buts par la qualité des données).
    pub ft_result: Outcome,
    /// Buts à domicile à la mi-temps.
    pub ht_home: Option<u8>,
    /// Buts à l'extérieur à la mi-temps.
    pub ht_away: Option<u8>,
    /// Arbitre.
    pub referee: Option<String>,
    /// Tirs, corners, fautes, cartons.
    pub stats: MatchStats,
    /// xG de l'équipe à domicile (publié depuis 2026/27 seulement).
    pub xg_home: Option<f64>,
    /// xG de l'équipe à l'extérieur (publié depuis 2026/27 seulement).
    pub xg_away: Option<f64>,
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
    fn euros_reject_negative_and_non_finite_amounts() {
        assert!(Euros::new(0.0).is_some());
        assert!(Euros::new(12.5).is_some());
        for amount in [-0.01, f64::NAN, f64::INFINITY] {
            assert!(Euros::new(amount).is_none(), "{amount} doit être refusé");
        }
    }

    #[test]
    fn stakes_are_floored_to_the_tenth_of_a_euro() {
        let floor = |x: f64| Euros::floor_to_step(x).unwrap().get();
        assert_eq!(floor(2.34), 2.3);
        assert_eq!(floor(2.39999), 2.3);
        assert_eq!(floor(2.0), 2.0);
        assert_eq!(floor(0.09), 0.0);
        // 2,30 représenté comme 2,2999999… ne doit pas perdre un dixième.
        assert_eq!(floor(0.1 + 0.2 + 2.0), 2.3);
        assert!(Euros::floor_to_step(-1.0).is_none());
    }

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
    fn probability_accepts_only_the_unit_interval() {
        assert!(Probability::new(0.0).is_some());
        assert!(Probability::new(1.0).is_some());
        for value in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
            assert!(
                Probability::new(value).is_none(),
                "{value} doit être refusée"
            );
        }
    }

    #[test]
    fn probs_1x2_must_sum_to_one() {
        assert!(Probs1x2::new(0.5, 0.3, 0.2).is_some());
        assert!(Probs1x2::new(0.5, 0.3, 0.3).is_none());
        assert!(Probs1x2::new(1.2, -0.1, -0.1).is_none());
    }

    #[test]
    fn implied_sum_exceeds_one_by_the_margin() {
        let odds = Odds1x2 {
            home: Odds::new(2.0).unwrap(),
            draw: Odds::new(4.0).unwrap(),
            away: Odds::new(4.0).unwrap(),
        };
        assert!((odds.implied_sum() - 1.0).abs() < 1e-12);
        assert_eq!(odds.get(Outcome::Draw).get(), 4.0);
    }

    #[test]
    fn outcome_indices_follow_the_ordinal_order() {
        let indices: Vec<usize> = Outcome::ALL.iter().map(|o| o.index()).collect();
        assert_eq!(indices, [0, 1, 2]);
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
