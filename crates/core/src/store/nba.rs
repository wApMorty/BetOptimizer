//! Résultats de la NBA (table `nba_game`, migration 9).

use chrono::{DateTime, Datelike, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::journal::{parse_timestamp, timestamp};
use crate::error::Result;
use crate::ingest::odds_api::FinishedGame;

/// Un match terminé relu dans la base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGame {
    /// Identifiant du match (celui de l'archive des cotes).
    pub event_id: String,
    /// Coup d'envoi (UTC).
    pub commence_time: DateTime<Utc>,
    /// Année de début de la saison.
    pub season: i32,
    /// Équipe à domicile.
    pub home_team: String,
    /// Équipe à l'extérieur.
    pub away_team: String,
    /// Points de l'équipe à domicile.
    pub home_score: u16,
    /// Points de l'équipe à l'extérieur.
    pub away_score: u16,
}

/// Année de début de la saison d'un match : d'août à décembre, l'année même ; de janvier à juillet, la précédente.
pub fn season_of(commence_time: DateTime<Utc>) -> i32 {
    if commence_time.month() >= 8 {
        commence_time.year()
    } else {
        commence_time.year() - 1
    }
}

/// Écrit un match terminé, ou met à jour celui qui a le même identifiant (l'API redonne les trois derniers jours).
pub fn upsert_game(
    conn: &Connection,
    game: &FinishedGame,
    fetched_at: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO nba_game (event_id, commence_time, season, home_team, away_team, home_score, away_score, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (event_id) DO UPDATE SET
             commence_time = excluded.commence_time, season = excluded.season, home_team = excluded.home_team,
             away_team = excluded.away_team, home_score = excluded.home_score, away_score = excluded.away_score,
             fetched_at = excluded.fetched_at",
        params![
            game.event_id,
            timestamp(game.commence_time),
            season_of(game.commence_time),
            game.home_team,
            game.away_team,
            game.home_score,
            game.away_score,
            timestamp(fetched_at),
        ],
    )?;
    Ok(())
}

/// Tous les matchs terminés, dans l'ordre chronologique (coup d'envoi, puis identifiant : ordre total et stable).
pub fn load_games(conn: &Connection) -> Result<Vec<StoredGame>> {
    let mut statement = conn.prepare(
        "SELECT event_id, commence_time, season, home_team, away_team, home_score, away_score
         FROM nba_game ORDER BY commence_time, event_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i32>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, u16>(5)?,
            row.get::<_, u16>(6)?,
        ))
    })?;
    let mut games = Vec::new();
    for row in rows {
        let (event_id, commence_time, season, home_team, away_team, home_score, away_score) = row?;
        games.push(StoredGame {
            event_id,
            commence_time: parse_timestamp(&commence_time)?,
            season,
            home_team,
            away_team,
            home_score,
            away_score,
        });
    }
    Ok(games)
}

/// Nombre de matchs terminés en base.
pub fn count_games(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("SELECT COUNT(*) FROM nba_game", [], |row| row.get(0))?)
}

/// Instant du dernier appel à `/scores` de la NBA (journalisé avec le libellé `events`, voir `archive::Endpoint`).
pub fn last_scores_call(conn: &Connection) -> Result<Option<DateTime<Utc>>> {
    let at: Option<String> = conn
        .query_row(
            "SELECT called_at FROM api_call
             WHERE endpoint = 'events' AND sport_key = 'basketball_nba' AND http_status = 200
             ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    at.map(|text| parse_timestamp(&text)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    fn game(id: &str, time: &str, home: u16, away: u16) -> FinishedGame {
        FinishedGame {
            event_id: id.into(),
            commence_time: time.parse().unwrap(),
            home_team: "Detroit Pistons".into(),
            away_team: "Boston Celtics".into(),
            home_score: home,
            away_score: away,
        }
    }

    #[test]
    fn the_season_starts_in_the_autumn_of_its_first_year() {
        let at = |s: &str| season_of(s.parse().unwrap());

        assert_eq!(at("2026-10-20T23:00:00Z"), 2026);
        assert_eq!(at("2027-03-01T01:00:00Z"), 2026);
        assert_eq!(
            at("2027-06-15T00:30:00Z"),
            2026,
            "les finales de juin sont encore la saison 2026"
        );
        assert_eq!(at("2027-10-19T23:00:00Z"), 2027);
    }

    #[test]
    fn a_game_is_stored_once_and_updated_when_the_api_corrects_it() {
        let conn = database();
        let fetched: DateTime<Utc> = "2026-10-21T22:00:00Z".parse().unwrap();

        upsert_game(&conn, &game("a", "2026-10-20T23:00:00Z", 112, 105), fetched).unwrap();
        upsert_game(&conn, &game("a", "2026-10-20T23:00:00Z", 112, 106), fetched).unwrap();
        upsert_game(&conn, &game("b", "2026-10-20T20:00:00Z", 99, 101), fetched).unwrap();

        assert_eq!(count_games(&conn).unwrap(), 2);
        let games = load_games(&conn).unwrap();
        assert_eq!(games[0].event_id, "b", "ordre chronologique");
        assert_eq!((games[1].home_score, games[1].away_score), (112, 106));
        assert_eq!(games[1].season, 2026);
    }

    #[test]
    fn the_last_scores_call_is_the_latest_successful_events_call_for_the_nba() {
        let conn = database();
        assert_eq!(last_scores_call(&conn).unwrap(), None);
        conn.execute_batch(
            "INSERT INTO api_call (called_at, endpoint, sport_key, params, http_status) VALUES
               ('2026-10-21T22:00:00Z', 'events', 'basketball_nba', 'daysFrom=3', 200),
               ('2026-10-22T22:00:00Z', 'events', 'basketball_nba', 'daysFrom=3', 500),
               ('2026-10-23T22:00:00Z', 'odds',   'basketball_nba', 'markets=h2h', 200)",
        )
        .unwrap();

        let last = last_scores_call(&conn).unwrap().unwrap();

        assert_eq!(
            last,
            "2026-10-21T22:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }
}
