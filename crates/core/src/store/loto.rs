//! Archive des tirages FDJ et journal du Grand Loto (migration 16) : téléchargements, tirages, séances, grilles
//! recommandées et jouées, alertes EuroMillions.
//!
//! Uniquement des `INSERT` : des déclencheurs interdisent toute modification ou suppression. Un tirage déjà connu n'est
//! pas réécrit : la première publication fait foi, et une différence dans une archive plus récente est seulement comptée.

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::journal::parse_timestamp;
use crate::error::{Error, Result};
use crate::loto::{Codes, Draw, Game, Grid, RankResult};

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn day(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn parse_day(text: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| Error::Data(format!("date « {text} » illisible en base")))
}

/// Un rang ou une position (`usize`) en entier SQLite : `rusqlite` ne convertit pas `usize`, dont la taille dépend de la
/// machine. Les valeurs ici sont minuscules : la conversion ne peut pas échouer en pratique.
fn int(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// `[2, 3, 30]` → `2-3-30`.
fn joined(numbers: &[u8]) -> String {
    numbers
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join("-")
}

fn split_numbers(text: &str) -> Result<Vec<u8>> {
    text.split('-')
        .map(|n| {
            n.parse()
                .map_err(|_| Error::Data(format!("numéros « {text} » illisibles en base")))
        })
        .collect()
}

/// Journalise un téléchargement d'archive et renvoie son identifiant.
pub fn record_fetch(
    conn: &Connection,
    at: DateTime<Utc>,
    document: &str,
    http_status: u16,
    draws: usize,
    error: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO loto_fetch (fetched_at, document, http_status, draws, error) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![timestamp(at), document, http_status, int(draws), error],
    )?;
    Ok(conn.last_insert_rowid())
}

/// `true` si l'archive `document` a déjà été lue sans erreur au moins une fois.
pub fn fetched_once(conn: &Connection, document: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM loto_fetch WHERE document = ?1 AND error IS NULL",
        [document],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Bilan de l'écriture des tirages d'une archive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Recorded {
    /// Tirages nouveaux.
    pub new: usize,
    /// Tirages déjà connus mais publiés autrement cette fois (non réécrits, à signaler).
    pub changed: usize,
}

/// Écrit les tirages nouveaux d'une archive, en une transaction. Un tirage déjà connu n'est pas réécrit ; s'il diffère
/// de la première publication, il est compté dans [`Recorded::changed`].
pub fn record_draws(conn: &mut Connection, fetch_id: i64, draws: &[Draw]) -> Result<Recorded> {
    let tx = conn.transaction()?;
    let mut recorded = Recorded::default();
    for draw in draws {
        if let Some(known) = draw_on(&tx, draw.game, draw.date)? {
            if known != *draw {
                recorded.changed += 1;
            }
            continue;
        }
        tx.execute(
            "INSERT INTO loto_draw (game, draw_date, balls, extra, codes, code_gain, fetch_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                draw.game.code(),
                day(draw.date),
                joined(&draw.balls),
                joined(&draw.extra),
                draw.codes.map(|c| c.count),
                draw.codes.map(|c| c.gain),
                fetch_id
            ],
        )?;
        for (i, rank) in draw.ranks.iter().enumerate() {
            tx.execute(
                "INSERT INTO loto_rank (game, draw_date, rank, winners, unit_gain) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    draw.game.code(),
                    day(draw.date),
                    int(i + 1),
                    // Les compteurs de gagnants tiennent dans un i64 (quelques millions au plus).
                    i64::try_from(rank.winners).unwrap_or(i64::MAX),
                    rank.unit_gain
                ],
            )?;
        }
        recorded.new += 1;
    }
    tx.commit()?;
    Ok(recorded)
}

/// Tirage de `game` à la date `date`, s'il est connu.
pub fn draw_on(conn: &Connection, game: Game, date: NaiveDate) -> Result<Option<Draw>> {
    let row = conn
        .query_row(
            "SELECT balls, extra, codes, code_gain FROM loto_draw WHERE game = ?1 AND draw_date = ?2",
            params![game.code(), day(date)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<u32>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((balls, extra, codes, code_gain)) = row else {
        return Ok(None);
    };
    let ranks = ranks_of(conn, game, &day(date))?;
    build_draw(game, date, &balls, &extra, codes.zip(code_gain), ranks).map(Some)
}

/// Tous les tirages connus de `game`, du plus ancien au plus récent.
pub fn draws(conn: &Connection, game: Game) -> Result<Vec<Draw>> {
    let mut statement = conn.prepare(
        "SELECT draw_date, balls, extra, codes, code_gain FROM loto_draw WHERE game = ?1 ORDER BY draw_date",
    )?;
    let rows = statement
        .query_map([game.code()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<u32>>(3)?,
                row.get::<_, Option<f64>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Les rangs de tous les tirages en une requête, plutôt qu'une requête par tirage (des milliers de tirages).
    let mut ranks_statement = conn.prepare(
        "SELECT draw_date, winners, unit_gain FROM loto_rank WHERE game = ?1 ORDER BY draw_date, rank",
    )?;
    let mut all_ranks: std::collections::HashMap<String, Vec<RankResult>> =
        std::collections::HashMap::new();
    let rank_rows = ranks_statement.query_map([game.code()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;
    for rank in rank_rows {
        let (date, winners, unit_gain) = rank?;
        all_ranks.entry(date).or_default().push(RankResult {
            winners: u64::try_from(winners).unwrap_or(0),
            unit_gain,
        });
    }
    rows.into_iter()
        .map(|(date, balls, extra, codes, gain)| {
            // `remove` sort la liste de la table au lieu de la copier : chaque tirage ne la prend qu'une fois.
            let ranks = all_ranks.remove(&date).unwrap_or_default();
            build_draw(
                game,
                parse_day(&date)?,
                &balls,
                &extra,
                codes.zip(gain),
                ranks,
            )
        })
        .collect()
}

fn ranks_of(conn: &Connection, game: Game, date: &str) -> Result<Vec<RankResult>> {
    let mut statement = conn.prepare(
        "SELECT winners, unit_gain FROM loto_rank WHERE game = ?1 AND draw_date = ?2 ORDER BY rank",
    )?;
    let ranks = statement
        .query_map(params![game.code(), date], |row| {
            Ok(RankResult {
                winners: u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                unit_gain: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ranks)
}

fn build_draw(
    game: Game,
    date: NaiveDate,
    balls: &str,
    extra: &str,
    codes: Option<(u32, f64)>,
    ranks: Vec<RankResult>,
) -> Result<Draw> {
    let balls: [u8; 5] = split_numbers(balls)?
        .try_into()
        .map_err(|_| Error::Data(format!("tirage du {date} : il faut cinq boules")))?;
    Ok(Draw {
        game,
        date,
        balls,
        extra: split_numbers(extra)?,
        ranks,
        codes: codes.map(|(count, gain)| Codes { count, gain }),
    })
}

/// Séance d'un Grand Loto à écrire.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSession<'a> {
    /// Instant de la séance.
    pub created_at: DateTime<Utc>,
    /// Date de l'édition.
    pub draw_date: NaiveDate,
    /// Version du modèle (`grandloto-1`).
    pub model_version: &'a str,
    /// Paramètres, en JSON.
    pub params: &'a serde_json::Value,
    /// Prévision, décision et raisons, en JSON.
    pub notes: &'a serde_json::Value,
    /// Grilles recommandées, dans l'ordre (meilleure espérance d'abord). Vide si la séance décide de ne pas jouer.
    pub recos: &'a [NewReco],
}

/// Grille recommandée à écrire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewReco {
    /// La grille.
    pub grid: Grid,
    /// Mise, en euros (le prix d'une grille).
    pub stake: f64,
    /// Euros rendus par grille, nombre de grilles central.
    pub ev: f64,
    /// Euros rendus par grille, nombre de grilles pessimiste.
    pub ev_pessimistic: f64,
}

/// Écrit une séance et ses grilles en une transaction ; renvoie l'identifiant de la séance. Une seconde séance pour la
/// même édition est refusée par la base (`UNIQUE`).
pub fn record_session(conn: &mut Connection, session: &NewSession<'_>) -> Result<i64> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO loto_session (created_at, draw_date, model_version, params, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            timestamp(session.created_at),
            day(session.draw_date),
            session.model_version,
            session.params.to_string(),
            session.notes.to_string()
        ],
    )?;
    let id = tx.last_insert_rowid();
    for (i, reco) in session.recos.iter().enumerate() {
        tx.execute(
            "INSERT INTO loto_reco (session_id, grid, position, stake, ev, ev_pessimistic) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, reco.grid.to_string(), int(i + 1), reco.stake, reco.ev, reco.ev_pessimistic],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

/// Séance écrite.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    /// Identifiant.
    pub id: i64,
    /// Instant de la séance.
    pub created_at: DateTime<Utc>,
    /// Date de l'édition.
    pub draw_date: NaiveDate,
    /// Version du modèle.
    pub model_version: String,
    /// Paramètres (JSON).
    pub params: String,
    /// Prévision et décision (JSON).
    pub notes: String,
}

/// Séance de l'édition du `draw_date`, si elle existe.
pub fn session_for(conn: &Connection, draw_date: NaiveDate) -> Result<Option<SessionRow>> {
    sessions_where(conn, "WHERE draw_date = ?1", &[&day(draw_date)]).map(|mut rows| rows.pop())
}

/// Toutes les séances, de la plus ancienne à la plus récente.
pub fn sessions(conn: &Connection) -> Result<Vec<SessionRow>> {
    sessions_where(conn, "", &[])
}

fn sessions_where(
    conn: &Connection,
    filter: &str,
    values: &[&dyn rusqlite::ToSql],
) -> Result<Vec<SessionRow>> {
    let sql = format!(
        "SELECT id, created_at, draw_date, model_version, params, notes FROM loto_session {filter} ORDER BY id"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement
        .query_map(values, |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(
            |(id, created_at, draw_date, model_version, params, notes)| {
                Ok(SessionRow {
                    id,
                    created_at: parse_timestamp(&created_at)?,
                    draw_date: parse_day(&draw_date)?,
                    model_version,
                    params,
                    notes,
                })
            },
        )
        .collect()
}

/// Grille recommandée écrite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecoRow {
    /// Identifiant (à donner à `edge loto played`).
    pub id: i64,
    /// Séance.
    pub session_id: i64,
    /// Date de l'édition.
    pub draw_date: NaiveDate,
    /// La grille.
    pub grid: Grid,
    /// Ordre dans la séance.
    pub position: u32,
    /// Mise, en euros.
    pub stake: f64,
    /// Euros rendus par grille, nombre de grilles central.
    pub ev: f64,
    /// Idem, nombre de grilles pessimiste.
    pub ev_pessimistic: f64,
}

/// Toutes les grilles recommandées, par séance puis par position.
pub fn recos(conn: &Connection) -> Result<Vec<RecoRow>> {
    let mut statement = conn.prepare(
        "SELECT r.id, r.session_id, s.draw_date, r.grid, r.position, r.stake, r.ev, r.ev_pessimistic
         FROM loto_reco r JOIN loto_session s ON s.id = r.session_id ORDER BY r.session_id, r.position",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(
            |(id, session_id, date, grid, position, stake, ev, ev_pessimistic)| {
                Ok(RecoRow {
                    id,
                    session_id,
                    draw_date: parse_day(&date)?,
                    grid: Grid::parse(&grid).ok_or_else(|| {
                        Error::Data(format!("grille « {grid} » illisible en base"))
                    })?,
                    position,
                    stake,
                    ev,
                    ev_pessimistic,
                })
            },
        )
        .collect()
}

/// Grille jouée à écrire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewPlay {
    /// Instant de la déclaration.
    pub played_at: DateTime<Utc>,
    /// Date de l'édition.
    pub draw_date: NaiveDate,
    /// La grille.
    pub grid: Grid,
    /// Mise, en euros.
    pub stake: f64,
    /// Recommandation correspondante ; `None` : grille hors modèle.
    pub reco_id: Option<i64>,
}

/// Écrit une grille jouée et renvoie son identifiant.
pub fn record_play(conn: &Connection, play: &NewPlay) -> Result<i64> {
    conn.execute(
        "INSERT INTO loto_play (played_at, draw_date, grid, stake, reco_id, off_model) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            timestamp(play.played_at),
            day(play.draw_date),
            play.grid.to_string(),
            play.stake,
            play.reco_id,
            i64::from(play.reco_id.is_none())
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Grille jouée écrite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayRow {
    /// Identifiant.
    pub id: i64,
    /// Date de l'édition.
    pub draw_date: NaiveDate,
    /// La grille.
    pub grid: Grid,
    /// Mise, en euros.
    pub stake: f64,
    /// Recommandation correspondante.
    pub reco_id: Option<i64>,
}

/// Toutes les grilles jouées, dans l'ordre de déclaration.
pub fn plays(conn: &Connection) -> Result<Vec<PlayRow>> {
    let mut statement =
        conn.prepare("SELECT id, draw_date, grid, stake, reco_id FROM loto_play ORDER BY id")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, date, grid, stake, reco_id)| {
            Ok(PlayRow {
                id,
                draw_date: parse_day(&date)?,
                grid: Grid::parse(&grid)
                    .ok_or_else(|| Error::Data(format!("grille « {grid} » illisible en base")))?,
                stake,
                reco_id,
            })
        })
        .collect()
}

/// `true` si l'alerte de niveau `level` a déjà été envoyée pour le tirage du `draw_date`.
pub fn alert_sent(conn: &Connection, draw_date: NaiveDate, level: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM loto_alert WHERE draw_date = ?1 AND level = ?2",
        params![day(draw_date), level],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Journalise une alerte envoyée.
pub fn record_alert(
    conn: &Connection,
    sent_at: DateTime<Utc>,
    draw_date: NaiveDate,
    level: &str,
    message: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO loto_alert (sent_at, draw_date, level, message) VALUES (?1, ?2, ?3, ?4)",
        params![timestamp(sent_at), day(draw_date), level, message],
    )?;
    Ok(())
}

/// Alertes envoyées : (instant, tirage déclencheur, niveau), de la plus ancienne à la plus récente.
pub fn alerts(conn: &Connection) -> Result<Vec<(DateTime<Utc>, NaiveDate, String)>> {
    let mut statement =
        conn.prepare("SELECT sent_at, draw_date, level FROM loto_alert ORDER BY id")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(at, date, level)| Ok((parse_timestamp(&at)?, parse_day(&date)?, level)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loto::fdj::{self, tests::GRAND_LOTO_CSV};
    use crate::store::migrate::migrate;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn noon() -> DateTime<Utc> {
        "2026-10-23T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn draws_round_trip_and_are_never_rewritten() {
        let mut conn = database();
        let draws = fdj::parse_csv(GRAND_LOTO_CSV, Game::GrandLoto).unwrap();
        let fetch = record_fetch(&conn, noon(), "doc", 200, 1, None).unwrap();

        let first = record_draws(&mut conn, fetch, &draws).unwrap();
        assert_eq!(first, Recorded { new: 1, changed: 0 });
        assert_eq!(super::draws(&conn, Game::GrandLoto).unwrap(), draws);

        let mut corrected = draws.clone();
        corrected[0].ranks[1].winners = 4;
        let second = record_draws(&mut conn, fetch, &corrected).unwrap();
        assert_eq!(second, Recorded { new: 0, changed: 1 });
        assert_eq!(
            super::draws(&conn, Game::GrandLoto).unwrap(),
            draws,
            "la première publication fait foi"
        );
        assert!(fetched_once(&conn, "doc").unwrap());
        assert!(!fetched_once(&conn, "autre").unwrap());
    }

    #[test]
    fn the_journal_is_immutable_and_one_session_per_edition() {
        let mut conn = database();
        let date = NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
        let grid = Grid::parse("1-2-3-4-5+6").unwrap();
        let empty = serde_json::json!({});
        let recos = [NewReco {
            grid,
            stake: 5.0,
            ev: 5.5,
            ev_pessimistic: 5.2,
        }];
        let session = NewSession {
            created_at: noon(),
            draw_date: date,
            model_version: "grandloto-1",
            params: &empty,
            notes: &empty,
            recos: &recos,
        };
        let id = record_session(&mut conn, &session).unwrap();
        assert!(
            record_session(&mut conn, &session).is_err(),
            "une seule séance par édition"
        );
        assert_eq!(session_for(&conn, date).unwrap().map(|s| s.id), Some(id));
        let stored = recos_of(&conn);
        assert_eq!(stored[0].grid, grid);

        let play = NewPlay {
            played_at: noon(),
            draw_date: date,
            grid,
            stake: 5.0,
            reco_id: Some(stored[0].id),
        };
        record_play(&conn, &play).unwrap();
        assert_eq!(plays(&conn).unwrap()[0].reco_id, Some(stored[0].id));
        assert!(conn.execute("UPDATE loto_reco SET ev = 9", []).is_err());
        assert!(conn.execute("DELETE FROM loto_play", []).is_err());

        record_alert(&conn, noon(), date, "fifth", "texte").unwrap();
        assert!(alert_sent(&conn, date, "fifth").unwrap());
        assert!(!alert_sent(&conn, date, "watch").unwrap());
        assert!(record_alert(&conn, noon(), date, "fifth", "texte").is_err());
    }

    fn recos_of(conn: &Connection) -> Vec<RecoRow> {
        recos(conn).unwrap()
    }
}
