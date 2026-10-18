//! Table d'alias des noms d'équipes entre sources.
//!
//! Chaque source écrit les noms à sa façon (`Man United` chez football-data, `Manchester United` chez
//! The Odds API). La table fait correspondre chaque nom publié à un nom canonique, ligue par ligue.
//! Règle du projet : un nom absent de la table **bloque** l'import. Jamais d'appariement approximatif
//! silencieux : un mauvais rapprochement fausserait des milliers de matchs sans que personne ne le voie.

use std::collections::{BTreeMap, HashMap};

use crate::error::{Error, Result};

/// Table versionnée dans le dépôt (`config/aliases.toml`), embarquée dans le binaire à la compilation.
const BUNDLED: &str = include_str!("../../../../config/aliases.toml");

/// Correspondance nom publié → nom canonique, par ligue (code football-data : `E0`, `SP1`…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aliases {
    by_div: HashMap<String, HashMap<String, String>>,
}

impl Aliases {
    /// La table embarquée dans le binaire.
    pub fn bundled() -> Result<Aliases> {
        Aliases::parse(BUNDLED)
    }

    /// Lit une table au format TOML : une section par ligue, `"nom canonique" = ["variante", …]`.
    ///
    /// Le nom canonique se désigne lui-même. Un même nom attribué à deux équipes d'une ligue est une erreur.
    pub fn parse(text: &str) -> Result<Aliases> {
        let raw: BTreeMap<String, BTreeMap<String, Vec<String>>> =
            toml::from_str(text).map_err(|e| Error::Aliases(e.to_string()))?;
        let mut by_div = HashMap::new();
        for (div, teams) in raw {
            let mut names: HashMap<String, String> = HashMap::new();
            for (canonical, variants) in teams {
                // `once(&canonical).chain(…)` parcourt le nom canonique puis ses variantes, sans les copier.
                for name in std::iter::once(&canonical).chain(variants.iter()) {
                    if let Some(previous) = names.insert(name.clone(), canonical.clone())
                        && previous != canonical
                    {
                        return Err(Error::Aliases(format!(
                            "{div} : « {name} » désigne à la fois « {previous} » et « {canonical} »"
                        )));
                    }
                }
            }
            by_div.insert(div, names);
        }
        Ok(Aliases { by_div })
    }

    /// Nom canonique d'un nom publié dans une ligue, ou `None` s'il est inconnu.
    pub fn resolve(&self, div: &str, name: &str) -> Option<&str> {
        // Le `?` fonctionne aussi sur les `Option` : il renvoie `None` si la ligue est inconnue.
        self.by_div.get(div)?.get(name).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: &str = r#"
[E0]
"Manchester United" = ["Man United", "Man Utd"]
"Burnley" = []
[SC0]
"Dundee FC" = ["Dundee"]
"#;

    #[test]
    fn canonical_names_and_variants_resolve_to_the_canonical_name() {
        let aliases = Aliases::parse(SMALL).unwrap();
        assert_eq!(
            aliases.resolve("E0", "Man United"),
            Some("Manchester United")
        );
        assert_eq!(aliases.resolve("E0", "Man Utd"), Some("Manchester United"));
        assert_eq!(
            aliases.resolve("E0", "Manchester United"),
            Some("Manchester United")
        );
        assert_eq!(aliases.resolve("E0", "Burnley"), Some("Burnley"));
    }

    #[test]
    fn an_unknown_name_or_league_is_not_guessed() {
        let aliases = Aliases::parse(SMALL).unwrap();
        assert_eq!(aliases.resolve("E0", "Man Utd FC"), None);
        // Les noms sont propres à une ligue : « Dundee » n'existe pas en Premier League.
        assert_eq!(aliases.resolve("E0", "Dundee"), None);
        assert_eq!(aliases.resolve("D1", "Burnley"), None);
        // Pas d'insensibilité à la casse : un nom doit être exact.
        assert_eq!(aliases.resolve("E0", "burnley"), None);
    }

    #[test]
    fn a_name_given_to_two_teams_is_an_error() {
        let err = Aliases::parse("[E0]\n\"A\" = [\"X\"]\n\"B\" = [\"X\"]\n").unwrap_err();
        assert!(matches!(err, Error::Aliases(_)));
        assert!(err.to_string().contains("« X »"));
    }

    #[test]
    fn malformed_toml_is_an_error() {
        assert!(matches!(
            Aliases::parse("ceci n'est pas du toml").unwrap_err(),
            Error::Aliases(_)
        ));
    }

    #[test]
    fn the_bundled_table_is_valid_and_resolves_known_clubs() {
        let aliases = Aliases::bundled().unwrap();
        assert_eq!(
            aliases.resolve("E0", "Man United"),
            Some("Manchester United")
        );
        assert_eq!(
            aliases.resolve("E0", "Manchester United"),
            Some("Manchester United")
        );
        assert_eq!(
            aliases.resolve("D1", "M'gladbach"),
            Some("Borussia Monchengladbach")
        );
        assert_eq!(
            aliases.resolve("F1", "Paris SG"),
            Some("Paris Saint Germain")
        );
        assert_eq!(aliases.resolve("SC0", "Falkirk"), Some("Falkirk F.C."));
        assert_eq!(aliases.resolve("SP1", "Espanol"), Some("Espanyol"));
    }
}
