//! Matchs et cotes de tennis (tables de la migration 8) et réglage gelé de l'Elo.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::journal::timestamp;
use crate::error::{Error, Result};
use crate::ingest::tennis::{ParsedMatch, Status, Surface, Tour};

/// Un match relu dans la base, avec les cotes `(vainqueur, perdant)` par bookmaker.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMatch {
    /// Identifiant.
    pub id: i64,
    /// Circuit.
    pub tour: Tour,
    /// Année du fichier.
    pub season: u16,
    /// Tournoi.
    pub tournament: String,
    /// Catégorie du tournoi, texte brut.
    pub series: Option<String>,
    /// Date du match.
    pub date: NaiveDate,
    /// Surface.
    pub surface: Surface,
    /// Tour du tournoi.
    pub round: String,
    /// Nombre maximal de sets.
    pub best_of: u8,
    /// Vainqueur.
    pub winner: String,
    /// Perdant.
    pub loser: String,
    /// Terminé, abandon ou forfait.
    pub status: Status,
    /// Cotes `(vainqueur, perdant)` par préfixe de bookmaker.
    pub odds: HashMap<String, (f64, f64)>,
}

/// Écrit un match (ou met à jour celui qui a la même clé naturelle) et ses cotes. Renvoie son identifiant.
pub fn upsert_match(conn: &Connection, m: &ParsedMatch) -> Result<i64> {
    let id: i64 = conn.query_row(
        "INSERT INTO tennis_match
             (tour, season, tournament, location, match_date, series, court, surface, round, best_of, winner, loser,
              winner_rank, loser_rank, winner_points, loser_points, winner_sets, loser_sets, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
         ON CONFLICT (tour, match_date, tournament, round, winner, loser) DO UPDATE SET
             season = excluded.season, location = excluded.location, series = excluded.series, court = excluded.court,
             surface = excluded.surface, best_of = excluded.best_of, winner_rank = excluded.winner_rank,
             loser_rank = excluded.loser_rank, winner_points = excluded.winner_points,
             loser_points = excluded.loser_points, winner_sets = excluded.winner_sets,
             loser_sets = excluded.loser_sets, status = excluded.status
         RETURNING id",
        params![
            m.tour.as_str(),
            m.season,
            m.tournament,
            m.location,
            m.date.format("%Y-%m-%d").to_string(),
            m.series,
            m.court,
            m.surface.as_str(),
            m.round,
            m.best_of,
            m.winner,
            m.loser,
            m.winner_rank,
            m.loser_rank,
            m.winner_points,
            m.loser_points,
            m.winner_sets,
            m.loser_sets,
            m.status.as_str(),
        ],
        |row| row.get(0),
    )?;
    for odds in &m.odds {
        conn.execute(
            "INSERT OR REPLACE INTO tennis_odds (match_id, bookmaker, winner_odds, loser_odds) VALUES (?1, ?2, ?3, ?4)",
            params![id, odds.bookmaker, odds.winner.get(), odds.loser.get()],
        )?;
    }
    Ok(id)
}

/// Nombre de matchs en base.
pub fn count_matches(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("SELECT COUNT(*) FROM tennis_match", [], |row| row.get(0))?)
}

/// Tous les matchs avec leurs cotes, dans l'ordre chronologique (date, circuit, tournoi, puis identifiant : un ordre
/// total et stable, donc des résultats reproductibles).
pub fn load_matches(conn: &Connection) -> Result<Vec<StoredMatch>> {
    let mut odds: HashMap<i64, HashMap<String, (f64, f64)>> = HashMap::new();
    let mut statement =
        conn.prepare("SELECT match_id, bookmaker, winner_odds, loser_odds FROM tennis_odds")?;
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, f64>(3)?,
        ))
    })? {
        let (id, bookmaker, winner, loser) = row?;
        odds.entry(id)
            .or_default()
            .insert(bookmaker, (winner, loser));
    }

    let mut statement = conn.prepare(
        "SELECT id, tour, season, tournament, series, match_date, surface, round, best_of, winner, loser, status
         FROM tennis_match ORDER BY match_date, tour, tournament, id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u16>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, u8>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
        ))
    })?;
    let mut matches = Vec::new();
    for row in rows {
        let (
            id,
            tour,
            season,
            tournament,
            series,
            date,
            surface,
            round,
            best_of,
            winner,
            loser,
            status,
        ) = row?;
        let bad =
            |what: &str, value: &str| Error::Data(format!("{what} inconnu dans la base : {value}"));
        matches.push(StoredMatch {
            id,
            tour: Tour::parse(&tour).ok_or_else(|| bad("circuit", &tour))?,
            season,
            tournament,
            series,
            date: NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(|e| {
                Error::Data(format!("date illisible dans la base « {date} » : {e}"))
            })?,
            surface: Surface::parse(&surface).ok_or_else(|| bad("surface", &surface))?,
            round,
            best_of,
            winner,
            loser,
            status: Status::parse(&status).ok_or_else(|| bad("statut", &status))?,
            odds: odds.remove(&id).unwrap_or_default(),
        });
    }
    Ok(matches)
}

/// Configuration de l'Elo gelée à l'issue d'une session de réglage.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenElo {
    /// Instant du gel.
    pub created_at: DateTime<Utc>,
    /// Numérateur de K.
    pub k_base: f64,
    /// Exposant de la décroissance de K.
    pub k_shape: f64,
    /// Poids de l'Elo de surface dans le mélange.
    pub surface_weight: f64,
    /// Log loss de la configuration retenue sur la validation.
    pub validation_log_loss: f64,
    /// Matchs de validation évalués.
    pub validation_matches: usize,
    /// Configurations essayées pendant la session.
    pub configs_tried: usize,
}

/// Enregistre une configuration gelée.
pub fn insert_tuning(conn: &Connection, frozen: &FrozenElo) -> Result<()> {
    conn.execute(
        "INSERT INTO tennis_tuning (created_at, k_base, k_shape, surface_weight, validation_log_loss, validation_matches, configs_tried)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            timestamp(frozen.created_at),
            frozen.k_base,
            frozen.k_shape,
            frozen.surface_weight,
            frozen.validation_log_loss,
            frozen.validation_matches as i64,
            frozen.configs_tried as i64,
        ],
    )?;
    Ok(())
}

/// La dernière configuration gelée, s'il y en a une.
pub fn latest_tuning(conn: &Connection) -> Result<Option<FrozenElo>> {
    let row = conn
        .query_row(
            "SELECT created_at, k_base, k_shape, surface_weight, validation_log_loss, validation_matches, configs_tried
             FROM tennis_tuning ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(created_at, k_base, k_shape, surface_weight, validation_log_loss, matches, tried)| {
            Ok(FrozenElo {
                created_at: super::journal::parse_timestamp(&created_at)?,
                k_base,
                k_shape,
                surface_weight,
                validation_log_loss,
                validation_matches: usize::try_from(matches).unwrap_or(0),
                configs_tried: usize::try_from(tried).unwrap_or(0),
            })
        },
    )
    .transpose()
}

/// Nombre de sessions de réglage déjà faites.
pub fn tuning_sessions(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("SELECT COUNT(*) FROM tennis_tuning", [], |row| row.get(0))?)
}
