//! Tennis : tableurs de tennis-data.co.uk (résultats, surface, cotes), lecture, import et contrôle qualité.
//!
//! Les fichiers sont téléchargés **à la main** par l'utilisateur (conditions d'usage et `robots.txt` du site, voir
//! `docs/sources.md` §21) dans `tennis-data/atp/` et `tennis-data/wta/` ; le programme ne lit que des fichiers locaux.

pub mod import;
pub mod parse;
pub mod quality;

use chrono::NaiveDate;

use crate::domain::Odds;

/// Circuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tour {
    /// Circuit masculin.
    Atp,
    /// Circuit féminin.
    Wta,
}

impl Tour {
    /// Les deux circuits.
    pub const ALL: [Tour; 2] = [Tour::Atp, Tour::Wta];

    /// Texte stocké en base et nom du sous-dossier.
    pub fn as_str(self) -> &'static str {
        match self {
            Tour::Atp => "atp",
            Tour::Wta => "wta",
        }
    }

    /// Relit le texte stocké en base.
    pub fn parse(text: &str) -> Option<Tour> {
        Tour::ALL.into_iter().find(|tour| tour.as_str() == text)
    }
}

/// Surface du court.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Surface {
    /// Dur.
    Hard,
    /// Terre battue.
    Clay,
    /// Gazon.
    Grass,
    /// Moquette (disparue du circuit, présente dans les vieux fichiers).
    Carpet,
}

impl Surface {
    /// Les quatre surfaces.
    pub const ALL: [Surface; 4] = [
        Surface::Hard,
        Surface::Clay,
        Surface::Grass,
        Surface::Carpet,
    ];

    /// Texte stocké en base.
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Hard => "hard",
            Surface::Clay => "clay",
            Surface::Grass => "grass",
            Surface::Carpet => "carpet",
        }
    }

    /// Relit le texte stocké en base ou publié par tennis-data (`Hard`, `Clay`…), sans tenir compte de la casse.
    pub fn parse(text: &str) -> Option<Surface> {
        let lower = text.trim().to_lowercase();
        Surface::ALL.into_iter().find(|s| s.as_str() == lower)
    }
}

/// Rang chronologique d'un tour de tournoi (`1st Round` = 1 … `The Final` = 8), 0 pour un libellé inconnu.
///
/// Les dates de tennis-data sont parfois approximatives (la demi-finale datée du jour de la finale, après une
/// interruption de pluie) : à date égale, le rang du tour rétablit l'ordre réel des matchs d'un même tournoi.
pub fn round_rank(round: &str) -> u8 {
    match round {
        "1st Round" => 1,
        "2nd Round" => 2,
        "3rd Round" => 3,
        "4th Round" => 4,
        "Round Robin" => 5,
        "Quarterfinals" => 6,
        "Semifinals" => 7,
        "The Final" => 8,
        _ => 0,
    }
}

/// Issue d'un match dans le fichier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Match terminé normalement.
    Completed,
    /// Abandon en cours de match : le vainqueur est connu, mais le score ne dit pas la force relative.
    Retired,
    /// Forfait avant le match : aucune balle jouée, les paris sont remboursés.
    Walkover,
}

impl Status {
    /// Texte stocké en base.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Completed => "completed",
            Status::Retired => "retired",
            Status::Walkover => "walkover",
        }
    }

    /// Relit le texte stocké en base.
    pub fn parse(text: &str) -> Option<Status> {
        [Status::Completed, Status::Retired, Status::Walkover]
            .into_iter()
            .find(|s| s.as_str() == text)
    }
}

/// Cotes d'un bookmaker : une pour le vainqueur, une pour le perdant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchOdds {
    /// Préfixe de colonne de tennis-data (`PS` = Pinnacle, `B365`, `Avg`…).
    pub bookmaker: &'static str,
    /// Cote du vainqueur.
    pub winner: Odds,
    /// Cote du perdant.
    pub loser: Odds,
}

/// Un match lu dans un tableur.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMatch {
    /// Circuit.
    pub tour: Tour,
    /// Année du fichier.
    pub season: u16,
    /// Tournoi.
    pub tournament: String,
    /// Lieu.
    pub location: Option<String>,
    /// Date du match.
    pub date: NaiveDate,
    /// Catégorie du tournoi (`Series` à l'ATP, `Tier` à la WTA), texte brut.
    pub series: Option<String>,
    /// `Indoor` ou `Outdoor`.
    pub court: Option<String>,
    /// Surface.
    pub surface: Surface,
    /// Tour du tournoi (`1st Round`, `Final`…).
    pub round: String,
    /// Nombre maximal de sets (3 ou 5).
    pub best_of: u8,
    /// Vainqueur, nom publié.
    pub winner: String,
    /// Perdant, nom publié.
    pub loser: String,
    /// Classement du vainqueur au début du tournoi.
    pub winner_rank: Option<u32>,
    /// Classement du perdant.
    pub loser_rank: Option<u32>,
    /// Points du vainqueur.
    pub winner_points: Option<u32>,
    /// Points du perdant.
    pub loser_points: Option<u32>,
    /// Sets gagnés par le vainqueur.
    pub winner_sets: Option<u8>,
    /// Sets gagnés par le perdant.
    pub loser_sets: Option<u8>,
    /// Terminé, abandon ou forfait.
    pub status: Status,
    /// Cotes valides, par bookmaker.
    pub odds: Vec<MatchOdds>,
}
