//! Initialisation de l'installation : dossiers, configuration par défaut, base à jour.
//!
//! Cette opération est idempotente : la relancer ne détruit rien (ni la configuration de
//! l'utilisateur, ni les données déjà archivées).

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

use crate::config::EXAMPLE_TOML;
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::store::{self, migrate};

/// Ce que [`initialize`] a fait, pour que l'appelant l'affiche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    /// `true` si le fichier de configuration vient d'être créé, `false` s'il existait déjà.
    pub config_created: bool,
    /// Version du schéma de la base après les migrations.
    pub schema_version: u32,
}

/// Crée les dossiers, dépose la configuration par défaut si elle est absente, puis ouvre la base
/// et applique les migrations.
pub fn initialize(paths: &Paths) -> Result<InitReport> {
    for file in [&paths.config_file, &paths.db_file] {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(Error::io(dir))?;
        }
    }
    let config_created = write_default_config(&paths.config_file)?;
    let mut conn = store::open(&paths.db_file)?;
    let schema_version = migrate::migrate(&mut conn)?;
    Ok(InitReport {
        config_created,
        schema_version,
    })
}

/// Écrit la configuration d'exemple en 0600, sauf si le fichier existe déjà. Renvoie `true` si créé.
fn write_default_config(path: &Path) -> Result<bool> {
    let mut options = fs::OpenOptions::new();
    // `create_new` : la création échoue si le fichier existe. Le test et la création forment une seule
    // opération atomique, et on n'écrase donc jamais la configuration (ni la clé) de l'utilisateur.
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Permissions fixées dès la création : le fichier n'est jamais, même un instant, lisible par d'autres.
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(EXAMPLE_TOML.as_bytes())
                .map_err(Error::io(path))?;
            Ok(true)
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(Error::io(path)(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn creates_directories_config_and_schema() {
        let root = tempfile::tempdir().unwrap();
        // Dossier qui n'existe pas encore : `initialize` doit le créer.
        let paths = Paths::under(&root.path().join("a").join("b"));

        let report = initialize(&paths).unwrap();

        assert!(report.config_created);
        assert_eq!(report.schema_version, 1);
        assert_eq!(
            fs::read_to_string(&paths.config_file).unwrap(),
            EXAMPLE_TOML
        );
        assert!(paths.db_file.exists());
        // La configuration déposée est valide et chargeable.
        assert_eq!(Config::load(&paths.config_file).unwrap(), Config::default());
    }

    #[cfg(unix)]
    #[test]
    fn config_is_created_with_mode_600() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let paths = Paths::under(root.path());

        initialize(&paths).unwrap();

        let mode = fs::metadata(&paths.config_file)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn running_twice_keeps_the_users_config() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::under(root.path());
        initialize(&paths).unwrap();
        fs::write(&paths.config_file, "monthly_credit_cap = 300").unwrap();

        let report = initialize(&paths).unwrap();

        assert!(!report.config_created);
        assert_eq!(report.schema_version, 1);
        assert_eq!(
            fs::read_to_string(&paths.config_file).unwrap(),
            "monthly_credit_cap = 300"
        );
    }
}
