//! Contrôle qualité de l'historique importé : on **signale**, on ne corrige jamais en silence.
//!
//! Quatre contrôles : saisons complètes ou non, résultat publié incohérent avec les buts, cotes dont la
//! somme des probabilités implicites sort de [1,00 ; 1,30], et couverture des statistiques.

use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;

/// Bornes de la somme des probabilités implicites d'un 1X2 réaliste (marge de 0 à 30 %).
pub const IMPLIED_SUM_RANGE: (f64, f64) = (1.0, 1.30);

/// État d'une saison d'une ligue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SeasonStatus {
    /// Tous les matchs attendus sont présents.
    Complete,
    /// Saison en cours : incomplète par nature.
    InProgress,
    /// Il manque des matchs (saison arrêtée, fichier tronqué…).
    Incomplete,
}

/// Une ligue et une saison : matchs présents contre matchs attendus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SeasonCheck {
    /// Code football-data de la ligue.
    pub div: String,
    /// Saison (`2025/26`).
    pub season: String,
    /// Matchs en base.
    pub games: u32,
    /// Équipes distinctes vues dans la saison.
    pub teams: u32,
    /// Matchs attendus pour ce nombre d'équipes.
    pub expected: u32,
    /// Verdict.
    pub status: SeasonStatus,
    /// Matchs avec une cote de clôture Pinnacle.
    pub with_pinnacle_close: u32,
}

/// Cotes hors bornes pour un bookmaker et un moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OddsCheck {
    /// Préfixe de colonne du bookmaker.
    pub bookmaker: String,
    /// `pre` ou `close`.
    pub moment: String,
    /// Triplets de cotes en base.
    pub rows: u32,
    /// Triplets dont la somme des probabilités implicites est hors de [1,00 ; 1,30].
    pub out_of_range: u32,
}

/// Un exemple de cotes aberrantes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OddsSample {
    /// Date du match.
    pub date: String,
    /// Ligue.
    pub div: String,
    /// Équipe à domicile.
    pub home: String,
    /// Équipe à l'extérieur.
    pub away: String,
    /// Bookmaker.
    pub bookmaker: String,
    /// Moment.
    pub moment: String,
    /// Somme des probabilités implicites.
    pub implied_sum: f64,
}

/// Résultat des contrôles.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QcReport {
    /// Une ligne par ligue et par saison.
    pub seasons: Vec<SeasonCheck>,
    /// Matchs dont le résultat publié (`H`/`D`/`A`) contredit les buts.
    pub result_mismatches: u32,
    /// Cotes hors bornes, par bookmaker et moment (le bookmaker `Max` est exclu : voir `run`).
    pub odds: Vec<OddsCheck>,
    /// Les pires exemples de cotes hors bornes.
    pub odds_samples: Vec<OddsSample>,
    /// Matchs sans aucune statistique de tirs.
    pub games_without_shots: u32,
    /// Matchs sans heure de coup d'envoi.
    pub games_without_kickoff: u32,
}

/// Nombre de matchs d'une saison complète pour `teams` équipes : aller-retour (`n(n−1)`) ; la
/// Premiership écossaise joue 33 journées puis 5 dans un groupe de six : 38 matchs par équipe.
fn expected_games(div: &str, teams: u32) -> u32 {
    if div == "SC0" {
        teams * 38 / 2
    } else {
        teams * (teams - 1)
    }
}

/// Lance tous les contrôles. `current_season` (`2026/27`) est signalée « en cours » au lieu d'« incomplète ».
///
/// Le bookmaker `Max` (meilleure cote de chaque issue parmi tous les bookmakers) est exclu du contrôle des
/// sommes : sa somme passe naturellement sous 1,00, ce n'est pas une anomalie. Il ne sert jamais à décider.
pub fn run(conn: &Connection, current_season: &str) -> Result<QcReport> {
    let mut seasons = Vec::new();
    let mut statement = conn.prepare(
        "SELECT g.div, g.season, COUNT(*),
                (SELECT COUNT(*) FROM (
                    SELECT home_id FROM game x WHERE x.div = g.div AND x.season = g.season
                    UNION
                    SELECT away_id FROM game x WHERE x.div = g.div AND x.season = g.season)),
                SUM(EXISTS (SELECT 1 FROM game_odds o
                             WHERE o.game_id = g.id AND o.bookmaker = 'PS' AND o.moment = 'close'))
           FROM game g
          GROUP BY g.div, g.season
          ORDER BY g.div, g.season",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, u32>(3)?,
            row.get::<_, u32>(4)?,
        ))
    })?;
    for row in rows {
        let (div, season, games, teams, with_pinnacle_close) = row?;
        let expected = expected_games(&div, teams);
        let status = if season == current_season {
            SeasonStatus::InProgress
        } else if games == expected {
            SeasonStatus::Complete
        } else {
            SeasonStatus::Incomplete
        };
        seasons.push(SeasonCheck {
            div,
            season,
            games,
            teams,
            expected,
            status,
            with_pinnacle_close,
        });
    }

    let result_mismatches = conn.query_row(
        "SELECT COUNT(*) FROM game
          WHERE (ft_home > ft_away AND ft_result != 'H')
             OR (ft_home = ft_away AND ft_result != 'D')
             OR (ft_home < ft_away AND ft_result != 'A')",
        [],
        |row| row.get(0),
    )?;

    let (low, high) = IMPLIED_SUM_RANGE;
    let mut odds = Vec::new();
    let mut statement = conn.prepare(
        "SELECT bookmaker, moment, COUNT(*),
                SUM(1.0 / home + 1.0 / draw + 1.0 / away < ?1 OR 1.0 / home + 1.0 / draw + 1.0 / away > ?2)
           FROM game_odds
          WHERE bookmaker != 'Max'
          GROUP BY bookmaker, moment
          ORDER BY bookmaker, moment",
    )?;
    for row in statement.query_map([low, high], |row| {
        Ok(OddsCheck {
            bookmaker: row.get(0)?,
            moment: row.get(1)?,
            rows: row.get(2)?,
            out_of_range: row.get(3)?,
        })
    })? {
        odds.push(row?);
    }

    let mut odds_samples = Vec::new();
    let mut statement = conn.prepare(
        "SELECT g.match_date, g.div, th.name, ta.name, o.bookmaker, o.moment,
                1.0 / o.home + 1.0 / o.draw + 1.0 / o.away AS implied_sum
           FROM game_odds o
           JOIN game g ON g.id = o.game_id
           JOIN team th ON th.id = g.home_id
           JOIN team ta ON ta.id = g.away_id
          WHERE o.bookmaker != 'Max' AND (implied_sum < ?1 OR implied_sum > ?2)
          ORDER BY ABS(implied_sum - 1.15) DESC
          LIMIT 10",
    )?;
    for row in statement.query_map([low, high], |row| {
        Ok(OddsSample {
            date: row.get(0)?,
            div: row.get(1)?,
            home: row.get(2)?,
            away: row.get(3)?,
            bookmaker: row.get(4)?,
            moment: row.get(5)?,
            implied_sum: row.get(6)?,
        })
    })? {
        odds_samples.push(row?);
    }

    let games_without_shots = conn.query_row(
        "SELECT COUNT(*) FROM game WHERE shots_home IS NULL",
        [],
        |row| row.get(0),
    )?;
    let games_without_kickoff = conn.query_row(
        "SELECT COUNT(*) FROM game WHERE kickoff_utc IS NULL",
        [],
        |row| row.get(0),
    )?;

    Ok(QcReport {
        seasons,
        result_mismatches,
        odds,
        odds_samples,
        games_without_shots,
        games_without_kickoff,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    fn add_team(conn: &Connection, name: &str) -> i64 {
        conn.execute("INSERT INTO team (name) VALUES (?1)", [name])
            .unwrap();
        conn.last_insert_rowid()
    }

    /// Ajoute un match (sans statistiques ni heure) et renvoie son identifiant.
    fn add_game(
        conn: &Connection,
        season: &str,
        date: &str,
        home: i64,
        away: i64,
        score: (u8, u8),
        result: &str,
    ) -> i64 {
        conn.execute(
            "INSERT INTO game (div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
             VALUES ('E0', ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![season, date, home, away, score.0, score.1, result],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn add_odds(
        conn: &Connection,
        game: i64,
        bookmaker: &str,
        moment: &str,
        odds: (f64, f64, f64),
    ) {
        conn.execute(
            "INSERT INTO game_odds (game_id, bookmaker, moment, home, draw, away) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![game, bookmaker, moment, odds.0, odds.1, odds.2],
        )
        .unwrap();
    }

    #[test]
    fn expected_games_follow_each_league_format() {
        assert_eq!(expected_games("E0", 20), 380);
        assert_eq!(expected_games("D1", 18), 306);
        assert_eq!(expected_games("SC0", 12), 228);
    }

    #[test]
    fn seasons_are_complete_incomplete_or_in_progress() {
        let conn = database();
        let (a, b, c) = (
            add_team(&conn, "A"),
            add_team(&conn, "B"),
            add_team(&conn, "C"),
        );
        // Saison de 3 équipes : 3 × 2 = 6 matchs attendus.
        let pairs = [(a, b), (b, a), (a, c), (c, a), (b, c), (c, b)];
        for (i, (home, away)) in pairs.iter().enumerate() {
            add_game(
                &conn,
                "2024/25",
                &format!("2025-01-{:02}", i + 1),
                *home,
                *away,
                (1, 0),
                "H",
            );
        }
        // Saison à qui il manque un match.
        for (i, (home, away)) in pairs.iter().take(5).enumerate() {
            add_game(
                &conn,
                "2025/26",
                &format!("2026-01-{:02}", i + 1),
                *home,
                *away,
                (1, 0),
                "H",
            );
        }
        // Saison en cours.
        add_game(&conn, "2026/27", "2026-08-01", a, b, (1, 0), "H");
        add_game(&conn, "2026/27", "2026-08-02", b, c, (1, 0), "H");

        let report = run(&conn, "2026/27").unwrap();

        let status = |season: &str| {
            report
                .seasons
                .iter()
                .find(|s| s.season == season)
                .unwrap()
                .status
        };
        assert_eq!(status("2024/25"), SeasonStatus::Complete);
        assert_eq!(status("2025/26"), SeasonStatus::Incomplete);
        assert_eq!(status("2026/27"), SeasonStatus::InProgress);
        assert_eq!(report.seasons[0].teams, 3);
        assert_eq!(report.seasons[0].expected, 6);
    }

    #[test]
    fn a_published_result_that_contradicts_the_score_is_counted() {
        let conn = database();
        let (a, b) = (add_team(&conn, "A"), add_team(&conn, "B"));
        add_game(&conn, "2025/26", "2025-08-01", a, b, (2, 1), "H");
        add_game(&conn, "2025/26", "2025-08-08", b, a, (2, 1), "A");
        add_game(&conn, "2025/26", "2025-08-15", a, b, (1, 1), "H");

        assert_eq!(run(&conn, "2026/27").unwrap().result_mismatches, 2);
    }

    #[test]
    fn odds_outside_the_realistic_range_are_counted_and_sampled_but_max_is_ignored() {
        let conn = database();
        let (a, b) = (add_team(&conn, "A"), add_team(&conn, "B"));
        let game = add_game(&conn, "2025/26", "2025-08-01", a, b, (1, 0), "H");
        add_odds(&conn, game, "PS", "close", (2.0, 3.5, 4.0)); // somme 1,036 : normale
        add_odds(&conn, game, "B365", "close", (2.0, 4.0, 4.0)); // somme 1,000 : à la borne, acceptée
        add_odds(&conn, game, "WH", "pre", (1.5, 2.0, 2.0)); // somme 1,667 : trop haute
        add_odds(&conn, game, "BW", "pre", (3.0, 4.0, 5.0)); // somme 0,783 : arbitrage
        add_odds(&conn, game, "Max", "close", (3.0, 4.0, 5.0)); // ignoré par construction

        let report = run(&conn, "2026/27").unwrap();

        let out = |bookmaker: &str| {
            report
                .odds
                .iter()
                .find(|o| o.bookmaker == bookmaker)
                .unwrap()
                .out_of_range
        };
        assert_eq!(out("PS"), 0);
        assert_eq!(out("B365"), 0);
        assert_eq!(out("WH"), 1);
        assert_eq!(out("BW"), 1);
        assert!(report.odds.iter().all(|o| o.bookmaker != "Max"));
        assert_eq!(report.odds_samples.len(), 2);
        assert!(
            report
                .odds_samples
                .iter()
                .any(|s| s.bookmaker == "WH" && s.home == "A")
        );
    }

    #[test]
    fn pinnacle_close_coverage_and_missing_data_are_reported() {
        let conn = database();
        let (a, b) = (add_team(&conn, "A"), add_team(&conn, "B"));
        let first = add_game(&conn, "2025/26", "2025-08-01", a, b, (1, 0), "H");
        add_game(&conn, "2025/26", "2025-08-08", b, a, (0, 0), "D");
        add_odds(&conn, first, "PS", "close", (2.0, 3.5, 4.0));

        let report = run(&conn, "2026/27").unwrap();

        assert_eq!(report.seasons[0].with_pinnacle_close, 1);
        assert_eq!(report.games_without_shots, 2);
        assert_eq!(report.games_without_kickoff, 2);
    }
}
