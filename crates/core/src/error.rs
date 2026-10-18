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

    /// Une valeur de la configuration est inutilisable (identifiant malformé, liste vide…).
    #[error("configuration invalide : {0}")]
    InvalidConfig(String),

    /// Aucune clé API n'est disponible, ni dans l'environnement ni dans le fichier.
    #[error(
        "clé API introuvable : définis la variable EDGE_ODDS_API_KEY ou ajoute odds_api_key dans le fichier de configuration"
    )]
    MissingApiKey,

    /// Aucune URL de webhook Discord n'est disponible, ni dans l'environnement ni dans le fichier.
    #[error(
        "webhook Discord introuvable : définis la variable EDGE_DISCORD_WEBHOOK ou ajoute discord_webhook_url dans le fichier de configuration"
    )]
    MissingWebhook,

    /// Erreur renvoyée par SQLite. `#[from]` écrit pour nous `impl From<rusqlite::Error> for Error` :
    /// c'est ce qui permet d'écrire `?` sur un résultat rusqlite dans une fonction qui renvoie notre `Result`.
    #[error("erreur SQLite : {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// La base n'a pas pu être ouverte (dossier manquant, droits insuffisants…).
    #[error("impossible d'ouvrir la base {path} : {source}")]
    OpenDatabase {
        /// Fichier de base de données.
        path: PathBuf,
        /// Erreur SQLite d'origine.
        source: rusqlite::Error,
    },

    /// La base a été créée par une version plus récente d'edge : on refuse de la modifier.
    #[error(
        "la base est en version {found} mais ce programme ne connaît que la version {supported} : mets edge à jour"
    )]
    DatabaseTooNew {
        /// Version du schéma trouvée dans la base.
        found: u32,
        /// Dernière version connue de ce programme.
        supported: u32,
    },

    /// Le corps d'une réponse de l'API n'a pas le format attendu.
    #[error("réponse de l'API illisible : {0}")]
    ResponseFormat(#[from] serde_json::Error),

    /// L'API a répondu par une erreur (clé invalide, paramètre refusé, limite de débit…).
    #[error("l'API a refusé la requête (HTTP {status}, {code}) : {message}")]
    Api {
        /// Code de statut HTTP.
        status: u16,
        /// Code d'erreur de l'API (`INVALID_KEY`…), `?` s'il est absent.
        code: String,
        /// Message de l'API.
        message: String,
    },

    /// Échec réseau ou protocole lors d'un appel HTTP (le message ne contient jamais la clé).
    #[error("échec de la requête HTTP : {0}")]
    Http(String),

    /// La garde de budget refuse l'appel : il ferait dépasser le plafond de crédits.
    #[error(
        "garde de budget : {used} crédits déjà utilisés + {cost} pour cet appel dépasseraient le plafond de {cap}"
    )]
    BudgetExceeded {
        /// Crédits déjà consommés.
        used: u32,
        /// Coût de l'appel refusé.
        cost: u32,
        /// Plafond configuré.
        cap: u32,
    },

    /// Le quota n'a pas pu être lu : par prudence, aucun appel payant n'est lancé.
    #[error("quota inconnu : l'API n'a pas renvoyé l'en-tête x-requests-used")]
    QuotaUnknown,

    /// Un fichier de données (CSV) est illisible ou contient une valeur impossible.
    #[error("données invalides : {0}")]
    Data(String),

    /// Lecture CSV impossible.
    #[error("lecture CSV : {0}")]
    Csv(#[from] csv::Error),

    /// La table d'alias est invalide (TOML illisible, nom attribué à deux équipes).
    #[error("table d'alias invalide : {0}")]
    Aliases(String),

    /// Des noms d'équipes ne figurent pas dans la table d'alias : l'import est bloqué sans rien écrire.
    #[error(
        "{} nom(s) d'équipe inconnu(s) : {} ; ajoute-les à config/aliases.toml puis réinstalle edge",
        names.len(),
        names.join(", ")
    )]
    UnknownTeams {
        /// Noms inconnus, au format `ligue: nom`.
        names: Vec<String>,
    },

    /// L'optimiseur n'a pas pu ajuster le modèle (problème mal posé, erreur numérique).
    #[error("ajustement du modèle impossible : {0}")]
    Optimizer(String),

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
