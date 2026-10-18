//! Modèle de Poisson : les buts des deux équipes sont indépendants.

use super::likelihood::fit_model;
use super::{FittedModel, MatchModel, TrainingGame};
use crate::error::Result;

/// Modèle de Poisson avec régularisation L2 des forces d'équipes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Poisson {
    /// Intensité de la régularisation L2 (`κ`) : plus elle est grande, plus les équipes sont ramenées vers la moyenne.
    pub ridge: f64,
}

impl MatchModel for Poisson {
    fn name(&self) -> &'static str {
        "poisson"
    }

    fn fit(
        &self,
        teams: usize,
        games: &[TrainingGame],
        start: Option<&FittedModel>,
    ) -> Result<FittedModel> {
        fit_model(teams, games, self.ridge, false, start)
    }
}
