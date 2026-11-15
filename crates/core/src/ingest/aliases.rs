//! Table d'alias des noms d'équipes entre sources.
//!
//! Chaque source écrit les noms à sa façon (`Man United` chez football-data, `Manchester United` chez
//! The Odds API). La table fait correspondre chaque nom publié à un nom canonique, ligue par ligue.
//! Règle du projet : un nom absent de la table **bloque** l'import. Jamais d'appariement approximatif
//! silencieux : un mauvais rapprochement fausserait des milliers de matchs sans que personne ne le voie.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::store::alias_proposals;

/// Table versionnée dans le dépôt (`config/aliases.toml`), embarquée dans le binaire à la compilation.
const BUNDLED: &str = include_str!("../../../../config/aliases.toml");

/// Correspondance nom publié → nom canonique, par ligue (code football-data : `E0`, `SP1`…).
///
/// `Default` donne la table vide, où aucun nom ne se résout : les comparaisons s'y font alors sur les noms bruts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    /// Cette table, complétée des alias acceptés dans l'application (déduction par la rencontre, phase 23) : ils servent
    /// tout de suite, sans recompiler edge, jusqu'à leur report dans `config/aliases.toml` (`edge aliases export`).
    ///
    /// `&self` et non `self` : l'application garde la table versionnée et en tire une table complète à chaque usage, car
    /// un alias peut être accepté entre deux pages.
    pub fn with_learned(&self, conn: &Connection) -> Result<Aliases> {
        // `clone` : la table versionnée reste intacte pour le prochain appel.
        let mut merged = self.clone();
        for (section, name, canonical) in alias_proposals::accepted(conn)? {
            merged.learn(&section, &name, &canonical);
        }
        Ok(merged)
    }

    /// Ajoute l'alias `name` → `canonical` à une section. Si `canonical` est lui-même une variante, l'alias va à l'équipe
    /// qu'elle désigne. Un nom déjà attribué à une autre équipe n'est pas touché : la table versionnée prime, et le conflit
    /// est journalisé (il se règle en refusant l'alias ou en corrigeant `config/aliases.toml`).
    fn learn(&mut self, section: &str, name: &str, canonical: &str) {
        let names = self.by_div.entry(section.to_string()).or_default();
        let team = names
            .get(canonical)
            .cloned()
            .unwrap_or_else(|| canonical.to_string());
        names.entry(team.clone()).or_insert_with(|| team.clone());
        match names.get(name) {
            Some(other) if *other != team => tracing::warn!(
                "alias appris ignoré : {section} « {name} » désigne déjà « {other} », pas « {team} »"
            ),
            Some(_) => {}
            None => {
                names.insert(name.to_string(), team);
            }
        }
    }

    /// Nom canonique d'un nom publié dans une ligue, ou `None` s'il est inconnu.
    pub fn resolve(&self, div: &str, name: &str) -> Option<&str> {
        // Le `?` fonctionne aussi sur les `Option` : il renvoie `None` si la ligue est inconnue.
        self.by_div.get(div)?.get(name).map(String::as_str)
    }

    /// Tous les noms d'une équipe dans une ligue (nom canonique et variantes), triés ; vide si elle est inconnue.
    pub fn names_of(&self, div: &str, canonical: &str) -> Vec<&str> {
        let Some(names) = self.by_div.get(div) else {
            return Vec::new();
        };
        let mut out: Vec<&str> = names
            .iter()
            .filter(|(_, team)| team.as_str() == canonical)
            .map(|(name, _)| name.as_str())
            .collect();
        // Une `HashMap` n'a pas d'ordre stable : le tri garantit le même résultat d'une exécution à l'autre.
        out.sort_unstable();
        out
    }
}

/// Écrit un nom comme une chaîne TOML entre guillemets (`\` et `"` échappés).
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Reporte des alias appris (section, nom, nom canonique) dans le texte de `config/aliases.toml`, en gardant ses
/// commentaires et son ordre. Renvoie le nouveau texte et les lignes ajoutées ou complétées, au format du fichier ; un alias
/// que le fichier connaît déjà n'y change rien.
///
/// Une variante s'ajoute à la ligne de son équipe (`"Málaga" = ["Malaga"]`) ; une équipe absente prend une ligne neuve en
/// fin de section. Le résultat est relu et vérifié : chaque alias doit s'y résoudre vers son équipe, sinon c'est une erreur
/// et rien ne doit être écrit (conflit avec un nom attribué à une autre équipe, section absente, tableau sur plusieurs
/// lignes).
pub fn export_learned(
    text: &str,
    learned: &[(String, String, String)],
) -> Result<(String, Vec<String>)> {
    let file = Aliases::parse(text)?;
    // Le fichier se modifie ligne à ligne ; `String` par ligne pour pouvoir en remplacer une ou en insérer.
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut changed: Vec<String> = Vec::new();
    // Alias à vérifier dans le résultat : (section, nom, équipe).
    let mut expected: Vec<(String, String, String)> = Vec::new();
    for (section, name, canonical) in learned {
        let team = file
            .resolve(section, canonical)
            .unwrap_or(canonical)
            .to_string();
        expected.push((section.clone(), name.clone(), team.clone()));
        if file.resolve(section, name).is_some() {
            continue; // déjà connu : la vérification finale dira s'il désigne bien la même équipe
        }
        let header = format!("[{section}]");
        let Some(start) = lines.iter().position(|l| l.trim() == header) else {
            return Err(Error::Aliases(format!(
                "section [{section}] absente du fichier"
            )));
        };
        // Fin de la section : la ligne d'en-tête suivante, ou la fin du fichier.
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .map_or(lines.len(), |i| start + 1 + i);
        let prefix = format!("{} = [", quoted(&team));
        let existing = (start + 1..end).find(|i| lines[*i].starts_with(&prefix));
        let line = match existing {
            Some(i) if name == &team => i, // l'équipe a déjà sa ligne : rien à ajouter
            Some(i) => {
                let current = lines[i].trim_end();
                let Some(inner) = current
                    .strip_prefix(&prefix)
                    .and_then(|r| r.strip_suffix(']'))
                else {
                    return Err(Error::Aliases(format!(
                        "[{section}] ligne de « {team} » illisible (tableau sur plusieurs lignes ?) : {current}"
                    )));
                };
                let separator = if inner.trim().is_empty() { "" } else { ", " };
                lines[i] = format!("{prefix}{}{separator}{}]", inner.trim(), quoted(name));
                i
            }
            None => {
                let variants = if name == &team {
                    String::new()
                } else {
                    quoted(name)
                };
                // Après la dernière ligne non vide de la section : les lignes vides qui la séparent de la suivante restent.
                let at = (start + 1..end)
                    .rev()
                    .find(|i| !lines[*i].trim().is_empty())
                    .map_or(start + 1, |i| i + 1);
                lines.insert(at, format!("{prefix}{variants}]"));
                at
            }
        };
        let shown = format!("[{section}] {}", lines[line]);
        // Une ligne complétée deux fois n'est montrée que sous sa forme finale.
        changed.retain(|c| !c.starts_with(&format!("[{section}] {prefix}")));
        changed.push(shown);
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    let result = Aliases::parse(&out)?;
    for (section, name, team) in &expected {
        if result.resolve(section, name) != Some(team.as_str()) {
            return Err(Error::Aliases(format!(
                "[{section}] « {name} » devrait désigner « {team} », le fichier dit {:?} : corrige à la main",
                result.resolve(section, name)
            )));
        }
    }
    Ok((out, changed))
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
    fn names_of_lists_every_name_of_a_team_in_a_stable_order() {
        let aliases = Aliases::parse(SMALL).unwrap();
        assert_eq!(
            aliases.names_of("E0", "Manchester United"),
            ["Man United", "Man Utd", "Manchester United"]
        );
        assert!(aliases.names_of("E0", "Inconnu FC").is_empty());
        assert!(aliases.names_of("Z9", "Burnley").is_empty());
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
    fn learned_aliases_join_the_team_of_their_canonical_name_without_overriding_the_table() {
        let mut aliases = Aliases::parse(SMALL).unwrap();
        // Nom canonique connu : simple variante de plus.
        aliases.learn("E0", "Man Utd FC", "Manchester United");
        assert_eq!(
            aliases.resolve("E0", "Man Utd FC"),
            Some("Manchester United")
        );
        // Nom « canonique » qui n'est qu'une variante : l'alias va à l'équipe qu'elle désigne.
        aliases.learn("E0", "MUFC", "Man United");
        assert_eq!(aliases.resolve("E0", "MUFC"), Some("Manchester United"));
        // Équipe nouvelle : elle se désigne elle-même.
        aliases.learn("E0", "Wrexham AFC", "Wrexham");
        assert_eq!(aliases.resolve("E0", "Wrexham"), Some("Wrexham"));
        assert_eq!(aliases.resolve("E0", "Wrexham AFC"), Some("Wrexham"));
        // Un nom déjà attribué garde son équipe.
        aliases.learn("E0", "Burnley", "Manchester United");
        assert_eq!(aliases.resolve("E0", "Burnley"), Some("Burnley"));
        // Section absente : créée.
        aliases.learn("ESPN", "FC Cologne", "1. FC Köln");
        assert_eq!(aliases.resolve("ESPN", "FC Cologne"), Some("1. FC Köln"));
    }

    const FILE: &str = "# Commentaire gardé\n[LOTOFOOT]\n# Clubs\n\"Fulham\" = []\n\"Málaga\" = [\"Malaga CF\"]\n\n[ESPN]\n\"Fulham\" = []\n";

    fn learned(items: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        items
            .iter()
            .map(|(s, n, c)| (s.to_string(), n.to_string(), c.to_string()))
            .collect()
    }

    #[test]
    fn exported_aliases_join_their_team_line_or_get_a_new_one_and_comments_stay() {
        let (out, changed) = export_learned(
            FILE,
            &learned(&[
                ("LOTOFOOT", "Malaga", "Málaga"),
                ("LOTOFOOT", "Fulham FC", "Fulham"),
                ("LOTOFOOT", "Burgos CF", "Burgos CF"),
                ("ESPN", "Málaga CF", "Málaga"),
                // Déjà dans le fichier : rien ne change.
                ("LOTOFOOT", "Malaga CF", "Málaga"),
            ]),
        )
        .unwrap();
        assert_eq!(
            out,
            "# Commentaire gardé\n[LOTOFOOT]\n# Clubs\n\"Fulham\" = [\"Fulham FC\"]\n\"Málaga\" = [\"Malaga CF\", \"Malaga\"]\n\"Burgos CF\" = []\n\n[ESPN]\n\"Fulham\" = []\n\"Málaga\" = [\"Málaga CF\"]\n"
        );
        assert_eq!(
            changed,
            [
                "[LOTOFOOT] \"Málaga\" = [\"Malaga CF\", \"Malaga\"]",
                "[LOTOFOOT] \"Fulham\" = [\"Fulham FC\"]",
                "[LOTOFOOT] \"Burgos CF\" = []",
                "[ESPN] \"Málaga\" = [\"Málaga CF\"]",
            ]
        );
        // Reporter une seconde fois ne change plus rien.
        let (again, changed) =
            export_learned(&out, &learned(&[("LOTOFOOT", "Malaga", "Málaga")])).unwrap();
        assert_eq!(again, out);
        assert!(changed.is_empty());
    }

    #[test]
    fn an_export_that_contradicts_the_file_is_refused() {
        // « Malaga CF » désigne déjà Málaga : il ne peut pas aller à Fulham.
        let err =
            export_learned(FILE, &learned(&[("LOTOFOOT", "Malaga CF", "Fulham")])).unwrap_err();
        assert!(
            err.to_string().contains("devrait désigner « Fulham »"),
            "{err}"
        );
        let err = export_learned(FILE, &learned(&[("E0", "Spurs", "Tottenham")])).unwrap_err();
        assert!(err.to_string().contains("section [E0] absente"), "{err}");
    }

    #[test]
    fn names_are_quoted_as_toml_strings() {
        assert_eq!(quoted("Nott'm Forest"), "\"Nott'm Forest\"");
        assert_eq!(quoted("A \"B\""), "\"A \\\"B\\\"\"");
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
        // Noms d'ESPN qui bloquaient le suivi en direct du 2026-10-24.
        assert_eq!(aliases.resolve("ESPN", "FC Cologne"), Some("1. FC Köln"));
        assert_eq!(
            aliases.resolve("ESPN", "Internazionale"),
            Some("Inter Milan")
        );
        // Noms de la grille LF8 n°148 (FDJ, puis ESPN) qui bloquaient la séance du 2026-10-25.
        assert_eq!(
            aliases.resolve("LOTOFOOT", "FC Copenhague"),
            Some("FC Copenhagen")
        );
        assert_eq!(
            aliases.resolve("ESPN", "F.C. København"),
            Some("FC Copenhagen")
        );
        // Abréviations de la FDJ (LF7 n°137 et 139, LF8 n°153 et 155, `docs/sources.md` §74).
        for (shown, canonical) in [
            ("Atl. Madrid", "Atlético Madrid"),
            ("Man. United", "Manchester United"),
            ("Man. City", "Manchester City"),
        ] {
            assert_eq!(
                aliases.resolve("LOTOFOOT", shown),
                Some(canonical),
                "{shown}"
            );
        }
    }
}
