//! Types communs aux sources du Loto Foot : grille en cours, rapports passés, résultat officiel.
//!
//! Chaque source (FDJ, resultats-fdj.fr, prono-match) est lue par son propre module, qui ne produit que ces types : le
//! reste du programme (archive, règlement, application) ne sait pas d'où vient une grille. Seuls les faits extraits sont
//! conservés, jamais les pages (leurs mentions légales interdisent la reproduction).

use chrono::{DateTime, NaiveDate, Utc};

use super::Game;
use crate::domain::{Euros, Outcome};

/// Grille en cours d'une formule, telle qu'affichée : ouverte aux paris, ou annoncée mais pas encore ouverte.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenGrid {
    /// Formule.
    pub game: Game,
    /// `true` si la grille accepte les prises de jeu ; `false` si elle est seulement annoncée, sans répartition.
    pub accepting: bool,
    /// Numéro du tirage (« n°133 »).
    pub number: u32,
    /// Fin de validation des prises de jeu, en UTC.
    pub closes_at: DateTime<Utc>,
    /// Gain minimum garanti du rang 1 (« pactole »), s'il est annoncé.
    pub jackpot: Option<Euros>,
    /// Rencontres : équipe à domicile, équipe à l'extérieur, noms bruts de la source.
    pub matches: Vec<(String, String)>,
    /// Répartition FDJ en pourcents entiers (1, N, 2) ; `None` tant que la FDJ ne l'a pas publiée.
    pub split: Option<Vec<[u32; 3]>>,
    /// Heure du relevé de la répartition, en UTC, telle que la source la donne (ou à défaut celle de notre lecture).
    pub split_at: Option<DateTime<Utc>>,
}

/// Rapports d'une grille passée : un élément par rang, du sans-faute au dernier rang payé.
#[derive(Debug, Clone, PartialEq)]
pub struct PastRanks {
    /// Numéro du tirage.
    pub number: u32,
    /// Jour de clôture (`None` s'il est illisible) : le numéro seul ne dit pas de quelle année est la grille.
    pub closed: Option<NaiveDate>,
    /// Rangs : nombre de gagnants et gain unitaire (`None` quand il n'y a « aucun gagnant »).
    pub ranks: Vec<RankLine>,
}

/// Un rang publié.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankLine {
    /// Bons pronostics du rang (7 pour le sans-faute du LF7) ; `None` si la source ne le dit pas (tableau d'historique).
    pub good: Option<u32>,
    /// Nombre de grilles gagnantes.
    pub winners: u32,
    /// Gain unitaire, `None` sans gagnant.
    pub unit_gain: Option<Euros>,
}

/// Page d'une formule : la grille en cours (s'il y en a une) et les rapports des dernières grilles.
#[derive(Debug, Clone, PartialEq)]
pub struct GridPage {
    /// Grille ouverte.
    pub open: Option<OpenGrid>,
    /// Rapports des dernières grilles.
    pub history: Vec<PastRanks>,
}

/// Résultat d'une rencontre : un signe, ou l'annulation de la rencontre.
///
/// Une `enum` plutôt qu'un `Option<Outcome>` : « annulée » est un résultat à part entière, qui change le calcul des
/// gagnants, pas une valeur manquante.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResult {
    /// Un signe est sorti (1, N ou 2).
    Sign(Outcome),
    /// Rencontre annulée : elle est bonne pour tous les signes (règlement Loto Sports v6, art. 10.6 ; `docs/sources.md` §74).
    Canceled,
}

impl MatchResult {
    /// Code à une lettre stocké dans `lotofoot_result.outcome` : `H`, `D` ou `A` pour un signe, `C` pour une rencontre
    /// annulée (migration 24).
    pub fn code(self) -> &'static str {
        match self {
            MatchResult::Sign(outcome) => outcome.code(),
            MatchResult::Canceled => "C",
        }
    }

    /// Résultat correspondant à un code de [`MatchResult::code`] ; `None` pour toute autre valeur.
    pub fn from_code(code: &str) -> Option<MatchResult> {
        if code == "C" {
            Some(MatchResult::Canceled)
        } else {
            Outcome::from_code(code).map(MatchResult::Sign)
        }
    }

    /// Signe sorti ; `None` pour une rencontre annulée, où aucun signe n'est sorti.
    pub fn sign(self) -> Option<Outcome> {
        match self {
            MatchResult::Sign(outcome) => Some(outcome),
            MatchResult::Canceled => None,
        }
    }

    /// Un pronostic sur `pick` est-il bon ? Une rencontre annulée est bonne pour tous les signes (règlement Loto Sports v6,
    /// art. 10.6 ; `docs/sources.md` §39 et §74) : les rangs restent « N bons » et « N − 1 bons ».
    pub fn is_good_for(self, pick: Outcome) -> bool {
        match self {
            MatchResult::Sign(outcome) => outcome == pick,
            MatchResult::Canceled => true,
        }
    }
}

/// Les signes de toute une grille, `None` si une rencontre est annulée : une rencontre annulée n'a aucun signe, et ce qui
/// suppose une issue par rencontre (calibration du modèle) ne peut pas la lire.
pub fn signs_of(results: &[MatchResult]) -> Option<Vec<Outcome>> {
    // `collect` vers `Option<Vec<_>>` s'arrête au premier `None` : une seule rencontre annulée suffit à rendre `None`.
    results.iter().map(|result| result.sign()).collect()
}

/// Résultat officiel d'une grille.
#[derive(Debug, Clone, PartialEq)]
pub struct GridResult {
    /// Formule, `None` pour une formule non suivie (prono-match renvoie ainsi le LF12 et le LF15).
    pub game: Option<Game>,
    /// Numéro du tirage.
    pub number: u32,
    /// Jour de clôture affiché (heure de Paris) : il donne l'année de la grille.
    pub date: Option<NaiveDate>,
    /// Instant de la fin de validation, en UTC, quand la source le donne (FDJ, resultats-fdj.fr) ; `None` pour prono-match.
    pub closes_at: Option<DateTime<Utc>>,
    /// Rencontres dans l'ordre : équipes et résultat (signe, ou rencontre annulée).
    pub matches: Vec<(String, String, MatchResult)>,
    /// Rapports, du sans-faute au dernier rang. Vide tant que la source ne les a pas publiés.
    pub ranks: Vec<RankLine>,
}

/// Répartition à garder : `None` si tous les pourcentages sont à zéro.
///
/// Une grille annoncée (pas encore ouverte) porte des pourcentages à 0 : ce n'est pas une répartition, et l'écrire dans
/// l'archive (qui n'est jamais modifiée) y inscrirait un faux relevé (`docs/sources.md` §74).
pub(super) fn published_split(split: Vec<[u32; 3]>) -> Option<Vec<[u32; 3]>> {
    split.iter().flatten().any(|&q| q > 0).then_some(split)
}

/// Rapports à garder : une liste vide si aucun rang n'a de gagnant.
///
/// Sur une grille LF7 ou LF8 réelle, le rang 2 a toujours des gagnants (des milliers, `docs/sources.md` §74) : des rangs
/// tous à zéro sont un rapport pas encore publié, non un rapport sans gagnant. L'archive n'écrit un rang qu'une fois
/// (`INSERT OR IGNORE`, jamais de mise à jour) : mieux vaut ne rien écrire que figer des zéros provisoires.
pub(super) fn published_ranks(ranks: Vec<RankLine>) -> Vec<RankLine> {
    if ranks.iter().all(|rank| rank.winners == 0) {
        Vec::new()
    } else {
        ranks
    }
}
