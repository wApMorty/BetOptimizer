//! Accès à la base SQLite : ouverture et migrations de schéma.

pub mod migrate;

use std::path::Path;

use rusqlite::Connection;

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
    fn open_in_a_missing_directory_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent").join("test.db");

        let err = open(&path).unwrap_err();

        assert!(err.to_string().contains("absent"));
    }
}
