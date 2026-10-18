//! Import des tableurs de tennis-data dans la base : idempotent (un fichier rejoué met à jour, ne duplique pas).

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::Tour;
use super::parse::{Rejected, parse_workbook};
use crate::error::{Error, Result};
use crate::store::tennis::upsert_match;

/// Années extrêmes acceptées dans un nom de fichier.
const YEARS: std::ops::RangeInclusive<u16> = 2000..=2100;

/// Un tableur trouvé sur le disque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TennisFile {
    /// Circuit (nom du sous-dossier).
    pub tour: Tour,
    /// Année (nom du fichier).
    pub season: u16,
    /// Chemin.
    pub path: PathBuf,
}

/// Les tableurs d'un dossier racine (`atp/2024.xlsx`, `wta/2024.xlsx`…), triés par circuit puis par année.
///
/// Les fichiers dont le nom n'est pas une année (`notes.txt`, `2024 (1).xlsx`) sont renvoyés à part : on les signale au
/// lieu de les ignorer en silence.
pub fn find_files(root: &Path) -> Result<(Vec<TennisFile>, Vec<PathBuf>)> {
    let mut files = Vec::new();
    let mut ignored = Vec::new();
    for tour in Tour::ALL {
        let dir = root.join(tour.as_str());
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::io(&dir)(e)),
        };
        for entry in entries {
            let path = entry.map_err(Error::io(&dir))?.path();
            let year = match path.extension().and_then(|e| e.to_str()) {
                Some("xlsx" | "xls") => path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse::<u16>().ok())
                    .filter(|y| YEARS.contains(y)),
                _ => None,
            };
            match year {
                Some(season) => files.push(TennisFile { tour, season, path }),
                None => ignored.push(path),
            }
        }
    }
    files.sort_by_key(|f| (f.tour, f.season));
    ignored.sort();
    Ok((files, ignored))
}

/// Ce qu'un fichier a donné.
#[derive(Debug, Clone, PartialEq)]
pub struct FileReport {
    /// Circuit.
    pub tour: Tour,
    /// Année.
    pub season: u16,
    /// Matchs écrits ou mis à jour.
    pub matches: usize,
    /// Lignes rejetées, avec leur raison.
    pub rejected: Vec<Rejected>,
    /// Paires de cotes écartées.
    pub invalid_odds: usize,
}

/// Bilan d'un import.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportReport {
    /// Un compte rendu par fichier.
    pub files: Vec<FileReport>,
}

impl ImportReport {
    /// Matchs écrits, tous fichiers confondus.
    pub fn total_matches(&self) -> usize {
        self.files.iter().map(|f| f.matches).sum()
    }

    /// Lignes rejetées, tous fichiers confondus.
    pub fn total_rejected(&self) -> usize {
        self.files.iter().map(|f| f.rejected.len()).sum()
    }
}

/// Importe les fichiers donnés, chacun dans sa transaction : un fichier illisible n'écrit rien et arrête l'import
/// (les fichiers précédents restent importés, relancer reprend sans doublon).
pub fn import_files(conn: &mut Connection, files: &[TennisFile]) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    for file in files {
        let bytes = fs::read(&file.path).map_err(Error::io(&file.path))?;
        let parsed = parse_workbook(&bytes, file.tour, file.season)
            .map_err(|e| Error::Data(format!("{} : {e}", file.path.display())))?;
        let tx = conn.transaction()?;
        for m in &parsed.matches {
            upsert_match(&tx, m)?;
        }
        tx.commit()?;
        report.files.push(FileReport {
            tour: file.tour,
            season: file.season,
            matches: parsed.matches.len(),
            rejected: parsed.rejected,
            invalid_odds: parsed.invalid_odds,
        });
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;
    use crate::store::tennis::{count_matches, load_matches};

    const MINI: &[u8] = include_bytes!("../../../tests/fixtures/tennis/mini_atp_2024.xlsx");

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    fn folder_with_the_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("atp")).unwrap();
        fs::write(dir.path().join("atp").join("2024.xlsx"), MINI).unwrap();
        fs::write(dir.path().join("atp").join("notes.txt"), "hors sujet").unwrap();
        dir
    }

    #[test]
    fn files_are_found_by_tour_and_year_and_strangers_are_reported() {
        let dir = folder_with_the_fixture();

        let (files, ignored) = find_files(dir.path()).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!((files[0].tour, files[0].season), (Tour::Atp, 2024));
        assert_eq!(ignored.len(), 1);
        assert!(ignored[0].ends_with("notes.txt"));
        // Un dossier absent n'est pas une erreur.
        let (none, _) = find_files(&dir.path().join("absent")).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn importing_twice_writes_the_matches_once() {
        let dir = folder_with_the_fixture();
        let (files, _) = find_files(dir.path()).unwrap();
        let mut conn = database();

        let first = import_files(&mut conn, &files).unwrap();
        let second = import_files(&mut conn, &files).unwrap();

        assert_eq!(first.total_matches(), 6);
        assert_eq!(first.total_rejected(), 2);
        assert_eq!(second.total_matches(), 6);
        assert_eq!(count_matches(&conn).unwrap(), 6);
    }

    #[test]
    fn stored_matches_come_back_in_date_order_with_their_odds() {
        let dir = folder_with_the_fixture();
        let (files, _) = find_files(dir.path()).unwrap();
        let mut conn = database();
        import_files(&mut conn, &files).unwrap();

        let matches = load_matches(&conn).unwrap();

        assert_eq!(matches.len(), 6);
        assert!(matches.windows(2).all(|w| w[0].date <= w[1].date));
        let first = &matches[0];
        assert_eq!(first.winner, "Alpha A.");
        assert_eq!(first.odds["PS"], (1.55, 2.60));
        assert_eq!(first.odds.len(), 4);
    }

    #[test]
    fn a_corrupt_file_stops_the_import_with_its_name() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("wta")).unwrap();
        fs::write(dir.path().join("wta").join("2024.xlsx"), b"pas un tableur").unwrap();
        let (files, _) = find_files(dir.path()).unwrap();
        let mut conn = database();

        let error = import_files(&mut conn, &files).unwrap_err().to_string();

        assert!(error.contains("2024.xlsx"));
        assert_eq!(count_matches(&conn).unwrap(), 0);
    }
}
