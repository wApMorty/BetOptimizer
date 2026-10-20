//! Archive des rapports quotidiens : un fichier HTML par exécution de `edge recommend`, jamais écrasé.
//!
//! Le nom porte l'instant d'exécution en UTC (`2026-10-20-121003Z.html`) : il trie les fichiers dans l'ordre chronologique
//! et suffit à les dater. Seuls les fichiers qui suivent exactement ce format sont listés, donc publiés par le site.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};

use crate::error::{Error, Result};

/// Format du nom de fichier (sans l'extension) : date et heure UTC, à la seconde.
const NAME_FORMAT: &str = "%Y-%m-%d-%H%M%SZ";

/// Un rapport archivé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyReport {
    /// Nom du fichier (`2026-10-20-121003Z.html`).
    pub name: String,
    /// Instant de l'exécution qui l'a produit.
    pub at: DateTime<Utc>,
}

/// Nom du fichier d'une exécution.
pub fn file_name(at: DateTime<Utc>) -> String {
    format!("{}.html", at.format(NAME_FORMAT))
}

/// Relit l'instant d'un nom de fichier, ou `None` si le nom ne suit pas le format de l'archive.
fn parse_name(name: &str) -> Option<DateTime<Utc>> {
    let stem = name.strip_suffix(".html")?;
    NaiveDateTime::parse_from_str(stem, NAME_FORMAT)
        .ok()
        .map(|naive| naive.and_utc())
}

/// Écrit le rapport d'une exécution. Refuse d'écraser un fichier existant : l'archive est immuable.
pub fn save(dir: &Path, at: DateTime<Utc>, html: &str) -> Result<PathBuf> {
    fs::create_dir_all(dir).map_err(Error::io(dir))?;
    let path = dir.join(file_name(at));
    // `create_new` : l'ouverture échoue si le fichier existe déjà, de façon atomique (pas de « vérifier puis écrire »).
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(Error::io(&path))?;
    file.write_all(html.as_bytes()).map_err(Error::io(&path))?;
    Ok(path)
}

/// Les rapports archivés, du plus récent au plus ancien. Un dossier absent donne une liste vide.
pub fn list(dir: &Path) -> Result<Vec<DailyReport>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(dir)(e)),
    };
    let mut reports = Vec::new();
    for entry in entries {
        let entry = entry.map_err(Error::io(dir))?;
        // Un nom non UTF-8 ne peut pas suivre le format : il est ignoré, comme tout fichier étranger à l'archive.
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if let Some(at) = parse_name(&name) {
            reports.push(DailyReport { name, at });
        }
    }
    // `Reverse` inverse l'ordre de la clé : du plus récent au plus ancien.
    reports.sort_by_key(|report| std::cmp::Reverse(report.at));
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    #[test]
    fn a_report_is_saved_under_its_utc_instant_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let daily = dir.path().join("daily");

        let path = save(&daily, at("2026-10-20T12:10:03Z"), "<p>premier</p>").unwrap();

        assert_eq!(path.file_name().unwrap(), "2026-10-20-121003Z.html");
        assert!(save(&daily, at("2026-10-20T12:10:03Z"), "<p>second</p>").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "<p>premier</p>");
    }

    #[test]
    fn the_list_is_newest_first_and_ignores_foreign_files() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), at("2026-10-19T12:10:00Z"), "a").unwrap();
        save(dir.path(), at("2026-10-20T12:10:00Z"), "b").unwrap();
        fs::write(dir.path().join("notes.html"), "x").unwrap();
        fs::write(dir.path().join("2026-10-21-121000Z.txt"), "x").unwrap();

        let reports = list(dir.path()).unwrap();

        let names: Vec<&str> = reports.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["2026-10-20-121000Z.html", "2026-10-19-121000Z.html"]
        );
        assert_eq!(reports[0].at, at("2026-10-20T12:10:00Z"));
        assert!(list(&dir.path().join("absent")).unwrap().is_empty());
    }
}
