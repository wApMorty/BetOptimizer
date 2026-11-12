//! Sauvegarde de la base (`edge backup`) : une copie cohérente, datée, dans un dossier choisi par Paul.
//!
//! L'archive des cotes ne se reconstitue pas (CLAUDE.md : « notre archive sera la seule source ») ; jusqu'à l'audit du
//! 2026-11-12 elle ne vivait que sur le disque du portable (`docs/sources.md` §73). La copie est faite par SQLite
//! lui-même (`VACUUM INTO`) : un instantané cohérent même si un timer écrit au même moment, vérifié avant d'être gardé.
//!
//! Ce module ne sort pas la copie du portable : il écrit dans `backup_dir`. C'est ce dossier qui doit se trouver sur un
//! autre support (disque externe, dossier synchronisé). Il n'est **jamais créé** : s'il manque, le disque n'est sans
//! doute pas monté, et écrire à sa place sur le disque interne donnerait une sauvegarde qui n'en est pas une.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};

use super::{migrate, open_read_only};
use crate::error::{Error, Result};

/// Format de l'instant dans le nom d'une copie : date et heure UTC, à la seconde. Le même que celui des rapports
/// quotidiens : il trie les fichiers dans l'ordre chronologique.
const NAME_FORMAT: &str = "%Y-%m-%d-%H%M%SZ";
/// Début et fin du nom d'une copie : `edge-2026-11-12-133000Z.db`.
const PREFIX: &str = "edge-";
const SUFFIX: &str = ".db";
/// Suffixe d'une copie en cours d'écriture : elle ne prend son nom définitif qu'une fois vérifiée.
const PARTIAL: &str = ".partial";

/// Bilan d'une sauvegarde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// La copie écrite.
    pub path: PathBuf,
    /// Sa taille, en octets.
    pub bytes: u64,
    /// Copies plus anciennes supprimées par la rotation (noms de fichiers).
    pub removed: Vec<String>,
    /// Copies présentes après la rotation, celle-ci comprise.
    pub kept: usize,
}

/// Nom de la copie faite à `at`.
pub fn file_name(at: DateTime<Utc>) -> String {
    format!("{PREFIX}{}{SUFFIX}", at.format(NAME_FORMAT))
}

/// Relit l'instant d'un nom de copie, ou `None` si le nom ne suit pas exactement le format : la rotation ne supprime
/// que des fichiers ainsi reconnus, jamais un autre fichier du dossier.
fn parse_name(name: &str) -> Option<DateTime<Utc>> {
    let stem = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    NaiveDateTime::parse_from_str(stem, NAME_FORMAT)
        .ok()
        .map(|naive| naive.and_utc())
}

/// Copie la base `db` dans `dir` sous un nom daté, la vérifie, puis ne garde que les `keep` copies les plus récentes.
///
/// Étapes : copie par `VACUUM INTO` vers un fichier `.partial` créé en 0600 ; contrôle d'intégrité et de version du
/// schéma sur la copie ; renommage (atomique) vers le nom définitif ; rotation. Une copie qui échoue au contrôle est
/// supprimée et la commande échoue : les copies précédentes ne sont pas touchées.
pub fn run(db: &Path, dir: &Path, keep: usize, now: DateTime<Utc>) -> Result<Backup> {
    if keep == 0 {
        return Err(Error::Data(
            "sauvegarde : il faut garder au moins une copie".to_string(),
        ));
    }
    if !dir.is_dir() {
        return Err(Error::Data(format!(
            "dossier de sauvegarde absent : {} (disque non monté ? il n'est jamais créé par edge)",
            dir.display()
        )));
    }
    let path = dir.join(file_name(now));
    if path.exists() {
        return Err(Error::Data(format!(
            "une sauvegarde porte déjà ce nom : {}",
            path.display()
        )));
    }
    let partial = dir.join(format!("{}{PARTIAL}", file_name(now)));
    create_private(&partial)?;

    // À partir d'ici, tout échec doit retirer le fichier partiel : la fermeture fait le travail, le `match` nettoie.
    let copied = copy_and_check(db, &partial);
    if let Err(error) = copied {
        // Suppression « au mieux » : l'erreur d'origine est celle qui compte, on ne la masque pas par celle du ménage.
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, &path).map_err(Error::io(&path))?;
    let bytes = fs::metadata(&path).map_err(Error::io(&path))?.len();
    tracing::info!(copie = %path.display(), octets = bytes, "sauvegarde écrite et vérifiée");

    let (removed, kept) = rotate(dir, keep)?;
    Ok(Backup {
        path,
        bytes,
        removed,
        kept,
    })
}

/// Crée le fichier vide qui recevra la copie, lisible par son seul propriétaire dès sa création (sous Unix) : la base
/// contient tes paris et tes mises, elle n'a pas à être lisible par un autre compte du poste, même un instant.
fn create_private(path: &Path) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    // `create_new` : échoue si le fichier existe déjà, on n'écrase jamais rien.
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(Error::io(path))?;
    Ok(())
}

/// Copie `db` dans `target` (un fichier vide) et vérifie la copie.
fn copy_and_check(db: &Path, target: &Path) -> Result<()> {
    let wanted = {
        // Connexion en lecture seule : la sauvegarde ne peut rien écrire dans la base, quoi qu'il arrive. En mode WAL,
        // un lecteur n'attend pas un écrivain : `VACUUM INTO` copie l'état validé à l'instant où il commence.
        let source = open_read_only(db)?;
        let Some(target_text) = target.to_str() else {
            return Err(Error::Data(format!(
                "chemin de sauvegarde illisible : {}",
                target.display()
            )));
        };
        tracing::info!(base = %db.display(), "copie de la base en cours (VACUUM INTO)");
        // SQLite accepte une cible qui existe si elle est vide : c'est ce qui permet de la créer en 0600 d'abord.
        source.execute("VACUUM INTO ?1", [target_text])?;
        migrate::schema_version(&source)?
        // `source` est fermée ici, à la fin du bloc : la base n'est plus tenue pendant le contrôle de la copie.
    };

    let copy = open_read_only(target)?;
    let verdict: String = copy.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if verdict != "ok" {
        return Err(Error::Data(format!(
            "la copie échoue au contrôle d'intégrité : {verdict}"
        )));
    }
    let found = migrate::schema_version(&copy)?;
    if found != wanted {
        return Err(Error::Data(format!(
            "la copie a la version de schéma {found}, la base {wanted}"
        )));
    }
    Ok(())
}

/// Ne garde que les `keep` copies les plus récentes de `dir`. Renvoie les noms supprimés et le nombre de copies gardées.
fn rotate(dir: &Path, keep: usize) -> Result<(Vec<String>, usize)> {
    let mut copies: Vec<(DateTime<Utc>, String)> = Vec::new();
    for entry in fs::read_dir(dir).map_err(Error::io(dir))? {
        let entry = entry.map_err(Error::io(dir))?;
        // Un nom non UTF-8 ou hors format n'est pas une copie d'edge : on n'y touche pas.
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if let Some(at) = parse_name(&name) {
            copies.push((at, name));
        }
    }
    // Du plus récent au plus ancien : les `keep` premières restent.
    copies.sort_by_key(|copy| std::cmp::Reverse(copy.0));
    let mut removed = Vec::new();
    for (_, name) in copies.iter().skip(keep) {
        let old = dir.join(name);
        fs::remove_file(&old).map_err(Error::io(&old))?;
        // `clone` : `copies` garde ses noms (on en lit encore la longueur), la liste des suppressions prend les siens.
        removed.push(name.clone());
    }
    Ok((removed, copies.len().min(keep)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Connection, open_migrated};

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    /// Une base migrée avec une ligne dans le journal des alertes, et un dossier de sauvegarde vide.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("edge.db");
        let conn = open_migrated(&db).unwrap();
        conn.execute(
            "INSERT INTO alert_log (sent_at, kind, subject, detail) VALUES ('2026-11-12T12:00:00Z', 'unit_failed', 'edge-x.service', 'd')",
            [],
        )
        .unwrap();
        let dir = root.path().join("sauvegardes");
        fs::create_dir(&dir).unwrap();
        (root, db, dir)
    }

    #[test]
    fn a_backup_is_a_dated_checked_copy_with_the_same_schema_and_rows() {
        let (_root, db, dir) = setup();

        let backup = run(&db, &dir, 7, at("2026-11-12T13:30:00Z")).unwrap();

        assert_eq!(
            backup.path.file_name().unwrap(),
            "edge-2026-11-12-133000Z.db"
        );
        assert!(backup.bytes > 0 && backup.removed.is_empty());
        assert_eq!(backup.kept, 1);
        // La copie s'ouvre seule : même version de schéma, mêmes lignes, déclencheurs d'immuabilité compris.
        let copy = Connection::open(&backup.path).unwrap();
        assert_eq!(
            migrate::schema_version(&copy).unwrap(),
            migrate::LATEST_VERSION
        );
        let rows: i64 = copy
            .query_row("SELECT COUNT(*) FROM alert_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        assert!(copy.execute("DELETE FROM alert_log", []).is_err());
        // Aucun fichier partiel ne reste.
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["edge-2026-11-12-133000Z.db"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_backup_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (_root, db, dir) = setup();

        let backup = run(&db, &dir, 7, at("2026-11-12T13:30:00Z")).unwrap();

        let mode = fs::metadata(&backup.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_backup_taken_while_a_write_is_in_progress_holds_only_what_was_committed() {
        let (_root, db, dir) = setup();
        let mut writer = open_migrated(&db).unwrap();
        let tx = writer.transaction().unwrap();
        tx.execute(
            "INSERT INTO alert_log (sent_at, kind, subject, detail) VALUES ('2026-11-12T13:00:00Z', 'unit_failed', 'edge-y.service', 'd')",
            [],
        )
        .unwrap();

        // La transaction n'est pas validée : la sauvegarde ne l'attend pas et ne la voit pas.
        let backup = run(&db, &dir, 7, at("2026-11-12T13:30:00Z")).unwrap();
        tx.commit().unwrap();

        let copy = Connection::open(&backup.path).unwrap();
        let rows: i64 = copy
            .query_row("SELECT COUNT(*) FROM alert_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn only_the_most_recent_copies_are_kept_and_foreign_files_are_never_touched() {
        let (_root, db, dir) = setup();
        fs::write(dir.join("notes.txt"), "à moi").unwrap();
        fs::write(dir.join("edge-ancien.db"), "pas une copie d'edge").unwrap();
        for day in 8..=11 {
            run(&db, &dir, 3, at(&format!("2026-11-{day:02}T13:30:00Z"))).unwrap();
        }

        let last = run(&db, &dir, 3, at("2026-11-12T13:30:00Z")).unwrap();

        assert_eq!(last.kept, 3);
        assert_eq!(last.removed, ["edge-2026-11-09-133000Z.db"]);
        let mut names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "edge-2026-11-10-133000Z.db",
                "edge-2026-11-11-133000Z.db",
                "edge-2026-11-12-133000Z.db",
                "edge-ancien.db",
                "notes.txt",
            ]
        );
    }

    #[test]
    fn a_missing_folder_is_an_error_and_is_never_created() {
        let (root, db, _dir) = setup();
        let unmounted = root.path().join("disque-externe").join("edge");

        let error = run(&db, &unmounted, 7, at("2026-11-12T13:30:00Z")).unwrap_err();

        assert!(error.to_string().contains("dossier de sauvegarde absent"));
        assert!(!unmounted.exists());
    }

    #[test]
    fn a_missing_database_leaves_no_partial_file_behind() {
        let (root, _db, dir) = setup();
        let absent = root.path().join("absente.db");

        assert!(run(&absent, &dir, 7, at("2026-11-12T13:30:00Z")).is_err());

        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        // Et garder zéro copie est refusé : la rotation supprimerait la sauvegarde qu'elle vient de faire.
        assert!(run(&absent, &dir, 0, at("2026-11-12T13:30:00Z")).is_err());
    }

    #[test]
    fn two_backups_in_the_same_second_do_not_overwrite_each_other() {
        let (_root, db, dir) = setup();
        let now = at("2026-11-12T13:30:00Z");
        run(&db, &dir, 7, now).unwrap();

        let error = run(&db, &dir, 7, now).unwrap_err();

        assert!(error.to_string().contains("porte déjà ce nom"));
    }
}
