//! Jeux de tirage de la FDJ (phase 16) : Grand Loto à jackpot forcé et alerte EuroMillions au plafond.
//!
//! Le Grand Loto verse toujours son jackpot, au premier rang ou aux rangs inférieurs : son espérance par grille dépend
//! surtout du nombre de grilles jouées ([`estimate`], [`expectation`]), et un peu des numéros choisis par le public
//! ([`popularity`]). L'EuroMillions verse son jackpot au rang 2 au cinquième tirage au plafond ([`alert`]). Les données
//! viennent de l'historique officiel de la FDJ ([`fdj`]). Études et choix : `docs/sources.md` §43 et §44.

pub mod alert;
pub mod estimate;
pub mod expectation;
pub mod fdj;
pub mod played;
pub mod popularity;
pub mod recommend;
pub mod rules;

use std::fmt;

use chrono::NaiveDate;

/// Jeu de tirage suivi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Game {
    /// Loto ordinaire (trois tirages par semaine) : sert à mesurer les préférences du public.
    Loto,
    /// Grand Loto : tirage exceptionnel à jackpot forcé, deux ou trois fois par an.
    GrandLoto,
    /// EuroMillions : sert seulement à l'alerte du cinquième tirage au plafond.
    EuroMillions,
}

impl Game {
    /// Code court, en base : `loto`, `grand_loto`, `euromillions`.
    pub fn code(self) -> &'static str {
        match self {
            Game::Loto => "loto",
            Game::GrandLoto => "grand_loto",
            Game::EuroMillions => "euromillions",
        }
    }

    /// Jeu correspondant à un code de la base.
    pub fn from_code(code: &str) -> Option<Game> {
        match code {
            "loto" => Some(Game::Loto),
            "grand_loto" => Some(Game::GrandLoto),
            "euromillions" => Some(Game::EuroMillions),
            _ => None,
        }
    }

    /// Nombre de rangs de gains publiés par la FDJ (9 au Loto, 13 à l'EuroMillions).
    pub fn rank_count(self) -> usize {
        match self {
            Game::Loto | Game::GrandLoto => 9,
            Game::EuroMillions => 13,
        }
    }
}

impl fmt::Display for Game {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Game::Loto => "Loto",
            Game::GrandLoto => "Grand Loto",
            Game::EuroMillions => "EuroMillions",
        };
        f.write_str(label)
    }
}

/// Résultat officiel d'un rang : nombre de gagnants et gain unitaire, en euros.
///
/// À l'EuroMillions, les gagnants sont ceux de toute l'Europe (le gain unitaire est commun aux pays participants).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankResult {
    /// Nombre de grilles gagnantes à ce rang.
    pub winners: u64,
    /// Gain d'une grille gagnante, en euros (0 si personne n'a gagné).
    pub unit_gain: f64,
}

/// Codes gagnants d'un tirage Loto (tirage au sort parmi les grilles jouées, cumulable avec les rangs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Codes {
    /// Nombre de codes tirés.
    pub count: u32,
    /// Gain de chaque code, en euros.
    pub gain: f64,
}

/// Un tirage, tel que publié dans l'historique de la FDJ.
#[derive(Debug, Clone, PartialEq)]
pub struct Draw {
    /// Jeu.
    pub game: Game,
    /// Date du tirage (heure de Paris : les tirages ont lieu le soir, la date ne change pas en UTC).
    pub date: NaiveDate,
    /// Les cinq boules tirées, dans l'ordre croissant.
    pub balls: [u8; 5],
    /// Numéro chance (Loto, un élément) ou étoiles (EuroMillions, deux éléments, dans l'ordre croissant).
    pub extra: Vec<u8>,
    /// Rangs dans l'ordre : `ranks[0]` est le rang 1.
    pub ranks: Vec<RankResult>,
    /// Codes gagnants (Loto et Grand Loto seulement).
    pub codes: Option<Codes>,
}

impl Draw {
    /// Montant total versé aux rangs (sans les codes), en euros.
    pub fn paid_to_ranks(&self) -> f64 {
        self.ranks
            .iter()
            .map(|r| r.winners as f64 * r.unit_gain)
            .sum()
    }

    /// Montant versé aux codes gagnants, en euros (0 sans codes).
    pub fn paid_to_codes(&self) -> f64 {
        self.codes
            .map(|c| f64::from(c.count) * c.gain)
            .unwrap_or(0.0)
    }
}

/// Grille de Loto ou de Grand Loto : cinq numéros de 1 à 49 et un numéro chance de 1 à 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Grid {
    /// Les cinq numéros, dans l'ordre croissant.
    pub balls: [u8; 5],
    /// Le numéro chance.
    pub chance: u8,
}

impl Grid {
    /// Crée une grille. Renvoie `None` si un numéro est hors de 1 à 49, s'il est répété, ou si la chance est hors de
    /// 1 à 10. Les numéros sont rangés dans l'ordre croissant.
    pub fn new(mut balls: [u8; 5], chance: u8) -> Option<Grid> {
        balls.sort_unstable();
        let distinct = balls.windows(2).all(|pair| pair[0] < pair[1]);
        let in_range = balls.iter().all(|&b| (1..=rules::LOTO_BALLS).contains(&b));
        (distinct && in_range && (1..=rules::LOTO_CHANCES).contains(&chance))
            .then_some(Grid { balls, chance })
    }

    /// Lit `2-3-30-35-44+10` (le format de l'historique FDJ et de la base).
    pub fn parse(text: &str) -> Option<Grid> {
        let (balls_text, chance_text) = text.trim().split_once('+')?;
        let numbers: Vec<u8> = balls_text
            .split('-')
            .map(|n| n.trim().parse().ok())
            .collect::<Option<Vec<u8>>>()?;
        let balls: [u8; 5] = numbers.try_into().ok()?;
        Grid::new(balls, chance_text.trim().parse().ok()?)
    }

    /// Rang gagné par la grille au tirage `draw` (de 1 à 9), ou `None` si elle ne gagne rien.
    pub fn rank_in(&self, draw: &Draw) -> Option<usize> {
        let good = self.balls.iter().filter(|b| draw.balls.contains(b)).count();
        let chance = draw.extra.first() == Some(&self.chance);
        rules::loto_rank(good, chance)
    }
}

/// Une grille s'écrit `2-3-30-35-44+10` dans le JSON du journal, comme en base.
impl serde::Serialize for Grid {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl fmt::Display for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e] = self.balls;
        write!(f, "{a}-{b}-{c}-{d}-{e}+{}", self.chance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_is_sorted_and_round_trips_through_its_text() {
        let grid = Grid::new([44, 3, 30, 2, 35], 10).unwrap();
        assert_eq!(grid.to_string(), "2-3-30-35-44+10");
        assert_eq!(Grid::parse("2-3-30-35-44+10"), Some(grid));
    }

    #[test]
    fn invalid_grids_are_refused() {
        assert_eq!(Grid::new([1, 1, 2, 3, 4], 1), None, "numéro répété");
        assert_eq!(Grid::new([0, 1, 2, 3, 4], 1), None, "numéro 0");
        assert_eq!(Grid::new([1, 2, 3, 4, 50], 1), None, "numéro 50");
        assert_eq!(Grid::new([1, 2, 3, 4, 5], 11), None, "chance 11");
        assert_eq!(Grid::parse("1-2-3-4+5"), None, "quatre numéros");
        assert_eq!(Grid::parse("1-2-3-4-5"), None, "sans chance");
    }

    #[test]
    fn the_rank_of_a_grid_follows_the_rules() {
        let draw = Draw {
            game: Game::GrandLoto,
            date: NaiveDate::from_ymd_opt(2025, 12, 25).unwrap(),
            balls: [2, 3, 30, 35, 44],
            extra: vec![10],
            ranks: Vec::new(),
            codes: None,
        };
        let rank = |text: &str| Grid::parse(text).unwrap().rank_in(&draw);
        assert_eq!(rank("2-3-30-35-44+10"), Some(1));
        assert_eq!(rank("2-3-30-35-44+9"), Some(2));
        assert_eq!(rank("2-3-31-1-4+9"), Some(8));
        assert_eq!(rank("2-3-30-1-4+9"), Some(6));
        assert_eq!(rank("1-4-5-6-7+10"), Some(9));
        assert_eq!(rank("1-4-5-6-7+9"), None);
        assert_eq!(rank("2-4-5-6-7+9"), None);
    }
}
