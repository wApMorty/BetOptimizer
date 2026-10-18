//! Écriture de l'historique des matchs et des cotes (football-data).
//!
//! Contrairement à l'archive des cotes en direct, ces données se mettent à jour : un match déjà présent est
//! mis à jour plutôt que dupliqué (clé naturelle : ligue, date, équipes), ce qui rend l'import idempotent.

use chrono::SecondsFormat;
use rusqlite::{Connection, OptionalExtension, named_params, params};

use crate::domain::{BookOdds, MatchRecord, Outcome, TotalsOdds};
use crate::error::Result;

/// Identifiant d'une équipe par son nom canonique, créée au besoin.
pub fn team_id(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute("INSERT OR IGNORE INTO team (name) VALUES (?1)", [name])?;
    let id = conn.query_row("SELECT id FROM team WHERE name = ?1", [name], |row| {
        row.get(0)
    })?;
    Ok(id)
}

fn result_code(result: Outcome) -> &'static str {
    match result {
        Outcome::Home => "H",
        Outcome::Draw => "D",
        Outcome::Away => "A",
    }
}

/// Insère ou met à jour un match et renvoie son identifiant (stable d'un import à l'autre).
pub fn upsert_game(
    conn: &Connection,
    div: &str,
    season: &str,
    record: &MatchRecord,
    home_id: i64,
    away_id: i64,
) -> Result<i64> {
    // `named_params!` associe chaque valeur à un nom (`:div`) : plus lisible qu'une trentaine de `?`.
    let mut statement = conn.prepare_cached(
        "INSERT INTO game (
             div, season, match_date, kickoff_utc, home_id, away_id, ft_home, ft_away, ft_result,
             ht_home, ht_away, referee,
             shots_home, shots_away, shots_on_target_home, shots_on_target_away,
             corners_home, corners_away, fouls_home, fouls_away,
             yellow_home, yellow_away, red_home, red_away, xg_home, xg_away)
         VALUES (
             :div, :season, :date, :kickoff, :home_id, :away_id, :ft_home, :ft_away, :ft_result,
             :ht_home, :ht_away, :referee,
             :shots_home, :shots_away, :sot_home, :sot_away,
             :corners_home, :corners_away, :fouls_home, :fouls_away,
             :yellow_home, :yellow_away, :red_home, :red_away, :xg_home, :xg_away)
         ON CONFLICT (div, match_date, home_id, away_id) DO UPDATE SET
             season = excluded.season, kickoff_utc = excluded.kickoff_utc,
             ft_home = excluded.ft_home, ft_away = excluded.ft_away, ft_result = excluded.ft_result,
             ht_home = excluded.ht_home, ht_away = excluded.ht_away, referee = excluded.referee,
             shots_home = excluded.shots_home, shots_away = excluded.shots_away,
             shots_on_target_home = excluded.shots_on_target_home,
             shots_on_target_away = excluded.shots_on_target_away,
             corners_home = excluded.corners_home, corners_away = excluded.corners_away,
             fouls_home = excluded.fouls_home, fouls_away = excluded.fouls_away,
             yellow_home = excluded.yellow_home, yellow_away = excluded.yellow_away,
             red_home = excluded.red_home, red_away = excluded.red_away,
             xg_home = excluded.xg_home, xg_away = excluded.xg_away
         RETURNING id",
    )?;
    let stats = &record.stats;
    let id = statement.query_row(
        named_params! {
            ":div": div,
            ":season": season,
            ":date": record.date.format("%Y-%m-%d").to_string(),
            ":kickoff": record.kickoff_utc.map(|t| t.to_rfc3339_opts(SecondsFormat::Secs, true)),
            ":home_id": home_id,
            ":away_id": away_id,
            ":ft_home": record.ft_home,
            ":ft_away": record.ft_away,
            ":ft_result": result_code(record.ft_result),
            ":ht_home": record.ht_home,
            ":ht_away": record.ht_away,
            ":referee": record.referee,
            ":shots_home": stats.shots_home,
            ":shots_away": stats.shots_away,
            ":sot_home": stats.shots_on_target_home,
            ":sot_away": stats.shots_on_target_away,
            ":corners_home": stats.corners_home,
            ":corners_away": stats.corners_away,
            ":fouls_home": stats.fouls_home,
            ":fouls_away": stats.fouls_away,
            ":yellow_home": stats.yellow_home,
            ":yellow_away": stats.yellow_away,
            ":red_home": stats.red_home,
            ":red_away": stats.red_away,
            ":xg_home": record.xg_home,
            ":xg_away": record.xg_away,
        },
        |row| row.get(0),
    )?;
    Ok(id)
}

/// Insère ou met à jour les cotes d'un bookmaker pour un match et un moment.
pub fn upsert_odds(conn: &Connection, game_id: i64, odds: &BookOdds) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO game_odds (game_id, bookmaker, moment, home, draw, away)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (game_id, bookmaker, moment) DO UPDATE SET
             home = excluded.home, draw = excluded.draw, away = excluded.away",
    )?
    .execute(params![
        game_id,
        odds.bookmaker,
        odds.moment.as_str(),
        odds.odds.home.get(),
        odds.odds.draw.get(),
        odds.odds.away.get(),
    ])?;
    Ok(())
}

/// Insère ou met à jour les cotes plus/moins d'un bookmaker pour un match, un moment et une ligne.
pub fn upsert_totals(conn: &Connection, game_id: i64, totals: &TotalsOdds) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO game_totals_odds (game_id, bookmaker, moment, line, over, under)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (game_id, bookmaker, moment, line) DO UPDATE SET
             over = excluded.over, under = excluded.under",
    )?
    .execute(params![
        game_id,
        totals.bookmaker,
        totals.moment.as_str(),
        totals.line,
        totals.over.get(),
        totals.under.get(),
    ])?;
    Ok(())
}

/// Nombre de matchs en base.
pub fn count_games(conn: &Connection) -> Result<u32> {
    let count = conn.query_row("SELECT COUNT(*) FROM game", [], |row| row.get(0))?;
    Ok(count)
}

/// Identifiant d'un match par sa clé naturelle, s'il existe.
pub fn find_game(
    conn: &Connection,
    div: &str,
    date: &str,
    home_id: i64,
    away_id: i64,
) -> Result<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM game WHERE div = ?1 AND match_date = ?2 AND home_id = ?3 AND away_id = ?4",
            params![div, date, home_id, away_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id)
}
