//! Déduction d'alias par la rencontre (phase 23, `docs/sources.md` §75) : quand un nom d'équipe manque à la table d'alias,
//! on cherche le match parmi ceux que l'outil a relevés, au lieu de comparer des noms qui se ressemblent.
//!
//! Exemple : la grille porte `Malaga – Fulham`, « Malaga » est inconnu, « Fulham » est connu. Si un seul match relevé dans la
//! fenêtre des rencontres oppose quelqu'un à Fulham, à l'extérieur, ce quelqu'un est « Malaga ». Rien n'est appliqué ici :
//! le résultat est une proposition, avec le match qui la prouve, que Paul valide ou refuse dans l'application.
//!
//! Une rencontre se reconnaît à ses deux places (domicile, extérieur). Chaque place est soit **ancrée** (nom connu de la
//! table qui désigne la même équipe, ou nom écrit à l'identique chez les deux sources), soit libre (nom inconnu, qui accepte
//! tout nom). Un match relevé convient s'il respecte les places ancrées et en a au moins une : deux noms inconnus et
//! différents de ceux de la source ne prouvent rien, sauf chez ESPN pour un match unique au coup d'envoi exact
//! ([`from_espn`]). Plusieurs matchs qui conviennent donnent plusieurs propositions, entre
//! lesquelles Paul choisit ; aucun n'est préféré en silence.

use chrono::{DateTime, Duration, Utc};

use super::odds::ALIAS_SECTION as LOTOFOOT;
use super::scores::ALIAS_SECTION as ESPN;
use crate::ingest::aliases::Aliases;

/// Source d'un match relevé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeenSource {
    /// The Odds API (`/odds` archivé, ou `/events` gratuit).
    OddsApi,
    /// OddsPapi (repli des séances Loto Foot).
    OddsPapi,
    /// Page « scoreboard » d'ESPN (suivi en direct).
    Espn,
}

impl SeenSource {
    /// Valeur écrite en base.
    pub fn code(self) -> &'static str {
        match self {
            SeenSource::OddsApi => "odds_api",
            SeenSource::OddsPapi => "oddspapi",
            SeenSource::Espn => "espn",
        }
    }

    /// Source correspondant à une valeur de la base ; `None` pour toute autre valeur.
    pub fn from_code(code: &str) -> Option<SeenSource> {
        match code {
            "odds_api" => Some(SeenSource::OddsApi),
            "oddspapi" => Some(SeenSource::OddsPapi),
            "espn" => Some(SeenSource::Espn),
            _ => None,
        }
    }

    /// Nom affiché.
    pub fn label(self) -> &'static str {
        match self {
            SeenSource::OddsApi => "The Odds API",
            SeenSource::OddsPapi => "OddsPapi",
            SeenSource::Espn => "ESPN",
        }
    }
}

/// Un match relevé chez une source, avec les noms qu'elle publie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// Source.
    pub source: SeenSource,
    /// Identifiant du match chez la source.
    pub id: String,
    /// Équipe à domicile, nom publié.
    pub home: String,
    /// Équipe à l'extérieur, nom publié.
    pub away: String,
    /// Coup d'envoi.
    pub kickoff: DateTime<Utc>,
}

/// Un alias proposé : dans la section `section`, `name` désignerait l'équipe `canonical`, d'après le match `seen`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// Section de la table d'alias (`LOTOFOOT`, `ESPN`).
    pub section: String,
    /// Nom inconnu, tel que publié.
    pub name: String,
    /// Nom canonique proposé.
    pub canonical: String,
    /// Où le nom a bloqué (« LF8 n°151, rencontre 3 : Malaga – Fulham »).
    pub context: String,
    /// Le match qui prouve la proposition.
    pub seen: Seen,
}

/// Noms inconnus (section `LOTOFOOT`) d'une rencontre de grille, `grid_match` (domicile, extérieur, noms de la grille),
/// déduits des matchs relevés `seen`, que l'appelant a déjà restreints à la fenêtre des rencontres de la grille et aux
/// relevés antérieurs à la séance.
///
/// Les matchs de The Odds API passent d'abord : ses noms sont les noms canoniques. Ceux d'OddsPapi ne servent que si The
/// Odds API n'en a aucun qui convienne, comme pour les cotes (repli de la phase 19).
pub fn from_grid(
    aliases: &Aliases,
    grid_match: (&str, &str),
    seen: &[Seen],
    context: &str,
) -> Vec<Proposal> {
    let ours = [grid_match.0, grid_match.1];
    // Nom canonique de chaque place, ou `None` si la grille y porte un nom inconnu.
    let known = ours.map(|name| aliases.resolve(LOTOFOOT, name));
    if known.iter().all(Option::is_some) {
        return Vec::new();
    }
    // Un nom publié par la source, ramené au nom canonique : celui de la table s'il y figure, sinon lui-même (un nom de
    // The Odds API est canonique par construction, même absent de la table).
    let canonical_of = |published: &'_ str| -> String {
        aliases
            .resolve(LOTOFOOT, published)
            .unwrap_or(published)
            .to_string()
    };
    for source in [SeenSource::OddsApi, SeenSource::OddsPapi] {
        let mut out: Vec<Proposal> = Vec::new();
        for s in seen.iter().filter(|s| s.source == source) {
            let theirs = [s.home.as_str(), s.away.as_str()];
            let mut anchored = false;
            let mut fits = true;
            for side in 0..2 {
                match known[side] {
                    // Place connue : la source doit y nommer la même équipe.
                    Some(team) => {
                        fits &= canonical_of(theirs[side]) == team;
                        anchored = true;
                    }
                    // Place inconnue : libre, mais un nom identique chez la source l'ancre.
                    None => anchored |= theirs[side] == ours[side],
                }
            }
            if !(fits && anchored) {
                continue;
            }
            for side in (0..2).filter(|side| known[*side].is_none()) {
                let canonical = canonical_of(theirs[side]);
                // L'équipe d'en face ne peut pas être aussi celle-ci (match aberrant, ou nom de la source mal relu).
                if known.contains(&Some(canonical.as_str())) {
                    continue;
                }
                push_new(
                    &mut out,
                    Proposal {
                        section: LOTOFOOT.to_string(),
                        name: ours[side].to_string(),
                        canonical,
                        context: context.to_string(),
                        // `clone` : la proposition possède son match, car elle survit à la liste des matchs relevés.
                        seen: s.clone(),
                    },
                );
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

/// Noms ESPN inconnus (section `ESPN`) d'une rencontre suivie, `ours` (domicile, extérieur, noms canoniques), d'après les
/// matchs de la page ESPN `games` dont le coup d'envoi est à moins de `tolerance` du nôtre.
///
/// Sens inverse de [`from_grid`] : ici nos noms sont connus, ce sont ceux d'ESPN qui manquent ; la proposition attribue le
/// nom d'ESPN à notre équipe de la même place.
///
/// Une règle de plus qu'avec les grilles, parce que le coup d'envoi est connu à la minute : si aucun match n'est ancré, un
/// match aux deux noms inconnus qui commence **exactement** à notre heure fait foi, à condition d'être le seul dans ce cas
/// (une page de Ligue 2 en multiplex n'en désigne aucun).
pub fn from_espn(
    aliases: &Aliases,
    ours: (&str, &str),
    kickoff: DateTime<Utc>,
    tolerance: Duration,
    games: &[Seen],
    context: &str,
) -> Vec<Proposal> {
    let ours = [ours.0, ours.1];
    let mut anchored_games: Vec<&Seen> = Vec::new();
    let mut same_time: Vec<&Seen> = Vec::new();
    for g in games {
        if (g.kickoff - kickoff).abs() > tolerance {
            continue;
        }
        let theirs = [g.home.as_str(), g.away.as_str()];
        let known = theirs.map(|name| aliases.resolve(ESPN, name));
        let mut anchored = false;
        let mut fits = true;
        for side in 0..2 {
            match known[side] {
                Some(team) => {
                    fits &= team == ours[side];
                    anchored = true;
                }
                None => anchored |= theirs[side] == ours[side],
            }
        }
        if fits && anchored && known.contains(&None) {
            anchored_games.push(g);
        } else if known == [None, None] && g.kickoff == kickoff {
            same_time.push(g);
        }
    }
    let chosen = if anchored_games.is_empty() && same_time.len() == 1 {
        same_time
    } else {
        anchored_games
    };
    let mut out: Vec<Proposal> = Vec::new();
    for g in chosen {
        let theirs = [g.home.as_str(), g.away.as_str()];
        for side in 0..2 {
            if aliases.resolve(ESPN, theirs[side]).is_some() {
                continue;
            }
            push_new(
                &mut out,
                Proposal {
                    section: ESPN.to_string(),
                    name: theirs[side].to_string(),
                    canonical: ours[side].to_string(),
                    context: context.to_string(),
                    // `clone` : la proposition possède son match, car elle survit à la page lue.
                    seen: g.clone(),
                },
            );
        }
    }
    out
}

/// Ajoute une proposition sauf si la même (section, nom, nom canonique) y est déjà : le premier match qui la prouve suffit.
fn push_new(out: &mut Vec<Proposal>, proposal: Proposal) {
    let duplicate = out.iter().any(|p| {
        p.section == proposal.section
            && p.name == proposal.name
            && p.canonical == proposal.canonical
    });
    if !duplicate {
        out.push(proposal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = r#"
[LOTOFOOT]
"Fulham" = []
"Lille" = []
"Málaga" = []
"FC Porto" = ["Porto"]
[ESPN]
"Paris Saint Germain" = ["Paris Saint-Germain"]
"Lille" = []
"#;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn seen(source: SeenSource, id: &str, home: &str, away: &str, kickoff: &str) -> Seen {
        Seen {
            source,
            id: id.to_string(),
            home: home.to_string(),
            away: away.to_string(),
            kickoff: at(kickoff),
        }
    }

    fn pairs(proposals: &[Proposal]) -> Vec<(&str, &str)> {
        proposals
            .iter()
            .map(|p| (p.name.as_str(), p.canonical.as_str()))
            .collect()
    }

    #[test]
    fn a_single_match_of_the_known_opponent_gives_the_unknown_name() {
        let aliases = Aliases::parse(TABLE).unwrap();
        let archive = [
            seen(
                SeenSource::OddsApi,
                "a",
                "Málaga",
                "Fulham",
                "2026-11-01T15:00:00Z",
            ),
            // Fulham à domicile : pas la même place, ne convient pas.
            seen(
                SeenSource::OddsApi,
                "b",
                "Fulham",
                "Everton",
                "2026-11-03T20:00:00Z",
            ),
        ];
        let found = from_grid(&aliases, ("Malaga", "Fulham"), &archive, "LF8 n°151");
        assert_eq!(pairs(&found), [("Malaga", "Málaga")]);
        assert_eq!(found[0].section, "LOTOFOOT");
        assert_eq!(found[0].seen.id, "a");
        assert_eq!(found[0].context, "LF8 n°151");
    }

    #[test]
    fn known_names_need_nothing() {
        let aliases = Aliases::parse(TABLE).unwrap();
        let archive = [seen(
            SeenSource::OddsApi,
            "a",
            "Málaga",
            "Fulham",
            "2026-11-01T15:00:00Z",
        )];
        assert!(from_grid(&aliases, ("Málaga", "Fulham"), &archive, "").is_empty());
    }

    #[test]
    fn several_fitting_matches_give_several_proposals_to_choose_from() {
        // LF7 n°137 : Lille a joué Bodø/Glimt puis Rennes, tous deux à l'extérieur, dans la fenêtre de la grille.
        let aliases = Aliases::parse(TABLE).unwrap();
        let archive = [
            seen(
                SeenSource::OddsApi,
                "a",
                "Bodø/Glimt",
                "Lille",
                "2026-11-03T20:00:00Z",
            ),
            seen(
                SeenSource::OddsApi,
                "b",
                "Rennes",
                "Lille",
                "2026-11-08T16:00:00Z",
            ),
        ];
        let found = from_grid(&aliases, ("Bodo Glimt", "Lille"), &archive, "");
        assert_eq!(
            pairs(&found),
            [("Bodo Glimt", "Bodø/Glimt"), ("Bodo Glimt", "Rennes")]
        );
    }

    #[test]
    fn two_unknown_names_need_one_identical_name() {
        let aliases = Aliases::parse(TABLE).unwrap();
        let archive = [
            seen(
                SeenSource::OddsApi,
                "a",
                "Montpellier",
                "Red Star",
                "2026-10-25T13:00:00Z",
            ),
            seen(
                SeenSource::OddsApi,
                "b",
                "Pau FC",
                "Grenoble",
                "2026-10-25T13:00:00Z",
            ),
        ];
        // Les deux noms sont écrits comme chez The Odds API : identités proposées.
        let found = from_grid(&aliases, ("Montpellier", "Red Star"), &archive, "");
        assert_eq!(
            pairs(&found),
            [("Montpellier", "Montpellier"), ("Red Star", "Red Star")]
        );
        // Un seul nom identique suffit à ancrer la rencontre ; l'autre en découle.
        let found = from_grid(&aliases, ("Montpellier", "Red Star FC"), &archive, "");
        assert_eq!(
            pairs(&found),
            [("Montpellier", "Montpellier"), ("Red Star FC", "Red Star")]
        );
        // Aucun nom identique, aucun nom connu : rien n'est déduit, même si les noms se ressemblent.
        assert!(from_grid(&aliases, ("Pau", "Grenoble Foot"), &archive, "").is_empty());
    }

    #[test]
    fn source_names_are_brought_back_to_their_canonical_name() {
        // The Odds API écrit `Porto` en coupe d'Europe : la variante désigne `FC Porto`.
        let aliases = Aliases::parse(TABLE).unwrap();
        let archive = [seen(
            SeenSource::OddsApi,
            "a",
            "Porto",
            "Fulham",
            "2026-11-05T20:00:00Z",
        )];
        let found = from_grid(&aliases, ("FC Porto", "Fulham"), &archive, "");
        assert!(found.is_empty(), "les deux noms sont connus");
        let found = from_grid(&aliases, ("Porto FC", "Fulham"), &archive, "");
        assert_eq!(pairs(&found), [("Porto FC", "FC Porto")]);
    }

    #[test]
    fn oddspapi_only_serves_when_the_odds_api_has_nothing() {
        let aliases = Aliases::parse(TABLE).unwrap();
        let papi = seen(
            SeenSource::OddsPapi,
            "p",
            "Malaga CF",
            "Fulham",
            "2026-11-01T15:00:00Z",
        );
        let api = seen(
            SeenSource::OddsApi,
            "a",
            "Málaga",
            "Fulham",
            "2026-11-01T15:00:00Z",
        );
        let found = from_grid(&aliases, ("Malaga", "Fulham"), &[papi.clone(), api], "");
        assert_eq!(pairs(&found), [("Malaga", "Málaga")]);
        let found = from_grid(&aliases, ("Malaga", "Fulham"), &[papi], "");
        assert_eq!(pairs(&found), [("Malaga", "Malaga CF")]);
        assert_eq!(found[0].seen.source, SeenSource::OddsPapi);
    }

    #[test]
    fn the_opponent_cannot_be_proposed_for_the_unknown_place() {
        let aliases = Aliases::parse(TABLE).unwrap();
        let archive = [seen(
            SeenSource::OddsApi,
            "a",
            "Fulham",
            "Fulham",
            "2026-11-01T15:00:00Z",
        )];
        assert!(from_grid(&aliases, ("Inconnu", "Fulham"), &archive, "").is_empty());
    }

    #[test]
    fn espn_names_are_deduced_near_our_kickoff_only() {
        let aliases = Aliases::parse(TABLE).unwrap();
        let kickoff = at("2026-11-08T16:00:00Z");
        let page = [
            seen(
                SeenSource::Espn,
                "1",
                "Stade Rennais",
                "Lille",
                "2026-11-08T16:00:00Z",
            ),
            // Même adversaire, mais trop loin de notre coup d'envoi.
            seen(
                SeenSource::Espn,
                "2",
                "Rennes B",
                "Lille",
                "2026-11-08T21:00:00Z",
            ),
            // Deux noms inconnus et différents des nôtres : rien n'est déduit.
            seen(
                SeenSource::Espn,
                "3",
                "Brest",
                "Nantes",
                "2026-11-08T16:00:00Z",
            ),
        ];
        let found = from_espn(
            &aliases,
            ("Rennes", "Lille"),
            kickoff,
            Duration::hours(3),
            &page,
            "LF8 n°160, rencontre 2",
        );
        assert_eq!(pairs(&found), [("Stade Rennais", "Rennes")]);
        assert_eq!(found[0].section, "ESPN");
        assert_eq!(found[0].seen.id, "1");
    }

    #[test]
    fn an_espn_name_identical_to_ours_is_proposed_as_itself() {
        let aliases = Aliases::parse(TABLE).unwrap();
        let page = [seen(
            SeenSource::Espn,
            "1",
            "Rennes",
            "Brest",
            "2026-11-08T16:00:00Z",
        )];
        let found = from_espn(
            &aliases,
            ("Rennes", "Brest"),
            at("2026-11-08T16:00:00Z"),
            Duration::hours(3),
            &page,
            "",
        );
        assert_eq!(pairs(&found), [("Rennes", "Rennes"), ("Brest", "Brest")]);
    }

    #[test]
    fn a_known_espn_name_of_another_team_rules_the_match_out() {
        let aliases = Aliases::parse(TABLE).unwrap();
        // ESPN y nomme Lille, connu : ce n'est donc pas notre match Paris Saint Germain – Rennes.
        let page = [seen(
            SeenSource::Espn,
            "1",
            "Lille",
            "Rennes",
            "2026-11-08T16:00:00Z",
        )];
        let found = from_espn(
            &aliases,
            ("Paris Saint Germain", "Rennes"),
            at("2026-11-08T16:00:00Z"),
            Duration::hours(3),
            &page,
            "",
        );
        assert!(found.is_empty());
    }

    #[test]
    fn two_unknown_espn_names_need_a_single_match_at_our_exact_kickoff() {
        let aliases = Aliases::parse(TABLE).unwrap();
        let kickoff = at("2026-10-24T16:00:00Z");
        let dijon = seen(
            SeenSource::Espn,
            "402",
            "Dijon FCO",
            "Rodez Aveyron",
            "2026-10-24T16:00:00Z",
        );
        // Une heure plus tard : dans la tolérance, mais pas à l'heure exacte.
        let later = seen(
            SeenSource::Espn,
            "403",
            "Amiens SC",
            "Laval",
            "2026-10-24T17:00:00Z",
        );
        let found = from_espn(
            &aliases,
            ("Dijon", "Rodez"),
            kickoff,
            Duration::hours(3),
            &[dijon.clone(), later],
            "",
        );
        assert_eq!(
            pairs(&found),
            [("Dijon FCO", "Dijon"), ("Rodez Aveyron", "Rodez")]
        );
        // Multiplex : deux matchs aux noms inconnus à la même heure, aucun n'est désigné.
        let other = seen(
            SeenSource::Espn,
            "404",
            "Bastia",
            "Annecy",
            "2026-10-24T16:00:00Z",
        );
        let found = from_espn(
            &aliases,
            ("Dijon", "Rodez"),
            kickoff,
            Duration::hours(3),
            &[dijon, other],
            "",
        );
        assert!(found.is_empty());
    }

    #[test]
    fn source_codes_round_trip() {
        for source in [SeenSource::OddsApi, SeenSource::OddsPapi, SeenSource::Espn] {
            assert_eq!(SeenSource::from_code(source.code()), Some(source));
        }
        assert_eq!(SeenSource::from_code("autre"), None);
    }
}
