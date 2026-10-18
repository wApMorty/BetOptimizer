//! Modèles de prévision : Poisson et Dixon-Coles pour les scores de football, Elo ([`elo`]) pour le tennis.
//!
//! Un modèle estime, pour chaque match, les buts attendus de chaque équipe, puis en déduit une grille de scores
//! d'où l'on lit les probabilités du 1X2, du plus/moins et de « les deux équipes marquent ».
//!
//! Paramétrage (par ligue) : `log λ = base + avantage + attaque_dom + défense_ext` pour l'équipe à domicile et
//! `log μ = base + attaque_ext + défense_dom` pour l'équipe à l'extérieur. Une *défense* élevée est une défense
//! qui encaisse beaucoup : `défense` s'ajoute aux buts de l'adversaire.

pub mod data;
pub mod dixon_coles;
pub mod elo;
pub mod elo_tuning;
pub mod grid;
pub mod likelihood;
pub mod poisson;
pub mod tuning;
pub mod walk_forward;
pub mod xg_proxy;

#[cfg(test)]
mod reference;
#[cfg(test)]
pub(crate) mod testing;

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
        self.grid_checked(home, away).map(|(grid, _)| grid)
    }

    /// Comme [`FittedModel::grid`], et indique si `ρ` a dû être ramené dans le domaine faisable du match.
    ///
    /// Le facteur `τ` doit rester positif : `1 + λρ > 0`, `1 + μρ > 0` et `1 − λμρ > 0`, soit
    /// `ρ > −1/max(λ, μ)` (pour `ρ < 0`) et `ρ < 1/(λμ)` (pour `ρ > 0`). Un `ρ` ajusté sur un petit échantillon
    /// peut sortir de ce domaine pour un match très déséquilibré : on le ramène à 90 % de la borne (donc
    /// `τ ≥ 0,1`) et on le signale, plutôt que de renvoyer une loi de probabilité invalide.
    pub fn grid_checked(&self, home: usize, away: usize) -> Option<(ScoreGrid, bool)> {
        let (home_mean, away_mean) = self.expectancies(home, away);
        let lower = -0.9 / home_mean.max(away_mean);
        let upper = 0.9 / (home_mean * away_mean);
        let rho = self.rho.clamp(lower, upper.max(lower));
        let clamped = rho != self.rho;
        ScoreGrid::dixon_coles(home_mean, away_mean, rho).map(|grid| (grid, clamped))
    }
}

/// Un modèle de scores : sait s'ajuster sur des matchs passés.
///
/// Deux implémentations réelles : [`poisson::Poisson`] (buts indépendants) et [`dixon_coles::DixonColes`]
/// (corrélation des scores faibles). Le trait existe parce que le moteur de walk-forward doit pouvoir lancer
/// l'un ou l'autre sans changer de code.
pub trait MatchModel: Sync {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn model(rho: f64) -> FittedModel {
        FittedModel {
            attack: vec![0.5, -0.5],
            defence: vec![0.0, 0.0],
            intercept: 0.4,
            home_advantage: 0.3,
            rho,
        }
    }

    #[test]
    fn a_rho_inside_the_feasible_domain_is_left_alone() {
        let (grid, clamped) = model(-0.1).grid_checked(0, 1).unwrap();
        assert!(!clamped);
        assert!(grid.home_draw_away().is_some());
    }

    #[test]
    fn a_rho_outside_the_feasible_domain_is_clamped_flagged_and_still_a_valid_distribution() {
        // Équipe 0 à domicile contre l'équipe 1 : λ = e^(0,4 + 0,3 + 0,5) ≈ 3,3 ; ρ = −0,4 rendrait τ(0,1) négatif.
        let extreme = model(-0.4);
        let (home_mean, _) = extreme.expectancies(0, 1);
        assert!(
            1.0 + home_mean * extreme.rho < 0.0,
            "le cas de test doit être hors domaine"
        );

        let (grid, clamped) = extreme.grid_checked(0, 1).unwrap();

        assert!(clamped);
        let total: f64 = (0..=10)
            .flat_map(|x| (0..=10).map(move |y| (x, y)))
            .map(|(x, y)| grid.probability(x, y))
            .sum();
        assert!((total - 1.0).abs() < 1e-12);
        assert!((0..=10).all(|x| (0..=10).all(|y| grid.probability(x, y) >= 0.0)));
    }
}
