//! Import des CSV football-data en base : lecture du cache, résolution des noms, écriture idempotente.
//!
//! Tout se joue en deux temps pour qu'un échec ne laisse rien derrière lui : d'abord on lit et on vérifie
//! **tous** les fichiers (noms d'équipes connus), puis on écrit le tout dans une seule transaction.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use rusqlite::Connection;

use super::fetch::cache_path;
use super::parse::{ParsedFile, parse_csv};
use super::{League, Season};
use crate::error::{Error, Result};
use crate::ingest::aliases::Aliases;
use crate::store::history;

/// Un fichier lu, prêt à être importé.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonFile {
    /// Championnat.
    pub league: League,
    /// Saison.
    pub season: Season,
    /// Contenu lu.
    pub parsed: ParsedFile,
}

/// Bilan d'un import.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportReport {
    /// Fichiers importés.
    pub files: usize,
    /// Matchs lus (nouveaux et mis à jour).
    pub games: usize,
    /// Matchs ajoutés (les autres existaient déjà et ont été mis à jour).
    pub new_games: usize,
    /// Lignes de cotes 1X2 écrites.
    pub odds_rows: usize,
    /// Lignes de cotes plus/moins écrites.
    pub totals_rows: usize,
    /// Lignes ignorées faute de données essentielles.
    pub incomplete_rows: usize,
    /// Triplets de cotes écartés (valeur invalide).
    pub dropped_odds: usize,
    /// Doublons ignorés dans les fichiers.
    pub duplicate_rows: usize,
}

/// Lit les fichiers du cache. Les fichiers absents sont rendus à part (saison en cours non publiée,
/// ou téléchargement pas encore fait) : à l'appelant de décider s'il s'en inquiète.
pub fn load_files(
    dir: &Path,
    wanted: &[(League, Season)],
) -> Result<(Vec<SeasonFile>, Vec<String>)> {
    let mut files = Vec::new();
    let mut missing = Vec::new();
    for (league, season) in wanted {
        let path = cache_path(dir, league, *season);
        match fs::read(&path) {
            Ok(bytes) => files.push(SeasonFile {
                league: *league,
                season: *season,
                parsed: parse_csv(&bytes)
                    .map_err(|e| Error::Data(format!("{} : {e}", path.display())))?,
            }),
            Err(e) if e.kind() == ErrorKind::NotFound => missing.push(path.display().to_string()),
            Err(e) => return Err(Error::io(&path)(e)),
        }
    }
    Ok((files, missing))
}

/// Écrit les fichiers en base. Un nom d'équipe inconnu bloque tout, avant la moindre écriture.
pub fn import_files(
    conn: &mut Connection,
    aliases: &Aliases,
    files: &[SeasonFile],
) -> Result<ImportReport> {
    let mut unknown = BTreeSet::new();
    for file in files {
        for parsed_match in &file.parsed.matches {
            let record = &parsed_match.record;
            for name in [&record.home_team, &record.away_team] {
                if aliases.resolve(file.league.div, name).is_none() {
                    unknown.insert(format!("{}: {name}", file.league.div));
                }
            }
        }
    }
    if !unknown.is_empty() {
        return Err(Error::UnknownTeams {
            names: unknown.into_iter().collect(),
        });
    }

    let tx = conn.transaction()?;
    let games_before = history::count_games(&tx)?;
    let mut team_ids: HashMap<String, i64> = HashMap::new();
    let mut report = ImportReport::default();
    for file in files {
        let season_label = file.season.label();
        report.files += 1;
        report.incomplete_rows += file.parsed.incomplete_rows;
        report.dropped_odds += file.parsed.dropped_odds;
        report.duplicate_rows += file.parsed.duplicate_rows;
        for parsed_match in &file.parsed.matches {
            let record = &parsed_match.record;
            let home_id = canonical_team_id(
                &tx,
                &mut team_ids,
                aliases,
                file.league.div,
                &record.home_team,
            )?;
            let away_id = canonical_team_id(
                &tx,
                &mut team_ids,
                aliases,
                file.league.div,
                &record.away_team,
            )?;
            let game_id = history::upsert_game(
                &tx,
                file.league.div,
                &season_label,
                record,
                home_id,
                away_id,
            )?;
            for odds in &parsed_match.odds {
                history::upsert_odds(&tx, game_id, odds)?;
                report.odds_rows += 1;
            }
            for totals in &parsed_match.totals {
                history::upsert_totals(&tx, game_id, totals)?;
                report.totals_rows += 1;
            }
            report.games += 1;
        }
    }
    report.new_games = (history::count_games(&tx)? - games_before) as usize;
    tx.commit()?;
    Ok(report)
}

/// Identifiant de l'équipe canonique d'un nom publié. `cache` évite de réinterroger la base à chaque match.
fn canonical_team_id(
    conn: &Connection,
    cache: &mut HashMap<String, i64>,
    aliases: &Aliases,
    div: &str,
    published_name: &str,
) -> Result<i64> {
    let canonical = aliases
        .resolve(div, published_name)
        .ok_or_else(|| Error::Data(format!("nom d'équipe non résolu : {div}: {published_name}")))?;
    if let Some(id) = cache.get(canonical) {
        return Ok(*id);
    }
    let id = history::team_id(conn, canonical)?;
    cache.insert(canonical.to_string(), id);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::football_data::LEAGUES;
    use crate::ingest::football_data::parse::parse_csv;
    use crate::store::migrate;

    const E0_2526: &[u8] = include_bytes!("../../../tests/fixtures/fd/E0_2526_sample.csv");
    const E0_2627: &[u8] = include_bytes!("../../../tests/fixtures/fd/E0_2627_sample.csv");
    const E0_1617: &[u8] = include_bytes!("../../../tests/fixtures/fd/E0_1617_sample.csv");

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    fn file(season: u16, bytes: &[u8]) -> SeasonFile {
        SeasonFile {
            league: LEAGUES[0],
            season: Season::new(season),
            parsed: parse_csv(bytes).unwrap(),
        }
    }

    fn files() -> Vec<SeasonFile> {
        vec![
            file(2016, E0_1617),
            file(2025, E0_2526),
            file(2026, E0_2627),
        ]
    }

    fn count(conn: &Connection, table: &str) -> u32 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn the_full_chain_stores_games_odds_and_canonical_teams() {
        let mut conn = database();
        let aliases = Aliases::bundled().unwrap();

        let report = import_files(&mut conn, &aliases, &files()).unwrap();

        assert_eq!(report.files, 3);
        assert_eq!(report.games, 11);
        assert_eq!(report.new_games, 11);
        assert_eq!(count(&conn, "game"), 11);
        assert!(report.odds_rows > 30);
        assert!(
            report.totals_rows > 10,
            "les cotes plus/moins des saisons récentes sont importées"
        );
        // « Man United » (football-data) est enregistré sous son nom canonique, une seule fois.
        let manchester: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM team WHERE name = 'Manchester United'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(manchester, 1);
        let alias_row: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM team WHERE name = 'Man United'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(alias_row, 0);
        // Un match de 2025/26 : buts, tirs, Pinnacle de clôture.
        let (score, shots, pinnacle_close): (String, i64, f64) = conn
            .query_row(
                "SELECT g.ft_home || '-' || g.ft_away, g.shots_home, o.home
                   FROM game g JOIN game_odds o ON o.game_id = g.id AND o.bookmaker = 'PS' AND o.moment = 'close'
                  WHERE g.match_date = '2025-08-15'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((score.as_str(), shots, pinnacle_close), ("4-2", 19, 1.29));
    }

    #[test]
    fn importing_twice_changes_nothing() {
        let mut conn = database();
        let aliases = Aliases::bundled().unwrap();
        import_files(&mut conn, &aliases, &files()).unwrap();
        let (games, odds, teams) = (
            count(&conn, "game"),
            count(&conn, "game_odds"),
            count(&conn, "team"),
        );

        let second = import_files(&mut conn, &aliases, &files()).unwrap();

        assert_eq!(second.new_games, 0);
        assert_eq!(second.games, 11);
        assert_eq!(
            (
                count(&conn, "game"),
                count(&conn, "game_odds"),
                count(&conn, "team")
            ),
            (games, odds, teams)
        );
    }

    #[test]
    fn a_corrected_score_updates_the_same_game_keeping_its_id() {
        let mut conn = database();
        let aliases = Aliases::bundled().unwrap();
        let mut corrected = files();
        import_files(&mut conn, &aliases, &corrected).unwrap();
        let id_before: i64 = conn
            .query_row(
                "SELECT id FROM game WHERE match_date = '2025-08-15'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        corrected[1].parsed.matches[0].record.ft_home = 5;
        import_files(&mut conn, &aliases, &corrected).unwrap();

        let (id_after, goals): (i64, i64) = conn
            .query_row(
                "SELECT id, ft_home FROM game WHERE match_date = '2025-08-15'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((id_after, goals), (id_before, 5));
    }

    #[test]
    fn an_unknown_team_blocks_everything_and_lists_every_unknown_name() {
        let mut conn = database();
        // Table volontairement incomplète : ni Liverpool ni Bournemouth.
        let aliases = Aliases::parse("[E0]\n\"Manchester United\" = [\"Man United\"]\n").unwrap();

        let err = import_files(&mut conn, &aliases, &files()).unwrap_err();

        let Error::UnknownTeams { names } = &err else {
            panic!("erreur inattendue : {err}");
        };
        assert!(names.contains(&"E0: Liverpool".to_string()));
        assert!(names.contains(&"E0: Bournemouth".to_string()));
        assert!(err.to_string().contains("config/aliases.toml"));
        assert_eq!(count(&conn, "game"), 0, "rien ne doit être écrit");
        assert_eq!(count(&conn, "team"), 0);
    }

    #[test]
    fn missing_cache_files_are_reported_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("E0_2526.csv"), E0_2526).unwrap();
        let wanted = [
            (LEAGUES[0], Season::new(2025)),
            (LEAGUES[0], Season::new(2026)),
        ];

        let (files, missing) = load_files(dir.path(), &wanted).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(missing.len(), 1);
        assert!(missing[0].ends_with("E0_2627.csv"));
    }

    #[test]
    fn xg_is_stored_when_published_and_null_otherwise() {
        let mut conn = database();
        import_files(&mut conn, &Aliases::bundled().unwrap(), &files()).unwrap();

        let with_xg: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM game WHERE xg_home IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(with_xg, 3, "seuls les 3 matchs de 2026/27 ont un xG");
    }
}
