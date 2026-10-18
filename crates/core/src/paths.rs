//! Emplacements des fichiers de l'application.
//!
//! Aucun chemin n'est écrit en dur : sous Linux, `directories` applique les conventions XDG
//! (`~/.config/edge`, `~/.local/share/edge`) ; sous Windows, celles d'`AppData`.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::error::{Error, Result};

/// Chemins des deux fichiers que l'application possède.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Fichier de configuration (peut contenir la clé API : permissions 0600 exigées).
    pub config_file: PathBuf,
    /// Base SQLite : archive des cotes, journal des appels.
    pub db_file: PathBuf,
}

impl Paths {
    /// Emplacements standard de l'utilisateur courant.
    pub fn discover() -> Result<Paths> {
        // `ok_or` convertit l'`Option` de `directories` en `Result` ; le `?` propage l'erreur.
        let dirs = ProjectDirs::from("", "", "edge").ok_or(Error::NoHomeDir)?;
        Ok(Paths {
            config_file: dirs.config_dir().join("edge.toml"),
            db_file: dirs.data_dir().join("edge.db"),
        })
    }

    /// Place les deux fichiers directement dans `root` (option `--home`, tests).
    pub fn under(root: &Path) -> Paths {
        Paths {
            config_file: root.join("edge.toml"),
            db_file: root.join("edge.db"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_places_both_files_in_the_root() {
        let paths = Paths::under(Path::new("/tmp/racine"));
        assert_eq!(paths.config_file, Path::new("/tmp/racine/edge.toml"));
        assert_eq!(paths.db_file, Path::new("/tmp/racine/edge.db"));
    }

    #[test]
    fn discover_uses_the_standard_file_names() {
        let paths = Paths::discover().unwrap();
        assert_eq!(paths.config_file.file_name().unwrap(), "edge.toml");
        assert_eq!(paths.db_file.file_name().unwrap(), "edge.db");
        // Config et données sont rangées dans des dossiers distincts (ou au moins pas confondues).
        assert_ne!(paths.config_file, paths.db_file);
    }
}
