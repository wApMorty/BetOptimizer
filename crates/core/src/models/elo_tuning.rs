//! Réglage de l'Elo sur la validation, puis gel : on essaie une grille, on garde le log loss le plus bas, et le test
//! n'est lu qu'ensuite, une seule fois (invariant 8 de CLAUDE.md : le rapport dit combien de configurations ont été essayées).
//!
//! Découpage (dates, fixé avant de voir les résultats) : les matchs avant la validation ne servent qu'à **chauffer** les
//! classements ; validation du 2022-01-01 au 2023-12-31 ; test à partir du 2024-01-01.

use chrono::NaiveDate;

use super::elo::{EloConfig, Prediction, walk_forward};
use crate::ingest::tennis::Status;
use crate::store::tennis::StoredMatch;

/// Premier jour de la validation.
pub const VALIDATION_START: NaiveDate = match NaiveDate::from_ymd_opt(2022, 1, 1) {
    Some(date) => date,
    None => panic!("date constante valide"),
};

/// Premier jour du test.
pub const TEST_START: NaiveDate = match NaiveDate::from_ymd_opt(2024, 1, 1) {
    Some(date) => date,
    None => panic!("date constante valide"),
};

/// Expérience minimale des deux joueurs (matchs déjà vus) pour qu'une prévision compte : un Elo neuf ne sait rien.
pub const MIN_EXPERIENCE: u32 = 10;

/// Valeurs essayées pour `k_base`.
pub const K_BASES: [f64; 3] = [150.0, 250.0, 350.0];
/// Valeurs essayées pour `k_shape`.
pub const K_SHAPES: [f64; 3] = [0.3, 0.4, 0.5];
/// Valeurs essayées pour `surface_weight` (0 = général seul, 0,5 = mélange de Sackmann, 1 = surface seule).
pub const SURFACE_WEIGHTS: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// Une configuration et son score de validation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TuningRow {
    /// Hyperparamètres.
    pub config: EloConfig,
    /// Log loss moyen sur la validation.
    pub log_loss: f64,
    /// Matchs de validation évalués.
    pub matches: usize,
}

/// Résultat d'une session de réglage.
#[derive(Debug, Clone, PartialEq)]
pub struct TuningResult {
    /// Toutes les configurations essayées, dans l'ordre de la grille.
    pub rows: Vec<TuningRow>,
    /// La meilleure (log loss le plus bas ; à égalité, la première de la grille).
    pub best: TuningRow,
}

/// Une prévision compte-t-elle pour l'évaluation ? Match terminé, deux joueurs installés.
pub fn is_scored(prediction: &Prediction) -> bool {
    prediction.status == Status::Completed && prediction.min_experience >= MIN_EXPERIENCE
}

/// Log loss moyen des prévisions de la fenêtre `[from, to)` qui comptent, avec leur nombre.
pub fn log_loss_between(
    predictions: &[Prediction],
    from: NaiveDate,
    to: NaiveDate,
) -> Option<(f64, usize)> {
    let losses: Vec<f64> = predictions
        .iter()
        .filter(|p| is_scored(p) && p.date >= from && p.date < to)
        .map(|p| -p.p_winner.ln())
        .collect();
    (!losses.is_empty()).then(|| {
        (
            losses.iter().sum::<f64>() / losses.len() as f64,
            losses.len(),
        )
    })
}

/// Essaie toute la grille sur la validation.
pub fn tune(matches: &[StoredMatch]) -> Option<TuningResult> {
    let mut rows = Vec::new();
    for k_base in K_BASES {
        for k_shape in K_SHAPES {
            for surface_weight in SURFACE_WEIGHTS {
                let config = EloConfig {
                    k_base,
                    k_shape,
                    surface_weight,
                };
                let predictions = walk_forward(matches, config);
                if let Some((log_loss, count)) =
                    log_loss_between(&predictions, VALIDATION_START, TEST_START)
                {
                    rows.push(TuningRow {
                        config,
                        log_loss,
                        matches: count,
                    });
                }
            }
        }
    }
    let best = rows.iter().copied().reduce(|best, row| {
        if row.log_loss < best.log_loss {
            row
        } else {
            best
        }
    })?;
    Some(TuningResult { rows, best })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::ingest::tennis::{Surface, Tour};

    /// Un petit monde où « Fort » bat toujours « Faible » et où les deux jouent assez pour passer le seuil d'expérience.
    fn world() -> Vec<StoredMatch> {
        let mut matches = Vec::new();
        let mut id = 0;
        // Un match par jour pendant 3 ans, à partir du 2021-01-01.
        let mut day = NaiveDate::from_ymd_opt(2021, 1, 1).unwrap();
        while day < NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() {
            id += 1;
            matches.push(StoredMatch {
                id,
                tour: Tour::Atp,
                season: 2021,
                tournament: "Monde".into(),
                series: None,
                date: day,
                surface: if id % 2 == 0 {
                    Surface::Clay
                } else {
                    Surface::Hard
                },
                round: "Final".into(),
                best_of: 3,
                winner: "Fort".into(),
                loser: "Faible".into(),
                status: Status::Completed,
                odds: HashMap::new(),
            });
            day = day.succ_opt().unwrap();
        }
        matches
    }

    #[test]
    fn the_grid_has_forty_five_configurations_and_the_best_has_the_lowest_log_loss() {
        let result = tune(&world()).unwrap();

        assert_eq!(
            result.rows.len(),
            K_BASES.len() * K_SHAPES.len() * SURFACE_WEIGHTS.len()
        );
        assert!(
            result
                .rows
                .iter()
                .all(|r| r.log_loss >= result.best.log_loss)
        );
        // Un monde sans surprise : on gagne à prévoir fort, donc un grand K est préféré à un petit.
        assert!(result.best.config.k_base >= 250.0);
        assert!(
            result.best.log_loss < 0.69,
            "mieux que le hasard (ln 2 = 0,693) : {}",
            result.best.log_loss
        );
    }

    #[test]
    fn only_validation_matches_with_experienced_players_are_scored() {
        let predictions = walk_forward(&world(), EloConfig::starting_point());

        let (_, validation) = log_loss_between(&predictions, VALIDATION_START, TEST_START).unwrap();
        let (_, warm_up) = log_loss_between(
            &predictions,
            NaiveDate::from_ymd_opt(2021, 1, 1).unwrap(),
            VALIDATION_START,
        )
        .unwrap();

        assert_eq!(validation, 365 + 365);
        // 365 jours de chauffe moins les 10 premiers matchs, où l'expérience est insuffisante.
        assert_eq!(warm_up, 365 - MIN_EXPERIENCE as usize);
    }

    #[test]
    fn no_validation_data_means_no_tuning() {
        assert!(tune(&[]).is_none());
    }
}
