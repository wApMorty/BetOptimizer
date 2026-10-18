//! Erreurs de la bibliothèque.
//!
//! Un seul type, [`Error`], pour tout `core`. Chaque variante porte assez de contexte pour que le
//! message affiché dise quoi corriger. Le binaire les enveloppe dans `anyhow` (voir `edge-cli`).

use std::path::PathBuf;

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
}

/// Résultat standard de la bibliothèque.
pub type Result<T> = std::result::Result<T, Error>;
