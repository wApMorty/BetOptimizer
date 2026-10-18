//! Erreurs de la bibliothèque.
//!
//! Un seul type, [`Error`], pour tout `core`. Chaque variante porte assez de contexte pour que le
//! message affiché dise quoi corriger. Le binaire les enveloppe dans `anyhow` (voir `edge-cli`).

use std::path::{Path, PathBuf};

/// Erreur de la bibliothèque `core`.
///
/// `thiserror` génère l'implémentation de `Display` à partir des attributs `#[error("...")]`
/// et celle de `std::error::Error` (dont `source()`).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Lecture ou écriture impossible sur un fichier ou un dossier.
    #[error("accès impossible à {path} : {source}")]
    Io {
        /// Fichier ou dossier concerné.
        path: PathBuf,
        /// Erreur système d'origine.
        source: std::io::Error,
    },

    /// Le dossier personnel de l'utilisateur est introuvable (variable `HOME` absente, par exemple).
    #[error("dossier personnel introuvable ; indique un dossier avec l'option --home")]
    NoHomeDir,

    /// Le fichier de configuration n'est pas un TOML valide, ou contient un champ inconnu.
    #[error("configuration invalide dans {path} : {source}")]
    ConfigParse {
        /// Fichier de configuration.
        path: PathBuf,
        /// Erreur de lecture TOML (indique la ligne et le champ fautifs).
        source: toml::de::Error,
    },

    /// Aucune clé API n'est disponible, ni dans l'environnement ni dans le fichier.
    #[error(
        "clé API introuvable : définis la variable EDGE_ODDS_API_KEY ou ajoute odds_api_key dans le fichier de configuration"
    )]
    MissingApiKey,

    /// Le fichier de configuration contient un secret mais d'autres utilisateurs peuvent le lire.
    #[error(
        "permissions trop ouvertes sur {path} ({mode:o}) : il contient un secret, exécute `chmod 600 {path}`"
    )]
    InsecurePermissions {
        /// Fichier concerné.
        path: PathBuf,
        /// Permissions actuelles (bits d'accès, affichées en octal).
        mode: u32,
    },
}

impl Error {
    /// Construit la fonction qui transforme une `std::io::Error` en [`Error::Io`] pour ce chemin.
    ///
    /// Usage : `fs::read_to_string(p).map_err(Error::io(p))?`. Le chemin n'est copié (`to_path_buf`)
    /// que si l'erreur survient réellement : la fermeture garde seulement une référence jusque-là.
    pub(crate) fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error {
        move |source| Error::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Résultat standard de la bibliothèque.
pub type Result<T> = std::result::Result<T, Error>;
