//! Lecture de resultats-fdj.fr, le repli du Loto Foot quand le site des points de vente FDJ ne répond pas ([`super::fdj`]).
//!
//! Site d'un particulier, « ni affilié, ni partenaire » de la FDJ ; ses mentions légales interdisent la reproduction, donc
//! seuls les faits extraits sont conservés, jamais les pages (`docs/sources.md` §74). Les classes CSS `lf-*` et `lr-*` ont
//! un sens (grille, ligne, équipe…) mais le site peut les renommer : la lecture s'appuie d'abord sur la structure, les
//! attributs `data-*` et les textes, et ne prend une classe que pour départager des éléments voisins.
//!
//! Trois pages se lisent : l'accueil (grilles ouvertes et répartition), la liste des résultats (liens vers le détail de
//! chaque grille) et le détail d'une grille (signes, rapports). Ce module ne fait aucun accès réseau.

use std::cmp::Reverse;

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Europe::Paris;
use scraper::{ElementRef, Html, Selector};

use super::Game;
use super::grid_page::{
    GridResult, MatchResult, OpenGrid, RankLine, published_ranks, published_split,
};
use crate::domain::{Euros, Outcome};
use crate::error::{Error, Result};

/// Racine du site.
pub const BASE_URL: &str = "https://resultats-fdj.fr/";

/// Chemin de l'accueil du Loto Foot : grilles ouvertes et répartition des joueurs.
pub const HOME_PATH: &str = "loto-foot/";

/// Chemin de la liste des résultats (première page ; les suivantes s'obtiennent par [`results_path`]).
pub const RESULTS_PATH: &str = "loto-foot/resultats";

/// Chemin de la page `page` de la liste des résultats, numérotée à partir de 1 (`?page=N`) ; la première page n'a pas de
/// paramètre, comme le site la sert.
pub fn results_path(page: u32) -> String {
    if page <= 1 {
        RESULTS_PATH.to_string()
    } else {
        format!("{RESULTS_PATH}?page={page}")
    }
}

/// Une ligne de la liste des résultats : la grille et le lien vers sa page de détail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultLink {
    /// Formule (LF7 ou LF8 ; les autres lignes de la liste sont ignorées).
    pub game: Game,
    /// Numéro du tirage.
    pub number: u32,
    /// Lien tel que la page le donne, par exemple `/loto-foot/grille/4957-loto-foot-7-n140`.
    pub href: String,
}

impl ResultLink {
    /// Chemin à ajouter à [`BASE_URL`] : le lien sans sa barre initiale.
    pub fn path(&self) -> &str {
        self.href.trim_start_matches('/')
    }
}

fn page_error(message: &str) -> Error {
    Error::Data(format!("page resultats-fdj.fr : {message}"))
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("sélecteur CSS constant valide")
}

/// Texte d'un élément, espaces normalisés.
///
/// `split_whitespace` coupe aussi sur l'espace insécable (U+00A0) et l'espace fine insécable (U+202F), que le site met
/// entre les milliers et avant `€` : « 1 883 » (avec U+202F) devient « 1 883 » avec une espace ordinaire.
fn text_of(element: ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Premier élément qui correspond à `css` sous `parent`.
///
/// La durée de vie `'a` dit que l'élément rendu emprunte la **page** (comme `parent`), non le sélecteur `css`, qui n'est
/// qu'un texte transitoire : l'élément reste utilisable après le retour de la fonction.
fn first<'a>(parent: ElementRef<'a>, css: &str) -> Option<ElementRef<'a>> {
    parent.select(&selector(css)).next()
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

/// « dimanche 15 novembre 2026 à 14h55 » (heure de Paris) → instant UTC.
///
/// Le site écrit l'année, ce que prono-match ne faisait pas : aucune année n'est à deviner. Le jour de la semaine est
/// ignoré. Lors du passage à l'heure d'hiver, une heure existe deux fois : on prend la plus tôt, celle où la grille
/// ferme le plus tôt. Une heure qui n'existe pas (saut du printemps) donne `None`.
fn parse_french_datetime(text: &str) -> Option<DateTime<Utc>> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (at, month) = words.iter().enumerate().find_map(|(i, word)| {
        Some((
            i,
            u32::try_from(MONTHS.iter().position(|m| m == word)?).ok()? + 1,
        ))
    })?;
    // « 1er » vaut 1 ; le jour précède le mois, l'année le suit.
    let day: u32 = words
        .get(at.checked_sub(1)?)?
        .trim_end_matches("er")
        .parse()
        .ok()?;
    let year: i32 = words.get(at + 1)?.parse().ok()?;
    // L'heure est le mot « 14h55 » : « dimanche » contient aussi un « h », il faut y trouver deux nombres.
    let (hour, minute) = words.iter().find_map(|word| {
        let (hour, minute) = word.split_once('h')?;
        Some((hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?))
    })?;
    let time = NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, 0)?;
    paris_to_utc(time)
}

/// Heure de Paris → UTC ; la plus tôt des deux lectures possibles lors du retour à l'heure d'hiver.
fn paris_to_utc(local: NaiveDateTime) -> Option<DateTime<Utc>> {
    Paris
        .from_local_datetime(&local)
        .earliest()
        .map(|at| at.with_timezone(&Utc))
}

/// « Loto Foot 8 n°158 » → (formule, numéro) = (8, 158).
fn parse_title(title: &str) -> Option<(usize, u32)> {
    let (formula, number) = title.strip_prefix("Loto Foot ")?.split_once(" n°")?;
    Some((formula.trim().parse().ok()?, number.trim().parse().ok()?))
}

/// Contrôle que le lien `/loto-foot/grille/855-loto-foot-8-n158` désigne la grille annoncée (LF8 n°158) : la formule et le
/// numéro lus dans un texte doivent s'accorder avec ceux de l'adresse. Jamais d'appariement approximatif en silence.
fn check_href(href: &str, game: Game, number: u32) -> Result<()> {
    if href.ends_with(&format!("-loto-foot-{}-n{number}", game.matches())) {
        return Ok(());
    }
    Err(page_error(&format!(
        "le lien « {href} » ne correspond pas à {game} n°{number}"
    )))
}

/// Équipe à domicile et équipe à l'extérieur d'une ligne `div.lf-row`.
///
/// Le nom de l'extérieur peut porter la mention « (match annulé) » (dans un `<em>`), qui n'est pas une partie du nom :
/// l'archive compare les noms bruts d'une source à l'autre, la mention ferait croire à une autre équipe.
fn row_teams(row: ElementRef<'_>) -> Option<(String, String)> {
    let home = text_of(first(row, "span.lf-team.lf-home")?);
    let away = text_of(first(row, "span.lf-team:not(.lf-home)")?);
    let away = away
        .trim_end_matches("(match annulé)")
        .trim_end()
        .to_string();
    Some((home, away))
}

/// « 90 % » → 90 ; `None` au-delà de 100 ou si le texte n'est pas un pourcentage entier.
fn parse_percent(text: &str) -> Option<u32> {
    let value: u32 = text.trim_end_matches('%').trim().parse().ok()?;
    (value <= 100).then_some(value)
}

/// Répartition (1, N, 2) d'une ligne de l'accueil, lue **par libellé** : chaque cellule `span[data-l]` est suivie, dans
/// la même boîte, d'un `span.lf-pct`.
fn row_split(row: ElementRef<'_>) -> Option<[u32; 3]> {
    let mut triple = [0; 3];
    for (slot, label) in triple.iter_mut().zip(["1", "N", "2"]) {
        let cell = first(row, &format!("span[data-l=\"{label}\"]"))?;
        let cellbox = cell.parent().and_then(ElementRef::wrap)?;
        *slot = parse_percent(&text_of(first(cellbox, "span.lf-pct")?))?;
    }
    Some(triple)
}

/// Lit l'accueil du Loto Foot : une [`OpenGrid`] par grille LF7 ou LF8 ouverte.
///
/// Seules les grilles ouvertes y figurent : `accepting` vaut toujours `true`. Le pactole n'y est pas lu : aucune grille
/// ouverte n'en portait quand le site a été étudié (`docs/sources.md` §74), donc on ne sait pas où il s'écrit, et il vaut
/// `None` plutôt qu'une valeur devinée. La répartition est celle de la lecture (`split_at` = `fetched_at`) : le site ne
/// donne pas d'heure de relevé, et elle a pu bouger d'un point depuis (§74).
pub fn parse_open_page(html: &str, fetched_at: DateTime<Utc>) -> Result<Vec<OpenGrid>> {
    let document = Html::parse_document(html);
    let section = first(document.root_element(), "section.lf-cards-open")
        .ok_or_else(|| page_error("section des grilles ouvertes introuvable"))?;
    section
        .select(&selector("article.lf-grid"))
        .filter_map(|article| open_grid(article, fetched_at).transpose())
        .collect()
}

/// Une grille de l'accueil ; `None` pour une formule non suivie (LF12, LF15).
fn open_grid(article: ElementRef<'_>, fetched_at: DateTime<Utc>) -> Result<Option<OpenGrid>> {
    let link =
        first(article, "h2 a[href]").ok_or_else(|| page_error("titre de grille introuvable"))?;
    let title = text_of(link);
    let (formula, number) =
        parse_title(&title).ok_or_else(|| page_error(&format!("titre illisible : « {title} »")))?;
    let Some(game) = Game::from_matches(formula) else {
        return Ok(None);
    };
    check_href(link.value().attr("href").unwrap_or_default(), game, number)?;
    let label = format!("{game} n°{number}");
    let deadline = first(article, "div.lf-deadline strong")
        .map(text_of)
        .and_then(|text| parse_french_datetime(&text))
        .ok_or_else(|| page_error(&format!("{label} : fin de validation illisible")))?;

    let mut matches = Vec::new();
    let mut split = Vec::new();
    // Les lignes de rencontre portent `data-pos` ; l'en-tête du tableau et les lignes de cotes n'en ont pas.
    for row in article.select(&selector("div[data-pos]")) {
        let (home, away) = row_teams(row)
            .ok_or_else(|| page_error(&format!("{label} : équipes d'une rencontre illisibles")))?;
        let shares = row_split(row).ok_or_else(|| {
            page_error(&format!(
                "{label} : répartition de {home} - {away} illisible"
            ))
        })?;
        matches.push((home, away));
        split.push(shares);
    }
    if matches.is_empty() {
        return Err(page_error(&format!("{label} : aucune rencontre lue")));
    }
    Ok(Some(OpenGrid {
        game,
        accepting: true,
        number,
        closes_at: deadline,
        jackpot: None,
        matches,
        split: published_split(split),
        split_at: Some(fetched_at),
    }))
}

/// Lit une page de la liste des résultats : un [`ResultLink`] par grille LF7 ou LF8, dans l'ordre de la page (clôture
/// décroissante). Les autres formules sont ignorées ; une page au-delà de la dernière a un tableau vide, donc aucun lien.
pub fn parse_results_list(html: &str) -> Result<Vec<ResultLink>> {
    let document = Html::parse_document(html);
    let table = first(document.root_element(), "table.lf-list")
        .ok_or_else(|| page_error("tableau des résultats introuvable"))?;
    let mut links = Vec::new();
    for row in table.select(&selector("tbody tr")) {
        let cell = first(row, "td[data-l=\"Grille\"]")
            .ok_or_else(|| page_error("ligne de résultat sans cellule « Grille »"))?;
        let product = first(cell, "span.lr-prod").map(text_of).unwrap_or_default();
        let number = first(cell, "strong").map(text_of).unwrap_or_default();
        // « LF 8 » → 8, « n°157 » → 157.
        let (Some(formula), Some(number)) = (
            product
                .strip_prefix("LF")
                .and_then(|n| n.trim().parse::<usize>().ok()),
            number
                .strip_prefix("n°")
                .and_then(|n| n.trim().parse::<u32>().ok()),
        ) else {
            return Err(page_error(&format!(
                "grille illisible dans la liste : « {product} {number} »"
            )));
        };
        let Some(game) = Game::from_matches(formula) else {
            continue;
        };
        let href = first(row, "a[href]")
            .and_then(|link| link.value().attr("href"))
            .ok_or_else(|| {
                page_error(&format!("{game} n°{number} : lien vers le détail absent"))
            })?;
        check_href(href, game, number)?;
        links.push(ResultLink {
            game,
            number,
            href: href.to_string(),
        });
    }
    Ok(links)
}

/// Résultat d'une rencontre d'après l'attribut `data-res` : `1`, `N`, `2`, ou `A` pour une rencontre annulée.
/// `Ok(None)` : pas encore de résultat (attribut vide). `Err` : valeur inconnue.
fn parse_result(shown: &str) -> std::result::Result<Option<MatchResult>, String> {
    match shown {
        "" => Ok(None),
        "1" => Ok(Some(MatchResult::Sign(Outcome::Home))),
        "N" => Ok(Some(MatchResult::Sign(Outcome::Draw))),
        "2" => Ok(Some(MatchResult::Sign(Outcome::Away))),
        // Art. 10.6 du règlement Loto Sports v6 : une rencontre annulée est bonne pour tous les signes (§74).
        "A" => Ok(Some(MatchResult::Canceled)),
        other => Err(other.to_string()),
    }
}

/// « 1 883 » → 1883 ; les marques d'absence de gagnant (« 0 », tiret, « aucun ») donnent 0.
///
/// Un rang sans gagnant n'a pas été vu sur ce site : ces formes sont celles qu'une absence de gagnant prendrait
/// vraisemblablement. Tout autre texte reste refusé, pour qu'une nouvelle présentation soit signalée, non devinée.
fn parse_winners(text: &str) -> Option<u32> {
    let compact: String = text.split_whitespace().collect();
    if let Ok(count) = compact.parse() {
        return Some(count);
    }
    matches!(
        compact.to_lowercase().as_str(),
        "–" | "—" | "-" | "aucun" | "aucungagnant"
    )
    .then_some(0)
}

/// « 2,60 € » → 2,60 (virgule décimale, espaces de groupement ignorés).
fn parse_euros(text: &str) -> Option<Euros> {
    let number: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, ',' | '.'))
        .collect();
    Euros::new(number.replace(',', ".").parse().ok()?)
}

/// Une ligne du tableau des rapports : « 7 sur 7 », gagnants, gain.
fn rank_row(row: ElementRef<'_>, label: &str) -> Result<RankLine> {
    let good = first(row, "td strong")
        .map(text_of)
        .and_then(|text| text.split(' ').next()?.parse::<u32>().ok())
        .ok_or_else(|| {
            page_error(&format!(
                "{label} : rang illisible dans le tableau des rapports"
            ))
        })?;
    let winners = first(row, "td.num")
        .map(text_of)
        .and_then(|text| parse_winners(&text))
        .ok_or_else(|| page_error(&format!("{label} : gagnants du rang {good} illisibles")))?;
    // Sans gagnant, il n'y a pas de gain unitaire : la cellule n'est pas lue.
    let unit_gain = if winners == 0 {
        None
    } else {
        let gain = first(row, "td.lf-gain")
            .map(text_of)
            .and_then(|text| parse_euros(&text))
            .ok_or_else(|| page_error(&format!("{label} : gain du rang {good} illisible")))?;
        Some(gain)
    };
    Ok(RankLine {
        good: Some(good),
        winners,
        unit_gain,
    })
}

/// Lit la page de détail d'une grille : noms, signes et rapports.
///
/// Renvoie `Ok(None)` quand il n'y a rien à écrire encore : formule non suivie, ou résultat incomplet (une rencontre au
/// `data-res` vide, ou provisoire, c'est-à-dire déduite d'un score en cours, `data-prov` non vide). Une page sans tableau
/// de rapports est une erreur : un résultat complet doit en avoir un. Des rangs tous sans gagnant passent pour un rapport
/// pas encore publié et ne sont pas gardés (`grid_page::published_ranks`).
pub fn parse_result_detail(html: &str) -> Result<Option<GridResult>> {
    let document = Html::parse_document(html);
    // Le titre de la grille est dans l'en-tête de l'article : un autre `h2` de la page (« … en bref ») ne doit pas servir.
    let article = first(document.root_element(), "article.lf-grid")
        .ok_or_else(|| page_error("grille introuvable dans la page de détail"))?;
    let title = first(article, "header h2")
        .map(text_of)
        .ok_or_else(|| page_error("titre de la grille introuvable"))?;
    let (formula, number) =
        parse_title(&title).ok_or_else(|| page_error(&format!("titre illisible : « {title} »")))?;
    let Some(game) = Game::from_matches(formula) else {
        return Ok(None);
    };
    let label = format!("{game} n°{number}");
    let closes_at = article
        .select(&selector("header span"))
        .map(text_of)
        .find_map(|text| {
            let deadline = text.strip_prefix("Fin de validation")?;
            parse_french_datetime(deadline.trim_start_matches([' ', ':']))
        })
        .ok_or_else(|| page_error(&format!("{label} : fin de validation illisible")))?;

    let Some(matches) = result_rows(article, &label)? else {
        return Ok(None);
    };
    let table = first(article, "div.draw-ranks-box table")
        .ok_or_else(|| page_error(&format!("{label} : tableau des rapports introuvable")))?;
    let mut ranks = table
        .select(&selector("tbody tr"))
        .map(|row| rank_row(row, &label))
        .collect::<Result<Vec<_>>>()?;
    if ranks.is_empty() {
        return Err(page_error(&format!("{label} : tableau des rapports vide")));
    }
    ranks.sort_by_key(|rank| Reverse(rank.good));
    Ok(Some(GridResult {
        game: Some(game),
        number,
        date: Some(closes_at.with_timezone(&Paris).date_naive()),
        closes_at: Some(closes_at),
        matches,
        ranks: published_ranks(ranks),
    }))
}

/// Rencontres d'une page de détail (`div[data-pos][data-res]`), ou `None` si l'une n'a pas de résultat définitif.
fn result_rows(
    article: ElementRef<'_>,
    label: &str,
) -> Result<Option<Vec<(String, String, MatchResult)>>> {
    let mut matches = Vec::new();
    let mut complete = true;
    for (i, row) in article
        .select(&selector("div[data-pos][data-res]"))
        .enumerate()
    {
        // Les signes s'écrivent par position : une ligne manquante ou répétée décalerait toutes les suivantes.
        if row
            .value()
            .attr("data-pos")
            .and_then(|pos| pos.parse::<usize>().ok())
            != Some(i + 1)
        {
            return Err(page_error(&format!("{label} : rencontres mal numérotées")));
        }
        let (home, away) = row_teams(row).ok_or_else(|| {
            page_error(&format!(
                "{label} : équipes de la rencontre {} illisibles",
                i + 1
            ))
        })?;
        let shown = row.value().attr("data-res").unwrap_or_default().trim();
        let provisional = !row
            .value()
            .attr("data-prov")
            .unwrap_or_default()
            .trim()
            .is_empty();
        match parse_result(shown) {
            Ok(Some(result)) => matches.push((home, away, result)),
            Ok(None) => complete = false,
            Err(other) => {
                return Err(page_error(&format!(
                    "{label} : résultat « {other} » inconnu à la rencontre {}",
                    i + 1
                )));
            }
        }
        complete &= !provisional;
    }
    if matches.is_empty() && complete {
        return Err(page_error(&format!("{label} : aucune rencontre lue")));
    }
    Ok(complete.then_some(matches))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pages SYNTHÉTIQUES, écrites pour les tests sur le modèle de la structure du site (aucune copie de ses pages).

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// Une ligne de l'accueil : équipes et répartition dans l'ordre 1, N, 2.
    fn open_row(pos: usize, home: &str, away: &str, split: [&str; 3]) -> String {
        let cell = |label: &str, pct: &str| {
            format!(
                r#"<div class="lf-cellbox"><span data-l="{label}" class="lf-cell">{label}</span><span class="lf-pct">{pct}</span></div>"#
            )
        };
        format!(
            r#"<div class="lf-row" data-pos="{pos}" data-res="" data-prov=""><span class="lf-num">{pos}</span><span class="lf-team lf-home">{home}</span><div class="lf-cells">{}{}{}</div><span class="lf-team">{away}</span></div>
<div class="lf-cotes"><span class="lf-cotes-lbl">Parions Sport</span></div>"#,
            cell("1", split[0]),
            cell("N", split[1]),
            cell("2", split[2]),
        )
    }

    fn open_article(title: &str, href: &str, deadline: &str, rows: &str) -> String {
        format!(
            r#"<article class="panel lf-grid"><header><h2><a href="{href}">{title}</a></h2></header>
<div class="lf-deadline">Fin de validation : <strong>{deadline}</strong></div>
<div class="lf-rows"><div class="lf-row lf-row-head"><span>N°</span><span class="lf-home">Domicile</span><span>Extérieur</span></div>{rows}</div></article>"#
        )
    }

    fn home_page(articles: &str) -> String {
        format!(
            r#"<html><body><section class="lf-cards lf-cards-open" aria-label="Grilles ouvertes">{articles}</section>
<section class="panel"><h2>Prochaines grilles</h2></section></body></html>"#
        )
    }

    fn lf8_article() -> String {
        open_article(
            "Loto Foot 8 n°158",
            "/loto-foot/grille/855-loto-foot-8-n158",
            "dimanche 15 novembre 2026 à 14h55",
            &[
                open_row(1, "France", "Turquie", ["90 %", "6 %", "4 %"]),
                open_row(2, "Espagne", "Angleterre", ["58 %", "25 %", "17 %"]),
            ]
            .concat(),
        )
    }

    #[test]
    fn the_open_page_gives_matches_split_and_close_in_utc() {
        let fetched = at("2026-11-14T22:45:00Z");
        let html = home_page(&lf8_article());

        let grids = parse_open_page(&html, fetched).unwrap();

        assert_eq!(grids.len(), 1);
        let grid = &grids[0];
        assert_eq!(grid.game, Game::Lf8);
        assert_eq!(grid.number, 158);
        assert!(grid.accepting);
        // 14 h 55 à Paris en novembre (heure d'hiver, UTC+1) = 13 h 55 UTC.
        assert_eq!(grid.closes_at, at("2026-11-15T13:55:00Z"));
        assert_eq!(grid.jackpot, None);
        assert_eq!(
            grid.matches,
            vec![
                ("France".to_string(), "Turquie".to_string()),
                ("Espagne".to_string(), "Angleterre".to_string())
            ]
        );
        assert_eq!(grid.split, Some(vec![[90, 6, 4], [58, 25, 17]]));
        assert_eq!(grid.split_at, Some(fetched));
    }

    #[test]
    fn only_lf7_and_lf8_are_read_from_the_open_page() {
        let lf12 = open_article(
            "Loto Foot 12 n°99",
            "/loto-foot/grille/2043-loto-foot-12-n99",
            "dimanche 15 novembre 2026 à 20h40",
            &open_row(1, "X", "Y", ["1 %", "1 %", "98 %"]),
        );
        let html = home_page(&format!("{lf12}{}", lf8_article()));
        let grids = parse_open_page(&html, at("2026-11-14T22:45:00Z")).unwrap();
        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].game, Game::Lf8);

        // Aucun LF7 ni LF8 ouvert : liste vide, sans erreur.
        let html = home_page(&lf12);
        assert!(
            parse_open_page(&html, at("2026-11-14T22:45:00Z"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_split_all_at_zero_is_none() {
        let article = open_article(
            "Loto Foot 7 n°143",
            "/loto-foot/grille/4960-loto-foot-7-n143",
            "lundi 16 novembre 2026 à 17h55",
            &open_row(1, "A", "B", ["0 %", "0 %", "0 %"]),
        );
        let grids = parse_open_page(&home_page(&article), at("2026-11-14T22:45:00Z")).unwrap();
        assert_eq!(grids[0].split, None);
    }

    #[test]
    fn the_open_page_refuses_a_link_that_names_another_grid() {
        let article = open_article(
            "Loto Foot 8 n°158",
            "/loto-foot/grille/855-loto-foot-8-n157",
            "dimanche 15 novembre 2026 à 14h55",
            &open_row(1, "A", "B", ["1 %", "1 %", "98 %"]),
        );
        let error = parse_open_page(&home_page(&article), at("2026-11-14T22:45:00Z")).unwrap_err();
        assert!(
            error.to_string().contains("ne correspond pas à LF8 n°158"),
            "{error}"
        );
    }

    #[test]
    fn an_unreadable_split_or_a_missing_section_is_an_error() {
        let article = open_article(
            "Loto Foot 8 n°158",
            "/loto-foot/grille/855-loto-foot-8-n158",
            "dimanche 15 novembre 2026 à 14h55",
            &open_row(1, "A", "B", ["90 %", "bientôt", "4 %"]),
        );
        let now = at("2026-11-14T22:45:00Z");
        let error = parse_open_page(&home_page(&article), now).unwrap_err();
        assert!(
            error.to_string().contains("répartition de A - B illisible"),
            "{error}"
        );

        let error =
            parse_open_page("<html><body><p>Maintenance</p></body></html>", now).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("section des grilles ouvertes introuvable"),
            "{error}"
        );
    }

    #[test]
    fn french_datetimes_convert_from_paris_summer_and_winter() {
        // Hiver : UTC+1.
        assert_eq!(
            parse_french_datetime("dimanche 15 novembre 2026 à 14h55"),
            Some(at("2026-11-15T13:55:00Z"))
        );
        // Été : UTC+2.
        assert_eq!(
            parse_french_datetime("samedi 4 juillet 2026 à 14h55"),
            Some(at("2026-07-04T12:55:00Z"))
        );
        // « 1er », sans jour de la semaine, minute à zéro ; l'année est celle du texte.
        assert_eq!(
            parse_french_datetime("1er janvier 2027 à 9h00"),
            Some(at("2027-01-01T08:00:00Z"))
        );
        assert_eq!(parse_french_datetime("bientôt"), None);
        // 2 h 30 le 29 mars 2026 n'existe pas à Paris (saut à l'heure d'été).
        assert_eq!(parse_french_datetime("dimanche 29 mars 2026 à 2h30"), None);
    }

    fn list_row(product: &str, number: &str, href: &str) -> String {
        format!(
            r#"<tr><td class="lr-grid" data-l="Grille"><span class="lr-prod lr-p7">{product}</span> <strong>{number}</strong><span class="lr-pot">Pactole 100 000 €</span></td>
<td class="lr-date" data-l="Date">Mar. 10 nov. 2026</td><td class="lr-go"><a href="{href}">Détails</a></td></tr>"#
        )
    }

    fn list_page(rows: &str) -> String {
        format!(
            r#"<html><body><table class="lf-list lr-table"><thead><tr><th>Grille</th></tr></thead><tbody>{rows}</tbody></table>
<nav class="pagination"><a href="/loto-foot/resultats">1</a></nav></body></html>"#
        )
    }

    #[test]
    fn the_results_list_gives_formula_number_and_link() {
        let rows = [
            list_row("LF 7", "n°140", "/loto-foot/grille/4957-loto-foot-7-n140"),
            list_row("LF 15", "n°94", "/loto-foot/grille/2060-loto-foot-15-n94"),
            list_row("LF 8", "n°155", "/loto-foot/grille/852-loto-foot-8-n155"),
        ]
        .concat();

        let links = parse_results_list(&list_page(&rows)).unwrap();

        assert_eq!(
            links,
            vec![
                ResultLink {
                    game: Game::Lf7,
                    number: 140,
                    href: "/loto-foot/grille/4957-loto-foot-7-n140".to_string()
                },
                ResultLink {
                    game: Game::Lf8,
                    number: 155,
                    href: "/loto-foot/grille/852-loto-foot-8-n155".to_string()
                },
            ]
        );
        assert_eq!(links[0].path(), "loto-foot/grille/4957-loto-foot-7-n140");
    }

    #[test]
    fn an_empty_list_is_fine_and_a_page_without_table_is_not() {
        assert!(parse_results_list(&list_page("")).unwrap().is_empty());
        let error = parse_results_list("<html><body>Erreur 500</body></html>").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("tableau des résultats introuvable"),
            "{error}"
        );
        let row = list_row("LF 8", "n°155", "/loto-foot/grille/852-loto-foot-8-n156");
        assert!(parse_results_list(&list_page(&row)).is_err());
    }

    fn detail_row(pos: usize, home: &str, away: &str, res: &str, prov: &str, note: &str) -> String {
        format!(
            r#"<div class="lf-row" data-pos="{pos}" data-res="{res}" data-prov="{prov}"><span class="lf-num">{pos}</span><span class="lf-team lf-home">{home}</span><div class="lf-mid"><span class="lf-score">1-2</span></div><span class="lf-team">{away}{note}</span></div>"#
        )
    }

    fn rank_html(label: &str, winners: &str, gain: &str) -> String {
        format!(
            r#"<tr><td><strong>{label}</strong></td><td class="num">{winners}</td><td class="num lf-gain">{gain}</td></tr>"#
        )
    }

    fn detail_page(title: &str, rows: &str, ranks: Option<&str>) -> String {
        let table = ranks
            .map(|body| {
                format!(
                    r#"<div class="table-box draw-ranks-box"><table><thead><tr><th>Rang</th></tr></thead><tbody>{body}</tbody></table></div>"#
                )
            })
            .unwrap_or_default();
        format!(
            r#"<html><body><article class="panel lf-grid"><header class="panel-head"><h2>{title}</h2><span>Fin de validation : mardi 10 novembre 2026 à 18h40</span></header>
<div class="lf-rows">{rows}</div><div class="panel-body">{table}</div></article>
<aside><h2>{title} en bref</h2></aside></body></html>"#
        )
    }

    fn lf7_rows(results: [&str; 7]) -> String {
        results
            .iter()
            .enumerate()
            .map(|(i, res)| {
                detail_row(
                    i + 1,
                    &format!("D{}", i + 1),
                    &format!("E{}", i + 1),
                    res,
                    "",
                    "",
                )
            })
            .collect()
    }

    #[test]
    fn a_detail_page_gives_signs_close_and_every_rank() {
        // Espaces fines insécables (U+202F) dans les milliers, espace insécable (U+00A0) avant « € ».
        let ranks = [
            rank_html("7 sur 7", "1\u{202f}883", "31\u{a0}€"),
            rank_html("6 sur 7", "26\u{202f}943", "2,60\u{a0}€"),
        ]
        .concat();
        let html = detail_page(
            "Loto Foot 7 n°140",
            &lf7_rows(["2", "2", "1", "1", "N", "N", "2"]),
            Some(&ranks),
        );

        let result = parse_result_detail(&html).unwrap().unwrap();

        assert_eq!(result.game, Some(Game::Lf7));
        assert_eq!(result.number, 140);
        // 18 h 40 à Paris le 10 novembre (UTC+1) = 17 h 40 UTC ; le jour de clôture est celui de Paris.
        assert_eq!(result.closes_at, Some(at("2026-11-10T17:40:00Z")));
        assert_eq!(result.date, NaiveDate::from_ymd_opt(2026, 11, 10));
        assert_eq!(result.matches.len(), 7);
        assert_eq!(
            result.matches[0],
            (
                "D1".to_string(),
                "E1".to_string(),
                MatchResult::Sign(Outcome::Away)
            )
        );
        assert_eq!(result.matches[2].2, MatchResult::Sign(Outcome::Home));
        assert_eq!(result.matches[4].2, MatchResult::Sign(Outcome::Draw));
        assert_eq!(
            result.ranks,
            vec![
                RankLine {
                    good: Some(7),
                    winners: 1883,
                    unit_gain: Euros::new(31.0)
                },
                RankLine {
                    good: Some(6),
                    winners: 26943,
                    unit_gain: Euros::new(2.6)
                },
            ]
        );
    }

    #[test]
    fn a_canceled_match_is_read_with_its_mention_removed_from_the_name() {
        let rows: String = (1..=7)
            .map(|i| match i {
                5 => detail_row(
                    5,
                    "D5",
                    "Austr.Vienne F",
                    "A",
                    "",
                    r#" <em class="muted small">(match annulé)</em>"#,
                ),
                _ => detail_row(i, &format!("D{i}"), &format!("E{i}"), "1", "", ""),
            })
            .collect();
        let ranks = [
            rank_html("7 sur 7", "10", "50 €"),
            rank_html("6 sur 7", "100", "5 €"),
        ]
        .concat();
        let html = detail_page("Loto Foot 7 n°140", &rows, Some(&ranks));

        let result = parse_result_detail(&html).unwrap().unwrap();

        assert_eq!(result.matches[4].1, "Austr.Vienne F");
        assert_eq!(result.matches[4].2, MatchResult::Canceled);
        // Les libellés de rang ne changent pas : 7 et 6 bons (§74).
        assert_eq!(result.ranks[0].good, Some(7));
    }

    #[test]
    fn a_grid_with_a_pending_or_provisional_result_is_not_returned() {
        let ranks = [
            rank_html("7 sur 7", "10", "50 €"),
            rank_html("6 sur 7", "100", "5 €"),
        ]
        .concat();
        // Une rencontre sans résultat (data-res vide).
        let pending = detail_page(
            "Loto Foot 7 n°141",
            &lf7_rows(["1", "1", "1", "1", "1", "1", ""]),
            Some(&ranks),
        );
        assert_eq!(parse_result_detail(&pending).unwrap(), None);
        // Une rencontre au résultat provisoire (data-prov non vide) : le signe est celui d'un score en cours.
        let rows = lf7_rows(["1", "1", "1", "1", "1", "1", "1"]).replacen(
            r#"data-pos="7" data-res="1" data-prov="""#,
            r#"data-pos="7" data-res="1" data-prov="1""#,
            1,
        );
        let provisional = detail_page("Loto Foot 7 n°141", &rows, Some(&ranks));
        assert_eq!(parse_result_detail(&provisional).unwrap(), None);
        // Sans résultat complet, l'absence de tableau n'est pas une erreur.
        let no_table = detail_page(
            "Loto Foot 7 n°141",
            &lf7_rows(["", "", "", "", "", "", ""]),
            None,
        );
        assert_eq!(parse_result_detail(&no_table).unwrap(), None);
    }

    #[test]
    fn a_complete_result_without_ranks_table_is_an_error() {
        let html = detail_page(
            "Loto Foot 7 n°140",
            &lf7_rows(["1", "1", "1", "1", "1", "1", "1"]),
            None,
        );
        let error = parse_result_detail(&html).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("tableau des rapports introuvable"),
            "{error}"
        );
    }

    #[test]
    fn a_rank_without_winner_has_no_gain_and_unpublished_ranks_are_dropped() {
        let rows = lf7_rows(["1", "1", "1", "1", "1", "1", "1"]);
        let ranks = [
            rank_html("7 sur 7", "–", ""),
            rank_html("6 sur 7", "3", "1 234,50 €"),
        ]
        .concat();
        let result = parse_result_detail(&detail_page("Loto Foot 7 n°5", &rows, Some(&ranks)))
            .unwrap()
            .unwrap();
        assert_eq!(
            result.ranks[0],
            RankLine {
                good: Some(7),
                winners: 0,
                unit_gain: None
            }
        );
        assert_eq!(result.ranks[1].unit_gain, Euros::new(1234.5));

        // Tous les rangs à zéro : rapport pas encore publié, rien n'est gardé.
        let zeros = [
            rank_html("7 sur 7", "0", "0 €"),
            rank_html("6 sur 7", "0", "0 €"),
        ]
        .concat();
        let result = parse_result_detail(&detail_page("Loto Foot 7 n°5", &rows, Some(&zeros)))
            .unwrap()
            .unwrap();
        assert!(result.ranks.is_empty());
    }

    #[test]
    fn other_formulas_and_unreadable_rows_are_handled() {
        let rows = lf7_rows(["1", "1", "1", "1", "1", "1", "1"]);
        let lf12 = detail_page("Loto Foot 12 n°99", &rows, None);
        assert_eq!(parse_result_detail(&lf12).unwrap(), None);

        let odd = lf7_rows(["1", "1", "1", "1", "1", "1", "X"]);
        let error = parse_result_detail(&detail_page("Loto Foot 7 n°5", &odd, None)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("résultat « X » inconnu à la rencontre 7"),
            "{error}"
        );

        let error = parse_result_detail("<html><body>Page introuvable</body></html>").unwrap_err();
        assert!(error.to_string().contains("grille introuvable"), "{error}");
    }

    #[test]
    fn numbers_are_read_through_non_breaking_spaces() {
        assert_eq!(parse_winners("1\u{202f}883"), Some(1883));
        assert_eq!(parse_winners("17 338"), Some(17338));
        assert_eq!(parse_winners("Aucun gagnant"), Some(0));
        assert_eq!(parse_winners("beaucoup"), None);
        assert_eq!(parse_euros("2,60\u{a0}€"), Euros::new(2.6));
        assert_eq!(parse_euros("62 €"), Euros::new(62.0));
        assert_eq!(parse_euros("n/d"), None);
    }

    #[test]
    fn results_pages_are_numbered_from_one() {
        assert_eq!(results_path(1), "loto-foot/resultats");
        assert_eq!(results_path(2), "loto-foot/resultats?page=2");
    }

    /// Contrôle sur de vraies pages, hors dépôt : `EDGE_RESULTATS_FDJ_PAGES=<dossier> cargo test -- --ignored resultats_fdj::tests::real_pages`.
    /// Le dossier contient des copies de pages de resultats-fdj.fr : `rfdj_home_*.html` (accueil), `rfdj_list_*.html`
    /// (liste des résultats) et `rfdj_detail_*.html` (détail d'une grille).
    #[test]
    #[ignore = "lit des pages réelles hors dépôt"]
    fn real_pages_are_read_without_error() {
        let Ok(dir) = std::env::var("EDGE_RESULTATS_FDJ_PAGES") else {
            return;
        };
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let html = std::fs::read_to_string(&path).unwrap();
            if name.starts_with("rfdj_home_") {
                let grids = parse_open_page(&html, at("2026-11-14T22:45:00Z"))
                    .unwrap_or_else(|e| panic!("{name} : {e}"));
                for g in &grids {
                    println!(
                        "{name} : {} n°{} clôture {} {} rencontres, répartition {}",
                        g.game,
                        g.number,
                        g.closes_at,
                        g.matches.len(),
                        g.split.is_some()
                    );
                }
            } else if name.starts_with("rfdj_list_") {
                let links = parse_results_list(&html).unwrap_or_else(|e| panic!("{name} : {e}"));
                println!("{name} : {} liens", links.len());
                for l in &links {
                    println!("    {} n°{} {}", l.game, l.number, l.href);
                }
            } else if name.starts_with("rfdj_detail_") {
                let result = parse_result_detail(&html).unwrap_or_else(|e| panic!("{name} : {e}"));
                println!("{name} : {result:?}");
            } else {
                continue;
            }
            checked += 1;
        }
        assert!(checked > 0, "aucune page lue");
    }
}
