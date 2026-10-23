//! Scores en direct des grilles Loto Foot (migration 17) : dernier score lu chez ESPN pour chaque rencontre, et dernier
//! appel à chaque page ESPN.
//!
//! Contrairement au reste de l'archive, ces lignes sont **remplacées** à chaque relevé : un score provisoire n'est qu'un
//! affichage, le règlement reste fondé sur les résultats officiels (`lotofoot_result`, immuables).

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::journal::parse_timestamp;
use crate::error::{Error, Result};
use crate::lotofoot::Game;
use crate::lotofoot::espn::Phase;

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Dernier score lu d'une rencontre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveScore {
    /// Code ESPN de la compétition (`fra.2`).
    pub league: String,
    /// Identifiant ESPN du match.
    pub event_id: String,
    /// Buts de l'équipe à domicile.
    pub home_goals: u16,
    /// Buts de l'équipe à l'extérieur.
    pub away_goals: u16,
    /// État du match.
    pub phase: Phase,
    /// État en clair (« 57' », « MT », « terminé »).
    pub label: String,
    /// Instant du relevé.
    pub fetched_at: DateTime<Utc>,
}

/// Écrit (ou remplace) le dernier score d'une rencontre, `position` comptée à partir de 1.
pub fn upsert_score(
    conn: &Connection,
    game: Game,
    number: u32,
    position: u32,
    score: &LiveScore,
) -> Result<()> {
    conn.execute(
        "INSERT INTO lotofoot_live (game, number, position, league, event_id, home_goals, away_goals, phase, label,
                                    fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (game, number, position) DO UPDATE SET
            league = excluded.league, event_id = excluded.event_id, home_goals = excluded.home_goals,
            away_goals = excluded.away_goals, phase = excluded.phase, label = excluded.label,
            fetched_at = excluded.fetched_at",
        params![
            game.code(),
            number,
            position,
            score.league,
            score.event_id,
            score.home_goals,
            score.away_goals,
            score.phase.code(),
            score.label,
            timestamp(score.fetched_at),
        ],
    )?;
    Ok(())
}

/// Derniers scores connus d'une grille, avec leur position (à partir de 1).
pub fn scores(conn: &Connection, game: Game, number: u32) -> Result<Vec<(u32, LiveScore)>> {
    let mut stmt = conn.prepare(
        "SELECT position, league, event_id, home_goals, away_goals, phase, label, fetched_at
         FROM lotofoot_live WHERE game = ?1 AND number = ?2 ORDER BY position",
    )?;
    let rows = stmt
        .query_map(params![game.code(), number], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u16>(3)?,
                row.get::<_, u16>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // `into_iter` consomme les lignes lues : chaque `String` passe dans le `LiveScore` sans copie.
    rows.into_iter()
        .map(
            |(position, league, event_id, home_goals, away_goals, phase, label, fetched_at)| {
                let phase = Phase::from_code(&phase).ok_or_else(|| {
                    Error::Data(format!("état de match inconnu en base : {phase}"))
                })?;
                Ok((
                    position,
                    LiveScore {
                        league,
                        event_id,
                        home_goals,
                        away_goals,
                        phase,
                        label,
                        fetched_at: parse_timestamp(&fetched_at)?,
                    },
                ))
            },
        )
        .collect()
}

/// Instant du dernier appel à la page ESPN d'une compétition et d'un jour (`20261024`), s'il y en a eu un.
pub fn last_fetch(conn: &Connection, league: &str, day: &str) -> Result<Option<DateTime<Utc>>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT fetched_at FROM lotofoot_live_fetch WHERE league = ?1 AND day = ?2",
            params![league, day],
            |row| row.get(0),
        )
        .optional()?;
    text.as_deref().map(parse_timestamp).transpose()
}

/// Note un appel à une page ESPN, avec ce qui a empêché d'apparier une rencontre (`None` : tout est apparié).
pub fn record_fetch(
    conn: &Connection,
    league: &str,
    day: &str,
    at: DateTime<Utc>,
    issue: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO lotofoot_live_fetch (league, day, fetched_at, issue) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (league, day) DO UPDATE SET fetched_at = excluded.fetched_at, issue = excluded.issue",
        params![league, day, timestamp(at), issue],
    )?;
    Ok(())
}

/// Problème noté au dernier appel d'une page ESPN, s'il y en a un.
pub fn fetch_issue(conn: &Connection, league: &str, day: &str) -> Result<Option<String>> {
    let issue: Option<Option<String>> = conn
        .query_row(
            "SELECT issue FROM lotofoot_live_fetch WHERE league = ?1 AND day = ?2",
            params![league, day],
            |row| row.get(0),
        )
        .optional()?;
    Ok(issue.flatten())
}
