//! Accès à la base SQLite : ouverture et migrations de schéma.

pub mod archive;
pub mod backtests;
pub mod bonus;
pub mod history;
pub mod journal;
pub mod loto;
pub mod lotofoot;
pub mod lotofoot_live;
pub mod migrate;
pub mod nba;
pub mod oddspapi;
pub mod predictions;
pub mod tennis;

use std::path::Path;
use std::time::Duration;

use rusqlite::OpenFlags;

/// Connexion SQLite, ré-exportée : la CLI passe par l'API de `core` sans dépendre de `rusqlite`.
pub use rusqlite::Connection;

use crate::error::{Error, Result};

/// Ouvre (et crée au besoin) la base SQLite, avec les réglages de connexion d'edge.
///
/// Le schéma n'est pas touché : appeler ensuite [`migrate::migrate`].
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).map_err(|source| Error::OpenDatabase {
        path: path.to_path_buf(),
        source,
    })?;
    // WAL : le relevé planifié et une commande manuelle peuvent se croiser sans se bloquer.
    // foreign_keys : SQLite ne vérifie les clés étrangères que si on le demande, connexion par connexion.
    // busy_timeout : attendre jusqu'à 5 s un verrou plutôt que d'échouer aussitôt.
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;",
    )?;
    Ok(conn)
}

/// Ouvre la base et applique les migrations manquantes : le point d'entrée de la CLI et de l'application.
pub fn open_migrated(path: &Path) -> Result<Connection> {
    let mut conn = open(path)?;
    migrate::migrate(&mut conn)?;
    Ok(conn)
}

/// Ouvre une base existante **en lecture seule** : pour `edge serve`, qui ne doit rien pouvoir écrire.
///
/// Ni création, ni migration : une base absente est une erreur. Une écriture tentée par erreur serait refusée par SQLite.
pub fn open_read_only(path: &Path) -> Result<Connection> {
    let conn =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|source| {
            Error::OpenDatabase {
                path: path.to_path_buf(),
                source,
            }
        })?;
    // Le relevé planifié peut écrire au même moment : attendre son verrou plutôt que d'échouer.
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_the_file_and_enables_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");

        let conn = open(&path).unwrap();

        assert!(path.exists());
        let enabled: u32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(enabled, 1);
    }

    #[test]
    fn a_read_only_connection_reads_but_cannot_write_nor_create() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        open_migrated(&path).unwrap();

        let conn = open_read_only(&path).unwrap();

        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert!(version > 0, "la base migrée se lit");
        assert!(conn.execute_batch("CREATE TABLE t (x INTEGER)").is_err());
        assert!(open_read_only(&dir.path().join("absente.db")).is_err());
    }

    #[test]
    fn open_in_a_missing_directory_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent").join("test.db");

        let err = open(&path).unwrap_err();

        assert!(err.to_string().contains("absent"));
    }
}
