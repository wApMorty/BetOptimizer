//! Modèle de Dixon-Coles : Poisson corrigé pour la dépendance des scores faibles (0-0, 1-0, 0-1, 1-1).

use super::likelihood::fit_model;
use super::{FittedModel, MatchModel, TrainingGame};
use crate::error::Result;

/// Modèle de Dixon-Coles (1997) avec pondération temporelle (portée par `TrainingGame::weight`) et
/// régularisation L2 des forces d'équipes (utile aux équipes promues, sans historique dans la ligue).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DixonColes {
    /// Intensité de la régularisation L2 (`κ`).
    pub ridge: f64,
}

impl MatchModel for DixonColes {
    fn name(&self) -> &'static str {
        "dixon_coles"
    }

    fn fit(
        &self,
        teams: usize,
        games: &[TrainingGame],
        start: Option<&FittedModel>,
    ) -> Result<FittedModel> {
        fit_model(teams, games, self.ridge, true, start)
    }
}
