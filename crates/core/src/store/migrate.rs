//! Migrations de schéma maison.
//!
//! Le numéro de version est stocké dans la base elle-même (`PRAGMA user_version`) et vaut le nombre
//! de migrations déjà appliquées. Le SQL de chaque migration est embarqué dans le binaire.

use rusqlite::Connection;

use crate::error::{Error, Result};

/// Migrations dans l'ordre : la première est la version 1, la deuxième la version 2, etc.
/// Une migration déjà publiée ne se modifie jamais : on en ajoute une nouvelle à la suite.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_api_call.sql"),
    include_str!("migrations/0002_odds_archive.sql"),
    include_str!("migrations/0003_history.sql"),
    include_str!("migrations/0004_predictions.sql"),
    include_str!("migrations/0005_totals_odds.sql"),
    include_str!("migrations/0006_backtest.sql"),
    include_str!("migrations/0007_recommendations.sql"),
    include_str!("migrations/0008_tennis.sql"),
    include_str!("migrations/0009_nba.sql"),
    include_str!("migrations/0010_nba_tuning.sql"),
    include_str!("migrations/0011_selection_criterion.sql"),
    include_str!("migrations/0012_selection_kelly.sql"),
    include_str!("migrations/0013_bonus.sql"),
    include_str!("migrations/0014_oddspapi.sql"),
    include_str!("migrations/0015_lotofoot.sql"),
    include_str!("migrations/0016_loto.sql"),
    include_str!("migrations/0017_lotofoot_scores.sql"),
    include_str!("migrations/0018_lotofoot_alert.sql"),
    include_str!("migrations/0019_lotofoot_budget.sql"),
    include_str!("migrations/0020_lotofoot_match_revision.sql"),
    include_str!("migrations/0021_lotofoot_oddspapi.sql"),
    include_str!("migrations/0022_lotofoot_year.sql"),
    include_str!("migrations/0023_alert_log.sql"),
    include_str!("migrations/0024_lotofoot_sources.sql"),
    include_str!("migrations/0025_alert_kinds.sql"),
    include_str!("migrations/0026_team_alias.sql"),
];

/// Version du schéma attendue par ce programme (le nombre de migrations connues).
pub const LATEST_VERSION: u32 = MIGRATIONS.len() as u32;

/// Version actuelle du schéma de la base (0 pour une base vierge).
pub fn schema_version(conn: &Connection) -> Result<u32> {
    let version = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version)
}

/// Applique les migrations manquantes et renvoie la version atteinte.
///
/// `&mut Connection` : ouvrir une transaction exige un accès exclusif à la connexion ; le compilateur
/// interdit ainsi d'utiliser la même connexion ailleurs pendant qu'une migration est en cours.
pub fn migrate(conn: &mut Connection) -> Result<u32> {
    migrate_with(conn, MIGRATIONS)
}

/// Cœur de [`migrate`], avec la liste de migrations en paramètre pour pouvoir tester un échec.
fn migrate_with(conn: &mut Connection, migrations: &[&str]) -> Result<u32> {
    let known = u32::try_from(migrations.len()).expect("le nombre de migrations tient dans un u32");
    let applied = schema_version(conn)?;
    if applied > known {
        // Base créée par une version plus récente du programme : on ne la touche pas.
        return Err(Error::DatabaseTooNew {
            found: applied,
            supported: known,
        });
    }

    // `(1..=known).zip(migrations)` associe à chaque migration son numéro de version ;
    // `skip` saute celles que la base a déjà.
    for (version, sql) in (1..=known).zip(migrations).skip(applied as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // PRAGMA n'accepte pas de paramètre lié ; `version` est un entier que nous contrôlons.
        tx.execute_batch(&format!("PRAGMA user_version = {version}"))?;
        // Sans `commit()`, `tx` est détruite à la fin de l'itération (ou à la sortie par `?`) et
        // SQLite annule tout : une migration réussit en entier ou pas du tout.
        tx.commit()?;
        tracing::info!(version, "migration appliquée");
    }
    schema_version(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get::<_, u32>(0),
        )
        .unwrap()
            == 1
    }

    #[test]
    fn fresh_database_receives_every_migration() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 0);

        let version = migrate(&mut conn).unwrap();

        assert_eq!(version, LATEST_VERSION);
        assert!(table_exists(&conn, "api_call"));
    }

    /// La migration 22 reconstruit les tables du Loto Foot : rien ne doit s'y perdre, et elles restent immuables.
    #[test]
    fn the_lotofoot_year_migration_keeps_every_row() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate_with(&mut conn, &MIGRATIONS[..21]).unwrap();
        conn.execute_batch(
            "INSERT INTO lotofoot_fetch (id, fetched_at, page, http_status) VALUES (7, '2026-10-24T09:00:00Z', 'lf7', 200);
             INSERT INTO lotofoot_grid (game, number, closes_at, jackpot, fetch_id)
                 VALUES ('lf7', 133, '2026-10-25T16:10:00Z', 100000.0, 7);
             INSERT INTO lotofoot_match (game, number, position, home_raw, away_raw) VALUES ('lf7', 133, 1, 'A', 'B');
             INSERT INTO lotofoot_match_revision (game, number, fetch_id, position, home_raw, away_raw)
                 VALUES ('lf7', 133, 7, 1, 'C', 'D');
             INSERT INTO lotofoot_split (fetch_id, game, number, position, q_home, q_draw, q_away)
                 VALUES (7, 'lf7', 133, 1, 50, 30, 20);
             INSERT INTO lotofoot_result (game, number, position, outcome, fetch_id) VALUES ('lf7', 133, 1, 'H', 7);
             INSERT INTO lotofoot_rank (game, number, rank, winners, unit_gain, fetch_id) VALUES ('lf7', 125, 2, 3160, 26.5, 7);
             INSERT INTO lotofoot_session (id, created_at, model_version, params, notes)
                 VALUES (3, '2026-10-24T10:00:00Z', 'lotofoot-1', '{}', '{}');
             INSERT INTO lotofoot_reco (id, session_id, game, number, ticket, position, ev_base, ev_pessimistic,
                                        paid_probability, stakes_estimate, split_fetch_id, inputs)
                 VALUES (11, 3, 'lf7', 133, '1111111', 1, 2.0, 1.8, 0.1, 200000.0, 7, '{}');
             INSERT INTO lotofoot_play (id, played_at, game, number, ticket, stake, reco_id, off_model)
                 VALUES (13, '2026-10-24T11:00:00Z', 'lf7', 133, '1111111', 1.0, 11, 0);
             INSERT INTO lotofoot_alert (game, number, sent_at) VALUES ('lf7', 133, '2026-10-24T09:10:00Z');
             INSERT INTO lotofoot_live (game, number, position, league, event_id, home_goals, away_goals, phase, label,
                                        fetched_at)
                 VALUES ('lf7', 133, 1, 'fra.1', '401', 1, 0, 'playing', '57''', '2026-10-25T17:00:00Z');",
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        for table in [
            "grid",
            "match",
            "match_revision",
            "split",
            "result",
            "rank",
            "reco",
            "play",
            "alert",
            "live",
        ] {
            let rows: (u32, u32) = conn
                .query_row(
                    &format!("SELECT COUNT(*), MIN(year) FROM lotofoot_{table}"),
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(rows, (1, 2026), "{table}");
        }
        let (reco, reco_of_play): (i64, i64) = conn
            .query_row(
                "SELECT r.id, p.reco_id FROM lotofoot_play p JOIN lotofoot_reco r ON r.id = p.reco_id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((reco, reco_of_play), (11, 11));
        for name in ["lotofoot_grid", "lotofoot_reco", "lotofoot_match_revision"] {
            assert!(conn.execute(&format!("DELETE FROM {name}"), []).is_err());
            assert!(
                conn.execute(&format!("UPDATE {name} SET year = 1"), [])
                    .is_err()
            );
        }
        let leftovers: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE instr(sql, '_old') > 0 OR instr(name, '_old') > 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);
        // Un même numéro peut désormais revenir l'année suivante.
        conn.execute(
            "INSERT INTO lotofoot_grid (game, year, number, closes_at, fetch_id)
             VALUES ('lf7', 2027, 133, '2027-10-25T16:10:00Z', 7)",
            [],
        )
        .unwrap();
    }

    /// Données d'un LF7 n°133 à la version 23 du schéma, chaque table du Loto Foot ayant au moins une ligne, dont les six
    /// tables qui référencent `lotofoot_fetch`. Deux lectures, pour que les identifiants conservés ne soient pas triviaux.
    const VERSION_23_ROWS: &str = "
        INSERT INTO lotofoot_fetch (id, fetched_at, page, http_status, error) VALUES
            (7, '2026-10-24T09:00:00Z', 'lf7', 200, NULL),
            (9, '2026-10-26T09:00:00Z', 'results', 200, 'page illisible');
        INSERT INTO lotofoot_grid (game, year, number, closes_at, jackpot, fetch_id)
            VALUES ('lf7', 2026, 133, '2026-10-25T16:10:00Z', 100000.0, 7);
        INSERT INTO lotofoot_match (game, year, number, position, home_raw, away_raw) VALUES ('lf7', 2026, 133, 1, 'A', 'B');
        INSERT INTO lotofoot_match_revision (game, year, number, fetch_id, position, home_raw, away_raw)
            VALUES ('lf7', 2026, 133, 7, 1, 'C', 'D');
        INSERT INTO lotofoot_split (fetch_id, game, year, number, position, q_home, q_draw, q_away)
            VALUES (7, 'lf7', 2026, 133, 1, 50, 30, 20);
        INSERT INTO lotofoot_result (game, year, number, position, outcome, fetch_id) VALUES ('lf7', 2026, 133, 1, 'H', 9);
        INSERT INTO lotofoot_rank (game, year, number, rank, winners, unit_gain, fetch_id)
            VALUES ('lf7', 2026, 133, 2, 3160, 26.5, 9);
        INSERT INTO lotofoot_session (id, created_at, model_version, params, notes)
            VALUES (3, '2026-10-24T10:00:00Z', 'lotofoot-1', '{}', '{}');
        INSERT INTO lotofoot_reco (id, session_id, game, year, number, ticket, position, ev_base, ev_pessimistic,
                                   paid_probability, stakes_estimate, split_fetch_id, inputs)
            VALUES (11, 3, 'lf7', 2026, 133, '1111111', 1, 2.0, 1.8, 0.1, 200000.0, 7, '{}');";

    fn count(conn: &Connection, table: &str) -> u32 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    /// Lignes du verdict d'un `PRAGMA` qui n'en rend aucune quand tout va bien (`foreign_key_check`).
    fn pragma_rows(conn: &Connection, pragma: &str) -> usize {
        conn.prepare(&format!("PRAGMA {pragma}"))
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count()
    }

    /// La migration 24 reconstruit `lotofoot_fetch`, que six tables référencent : rien ne se perd, les clés étrangères
    /// retrouvent leur parent, et les déclencheurs d'immuabilité sont recréés. Clés étrangères actives, comme en production.
    #[test]
    fn the_lotofoot_sources_migration_keeps_rows_and_references() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate_with(&mut conn, &MIGRATIONS[..23]).unwrap();
        conn.execute_batch(VERSION_23_ROWS).unwrap();
        let tables = [
            "fetch",
            "grid",
            "match",
            "match_revision",
            "split",
            "result",
            "rank",
            "reco",
        ];
        let before: Vec<u32> = tables
            .iter()
            .map(|t| count(&conn, &format!("lotofoot_{t}")))
            .collect();
        assert_eq!(before, [2, 1, 1, 1, 1, 1, 1, 1]);

        assert_eq!(migrate(&mut conn).unwrap(), LATEST_VERSION);

        let after: Vec<u32> = tables
            .iter()
            .map(|t| count(&conn, &format!("lotofoot_{t}")))
            .collect();
        assert_eq!(after, before);
        assert_eq!(pragma_rows(&conn, "foreign_key_check"), 0);
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        // Identifiants, pages et erreurs repris tels quels ; toutes les lectures déjà journalisées viennent de prono-match.
        let mut stmt = conn
            .prepare("SELECT id, source, page, http_status, error FROM lotofoot_fetch ORDER BY id")
            .unwrap();
        let fetches: Vec<(i64, String, String, i64, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            fetches,
            [
                (7, "pronomatch".to_string(), "lf7".to_string(), 200, None),
                (
                    9,
                    "pronomatch".to_string(),
                    "results".to_string(),
                    200,
                    Some("page illisible".to_string())
                ),
            ]
        );
        // Les six tables filles pointent toujours `lotofoot_fetch` : leur clé ne suit pas une table supprimée.
        for child in ["grid", "match_revision", "split", "result", "rank", "reco"] {
            let parents: Vec<String> = conn
                .prepare(&format!("PRAGMA foreign_key_list(lotofoot_{child})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>("table"))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert!(parents.iter().any(|p| p == "lotofoot_fetch"), "{child}");
        }
        // Une clé étrangère reste appliquée : un rapport ne peut citer une lecture qui n'existe pas.
        assert!(
            conn.execute(
                "INSERT INTO lotofoot_rank (game, year, number, rank, winners, unit_gain, fetch_id)
                 VALUES ('lf7', 2026, 133, 1, 1, 100.0, 999)",
                [],
            )
            .is_err()
        );
        let leftovers: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE instr(sql, '_old') > 0 OR instr(name, '_old') > 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);
    }

    /// Mêmes lignes, clés étrangères inactives (le réglage par défaut d'une connexion qui ne les active pas) : la
    /// migration passe aussi, et laisse le même contenu.
    #[test]
    fn the_lotofoot_sources_migration_also_runs_without_foreign_keys() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        migrate_with(&mut conn, &MIGRATIONS[..23]).unwrap();
        conn.execute_batch(VERSION_23_ROWS).unwrap();

        migrate(&mut conn).unwrap();

        assert_eq!(count(&conn, "lotofoot_fetch"), 2);
        assert_eq!(pragma_rows(&conn, "foreign_key_check"), 0);
    }

    /// Après la migration 24, une rencontre annulée s'écrit (`C`), un signe inconnu est refusé, la source et la page sont
    /// contrôlées, et les deux tables restent immuables.
    #[test]
    fn the_new_schema_accepts_canceled_matches_and_new_pages_and_stays_immutable() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate(&mut conn).unwrap();
        let insert_fetch = |source: &str, page: &str| {
            conn.execute(
                "INSERT INTO lotofoot_fetch (fetched_at, source, page, http_status)
                 VALUES ('2026-11-15T09:00:00Z', ?1, ?2, 200)",
                [source, page],
            )
        };
        for (source, page) in [
            ("pronomatch", "lf8"),
            ("fdj", "open"),
            ("fdj", "results"),
            ("fdj", "detail"),
            ("resultats_fdj", "open"),
        ] {
            insert_fetch(source, page).unwrap();
        }
        assert!(insert_fetch("inconnue", "open").is_err());
        assert!(insert_fetch("fdj", "inconnue").is_err());
        // La colonne `source` n'a pas de valeur par défaut : une écriture qui l'oublie est refusée.
        assert!(
            conn.execute(
                "INSERT INTO lotofoot_fetch (fetched_at, page, http_status) VALUES ('2026-11-15T09:00:00Z', 'open', 200)",
                [],
            )
            .is_err()
        );

        let insert_result = |position: i64, outcome: &str| {
            conn.execute(
                "INSERT INTO lotofoot_result (game, year, number, position, outcome, fetch_id)
                 VALUES ('lf7', 2026, 140, ?1, ?2, 1)",
                rusqlite::params![position, outcome],
            )
        };
        for (position, outcome) in [(1, "H"), (2, "D"), (3, "A"), (4, "C")] {
            insert_result(position, outcome).unwrap();
        }
        assert!(insert_result(5, "X").is_err());
        assert!(insert_result(6, "N").is_err());

        for table in ["lotofoot_fetch", "lotofoot_result"] {
            assert!(conn.execute(&format!("DELETE FROM {table}"), []).is_err());
        }
        assert!(
            conn.execute("UPDATE lotofoot_fetch SET http_status = 500", [])
                .is_err()
        );
        assert!(
            conn.execute("UPDATE lotofoot_result SET outcome = 'H'", [])
                .is_err()
        );
    }

    /// La migration 25 reconstruit `alert_log` pour y ajouter trois natures d'alerte : les lignes existantes gardent leurs
    /// identifiants, les nouvelles natures s'écrivent, une inconnue est refusée, et le journal reste immuable.
    #[test]
    fn the_alert_kinds_migration_keeps_rows_and_stays_immutable() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate_with(&mut conn, &MIGRATIONS[..24]).unwrap();
        conn.execute_batch(
            "INSERT INTO alert_log (id, sent_at, kind, subject, detail)
                 VALUES (4, '2026-11-12T12:00:00Z', 'unit_failed', 'edge-snapshot.service', 'échec');
             INSERT INTO alert_log (id, sent_at, kind, subject, detail)
                 VALUES (9, '2026-11-12T13:00:00Z', 'stale_source', 'lf8-2026-154', 'figée');",
        )
        .unwrap();
        // Avant la migration, une nature de la phase 22 est refusée.
        assert!(
            conn.execute(
                "INSERT INTO alert_log (sent_at, kind, subject, detail) VALUES ('x', 'source_down', 'open', 'd')",
                [],
            )
            .is_err()
        );

        assert_eq!(migrate(&mut conn).unwrap(), LATEST_VERSION);

        let rows: Vec<(i64, String, String)> = conn
            .prepare("SELECT id, kind, subject FROM alert_log ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                (
                    4,
                    "unit_failed".to_string(),
                    "edge-snapshot.service".to_string()
                ),
                (9, "stale_source".to_string(), "lf8-2026-154".to_string()),
            ]
        );
        let insert = |kind: &str| {
            conn.execute(
                "INSERT INTO alert_log (sent_at, kind, subject, detail) VALUES ('2026-11-15T09:00:00Z', ?1, 'x', 'd')",
                [kind],
            )
        };
        for kind in ["source_fallback", "source_down", "canceled_grid"] {
            insert(kind).unwrap();
        }
        assert!(insert("autre").is_err());
        assert!(
            conn.execute("UPDATE alert_log SET detail = ''", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM alert_log", []).is_err());
        // L'index de recherche par sujet est recréé : il ne reste qu'un seul index de ce nom.
        let indexes: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'alert_log_by_subject'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexes, 1);
    }

    #[test]
    fn second_run_changes_nothing() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO api_call (called_at, endpoint, params, http_status)
             VALUES ('2026-10-18T12:00:00Z', 'sports', '', 200)",
            [],
        )
        .unwrap();

        let version = migrate(&mut conn).unwrap();

        assert_eq!(version, LATEST_VERSION);
        let rows: u32 = conn
            .query_row("SELECT COUNT(*) FROM api_call", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1, "les données existantes doivent survivre");
    }

    #[test]
    fn failing_migration_is_rolled_back_and_earlier_ones_are_kept() {
        let mut conn = Connection::open_in_memory().unwrap();
        let migrations = [
            "CREATE TABLE a (x);",
            "CREATE TABLE b (x); CECI N'EST PAS DU SQL;",
        ];

        assert!(migrate_with(&mut conn, &migrations).is_err());

        assert_eq!(schema_version(&conn).unwrap(), 1);
        assert!(table_exists(&conn, "a"));
        assert!(
            !table_exists(&conn, "b"),
            "la table b de la migration fautive doit être annulée"
        );
    }

    #[test]
    fn database_newer_than_the_program_is_refused_untouched() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 99").unwrap();

        let err = migrate(&mut conn).unwrap_err();

        assert!(matches!(
            err,
            Error::DatabaseTooNew {
                found: 99,
                supported: LATEST_VERSION
            }
        ));
        assert_eq!(schema_version(&conn).unwrap(), 99);
        assert!(!table_exists(&conn, "api_call"));
    }

    #[test]
    fn api_call_rejects_an_unknown_endpoint() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();

        let result = conn.execute(
            "INSERT INTO api_call (called_at, endpoint, params, http_status)
             VALUES ('2026-10-18T12:00:00Z', 'inconnu', '', 200)",
            [],
        );

        assert!(result.is_err());
    }
}
