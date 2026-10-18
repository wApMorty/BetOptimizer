//! Outils de test partagés : ligues simulées à paramètres connus (déterministes, sans dépendance).

use chrono::{Days, NaiveDate};

use super::FittedModel;
use super::data::{HistoricalGame, LeagueData};
use super::grid::ScoreGrid;

/// Générateur congruentiel (Knuth, MMIX).
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

/// Tire un score dans une grille (inversion de la fonction de répartition).
pub fn sample_score(grid: &ScoreGrid, rng: &mut Lcg) -> (u8, u8) {
    let mut remaining = rng.next();
    for x in 0..=10 {
        for y in 0..=10 {
            remaining -= grid.probability(x, y);
            if remaining <= 0.0 {
                return (x as u8, y as u8);
            }
        }
    }
    (0, 0)
}

/// Vérité terrain : 8 équipes aux forces connues.
pub fn truth() -> FittedModel {
    FittedModel {
        attack: vec![0.35, 0.2, 0.1, 0.0, -0.05, -0.15, -0.2, -0.25],
        defence: vec![-0.3, -0.15, -0.05, 0.0, 0.05, 0.1, 0.2, 0.25],
        intercept: 0.1,
        home_advantage: 0.3,
        rho: -0.12,
    }
}

/// Ligue simulée : `seasons` saisons de 56 matchs (aller-retour à 8 équipes), 4 matchs par semaine, un samedi.
/// Les tirs sont fabriqués à partir des buts (cadrés ≈ 3 × buts + bruit, tirs = 2 × cadrés + 3).
pub fn simulated_league(seasons: usize, seed: u64) -> LeagueData {
    let truth = truth();
    let mut rng = Lcg(seed);
    let mut games = Vec::new();
    let mut game_id = 0;
    for season in 0..seasons {
        // Ordre des 56 rencontres mélangé de façon déterministe (Fisher-Yates).
        let mut fixtures: Vec<(usize, usize)> = (0..8)
            .flat_map(|h| (0..8).filter(move |a| *a != h).map(move |a| (h, a)))
            .collect();
        for i in (1..fixtures.len()).rev() {
            let j = (rng.next() * (i + 1) as f64) as usize;
            fixtures.swap(i, j.min(i));
        }
        let season_start =
            NaiveDate::from_ymd_opt(2020, 8, 1).unwrap() + Days::new(season as u64 * 365);
        for (index, (home, away)) in fixtures.into_iter().enumerate() {
            let week = (index / 4) as u64;
            let date = season_start + Days::new(week * 7);
            let grid = truth.grid(home, away).unwrap();
            let (goals_home, goals_away) = sample_score(&grid, &mut rng);
            let on_target = |goals: u8, rng: &mut Lcg| {
                (u16::from(goals) * 3 + (rng.next() * 3.0) as u16).max(1)
            };
            let (on_home, on_away) = (
                on_target(goals_home, &mut rng),
                on_target(goals_away, &mut rng),
            );
            game_id += 1;
            games.push(HistoricalGame {
                game_id,
                date,
                home,
                away,
                goals_home,
                goals_away,
                shots: Some([(on_home * 2 + 3, on_home), (on_away * 2 + 3, on_away)]),
                published_xg: None,
            });
        }
    }
    games.sort_by_key(|g| (g.date, g.game_id));
    LeagueData {
        div: "E0".into(),
        teams: (0..8).map(|i| format!("Equipe {i}")).collect(),
        games,
    }
}
