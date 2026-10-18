//! Modèles de prévision des scores : Poisson et Dixon-Coles.
//!
//! Un modèle estime, pour chaque match, les buts attendus de chaque équipe, puis en déduit une grille de scores
//! d'où l'on lit les probabilités du 1X2, du plus/moins et de « les deux équipes marquent ».
//!
//! Paramétrage (par ligue) : `log λ = base + avantage + attaque_dom + défense_ext` pour l'équipe à domicile et
//! `log μ = base + attaque_ext + défense_dom` pour l'équipe à l'extérieur. Une *défense* élevée est une défense
//! qui encaisse beaucoup : `défense` s'ajoute aux buts de l'adversaire.

pub mod dixon_coles;
pub mod grid;
pub mod likelihood;
pub mod poisson;

#[cfg(test)]
mod reference;

use crate::error::Result;
use grid::ScoreGrid;

/// Un match d'entraînement d'une ligue. Les équipes sont des indices propres à la ligue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrainingGame {
    /// Indice de l'équipe à domicile.
    pub home: usize,
    /// Indice de l'équipe à l'extérieur.
    pub away: usize,
    /// Buts réels de l'équipe à domicile (sert à la correction des scores faibles).
    pub goals_home: u8,
    /// Buts réels de l'équipe à l'extérieur.
    pub goals_away: u8,
    /// Cible de la partie Poisson, domicile : les buts réels, ou des « buts attendus » estimés à partir des tirs.
    pub target_home: f64,
    /// Cible de la partie Poisson, extérieur.
    pub target_away: f64,
    /// Poids du match dans la vraisemblance (décroissance temporelle) : 1 pour le plus récent, moins pour les anciens.
    pub weight: f64,
}

/// Paramètres ajustés d'une ligue.
#[derive(Debug, Clone, PartialEq)]
pub struct FittedModel {
    /// Force offensive de chaque équipe (échelle logarithmique ; 0 = moyenne).
    pub attack: Vec<f64>,
    /// Faiblesse défensive de chaque équipe (0 = moyenne ; plus c'est haut, plus elle encaisse).
    pub defence: Vec<f64>,
    /// Logarithme du nombre moyen de buts d'une équipe sans avantage ni force particulière.
    pub intercept: f64,
    /// Avantage du terrain (logarithme du facteur appliqué aux buts de l'équipe à domicile).
    pub home_advantage: f64,
    /// Paramètre `ρ` de la correction des scores faibles (0 pour Poisson).
    pub rho: f64,
}

impl FittedModel {
    /// Buts attendus `(λ, μ)` de l'équipe à domicile et de celle à l'extérieur.
    pub fn expectancies(&self, home: usize, away: usize) -> (f64, f64) {
        let home_mean =
            (self.intercept + self.home_advantage + self.attack[home] + self.defence[away]).exp();
        let away_mean = (self.intercept + self.attack[away] + self.defence[home]).exp();
        (home_mean, away_mean)
    }

    /// Grille de scores du match, `None` si les paramètres ne donnent pas une loi de probabilité valide.
    pub fn grid(&self, home: usize, away: usize) -> Option<ScoreGrid> {
        let (home_mean, away_mean) = self.expectancies(home, away);
        ScoreGrid::dixon_coles(home_mean, away_mean, self.rho)
    }
}

/// Un modèle de scores : sait s'ajuster sur des matchs passés.
///
/// Deux implémentations réelles : [`poisson::Poisson`] (buts indépendants) et [`dixon_coles::DixonColes`]
/// (corrélation des scores faibles). Le trait existe parce que le moteur de walk-forward doit pouvoir lancer
/// l'un ou l'autre sans changer de code.
pub trait MatchModel {
    /// Nom court du modèle (stocké avec les prédictions).
    fn name(&self) -> &'static str;

    /// Ajuste le modèle sur `games` pour une ligue de `teams` équipes.
    ///
    /// `start` : paramètres d'un ajustement précédent, utilisés comme point de départ (le walk-forward
    /// ajuste chaque semaine un jeu de données presque identique : le point de départ accélère beaucoup).
    fn fit(
        &self,
        teams: usize,
        games: &[TrainingGame],
        start: Option<&FittedModel>,
    ) -> Result<FittedModel>;
}
