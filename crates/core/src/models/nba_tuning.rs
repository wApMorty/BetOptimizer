//! Réglage du modèle NBA sur la validation, puis gel : le test n'est lu qu'ensuite.
//!
//! Il n'existe aucun historique gratuit (`docs/sources.md` §23) : tout vient de l'archive, qui commence avec la saison
//! 2026-27. Les fenêtres sont donc fixées **avant** de voir une seule donnée :
//! - jusqu'au 2026-11-15 : chauffe des classements (aucun match évalué) ;
//! - du 2026-11-15 au 2027-02-01 : validation, où l'on règle ;
//! - à partir du 2027-02-01 : test.
//!
//! Le réglage de l'Elo (54 configurations) minimise le log loss de la victoire à domicile ; celui du pas du modèle de total
//! (3 valeurs) minimise l'erreur quadratique du total ; la pente marge/Elo et les deux écarts-types sont des estimations
//! de moindres carrés sur la même validation, pas des choix.

use chrono::NaiveDate;

use super::nba::{EloConfig, Prediction, margin_per_elo, rmse, walk_forward};
use crate::store::nba::StoredGame;

/// Premier jour de la validation.
pub const VALIDATION_START: NaiveDate = match NaiveDate::from_ymd_opt(2026, 11, 15) {
    Some(date) => date,
    None => panic!("date constante valide"),
};

/// Premier jour du test.
pub const TEST_START: NaiveDate = match NaiveDate::from_ymd_opt(2027, 2, 1) {
    Some(date) => date,
    None => panic!("date constante valide"),
};

/// Matchs déjà vus que doivent avoir les deux équipes pour qu'une prévision compte.
pub const MIN_GAMES_SEEN: u32 = 10;

/// Valeurs essayées pour `K`.
pub const KS: [f64; 3] = [10.0, 20.0, 30.0];
/// Valeurs essayées pour l'avantage du terrain (points d'Elo).
pub const HOME_ADVANTAGES: [f64; 3] = [60.0, 100.0, 140.0];
/// Valeurs essayées pour la pénalité du dos à dos (points d'Elo).
pub const BACK_TO_BACKS: [f64; 3] = [0.0, 30.0, 60.0];
/// Valeurs essayées pour la part de classement conservée entre deux saisons.
pub const CARRY_OVERS: [f64; 2] = [0.6, 0.8];
/// Valeurs essayées pour le pas d'apprentissage du modèle de total.
pub const TOTALS_RATES: [f64; 3] = [0.03, 0.06, 0.1];

/// Pas d'apprentissage du total pendant le réglage de l'Elo (sans effet sur les probabilités de victoire).
const TUNING_TOTALS_RATE: f64 = 0.06;

/// Une prévision compte-t-elle ? Les deux équipes ont déjà joué au moins [`MIN_GAMES_SEEN`] matchs.
pub fn is_scored(prediction: &Prediction) -> bool {
    prediction.min_games_seen >= MIN_GAMES_SEEN
}

/// Date (UTC) du coup d'envoi.
pub fn day_of(prediction: &Prediction) -> NaiveDate {
    prediction.commence_time.date_naive()
}

/// Prévisions de la fenêtre `[from, to)` qui comptent.
pub fn window(
    predictions: &[Prediction],
    from: NaiveDate,
    to: Option<NaiveDate>,
) -> Vec<&Prediction> {
    predictions
        .iter()
        .filter(|p| is_scored(p) && day_of(p) >= from && to.is_none_or(|end| day_of(p) < end))
        .collect()
}

/// Log loss moyen de la victoire à domicile.
pub fn home_log_loss(predictions: &[&Prediction]) -> Option<f64> {
    (!predictions.is_empty()).then(|| {
        predictions
            .iter()
            .map(|p| {
                let home_won = p.actual_margin > 0;
                -(if home_won { p.p_home } else { 1.0 - p.p_home }).ln()
            })
            .sum::<f64>()
            / predictions.len() as f64
    })
}

/// Le résultat d'une session de réglage.
#[derive(Debug, Clone, PartialEq)]
pub struct Tuned {
    /// Hyperparamètres de l'Elo retenus.
    pub elo: EloConfig,
    /// Pas d'apprentissage du modèle de total retenu.
    pub totals_rate: f64,
    /// Points de marge par point d'écart d'Elo.
    pub margin_per_elo: f64,
    /// Écart-type de la marge autour de la prévision.
    pub margin_sigma: f64,
    /// Écart-type du total autour de la prévision.
    pub total_sigma: f64,
    /// Log loss de la validation pour la configuration retenue.
    pub validation_log_loss: f64,
    /// Matchs de validation évalués.
    pub validation_games: usize,
    /// Configurations essayées (Elo et pas du total).
    pub configs_tried: usize,
}

/// Règle le modèle sur la fenêtre de validation. `None` s'il n'y a aucun match de validation exploitable.
pub fn tune(games: &[StoredGame]) -> Option<Tuned> {
    let mut best: Option<(EloConfig, f64)> = None;
    let mut tried = 0;
    for k in KS {
        for home_advantage in HOME_ADVANTAGES {
            for back_to_back in BACK_TO_BACKS {
                for carry_over in CARRY_OVERS {
                    let config = EloConfig {
                        k,
                        home_advantage,
                        back_to_back,
                        carry_over,
                    };
                    let predictions = walk_forward(games, config, TUNING_TOTALS_RATE);
                    let validation = window(&predictions, VALIDATION_START, Some(TEST_START));
                    tried += 1;
                    if let Some(loss) = home_log_loss(&validation)
                        && best.is_none_or(|(_, current)| loss < current)
                    {
                        best = Some((config, loss));
                    }
                }
            }
        }
    }
    let (elo, validation_log_loss) = best?;

    let mut best_rate: Option<(f64, f64)> = None;
    for rate in TOTALS_RATES {
        let predictions = walk_forward(games, elo, rate);
        let validation = window(&predictions, VALIDATION_START, Some(TEST_START));
        tried += 1;
        if let Some(error) = rmse(
            validation
                .iter()
                .map(|p| f64::from(p.actual_total) - p.total),
        ) && best_rate.is_none_or(|(_, current)| error < current)
        {
            best_rate = Some((rate, error));
        }
    }
    let (totals_rate, total_sigma) = best_rate?;

    let predictions = walk_forward(games, elo, totals_rate);
    let validation = window(&predictions, VALIDATION_START, Some(TEST_START));
    let slope = margin_per_elo(&validation)?;
    let margin_sigma = rmse(
        validation
            .iter()
            .map(|p| f64::from(p.actual_margin) - p.gap * slope),
    )?;
    Some(Tuned {
        elo,
        totals_rate,
        margin_per_elo: slope,
        margin_sigma,
        total_sigma,
        validation_log_loss,
        validation_games: validation.len(),
        configs_tried: tried,
    })
}

/// Saison simulée, partagée par les tests du réglage et du rapport.
#[cfg(test)]
pub(crate) mod simulation {
    use chrono::NaiveDate;

    use crate::store::nba::{StoredGame, season_of};

    /// Petit générateur déterministe (SplitMix64) : une saison simulée reproductible.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        }
        /// Approximation gaussienne : somme de 12 uniformes moins 6.
        fn normal(&mut self, sigma: f64) -> f64 {
            sigma * ((0..12).map(|_| self.next()).sum::<f64>() - 6.0)
        }
    }

    /// Une saison de 30 équipes aux forces connues (points d'écart au meilleur) : marge = forces + 2,5 à domicile + bruit de 12.
    pub(crate) fn simulated_season() -> Vec<StoredGame> {
        let mut rng = Rng(42);
        let strengths: Vec<f64> = (0..30).map(|i| (i as f64 - 14.5) * 0.45).collect();
        let mut games = Vec::new();
        let mut day = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        let mut id = 0;
        while day < NaiveDate::from_ymd_opt(2027, 4, 10).unwrap() {
            for pair in 0..8 {
                id += 1;
                let home = (id * 7 + pair * 3) % 30;
                let away = (home + 1 + (id + pair) % 29) % 30;
                let margin = strengths[home] - strengths[away] + 2.5 + rng.normal(12.0);
                let total = 226.0 + rng.normal(18.0);
                let home_points = ((total + margin) / 2.0).round().max(60.0) as u16;
                let away_points = ((total - margin) / 2.0).round().max(60.0) as u16;
                let time = format!("{}T{}:00:00Z", day.format("%Y-%m-%d"), 23 + pair / 4)
                    .replace("T24", "T23");
                let commence = time.parse().unwrap();
                games.push(StoredGame {
                    event_id: format!("g{id}"),
                    commence_time: commence,
                    season: season_of(commence),
                    home_team: format!("Équipe {home}"),
                    away_team: format!("Équipe {away}"),
                    home_score: home_points,
                    away_score: away_points,
                });
            }
            day = day.succ_opt().unwrap();
        }
        games
    }
}

#[cfg(test)]
mod tests {
    use super::simulation::simulated_season;
    use super::*;

    #[test]
    fn tuning_on_a_simulated_season_finds_a_model_better_than_a_coin_flip_and_sensible_scales() {
        let games = simulated_season();

        let tuned = tune(&games).unwrap();

        assert_eq!(tuned.configs_tried, 54 + 3);
        assert!(
            tuned.validation_log_loss < std::f64::consts::LN_2,
            "{}",
            tuned.validation_log_loss
        );
        assert!(tuned.validation_games > 500, "{}", tuned.validation_games);
        // Le bruit simulé de la marge est de 12 points, celui du total de 18.
        assert!(
            (tuned.margin_sigma - 12.0).abs() < 1.5,
            "{}",
            tuned.margin_sigma
        );
        assert!(
            (tuned.total_sigma - 18.0).abs() < 2.0,
            "{}",
            tuned.total_sigma
        );
        // Une marge attendue positive par point d'Elo, de l'ordre de 1/28 à 1/60.
        assert!(
            tuned.margin_per_elo > 0.0 && tuned.margin_per_elo < 0.1,
            "{}",
            tuned.margin_per_elo
        );
    }

    #[test]
    fn the_validation_window_ends_where_the_test_begins_and_both_exclude_the_warm_up() {
        let games = simulated_season();
        let predictions = walk_forward(&games, EloConfig::starting_point(), 0.06);

        let validation = window(&predictions, VALIDATION_START, Some(TEST_START));
        let test = window(&predictions, TEST_START, None);

        assert!(
            validation
                .iter()
                .all(|p| day_of(p) >= VALIDATION_START && day_of(p) < TEST_START)
        );
        assert!(test.iter().all(|p| day_of(p) >= TEST_START));
        assert!(
            validation
                .iter()
                .all(|p| p.min_games_seen >= MIN_GAMES_SEEN)
        );
        assert!(!validation.is_empty() && !test.is_empty());
    }

    #[test]
    fn no_validation_data_means_no_tuning() {
        assert!(tune(&[]).is_none());
    }
}
