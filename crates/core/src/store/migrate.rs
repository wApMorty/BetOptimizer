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
