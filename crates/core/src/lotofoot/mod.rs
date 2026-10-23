//! Loto Foot (FDJ) : jeu à répartition où les gagnants se partagent les mises des autres joueurs (phase 14).
//!
//! On y cherche les combinaisons que le marché juge plausibles mais que le public délaisse. Le module calcule
//! l'espérance exacte d'un ticket ([`model`]), choisit les tickets de la semaine ([`optimize`]), lit les pages de
//! prono-match ([`pronomatch`]) et règle les tickets joués ([`settle`]). Étude et choix : `docs/sources.md` §37 et §38.

pub mod bulletin;
pub mod fetch;
pub mod model;
pub mod odds;
pub mod optimize;
pub mod played;
pub mod pronomatch;
pub mod session;
pub mod settle;
pub mod verify;

use std::fmt;

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
}
