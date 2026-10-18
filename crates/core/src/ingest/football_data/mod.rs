//! Données historiques de football-data.co.uk : téléchargement, lecture des CSV et import en base.

pub mod fetch;
pub mod import;
pub mod parse;

use chrono::{Datelike, NaiveDate};

/// Racine des fichiers CSV.
pub const BASE_URL: &str = "https://football-data.co.uk/mmz4281";

/// Un championnat suivi, avec son code football-data et la clé correspondante de The Odds API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct League {
    /// Code football-data (`E0`…), utilisé dans le nom des fichiers et la colonne `Div`.
    pub div: &'static str,
    /// Clé de The Odds API (`soccer_epl`…).
    pub sport_key: &'static str,
    /// Nom lisible.
    pub name: &'static str,
}

/// Les six championnats du projet.
pub const LEAGUES: [League; 6] = [
    League {
        div: "E0",
        sport_key: "soccer_epl",
        name: "Premier League",
    },
    League {
        div: "SP1",
        sport_key: "soccer_spain_la_liga",
        name: "La Liga",
    },
    League {
        div: "D1",
        sport_key: "soccer_germany_bundesliga",
        name: "Bundesliga",
    },
    League {
        div: "I1",
        sport_key: "soccer_italy_serie_a",
        name: "Serie A",
    },
    League {
        div: "F1",
        sport_key: "soccer_france_ligue_one",
        name: "Ligue 1",
    },
    League {
        div: "SC0",
        sport_key: "soccer_spl",
        name: "Premiership écossaise",
    },
];

/// Une saison, identifiée par l'année où elle commence (2025 pour 2025/26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Season {
    start_year: u16,
}

impl Season {
    /// Saison qui commence en `start_year`.
    pub fn new(start_year: u16) -> Season {
        Season { start_year }
    }

    /// Saison qui contient une date : de juillet à décembre, celle qui commence cette année ; sinon la précédente.
    pub fn containing(date: NaiveDate) -> Season {
        let year = u16::try_from(date.year()).unwrap_or(0);
        if date.month() >= 7 {
            Season::new(year)
        } else {
            Season::new(year.saturating_sub(1))
        }
    }

    /// Année de début.
    pub fn start_year(self) -> u16 {
        self.start_year
    }

    /// Code des noms de fichiers football-data : `2526` pour 2025/26.
    pub fn code(self) -> String {
        format!(
            "{:02}{:02}",
            self.start_year % 100,
            (self.start_year + 1) % 100
        )
    }

    /// Libellé : `2025/26`.
    pub fn label(self) -> String {
        format!("{}/{:02}", self.start_year, (self.start_year + 1) % 100)
    }

    /// Adresse du fichier CSV d'une ligue pour cette saison.
    pub fn url(self, league: &League) -> String {
        format!("{BASE_URL}/{}/{}.csv", self.code(), league.div)
    }
}

/// Toutes les saisons de `first` à `current` incluses.
pub fn seasons_between(first: Season, current: Season) -> Vec<Season> {
    (first.start_year..=current.start_year)
        .map(Season::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn season_codes_labels_and_urls() {
        let season = Season::new(2025);
        assert_eq!(season.code(), "2526");
        assert_eq!(season.label(), "2025/26");
        assert_eq!(
            season.url(&LEAGUES[0]),
            "https://football-data.co.uk/mmz4281/2526/E0.csv"
        );
        assert_eq!(Season::new(1999).code(), "9900");
        assert_eq!(Season::new(2009).code(), "0910");
    }

    #[test]
    fn season_containing_a_date_switches_in_july() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(Season::containing(day(2026, 10, 18)), Season::new(2026));
        assert_eq!(Season::containing(day(2026, 6, 30)), Season::new(2025));
        assert_eq!(Season::containing(day(2026, 7, 1)), Season::new(2026));
        assert_eq!(Season::containing(day(2027, 1, 5)), Season::new(2026));
    }

    #[test]
    fn seasons_between_is_inclusive() {
        let seasons = seasons_between(Season::new(2016), Season::new(2026));
        assert_eq!(seasons.len(), 11);
        assert_eq!(seasons[0].label(), "2016/17");
        assert_eq!(seasons[10].label(), "2026/27");
    }
}
