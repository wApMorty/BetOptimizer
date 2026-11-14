//! Loto Foot (FDJ) : jeu à répartition où les gagnants se partagent les mises des autres joueurs (phase 14).
//!
//! On y cherche les combinaisons que le marché juge plausibles mais que le public délaisse. Le module calcule
//! l'espérance exacte d'un ticket ([`model`]), choisit les tickets de la semaine ([`optimize`]), lit les pages des
//! sources ([`pronomatch`] ; les types communs aux sources sont dans [`grid_page`]) et règle les tickets joués
//! ([`settle`]). Étude et choix : `docs/sources.md` §37, §38 et §74.

pub mod alert;
pub mod bulletin;
pub mod espn;
pub mod exact;
pub mod fetch;
pub mod grid_page;
pub mod live;
pub mod model;
pub mod odds;
pub mod oddspapi;
pub mod optimize;
pub mod outlets;
pub mod played;
pub mod price;
pub mod pronomatch;
pub mod scores;
pub mod session;
pub mod settle;
pub mod verify;

use std::fmt;

use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Europe::Paris;

use crate::domain::Outcome;

/// Formule de jeu suivie. Seuls le LF7 et le LF8 sont joués (choix de Paul du 2026-10-22 : le LF12 et le LF15 tiennent
/// de la loterie, `docs/sources.md` §37).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Game {
    /// Loto Foot 7 : 7 rencontres, deux rangs payés.
    Lf7,
    /// Loto Foot 8 : 8 rencontres, deux rangs payés.
    Lf8,
}

impl Game {
    /// Les deux formules suivies.
    pub const ALL: [Game; 2] = [Game::Lf7, Game::Lf8];

    /// Code court, en base et dans les URL de prono-match : `lf7`, `lf8`.
    pub fn code(self) -> &'static str {
        match self {
            Game::Lf7 => "lf7",
            Game::Lf8 => "lf8",
        }
    }

    /// Formule correspondant à un code court ; `None` pour toute autre valeur.
    pub fn from_code(code: &str) -> Option<Game> {
        Game::ALL.into_iter().find(|game| game.code() == code)
    }

    /// Formule correspondant au nombre de rencontres affiché par prono-match (« Loto Foot 7 ») ; `None` sinon.
    pub fn from_matches(matches: usize) -> Option<Game> {
        Game::ALL.into_iter().find(|game| game.matches() == matches)
    }

    /// Nombre de rencontres d'une grille normale (règlement Loto Sports v6, art. 4.1 et 4.2).
    pub fn matches(self) -> usize {
        match self {
            Game::Lf7 => 7,
            Game::Lf8 => 8,
        }
    }

    /// Part de la masse des gains attribuée à chaque rang, du sans-faute (rang 1) à une erreur (rang 2).
    /// Règlement Loto Sports v6, art. 10.1.1 et 10.1.2 : 45 % et 55 % pour les deux formules.
    pub fn rank_shares(self) -> [f64; 2] {
        [0.45, 0.55]
    }
}

impl fmt::Display for Game {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Game::Lf7 => write!(f, "LF7"),
            Game::Lf8 => write!(f, "LF8"),
        }
    }
}

/// Identité d'une grille : sa formule, son année et son numéro.
///
/// La FDJ renumérote chaque formule à partir de 1 chaque 1er janvier (`docs/sources.md` §66) : un numéro seul ne désigne pas
/// une grille. `year` est l'année civile de la clôture, heure de Paris. `Copy` : trois petits champs, passés par valeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GridId {
    /// Formule.
    pub game: Game,
    /// Année civile de la clôture, heure de Paris.
    pub year: i32,
    /// Numéro dans l'année.
    pub number: u32,
}

impl GridId {
    /// Identité d'une grille qui ferme à `closes_at`.
    pub fn closing(game: Game, number: u32, closes_at: DateTime<Utc>) -> GridId {
        GridId {
            game,
            year: closes_at.with_timezone(&Paris).year(),
            number,
        }
    }

    /// Forme écrite des boutons de l'application : `lf7-2026-133`.
    pub fn code(self) -> String {
        format!("{}-{}-{}", self.game.code(), self.year, self.number)
    }

    /// Lit la forme écrite de [`GridId::code`] ; `None` si elle est illisible.
    pub fn parse(code: &str) -> Option<GridId> {
        let mut parts = code.trim().split('-');
        let id = GridId {
            game: Game::from_code(parts.next()?)?,
            year: parts.next()?.parse().ok()?,
            number: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(id)
    }
}

impl fmt::Display for GridId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} n°{}", self.game, self.number)
    }
}

/// Identité d'une grille de 2026, pour les tests.
#[cfg(test)]
pub(crate) fn grid_of_2026(game: Game, number: u32) -> GridId {
    GridId {
        game,
        year: 2026,
        number,
    }
}

/// Part des mises redistribuée aux gagnants : 70 % (règlement Loto Sports v6, art. 7.6 ; ANJ, décision 2026-146).
pub const PAYOUT_RATE: f64 = 0.70;

/// Prix d'une grille simple (règlement v6, art. 7) : un ticket coûte 1 €.
pub const TICKET_PRICE: f64 = 1.0;

/// Ticket simple : un signe par rencontre, dans l'ordre de la grille.
///
/// Newtype autour de `Vec<Outcome>` : un ticket n'est pas une liste quelconque d'issues (il en faut exactement une par
/// rencontre), et on lui donne sa forme écrite (`1N2…`) au même endroit.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ticket(Vec<Outcome>);

impl Ticket {
    /// Crée un ticket à partir des signes, dans l'ordre de la grille.
    pub fn new(signs: Vec<Outcome>) -> Ticket {
        Ticket(signs)
    }

    /// Lit la forme écrite d'un ticket (`1N2…`) ; `None` si un caractère n'est ni `1`, ni `N`, ni `2`.
    pub fn parse(code: &str) -> Option<Ticket> {
        code.chars()
            .map(|c| match c {
                '1' => Some(Outcome::Home),
                'N' => Some(Outcome::Draw),
                '2' => Some(Outcome::Away),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(Ticket)
    }

    /// Signes du ticket.
    pub fn signs(&self) -> &[Outcome] {
        &self.0
    }

    /// Nombre de rencontres.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` pour un ticket sans rencontre (n'arrive pas en pratique ; exigé par Clippy avec `len`).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Nombre de bons pronostics contre les résultats donnés (dans l'ordre de la grille).
    pub fn good(&self, results: &[Outcome]) -> usize {
        self.0.iter().zip(results).filter(|(a, b)| a == b).count()
    }
}

impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &sign in &self.0 {
            write!(f, "{}", sign_char(sign))?;
        }
        Ok(())
    }
}

/// Signe écrit sur le bulletin : `1`, `N` ou `2`.
pub(crate) fn sign_char(sign: Outcome) -> char {
    match sign {
        Outcome::Home => '1',
        Outcome::Draw => 'N',
        Outcome::Away => '2',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_reads_and_writes_its_signs() {
        let ticket = Ticket::parse("1N2221N").unwrap();
        assert_eq!(ticket.len(), 7);
        assert_eq!(ticket.to_string(), "1N2221N");
        assert!(Ticket::parse("1X2").is_none());
    }

    #[test]
    fn good_counts_matching_signs() {
        let ticket = Ticket::parse("1N2").unwrap();
        let results = [Outcome::Home, Outcome::Away, Outcome::Away];
        assert_eq!(ticket.good(&results), 2);
    }

    #[test]
    fn games_map_their_codes_and_sizes() {
        assert_eq!(Game::from_code("lf8"), Some(Game::Lf8));
        assert_eq!(Game::from_matches(7), Some(Game::Lf7));
        assert_eq!(Game::from_matches(12), None);
        let shares = Game::Lf7.rank_shares();
        assert!((shares[0] + shares[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_grid_belongs_to_the_paris_year_of_its_closing() {
        let closing = |text: &str| {
            let at = DateTime::parse_from_rfc3339(text).unwrap();
            GridId::closing(Game::Lf7, 1, at.with_timezone(&Utc)).year
        };
        assert_eq!(closing("2026-12-31T22:59:00Z"), 2026);
        // 23 h 30 UTC le 31 décembre : déjà le 1er janvier à Paris.
        assert_eq!(closing("2026-12-31T23:30:00Z"), 2027);
    }

    #[test]
    fn a_grid_code_reads_back() {
        let id = GridId {
            game: Game::Lf8,
            year: 2027,
            number: 12,
        };
        assert_eq!(id.code(), "lf8-2027-12");
        assert_eq!(GridId::parse("lf8-2027-12"), Some(id));
        assert_eq!(id.to_string(), "LF8 n°12");
        assert_eq!(GridId::parse("lf8-12"), None);
        assert_eq!(GridId::parse("lf8-2027-12-1"), None);
    }
}
