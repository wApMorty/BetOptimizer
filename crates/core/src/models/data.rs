//! Données d'entraînement : les matchs d'une ligue, avec indices d'équipes, et leur transformation en
//! `TrainingGame` à une date donnée (poids temporels, cibles éventuellement informées par les tirs).
//!
//! Règle d'or (invariant 1, « aucune fuite du futur ») : un ajustement « à la date D » n'utilise que des matchs
//! **strictement antérieurs** à D. C'est ici, et seulement ici, que cette règle est appliquée.

use std::collections::HashMap;

use chrono::NaiveDate;
use rusqlite::Connection;

use super::TrainingGame;
use super::xg_proxy::XgProxy;
use crate::error::Result;

/// Un match terminé tel qu'il est en base.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoricalGame {
    /// Identifiant du match en base.
    pub game_id: i64,
    /// Date du match.
    pub date: NaiveDate,
    /// Indice de l'équipe à domicile dans sa ligue.
    pub home: usize,
    /// Indice de l'équipe à l'extérieur.
    pub away: usize,
    /// Buts à domicile.
    pub goals_home: u8,
    /// Buts à l'extérieur.
    pub goals_away: u8,
    /// Tirs et tirs cadrés `(domicile, extérieur)` si publiés.
    pub shots: Option<[(u16, u16); 2]>,
    /// xG publiés par football-data (2026/27) `(domicile, extérieur)`.
    pub published_xg: Option<(f64, f64)>,
}

/// Tous les matchs d'une ligue, du plus ancien au plus récent.
#[derive(Debug, Clone, PartialEq)]
pub struct LeagueData {
    /// Code football-data (`E0`…).
    pub div: String,
    /// Noms canoniques des équipes, dans l'ordre de leurs indices.
    pub teams: Vec<String>,
    /// Matchs triés par date puis identifiant.
    pub games: Vec<HistoricalGame>,
}

/// Charge toutes les ligues de la base.
pub fn load_leagues(conn: &Connection) -> Result<Vec<LeagueData>> {
    let mut statement = conn.prepare(
        "SELECT g.id, g.div, g.match_date, th.name, ta.name, g.ft_home, g.ft_away,
                g.shots_home, g.shots_away, g.shots_on_target_home, g.shots_on_target_away, g.xg_home, g.xg_away
           FROM game g
           JOIN team th ON th.id = g.home_id
           JOIN team ta ON ta.id = g.away_id
          ORDER BY g.div, g.match_date, g.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, u8>(5)?,
            row.get::<_, u8>(6)?,
            [
                row.get::<_, Option<u16>>(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
            ],
            (
                row.get::<_, Option<f64>>(11)?,
                row.get::<_, Option<f64>>(12)?,
            ),
        ))
    })?;

    let mut leagues: Vec<LeagueData> = Vec::new();
    let mut team_index: HashMap<(String, String), usize> = HashMap::new();
    for row in rows {
        let (game_id, div, date, home_name, away_name, goals_home, goals_away, shot_cells, xg) =
            row?;
        let Ok(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") else {
            continue;
        };
        if leagues.last().is_none_or(|league| league.div != div) {
            leagues.push(LeagueData {
                div: div.clone(),
                teams: Vec::new(),
                games: Vec::new(),
            });
        }
        let Some(league) = leagues.last_mut() else {
            continue;
        };
        let mut index_of = |name: String, league: &mut LeagueData| -> usize {
            let next = league.teams.len();
            let index = *team_index
                .entry((div.clone(), name.clone()))
                .or_insert(next);
            if index == next {
                league.teams.push(name);
            }
            index
        };
        let home = index_of(home_name, league);
        let away = index_of(away_name, league);
        // Tirs : les quatre valeurs doivent être présentes, et un tir cadré est un tir.
        let shots = match shot_cells {
            [
                Some(shots_home),
                Some(shots_away),
                Some(on_target_home),
                Some(on_target_away),
            ] if on_target_home <= shots_home && on_target_away <= shots_away => {
                Some([(shots_home, on_target_home), (shots_away, on_target_away)])
            }
            _ => None,
        };
        league.games.push(HistoricalGame {
            game_id,
            date,
            home,
            away,
            goals_home,
            goals_away,
            shots,
            published_xg: xg.0.zip(xg.1),
        });
    }
    Ok(leagues)
}

/// Matchs d'entraînement « à la date `as_of` » : uniquement les matchs **strictement antérieurs**.
///
/// - poids temporel `exp(−ξ · jours séparant le match de as_of)` (Dixon & Coles, 1997, section 4) ;
/// - cible de la partie Poisson : `(1 − blend)·buts + blend·buts_attendus(tirs)` si un proxy est fourni et si les
///   tirs du match sont connus, sinon les buts réels.
pub fn training_games(
    league: &LeagueData,
    as_of: NaiveDate,
    decay: f64,
    blend: f64,
    proxy: Option<&XgProxy>,
) -> Vec<TrainingGame> {
    league
        .games
        .iter()
        .filter(|game| game.date < as_of)
        .map(|game| {
            let age_days = (as_of - game.date).num_days() as f64;
            let (goals_home, goals_away) = (f64::from(game.goals_home), f64::from(game.goals_away));
            let (target_home, target_away) = match (proxy, game.shots) {
                (Some(proxy), Some([(shots_home, on_home), (shots_away, on_away)]))
                    if blend > 0.0 =>
                {
                    (
                        (1.0 - blend) * goals_home + blend * proxy.expected(shots_home, on_home),
                        (1.0 - blend) * goals_away + blend * proxy.expected(shots_away, on_away),
                    )
                }
                _ => (goals_home, goals_away),
            };
            TrainingGame {
                home: game.home,
                away: game.away,
                goals_home: game.goals_home,
                goals_away: game.goals_away,
                target_home,
                target_away,
                weight: (-decay * age_days).exp(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;
    use crate::store::migrate;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn league_with(games: Vec<HistoricalGame>) -> LeagueData {
        LeagueData {
            div: "E0".into(),
            teams: vec!["A".into(), "B".into()],
            games,
        }
    }

    fn played(date: NaiveDate, goals: (u8, u8), shots: Option<[(u16, u16); 2]>) -> HistoricalGame {
        HistoricalGame {
            game_id: 0,
            date,
            home: 0,
            away: 1,
            goals_home: goals.0,
            goals_away: goals.1,
            shots,
            published_xg: None,
        }
    }

    #[test]
    fn only_games_strictly_before_the_date_are_used() {
        let league = league_with(vec![
            played(day(2025, 8, 1), (1, 0), None),
            played(day(2025, 8, 8), (2, 2), None),
            played(day(2025, 8, 9), (3, 0), None), // le jour même : exclu
            played(day(2025, 8, 15), (0, 1), None), // futur : exclu
        ]);

        let training = training_games(&league, day(2025, 8, 9), 0.0, 0.0, None);

        assert_eq!(training.len(), 2);
        assert_eq!(training[1].goals_home, 2);
    }

    #[test]
    fn weights_decay_with_the_age_of_the_game() {
        let league = league_with(vec![
            played(day(2025, 1, 1), (1, 0), None),
            played(day(2025, 7, 1), (1, 0), None),
        ]);

        let training = training_games(&league, day(2025, 8, 1), 0.002, 0.0, None);

        // 212 jours et 31 jours avant la date : exp(−0,002 × âge).
        assert_abs_diff_eq!(
            training[0].weight,
            (-0.002_f64 * 212.0).exp(),
            epsilon = 1e-12
        );
        assert_abs_diff_eq!(
            training[1].weight,
            (-0.002_f64 * 31.0).exp(),
            epsilon = 1e-12
        );
        assert!(training[1].weight > training[0].weight);
    }

    #[test]
    fn the_target_blends_goals_and_shot_based_expected_goals() {
        let shots = Some([(20, 10), (8, 2)]);
        let league = league_with(vec![
            played(day(2025, 8, 1), (3, 0), shots),
            played(day(2025, 8, 2), (1, 1), None),
        ]);
        // Proxy : 0,3 par tir cadré, 0,05 par tir non cadré → domicile 3,5 ; extérieur 0,3×2 + 0,05×6 = 0,9.
        let proxy = XgProxy {
            on_target: 0.3,
            off_target: 0.05,
        };

        let pure_goals = training_games(&league, day(2025, 9, 1), 0.0, 0.0, Some(&proxy));
        let half = training_games(&league, day(2025, 9, 1), 0.0, 0.5, Some(&proxy));
        let all_shots = training_games(&league, day(2025, 9, 1), 0.0, 1.0, Some(&proxy));

        assert_eq!(pure_goals[0].target_home, 3.0);
        assert_abs_diff_eq!(half[0].target_home, 0.5 * 3.0 + 0.5 * 3.5, epsilon = 1e-12);
        assert_abs_diff_eq!(all_shots[0].target_home, 3.5, epsilon = 1e-12);
        assert_abs_diff_eq!(all_shots[0].target_away, 0.9, epsilon = 1e-12);
        // Un match sans tirs retombe sur les buts réels, et les buts réels restent disponibles pour τ.
        assert_eq!(all_shots[1].target_home, 1.0);
        assert_eq!(all_shots[0].goals_home, 3);
    }

    #[test]
    fn leagues_are_loaded_with_team_indices_and_shots() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'A'), (2, 'B'), (3, 'C');
             INSERT INTO game (id, div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result,
                               shots_home, shots_away, shots_on_target_home, shots_on_target_away, xg_home, xg_away) VALUES
               (1, 'E0', '2025/26', '2025-08-01', 1, 2, 2, 1, 'H', 12, 8, 5, 3, NULL, NULL),
               (2, 'E0', '2025/26', '2025-08-08', 2, 3, 0, 0, 'D', NULL, NULL, NULL, NULL, 1.1, 0.7),
               (3, 'SC0', '2025/26', '2025-08-02', 3, 1, 1, 3, 'A', 9, 10, 20, 2, NULL, NULL);",
        )
        .unwrap();

        let leagues = load_leagues(&conn).unwrap();

        assert_eq!(leagues.len(), 2);
        let e0 = &leagues[0];
        assert_eq!(e0.div, "E0");
        assert_eq!(e0.teams, vec!["A", "B", "C"]);
        assert_eq!(e0.games[0].shots, Some([(12, 5), (8, 3)]));
        assert_eq!(e0.games[1].shots, None);
        assert_eq!(e0.games[1].published_xg, Some((1.1, 0.7)));
        // Les indices d'équipes sont propres à chaque ligue.
        assert_eq!(leagues[1].teams, vec!["C", "A"]);
        // Tirs cadrés (20) > tirs (9) : donnée incohérente, ignorée.
        assert_eq!(leagues[1].games[0].shots, None);
    }
}
