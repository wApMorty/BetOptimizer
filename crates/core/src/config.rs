//! Configuration de l'application et clé API.
//!
//! La configuration est un fichier TOML facultatif ; chaque valeur omise prend sa valeur par défaut.
//! La clé API se lit d'abord dans la variable d'environnement, puis dans le fichier, qui doit alors
//! être en permissions 0600 (jamais lisible par un autre utilisateur).

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::market::margin::MarginMethod;

/// Nom de la variable d'environnement qui contient la clé API.
pub const API_KEY_ENV: &str = "EDGE_ODDS_API_KEY";

/// Contenu de `config/edge.example.toml`, embarqué dans le binaire à la compilation (`include_str!`).
/// `edge init` l'écrit chez l'utilisateur ; un test garantit qu'il reste identique aux valeurs par défaut.
pub const EXAMPLE_TOML: &str = include_str!("../../../config/edge.example.toml");

/// Clé secrète de l'API.
///
/// C'est un *newtype* : une structure à un seul champ qui enveloppe un `String`. Pour le compilateur,
/// une `ApiKey` n'est pas un `String` : on ne peut pas la passer par erreur à une fonction qui attend
/// un nom de ligue, ni l'afficher sans le vouloir. Ici, `Debug` est écrit à la main pour masquer la
/// valeur : même un `{:?}` oublié dans un log ne divulgue rien.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct ApiKey(String);

impl ApiKey {
    /// Enveloppe une valeur déjà lue.
    pub fn new(value: impl Into<String>) -> ApiKey {
        ApiKey(value.into())
    }

    /// Donne accès à la valeur en clair. Le nom rend chaque usage visible à la relecture :
    /// à n'appeler que pour construire la requête HTTP.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

/// Configuration de l'application.
///
/// `#[serde(default)]` : un champ absent du fichier reçoit la valeur de [`Config::default`].
/// `deny_unknown_fields` : un champ inconnu (faute de frappe) est une erreur, jamais ignoré.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Plafond de crédits consommés par période de quota (garde de budget).
    pub monthly_credit_cap: u32,
    /// Ligues archivées, au format des clés de The Odds API (`soccer_epl`…).
    pub leagues: Vec<String>,
    /// Bookmakers interrogés (`pinnacle`, `winamax_fr`…).
    pub bookmakers: Vec<String>,
    /// Marchés interrogés (`h2h` = victoire / nul / défaite).
    pub markets: Vec<String>,
    /// Méthode de retrait de la marge des cotes. `power` : log loss le plus bas sur 19 138 matchs Pinnacle
    /// (rapport `edge market report`, 2026-10-18), écart avec les autres non significatif au niveau Pinnacle.
    pub margin_method: MarginMethod,
    /// Clé API écrite dans le fichier (alternative à la variable d'environnement).
    pub odds_api_key: Option<ApiKey>,
}

impl Default for Config {
    fn default() -> Config {
        // `to_string` crée un `String` possédé à partir du littéral `&'static str` : le champ
        // doit posséder sa donnée pour que la configuration vive indépendamment du code source.
        let owned = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        Config {
            monthly_credit_cap: 450,
            leagues: owned(&[
                "soccer_epl",
                "soccer_spain_la_liga",
                "soccer_germany_bundesliga",
                "soccer_italy_serie_a",
                "soccer_france_ligue_one",
                "soccer_spl",
            ]),
            bookmakers: owned(&[
                "pinnacle",
                "winamax_fr",
                "betclic_fr",
                "unibet_fr",
                "pmu_fr",
                "netbet_fr",
            ]),
            markets: owned(&["h2h"]),
            margin_method: MarginMethod::Power,
            odds_api_key: None,
        }
    }
}

impl Config {
    /// Lit la configuration depuis un texte TOML.
    pub fn parse(text: &str) -> std::result::Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    /// Charge le fichier de configuration. Un fichier absent donne la configuration par défaut.
    ///
    /// Si le fichier contient une clé API, ses permissions doivent être 0600 (Unix).
    pub fn load(path: &Path) -> Result<Config> {
        // `match` avec garde (`if`) : on distingue « fichier absent » (cas normal à la première
        // utilisation) des autres erreurs d'entrée/sortie, qui sont de vrais problèmes.
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(Error::io(path)(e)),
        };
        let config = Config::parse(&text).map_err(|source| Error::ConfigParse {
            path: path.to_path_buf(),
            source,
        })?;
        config.validate()?;
        if config.odds_api_key.is_some() {
            check_permissions(path)?;
        }
        Ok(config)
    }

    /// Vérifie les valeurs avant tout appel réseau : un identifiant malformé ou une liste vide
    /// serait envoyé à l'API, qui peut facturer la requête sans la rejeter.
    pub fn validate(&self) -> Result<()> {
        if self.monthly_credit_cap == 0 {
            return Err(Error::InvalidConfig(
                "monthly_credit_cap doit être supérieur à 0".to_string(),
            ));
        }
        let lists = [
            ("leagues", &self.leagues),
            ("bookmakers", &self.bookmakers),
            ("markets", &self.markets),
        ];
        for (field, names) in lists {
            if names.is_empty() {
                return Err(Error::InvalidConfig(format!(
                    "{field} ne doit pas être vide"
                )));
            }
            if let Some(bad) = names.iter().find(|name| !is_api_name(name)) {
                return Err(Error::InvalidConfig(format!(
                    "{field} : « {bad} » n'est pas un identifiant valide (minuscules, chiffres et _ uniquement)"
                )));
            }
        }
        Ok(())
    }

    /// Détermine la clé API : la variable d'environnement (`from_env`) prime, sinon celle du fichier.
    ///
    /// L'appelant lit l'environnement et nous passe la valeur : la fonction reste pure, donc testable
    /// sans modifier l'environnement du processus.
    pub fn api_key(&self, from_env: Option<&str>) -> Result<ApiKey> {
        match from_env.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => Ok(ApiKey::new(value)),
            // `clone()` : `self` n'est qu'emprunté ici ; l'appelant reçoit sa propre copie de la clé
            // (une petite chaîne) et n'est plus lié à la durée de vie de la configuration.
            // Une clé vide dans le fichier (`odds_api_key = ""`) équivaut à une clé absente.
            None => self
                .odds_api_key
                .clone()
                .filter(|key| !key.expose().is_empty())
                .ok_or(Error::MissingApiKey),
        }
    }
}

/// Les clés de The Odds API (`soccer_epl`, `winamax_fr`, `h2h`) sont en minuscules, chiffres et `_`.
/// Cela garantit aussi qu'aucun caractère spécial ne peut se glisser dans l'URL.
fn is_api_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Refuse un fichier de secrets lisible ou modifiable par le groupe ou les autres.
#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)
        .map_err(Error::io(path))?
        .permissions()
        .mode()
        & 0o777;
    // 0o077 = bits « groupe » et « autres » ; s'il y en a un d'allumé, le fichier est trop ouvert.
    if mode & 0o077 != 0 {
        return Err(Error::InsecurePermissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

/// Hors Unix (Windows), les permissions fonctionnent autrement : aucun contrôle.
#[cfg(not(unix))]
fn check_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_file_matches_the_defaults() {
        // Garantit que l'exemple documenté et les valeurs par défaut du code ne divergent jamais.
        assert_eq!(Config::parse(EXAMPLE_TOML).unwrap(), Config::default());
    }

    #[test]
    fn default_cap_is_450() {
        assert_eq!(Config::default().monthly_credit_cap, 450);
    }

    #[test]
    fn omitted_fields_keep_their_defaults() {
        let config = Config::parse("monthly_credit_cap = 300").unwrap();
        assert_eq!(config.monthly_credit_cap, 300);
        assert_eq!(config.leagues, Config::default().leagues);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = Config::parse("monthly_credit_capp = 300").unwrap_err();
        assert!(err.to_string().contains("monthly_credit_capp"));
    }

    #[test]
    fn missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("absent.toml")).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn invalid_toml_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.toml");
        fs::write(&path, "ceci n'est pas du toml").unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(err.to_string().contains("edge.toml"));
    }

    #[test]
    fn environment_key_has_priority_over_file_key() {
        let config = Config::parse(r#"odds_api_key = "du-fichier""#).unwrap();
        let key = config.api_key(Some("de-l-environnement")).unwrap();
        assert_eq!(key.expose(), "de-l-environnement");
    }

    #[test]
    fn file_key_is_used_without_environment() {
        let config = Config::parse(r#"odds_api_key = "du-fichier""#).unwrap();
        assert_eq!(config.api_key(None).unwrap().expose(), "du-fichier");
    }

    #[test]
    fn empty_environment_value_counts_as_absent() {
        let config = Config::parse(r#"odds_api_key = "du-fichier""#).unwrap();
        assert_eq!(config.api_key(Some("  ")).unwrap().expose(), "du-fichier");
    }

    #[test]
    fn empty_key_in_the_file_counts_as_missing() {
        let config = Config::parse("odds_api_key = \"\"").unwrap();
        assert!(matches!(
            config.api_key(None).unwrap_err(),
            Error::MissingApiKey
        ));
    }

    #[test]
    fn missing_key_error_tells_what_to_do() {
        let err = Config::default().api_key(None).unwrap_err();
        let message = err.to_string();
        assert!(message.contains(API_KEY_ENV));
        assert!(message.contains("odds_api_key"));
    }

    #[test]
    fn default_config_is_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn malformed_identifier_is_rejected_with_its_field() {
        let config = Config::parse(r#"bookmakers = ["pinnacle", "Winamax FR"]"#).unwrap();
        let message = config.validate().unwrap_err().to_string();
        assert!(message.contains("bookmakers"));
        assert!(message.contains("Winamax FR"));
    }

    #[test]
    fn empty_list_and_zero_cap_are_rejected() {
        assert!(Config::parse("leagues = []").unwrap().validate().is_err());
        assert!(
            Config::parse("monthly_credit_cap = 0")
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[test]
    fn load_rejects_an_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.toml");
        fs::write(&path, r#"markets = ["h2h;drop"]"#).unwrap();
        assert!(matches!(
            Config::load(&path).unwrap_err(),
            Error::InvalidConfig(_)
        ));
    }

    #[test]
    fn debug_output_never_shows_the_key() {
        let config = Config::parse(r#"odds_api_key = "SECRET-123""#).unwrap();
        assert!(!format!("{config:?}").contains("SECRET-123"));
        assert!(!format!("{:?}", config.api_key(None).unwrap()).contains("SECRET-123"));
    }

    #[cfg(unix)]
    mod permissions {
        use std::os::unix::fs::PermissionsExt;

        use super::*;

        fn write_with_mode(dir: &Path, content: &str, mode: u32) -> std::path::PathBuf {
            let path = dir.join("edge.toml");
            fs::write(&path, content).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            path
        }

        #[test]
        fn file_with_a_key_and_open_permissions_is_refused() {
            let dir = tempfile::tempdir().unwrap();
            let path = write_with_mode(dir.path(), r#"odds_api_key = "x""#, 0o644);
            let err = Config::load(&path).unwrap_err();
            assert!(matches!(
                err,
                Error::InsecurePermissions { mode: 0o644, .. }
            ));
            assert!(err.to_string().contains("chmod 600"));
        }

        #[test]
        fn file_with_a_key_and_mode_600_is_accepted() {
            let dir = tempfile::tempdir().unwrap();
            let path = write_with_mode(dir.path(), r#"odds_api_key = "x""#, 0o600);
            assert!(Config::load(&path).unwrap().odds_api_key.is_some());
        }

        #[test]
        fn file_without_a_key_may_stay_readable() {
            let dir = tempfile::tempdir().unwrap();
            let path = write_with_mode(dir.path(), "monthly_credit_cap = 300", 0o644);
            assert_eq!(Config::load(&path).unwrap().monthly_credit_cap, 300);
        }
    }
}
