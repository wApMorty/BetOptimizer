//! Archive du Loto Foot (migration 15) : faits relevés sur prono-match, séances de recommandation, tickets joués.
//!
//! Uniquement des `INSERT` : des déclencheurs interdisent toute modification ou suppression. Une grille, un résultat ou
//! un rapport déjà connu n'est pas réécrit (`INSERT OR IGNORE`) : la première observation fait foi.

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::journal::parse_timestamp;
use crate::domain::{Euros, Outcome};
use crate::error::{Error, Result};
use crate::lotofoot::pronomatch::{GridPage, GridResult};
use crate::lotofoot::{Game, PAYOUT_RATE, Ticket};

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Un rang ou une position (`usize`) en entier SQLite : `rusqlite` ne convertit pas `usize`, dont la taille dépend de la
/// machine. Les valeurs ici sont minuscules : la conversion ne peut pas échouer en pratique.
fn int(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Page lue : `lf7`, `lf8` ou `results`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Page d'une formule.
    Grid(Game),
    /// Page des derniers résultats.
    Results,
}

impl Page {
    fn code(self) -> &'static str {
        match self {
            Page::Grid(game) => game.code(),
            Page::Results => "results",
        }
    }
}

/// Journalise la lecture d'une page et renvoie son identifiant.
pub fn record_fetch(
    conn: &Connection,
    at: DateTime<Utc>,
    page: Page,
    http_status: u16,
    error: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO lotofoot_fetch (fetched_at, page, http_status, error) VALUES (?1, ?2, ?3, ?4)",
        params![timestamp(at), page.code(), http_status, error],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Instant d'une lecture de page.
pub fn fetch_time(conn: &Connection, fetch_id: i64) -> Result<DateTime<Utc>> {
    let text: String = conn.query_row(
        "SELECT fetched_at FROM lotofoot_fetch WHERE id = ?1",
        [fetch_id],
        |row| row.get(0),
    )?;
    parse_timestamp(&text)
}

/// Enregistre ce qu'a montré la page d'une formule : la grille en cours, sa répartition, les rapports passés.
/// Une seule transaction : tout ou rien. Renvoie le nombre de répartitions écrites (0 si la FDJ n'en a pas publié).
pub fn record_grid_page(
    conn: &mut Connection,
    fetch_id: i64,
    game: Game,
    page: &GridPage,
) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut splits = 0;
    if let Some(grid) = &page.open {
        tx.execute(
            "INSERT OR IGNORE INTO lotofoot_grid (game, number, closes_at, jackpot, fetch_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![game.code(), grid.number, timestamp(grid.closes_at), grid.jackpot.map(Euros::get), fetch_id],
        )?;
        for (i, (home, away)) in grid.matches.iter().enumerate() {
            tx.execute(
                "INSERT OR IGNORE INTO lotofoot_match (game, number, position, home_raw, away_raw) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![game.code(), grid.number, int(i + 1), home, away],
            )?;
        }
        if let Some(split) = &grid.split {
            for (i, q) in split.iter().enumerate() {
                tx.execute(
                    "INSERT INTO lotofoot_split (fetch_id, game, number, position, q_home, q_draw, q_away, split_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![fetch_id, game.code(), grid.number, int(i + 1), q[0], q[1], q[2], grid.split_at.map(timestamp)],
                )?;
                splits += 1;
            }
        }
    }
    for past in &page.history {
        for (i, rank) in past.ranks.iter().enumerate().take(2) {
            tx.execute(
                "INSERT OR IGNORE INTO lotofoot_rank (game, number, rank, winners, unit_gain, fetch_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![game.code(), past.number, int(i + 1), rank.winners, rank.unit_gain.map(Euros::get), fetch_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(splits)
}

/// Enregistre les résultats et rapports des grilles LF7 et LF8 de la page des résultats. Renvoie le nombre de grilles
/// dont les signes ont été écrits pour la première fois.
pub fn record_results(
    conn: &mut Connection,
    fetch_id: i64,
    results: &[GridResult],
) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut new = 0;
    for result in results {
        let Some(game) = result.game else { continue };
        if result.matches.len() != game.matches() {
            continue;
        }
        let mut written = 0;
        for (i, (_, _, outcome)) in result.matches.iter().enumerate() {
            written += tx.execute(
                "INSERT OR IGNORE INTO lotofoot_result (game, number, position, outcome, fetch_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![game.code(), result.number, int(i + 1), outcome.code(), fetch_id],
            )?;
        }
        new += usize::from(written > 0);
        for (i, rank) in result.ranks.iter().enumerate().take(2) {
            tx.execute(
                "INSERT OR IGNORE INTO lotofoot_rank (game, number, rank, winners, unit_gain, fetch_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![game.code(), result.number, int(i + 1), rank.winners, rank.unit_gain.map(Euros::get), fetch_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(new)
}

/// Une grille connue, avec ses rencontres.
#[derive(Debug, Clone, PartialEq)]
pub struct GridRow {
    /// Formule.
    pub game: Game,
    /// Numéro.
    pub number: u32,
    /// Fin de validation.
    pub closes_at: DateTime<Utc>,
    /// Gain minimum garanti du rang 1.
    pub jackpot: Option<Euros>,
    /// Rencontres (noms bruts du site).
    pub matches: Vec<(String, String)>,
}

/// Grilles qui ferment après `after`, dans l'ordre de clôture.
pub fn grids_closing_after(conn: &Connection, after: DateTime<Utc>) -> Result<Vec<GridRow>> {
    let mut stmt = conn.prepare(
        "SELECT game, number, closes_at, jackpot FROM lotofoot_grid WHERE closes_at > ?1 ORDER BY closes_at, game",
    )?;
    let heads = stmt
        .query_map([timestamp(after)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<f64>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    heads
        .into_iter()
        .map(|(game, number, closes_at, jackpot)| {
            let game = Game::from_code(&game)
                .ok_or_else(|| Error::Data(format!("formule inconnue en base : {game}")))?;
            Ok(GridRow {
                game,
                number,
                closes_at: parse_timestamp(&closes_at)?,
                jackpot: jackpot.and_then(Euros::new),
                matches: matches(conn, game, number)?,
            })
        })
        .collect()
}

/// Une grille par sa formule et son numéro.
pub fn grid(conn: &Connection, game: Game, number: u32) -> Result<Option<GridRow>> {
    let head = conn
        .query_row(
            "SELECT closes_at, jackpot FROM lotofoot_grid WHERE game = ?1 AND number = ?2",
            params![game.code(), number],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(1)?)),
        )
        .optional()?;
    let Some((closes_at, jackpot)) = head else {
        return Ok(None);
    };
    Ok(Some(GridRow {
        game,
        number,
        closes_at: parse_timestamp(&closes_at)?,
        jackpot: jackpot.and_then(Euros::new),
        matches: matches(conn, game, number)?,
    }))
}

fn matches(conn: &Connection, game: Game, number: u32) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT home_raw, away_raw FROM lotofoot_match WHERE game = ?1 AND number = ?2 ORDER BY position",
    )?;
    let rows = stmt
        .query_map(params![game.code(), number], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Dernière répartition complète relevée au plus tard à `at` : identifiant du relevé et pourcents par rencontre.
pub fn latest_split(
    conn: &Connection,
    game: Game,
    number: u32,
    at: DateTime<Utc>,
) -> Result<Option<(i64, Vec<[u32; 3]>)>> {
    let fetch_id: Option<i64> = conn
        .query_row(
            "SELECT s.fetch_id FROM lotofoot_split s JOIN lotofoot_fetch f ON f.id = s.fetch_id
             WHERE s.game = ?1 AND s.number = ?2 AND f.fetched_at <= ?3
             GROUP BY s.fetch_id HAVING COUNT(*) = ?4 ORDER BY f.fetched_at DESC LIMIT 1",
            params![game.code(), number, timestamp(at), int(game.matches())],
            |row| row.get(0),
        )
        .optional()?;
    let Some(fetch_id) = fetch_id else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT q_home, q_draw, q_away FROM lotofoot_split WHERE fetch_id = ?1 AND game = ?2 AND number = ?3 ORDER BY position",
    )?;
    let split = stmt
        .query_map(params![fetch_id, game.code(), number], |row| {
            Ok([row.get(0)?, row.get(1)?, row.get(2)?])
        })?
        .collect::<rusqlite::Result<Vec<[u32; 3]>>>()?;
    Ok(Some((fetch_id, split)))
}

/// Mises estimées d'une prochaine grille : médiane, sur les `window` dernières grilles de la formule de numéro inférieur,
/// de la masse du rang 2 divisée par sa part des mises (55 % de 70 %). `None` sans aucun rapport.
///
/// L'espérance d'un ticket dépend peu des mises (environ `0,7 · p(c)/q(c)` quand les concurrents sont nombreux) : une
/// estimation à 20 % près suffit.
pub fn stakes_estimate(
    conn: &Connection,
    game: Game,
    before: u32,
    window: usize,
) -> Result<Option<Euros>> {
    let share = game.rank_shares()[1] * PAYOUT_RATE;
    let mut stmt = conn.prepare(
        "SELECT winners * unit_gain FROM lotofoot_rank
         WHERE game = ?1 AND rank = 2 AND number < ?2 AND winners > 0 AND unit_gain IS NOT NULL
         ORDER BY number DESC LIMIT ?3",
    )?;
    let mut masses = stmt
        .query_map(params![game.code(), before, int(window)], |row| {
            row.get::<_, f64>(0)
        })?
        .collect::<rusqlite::Result<Vec<f64>>>()?;
    if masses.is_empty() {
        return Ok(None);
    }
    masses.sort_by(f64::total_cmp);
    let mid = masses.len() / 2;
    let median = if masses.len() % 2 == 1 {
        masses[mid]
    } else {
        (masses[mid - 1] + masses[mid]) / 2.0
    };
    Ok(Euros::new(median / share))
}

/// Signes sortis d'une grille, si les résultats de toutes ses rencontres sont connus.
pub fn results(conn: &Connection, game: Game, number: u32) -> Result<Option<Vec<Outcome>>> {
    let mut stmt = conn.prepare(
        "SELECT outcome FROM lotofoot_result WHERE game = ?1 AND number = ?2 ORDER BY position",
    )?;
    let codes = stmt
        .query_map(params![game.code(), number], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if codes.len() != game.matches() {
        return Ok(None);
    }
    Ok(codes.iter().map(|c| Outcome::from_code(c)).collect())
}

/// Rapport publié d'un rang : gagnants et gain unitaire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PublishedRank {
    /// Rang (1 : sans-faute).
    pub rank: u32,
    /// Grilles gagnantes.
    pub winners: u32,
    /// Gain unitaire, `None` sans gagnant.
    pub unit_gain: Option<Euros>,
}

/// Rapports publiés d'une grille, par rang croissant.
pub fn ranks(conn: &Connection, game: Game, number: u32) -> Result<Vec<PublishedRank>> {
    let mut stmt = conn.prepare(
        "SELECT rank, winners, unit_gain FROM lotofoot_rank WHERE game = ?1 AND number = ?2 ORDER BY rank",
    )?;
    let rows = stmt
        .query_map(params![game.code(), number], |row| {
            Ok(PublishedRank {
                rank: row.get(0)?,
                winners: row.get(1)?,
                unit_gain: row.get::<_, Option<f64>>(2)?.and_then(Euros::new),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Une séance à enregistrer.
#[derive(Debug, Clone)]
pub struct NewSession<'a> {
    /// Instant de la séance.
    pub created_at: DateTime<Utc>,
    /// Version du modèle.
    pub model_version: &'a str,
    /// Paramètres (JSON).
    pub params: &'a str,
    /// Grilles évaluées et écartées (JSON).
    pub notes: &'a str,
}

/// Un ticket recommandé à enregistrer.
#[derive(Debug, Clone)]
pub struct NewReco<'a> {
    /// Formule.
    pub game: Game,
    /// Numéro de la grille.
    pub number: u32,
    /// Ticket.
    pub ticket: &'a Ticket,
    /// Rang dans la séance (1 : meilleur).
    pub position: usize,
    /// Espérance, modèle de base.
    pub ev_base: f64,
    /// Espérance, scénario défavorable.
    pub ev_pessimistic: f64,
    /// Probabilité d'un rang payé.
    pub paid_probability: f64,
    /// Mises estimées de la grille.
    pub stakes_estimate: f64,
    /// Relevé de répartition utilisé.
    pub split_fetch_id: i64,
    /// Entrées du calcul (JSON).
    pub inputs: &'a str,
}

/// Enregistre une séance et ses tickets en une transaction ; renvoie l'identifiant de la séance.
pub fn record_session(
    conn: &mut Connection,
    session: &NewSession<'_>,
    recos: &[NewReco<'_>],
) -> Result<i64> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO lotofoot_session (created_at, model_version, params, notes) VALUES (?1, ?2, ?3, ?4)",
        params![timestamp(session.created_at), session.model_version, session.params, session.notes],
    )?;
    let session_id = tx.last_insert_rowid();
    for reco in recos {
        tx.execute(
            "INSERT INTO lotofoot_reco (session_id, game, number, ticket, position, ev_base, ev_pessimistic,
                 paid_probability, stakes_estimate, split_fetch_id, inputs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                session_id,
                reco.game.code(),
                reco.number,
                reco.ticket.to_string(),
                int(reco.position),
                reco.ev_base,
                reco.ev_pessimistic,
                reco.paid_probability,
                reco.stakes_estimate,
                reco.split_fetch_id,
                reco.inputs,
            ],
        )?;
    }
    tx.commit()?;
    Ok(session_id)
}

/// Dernière séance : identifiant, instant et notes (JSON des grilles évaluées et écartées).
pub fn last_session(conn: &Connection) -> Result<Option<(i64, DateTime<Utc>, String)>> {
    let row: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT id, created_at, notes FROM lotofoot_session ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(id, at, notes)| Ok((id, parse_timestamp(&at)?, notes)))
        .transpose()
}

/// Les tickets en vigueur de la semaine de la dernière séance : ceux des séances écrites au plus 6 jours avant elle.
/// C'est ce que montre la page `/lotofoot` et ce que déclare « J'ai joué » : un clic le matin puis le timer de midi
/// sur une nouvelle grille donnent deux séances, dont les tickets se jouent ensemble.
pub fn week_recos(conn: &Connection) -> Result<Vec<RecoRow>> {
    let Some((_, latest, _)) = last_session(conn)? else {
        return Ok(Vec::new());
    };
    let since = latest - chrono::Duration::days(6);
    Ok(current_recos(conn)?
        .into_iter()
        .filter(|r| r.created_at >= since)
        .collect())
}

/// Un ticket recommandé, relu du journal.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoRow {
    /// Identifiant.
    pub id: i64,
    /// Séance.
    pub session_id: i64,
    /// Instant de la séance.
    pub created_at: DateTime<Utc>,
    /// Formule.
    pub game: Game,
    /// Numéro de la grille.
    pub number: u32,
    /// Ticket.
    pub ticket: Ticket,
    /// Rang dans la séance.
    pub position: u32,
    /// Espérance, modèle de base.
    pub ev_base: f64,
    /// Espérance, scénario défavorable.
    pub ev_pessimistic: f64,
    /// Probabilité d'un rang payé.
    pub paid_probability: f64,
    /// Mises estimées de la grille.
    pub stakes_estimate: f64,
    /// Entrées du calcul (JSON : p, q, mises, pactole, appels de cotes, rencontres).
    pub inputs: String,
}

/// Les tickets recommandés **en vigueur**, du plus ancien au plus récent : pour chaque grille, ceux de la dernière séance
/// qui l'a recommandée, sauf si une séance plus récente l'a recalculée sans y retenir de ticket (grille listée dans le
/// champ `replaces` de ses paramètres : « remplacée par rien »). Une séance relancée par le bouton de l'application
/// remplace ainsi la précédente pour ses grilles ; les anciennes lignes restent au journal (immuable) mais ne comptent
/// plus nulle part (`docs/sources.md` §41).
pub fn current_recos(conn: &Connection) -> Result<Vec<RecoRow>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.session_id, s.created_at, r.game, r.number, r.ticket, r.position, r.ev_base, r.ev_pessimistic,
                r.paid_probability, r.stakes_estimate, r.inputs
         FROM lotofoot_reco r JOIN lotofoot_session s ON s.id = r.session_id
         WHERE r.session_id = (SELECT MAX(r2.session_id) FROM lotofoot_reco r2
                               WHERE r2.game = r.game AND r2.number = r.number)
           AND NOT EXISTS (SELECT 1 FROM lotofoot_session s2, json_each(s2.params, '$.replaces') j
                           WHERE s2.id > r.session_id
                             AND json_extract(j.value, '$[0]') = r.game
                             AND json_extract(j.value, '$[1]') = r.number)
         ORDER BY r.session_id, r.position",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, u32>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, f64>(8)?,
                row.get::<_, f64>(9)?,
                row.get::<_, f64>(10)?,
                row.get::<_, String>(11)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raw.into_iter()
        .map(
            |(
                id,
                session_id,
                created_at,
                game,
                number,
                ticket,
                position,
                ev_base,
                ev_pessimistic,
                paid,
                stakes,
                inputs,
            )| {
                Ok(RecoRow {
                    id,
                    session_id,
                    created_at: parse_timestamp(&created_at)?,
                    game: Game::from_code(&game)
                        .ok_or_else(|| Error::Data(format!("formule inconnue en base : {game}")))?,
                    number,
                    ticket: Ticket::parse(&ticket).ok_or_else(|| {
                        Error::Data(format!("ticket illisible en base : {ticket}"))
                    })?,
                    position,
                    ev_base,
                    ev_pessimistic,
                    paid_probability: paid,
                    stakes_estimate: stakes,
                    inputs,
                })
            },
        )
        .collect()
}

/// Un ticket joué à enregistrer.
#[derive(Debug, Clone)]
pub struct NewPlay<'a> {
    /// Instant de la déclaration.
    pub played_at: DateTime<Utc>,
    /// Formule.
    pub game: Game,
    /// Numéro de la grille.
    pub number: u32,
    /// Ticket.
    pub ticket: &'a Ticket,
    /// Mise (1 € pour une grille simple).
    pub stake: Euros,
    /// Recommandation correspondante ; `None` : ticket hors modèle.
    pub reco_id: Option<i64>,
}

/// Enregistre un ticket joué ; renvoie son identifiant. `off_model` vaut 1 exactement quand `reco_id` est absent.
pub fn record_play(conn: &Connection, play: &NewPlay<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO lotofoot_play (played_at, game, number, ticket, stake, reco_id, off_model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            timestamp(play.played_at),
            play.game.code(),
            play.number,
            play.ticket.to_string(),
            play.stake.get(),
            play.reco_id,
            i64::from(play.reco_id.is_none()),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Un ticket joué, relu du journal.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayRow {
    /// Identifiant.
    pub id: i64,
    /// Instant de la déclaration.
    pub played_at: DateTime<Utc>,
    /// Formule.
    pub game: Game,
    /// Numéro de la grille.
    pub number: u32,
    /// Ticket.
    pub ticket: Ticket,
    /// Mise.
    pub stake: f64,
    /// Recommandation correspondante.
    pub reco_id: Option<i64>,
}

/// Tous les tickets joués, du plus ancien au plus récent.
pub fn plays(conn: &Connection) -> Result<Vec<PlayRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, played_at, game, number, ticket, stake, reco_id FROM lotofoot_play ORDER BY id",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raw.into_iter()
        .map(|(id, played_at, game, number, ticket, stake, reco_id)| {
            Ok(PlayRow {
                id,
                played_at: parse_timestamp(&played_at)?,
                game: Game::from_code(&game)
                    .ok_or_else(|| Error::Data(format!("formule inconnue en base : {game}")))?,
                number,
                ticket: Ticket::parse(&ticket)
                    .ok_or_else(|| Error::Data(format!("ticket illisible en base : {ticket}")))?,
                stake,
                reco_id,
            })
        })
        .collect()
}

/// Répartition finale d'une grille (dernier relevé avant sa clôture), pour la vérification du modèle.
pub fn final_split(conn: &Connection, game: Game, number: u32) -> Result<Option<Vec<[u32; 3]>>> {
    let Some(grid) = grid(conn, game, number)? else {
        return Ok(None);
    };
    Ok(latest_split(conn, game, number, grid.closes_at)?.map(|(_, split)| split))
}

/// Grilles dont les résultats sont connus, par formule et numéro croissants.
pub fn settled_grids(conn: &Connection) -> Result<Vec<(Game, u32)>> {
    let mut stmt = conn.prepare(
        "SELECT game, number FROM lotofoot_result GROUP BY game, number ORDER BY game, number",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(raw
        .into_iter()
        .filter_map(|(game, number)| Game::from_code(&game).map(|g| (g, number)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lotofoot::pronomatch::{OpenGrid, PastRanks, RankLine};
    use crate::store::migrate::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn page(number: u32, split: Option<Vec<[u32; 3]>>) -> GridPage {
        GridPage {
            open: Some(OpenGrid {
                game: Game::Lf7,
                accepting: split.is_some(),
                number,
                closes_at: at("2026-10-25T16:10:00Z"),
                jackpot: Euros::new(100_000.0),
                matches: (1..=7)
                    .map(|i| (format!("A{i}"), format!("B{i}")))
                    .collect(),
                split,
                split_at: None,
            }),
            history: vec![
                PastRanks {
                    number: 131,
                    ranks: vec![rank(135, 578.0), rank(2393, 39.8)],
                },
                PastRanks {
                    number: 130,
                    ranks: vec![rank(588, 170.0), rank(7124, 16.6)],
                },
                PastRanks {
                    number: 129,
                    ranks: vec![rank(2539, 24.0), rank(17631, 4.2)],
                },
            ],
        }
    }

    fn rank(winners: u32, gain: f64) -> RankLine {
        RankLine {
            good: None,
            winners,
            unit_gain: Euros::new(gain),
        }
    }

    #[test]
    fn a_grid_page_is_archived_once_and_its_splits_accumulate() {
        let mut conn = db();
        let f1 = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        assert_eq!(
            record_grid_page(&mut conn, f1, Game::Lf7, &page(133, None)).unwrap(),
            0
        );
        let f2 = record_fetch(
            &conn,
            at("2026-10-24T10:00:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        assert_eq!(
            record_grid_page(
                &mut conn,
                f2,
                Game::Lf7,
                &page(133, Some(vec![[50, 30, 20]; 7]))
            )
            .unwrap(),
            7
        );
        let f3 = record_fetch(
            &conn,
            at("2026-10-24T11:00:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        record_grid_page(
            &mut conn,
            f3,
            Game::Lf7,
            &page(133, Some(vec![[52, 29, 19]; 7])),
        )
        .unwrap();

        let grids = grids_closing_after(&conn, at("2026-10-24T12:00:00Z")).unwrap();
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].matches.len(), 7);
        // Pas de fuite du futur : à 10 h 30, seul le relevé de 10 h est visible.
        let (id, split) = latest_split(&conn, Game::Lf7, 133, at("2026-10-24T10:30:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!((id, split[0]), (f2, [50, 30, 20]));
        let (id, _) = latest_split(&conn, Game::Lf7, 133, at("2026-10-24T12:00:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(id, f3);
        assert!(
            latest_split(&conn, Game::Lf7, 133, at("2026-10-24T09:30:00Z"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn stakes_are_estimated_from_the_second_rank_of_previous_grids() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        record_grid_page(&mut conn, f, Game::Lf7, &page(133, None)).unwrap();
        let stakes = stakes_estimate(&conn, Game::Lf7, 133, 8)
            .unwrap()
            .unwrap()
            .get();
        // Masses du rang 2 : 95 241,4 ; 118 258,4 ; 74 050,2 → médiane 95 241,4 / 0,385.
        assert!((stakes - 2393.0 * 39.8 / 0.385).abs() < 1e-6);
        assert!(stakes_estimate(&conn, Game::Lf8, 200, 8).unwrap().is_none());
    }

    #[test]
    fn the_journal_refuses_changes() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        record_grid_page(
            &mut conn,
            f,
            Game::Lf7,
            &page(133, Some(vec![[50, 30, 20]; 7])),
        )
        .unwrap();
        let ticket = Ticket::parse("1N21N21").unwrap();
        let session = NewSession {
            created_at: at("2026-10-24T10:00:00Z"),
            model_version: "lotofoot-1",
            params: "{}",
            notes: "{}",
        };
        let reco = NewReco {
            game: Game::Lf7,
            number: 133,
            ticket: &ticket,
            position: 1,
            ev_base: 1.8,
            ev_pessimistic: 1.4,
            paid_probability: 0.02,
            stakes_estimate: 200_000.0,
            split_fetch_id: f,
            inputs: "{}",
        };
        // `from_ref` : une tranche d'un élément qui emprunte `reco`, réutilisée (ticket changé) en fin de test.
        record_session(&mut conn, &session, std::slice::from_ref(&reco)).unwrap();
        let recos = current_recos(&conn).unwrap();
        assert_eq!(recos.len(), 1);
        assert_eq!(recos[0].ticket, ticket);
        assert!(
            conn.execute("UPDATE lotofoot_reco SET ev_base = 9", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM lotofoot_session", []).is_err());
        let play = NewPlay {
            played_at: at("2026-10-24T13:00:00Z"),
            game: Game::Lf7,
            number: 133,
            ticket: &ticket,
            stake: Euros::new(1.0).unwrap(),
            reco_id: Some(recos[0].id),
        };
        record_play(&conn, &play).unwrap();
        let off = NewPlay {
            reco_id: None,
            ..play
        };
        record_play(&conn, &off).unwrap();
        let plays = plays(&conn).unwrap();
        assert_eq!(plays.len(), 2);
        assert_eq!(plays[1].reco_id, None);
        assert!(conn.execute("DELETE FROM lotofoot_play", []).is_err());

        // Une seconde séance sur la même grille la remplace : seul son ticket est en vigueur, l'ancien reste au journal.
        let other = Ticket::parse("2N21N21").unwrap();
        let second = NewReco {
            ticket: &other,
            ..reco
        };
        record_session(&mut conn, &session, &[second]).unwrap();
        let current = current_recos(&conn).unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].ticket, other);
        let all: i64 = conn
            .query_row("SELECT COUNT(*) FROM lotofoot_reco", [], |row| row.get(0))
            .unwrap();
        assert_eq!(all, 2);
    }

    #[test]
    fn results_are_kept_for_lf7_and_lf8_only() {
        let mut conn = db();
        let f = record_fetch(&conn, at("2026-10-23T09:00:00Z"), Page::Results, 200, None).unwrap();
        let lf7 = GridResult {
            game: Some(Game::Lf7),
            number: 132,
            date: None,
            matches: (0..7)
                .map(|_| ("a".to_string(), "b".to_string(), Outcome::Draw))
                .collect(),
            ranks: vec![rank(135, 578.0), rank(2393, 39.8)],
        };
        let lf15 = GridResult {
            game: None,
            number: 88,
            ..lf7.clone()
        };
        assert_eq!(
            record_results(&mut conn, f, &[lf7.clone(), lf15]).unwrap(),
            1
        );
        // Un second relevé identique n'écrit rien de nouveau.
        assert_eq!(record_results(&mut conn, f, &[lf7]).unwrap(), 0);
        assert_eq!(
            results(&conn, Game::Lf7, 132).unwrap(),
            Some(vec![Outcome::Draw; 7])
        );
        let ranks = ranks(&conn, Game::Lf7, 132).unwrap();
        assert_eq!(ranks[1].winners, 2393);
        assert_eq!(settled_grids(&conn).unwrap(), vec![(Game::Lf7, 132)]);
    }
}
