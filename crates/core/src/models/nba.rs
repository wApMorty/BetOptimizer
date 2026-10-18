//! NBA : un Elo avec marge de victoire, l'avantage du terrain et le repos, et un modèle de total de points.
//!
//! **Elo.** Probabilité que l'équipe à domicile gagne, avec `écart = r_dom + avantage − pénalité_repos_dom − (r_ext − pénalité_repos_ext)` :
//!
//! ```text
//! P(dom gagne) = 1 / (1 + 10^(−écart / 400))        marge attendue = écart / points_par_elo
//! ```
//!
//! Après un match : `Δ = K · multiplicateur · (S − P)`, où `S` vaut 1 si l'équipe à domicile a gagné, et
//! `multiplicateur = (marge + 3)^0,8 / (7,5 + 0,006 · écart_du_vainqueur)` : une victoire large pèse plus, mais avec des
//! rendements décroissants, et une victoire du favori pèse moins que celle de l'outsider (l'écart du vainqueur est négatif
//! quand l'outsider gagne). Au début d'une saison, les classements reviennent vers 1500 : `r ← carry·r + (1 − carry)·1500`.
//!
//! Forme du multiplicateur, avantage du terrain de 100 points et idée du repos : FiveThirtyEight (NBA/WNBA). Leurs pages ont
//! disparu et n'ont été recoupées que par un résumé de recherche (`docs/sources.md` §23) : ce sont des **points de départ**,
//! tous réglés sur les données, présentés comme des choix du projet.
//!
//! **Total.** Chaque équipe a une « tendance au total » (rythme et scoring des matchs auxquels elle participe), apprise en ligne :
//! `total_prévu = moyenne + t_dom + t_ext`, et après le match `t ← t + taux · (total − total_prévu)` pour les deux équipes.
//!
//! **Loi normale.** Marge et total sont supposés gaussiens autour de ces prévisions, d'écart-type estimé sur les résidus :
//! `P(handicap couvert) = Φ((marge + handicap_dom) / σ)`, `P(plus) = Φ((total − ligne) / σ)`.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use statrs::distribution::{ContinuousCDF, Normal};

use crate::store::nba::StoredGame;

/// Classement de départ et valeur vers laquelle on revient entre deux saisons.
pub const INITIAL_RATING: f64 = 1500.0;

/// Au-dessous de ce nombre d'heures entre deux coups d'envoi, l'équipe joue un « dos à dos » (deux soirs de suite).
const BACK_TO_BACK_HOURS: i64 = 30;

/// Total moyen par match avant d'avoir vu le moindre résultat (la NBA récente tourne autour de 228 points).
const PRIOR_TOTAL: f64 = 228.0;

/// Poids, en matchs, de ce total de départ dans la moyenne apprise.
const PRIOR_WEIGHT: f64 = 20.0;

/// Hyperparamètres de l'Elo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EloConfig {
    /// Facteur `K`.
    pub k: f64,
    /// Avantage du terrain, en points d'Elo.
    pub home_advantage: f64,
    /// Pénalité de l'équipe qui joue un dos à dos, en points d'Elo.
    pub back_to_back: f64,
    /// Part du classement conservée d'une saison à l'autre.
    pub carry_over: f64,
}

impl EloConfig {
    /// Point de départ avant réglage.
    pub fn starting_point() -> EloConfig {
        EloConfig {
            k: 20.0,
            home_advantage: 100.0,
            back_to_back: 30.0,
            carry_over: 0.75,
        }
    }
}

/// L'état de l'Elo.
#[derive(Debug, Clone)]
pub struct Elo {
    config: EloConfig,
    ratings: HashMap<String, f64>,
    last_start: HashMap<String, DateTime<Utc>>,
    season: Option<i32>,
}

/// `1 / (1 + 10^(−écart / 400))`.
pub fn win_probability(gap: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-gap / 400.0))
}

impl Elo {
    /// Un Elo vide.
    pub fn new(config: EloConfig) -> Elo {
        Elo {
            config,
            ratings: HashMap::new(),
            last_start: HashMap::new(),
            season: None,
        }
    }

    /// Classement d'une équipe (1500 si elle est inconnue).
    pub fn rating(&self, team: &str) -> f64 {
        self.ratings.get(team).copied().unwrap_or(INITIAL_RATING)
    }

    /// L'équipe joue-t-elle un dos à dos au coup d'envoi `start` ? Inconnu (aucun match vu) : non.
    pub fn is_back_to_back(&self, team: &str, start: DateTime<Utc>) -> bool {
        self.last_start
            .get(team)
            .is_some_and(|last| start - *last < Duration::hours(BACK_TO_BACK_HOURS))
    }

    /// Écart d'Elo du point de vue de l'équipe à domicile, avantage du terrain et repos compris.
    pub fn gap(&self, home: &str, away: &str, start: DateTime<Utc>) -> f64 {
        let tired = |team: &str| {
            if self.is_back_to_back(team, start) {
                self.config.back_to_back
            } else {
                0.0
            }
        };
        (self.rating(home) - tired(home)) + self.config.home_advantage
            - (self.rating(away) - tired(away))
    }

    /// Fait revenir tous les classements vers 1500 quand une nouvelle saison commence.
    fn start_season(&mut self, season: i32) {
        if self.season.is_some_and(|current| current != season) {
            for rating in self.ratings.values_mut() {
                *rating = self.config.carry_over * *rating
                    + (1.0 - self.config.carry_over) * INITIAL_RATING;
            }
            self.last_start.clear();
        }
        self.season = Some(season);
    }

    /// Enregistre un résultat.
    pub fn update(&mut self, game: &StoredGame) {
        self.start_season(game.season);
        let gap = self.gap(&game.home_team, &game.away_team, game.commence_time);
        let expected = win_probability(gap);
        let home_won = game.home_score > game.away_score;
        let margin = f64::from(game.home_score.abs_diff(game.away_score));
        let winner_gap = if home_won { gap } else { -gap };
        let multiplier = (margin + 3.0).powf(0.8) / (7.5 + 0.006 * winner_gap);
        let delta = self.config.k * multiplier * (f64::from(u8::from(home_won)) - expected);
        *self
            .ratings
            .entry(game.home_team.clone())
            .or_insert(INITIAL_RATING) += delta;
        *self
            .ratings
            .entry(game.away_team.clone())
            .or_insert(INITIAL_RATING) -= delta;
        self.last_start
            .insert(game.home_team.clone(), game.commence_time);
        self.last_start
            .insert(game.away_team.clone(), game.commence_time);
    }
}

/// Modèle de total : une tendance par équipe, apprise en ligne.
#[derive(Debug, Clone)]
pub struct TotalsModel {
    rate: f64,
    tendencies: HashMap<String, f64>,
    sum: f64,
    count: f64,
}

impl TotalsModel {
    /// Un modèle vide ; `rate` est le pas d'apprentissage (0,03 à 0,1).
    pub fn new(rate: f64) -> TotalsModel {
        TotalsModel {
            rate,
            tendencies: HashMap::new(),
            sum: 0.0,
            count: 0.0,
        }
    }

    fn league_mean(&self) -> f64 {
        (PRIOR_TOTAL * PRIOR_WEIGHT + self.sum) / (PRIOR_WEIGHT + self.count)
    }

    fn tendency(&self, team: &str) -> f64 {
        self.tendencies.get(team).copied().unwrap_or(0.0)
    }

    /// Total de points prévu pour ce match.
    pub fn predict(&self, home: &str, away: &str) -> f64 {
        self.league_mean() + self.tendency(home) + self.tendency(away)
    }

    /// Enregistre un résultat.
    pub fn update(&mut self, game: &StoredGame) {
        let total = f64::from(game.home_score) + f64::from(game.away_score);
        let error = total - self.predict(&game.home_team, &game.away_team);
        *self.tendencies.entry(game.home_team.clone()).or_insert(0.0) += self.rate * error;
        *self.tendencies.entry(game.away_team.clone()).or_insert(0.0) += self.rate * error;
        self.sum += total;
        self.count += 1.0;
    }
}

/// La prévision d'un match, faite avant son résultat.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// Identifiant du match.
    pub event_id: String,
    /// Coup d'envoi.
    pub commence_time: DateTime<Utc>,
    /// Saison.
    pub season: i32,
    /// Équipe à domicile.
    pub home_team: String,
    /// Équipe à l'extérieur.
    pub away_team: String,
    /// Écart d'Elo vu de l'équipe à domicile (avantage et repos compris).
    pub gap: f64,
    /// Probabilité que l'équipe à domicile gagne.
    pub p_home: f64,
    /// Total de points prévu.
    pub total: f64,
    /// Matchs déjà vus de l'équipe qui en a vu le moins : un classement neuf ne sait rien.
    pub min_games_seen: u32,
    /// Résultat réel : marge de l'équipe à domicile (points).
    pub actual_margin: i32,
    /// Résultat réel : total de points.
    pub actual_total: u32,
}

/// Rejoue la saison dans l'ordre : chaque match est prévu avec ce que l'on savait **avant** son coup d'envoi, puis appris.
///
/// Une équipe ne joue qu'une fois par jour : prévoir puis apprendre match après match ne mélange jamais deux matchs d'une même
/// équipe. Les matchs reçus dans le désordre sont triés par coup d'envoi (tri stable, puis identifiant).
pub fn walk_forward(games: &[StoredGame], elo: EloConfig, totals_rate: f64) -> Vec<Prediction> {
    let mut ordered: Vec<&StoredGame> = games.iter().collect();
    ordered.sort_by(|a, b| (a.commence_time, &a.event_id).cmp(&(b.commence_time, &b.event_id)));

    let mut elo_state = Elo::new(elo);
    let mut totals = TotalsModel::new(totals_rate);
    let mut seen: HashMap<&str, u32> = HashMap::new();
    let mut predictions = Vec::with_capacity(ordered.len());
    for game in ordered {
        // Le retour vers la moyenne d'une nouvelle saison doit précéder la prévision de son premier match.
        elo_state.start_season(game.season);
        let gap = elo_state.gap(&game.home_team, &game.away_team, game.commence_time);
        predictions.push(Prediction {
            event_id: game.event_id.clone(),
            commence_time: game.commence_time,
            season: game.season,
            home_team: game.home_team.clone(),
            away_team: game.away_team.clone(),
            gap,
            p_home: win_probability(gap),
            total: totals.predict(&game.home_team, &game.away_team),
            min_games_seen: seen
                .get(game.home_team.as_str())
                .copied()
                .unwrap_or(0)
                .min(seen.get(game.away_team.as_str()).copied().unwrap_or(0)),
            actual_margin: i32::from(game.home_score) - i32::from(game.away_score),
            actual_total: u32::from(game.home_score) + u32::from(game.away_score),
        });
        elo_state.update(game);
        totals.update(game);
        *seen.entry(game.home_team.as_str()).or_default() += 1;
        *seen.entry(game.away_team.as_str()).or_default() += 1;
    }
    predictions
}

/// `P(l'équipe à domicile couvre le handicap)` : la marge réelle dépasse `−handicap_dom` (un handicap de −5,5 donne 5,5 points
/// d'avance à l'adversaire). `margin_mean` est la marge prévue pour l'équipe à domicile.
pub fn cover_probability(margin_mean: f64, sigma: f64, home_spread: f64) -> f64 {
    normal_cdf((margin_mean + home_spread) / sigma)
}

/// `P(le total dépasse la ligne)`.
pub fn over_probability(total_mean: f64, sigma: f64, line: f64) -> f64 {
    normal_cdf((total_mean - line) / sigma)
}

/// Fonction de répartition de la loi normale centrée réduite.
fn normal_cdf(z: f64) -> f64 {
    match Normal::new(0.0, 1.0) {
        Ok(standard) => standard.cdf(z),
        // `Normal::new(0, 1)` ne peut pas échouer ; la branche existe pour ne pas paniquer si la bibliothèque évolue.
        Err(_) => 0.5,
    }
}

/// Pente de la régression de la marge réelle sur l'écart d'Elo, sans ordonnée : `Σ écart·marge / Σ écart²`.
/// Son inverse est le nombre de points d'Elo par point de marge (≈ 28 chez FiveThirtyEight).
pub fn margin_per_elo(predictions: &[&Prediction]) -> Option<f64> {
    let numerator: f64 = predictions
        .iter()
        .map(|p| p.gap * f64::from(p.actual_margin))
        .sum();
    let denominator: f64 = predictions.iter().map(|p| p.gap * p.gap).sum();
    (denominator > 0.0 && numerator > 0.0).then(|| numerator / denominator)
}

/// Écart-type d'un échantillon de résidus (racine de l'erreur quadratique moyenne).
pub fn rmse(residuals: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, count) = residuals.fold((0.0, 0usize), |(s, c), r| (s + r * r, c + 1));
    (count > 0).then(|| (sum / count as f64).sqrt())
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn game(
        id: &str,
        time: &str,
        home: &str,
        away: &str,
        home_score: u16,
        away_score: u16,
    ) -> StoredGame {
        StoredGame {
            event_id: id.into(),
            commence_time: time.parse().unwrap(),
            season: crate::store::nba::season_of(time.parse().unwrap()),
            home_team: home.into(),
            away_team: away.into(),
            home_score,
            away_score,
        }
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    #[test]
    fn equal_teams_at_home_win_more_often_than_one_in_two() {
        let elo = Elo::new(EloConfig::starting_point());

        let gap = elo.gap("A", "B", at("2026-10-20T23:00:00Z"));

        assert_abs_diff_eq!(gap, 100.0, epsilon = 1e-12);
        assert_abs_diff_eq!(win_probability(gap), 0.640, epsilon = 1e-3);
        assert_abs_diff_eq!(win_probability(0.0), 0.5, epsilon = 1e-12);
    }

    #[test]
    fn a_win_moves_the_ratings_in_opposite_directions_by_the_same_amount() {
        let mut elo = Elo::new(EloConfig::starting_point());

        elo.update(&game("1", "2026-10-20T23:00:00Z", "A", "B", 110, 100));

        let up = elo.rating("A") - INITIAL_RATING;
        let down = INITIAL_RATING - elo.rating("B");
        assert!(up > 0.0);
        assert_abs_diff_eq!(up, down, epsilon = 1e-9);
    }

    #[test]
    fn the_margin_multiplier_follows_the_published_shape() {
        // Équipes égales, A à domicile gagne de 10 : écart = 100 (avantage), P = 0,640, multiplicateur = 13^0,8 / (7,5 + 0,6).
        let mut elo = Elo::new(EloConfig::starting_point());
        elo.update(&game("1", "2026-10-20T23:00:00Z", "A", "B", 110, 100));

        let multiplier = 13f64.powf(0.8) / (7.5 + 0.006 * 100.0);
        let expected = 20.0 * multiplier * (1.0 - win_probability(100.0));
        assert_abs_diff_eq!(elo.rating("A") - INITIAL_RATING, expected, epsilon = 1e-9);
    }

    #[test]
    fn a_blowout_counts_more_than_a_narrow_win_but_less_than_proportionally() {
        let delta = |margin: u16| {
            let mut elo = Elo::new(EloConfig::starting_point());
            elo.update(&game(
                "1",
                "2026-10-20T23:00:00Z",
                "A",
                "B",
                100 + margin,
                100,
            ));
            elo.rating("A") - INITIAL_RATING
        };

        assert!(delta(20) > delta(5));
        assert!(delta(20) < 4.0 * delta(5), "rendements décroissants");
    }

    #[test]
    fn an_upset_moves_the_ratings_more_than_the_favourite_winning_by_the_same_margin() {
        let mut base = Elo::new(EloConfig::starting_point());
        for i in 0..20 {
            base.update(&game(
                &format!("w{i}"),
                &format!("2026-10-{:02}T23:00:00Z", 1 + i),
                "Fort",
                "Faible",
                120,
                100,
            ));
        }
        let mut expected = base.clone();
        let mut upset = base.clone();
        let when = "2026-11-15T23:00:00Z";

        expected.update(&game("e", when, "Fort", "Faible", 110, 100));
        upset.update(&game("u", when, "Faible", "Fort", 110, 100));

        let gain_expected = expected.rating("Fort") - base.rating("Fort");
        let gain_upset = upset.rating("Faible") - base.rating("Faible");
        assert!(gain_upset > gain_expected);
    }

    #[test]
    fn a_back_to_back_lowers_the_gap_of_the_tired_team() {
        let mut elo = Elo::new(EloConfig::starting_point());
        elo.update(&game("1", "2026-10-20T23:00:00Z", "A", "C", 100, 100 - 1));
        let rested = elo.gap("B", "A", at("2026-10-23T23:00:00Z"));
        let tired = elo.gap("B", "A", at("2026-10-21T23:30:00Z"));

        assert!(elo.is_back_to_back("A", at("2026-10-21T23:30:00Z")));
        assert!(!elo.is_back_to_back("A", at("2026-10-23T23:00:00Z")));
        // A, qui joue à l'extérieur, est fatigué : l'écart de B (à domicile) augmente de la pénalité.
        assert_abs_diff_eq!(tired - rested, 30.0, epsilon = 1e-9);
    }

    #[test]
    fn ratings_return_toward_1500_when_a_new_season_starts() {
        let mut elo = Elo::new(EloConfig::starting_point());
        for i in 0..30 {
            elo.update(&game(
                &format!("a{i}"),
                &format!("2026-11-{:02}T23:00:00Z", 1 + i % 28),
                "A",
                "B",
                120,
                100,
            ));
        }
        let before = elo.rating("A");
        assert!(before > 1520.0);

        elo.update(&game("n", "2027-10-25T23:00:00Z", "C", "D", 100, 99));

        let after = elo.rating("A");
        assert_abs_diff_eq!(after, 0.75 * before + 0.25 * INITIAL_RATING, epsilon = 1e-9);
    }

    #[test]
    fn predictions_never_see_their_own_result_or_the_future() {
        let past = vec![
            game("1", "2026-10-20T23:00:00Z", "A", "B", 110, 100),
            game("2", "2026-10-22T23:00:00Z", "B", "C", 99, 101),
            game("3", "2026-10-24T23:00:00Z", "A", "C", 105, 104),
        ];
        let mut other_future = past.clone();
        other_future.push(game("4", "2026-11-01T23:00:00Z", "C", "A", 130, 90));
        let mut another_future = past.clone();
        another_future.push(game("4", "2026-11-01T23:00:00Z", "A", "C", 130, 90));

        let reference = walk_forward(&past, EloConfig::starting_point(), 0.05);
        let first = walk_forward(&other_future, EloConfig::starting_point(), 0.05);
        let second = walk_forward(&another_future, EloConfig::starting_point(), 0.05);

        for index in 0..past.len() {
            assert_eq!(reference[index].p_home, first[index].p_home);
            assert_eq!(reference[index].total, second[index].total);
        }
        // Et le futur, lui, change bien (le test de fuite a des dents).
        assert_ne!(first[3].p_home, second[3].p_home);
        // Le tout premier match est prévu avec des classements vierges.
        assert_abs_diff_eq!(reference[0].p_home, win_probability(100.0), epsilon = 1e-12);
        assert_eq!(reference[0].min_games_seen, 0);
        assert_eq!(reference[2].min_games_seen, 1);
    }

    #[test]
    fn games_given_out_of_order_are_replayed_in_time_order() {
        let ordered = vec![
            game("1", "2026-10-20T23:00:00Z", "A", "B", 110, 100),
            game("2", "2026-10-22T23:00:00Z", "A", "C", 99, 101),
        ];
        let shuffled = vec![ordered[1].clone(), ordered[0].clone()];

        let a = walk_forward(&ordered, EloConfig::starting_point(), 0.05);
        let b = walk_forward(&shuffled, EloConfig::starting_point(), 0.05);

        assert_eq!(a, b);
    }

    #[test]
    fn the_totals_model_learns_that_a_team_plays_high_scoring_games() {
        let mut model = TotalsModel::new(0.08);
        assert_abs_diff_eq!(model.predict("A", "B"), PRIOR_TOTAL, epsilon = 1e-12);

        // « Rapide » joue des matchs à 250 points, « Lent » des matchs à 206 : 44 points d'écart entre les deux.
        for i in 0..120 {
            let day = 1 + i % 28;
            let (rapide, lent) = (
                game(
                    &format!("r{i}"),
                    &format!("2026-11-{day:02}T23:00:00Z"),
                    "Rapide",
                    &format!("A{}", i % 5),
                    130,
                    120,
                ),
                game(
                    &format!("l{i}"),
                    &format!("2026-11-{day:02}T23:30:00Z"),
                    "Lent",
                    &format!("B{}", i % 5),
                    106,
                    100,
                ),
            );
            model.update(&rapide);
            model.update(&lent);
        }

        let gap = model.predict("Rapide", "Z") - model.predict("Lent", "Z");
        assert!(gap > 30.0, "{gap}");
        assert!(gap < 50.0, "{gap}");
    }

    #[test]
    fn normal_probabilities_are_one_half_on_the_forecast_and_symmetric() {
        // Marge prévue +5 pour l'équipe à domicile, handicap −5 : couvre une fois sur deux.
        assert_abs_diff_eq!(cover_probability(5.0, 12.0, -5.0), 0.5, epsilon = 1e-12);
        assert!(cover_probability(5.0, 12.0, -3.0) > 0.5);
        assert_abs_diff_eq!(
            cover_probability(5.0, 12.0, -8.0) + cover_probability(-5.0, 12.0, 8.0),
            1.0,
            epsilon = 1e-12
        );
        assert_abs_diff_eq!(over_probability(228.0, 18.0, 228.0), 0.5, epsilon = 1e-12);
        assert!(over_probability(235.0, 18.0, 228.0) > 0.6);
    }

    #[test]
    fn the_margin_slope_and_the_rmse_follow_a_hand_computation() {
        let make = |gap: f64, margin: i32| Prediction {
            event_id: "x".into(),
            commence_time: at("2026-10-20T23:00:00Z"),
            season: 2026,
            home_team: "A".into(),
            away_team: "B".into(),
            gap,
            p_home: 0.5,
            total: 228.0,
            min_games_seen: 10,
            actual_margin: margin,
            actual_total: 228,
        };
        let predictions = [make(100.0, 4), make(-50.0, -2), make(200.0, 8)];
        let refs: Vec<&Prediction> = predictions.iter().collect();

        // Σ écart·marge = 400 + 100 + 1600 = 2100 ; Σ écart² = 10000 + 2500 + 40000 = 52500 ; pente = 0,04 (25 Elo par point).
        assert_abs_diff_eq!(margin_per_elo(&refs).unwrap(), 0.04, epsilon = 1e-12);
        assert_abs_diff_eq!(
            rmse([3.0, 4.0].into_iter()).unwrap(),
            (12.5f64).sqrt(),
            epsilon = 1e-12
        );
        assert_eq!(rmse(std::iter::empty()), None);
    }
}
