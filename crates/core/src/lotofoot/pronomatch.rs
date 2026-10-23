//! Lecture des pages de prono-match.fr : grille en cours (matchs, répartition FDJ, clôture, pactole), rapports des
//! dernières grilles et résultats officiels.
//!
//! Le site reprend la répartition et les rapports publiés par la FDJ (`docs/sources.md` §37). Ses classes CSS sont des
//! identifiants générés qui changent : la lecture s'appuie sur la structure et sur les textes (« grille ouverte »,
//! en-tête « Joueurs », attribut `title` des signes), jamais sur ces classes. Seuls les faits extraits sont conservés,
//! jamais les pages (leurs mentions légales interdisent la reproduction).

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Europe::Paris;
use scraper::{ElementRef, Html, Selector};

use super::Game;
use crate::domain::{Euros, Outcome};
use crate::error::{Error, Result};

/// Racine du site.
pub const BASE_URL: &str = "https://prono-match.fr/";

/// Chemin de la page d'une formule (`loto-foot-7/`).
pub fn grid_path(game: Game) -> String {
    format!("loto-foot-{}/", game.matches())
}

/// Chemin de la page des derniers résultats.
pub const RESULTS_PATH: &str = "resultats-loto-foot/";

/// Grille en cours d'une formule, telle qu'affichée : ouverte aux paris, ou annoncée mais pas encore ouverte.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenGrid {
    /// Formule.
    pub game: Game,
    /// `true` si la grille accepte les prises de jeu (« grille ouverte ») ; `false` si elle est seulement annoncée
    /// (« grille pas encore ouverte aux paris »), sans répartition.
    pub accepting: bool,
    /// Numéro du tirage (« n°133 »).
    pub number: u32,
    /// Fin de validation des prises de jeu, en UTC.
    pub closes_at: DateTime<Utc>,
    /// Gain minimum garanti du rang 1 (« pactole »), s'il est annoncé.
    pub jackpot: Option<Euros>,
    /// Rencontres : équipe à domicile, équipe à l'extérieur, noms bruts du site.
    pub matches: Vec<(String, String)>,
    /// Répartition FDJ en pourcents entiers (1, N, 2) ; `None` tant que la FDJ ne l'a pas publiée (« — »).
    pub split: Option<Vec<[u32; 3]>>,
    /// Heure du relevé de la répartition annoncée par le site, en UTC.
    pub split_at: Option<DateTime<Utc>>,
}

/// Rapports d'une grille passée : un élément par rang, du sans-faute au dernier rang payé.
#[derive(Debug, Clone, PartialEq)]
pub struct PastRanks {
    /// Numéro du tirage.
    pub number: u32,
    /// Rangs : nombre de gagnants et gain unitaire (`None` quand il n'y a « aucun gagnant »).
    pub ranks: Vec<RankLine>,
}

/// Un rang publié.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankLine {
    /// Bons pronostics du rang (7 pour le sans-faute du LF7) ; `None` si la page ne le dit pas (tableau d'historique).
    pub good: Option<u32>,
    /// Nombre de grilles gagnantes.
    pub winners: u32,
    /// Gain unitaire, `None` sans gagnant.
    pub unit_gain: Option<Euros>,
}

/// Page d'une formule : la grille en cours (s'il y en a une) et les rapports des dernières grilles.
#[derive(Debug, Clone, PartialEq)]
pub struct GridPage {
    /// Grille ouverte.
    pub open: Option<OpenGrid>,
    /// Rapports des dernières grilles.
    pub history: Vec<PastRanks>,
}

/// Résultat officiel d'une grille (page des résultats).
#[derive(Debug, Clone, PartialEq)]
pub struct GridResult {
    /// Formule, `None` pour le LF12 et le LF15 (non suivis).
    pub game: Option<Game>,
    /// Numéro du tirage.
    pub number: u32,
    /// Date affichée.
    pub date: Option<NaiveDate>,
    /// Rencontres dans l'ordre : équipes et signe sorti.
    pub matches: Vec<(String, String, Outcome)>,
    /// Rapports, du sans-faute au dernier rang.
    pub ranks: Vec<RankLine>,
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("sélecteur CSS constant valide")
}

/// Texte d'un élément, espaces normalisés (espaces insécables compris).
fn text_of(element: ElementRef<'_>) -> String {
    normalize(&element.text().collect::<Vec<_>>().join(""))
}

fn normalize(text: &str) -> String {
    text.replace(['\u{a0}', '\u{202f}'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lit la page d'une formule. `fetched_at` sert à dater les jours affichés sans année (« samedi 24 octobre »).
pub fn parse_grid_page(html: &str, game: Game, fetched_at: DateTime<Utc>) -> Result<GridPage> {
    let document = Html::parse_document(html);
    let open = parse_open_grid(&document, game, fetched_at)?;
    let history = parse_history(&document);
    Ok(GridPage { open, history })
}

fn parse_open_grid(
    document: &Html,
    game: Game,
    fetched_at: DateTime<Utc>,
) -> Result<Option<OpenGrid>> {
    let Some(head) = document
        .select(&selector("p"))
        .map(text_of)
        .find(|t| t.contains(" n°") && t.contains("· clôture ") && t.contains("grille"))
    else {
        return Ok(None);
    };
    let parts: Vec<&str> = head.split(" · ").map(str::trim).collect();
    let accepting = parts.contains(&"grille ouverte");
    let number = parts
        .first()
        .and_then(|p| p.split("n°").nth(1))
        .and_then(|n| n.trim().parse::<u32>().ok())
        .ok_or_else(|| page_error(&format!("numéro de grille illisible : « {head} »")))?;
    let closes_at = parts
        .iter()
        .find_map(|p| p.strip_prefix("clôture "))
        .and_then(|d| parse_french_datetime(d, fetched_at))
        .ok_or_else(|| page_error(&format!("clôture illisible : « {head} »")))?;
    let jackpot = parts
        .iter()
        .find_map(|p| p.strip_prefix("pactole de "))
        .and_then(|amount| parse_euros(amount.trim_end_matches('.')));
    let split_at = parts
        .iter()
        .find_map(|p| p.strip_prefix("répartition des joueurs relevée "))
        .and_then(|d| parse_french_datetime(d.trim_end_matches('.'), fetched_at));

    let (matches, split) = parse_split_table(document)?;
    if matches.len() != game.matches() {
        return Err(page_error(&format!(
            "{game} n°{number} : {} rencontres lues au lieu de {}",
            matches.len(),
            game.matches()
        )));
    }
    Ok(Some(OpenGrid {
        game,
        accepting,
        number,
        closes_at,
        jackpot,
        matches,
        split,
        split_at,
    }))
}

/// Tableau de la grille en cours (celui dont l'en-tête contient « Joueurs ») : rencontres et répartition.
type SplitTable = (Vec<(String, String)>, Option<Vec<[u32; 3]>>);

fn parse_split_table(document: &Html) -> Result<SplitTable> {
    let table = document
        .select(&selector("table"))
        .find(|t| {
            t.select(&selector("th"))
                .any(|th| text_of(th).starts_with("Joueurs"))
        })
        .ok_or_else(|| page_error("tableau de la grille introuvable"))?;
    let mut matches = Vec::new();
    let mut split = Vec::new();
    let mut published = true;
    for row in table.select(&selector("tbody tr")) {
        let cells: Vec<ElementRef<'_>> = row.select(&selector("td")).collect();
        let (Some(name_cell), Some(split_cell)) = (cells.get(1), cells.get(2)) else {
            continue;
        };
        // Le premier nœud de texte seulement : une note (« le 2 est surjoué… ») peut suivre le nom des équipes.
        let name = normalize(name_cell.text().next().unwrap_or(""));
        let (home, away) = name
            .split_once(" - ")
            .ok_or_else(|| page_error(&format!("rencontre illisible : « {name} »")))?;
        matches.push((home.trim().to_string(), away.trim().to_string()));
        match parse_triplet(&text_of(*split_cell)) {
            Some(triplet) => split.push(triplet),
            None => published = false,
        }
    }
    Ok((matches, published.then_some(split)))
}

/// « 4·9·87 » → [4, 9, 87] ; `None` pour « — » ou toute autre forme.
fn parse_triplet(text: &str) -> Option<[u32; 3]> {
    let values: Vec<u32> = text
        .split('·')
        .map(|v| v.trim().parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    <[u32; 3]>::try_from(values).ok()
}

/// Tableau des rapports des dernières grilles : lignes dont la première cellule est « n°… ».
fn parse_history(document: &Html) -> Vec<PastRanks> {
    let mut out = Vec::new();
    for row in document.select(&selector("tr")) {
        let cells: Vec<ElementRef<'_>> = row.select(&selector("td")).collect();
        let Some(number) = cells
            .first()
            .and_then(|c| text_of(*c).strip_prefix("n°").map(str::to_string))
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let ranks: Vec<RankLine> = cells
            .iter()
            .skip(2)
            .filter_map(|cell| parse_history_cell(&text_of(*cell)))
            .collect();
        if !ranks.is_empty() && out.iter().all(|p: &PastRanks| p.number != number) {
            out.push(PastRanks { number, ranks });
        }
    }
    out
}

/// « 578 € 135 gagnants » ou « aucun gagnant » ; `None` pour une cellule vide.
fn parse_history_cell(text: &str) -> Option<RankLine> {
    if text.contains("aucun gagnant") {
        return Some(RankLine {
            good: None,
            winners: 0,
            unit_gain: None,
        });
    }
    let (amount, rest) = text.split_once('€')?;
    Some(RankLine {
        good: None,
        winners: parse_count(rest)?,
        unit_gain: parse_euros(amount),
    })
}

/// Lit la page des résultats : une grille par bloc « Tirage N° … », rencontres avec le signe sorti, rapports.
pub fn parse_results_page(html: &str) -> Result<Vec<GridResult>> {
    let document = Html::parse_document(html);
    let mut out: Vec<GridResult> = Vec::new();
    // Parcours de tous les éléments dans l'ordre du document : un en-tête ouvre un bloc, les rencontres et les rangs qui
    // suivent lui appartiennent.
    for element in document
        .root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
    {
        let name = element.value().name();
        let own = text_of(element);
        if name == "b" && own.starts_with("Tirage N°") {
            let number = own
                .trim_start_matches("Tirage N°")
                .trim()
                .parse::<u32>()
                .map_err(|_| page_error(&format!("tirage illisible : « {own} »")))?;
            let header = element
                .parent()
                .and_then(|p| p.parent())
                .and_then(ElementRef::wrap);
            let header_text = header.map(text_of).unwrap_or_default();
            let size = header_text.split("Tirage").next().and_then(|h| {
                h.trim()
                    .trim_start_matches("Loto Foot")
                    .trim()
                    .parse::<usize>()
                    .ok()
            });
            let date = element
                .next_siblings()
                .filter_map(ElementRef::wrap)
                .map(text_of)
                .find_map(|t| NaiveDate::parse_from_str(&t, "%d/%m/%Y").ok());
            out.push(GridResult {
                game: size.and_then(Game::from_matches),
                number,
                date,
                matches: Vec::new(),
                ranks: Vec::new(),
            });
        } else if let Some(sign) = element.value().attr("title").and_then(sign_from_title) {
            let home = element
                .prev_siblings()
                .filter_map(ElementRef::wrap)
                .next()
                .map(text_of);
            let away = element
                .next_siblings()
                .filter_map(ElementRef::wrap)
                .next()
                .map(text_of);
            if let (Some(block), Some(home), Some(away)) = (out.last_mut(), home, away) {
                block.matches.push((home, away, sign));
            }
        } else if element.value().classes().any(|c| c == "rk") {
            let good = own.trim_end_matches("bons").trim().parse::<u32>().ok();
            let mut next = element
                .next_siblings()
                .filter_map(ElementRef::wrap)
                .map(text_of);
            let amount = next.next();
            let winners = next.next();
            if let (Some(block), Some(amount), Some(winners)) = (out.last_mut(), amount, winners) {
                let winners = if winners.contains("aucun") {
                    Some(0)
                } else {
                    parse_count(&winners)
                };
                if let Some(winners) = winners {
                    block.ranks.push(RankLine {
                        good,
                        winners,
                        unit_gain: if winners == 0 {
                            None
                        } else {
                            parse_euros(&amount)
                        },
                    });
                }
            }
        }
    }
    Ok(out)
}

fn sign_from_title(title: &str) -> Option<Outcome> {
    match title {
        "Victoire domicile" => Some(Outcome::Home),
        "Match nul" => Some(Outcome::Draw),
        "Victoire extérieur" => Some(Outcome::Away),
        _ => None,
    }
}

/// « 500 000,00 € » ou « 2 769 » → montant ; les espaces de groupement (insécables compris) sont ignorés.
fn parse_euros(text: &str) -> Option<Euros> {
    let digits: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == ',' || *c == '.')
        .collect();
    let value = digits.replace(',', ".").parse::<f64>().ok()?;
    Euros::new(value)
}

/// « 1 342 gagnants » → 1342.
fn parse_count(text: &str) -> Option<u32> {
    let digits: String = text
        .chars()
        .take_while(|c| !c.is_alphabetic())
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

const MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

/// « samedi 24 octobre à 20h40 » (heure de Paris) → instant UTC. L'année n'est pas affichée : on retient celle qui
/// place la date au plus près de `reference` (l'instant du relevé).
fn parse_french_datetime(text: &str, reference: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let words: Vec<&str> = text.split_whitespace().collect();
    // Le jour de la semaine est facultatif ; « 1er » vaut 1.
    let start = usize::from(
        words
            .first()
            .is_some_and(|w| w.parse::<u32>().is_err() && !w.ends_with("er")),
    );
    let day: u32 = words.get(start)?.trim_end_matches("er").parse().ok()?;
    let month = MONTHS
        .iter()
        .position(|m| Some(m) == words.get(start + 1))?
        + 1;
    // « 17h55. » : en fin de phrase (sans pactole derrière), l'heure porte le point final.
    let time = words.get(start + 3)?.trim_end_matches('.');
    let (hour, minute) = time.split_once('h')?;
    let (hour, minute): (u32, u32) = (
        hour.parse().ok()?,
        if minute.is_empty() {
            0
        } else {
            minute.parse().ok()?
        },
    );
    let month = u32::try_from(month).ok()?;
    let year = reference.with_timezone(&Paris).year();
    (year - 1..=year + 1)
        .filter_map(|y| {
            let naive = NaiveDate::from_ymd_opt(y, month, day)?.and_hms_opt(hour, minute, 0)?;
            paris_to_utc(naive)
        })
        .min_by_key(|t| (*t - reference).num_seconds().abs())
}

/// Heure de Paris → UTC. Lors du passage à l'heure d'hiver, une heure existe deux fois : on prend la plus tôt.
fn paris_to_utc(naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    Paris
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

fn page_error(message: &str) -> Error {
    Error::Data(format!("page prono-match : {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pages SYNTHÉTIQUES, écrites pour les tests sur le modèle de la structure du site (aucune copie de ses pages).
    const GRID_PAGE: &str = r#"<html><body>
<p class="x1"><b>Loto Foot 7 n°133</b> · grille ouverte · clôture dimanche 25 octobre à 17h10 · pactole de 100&nbsp;000 € · répartition des joueurs relevée samedi 24 octobre à 11h55.</p>
<table class="x2"><thead><tr><th>#</th><th>Match</th><th>Joueurs<br><small>1·N·2</small></th><th>Modèle</th><th>Choix</th></tr></thead><tbody>
<tr><td>1</td><td>Equipe A - Equipe B</td><td><b>50</b><span>·</span>30<span>·</span>20</td><td>—</td><td>1</td></tr>
<tr><td>2</td><td>Equipe C - Equipe D<small>le 2 est surjoué : 87 % des joueurs</small></td><td>4<span>·</span>9<span>·</span><b>87</b></td><td>—</td><td>2</td></tr>
<tr><td>3</td><td>E - F</td><td>33·33·34</td><td>—</td><td>1</td></tr>
<tr><td>4</td><td>G - H</td><td>40·30·30</td><td>—</td><td>1</td></tr>
<tr><td>5</td><td>I - J</td><td>40·30·30</td><td>—</td><td>1</td></tr>
<tr><td>6</td><td>K - L</td><td>40·30·30</td><td>—</td><td>1</td></tr>
<tr><td>7</td><td>M - N</td><td>40·30·30</td><td>—</td><td>1</td></tr>
</tbody></table>
<table><tbody>
<tr><td>n°132</td><td>jeudi 22 octobre</td><td><b>578 €</b><br><small>135 gagnants</small></td><td><b>39,80 €</b><br><small>2&nbsp;393 gagnants</small></td><td></td></tr>
<tr><td>n°129</td><td>jeudi 15 octobre</td><td><span>aucun gagnant</span></td><td><b>4,20 €</b><br><small>17631 gagnants</small></td><td></td></tr>
</tbody></table></body></html>"#;

    const RESULTS_PAGE: &str = r#"<html><body>
<div><div><span>Loto Foot</span><span>7</span><div><b>Tirage N° 132</b><span>22/10/2026</span></div></div>
<div><div><span>Equipe A</span><span title="Victoire domicile">1</span><span>Equipe B</span></div>
<div><span>Equipe C</span><span title="Match nul">N</span><span>Equipe D</span></div></div>
<div>Rapports</div><div><div><span class="rk">7<em>bons</em></span><span>578,00 €</span><span class="win">135 gagnants</span></div>
<div><span class="rk">6<em>bons</em></span><span>39,80 €</span><span class="win">2 393 gagnants</span></div></div></div>
<div><div><span>Loto Foot</span><span>15</span><div><b>Tirage N° 88</b><span>24/10/2026</span></div></div>
<div><div><span>X</span><span title="Victoire extérieur">2</span><span>Y</span></div></div></div>
</body></html>"#;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn the_open_grid_is_read_with_its_split_close_and_jackpot() {
        let page = parse_grid_page(GRID_PAGE, Game::Lf7, at("2026-10-24T10:00:00Z")).unwrap();
        let grid = page.open.unwrap();
        assert!(grid.accepting);
        assert_eq!(grid.number, 133);
        // 17 h 10 à Paris le 25 octobre 2026 (heure d'hiver depuis la nuit) = 16 h 10 UTC.
        assert_eq!(grid.closes_at, at("2026-10-25T16:10:00Z"));
        assert_eq!(grid.jackpot, Euros::new(100_000.0));
        assert_eq!(grid.split_at, Some(at("2026-10-24T09:55:00Z")));
        assert_eq!(
            grid.matches[1],
            ("Equipe C".to_string(), "Equipe D".to_string())
        );
        let split = grid.split.unwrap();
        assert_eq!(split[0], [50, 30, 20]);
        assert_eq!(split[1], [4, 9, 87]);
    }

    #[test]
    fn an_unpublished_split_is_none_not_an_error() {
        let page = GRID_PAGE.replace("33·33·34", "<span>—</span>");
        let grid = parse_grid_page(&page, Game::Lf7, at("2026-10-24T10:00:00Z"))
            .unwrap()
            .open
            .unwrap();
        assert!(grid.split.is_none());
    }

    #[test]
    fn an_announced_grid_is_read_as_not_yet_accepting() {
        let page = GRID_PAGE
            .replace("grille ouverte", "grille pas encore ouverte aux paris")
            .replace(
                " · répartition des joueurs relevée samedi 24 octobre à 11h55.",
                ".",
            )
            .replace("40·30·30", "—");
        let grid = parse_grid_page(&page, Game::Lf7, at("2026-10-24T10:00:00Z"))
            .unwrap()
            .open
            .unwrap();
        assert!(!grid.accepting);
        assert_eq!(grid.jackpot, Euros::new(100_000.0));
        assert!(grid.split.is_none());
        assert!(grid.split_at.is_none());
    }

    #[test]
    fn a_grid_with_the_wrong_number_of_matches_is_refused() {
        assert!(parse_grid_page(GRID_PAGE, Game::Lf8, at("2026-10-24T10:00:00Z")).is_err());
    }

    #[test]
    fn past_ranks_are_read_from_the_history_table() {
        let page = parse_grid_page(GRID_PAGE, Game::Lf7, at("2026-10-24T10:00:00Z")).unwrap();
        assert_eq!(page.history.len(), 2);
        assert_eq!(page.history[0].number, 132);
        assert_eq!(page.history[0].ranks[1].winners, 2393);
        assert_eq!(page.history[0].ranks[1].unit_gain, Euros::new(39.8));
        assert_eq!(page.history[1].ranks[0].winners, 0);
        assert_eq!(page.history[1].ranks[0].unit_gain, None);
    }

    #[test]
    fn results_are_grouped_by_draw_with_signs_and_ranks() {
        let results = parse_results_page(RESULTS_PAGE).unwrap();
        assert_eq!(results.len(), 2);
        let lf7 = &results[0];
        assert_eq!(lf7.game, Some(Game::Lf7));
        assert_eq!(lf7.number, 132);
        assert_eq!(lf7.date, NaiveDate::from_ymd_opt(2026, 10, 22));
        assert_eq!(lf7.matches[1].2, Outcome::Draw);
        assert_eq!(
            lf7.ranks[0],
            RankLine {
                good: Some(7),
                winners: 135,
                unit_gain: Euros::new(578.0)
            }
        );
        assert_eq!(lf7.ranks[1].winners, 2393);
        assert_eq!(results[1].game, None);
    }

    /// Contrôle sur de vraies pages, hors dépôt : `EDGE_LOTOFOOT_PAGES=<dossier> cargo test -- --ignored real_pages`.
    /// Le dossier contient des pages `lf7_<horodatage>Z.html`, `lf8_…` et `res_…` (relevé d'étude de prono-match).
    #[test]
    #[ignore = "lit des pages réelles hors dépôt"]
    fn real_pages_are_read_without_error() {
        let Ok(dir) = std::env::var("EDGE_LOTOFOOT_PAGES") else {
            return;
        };
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let html = std::fs::read_to_string(&path).unwrap();
            let stamp = name.split('_').nth(1).unwrap().trim_end_matches(".html");
            let fetched = NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ")
                .unwrap()
                .and_utc();
            if let Some(game) = Game::from_code(name.split('_').next().unwrap()) {
                let page = parse_grid_page(&html, game, fetched)
                    .unwrap_or_else(|e| panic!("{name} : {e}"));
                let open = page.open.as_ref().map(|g| {
                    (
                        g.number,
                        g.accepting,
                        g.closes_at,
                        g.jackpot,
                        g.split.is_some(),
                    )
                });
                println!(
                    "{name} : grille {open:?}, {} rapports passés",
                    page.history.len()
                );
                assert!(!page.history.is_empty(), "{name} : aucun rapport passé");
            } else if name.starts_with("res_") {
                let results = parse_results_page(&html).unwrap_or_else(|e| panic!("{name} : {e}"));
                for r in &results {
                    println!(
                        "{name} : {:?} n°{} {:?} {} rencontres, {} rangs",
                        r.game,
                        r.number,
                        r.date,
                        r.matches.len(),
                        r.ranks.len()
                    );
                }
                assert!(results.iter().any(|r| r.game == Some(Game::Lf7)
                    && r.matches.len() == 7
                    && r.ranks.len() == 2));
            }
            checked += 1;
        }
        assert!(checked > 0);
    }

    #[test]
    fn french_dates_pick_the_nearest_year() {
        let reference = at("2026-12-30T12:00:00Z");
        // Le 2 janvier affiché fin décembre est celui de l'année suivante.
        assert_eq!(
            parse_french_datetime("samedi 2 janvier à 15h00", reference),
            Some(at("2027-01-02T14:00:00Z"))
        );
        assert_eq!(
            parse_french_datetime("jeudi 1er octobre à 20h55", at("2026-10-01T10:00:00Z")),
            Some(at("2026-10-01T18:55:00Z"))
        );
        // En-tête du LF8 n°147 du 23 octobre, sans pactole : la clôture finit la phrase.
        assert_eq!(
            parse_french_datetime("samedi 24 octobre à 17h55.", at("2026-10-23T21:00:00Z")),
            Some(at("2026-10-24T15:55:00Z"))
        );
    }
}
