//! Traduction de l'intitulé d'une cote boostée (« Lyon gagne et les deux équipes marquent ») en conditions sur le
//! **score final** du match. Seuls ces paris se tarifent avec une grille de scores ; les autres (buteurs,
//! mi-temps, corners…) sont déclarés non évaluables plutôt que devinés.
//!
//! La reconnaissance est volontairement littérale : une tournure inconnue donne `None`, jamais une approximation.

use super::wepari::clean;

/// Équipe désignée par l'intitulé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Équipe à domicile.
    Home,
    /// Équipe à l'extérieur.
    Away,
}

/// Une condition sur le score final `(buts à domicile, buts à l'extérieur)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// L'équipe gagne.
    Win(Side),
    /// L'équipe gagne ou fait match nul (double chance).
    NotLose(Side),
    /// Match nul.
    Draw,
    /// Une des deux équipes gagne (pas de match nul).
    NoDraw,
    /// Les deux équipes marquent.
    BothScore,
    /// Au moins `n` buts dans le match (« plus de 2,5 buts » : au moins 3).
    TotalAtLeast(u32),
    /// Au plus `n` buts dans le match (« moins de 2,5 buts » : au plus 2).
    TotalAtMost(u32),
    /// L'équipe gagne par au moins `n` buts d'écart.
    WinBy(Side, u32),
    /// L'équipe gagne sans encaisser de but.
    WinToNil(Side),
    /// L'équipe gagne sur l'un des scores donnés, écrits `(buts de l'équipe, buts de l'adversaire)`.
    Scores(Side, Vec<(u32, u32)>),
    /// L'équipe marque au moins `n` buts.
    TeamAtLeast(Side, u32),
}

impl Condition {
    /// La condition est-elle remplie par le score final ?
    pub fn holds(&self, home_goals: u32, away_goals: u32) -> bool {
        // Buts de l'équipe désignée, puis de son adversaire.
        let goals = |side: &Side| match side {
            Side::Home => (home_goals, away_goals),
            Side::Away => (away_goals, home_goals),
        };
        let total = home_goals + away_goals;
        match self {
            Condition::Win(side) => {
                let (us, them) = goals(side);
                us > them
            }
            Condition::NotLose(side) => {
                let (us, them) = goals(side);
                us >= them
            }
            Condition::Draw => home_goals == away_goals,
            Condition::NoDraw => home_goals != away_goals,
            Condition::BothScore => home_goals > 0 && away_goals > 0,
            Condition::TotalAtLeast(n) => total >= *n,
            Condition::TotalAtMost(n) => total <= *n,
            Condition::WinBy(side, n) => {
                let (us, them) = goals(side);
                us >= them + n
            }
            Condition::WinToNil(side) => {
                let (us, them) = goals(side);
                us > 0 && them == 0
            }
            Condition::Scores(side, scores) => scores.contains(&goals(side)),
            Condition::TeamAtLeast(side, n) => goals(side).0 >= *n,
        }
    }

    /// Famille de pari, pour les tableaux de l'étude.
    fn family(&self) -> &'static str {
        match self {
            Condition::Win(_) => "victoire",
            Condition::NotLose(_) => "double chance",
            Condition::Draw => "nul",
            Condition::NoDraw => "pas de nul",
            Condition::BothScore => "les deux marquent",
            Condition::TotalAtLeast(_) => "plus de buts",
            Condition::TotalAtMost(_) => "moins de buts",
            Condition::WinBy(..) => "victoire par écart",
            Condition::WinToNil(_) => "victoire sans encaisser",
            Condition::Scores(..) => "score exact",
            Condition::TeamAtLeast(..) => "buts d'une équipe",
        }
    }
}

/// Un pari sur le score final : toutes ses conditions doivent être remplies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Conditions, dans l'ordre de l'intitulé.
    pub conditions: Vec<Condition>,
}

impl Selection {
    /// Le pari est-il gagné avec ce score final ?
    pub fn holds(&self, home_goals: u32, away_goals: u32) -> bool {
        self.conditions
            .iter()
            .all(|c| c.holds(home_goals, away_goals))
    }

    /// Famille du pari (« victoire + les deux marquent »).
    pub fn category(&self) -> String {
        let families: Vec<&str> = self.conditions.iter().map(Condition::family).collect();
        families.join(" + ")
    }
}

/// Lit l'intitulé d'un pari sur le match `home`–`away` (noms tels qu'écrits dans l'événement). `None` si une partie
/// de l'intitulé n'est pas reconnue.
pub fn parse(description: &str, home: &str, away: &str) -> Option<Selection> {
    let text = normalize(description, home, away);
    let conditions = text
        .split(" et ")
        .map(|clause| parse_clause(clause.trim()))
        .collect::<Option<Vec<Condition>>>()?;
    // `collect` vers `Option<Vec<_>>` : un seul `None` (une clause inconnue) rend tout le résultat `None`.
    Some(Selection { conditions })
}

/// Met l'intitulé sous une forme canonique : équipes remplacées par `@h` et `@a`, minuscules, formulations
/// équivalentes ramenées à une seule, mentions entre parenthèses et suffixes de Parions Sport retirés.
///
/// Public pour lister, dans l'étude, les tournures les plus fréquentes que [`parse`] ne reconnaît pas.
pub fn normalize(description: &str, home: &str, away: &str) -> String {
    let mut text = clean(description);
    for prefix in ["CB FLASH :", "CB FLASH -", "CB -", "CB :"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim().to_string();
        }
    }
    // « … ? (2,65 => 3,10 / Mise max 25 €) - 90 Mins » : seul ce qui précède le premier « ? » ou « ( » compte.
    if let Some(end) = text.find(['?', '(']) {
        text.truncate(end);
    }
    if let Some(end) = text.find(" - ") {
        text.truncate(end);
    }
    // Le nom le plus long d'abord : « Paris FC » ne doit pas être lu comme « Paris » suivi de « FC ».
    let (long, short) = if home.len() >= away.len() {
        ((home, "@h"), (away, "@a"))
    } else {
        ((away, "@a"), (home, "@h"))
    };
    let mut text = text
        .replace(long.0, long.1)
        .replace(short.0, short.1)
        .to_lowercase();
    for (from, to) in [
        ("les 2 équipes", "les deux équipes"),
        (" remporte le match", " gagne"),
        (" remporte", " gagne"),
        (" s'impose", " gagne"),
        (" gagne le match", " gagne"),
        (" gagne sur le score de", " gagne"),
        (" sans prendre de but", " sans encaisser de but"),
        (" lors du match", ""),
        (" dans le match", ""),
        (" du match", ""),
    ] {
        text = text.replace(from, to);
    }
    text.trim().to_string()
}

/// Une clause de l'intitulé (le texte entre deux « et »).
fn parse_clause(clause: &str) -> Option<Condition> {
    match clause {
        "match nul" => return Some(Condition::Draw),
        "les deux équipes marquent" => return Some(Condition::BothScore),
        "@h ou @a gagne" | "@a ou @h gagne" => return Some(Condition::NoDraw),
        _ => {}
    }
    if let Some(rest) = clause
        .strip_prefix("plus de ")
        .or_else(|| clause.strip_prefix("plus "))
    {
        // Ligne « x,5 » : plus de 2,5 buts = au moins 3.
        return Some(Condition::TotalAtLeast(half_line(goals_count(rest)?)? + 1));
    }
    if let Some(rest) = clause.strip_prefix("moins de ") {
        return Some(Condition::TotalAtMost(half_line(goals_count(rest)?)?));
    }
    if let Some(rest) = clause.strip_prefix("au moins ") {
        return Some(Condition::TotalAtLeast(whole(goals_count(rest)?)?));
    }
    let (side, rest) = if let Some(rest) = clause.strip_prefix("@h ") {
        (Side::Home, rest)
    } else {
        (Side::Away, clause.strip_prefix("@a ")?)
    };
    match rest {
        "gagne" => return Some(Condition::Win(side)),
        "gagne ou fait match nul" | "ne perd pas" => return Some(Condition::NotLose(side)),
        "gagne sans encaisser de but" => return Some(Condition::WinToNil(side)),
        _ => {}
    }
    if let Some(margin) = rest
        .strip_prefix("gagne par au moins ")
        .and_then(|r| r.strip_suffix(" d'écart"))
    {
        return Some(Condition::WinBy(side, whole(goals_count(margin)?)?));
    }
    if let Some(goals) = rest.strip_prefix("marque au moins ") {
        return Some(Condition::TeamAtLeast(side, whole(goals_count(goals)?)?));
    }
    let scores = rest.strip_prefix("gagne ")?;
    Some(Condition::Scores(side, winning_scores(scores)?))
}

/// « 2,5 buts » → 2,5 ; « 1 but » → 1. `None` si ce n'est pas un nombre de buts.
fn goals_count(text: &str) -> Option<f64> {
    let number = text
        .strip_suffix(" buts")
        .or_else(|| text.strip_suffix(" but"))?;
    let value: f64 = number.replace(',', ".").parse().ok()?;
    // Au-delà de 10 buts, ce n'est pas du football (handball, basket…).
    (0.0..=10.0).contains(&value).then_some(value)
}

/// Partie entière d'une ligne `x,5` ; `None` pour une ligne entière (« plus de 2 buts » serait ambigu).
fn half_line(line: f64) -> Option<u32> {
    (line.fract() == 0.5).then_some(line.floor() as u32)
}

/// Nombre entier de buts ; `None` pour une ligne `x,5`.
fn whole(value: f64) -> Option<u32> {
    (value.fract() == 0.0).then_some(value as u32)
}

/// « 1-0, 2-0 ou 3-0 » → scores gagnants écrits (buts du vainqueur, buts du perdant).
///
/// Les intitulés écrivent les scores tantôt dans l'ordre domicile-extérieur (« 0-1, 0-2 ou 0-3 » pour une victoire à
/// l'extérieur), tantôt du point de vue du vainqueur : pour une victoire, le plus grand nombre est forcément celui de
/// l'équipe qui gagne, ce qui lève l'ambiguïté. Un score nul est refusé.
fn winning_scores(text: &str) -> Option<Vec<(u32, u32)>> {
    text.replace(" ou ", ", ")
        .split(", ")
        .map(|score| {
            // « 1-0 » chez Winamax, « 1:0 » chez Parions Sport.
            let (first, second) = score.trim().split_once(['-', ':'])?;
            let (first, second): (u32, u32) = (first.parse().ok()?, second.parse().ok()?);
            (first != second).then_some((first.max(second), first.min(second)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(description: &str) -> Option<Vec<Condition>> {
        parse(description, "Paris SG", "Paris FC").map(|s| s.conditions)
    }

    #[test]
    fn common_winamax_wordings_are_recognised() {
        use Condition::*;
        use Side::*;
        let cases = [
            ("Paris SG gagne", vec![Win(Home)]),
            (
                "Paris FC gagne et les deux équipes marquent",
                vec![Win(Away), BothScore],
            ),
            (
                "Paris SG gagne ou fait match nul et plus de 2,5 buts dans le match",
                vec![NotLose(Home), TotalAtLeast(3)],
            ),
            ("Plus de 3,5 buts lors du match", vec![TotalAtLeast(4)]),
            ("Moins de 1,5 buts lors du match", vec![TotalAtMost(1)]),
            (
                "Paris SG gagne par au moins 2 buts d'écart",
                vec![WinBy(Home, 2)],
            ),
            ("Paris FC gagne sans encaisser de but", vec![WinToNil(Away)]),
            (
                "Paris FC gagne le match 0-1, 0-2 ou 0-3",
                vec![Scores(Away, vec![(1, 0), (2, 0), (3, 0)])],
            ),
            ("Paris SG gagne 2-1", vec![Scores(Home, vec![(2, 1)])]),
            (
                "Paris SG marque au moins 3 buts",
                vec![TeamAtLeast(Home, 3)],
            ),
            (
                "Paris SG ou Paris FC gagne et les deux équipes marquent",
                vec![NoDraw, BothScore],
            ),
            ("Les deux équipes marquent lors du match", vec![BothScore]),
            (
                "Paris SG gagne et plus de 2,5 buts dans le match (hors prol. et TAB)",
                vec![Win(Home), TotalAtLeast(3)],
            ),
        ];
        for (description, expected) in cases {
            assert_eq!(parsed(description), Some(expected), "{description}");
        }
    }

    #[test]
    fn parions_sport_wordings_are_recognised() {
        use Condition::*;
        use Side::*;
        assert_eq!(
            parsed(
                "CB - Paris SG gagne le match et les 2 équipes marquent (2,15 -> 2,50 / Mise max 25 €) - 90 Mins - Oui"
            ),
            Some(vec![Win(Home), BothScore])
        );
        assert_eq!(
            parsed(
                "Paris FC gagne sur le score de 1-0, 2-0 ou 3-0 ? (2,65 => 3,10 / Mise max 25 €)"
            ),
            Some(vec![Scores(Away, vec![(1, 0), (2, 0), (3, 0)])])
        );
        assert_eq!(
            parsed("Paris SG gagne sans prendre de but ?"),
            Some(vec![WinToNil(Home)])
        );
        assert_eq!(
            parsed("Paris SG gagne 1:0, 2:0 ou 3:0 ?"),
            Some(vec![Scores(Home, vec![(1, 0), (2, 0), (3, 0)])])
        );
        assert_eq!(
            parsed("CB - Les 2 équipes marquent et Plus 2,5 buts dans le match"),
            Some(vec![BothScore, TotalAtLeast(3)])
        );
    }

    #[test]
    fn unknown_or_out_of_scope_wordings_are_refused() {
        for description in [
            "Kylian Mbappé buteur et Paris SG gagne",
            "Paris SG gagne à la mi-temps et à la fin du match",
            "Paris SG marque dans les deux mi-temps",
            "Plus de 59,5 buts lors du match",
            "Plus de 2 buts lors du match",
            "Paris SG gagne 1-1",
            "Paris SG gagne et plus de 160,5 points dans le match",
        ] {
            assert_eq!(parsed(description), None, "{description}");
        }
    }

    #[test]
    fn conditions_are_checked_against_the_final_score() {
        let selection =
            parse("Paris FC gagne et plus de 2,5 buts", "Paris SG", "Paris FC").unwrap();
        assert!(selection.holds(1, 2));
        assert!(!selection.holds(0, 2));
        assert!(!selection.holds(2, 1));
        assert_eq!(selection.category(), "victoire + plus de buts");
        let scores = parse(
            "Paris FC gagne le match 0-1, 0-2 ou 0-3",
            "Paris SG",
            "Paris FC",
        )
        .unwrap();
        assert!(scores.holds(0, 2));
        assert!(!scores.holds(1, 2));
    }
}
