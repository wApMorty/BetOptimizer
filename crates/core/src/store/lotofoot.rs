//! Archive du Loto Foot (migration 15) : faits relevés sur les sites des sources (FDJ, resultats-fdj.fr, et prono-match
//! jusqu'à son retrait), séances de recommandation, tickets joués.
//!
//! Uniquement des `INSERT` : des déclencheurs interdisent toute modification ou suppression. Une grille, un résultat ou
//! un rapport déjà connu n'est pas réécrit (`INSERT OR IGNORE`) : la première observation fait foi. Seule exception, la
//! composition d'une grille : si la page montre d'autres rencontres pour le même numéro, la nouvelle composition est
//! ajoutée (migration 20, `docs/sources.md` §52) et fait foi à partir de ce relevé ; rien n'est jamais apparié en silence.
//!
//! Une grille s'identifie par [`GridId`] : la FDJ renumérote chaque formule tous les 1er janvier (migration 22, §66).

use std::fmt;

use chrono::{DateTime, Datelike, Duration, SecondsFormat, Utc, Weekday};
use chrono_tz::Europe::Paris;
use rusqlite::{Connection, OptionalExtension, params};

use super::journal::parse_timestamp;
use crate::domain::Euros;
use crate::error::{Error, Result};
use crate::ingest::aliases::Aliases;
use crate::lotofoot::grid_page::{GridResult, MatchResult, OpenGrid, RankLine};
use crate::lotofoot::odds::first_difference;
use crate::lotofoot::{Game, GridId, PAYOUT_RATE, Ticket};

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Un rang ou une position (`usize`) en entier SQLite : `rusqlite` ne convertit pas `usize`, dont la taille dépend de la
/// machine. Les valeurs ici sont minuscules : la conversion ne peut pas échouer en pratique.
fn int(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Site d'où vient une page lue (colonne `lotofoot_fetch.source`, migration 24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// prono-match.fr, lu jusqu'au 2026-11-14 : figé depuis le 5 novembre (`docs/sources.md` §74). Son archive reste.
    PronoMatch,
    /// Site des points de vente de la FDJ : source principale depuis la phase 22.
    Fdj,
    /// resultats-fdj.fr : repli, lu seulement si la FDJ échoue.
    ResultatsFdj,
}

impl Source {
    /// Code stocké dans `lotofoot_fetch.source`.
    fn code(self) -> &'static str {
        match self {
            Source::PronoMatch => "pronomatch",
            Source::Fdj => "fdj",
            Source::ResultatsFdj => "resultats_fdj",
        }
    }

    /// Nom de la source dans les messages et dans l'application.
    pub fn label(self) -> &'static str {
        match self {
            Source::PronoMatch => "prono-match",
            Source::Fdj => "FDJ",
            Source::ResultatsFdj => "resultats-fdj.fr",
        }
    }
}

/// Page lue : `lf7`, `lf8`, `results`, `open` ou `detail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Page d'une formule : celle de prono-match, que le programme ne lit plus (son archive garde ces lectures).
    Grid(Game),
    /// Page des derniers résultats.
    Results,
    /// Page des grilles ouvertes, toutes formules : `/grilles/ouvertes/loto-foot` de la FDJ, accueil de resultats-fdj.fr.
    Open,
    /// Page de détail d'une grille (repli de resultats-fdj.fr).
    Detail,
}

impl Page {
    /// Code stocké dans `lotofoot_fetch.page` ; sert aussi de sujet aux alertes de source (`alert_log.subject`).
    pub fn code(self) -> &'static str {
        match self {
            Page::Grid(game) => game.code(),
            Page::Results => "results",
            Page::Open => "open",
            Page::Detail => "detail",
        }
    }
}

/// Nom de la page dans les messages : « grilles ouvertes », « résultats », « détail d'une grille ».
impl fmt::Display for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Page::Grid(game) => write!(f, "page du {game}"),
            Page::Results => write!(f, "résultats"),
            Page::Open => write!(f, "grilles ouvertes"),
            Page::Detail => write!(f, "détail d'une grille"),
        }
    }
}

/// Journalise la lecture d'une page de `source` et renvoie son identifiant.
pub fn record_fetch(
    conn: &Connection,
    at: DateTime<Utc>,
    source: Source,
    page: Page,
    http_status: u16,
    error: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO lotofoot_fetch (fetched_at, source, page, http_status, error) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![timestamp(at), source.code(), page.code(), http_status, error],
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

/// État du relevé d'une page : sa dernière lecture réussie, et l'erreur en cours s'il y en a une.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageHealth {
    /// Dernière lecture sans erreur ; `None` si la page n'a jamais été lue correctement.
    pub last_ok: Option<DateTime<Utc>>,
    /// Erreur de la lecture la plus récente ; `None` si elle a réussi, ou si la page n'a jamais été relevée.
    pub failing: Option<String>,
}

/// État du relevé de `page` chez `source` (page injoignable, illisible ou périmée : `lotofoot_fetch.error`). La même page
/// (`results`, par exemple) peut venir de deux sources : leurs états ne se mêlent pas.
pub fn page_health(conn: &Connection, source: Source, page: Page) -> Result<PageHealth> {
    let last_ok: Option<String> = conn.query_row(
        "SELECT MAX(fetched_at) FROM lotofoot_fetch WHERE source = ?1 AND page = ?2 AND error IS NULL",
        params![source.code(), page.code()],
        |row| row.get(0),
    )?;
    // La lecture la plus récente, réussie ou non : `error` y est NULL si elle a réussi. Deux `Option` emboîtées, donc :
    // celle d'`optional()` (aucune lecture du tout) et celle de la colonne (lecture réussie) ; `flatten` les réunit.
    let failing: Option<String> = conn
        .query_row(
            "SELECT error FROM lotofoot_fetch WHERE source = ?1 AND page = ?2 ORDER BY id DESC LIMIT 1",
            params![source.code(), page.code()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    Ok(PageHealth {
        last_ok: last_ok.map(|t| parse_timestamp(&t)).transpose()?,
        failing,
    })
}

/// Comment une page est servie à l'instant, d'après les dernières lectures de ses deux sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Serving {
    /// La FDJ a servi la page à la dernière lecture.
    Fdj,
    /// La FDJ a échoué, resultats-fdj.fr a servi la page à sa place (lu depuis la dernière lecture réussie de la FDJ).
    Fallback,
    /// Aucune source ne sert la page : la FDJ est en échec, et le repli n'a pas été lu depuis, ou a échoué lui aussi.
    Failing,
}

/// État du relevé d'une page, vu à travers la FDJ et resultats-fdj.fr (les sources réellement lues depuis la phase 22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcesHealth {
    /// La page.
    pub page: Page,
    /// État chez la FDJ.
    pub fdj: PageHealth,
    /// État chez resultats-fdj.fr.
    pub fallback: PageHealth,
}

impl SourcesHealth {
    /// Lit l'état de `page` chez les deux sources. Sont utiles [`Page::Open`] et [`Page::Results`] : les deux sont lues à
    /// chaque passage, par la FDJ d'abord, par le repli seulement si la FDJ échoue.
    pub fn read(conn: &Connection, page: Page) -> Result<SourcesHealth> {
        Ok(SourcesHealth {
            page,
            fdj: page_health(conn, Source::Fdj, page)?,
            fallback: page_health(conn, Source::ResultatsFdj, page)?,
        })
    }

    /// Dernière lecture réussie, de l'une ou l'autre source ; `None` si la page n'a jamais été lue correctement.
    pub fn last_ok(&self) -> Option<DateTime<Utc>> {
        // Une `Option` de date se compare : `None` est plus petit que tout `Some`, `max` garde donc la date existante.
        self.fdj.last_ok.max(self.fallback.last_ok)
    }

    /// Comment la page est servie ; `None` si elle n'a jamais été relevée (installation neuve).
    ///
    /// Le repli sert la page s'il a réussi **après** la dernière lecture réussie de la FDJ et que sa lecture la plus récente
    /// n'est pas en erreur. Il n'est lu que si la FDJ échoue : sa dernière lecture réussie, plus ancienne que celle de la
    /// FDJ, ne dit rien d'aujourd'hui.
    pub fn serving(&self) -> Option<Serving> {
        let never_read = self.fdj.last_ok.is_none()
            && self.fdj.failing.is_none()
            && self.fallback.last_ok.is_none()
            && self.fallback.failing.is_none();
        if never_read {
            return None;
        }
        if self.fdj.failing.is_none() {
            return Some(Serving::Fdj);
        }
        let fallback_serves =
            self.fallback.failing.is_none() && self.fallback.last_ok > self.fdj.last_ok;
        Some(if fallback_serves {
            Serving::Fallback
        } else {
            Serving::Failing
        })
    }
}

/// Nombre de grilles connues (LF7 et LF8).
pub fn count_grids(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM lotofoot_grid", [], |row| row.get(0))?)
}

/// Ce qu'a écrit le relevé d'une grille ouverte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GridRecord {
    /// Répartitions de rencontres écrites (0 si la FDJ n'en a pas publié).
    pub splits: usize,
    /// Les rencontres de la grille en cours diffèrent de la composition en vigueur : la nouvelle composition a été
    /// enregistrée et fait foi à partir de ce relevé.
    pub recomposed: bool,
    /// La grille en cours n'a pas été écrite : son nombre de rencontres n'est pas celui de la formule ([`wrong_size`]), ou
    /// sa clôture ou son numéro contredit ce qui est connu ([`refusal`]).
    pub refused: Option<String>,
}

/// Écart maximal entre deux clôtures relevées pour une même grille. Au-delà, ce n'est plus un report : la grille a changé.
const MAX_CLOSE_SHIFT_DAYS: i64 = 7;

/// Raison de refuser la grille en cours d'une page, `None` si elle est cohérente avec la base :
/// - grille connue, mais clôture éloignée de plus de [`MAX_CLOSE_SHIFT_DAYS`] jours : ce serait une autre grille ;
/// - grille nouvelle, mais de numéro inférieur à une grille déjà close plus tôt la même année : dans une année, les numéros
///   croissent avec la clôture (1 109 grilles, `docs/sources.md` §66), l'année attribuée serait donc fausse.
///
/// Un refus ne se transforme jamais en recomposition : l'année d'une grille ne se devine pas.
fn refusal(conn: &Connection, id: GridId, closes_at: DateTime<Utc>) -> Result<Option<String>> {
    let known: Option<String> = conn
        .query_row(
            "SELECT closes_at FROM lotofoot_grid WHERE game = ?1 AND year = ?2 AND number = ?3",
            params![id.game.code(), id.year, id.number],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(known) = known {
        let known = parse_timestamp(&known)?;
        if (closes_at - known).abs() > Duration::days(MAX_CLOSE_SHIFT_DAYS) {
            return Ok(Some(format!(
                "{id} : clôture du {} au lieu du {} déjà connu",
                timestamp(closes_at),
                timestamp(known)
            )));
        }
        return Ok(None);
    }
    // `MIN` sur aucune ligne donne NULL, lu en `None`.
    let later: Option<u32> = conn.query_row(
        "SELECT MIN(number) FROM lotofoot_grid
         WHERE game = ?1 AND year = ?2 AND number > ?3 AND closes_at < ?4",
        params![id.game.code(), id.year, id.number, timestamp(closes_at)],
        |row| row.get(0),
    )?;
    Ok(later.map(|number| {
        format!(
            "{id} : numéro inférieur au n°{number} de {}, déjà clos avant elle",
            id.year
        )
    }))
}

/// Raison de refuser une grille dont le nombre de rencontres n'est pas celui de sa formule, `None` sinon.
///
/// Le règlement permet un LF8 à 7 rencontres (`docs/sources.md` §74), mais le modèle et l'optimiseur supposent N rencontres
/// par formule : une telle grille n'est pas prise en charge, et le dit au lieu d'être écrite de travers. Comparée au nombre
/// de la formule, jamais au nombre lu : une grille de 7 rencontres ne devient pas un LF7 sous prétexte qu'elle en a 7.
fn wrong_size(id: GridId, grid: &OpenGrid) -> Option<String> {
    let expected = id.game.matches();
    (grid.matches.len() != expected).then(|| {
        format!(
            "{id} : {} rencontres lues au lieu de {expected}, grille non prise en charge, non écrite",
            grid.matches.len()
        )
    })
}

/// Enregistre une grille ouverte telle que la source l'affiche : sa ligne, sa composition si elle a changé, sa répartition.
/// Une seule transaction : tout ou rien.
///
/// Les rencontres sont comparées à la composition en vigueur, dans l'ordre : la répartition est publiée par position, et une
/// rencontre retirée décale toutes les suivantes (LF7 n°133, `docs/sources.md` §52). La comparaison passe par la table
/// d'alias ([`first_difference`]) : un même club écrit autrement par une autre source (`Atl. Madrid` pour
/// `Atlético Madrid`) n'est pas une nouvelle composition, qui écarterait les répartitions et les tickets déjà archivés.
pub fn record_open_grid(
    conn: &mut Connection,
    aliases: &Aliases,
    fetch_id: i64,
    grid: &OpenGrid,
) -> Result<GridRecord> {
    let tx = conn.transaction()?;
    let mut record = GridRecord::default();
    let id = GridId::closing(grid.game, grid.number, grid.closes_at);
    record.refused = match wrong_size(id, grid) {
        Some(reason) => Some(reason),
        None => refusal(&tx, id, grid.closes_at)?,
    };
    if record.refused.is_none() {
        write_open_grid(&tx, aliases, fetch_id, id, grid, &mut record)?;
    }
    tx.commit()?;
    Ok(record)
}

/// Écrit la grille en cours : sa ligne si elle est nouvelle, sa composition si elle a changé, sa répartition.
fn write_open_grid(
    conn: &Connection,
    aliases: &Aliases,
    fetch_id: i64,
    id: GridId,
    grid: &OpenGrid,
    record: &mut GridRecord,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO lotofoot_grid (game, year, number, closes_at, jackpot, fetch_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id.game.code(),
            id.year,
            id.number,
            timestamp(grid.closes_at),
            grid.jackpot.map(Euros::get),
            fetch_id
        ],
    )?;
    let current = matches(conn, id)?;
    let first = current.is_empty();
    record.recomposed = !first && first_difference(aliases, &current, &grid.matches).is_some();
    for (i, (home, away)) in grid.matches.iter().enumerate() {
        if first {
            conn.execute(
                "INSERT INTO lotofoot_match (game, year, number, position, home_raw, away_raw)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id.game.code(), id.year, id.number, int(i + 1), home, away],
            )?;
        } else if record.recomposed {
            conn.execute(
                "INSERT INTO lotofoot_match_revision (game, year, number, fetch_id, position, home_raw, away_raw)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id.game.code(), id.year, id.number, fetch_id, int(i + 1), home, away],
            )?;
        }
    }
    if let Some(split) = &grid.split {
        for (i, q) in split.iter().enumerate() {
            conn.execute(
                "INSERT INTO lotofoot_split (fetch_id, game, year, number, position, q_home, q_draw, q_away, split_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    fetch_id,
                    id.game.code(),
                    id.year,
                    id.number,
                    int(i + 1),
                    q[0],
                    q[1],
                    q[2],
                    grid.split_at.map(timestamp)
                ],
            )?;
            record.splits += 1;
        }
    }
    Ok(())
}

/// Écrit le rapport d'un rang, s'il n'est pas déjà connu.
fn write_rank(
    conn: &Connection,
    id: GridId,
    rank: i64,
    line: &RankLine,
    fetch_id: i64,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO lotofoot_rank (game, year, number, rank, winners, unit_gain, fetch_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id.game.code(),
            id.year,
            id.number,
            rank,
            line.winners,
            line.unit_gain.map(Euros::get),
            fetch_id
        ],
    )?;
    Ok(())
}

/// Ce qu'a écrit le relevé de la page des résultats.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultsRecord {
    /// Grilles dont les signes ont été écrits pour la première fois.
    pub new: usize,
    /// Grilles dont les résultats n'ont pas été écrits, avec la raison : rencontres différentes de la composition en
    /// vigueur, ou composition changée après des tickets déclarés joués.
    pub refused: Vec<String>,
}

/// Enregistre les résultats et rapports des grilles LF7 et LF8 de la page des résultats.
///
/// L'année de la grille vient de la date affichée, égale au jour de clôture ; sans date, la grille n'est pas écrite. Pour
/// une grille connue, les rencontres de la page doivent être celles de la composition en vigueur : les signes sont écrits
/// par position. Les équipes se comparent par nom canonique quand la table d'alias `aliases` connaît les deux noms (la
/// FDJ écrit `Atl. Madrid`, l'archive de prono-match `Atlético Madrid`), sinon par nom brut
/// ([`crate::lotofoot::odds::first_difference`]). Une grille dont la composition a changé après des tickets déclarés joués
/// n'est pas réglée : ces tickets portent sur des rencontres qui ne sont plus celles de la grille, ils se règlent à la main.
///
/// Toute grille non écrite est signalée dans [`ResultsRecord::refused`]. Une rencontre annulée (`docs/sources.md` §74)
/// s'écrit `C` dans la colonne `outcome` (migration 24) : la grille n'est pas refusée pour autant, son règlement la compte
/// bonne pour tous les signes ([`MatchResult::is_good_for`]).
pub fn record_results(
    conn: &mut Connection,
    aliases: &Aliases,
    fetch_id: i64,
    results: &[GridResult],
) -> Result<ResultsRecord> {
    let tx = conn.transaction()?;
    let mut record = ResultsRecord::default();
    for result in results {
        let game = result.game;
        if result.matches.len() != game.matches() {
            record.refused.push(format!(
                "{game} n°{} : {} rencontres lues au lieu de {}, résultats non écrits",
                result.number,
                result.matches.len(),
                game.matches()
            ));
            continue;
        }
        let Some(date) = result.date else {
            record.refused.push(format!(
                "{game} n°{} : date illisible, année inconnue, résultats non écrits",
                result.number
            ));
            continue;
        };
        let id = GridId {
            game,
            year: date.year(),
            number: result.number,
        };
        let current = matches(&tx, id)?;
        let shown: Vec<(String, String)> = result
            .matches
            .iter()
            .map(|(home, away, _)| (home.clone(), away.clone()))
            .collect();
        if !current.is_empty()
            && let Some(at) = first_difference(aliases, &current, &shown)
        {
            // Le message nomme la rencontre : l'écart vient presque toujours d'un nom à ajouter dans `config/aliases.toml`.
            let saved = current
                .get(at)
                .map_or(String::new(), |(h, a)| format!("« {h} – {a} »"));
            let read = shown
                .get(at)
                .map_or(String::new(), |(h, a)| format!("« {h} – {a} »"));
            record.refused.push(format!(
                "{id} : les rencontres des résultats diffèrent de la composition enregistrée (rencontre {} : {read} lu, {saved} \
                 en base), résultats non écrits",
                at + 1
            ));
            continue;
        }
        if recomposition(&tx, id)?.is_some_and(|r| r.played_before) {
            record.refused.push(format!(
                "{id} : composition changée après des tickets déclarés joués, résultats non écrits (règlement à la main)"
            ));
            continue;
        }
        let mut written = 0;
        for (i, (_, _, shown)) in result.matches.iter().enumerate() {
            written += tx.execute(
                "INSERT OR IGNORE INTO lotofoot_result (game, year, number, position, outcome, fetch_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id.game.code(), id.year, id.number, int(i + 1), shown.code(), fetch_id],
            )?;
        }
        record.new += usize::from(written > 0);
        for (i, rank) in result.ranks.iter().enumerate().take(2) {
            write_rank(&tx, id, int(i + 1), rank, fetch_id)?;
        }
    }
    tx.commit()?;
    Ok(record)
}

/// Une grille connue, avec ses rencontres.
#[derive(Debug, Clone, PartialEq)]
pub struct GridRow {
    /// Identité de la grille.
    pub id: GridId,
    /// Fin de validation.
    pub closes_at: DateTime<Utc>,
    /// Gain minimum garanti du rang 1.
    pub jackpot: Option<Euros>,
    /// Rencontres (noms bruts du site).
    pub matches: Vec<(String, String)>,
}

/// `true` si l'alerte « nouvelle grille » de cette grille a déjà été envoyée.
pub fn alerted(conn: &Connection, id: GridId) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM lotofoot_alert WHERE game = ?1 AND year = ?2 AND number = ?3",
            params![id.game.code(), id.year, id.number],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// Note l'alerte d'une grille comme envoyée (une seule fois : une seconde écriture est ignorée).
pub fn record_alert(conn: &Connection, id: GridId, sent_at: DateTime<Utc>) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO lotofoot_alert (game, year, number, sent_at) VALUES (?1, ?2, ?3, ?4)",
        params![id.game.code(), id.year, id.number, timestamp(sent_at)],
    )?;
    Ok(())
}

/// Enregistre un budget mensuel saisi (migration 19, historique immuable).
pub fn record_budget(conn: &Connection, set_at: DateTime<Utc>, monthly_euros: u32) -> Result<()> {
    conn.execute(
        "INSERT INTO lotofoot_budget (set_at, monthly_euros) VALUES (?1, ?2)",
        params![timestamp(set_at), monthly_euros],
    )?;
    Ok(())
}

/// Budget mensuel en vigueur à `at` : la dernière saisie antérieure, `None` si rien n'a encore été saisi.
pub fn budget_at(conn: &Connection, at: DateTime<Utc>) -> Result<Option<u32>> {
    Ok(conn
        .query_row(
            "SELECT monthly_euros FROM lotofoot_budget WHERE set_at <= ?1 ORDER BY set_at DESC, id DESC LIMIT 1",
            [timestamp(at)],
            |row| row.get(0),
        )
        .optional()?)
}

/// Première apparition d'une grille dans un relevé (le relevé qui l'a inscrite) : sert à son âge. Si sa composition a
/// changé, l'âge part du relevé qui a vu la composition en vigueur : les répartitions antérieures ne comptent pas (§52).
pub fn first_seen(conn: &Connection, id: GridId) -> Result<Option<DateTime<Utc>>> {
    if let Some(fetch_id) = revision(conn, id)? {
        return fetch_time(conn, fetch_id).map(Some);
    }
    let seen: Option<String> = conn
        .query_row(
            "SELECT f.fetched_at FROM lotofoot_grid g JOIN lotofoot_fetch f ON f.id = g.fetch_id
             WHERE g.game = ?1 AND g.year = ?2 AND g.number = ?3",
            params![id.game.code(), id.year, id.number],
            |row| row.get(0),
        )
        .optional()?;
    seen.map(|s| parse_timestamp(&s)).transpose()
}

/// Grilles qui ferment après `after`, dans l'ordre de clôture.
pub fn grids_closing_after(conn: &Connection, after: DateTime<Utc>) -> Result<Vec<GridRow>> {
    let mut stmt = conn.prepare(
        "SELECT game, year, number, closes_at, jackpot FROM lotofoot_grid WHERE closes_at > ?1 ORDER BY closes_at, game",
    )?;
    let heads = stmt
        .query_map([timestamp(after)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<f64>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    heads
        .into_iter()
        .map(|(game, year, number, closes_at, jackpot)| {
            let id = GridId {
                game: game_of(&game)?,
                year,
                number,
            };
            Ok(GridRow {
                id,
                closes_at: parse_timestamp(&closes_at)?,
                jackpot: jackpot.and_then(Euros::new),
                matches: matches(conn, id)?,
            })
        })
        .collect()
}

/// Formule d'un code lu en base.
fn game_of(code: &str) -> Result<Game> {
    Game::from_code(code).ok_or_else(|| Error::Data(format!("formule inconnue en base : {code}")))
}

/// Une grille par son identité.
pub fn grid(conn: &Connection, id: GridId) -> Result<Option<GridRow>> {
    let head = conn
        .query_row(
            "SELECT closes_at, jackpot FROM lotofoot_grid WHERE game = ?1 AND year = ?2 AND number = ?3",
            params![id.game.code(), id.year, id.number],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(1)?)),
        )
        .optional()?;
    let Some((closes_at, jackpot)) = head else {
        return Ok(None);
    };
    Ok(Some(GridRow {
        id,
        closes_at: parse_timestamp(&closes_at)?,
        jackpot: jackpot.and_then(Euros::new),
        matches: matches(conn, id)?,
    }))
}

/// Composition en vigueur d'une grille, rencontres dans l'ordre : celle de sa dernière révision, ou à défaut la première
/// composition (`lotofoot_match`). Vide si la grille est inconnue.
fn matches(conn: &Connection, id: GridId) -> Result<Vec<(String, String)>> {
    let rows = match revision(conn, id)? {
        Some(fetch_id) => conn
            .prepare(
                "SELECT home_raw, away_raw FROM lotofoot_match_revision
                 WHERE game = ?1 AND year = ?2 AND number = ?3 AND fetch_id = ?4 ORDER BY position",
            )?
            .query_map(
                params![id.game.code(), id.year, id.number, fetch_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?,
        None => conn
            .prepare(
                "SELECT home_raw, away_raw FROM lotofoot_match
                 WHERE game = ?1 AND year = ?2 AND number = ?3 ORDER BY position",
            )?
            .query_map(params![id.game.code(), id.year, id.number], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    };
    Ok(rows)
}

/// Relevé de la dernière révision de composition d'une grille, `None` si elle n'a jamais changé.
fn revision(conn: &Connection, id: GridId) -> Result<Option<i64>> {
    // `MAX` sur aucune ligne donne NULL, lu en `None`.
    let fetch_id = conn.query_row(
        "SELECT MAX(fetch_id) FROM lotofoot_match_revision WHERE game = ?1 AND year = ?2 AND number = ?3",
        params![id.game.code(), id.year, id.number],
        |row| row.get(0),
    )?;
    Ok(fetch_id)
}

/// Changement de composition d'une grille ([`recomposition`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recomposition {
    /// Relevé qui a vu la composition en vigueur.
    pub fetch_id: i64,
    /// Instant de ce relevé.
    pub at: DateTime<Utc>,
    /// Des tickets de la grille ont été déclarés joués avant ce relevé : ils portent sur l'ancienne composition. La
    /// grille est alors bloquée (ni recalcul, ni règlement automatique).
    pub played_before: bool,
}

/// Dernier changement de composition d'une grille, `None` si elle n'en a jamais eu.
pub fn recomposition(conn: &Connection, id: GridId) -> Result<Option<Recomposition>> {
    let Some(fetch_id) = revision(conn, id)? else {
        return Ok(None);
    };
    let at = fetch_time(conn, fetch_id)?;
    let played: i64 = conn.query_row(
        "SELECT COUNT(*) FROM lotofoot_play WHERE game = ?1 AND year = ?2 AND number = ?3 AND played_at < ?4",
        params![id.game.code(), id.year, id.number, timestamp(at)],
        |row| row.get(0),
    )?;
    Ok(Some(Recomposition {
        fetch_id,
        at,
        played_before: played > 0,
    }))
}

/// Dernière répartition complète relevée au plus tard à `at` : identifiant du relevé et pourcents par rencontre. Une
/// répartition relevée avant la composition en vigueur ne compte pas : ses positions peuvent désigner d'autres
/// rencontres (§52). Les identifiants de relevé croissent avec le temps (clé `INTEGER PRIMARY KEY`, rien n'est supprimé).
pub fn latest_split(
    conn: &Connection,
    id: GridId,
    at: DateTime<Utc>,
) -> Result<Option<(i64, Vec<[u32; 3]>)>> {
    let since = revision(conn, id)?.unwrap_or(0);
    let fetch_id: Option<i64> = conn
        .query_row(
            "SELECT s.fetch_id FROM lotofoot_split s JOIN lotofoot_fetch f ON f.id = s.fetch_id
             WHERE s.game = ?1 AND s.year = ?2 AND s.number = ?3 AND f.fetched_at <= ?4 AND s.fetch_id >= ?6
             GROUP BY s.fetch_id HAVING COUNT(*) = ?5 ORDER BY f.fetched_at DESC LIMIT 1",
            params![
                id.game.code(),
                id.year,
                id.number,
                timestamp(at),
                int(id.game.matches()),
                since
            ],
            |row| row.get(0),
        )
        .optional()?;
    let Some(fetch_id) = fetch_id else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT q_home, q_draw, q_away FROM lotofoot_split
         WHERE fetch_id = ?1 AND game = ?2 AND year = ?3 AND number = ?4 ORDER BY position",
    )?;
    let split = stmt
        .query_map(
            params![fetch_id, id.game.code(), id.year, id.number],
            |row| Ok([row.get(0)?, row.get(1)?, row.get(2)?]),
        )?
        .collect::<rusqlite::Result<Vec<[u32; 3]>>>()?;
    Ok(Some((fetch_id, split)))
}

/// Valeurs minimales d'une case de [`stakes_estimate`] ; en dessous, on passe à la case plus large.
const MIN_STAKES_CELL: usize = 3;

/// Rapport d'une grille passée : masse du rang 2 et, si la grille elle-même est connue, sa case (jour de clôture, présence
/// d'un pactole). Les rapports sans ligne `lotofoot_grid` n'ont pas de case : ils ne servent qu'au dernier repli.
struct PastMass {
    mass: f64,
    cell: Option<(Weekday, bool)>,
}

/// Jour de la semaine d'une clôture, heure de Paris : c'est le jour où le public joue.
fn paris_day(at: DateTime<Utc>) -> Weekday {
    at.with_timezone(&Paris).weekday()
}

/// Rapports antérieurs à `grid`, de sa formule, du plus récent au plus ancien, à cheval sur les années. Sans clôture connue,
/// seuls l'année et le numéro permettent de situer un rapport : ceux-là viennent après les autres, de numéro inférieur.
fn past_masses(conn: &Connection, grid: &GridRow) -> Result<Vec<PastMass>> {
    let mut stmt = conn.prepare(
        "SELECT r.winners * r.unit_gain, g.closes_at, g.jackpot IS NOT NULL
         FROM lotofoot_rank r
         LEFT JOIN lotofoot_grid g ON g.game = r.game AND g.year = r.year AND g.number = r.number
         WHERE r.game = ?1 AND r.rank = 2 AND r.winners > 0 AND r.unit_gain IS NOT NULL
           AND (g.closes_at < ?2 OR (g.closes_at IS NULL AND (r.year, r.number) < (?3, ?4)))
         ORDER BY g.closes_at IS NULL, g.closes_at DESC, r.year DESC, r.number DESC",
    )?;
    let rows = stmt
        .query_map(
            params![
                grid.id.game.code(),
                timestamp(grid.closes_at),
                grid.id.year,
                grid.id.number
            ],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(mass, closes_at, jackpot)| {
            // `transpose` change `Option<Result<_>>` en `Result<Option<_>>` : le `?` remonte une date illisible, et une
            // clôture absente reste un simple `None`.
            let cell = closes_at
                .map(|text| parse_timestamp(&text).map(|at| (paris_day(at), jackpot)))
                .transpose()?;
            Ok(PastMass { mass, cell })
        })
        .collect()
}

/// Médiane d'une liste non vide (`0` si elle est vide : l'appelant l'écarte avant).
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    match sorted.len() {
        0 => 0.0,
        n if n % 2 == 1 => sorted[mid],
        _ => (sorted[mid - 1] + sorted[mid]) / 2.0,
    }
}

/// Mises estimées d'une prochaine grille : médiane, sur les `window` dernières grilles de la même formule closes avant elle,
/// de la masse du rang 2 divisée par sa part des mises (55 % de 70 %). `None` sans aucun rapport.
///
/// Les grilles comparées sont celles de même jour et de même présence d'un pactole ; sous [`MIN_STAKES_CELL`] valeurs, celles
/// de même jour, puis toutes. Les mises varient du simple au double selon le jour et sont plus fortes avec un pactole. Sans
/// pactole, cette erreur pèse peu : masses et concurrents croissent ensemble (élasticité de l'espérance aux mises de −0,02).
/// Avec un pactole, la masse du rang 1 est fixe : des mises sous-estimées font surestimer l'espérance (élasticité de −0,5,
/// `docs/sources.md` §64).
pub fn stakes_estimate(conn: &Connection, grid: &GridRow, window: usize) -> Result<Option<Euros>> {
    let share = grid.id.game.rank_shares()[1] * PAYOUT_RATE;
    let past = past_masses(conn, grid)?;
    let day = paris_day(grid.closes_at);
    let cell = Some((day, grid.jackpot.is_some()));
    // `past.iter()` emprunte la liste sans la consommer : les trois lectures partent de la même liste, déjà triée du plus
    // récent au plus ancien.
    let by_cell: Vec<f64> = past
        .iter()
        .filter(|p| p.cell == cell)
        .map(|p| p.mass)
        .collect();
    let by_day: Vec<f64> = past
        .iter()
        .filter(|p| p.cell.is_some_and(|(past_day, _)| past_day == day))
        .map(|p| p.mass)
        .collect();
    let masses = if by_cell.len() >= MIN_STAKES_CELL {
        by_cell
    } else if by_day.len() >= MIN_STAKES_CELL {
        by_day
    } else {
        past.iter().map(|p| p.mass).collect()
    };
    if masses.is_empty() {
        return Ok(None);
    }
    let recent = &masses[..window.min(masses.len())];
    Ok(Euros::new(median(recent) / share))
}

/// Résultats d'une grille, dans l'ordre des rencontres, si ceux de toutes ses rencontres sont connus. Une rencontre annulée
/// y figure comme telle ([`MatchResult::Canceled`]), et non comme un signe : le règlement la compte bonne pour tous.
pub fn results(conn: &Connection, id: GridId) -> Result<Option<Vec<MatchResult>>> {
    let mut stmt = conn.prepare(
        "SELECT outcome FROM lotofoot_result WHERE game = ?1 AND year = ?2 AND number = ?3 ORDER BY position",
    )?;
    let codes = stmt
        .query_map(params![id.game.code(), id.year, id.number], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if codes.len() != id.game.matches() {
        return Ok(None);
    }
    // `collect` vers `Option<Vec<_>>` : un code inconnu en base rend `None` pour toute la grille, comme un résultat manquant.
    Ok(codes.iter().map(|c| MatchResult::from_code(c)).collect())
}

/// `true` si un résultat ou un rapport de la grille a été relevé après `at` : une séance de cet instant ne peut pas le
/// connaître (CLAUDE.md, invariant 1).
pub fn published_after(conn: &Connection, id: GridId, at: DateTime<Utc>) -> Result<bool> {
    let later: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (
             SELECT fetch_id FROM lotofoot_result WHERE game = ?1 AND year = ?2 AND number = ?3
             UNION ALL
             SELECT fetch_id FROM lotofoot_rank WHERE game = ?1 AND year = ?2 AND number = ?3
         ) published JOIN lotofoot_fetch f ON f.id = published.fetch_id
         WHERE f.fetched_at > ?4",
        params![id.game.code(), id.year, id.number, timestamp(at)],
        |row| row.get(0),
    )?;
    Ok(later > 0)
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
pub fn ranks(conn: &Connection, id: GridId) -> Result<Vec<PublishedRank>> {
    let mut stmt = conn.prepare(
        "SELECT rank, winners, unit_gain FROM lotofoot_rank
         WHERE game = ?1 AND year = ?2 AND number = ?3 ORDER BY rank",
    )?;
    let rows = stmt
        .query_map(params![id.game.code(), id.year, id.number], |row| {
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
    /// Grille.
    pub grid: GridId,
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
            "INSERT INTO lotofoot_reco (session_id, game, year, number, ticket, position, ev_base, ev_pessimistic,
                 paid_probability, stakes_estimate, split_fetch_id, inputs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                session_id,
                reco.grid.game.code(),
                reco.grid.year,
                reco.grid.number,
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

/// Paramètres d'une séance (JSON), `None` si elle n'existe pas.
pub fn session_params(conn: &Connection, id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT params FROM lotofoot_session WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?)
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
    /// Grille.
    pub grid: GridId,
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
///
/// Un ticket calculé sur une répartition antérieure à la composition en vigueur de sa grille n'est plus en vigueur
/// (§52), sauf s'il a été déclaré joué : il reste alors suivi, et la grille est bloquée ([`Recomposition`]).
///
/// `replaces` liste `[formule, numéro, année]` ; les séances écrites avant la migration 22, immuables, n'ont pas d'année :
/// elles ne peuvent désigner que 2026 (§66).
pub fn current_recos(conn: &Connection) -> Result<Vec<RecoRow>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.session_id, s.created_at, r.game, r.year, r.number, r.ticket, r.position, r.ev_base, r.ev_pessimistic,
                r.paid_probability, r.stakes_estimate, r.inputs
         FROM lotofoot_reco r JOIN lotofoot_session s ON s.id = r.session_id
         WHERE r.session_id = (SELECT MAX(r2.session_id) FROM lotofoot_reco r2
                               WHERE r2.game = r.game AND r2.year = r.year AND r2.number = r.number)
           AND NOT EXISTS (SELECT 1 FROM lotofoot_session s2, json_each(s2.params, '$.replaces') j
                           WHERE s2.id > r.session_id
                             AND json_extract(j.value, '$[0]') = r.game
                             AND json_extract(j.value, '$[1]') = r.number
                             AND COALESCE(json_extract(j.value, '$[2]'), 2026) = r.year)
           AND (r.split_fetch_id >= COALESCE((SELECT MAX(v.fetch_id) FROM lotofoot_match_revision v
                                              WHERE v.game = r.game AND v.year = r.year AND v.number = r.number), 0)
                OR EXISTS (SELECT 1 FROM lotofoot_play p WHERE p.reco_id = r.id))
         ORDER BY r.session_id, r.position",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i32>(4)?,
                row.get::<_, u32>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, u32>(7)?,
                row.get::<_, f64>(8)?,
                row.get::<_, f64>(9)?,
                row.get::<_, f64>(10)?,
                row.get::<_, f64>(11)?,
                row.get::<_, String>(12)?,
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
                year,
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
                    grid: GridId {
                        game: game_of(&game)?,
                        year,
                        number,
                    },
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
    /// Grille.
    pub grid: GridId,
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
        "INSERT INTO lotofoot_play (played_at, game, year, number, ticket, stake, reco_id, off_model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            timestamp(play.played_at),
            play.grid.game.code(),
            play.grid.year,
            play.grid.number,
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
    /// Grille.
    pub grid: GridId,
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
        "SELECT id, played_at, game, year, number, ticket, stake, reco_id FROM lotofoot_play ORDER BY id",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raw.into_iter()
        .map(
            |(id, played_at, game, year, number, ticket, stake, reco_id)| {
                Ok(PlayRow {
                    id,
                    played_at: parse_timestamp(&played_at)?,
                    grid: GridId {
                        game: game_of(&game)?,
                        year,
                        number,
                    },
                    ticket: Ticket::parse(&ticket).ok_or_else(|| {
                        Error::Data(format!("ticket illisible en base : {ticket}"))
                    })?,
                    stake,
                    reco_id,
                })
            },
        )
        .collect()
}

/// Nombre de tickets déclarés joués sur une grille. Sert à signaler une grille entièrement annulée (`drawCancelled`,
/// `docs/sources.md` §74) à qui a joué dessus : les mises sont remboursées au guichet, et à saisir à la main. Rien n'est
/// réglé automatiquement ici ; `played_count(..)? > 0` dit s'il y a quelque chose à signaler.
pub fn played_count(conn: &Connection, id: GridId) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM lotofoot_play WHERE game = ?1 AND year = ?2 AND number = ?3",
        params![id.game.code(), id.year, id.number],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(count).unwrap_or(usize::MAX))
}

/// Répartition finale d'une grille (dernier relevé avant sa clôture), pour la vérification du modèle.
pub fn final_split(conn: &Connection, id: GridId) -> Result<Option<Vec<[u32; 3]>>> {
    let Some(grid) = grid(conn, id)? else {
        return Ok(None);
    };
    Ok(latest_split(conn, id, grid.closes_at)?.map(|(_, split)| split))
}

/// Grilles dont les résultats sont connus, par formule, année et numéro croissants.
pub fn settled_grids(conn: &Connection) -> Result<Vec<GridId>> {
    let mut stmt = conn.prepare(
        "SELECT game, year, number FROM lotofoot_result GROUP BY game, year, number ORDER BY game, year, number",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(raw
        .into_iter()
        .filter_map(|(game, year, number)| {
            Game::from_code(&game).map(|game| GridId { game, year, number })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Outcome;
    use crate::lotofoot::grid_of_2026;
    use crate::lotofoot::grid_page::{OpenGrid, RankLine};
    use crate::store::migrate::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn the_health_of_a_page_is_its_last_good_read_and_its_current_error() {
        let conn = db();
        let lf8 = Page::Grid(Game::Lf8);

        // Jamais relevée : ni lecture réussie, ni erreur en cours.
        let never = page_health(&conn, Source::PronoMatch, lf8).unwrap();
        assert_eq!((never.last_ok, never.failing), (None, None));

        record_fetch(
            &conn,
            at("2026-11-05T12:07:00Z"),
            Source::PronoMatch,
            lf8,
            200,
            None,
        )
        .unwrap();
        record_fetch(
            &conn,
            at("2026-11-06T22:07:00Z"),
            Source::PronoMatch,
            lf8,
            200,
            Some("page périmée"),
        )
        .unwrap();
        // La page du LF7, elle, se lit bien : son état ne se mêle pas à celui du LF8.
        record_fetch(
            &conn,
            at("2026-11-06T22:07:05Z"),
            Source::PronoMatch,
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();

        let failing = page_health(&conn, Source::PronoMatch, lf8).unwrap();
        assert_eq!(failing.last_ok, Some(at("2026-11-05T12:07:00Z")));
        assert_eq!(failing.failing.as_deref(), Some("page périmée"));
        assert_eq!(
            page_health(&conn, Source::PronoMatch, Page::Grid(Game::Lf7))
                .unwrap()
                .failing,
            None
        );

        // Le site revient : la dernière lecture réussit, l'erreur disparaît.
        record_fetch(
            &conn,
            at("2026-11-13T09:07:00Z"),
            Source::PronoMatch,
            lf8,
            200,
            None,
        )
        .unwrap();
        let back = page_health(&conn, Source::PronoMatch, lf8).unwrap();
        assert_eq!(back.last_ok, Some(at("2026-11-13T09:07:00Z")));
        assert_eq!(back.failing, None);
    }

    /// Une lecture de `page` chez `source` à `when`, en erreur si `error` est donné.
    fn read(conn: &Connection, when: &str, source: Source, page: Page, error: Option<&str>) {
        let status = if error.is_some() { 403 } else { 200 };
        record_fetch(conn, at(when), source, page, status, error).unwrap();
    }

    /// La page est servie par la FDJ, puis par le repli quand la FDJ échoue, puis par personne quand le repli échoue aussi ;
    /// et la FDJ revenue reprend la main. Une page jamais lue n'a pas d'état.
    #[test]
    fn a_page_is_served_by_the_fdj_then_the_fallback_then_nobody() {
        let conn = db();
        let state = |conn: &Connection| SourcesHealth::read(conn, Page::Open).unwrap();
        assert_eq!(state(&conn).serving(), None);
        assert_eq!(state(&conn).last_ok(), None);

        read(&conn, "2026-11-15T09:00:00Z", Source::Fdj, Page::Open, None);
        assert_eq!(state(&conn).serving(), Some(Serving::Fdj));

        // La FDJ refuse, le repli lit : repli en service.
        read(
            &conn,
            "2026-11-15T10:00:00Z",
            Source::Fdj,
            Page::Open,
            Some("HTTP 403"),
        );
        read(
            &conn,
            "2026-11-15T10:00:05Z",
            Source::ResultatsFdj,
            Page::Open,
            None,
        );
        let fallback = state(&conn);
        assert_eq!(fallback.serving(), Some(Serving::Fallback));
        assert_eq!(fallback.last_ok(), Some(at("2026-11-15T10:00:05Z")));

        // Le repli échoue à son tour : personne ne sert la page.
        read(
            &conn,
            "2026-11-15T11:00:00Z",
            Source::Fdj,
            Page::Open,
            Some("HTTP 403"),
        );
        read(
            &conn,
            "2026-11-15T11:00:05Z",
            Source::ResultatsFdj,
            Page::Open,
            Some("HTTP 503"),
        );
        let down = state(&conn);
        assert_eq!(down.serving(), Some(Serving::Failing));
        assert_eq!(down.fallback.failing.as_deref(), Some("HTTP 503"));
        // La dernière lecture réussie reste celle du repli, à 10 h.
        assert_eq!(down.last_ok(), Some(at("2026-11-15T10:00:05Z")));

        // La FDJ revient : l'ancien échec du repli ne compte plus.
        read(&conn, "2026-11-15T12:00:00Z", Source::Fdj, Page::Open, None);
        assert_eq!(state(&conn).serving(), Some(Serving::Fdj));
    }

    /// Une page FDJ périmée (lue, mais en erreur) sans repli lu depuis : personne ne la sert. Un repli lu AVANT la dernière
    /// lecture réussie de la FDJ ne la sert pas non plus (il date d'une autre panne).
    #[test]
    fn a_stale_fdj_page_or_an_old_fallback_read_does_not_count_as_served() {
        let conn = db();
        read(
            &conn,
            "2026-11-10T09:00:00Z",
            Source::ResultatsFdj,
            Page::Results,
            None,
        );
        read(
            &conn,
            "2026-11-15T09:00:00Z",
            Source::Fdj,
            Page::Results,
            None,
        );
        read(
            &conn,
            "2026-11-15T10:00:00Z",
            Source::Fdj,
            Page::Results,
            Some("page périmée (FDJ)"),
        );
        let health = SourcesHealth::read(&conn, Page::Results).unwrap();
        assert_eq!(health.serving(), Some(Serving::Failing));
        // Les pages sont indépendantes : la page des grilles ouvertes n'a jamais été lue.
        assert_eq!(
            SourcesHealth::read(&conn, Page::Open).unwrap().serving(),
            None
        );
    }

    /// La même page chez deux sources a deux états : l'échec de la FDJ ne dit rien de resultats-fdj.fr, ni de prono-match.
    #[test]
    fn the_health_of_a_page_does_not_mix_sources() {
        let conn = db();
        record_fetch(
            &conn,
            at("2026-11-15T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_fetch(
            &conn,
            at("2026-11-15T10:00:00Z"),
            Source::Fdj,
            Page::Open,
            503,
            Some("réponse HTTP inattendue"),
        )
        .unwrap();
        record_fetch(
            &conn,
            at("2026-11-15T10:00:05Z"),
            Source::ResultatsFdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();

        let fdj = page_health(&conn, Source::Fdj, Page::Open).unwrap();
        assert_eq!(fdj.last_ok, Some(at("2026-11-15T09:00:00Z")));
        assert_eq!(fdj.failing.as_deref(), Some("réponse HTTP inattendue"));
        let fallback = page_health(&conn, Source::ResultatsFdj, Page::Open).unwrap();
        assert_eq!(fallback.last_ok, Some(at("2026-11-15T10:00:05Z")));
        assert_eq!(fallback.failing, None);
        // prono-match (archive) n'a aucune lecture de cette page : son état reste vide.
        let old = page_health(&conn, Source::PronoMatch, Page::Open).unwrap();
        assert_eq!((old.last_ok, old.failing), (None, None));
        // Les pages du détail et des résultats s'enregistrent aussi (migration 24).
        record_fetch(
            &conn,
            at("2026-11-15T10:01:00Z"),
            Source::Fdj,
            Page::Detail,
            200,
            None,
        )
        .unwrap();
        record_fetch(
            &conn,
            at("2026-11-15T10:02:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
    }

    fn page(number: u32, split: Option<Vec<[u32; 3]>>) -> OpenGrid {
        OpenGrid {
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
        }
    }

    /// Écrit les deux rangs de la grille LF7 `number` de 2026 : (gagnants, gain unitaire) du rang 1, puis du rang 2.
    fn write_ranks(conn: &Connection, fetch_id: i64, number: u32, ranks: [(u32, f64); 2]) {
        for (i, (winners, gain)) in ranks.into_iter().enumerate() {
            write_rank(
                conn,
                grid_of_2026(Game::Lf7, number),
                int(i + 1),
                &rank(winners, gain),
                fetch_id,
            )
            .unwrap();
        }
    }

    fn rank(winners: u32, gain: f64) -> RankLine {
        RankLine {
            good: 7,
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
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        assert_eq!(
            record_open_grid(&mut conn, &Aliases::default(), f1, &page(133, None))
                .unwrap()
                .splits,
            0
        );
        let f2 = record_fetch(
            &conn,
            at("2026-10-24T10:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        assert_eq!(
            record_open_grid(
                &mut conn,
                &Aliases::default(),
                f2,
                &page(133, Some(vec![[50, 30, 20]; 7]))
            )
            .unwrap()
            .splits,
            7
        );
        let f3 = record_fetch(
            &conn,
            at("2026-10-24T11:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(
            &mut conn,
            &Aliases::default(),
            f3,
            &page(133, Some(vec![[52, 29, 19]; 7])),
        )
        .unwrap();

        let grids = grids_closing_after(&conn, at("2026-10-24T12:00:00Z")).unwrap();
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].matches.len(), 7);
        // Pas de fuite du futur : à 10 h 30, seul le relevé de 10 h est visible.
        let (id, split) = latest_split(
            &conn,
            grid_of_2026(Game::Lf7, 133),
            at("2026-10-24T10:30:00Z"),
        )
        .unwrap()
        .unwrap();
        assert_eq!((id, split[0]), (f2, [50, 30, 20]));
        let (id, _) = latest_split(
            &conn,
            grid_of_2026(Game::Lf7, 133),
            at("2026-10-24T12:00:00Z"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(id, f3);
        assert!(
            latest_split(
                &conn,
                grid_of_2026(Game::Lf7, 133),
                at("2026-10-24T09:30:00Z")
            )
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
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(&mut conn, &Aliases::default(), f, &page(133, None)).unwrap();
        // Rapports des trois grilles précédentes, tous clos le 20 octobre.
        write_ranks(&conn, f, 131, [(135, 578.0), (2393, 39.8)]);
        write_ranks(&conn, f, 130, [(588, 170.0), (7124, 16.6)]);
        write_ranks(&conn, f, 129, [(2539, 24.0), (17631, 4.2)]);
        let open = target(Game::Lf7, 133, "2026-10-25T16:10:00Z", None);
        let stakes = stakes_estimate(&conn, &open, 8).unwrap().unwrap().get();
        // Masses du rang 2 : 95 241,4 ; 118 258,4 ; 74 050,2 → médiane 95 241,4 / 0,385.
        assert!((stakes - 2393.0 * 39.8 / 0.385).abs() < 1e-6);
        let other = target(Game::Lf8, 200, "2026-10-25T16:10:00Z", None);
        assert!(stakes_estimate(&conn, &other, 8).unwrap().is_none());
    }

    fn target(game: Game, number: u32, closes_at: &str, jackpot: Option<f64>) -> GridRow {
        GridRow {
            id: GridId::closing(game, number, at(closes_at)),
            closes_at: at(closes_at),
            jackpot: jackpot.and_then(Euros::new),
            matches: Vec::new(),
        }
    }

    /// Grille LF7 close, de masse du rang 2 `gagnants × 10 €`.
    fn closed_grid(
        conn: &Connection,
        fetch: i64,
        number: u32,
        closes_at: &str,
        jackpot: Option<f64>,
        winners: u32,
    ) {
        conn.execute(
            "INSERT INTO lotofoot_grid (game, year, number, closes_at, jackpot, fetch_id)
             VALUES ('lf7', 2026, ?1, ?2, ?3, ?4)",
            params![number, closes_at, jackpot, fetch],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO lotofoot_rank (game, year, number, rank, winners, unit_gain, fetch_id)
             VALUES ('lf7', 2026, ?1, 2, ?2, 10.0, ?3)",
            params![number, winners, fetch],
        )
        .unwrap();
    }

    #[test]
    fn stakes_use_the_same_day_and_jackpot_then_wider_cells() {
        let conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        // Trois dimanches à pactole, un dimanche sans, un lundi sans.
        closed_grid(&conn, f, 10, "2026-10-04T16:00:00Z", Some(1e5), 300);
        closed_grid(&conn, f, 11, "2026-10-11T16:00:00Z", Some(1e5), 310);
        closed_grid(&conn, f, 12, "2026-10-18T16:00:00Z", Some(1e5), 320);
        closed_grid(&conn, f, 13, "2026-10-25T16:00:00Z", None, 100);
        closed_grid(&conn, f, 14, "2026-10-26T18:00:00Z", None, 50);
        // Pas de fuite du futur : une grille close après la cible ne compte jamais, malgré sa case et son numéro.
        closed_grid(&conn, f, 15, "2026-11-08T16:00:00Z", Some(1e5), 9000);
        let estimate = |grid: &GridRow, window| {
            stakes_estimate(&conn, grid, window).unwrap().unwrap().get() * 0.385
        };
        // Même jour, même pactole : 3 000, 3 100, 3 200.
        let sunday = target(Game::Lf7, 16, "2026-11-01T16:00:00Z", Some(1e5));
        assert!((estimate(&sunday, 8) - 3100.0).abs() < 1e-6);
        // La fenêtre garde les plus récentes : 3 100, 3 200.
        assert!((estimate(&sunday, 2) - 3150.0).abs() < 1e-6);
        // Une seule valeur dans la case : repli sur le jour (1 000, 3 000, 3 100, 3 200).
        let no_jackpot = target(Game::Lf7, 16, "2026-11-01T16:00:00Z", None);
        assert!((estimate(&no_jackpot, 8) - 3050.0).abs() < 1e-6);
        // Un seul lundi : repli sur toutes les grilles (500, 1 000, 3 000, 3 100, 3 200).
        let monday = target(Game::Lf7, 16, "2026-11-02T18:00:00Z", None);
        assert!((estimate(&monday, 8) - 3000.0).abs() < 1e-6);
    }

    #[test]
    fn the_day_of_a_closing_is_read_in_paris() {
        // Samedi 23 h 30 UTC : déjà dimanche à Paris.
        assert_eq!(paris_day(at("2026-10-24T23:30:00Z")), Weekday::Sun);
    }

    #[test]
    fn the_journal_refuses_changes() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(
            &mut conn,
            &Aliases::default(),
            f,
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
            grid: grid_of_2026(Game::Lf7, 133),
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
            grid: grid_of_2026(Game::Lf7, 133),
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

    /// LF7 n°133 tel que vu le 2026-10-24 (`docs/sources.md` §52) : annoncé avec PSG – Lyon, ouvert sans lui, avec
    /// Fribourg – Bayern en rencontre 6 ; les autres rencontres remontent d'un rang.
    const ANNOUNCED: [(&str, &str); 7] = [
        ("Paris SG", "Lyon"),
        ("Marseille", "Le Havre"),
        ("Rennes", "Strasbourg"),
        ("Auxerre", "Le Mans"),
        ("Angers", "Lorient"),
        ("Toulouse", "Troyes"),
        ("FC Barcelone", "Real Madrid"),
    ];
    const OPENED: [(&str, &str); 7] = [
        ("Marseille", "Le Havre"),
        ("Rennes", "Strasbourg"),
        ("Auxerre", "Le Mans"),
        ("Angers", "Lorient"),
        ("Toulouse", "Troyes"),
        ("Fribourg", "Bayern Munich"),
        ("FC Barcelone", "Real Madrid"),
    ];

    fn composed(matches: &[(&str, &str)], split: Option<Vec<[u32; 3]>>) -> OpenGrid {
        let mut grid = page(133, split);
        grid.matches = matches
            .iter()
            .map(|(h, a)| (h.to_string(), a.to_string()))
            .collect();
        grid
    }

    fn results_of(matches: &[(&str, &str)]) -> GridResult {
        GridResult {
            game: Game::Lf7,
            number: 133,
            date: chrono::NaiveDate::from_ymd_opt(2026, 10, 25),
            closes_at: None,
            matches: matches
                .iter()
                .map(|(h, a)| {
                    (
                        h.to_string(),
                        a.to_string(),
                        MatchResult::Sign(Outcome::Home),
                    )
                })
                .collect(),
            ranks: vec![rank(135, 578.0), rank(2393, 39.8)],
        }
    }

    /// Un ticket recommandé sur la répartition du relevé `split_fetch_id`.
    fn recommend(conn: &mut Connection, split_fetch_id: i64, ticket: &Ticket) {
        let session = NewSession {
            created_at: at("2026-10-24T03:30:00Z"),
            model_version: "lotofoot-1",
            params: "{}",
            notes: "{}",
        };
        let reco = NewReco {
            grid: grid_of_2026(Game::Lf7, 133),
            ticket,
            position: 1,
            ev_base: 11.0,
            ev_pessimistic: 9.0,
            paid_probability: 0.3,
            stakes_estimate: 235_000.0,
            split_fetch_id,
            inputs: "{}",
        };
        record_session(conn, &session, &[reco]).unwrap();
    }

    #[test]
    fn a_changed_composition_voids_earlier_splits_and_tickets() {
        let mut conn = db();
        let grid_fetch = |conn: &Connection, when: &str| {
            record_fetch(conn, at(when), Source::Fdj, Page::Open, 200, None).unwrap()
        };
        let f1 = grid_fetch(&conn, "2026-10-24T03:07:00Z");
        let first = record_open_grid(
            &mut conn,
            &Aliases::default(),
            f1,
            &composed(&ANNOUNCED, Some(vec![[50, 30, 20]; 7])),
        )
        .unwrap();
        assert!(!first.recomposed);
        let ticket = Ticket::parse("1111111").unwrap();
        recommend(&mut conn, f1, &ticket);
        assert_eq!(current_recos(&conn).unwrap().len(), 1);
        assert_eq!(
            recomposition(&conn, grid_of_2026(Game::Lf7, 133)).unwrap(),
            None
        );

        // La grille ouvre avec d'autres rencontres : nouvelle composition, rattachée au relevé qui l'a vue.
        let f2 = grid_fetch(&conn, "2026-10-24T06:07:00Z");
        let opened = record_open_grid(
            &mut conn,
            &Aliases::default(),
            f2,
            &composed(&OPENED, Some(vec![[40, 30, 30]; 7])),
        )
        .unwrap();
        assert!(opened.recomposed);
        assert_eq!(opened.splits, 7);
        let grid = grid(&conn, grid_of_2026(Game::Lf7, 133)).unwrap().unwrap();
        assert_eq!(grid.matches[5], ("Fribourg".into(), "Bayern Munich".into()));
        let change = recomposition(&conn, grid_of_2026(Game::Lf7, 133))
            .unwrap()
            .unwrap();
        assert_eq!((change.fetch_id, change.played_before), (f2, false));
        assert_eq!(
            first_seen(&conn, grid_of_2026(Game::Lf7, 133)).unwrap(),
            Some(at("2026-10-24T06:07:00Z"))
        );
        // La répartition antérieure ne vaut plus : avant le nouveau relevé, il n'y en a aucune.
        assert_eq!(
            latest_split(
                &conn,
                grid_of_2026(Game::Lf7, 133),
                at("2026-10-24T05:00:00Z")
            )
            .unwrap(),
            None
        );
        let (split_fetch, split) = latest_split(
            &conn,
            grid_of_2026(Game::Lf7, 133),
            at("2026-10-24T07:00:00Z"),
        )
        .unwrap()
        .unwrap();
        assert_eq!((split_fetch, split[0]), (f2, [40, 30, 30]));
        // Le ticket calculé sur l'ancienne composition n'est plus en vigueur, mais reste au journal.
        assert!(current_recos(&conn).unwrap().is_empty());
        recommend(&mut conn, f2, &ticket);
        assert_eq!(current_recos(&conn).unwrap().len(), 1);

        // Même composition au relevé suivant : rien de nouveau.
        let f3 = grid_fetch(&conn, "2026-10-24T07:07:00Z");
        let again =
            record_open_grid(&mut conn, &Aliases::default(), f3, &composed(&OPENED, None)).unwrap();
        assert!(!again.recomposed);
        let revisions: i64 = conn
            .query_row("SELECT COUNT(*) FROM lotofoot_match_revision", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(revisions, 7);
        assert!(
            conn.execute("DELETE FROM lotofoot_match_revision", [])
                .is_err()
        );

        // Les résultats s'écrivent par position : seuls ceux de la composition en vigueur sont acceptés.
        let r = record_fetch(
            &conn,
            at("2026-10-26T09:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let refused =
            record_results(&mut conn, &Aliases::default(), r, &[results_of(&ANNOUNCED)]).unwrap();
        assert_eq!(refused.new, 0);
        assert!(refused.refused[0].contains("LF7 n°133"), "{refused:?}");
        assert_eq!(results(&conn, grid_of_2026(Game::Lf7, 133)).unwrap(), None);
        let written =
            record_results(&mut conn, &Aliases::default(), r, &[results_of(&OPENED)]).unwrap();
        assert_eq!((written.new, written.refused.len()), (1, 0));
    }

    #[test]
    fn a_grid_changed_after_a_declared_ticket_is_blocked() {
        let mut conn = db();
        let f1 = record_fetch(
            &conn,
            at("2026-10-24T03:07:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(
            &mut conn,
            &Aliases::default(),
            f1,
            &composed(&ANNOUNCED, Some(vec![[50, 30, 20]; 7])),
        )
        .unwrap();
        let ticket = Ticket::parse("1111111").unwrap();
        recommend(&mut conn, f1, &ticket);
        let reco_id = current_recos(&conn).unwrap()[0].id;
        record_play(
            &conn,
            &NewPlay {
                played_at: at("2026-10-24T04:00:00Z"),
                grid: grid_of_2026(Game::Lf7, 133),
                ticket: &ticket,
                stake: Euros::new(1.0).unwrap(),
                reco_id: Some(reco_id),
            },
        )
        .unwrap();
        let f2 = record_fetch(
            &conn,
            at("2026-10-24T06:07:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(&mut conn, &Aliases::default(), f2, &composed(&OPENED, None)).unwrap();
        let change = recomposition(&conn, grid_of_2026(Game::Lf7, 133))
            .unwrap()
            .unwrap();
        assert!(change.played_before);
        // Le ticket joué reste suivi ; ses résultats ne sont pas réglés automatiquement.
        assert_eq!(current_recos(&conn).unwrap().len(), 1);
        let r = record_fetch(
            &conn,
            at("2026-10-26T09:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let record =
            record_results(&mut conn, &Aliases::default(), r, &[results_of(&OPENED)]).unwrap();
        assert_eq!(record.new, 0);
        assert!(record.refused[0].contains("à la main"), "{record:?}");
    }

    /// Grille LF7 `number` qui ferme à `closes` (rencontres propres au tag).
    fn year_page(number: u32, closes: &str, tag: &str, split: bool) -> OpenGrid {
        OpenGrid {
            game: Game::Lf7,
            accepting: true,
            number,
            closes_at: at(closes),
            jackpot: None,
            matches: (1..=7)
                .map(|i| (format!("{tag}H{i}"), format!("{tag}A{i}")))
                .collect(),
            split: split.then(|| vec![[50, 30, 20]; 7]),
            split_at: None,
        }
    }

    fn lf7(year: i32, number: u32) -> GridId {
        GridId {
            game: Game::Lf7,
            year,
            number,
        }
    }

    fn results_page(
        number: u32,
        date: Option<(i32, u32, u32)>,
        tag: &str,
        gain: f64,
    ) -> GridResult {
        GridResult {
            game: Game::Lf7,
            number,
            date: date.and_then(|(y, m, d)| chrono::NaiveDate::from_ymd_opt(y, m, d)),
            closes_at: None,
            matches: (1..=7)
                .map(|i| {
                    (
                        format!("{tag}H{i}"),
                        format!("{tag}A{i}"),
                        MatchResult::Sign(Outcome::Away),
                    )
                })
                .collect(),
            ranks: vec![rank(7, gain), rank(77, gain / 10.0)],
        }
    }

    /// Le scénario du §66 : une grille de 2027 reprend le numéro d'une grille de 2026, avec un ticket jamais joué.
    #[test]
    fn a_new_year_reuses_numbers_without_touching_the_old_grid() {
        let mut conn = db();
        let grid_fetch = |conn: &Connection, when: &str| {
            record_fetch(conn, at(when), Source::Fdj, Page::Open, 200, None).unwrap()
        };
        let f1 = grid_fetch(&conn, "2026-10-24T09:00:00Z");
        let old = year_page(133, "2026-10-25T16:10:00Z", "A", true);
        record_open_grid(&mut conn, &Aliases::default(), f1, &old).unwrap();
        let ticket = Ticket::parse("1111111").unwrap();
        recommend(&mut conn, f1, &ticket);
        let r1 = record_fetch(
            &conn,
            at("2026-10-26T09:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let old_results = results_page(133, Some((2026, 10, 25)), "A", 500.0);
        assert_eq!(
            record_results(&mut conn, &Aliases::default(), r1, &[old_results])
                .unwrap()
                .new,
            1
        );
        let before = current_recos(&conn).unwrap();
        assert_eq!(before.len(), 1);

        let f2 = grid_fetch(&conn, "2027-10-20T09:00:00Z");
        let new = year_page(133, "2027-10-24T16:10:00Z", "B", true);
        let record = record_open_grid(&mut conn, &Aliases::default(), f2, &new).unwrap();
        // Pas de fausse recomposition : c'est une autre grille, visible, avec ses rencontres.
        assert!(!record.recomposed);
        assert_eq!(record.refused, None);
        let open = grids_closing_after(&conn, at("2027-10-20T10:00:00Z")).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, lf7(2027, 133));
        assert_eq!(open[0].matches[0], ("BH1".to_string(), "BA1".to_string()));
        assert_eq!(
            latest_split(&conn, lf7(2027, 133), at("2027-10-21T09:00:00Z"))
                .unwrap()
                .map(|(fetch, _)| fetch),
            Some(f2)
        );

        // Rien ne bouge pour 2026 : composition, ticket non joué, bilan.
        let old_grid = grid(&conn, lf7(2026, 133)).unwrap().unwrap();
        assert_eq!(old_grid.matches[0], ("AH1".to_string(), "AA1".to_string()));
        assert_eq!(old_grid.closes_at, at("2026-10-25T16:10:00Z"));
        assert_eq!(recomposition(&conn, lf7(2026, 133)).unwrap(), None);
        assert_eq!(current_recos(&conn).unwrap(), before);

        // Résultats et rapports de chaque année restent séparés.
        let r2 = record_fetch(
            &conn,
            at("2027-10-25T09:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let new_results = results_page(133, Some((2027, 10, 24)), "B", 900.0);
        let written = record_results(&mut conn, &Aliases::default(), r2, &[new_results]).unwrap();
        assert_eq!((written.new, written.refused.len()), (1, 0));
        let gain = |year| {
            ranks(&conn, lf7(year, 133)).unwrap()[0]
                .unit_gain
                .map(Euros::get)
        };
        assert_eq!((gain(2026), gain(2027)), (Some(500.0), Some(900.0)));
        assert_eq!(
            settled_grids(&conn).unwrap(),
            vec![lf7(2026, 133), lf7(2027, 133)]
        );
    }

    /// Le n°1 de janvier estime ses mises sur les rapports de décembre : la fenêtre passe à cheval sur l'année, les rapports
    /// restant rangés sous l'année de leur grille.
    #[test]
    fn a_january_grid_sees_the_reports_of_december() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2027-01-02T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        let page = year_page(1, "2027-01-03T13:55:00Z", "N", false);
        record_open_grid(&mut conn, &Aliases::default(), f, &page).unwrap();
        // Rapport du n°165, clos le 30 décembre 2026 : rangé sous 2026, sans ligne de grille (jamais vue ouverte).
        let december = GridId {
            game: Game::Lf7,
            year: 2026,
            number: 165,
        };
        for (i, (winners, gain)) in [(100, 500.0), (3000, 10.0)].into_iter().enumerate() {
            write_rank(&conn, december, int(i + 1), &rank(winners, gain), f).unwrap();
        }
        assert_eq!(ranks(&conn, december).unwrap().len(), 2);
        assert!(ranks(&conn, lf7(2027, 165)).unwrap().is_empty());
        let open = grids_closing_after(&conn, at("2027-01-02T10:00:00Z")).unwrap();
        let stakes = stakes_estimate(&conn, &open[0], 8).unwrap().unwrap().get();
        assert!((stakes - 3000.0 * 10.0 / 0.385).abs() < 1e-6);
    }

    /// Un résultat sans jour de clôture lisible n'est pas écrit : son année serait devinée.
    #[test]
    fn a_result_without_a_readable_date_is_refused_not_guessed() {
        let mut conn = db();
        let r = record_fetch(
            &conn,
            at("2027-01-02T09:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let record = record_results(
            &mut conn,
            &Aliases::default(),
            r,
            &[results_page(1, None, "N", 90.0)],
        )
        .unwrap();
        assert_eq!(record.new, 0);
        assert!(record.refused[0].contains("date illisible"), "{record:?}");
        assert_eq!(rank_rows(&conn), 0);
    }

    fn rank_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM lotofoot_rank", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_closing_that_contradicts_the_year_is_refused_not_recomposed() {
        let mut conn = db();
        let f1 = record_fetch(
            &conn,
            at("2027-03-01T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        let page = year_page(40, "2027-03-02T18:00:00Z", "A", false);
        record_open_grid(&mut conn, &Aliases::default(), f1, &page).unwrap();

        // Même identité, clôture à un mois : ce n'est plus la même grille.
        let f2 = record_fetch(
            &conn,
            at("2027-04-01T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        let moved = year_page(40, "2027-04-02T18:00:00Z", "B", true);
        let record = record_open_grid(&mut conn, &Aliases::default(), f2, &moved).unwrap();
        assert!(!record.recomposed);
        assert!(record.refused.is_some_and(|r| r.contains("clôture")));
        let kept = grid(&conn, lf7(2027, 40)).unwrap().unwrap();
        assert_eq!(kept.matches[0], ("AH1".to_string(), "AA1".to_string()));
        assert_eq!(recomposition(&conn, lf7(2027, 40)).unwrap(), None);
        assert_eq!(
            latest_split(&conn, lf7(2027, 40), at("2027-04-02T09:00:00Z")).unwrap(),
            None
        );

        // Un report de quelques jours reste la même grille.
        let f3 = record_fetch(
            &conn,
            at("2027-03-02T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        let shifted = year_page(40, "2027-03-05T18:00:00Z", "A", false);
        let record = record_open_grid(&mut conn, &Aliases::default(), f3, &shifted).unwrap();
        assert_eq!(record.refused, None);

        // Numéro inférieur à une grille close plus tôt la même année : l'année attribuée serait fausse.
        let low = year_page(3, "2027-05-02T18:00:00Z", "C", false);
        let f4 = record_fetch(
            &conn,
            at("2027-05-01T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        let record = record_open_grid(&mut conn, &Aliases::default(), f4, &low).unwrap();
        assert!(record.refused.is_some_and(|r| r.contains("n°40")));
        assert_eq!(grid(&conn, lf7(2027, 3)).unwrap(), None);
    }

    #[test]
    fn replaces_without_a_year_only_mean_2026() {
        let mut conn = db();
        let f1 = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(
            &mut conn,
            &Aliases::default(),
            f1,
            &page(133, Some(vec![[50, 30, 20]; 7])),
        )
        .unwrap();
        let f2 = record_fetch(
            &conn,
            at("2027-10-24T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        let next = year_page(133, "2027-10-25T16:10:00Z", "B", true);
        record_open_grid(&mut conn, &Aliases::default(), f2, &next).unwrap();
        let ticket = Ticket::parse("1111111").unwrap();
        let reco = |grid, split_fetch_id| NewReco {
            grid,
            ticket: &ticket,
            position: 1,
            ev_base: 2.0,
            ev_pessimistic: 1.8,
            paid_probability: 0.1,
            stakes_estimate: 200_000.0,
            split_fetch_id,
            inputs: "{}",
        };
        let session = |params: &'static str| NewSession {
            created_at: at("2027-10-24T10:00:00Z"),
            model_version: "lotofoot-3",
            params,
            notes: "{}",
        };
        record_session(
            &mut conn,
            &session("{}"),
            &[reco(lf7(2026, 133), f1), reco(lf7(2027, 133), f2)],
        )
        .unwrap();
        // Séance écrite avant la migration 22 : `[formule, numéro]`, soit 2026.
        record_session(&mut conn, &session(r#"{"replaces": [["lf7", 133]]}"#), &[]).unwrap();
        let current: Vec<GridId> = current_recos(&conn)
            .unwrap()
            .iter()
            .map(|r| r.grid)
            .collect();
        assert_eq!(current, vec![lf7(2027, 133)]);
        // Séance récente : l'année est dite.
        record_session(
            &mut conn,
            &session(r#"{"replaces": [["lf7", 133, 2027]]}"#),
            &[],
        )
        .unwrap();
        assert!(current_recos(&conn).unwrap().is_empty());
    }

    #[test]
    fn a_second_identical_results_read_writes_nothing() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-23T09:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let lf7 = GridResult {
            game: Game::Lf7,
            number: 132,
            date: chrono::NaiveDate::from_ymd_opt(2026, 10, 25),
            closes_at: None,
            matches: (0..7)
                .map(|_| {
                    (
                        "a".to_string(),
                        "b".to_string(),
                        MatchResult::Sign(Outcome::Draw),
                    )
                })
                .collect(),
            ranks: vec![rank(135, 578.0), rank(2393, 39.8)],
        };
        // `from_ref` fait d'une référence un tableau d'un élément, sans cloner `lf7`, que le test relit ensuite.
        assert_eq!(
            record_results(
                &mut conn,
                &Aliases::default(),
                f,
                std::slice::from_ref(&lf7)
            )
            .unwrap()
            .new,
            1
        );
        // Un second relevé identique n'écrit rien de nouveau.
        assert_eq!(
            record_results(&mut conn, &Aliases::default(), f, &[lf7])
                .unwrap()
                .new,
            0
        );
        assert_eq!(
            results(&conn, grid_of_2026(Game::Lf7, 132)).unwrap(),
            Some(vec![MatchResult::Sign(Outcome::Draw); 7])
        );
        let ranks = ranks(&conn, grid_of_2026(Game::Lf7, 132)).unwrap();
        assert_eq!(ranks[1].winners, 2393);
        assert_eq!(
            settled_grids(&conn).unwrap(),
            vec![grid_of_2026(Game::Lf7, 132)]
        );
    }

    /// LF7 n°139 (`docs/sources.md` §74) : la composition est en base sous les noms de prono-match (`Atlético Madrid`), la
    /// page de la FDJ écrit `Atl. Madrid`. Avec la table d'alias, c'est la même rencontre ; sans elle, ou avec un nom
    /// que la table ne connaît pas, les résultats sont refusés, et le message nomme la rencontre.
    #[test]
    fn results_under_an_alias_are_matched_by_canonical_name_and_others_are_refused() {
        const STORED: [(&str, &str); 7] = [
            ("Atlético Madrid", "Barcelona"),
            ("Manchester City", "Arsenal"),
            ("Rennes", "Strasbourg"),
            ("Auxerre", "Angers"),
            ("Brest", "Lorient"),
            ("Toulouse", "Nantes"),
            ("Celtic", "Rangers"),
        ];
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(&mut conn, &Aliases::default(), f, &composed(&STORED, None)).unwrap();
        let r = record_fetch(
            &conn,
            at("2026-10-26T09:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let aliases = Aliases::bundled().unwrap();
        let mut abbreviated = STORED;
        abbreviated[0].0 = "Atl. Madrid";
        abbreviated[1].0 = "Man. City";

        // Sans table : deux noms bruts différents ne se rapprochent pas, jamais d'appariement approximatif.
        let without = record_results(
            &mut conn,
            &Aliases::default(),
            r,
            &[results_of(&abbreviated)],
        )
        .unwrap();
        assert_eq!(without.new, 0);
        assert!(
            without.refused[0].contains("rencontre 1 : « Atl. Madrid – Barcelona » lu, « Atlético Madrid – Barcelona » en base"),
            "{without:?}"
        );

        // Avec la table : même rencontre. Les noms écrits au journal restent ceux de la première composition.
        let with = record_results(&mut conn, &aliases, r, &[results_of(&abbreviated)]).unwrap();
        assert_eq!((with.new, with.refused.len()), (1, 0), "{with:?}");
        let id = grid_of_2026(Game::Lf7, 133);
        assert_eq!(
            grid(&conn, id).unwrap().unwrap().matches[0].0,
            "Atlético Madrid"
        );
    }

    /// Un nom que la table ne connaît pas reste refusé, même si un seul des deux noms est connu.
    #[test]
    fn a_name_the_alias_table_does_not_know_is_still_refused() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(&mut conn, &Aliases::default(), f, &composed(&OPENED, None)).unwrap();
        let r = record_fetch(
            &conn,
            at("2026-10-26T09:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let mut other = OPENED;
        // « Marseille » est connue de la table, « Olympique Inconnu » non : pas de rapprochement.
        other[0].0 = "Olympique Inconnu";
        let record = record_results(
            &mut conn,
            &Aliases::bundled().unwrap(),
            r,
            &[results_of(&other)],
        )
        .unwrap();
        assert_eq!(record.new, 0);
        assert!(record.refused[0].contains("rencontre 1"), "{record:?}");
    }

    /// Sept rencontres pour un LF7, la cinquième annulée, tirées de [`results_of`].
    fn lf7_with_a_canceled_match(number: u32) -> GridResult {
        let mut result = results_of(&[
            ("a", "b"),
            ("c", "d"),
            ("e", "f"),
            ("g", "h"),
            ("i", "j"),
            ("k", "l"),
            ("m", "n"),
        ]);
        result.number = number;
        result.matches[4].2 = MatchResult::Canceled;
        result
    }

    /// LF7 n°140 de la FDJ (`docs/sources.md` §74) : la rencontre 5 est annulée. La grille s'écrit, la rencontre annulée
    /// vaut `C`, et `results` la relit comme telle, avec les signes des autres rencontres.
    #[test]
    fn a_canceled_match_is_written_and_read_back() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-11-14T22:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let result = lf7_with_a_canceled_match(133);

        // `from_ref` fait d'une référence un tableau d'un élément, sans cloner `result`, que le test relit ensuite.
        let record = record_results(
            &mut conn,
            &Aliases::default(),
            f,
            std::slice::from_ref(&result),
        )
        .unwrap();

        assert_eq!((record.new, record.refused.len()), (1, 0), "{record:?}");
        let id = grid_of_2026(Game::Lf7, 133);
        let mut expected = vec![MatchResult::Sign(Outcome::Home); 7];
        expected[4] = MatchResult::Canceled;
        assert_eq!(results(&conn, id).unwrap(), Some(expected));
        let code: String = conn
            .query_row(
                "SELECT outcome FROM lotofoot_result WHERE game = 'lf7' AND number = 133 AND position = 5",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(code, "C");
        assert_eq!(settled_grids(&conn).unwrap(), vec![id]);
        assert_eq!(ranks(&conn, id).unwrap().len(), 2);
        // Un second relevé identique n'écrit rien : la première observation fait foi.
        let again = record_results(&mut conn, &Aliases::default(), f, &[result]).unwrap();
        assert_eq!(again.new, 0);
    }

    /// Une grille que l'archive ne sait pas écrire est refusée avec un message, jamais sautée en silence : ici un nombre
    /// de rencontres différent de la formule, un LF7 à 6 rencontres.
    #[test]
    fn a_result_with_the_wrong_number_of_matches_is_refused_with_a_message() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-11-14T22:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let mut short = lf7_with_a_canceled_match(134);
        short.matches.truncate(6);

        let record = record_results(&mut conn, &Aliases::default(), f, &[short]).unwrap();

        assert_eq!(record.new, 0);
        assert_eq!(record.refused.len(), 1);
        assert!(record.refused[0].contains("6 rencontres lues au lieu de 7"));
        assert!(settled_grids(&conn).unwrap().is_empty());
    }

    /// Le règlement permet un LF8 à 7 rencontres (§74), mais le modèle suppose 8 : la grille est refusée avec un message,
    /// et rien n'en est écrit (ni grille, ni rencontres, ni répartition).
    #[test]
    fn a_lf8_with_seven_matches_is_refused_loudly_and_not_written() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        let mut seven = page(150, Some(vec![[50, 30, 20]; 7]));
        seven.game = Game::Lf8;

        let record = record_open_grid(&mut conn, &Aliases::default(), f, &seven).unwrap();

        let reason = record.refused.as_deref().unwrap_or_default();
        assert!(
            reason.contains("LF8 n°150") && reason.contains("7 rencontres lues au lieu de 8"),
            "{reason}"
        );
        assert_eq!((record.splits, record.recomposed), (0, false));
        assert_eq!(count_grids(&conn).unwrap(), 0);
        for table in [
            "lotofoot_match",
            "lotofoot_split",
            "lotofoot_match_revision",
        ] {
            let rows: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
        // Le même nombre de rencontres, pour un LF7, est bien accepté : le refus tient à la formule, non au nombre lu.
        let accepted = record_open_grid(
            &mut conn,
            &Aliases::default(),
            f,
            &page(133, Some(vec![[50, 30, 20]; 7])),
        )
        .unwrap();
        assert_eq!(accepted.refused, None);
        assert_eq!(count_grids(&conn).unwrap(), 1);
    }

    /// Une grille entièrement annulée (`drawCancelled`) ne s'écrit pas ici, mais le magasin dit si des tickets déclarés
    /// joués existent dessus : c'est ce qu'il faudra signaler (mises remboursées au guichet).
    #[test]
    fn played_tickets_of_a_grid_are_counted() {
        let mut conn = db();
        let f = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        record_open_grid(&mut conn, &Aliases::default(), f, &page(133, None)).unwrap();
        let id = grid_of_2026(Game::Lf7, 133);
        assert_eq!(played_count(&conn, id).unwrap(), 0);

        let ticket = Ticket::parse("1111111").unwrap();
        for stake in [1.0, 2.0] {
            record_play(
                &conn,
                &NewPlay {
                    played_at: at("2026-10-24T11:00:00Z"),
                    grid: id,
                    ticket: &ticket,
                    stake: Euros::new(stake).unwrap(),
                    reco_id: None,
                },
            )
            .unwrap();
        }

        assert_eq!(played_count(&conn, id).unwrap(), 2);
        // Une autre grille n'est pas comptée.
        assert_eq!(
            played_count(&conn, grid_of_2026(Game::Lf8, 133)).unwrap(),
            0
        );
        assert_eq!(played_count(&conn, GridId { year: 2027, ..id }).unwrap(), 0);
    }
}
