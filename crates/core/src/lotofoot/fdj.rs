//! Lecture des pages de grilles du site des points de vente FDJ, source principale du Loto Foot depuis le 2026-11-14.
//!
//! Chaque page embarque l'état de l'application Angular dans un `<script id="serverApp-state">` : un JSON dont les
//! guillemets et les chevrons sont écrits `&q;`, `&l;`… On le décode, puis on ne lit que les champs utiles
//! (`docs/sources.md` §74). Les clés `environment` et `pfa-token` du même état (configuration du site, jeton opaque) sont
//! ignorées et ne sont jamais journalisées. Seuls les faits extraits sont conservés, jamais les pages (leurs mentions
//! légales interdisent la reproduction). Le LF12, le LF15 et tout autre jeu sont ignorés sans erreur : seuls le LF7 et
//! le LF8 sont joués.
//!
//! Ce module ne fait aucun accès réseau : il lit une page déjà téléchargée.

use std::cmp::Reverse;

use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Europe::Paris;
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::Value;

use super::grid_page::{
    GridResult, MatchResult, OpenGrid, RankLine, published_ranks, published_split,
};
use super::{Game, GridId};
use crate::domain::{Euros, Outcome};
use crate::error::{Error, Result};

/// Racine du site.
pub const BASE_URL: &str = "https://www.pointdevente.parionssport.fdj.fr/";

/// Chemin de la page des grilles ouvertes (toutes formules).
pub const OPEN_PATH: &str = "grilles/ouvertes/loto-foot";

/// Chemin de la page des résultats (première page ; les suivantes s'obtiennent par [`results_path`]).
pub const RESULTS_PATH: &str = "grilles/resultats/loto-foot";

/// Chemin de la page `page` des résultats, numérotée à partir de 1 (`?page=N`). La première page s'obtient sans
/// paramètre, comme le site la sert (`docs/sources.md` §74).
pub fn results_path(page: u32) -> String {
    if page <= 1 {
        RESULTS_PATH.to_string()
    } else {
        format!("{RESULTS_PATH}?page={page}")
    }
}

/// Une page de résultats : les grilles LF7 et LF8 dont le résultat est complet, et ce qu'il faut savoir du reste.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultsPage {
    /// Grilles dont toutes les rencontres ont un résultat (signe ou annulation), dans l'ordre de la page : cet ordre n'est
    /// pas chronologique (`docs/sources.md` §74). Une grille dont une rencontre n'a pas encore de résultat n'y figure pas.
    pub results: Vec<GridResult>,
    /// Grilles **entièrement** annulées (`drawCancelled`, vu une fois en 2021) : leurs mises sont remboursées, elles
    /// n'ont ni signes ni rapports. À part de `results`, pour que personne ne les règle comme un tirage ordinaire.
    pub canceled_draws: Vec<GridId>,
    /// Grilles non lues pour une raison anormale (résultat au libellé inconnu) : à montrer à l'utilisateur.
    pub warnings: Vec<String>,
    /// Nombre total de grilles de la liste (toutes formules), selon la pagination de la page.
    pub total: u32,
    /// Rang de la première grille de cette page dans la liste.
    pub offset: u32,
    /// Nombre de grilles par page.
    pub limit: u32,
}

/// État Angular de la page, réduit à ce que l'on lit : serde ignore toutes les autres clés (`environment`, `pfa-token`…).
#[derive(Deserialize)]
struct State {
    #[serde(rename = "gridsResolver")]
    grids_resolver: Option<GridsResolver>,
}

/// Liste de grilles (pages des grilles ouvertes, annoncées et des résultats).
#[derive(Deserialize)]
struct GridsResolver {
    /// Grilles de toutes les formules, gardées en JSON brut : seules celles du LF7 et du LF8 sont lues (voir
    /// [`tracked_grids`]), pour qu'une grille du LF12 au format inattendu n'empêche pas de lire les autres.
    grids: Vec<Value>,
    /// Présente sur toute page de liste ; sert aux résultats, que l'on parcourt page par page.
    pagination: Option<Pagination>,
}

#[derive(Deserialize)]
struct Pagination {
    limit: u32,
    offset: u32,
    total: u32,
}

/// Une grille LF7 ou LF8 telle que la FDJ la sert.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGrid {
    grid_number: u32,
    external_draw_number: u32,
    start_validation_date: String,
    end_validation_date: String,
    draw_cancelled: bool,
    jackpot: RawJackpot,
    product: RawProduct,
    events: Vec<RawEvent>,
    /// Absent ou vide tant que la grille n'est pas close.
    #[serde(default)]
    winranks: Vec<RawRank>,
}

#[derive(Deserialize)]
struct RawJackpot {
    /// Euros entiers, 0 sans pactole.
    amount: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProduct {
    number_events: usize,
}

#[derive(Deserialize)]
struct RawEvent {
    /// Équipe à domicile, équipe à l'extérieur.
    opponents: Vec<String>,
    /// `null` tant que la rencontre n'est pas jouée.
    result: Option<String>,
    /// Répartition des joueurs ; réduite à un élément sans libellé une fois la grille close.
    #[serde(default)]
    outcomes: Vec<RawOutcome>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOutcome {
    label: String,
    percentage_played: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRank {
    /// Bons pronostics du rang.
    rank_number_market: u32,
    /// Nombre de grilles gagnantes.
    rank_number_grid: u32,
    /// Gain unitaire **en centimes**.
    rank_amount: u64,
}

/// Une grille LF7 ou LF8 lue et dont l'identité a été contrôlée.
struct Tracked {
    game: Game,
    closes_at: DateTime<Utc>,
    raw: RawGrid,
}

fn page_error(message: &str) -> Error {
    Error::Data(format!("page FDJ : {message}"))
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("sélecteur CSS constant valide")
}

/// Décode les entités de l'état Angular : `&q;` pour `"`, `&s;` pour `'`, `&l;` pour `<`, `&g;` pour `>`, `&a;` pour `&`.
///
/// `&a;` passe **en dernier**. Angular échappe d'abord `&` en `&a;`, puis les autres caractères : un `&q;` écrit dans un
/// texte devient donc `&a;q;`. Décoder `&a;` en premier donnerait `&q;`, que l'étape suivante prendrait pour un guillemet ;
/// en dernier, `&a;q;` ne contient aucun `&q;` et redonne bien le texte d'origine.
fn decode_entities(state: &str) -> String {
    state
        .replace("&q;", "\"")
        .replace("&s;", "'")
        .replace("&l;", "<")
        .replace("&g;", ">")
        .replace("&a;", "&")
}

/// Extrait de la page la liste de grilles (`gridsResolver`) de son état.
fn grids_resolver(html: &str) -> Result<GridsResolver> {
    let document = Html::parse_document(html);
    let script = document
        .select(&selector("script#serverApp-state"))
        .next()
        .ok_or_else(|| {
            page_error(
                "état serverApp-state introuvable (page modifiée, ou défi anti-robot à la place de la page)",
            )
        })?;
    let json = decode_entities(&script.text().collect::<String>());
    let state: State = serde_json::from_str(&json)
        .map_err(|e| page_error(&format!("état serverApp-state illisible : {e}")))?;
    state
        .grids_resolver
        .ok_or_else(|| page_error("gridsResolver absent de l'état"))
}

/// Formule d'un jeu d'après son `deeplink` (`lotofoot7`, `lotofoot8`) ; `None` pour tout autre jeu.
///
/// Le `deeplink` désigne la formule, non `numberEvents` : le règlement permet un LF8 à 7 rencontres, qui reste un LF8
/// (`docs/sources.md` §74).
fn game_of(deeplink: &str) -> Option<Game> {
    match deeplink {
        "lotofoot7" => Some(Game::Lf7),
        "lotofoot8" => Some(Game::Lf8),
        _ => None,
    }
}

/// Date de la FDJ (`2026-11-15T19:40:00.000Z`) en instant UTC ; `field` nomme le champ dans le message d'erreur.
fn parse_instant(text: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|_| page_error(&format!("{field} illisible : « {text} »")))
}

/// Lit les grilles LF7 et LF8 d'une liste, et ignore les autres jeux sans les analyser.
///
/// `values` est pris par valeur : `serde_json::from_value` consomme le JSON, ce qui évite d'en copier chaque grille.
fn tracked_grids(values: Vec<Value>) -> Result<Vec<Tracked>> {
    let mut grids = Vec::new();
    for value in values {
        let deeplink = value.pointer("/product/deeplink").and_then(Value::as_str);
        // `game` est `Copy` : l'emprunt de `value` par `deeplink` est fini avant que `from_value` ne la consomme.
        let Some(game) = deeplink.and_then(game_of) else {
            continue;
        };
        let raw: RawGrid = serde_json::from_value(value)
            .map_err(|e| page_error(&format!("grille {game} illisible : {e}")))?;
        let label = format!("{game} n°{}", raw.grid_number);
        if raw.events.len() != raw.product.number_events {
            return Err(page_error(&format!(
                "{label} : {} rencontres lues, numberEvents en annonce {}",
                raw.events.len(),
                raw.product.number_events
            )));
        }
        let closes_at = parse_instant(&raw.end_validation_date, "endValidationDate")?;
        check_external_number(&label, &raw, closes_at)?;
        grids.push(Tracked {
            game,
            closes_at,
            raw,
        });
    }
    Ok(grids)
}

/// Contrôle de cohérence de l'identité d'une grille.
///
/// `externalDrawNumber` vaut (dernier chiffre de l'année de **clôture**, heure de Paris) × 1 000 + `gridNumber` : sur
/// 1 735 grilles de 2021 à 2026 et sur les 51 lues le 2026-11-14, sans exception (`docs/sources.md` §74). La FDJ
/// renumérote chaque formule le 1er janvier (§66) : l'année que nous déduisons de la clôture doit donc s'accorder avec ce
/// numéro. Un désaccord (fuseau, renumérotation d'un autre type) fait refuser la page plutôt que d'écrire dans l'archive,
/// qui ne se corrige pas, une grille sous la mauvaise année.
fn check_external_number(label: &str, raw: &RawGrid, closes_at: DateTime<Utc>) -> Result<()> {
    let year = closes_at.with_timezone(&Paris).year();
    let expected = i64::from(year.rem_euclid(10)) * 1000 + i64::from(raw.grid_number);
    if i64::from(raw.external_draw_number) == expected {
        return Ok(());
    }
    Err(page_error(&format!(
        "{label} : externalDrawNumber {} au lieu de {expected} attendu pour une clôture en {year} ; numérotation changée, \
         grille refusée (docs/sources.md §66 et §74)",
        raw.external_draw_number
    )))
}

/// Les deux équipes d'une rencontre (domicile, extérieur) ; une erreur si la FDJ n'en donne pas exactement deux.
fn team_names(opponents: Vec<String>, label: &str) -> Result<(String, String)> {
    let [home, away] = <[String; 2]>::try_from(opponents).map_err(|other| {
        page_error(&format!(
            "{label} : une rencontre a {} équipes au lieu de 2",
            other.len()
        ))
    })?;
    Ok((home, away))
}

/// Pourcentages (1, N, 2) d'une rencontre, lus **par libellé** : l'ordre du tableau `outcomes` n'est pas garanti.
fn split_of(outcomes: &[RawOutcome], label: &str) -> Result<[u32; 3]> {
    let mut triple = [0; 3];
    for (slot, wanted) in triple.iter_mut().zip(["1", "N", "2"]) {
        let found = outcomes
            .iter()
            .find(|outcome| outcome.label == wanted)
            .ok_or_else(|| {
                page_error(&format!(
                    "{label} : issue « {wanted} » absente de la répartition"
                ))
            })?;
        if found.percentage_played > 100 {
            return Err(page_error(&format!(
                "{label} : pourcentage {} hors de 0 à 100",
                found.percentage_played
            )));
        }
        *slot = found.percentage_played;
    }
    Ok(triple)
}

/// Lit la page des grilles ouvertes : une [`OpenGrid`] par grille LF7 ou LF8.
///
/// `fetched_at` est l'instant de la lecture. Il décide si la grille accepte déjà les prises de jeu (`startValidationDate`
/// passée) et date la répartition : la page n'a pas d'heure de relevé, et elle peut avoir cinq minutes de retard
/// (`Cache-Control: max-age=300`, `docs/sources.md` §74). Une grille annoncée porte des pourcentages à 0 : sa
/// répartition est alors `None`.
pub fn parse_open_page(html: &str, fetched_at: DateTime<Utc>) -> Result<Vec<OpenGrid>> {
    let resolver = grids_resolver(html)?;
    tracked_grids(resolver.grids)?
        .into_iter()
        .map(|grid| open_grid(grid, fetched_at))
        .collect()
}

fn open_grid(grid: Tracked, fetched_at: DateTime<Utc>) -> Result<OpenGrid> {
    let Tracked {
        game,
        closes_at,
        raw,
    } = grid;
    let label = format!("{game} n°{}", raw.grid_number);
    let starts_at = parse_instant(&raw.start_validation_date, "startValidationDate")?;
    let mut matches = Vec::new();
    let mut split = Vec::new();
    // `raw.events` est consommé : les noms d'équipes passent dans l'`OpenGrid` sans `clone()`.
    for event in raw.events {
        split.push(split_of(&event.outcomes, &label)?);
        matches.push(team_names(event.opponents, &label)?);
    }
    Ok(OpenGrid {
        game,
        accepting: starts_at <= fetched_at,
        number: raw.grid_number,
        closes_at,
        jackpot: Euros::new(raw.jackpot.amount).filter(|amount| amount.get() > 0.0),
        matches,
        split: published_split(split),
        split_at: Some(fetched_at),
    })
}

/// Lit une page de résultats (`?page=N`) : les grilles LF7 et LF8 dont toutes les rencontres ont un résultat.
///
/// La pagination (`total`, `offset`, `limit`) vient de la page, non du nombre de grilles lues : la page porte d'autres
/// formules, et une grille peut en être retirée (`docs/sources.md` §74). Quatre cas ne donnent pas un [`GridResult`] :
/// - une rencontre sans résultat (`null`) : la grille n'est pas finie, on la relira à la prochaine lecture ;
/// - un libellé de résultat inconnu : la grille est écartée avec un message dans [`ResultsPage::warnings`] (un seul
///   libellé inattendu ne doit pas faire perdre toute la page) ;
/// - une grille entièrement annulée : son identité va dans [`ResultsPage::canceled_draws`] ;
/// - une formule autre que le LF7 et le LF8 : ignorée.
pub fn parse_results_page(html: &str) -> Result<ResultsPage> {
    let resolver = grids_resolver(html)?;
    let pagination = resolver
        .pagination
        .ok_or_else(|| page_error("pagination absente de la page des résultats"))?;
    let mut page = ResultsPage {
        results: Vec::new(),
        canceled_draws: Vec::new(),
        warnings: Vec::new(),
        total: pagination.total,
        offset: pagination.offset,
        limit: pagination.limit,
    };
    for grid in tracked_grids(resolver.grids)? {
        read_result(grid, &mut page)?;
    }
    Ok(page)
}

/// Résultat d'une rencontre d'après son libellé FDJ ; `None` pour un libellé inconnu.
///
/// `canceled` (un seul « l ») : rencontre annulée, bonne pour tous les signes (règlement Loto Sports v6, art. 10.6).
fn match_result(label: &str) -> Option<MatchResult> {
    match label {
        "firstOpponentWinner" => Some(MatchResult::Sign(Outcome::Home)),
        "draw" => Some(MatchResult::Sign(Outcome::Draw)),
        "secondOpponentWinner" => Some(MatchResult::Sign(Outcome::Away)),
        "canceled" => Some(MatchResult::Canceled),
        _ => None,
    }
}

/// Rapport d'un rang. `rankAmount` est en **centimes** (6 200 pour 62 €) ; sans gagnant, il n'y a pas de gain unitaire.
fn rank_line(raw: &RawRank) -> RankLine {
    let winners = raw.rank_number_grid;
    RankLine {
        good: Some(raw.rank_number_market),
        winners,
        // Les gains tiennent largement dans les 53 bits de mantisse d'un `f64` : la conversion est exacte.
        unit_gain: (winners > 0)
            .then(|| Euros::new(raw.rank_amount as f64 / 100.0))
            .flatten(),
    }
}

/// Ajoute la grille à la page des résultats, à ses annulations ou à ses avertissements, selon ce qu'elle contient.
fn read_result(grid: Tracked, page: &mut ResultsPage) -> Result<()> {
    let Tracked {
        game,
        closes_at,
        raw,
    } = grid;
    let id = GridId::closing(game, raw.grid_number, closes_at);
    if raw.draw_cancelled {
        page.canceled_draws.push(id);
        return Ok(());
    }
    let label = format!("{game} n°{}", raw.grid_number);
    let mut matches = Vec::new();
    for event in raw.events {
        // Sans résultat : la grille n'est pas finie (aucun avertissement, c'est l'état normal après la clôture).
        let Some(shown) = event.result.as_deref() else {
            return Ok(());
        };
        let Some(result) = match_result(shown) else {
            page.warnings.push(format!(
                "FDJ {label} : résultat « {shown} » inconnu, grille non lue"
            ));
            return Ok(());
        };
        let (home, away) = team_names(event.opponents, &label)?;
        matches.push((home, away, result));
    }
    let mut ranks: Vec<RankLine> = raw.winranks.iter().map(rank_line).collect();
    // Rangs par bons pronostics décroissants : le sans-faute d'abord, quel que soit l'ordre de la page.
    ranks.sort_by_key(|rank| Reverse(rank.good));
    page.results.push(GridResult {
        game: Some(game),
        number: raw.grid_number,
        date: Some(closes_at.with_timezone(&Paris).date_naive()),
        closes_at: Some(closes_at),
        matches,
        ranks: published_ranks(ranks),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Pages SYNTHÉTIQUES, écrites pour les tests sur le modèle de la structure du site (aucune copie de ses pages).

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// Échappement d'Angular, inverse de `decode_entities` : `&` d'abord.
    fn encode(json: &str) -> String {
        json.replace('&', "&a;")
            .replace('"', "&q;")
            .replace('\'', "&s;")
            .replace('<', "&l;")
            .replace('>', "&g;")
    }

    /// Une page avec son état, plus les deux clés que l'on ignore (une valeur y ressemble à un jeton).
    fn page(resolver: Value) -> String {
        let state = json!({
            "environment": "{\"production\":true}",
            "pfa-token": {"iv": "00", "content": "ff"},
            "gridsResolver": resolver,
        });
        format!(
            "<html><body><script id=\"serverApp-state\" type=\"application/json\">{}</script></body></html>",
            encode(&state.to_string())
        )
    }

    /// Une rencontre ouverte, avec sa répartition dans l'ordre 1, N, 2.
    fn open_event(home: &str, away: &str, split: [u32; 3]) -> Value {
        json!({
            "eventId": 1, "startDate": "2026-11-15T14:00:00.000Z", "result": null,
            "opponents": [home, away],
            "outcomes": [
                {"label": "1", "percentagePlayed": split[0]},
                {"label": "N", "percentagePlayed": split[1]},
                {"label": "2", "percentagePlayed": split[2]},
            ],
        })
    }

    /// Une rencontre jouée : la FDJ ne garde alors qu'un élément sans libellé dans `outcomes`.
    fn played_event(result: Value) -> Value {
        json!({
            "eventId": 1, "startDate": "", "result": result,
            "opponents": ["Domicile", "Extérieur"],
            "outcomes": [{"label": "", "percentagePlayed": 0}],
        })
    }

    /// Une grille : `deeplink`, numéro, clôture, rencontres et rangs ; le reste est sans importance pour les tests.
    fn grid(
        deeplink: &str,
        number: u32,
        closes: &str,
        events: Vec<Value>,
        winranks: Value,
    ) -> Value {
        let year_digit = u32::from(closes.as_bytes()[3] - b'0');
        json!({
            "gridNumber": number,
            "externalDrawNumber": year_digit * 1000 + number,
            "startValidationDate": "2026-11-13T23:10:00.000Z",
            "endValidationDate": closes,
            "drawCancelled": false,
            "jackpot": {"amount": 0, "type": 0, "gmg": ""},
            "product": {"deeplink": deeplink, "numberEvents": events.len(), "label": "Loto Foot"},
            "events": events,
            "winranks": winranks,
        })
    }

    fn resolver(grids: Vec<Value>) -> Value {
        json!({"grids": grids, "pagination": {"limit": 20, "offset": 0, "total": 99, "page": 1}})
    }

    fn open_lf8(events: Vec<Value>) -> Value {
        grid(
            "lotofoot8",
            158,
            "2026-11-15T13:55:00.000Z",
            events,
            json!([]),
        )
    }

    #[test]
    fn entities_are_decoded_with_the_ampersand_last() {
        assert_eq!(decode_entities("{&q;a&q;:&s;b&s;&l;&g;}"), "{\"a\":'b'<>}");
        // Un `&q;` écrit dans un texte arrive sous la forme `&a;q;` : il doit redonner `&q;`, non un guillemet.
        assert_eq!(decode_entities("&a;q;"), "&q;");
        assert_eq!(decode_entities("&a;a;"), "&a;");
    }

    #[test]
    fn team_names_survive_the_encoding_round_trip() {
        let events = vec![open_event("Côte d'Ivoire & Cie <1>", "A&q;B", [1, 2, 97])];
        let html = page(resolver(vec![open_lf8(events)]));
        let grids = parse_open_page(&html, at("2026-11-14T22:45:00Z")).unwrap();
        assert_eq!(
            grids[0].matches[0],
            ("Côte d'Ivoire & Cie <1>".to_string(), "A&q;B".to_string())
        );
    }

    #[test]
    fn an_open_grid_is_read_with_close_jackpot_and_split() {
        let mut lf8 = open_lf8(vec![
            open_event("France", "Turquie", [90, 6, 4]),
            open_event("Espagne", "Angleterre", [58, 25, 17]),
        ]);
        lf8["jackpot"]["amount"] = json!(150_000);
        let html = page(resolver(vec![lf8]));
        let fetched = at("2026-11-14T22:45:00Z");

        let grids = parse_open_page(&html, fetched).unwrap();

        assert_eq!(grids.len(), 1);
        let grid = &grids[0];
        assert_eq!(grid.game, Game::Lf8);
        assert_eq!(grid.number, 158);
        // Clôture lue telle quelle en UTC (14 h 55 à Paris, le 15 novembre).
        assert_eq!(grid.closes_at, at("2026-11-15T13:55:00Z"));
        assert!(grid.accepting, "startValidationDate (13 nov.) est passée");
        assert_eq!(grid.jackpot, Euros::new(150_000.0));
        assert_eq!(grid.split, Some(vec![[90, 6, 4], [58, 25, 17]]));
        assert_eq!(grid.split_at, Some(fetched));
        assert_eq!(
            grid.matches[1],
            ("Espagne".to_string(), "Angleterre".to_string())
        );
    }

    #[test]
    fn no_jackpot_amount_means_none() {
        let html = page(resolver(vec![open_lf8(vec![open_event(
            "A",
            "B",
            [1, 1, 98],
        )])]));
        let grids = parse_open_page(&html, at("2026-11-14T22:45:00Z")).unwrap();
        assert_eq!(grids[0].jackpot, None);
    }

    #[test]
    fn the_formula_comes_from_the_deeplink_and_other_games_are_ignored() {
        // Un LF8 à 7 rencontres reste un LF8 ; le LF12 et le LF15 sont ignorés, même au format inattendu.
        let lf8_short = open_lf8(vec![open_event("A", "B", [1, 1, 98]); 7]);
        let lf12 = grid(
            "lotofoot12",
            99,
            "2026-11-15T19:40:00.000Z",
            vec![open_event("X", "Y", [1, 1, 98]); 12],
            json!([]),
        );
        let lf15_broken =
            json!({"product": {"deeplink": "lotofoot15"}, "gridNumber": "pas un nombre"});
        let html = page(resolver(vec![lf12, lf8_short, lf15_broken]));

        let grids = parse_open_page(&html, at("2026-11-14T22:45:00Z")).unwrap();

        assert_eq!(grids.len(), 1);
        assert_eq!(grids[0].game, Game::Lf8);
        assert_eq!(grids[0].matches.len(), 7);
    }

    #[test]
    fn a_number_of_events_that_disagrees_with_the_announcement_is_an_error() {
        let mut lf8 = open_lf8(vec![open_event("A", "B", [1, 1, 98]); 8]);
        lf8["product"]["numberEvents"] = json!(7);
        let html = page(resolver(vec![lf8]));
        let error = parse_open_page(&html, at("2026-11-14T22:45:00Z")).unwrap_err();
        assert!(error.to_string().contains("8 rencontres lues"), "{error}");
    }

    #[test]
    fn the_split_is_read_by_label_not_by_position() {
        let mut event = open_event("A", "B", [0, 0, 0]);
        event["outcomes"] = json!([
            {"label": "2", "percentagePlayed": 20},
            {"label": "1", "percentagePlayed": 50},
            {"label": "N", "percentagePlayed": 30},
        ]);
        let html = page(resolver(vec![open_lf8(vec![event])]));
        let grids = parse_open_page(&html, at("2026-11-14T22:45:00Z")).unwrap();
        assert_eq!(grids[0].split, Some(vec![[50, 30, 20]]));
    }

    #[test]
    fn a_missing_label_is_an_error_not_a_zero() {
        let mut event = open_event("A", "B", [50, 30, 20]);
        event["outcomes"] = json!([{"label": "1", "percentagePlayed": 50}]);
        let html = page(resolver(vec![open_lf8(vec![event])]));
        let error = parse_open_page(&html, at("2026-11-14T22:45:00Z")).unwrap_err();
        assert!(error.to_string().contains("issue « N » absente"), "{error}");
    }

    #[test]
    fn an_announced_grid_has_no_split_and_does_not_accept_yet() {
        let mut lf8 = open_lf8(vec![open_event("A", "B", [0, 0, 0]); 2]);
        lf8["startValidationDate"] = json!("2026-11-15T23:10:00.000Z");
        let html = page(resolver(vec![lf8]));

        let grids = parse_open_page(&html, at("2026-11-14T22:45:00Z")).unwrap();

        assert!(!grids[0].accepting);
        assert_eq!(grids[0].split, None);
    }

    #[test]
    fn a_grid_accepts_from_its_opening_instant() {
        let html = page(resolver(vec![open_lf8(vec![open_event(
            "A",
            "B",
            [1, 1, 98],
        )])]));
        let accepting = |fetched| parse_open_page(&html, at(fetched)).unwrap()[0].accepting;
        assert!(!accepting("2026-11-13T23:09:59Z"));
        assert!(accepting("2026-11-13T23:10:00Z"));
    }

    #[test]
    fn the_external_number_must_match_the_closing_year() {
        let events = vec![open_event("A", "B", [1, 1, 98])];
        let with_external = |closes: &str, number: u32, external: u32| {
            let mut g = grid("lotofoot7", number, closes, events.clone(), json!([]));
            g["externalDrawNumber"] = json!(external);
            parse_open_page(&page(resolver(vec![g])), at("2026-11-14T22:45:00Z"))
        };
        assert!(with_external("2026-11-15T13:55:00.000Z", 143, 6143).is_ok());
        let error = with_external("2026-11-15T13:55:00.000Z", 143, 7143).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("externalDrawNumber 7143 au lieu de 6143"),
            "{error}"
        );
        // Clôture le 2 janvier 2027 : la grille n°1 porte le chiffre de la nouvelle année (7001), non celui de 2026.
        assert!(with_external("2027-01-02T13:55:00.000Z", 1, 7001).is_ok());
        assert!(with_external("2027-01-02T13:55:00.000Z", 1, 6001).is_err());
        // 23 h 30 UTC le 31 décembre est déjà le 1er janvier à Paris : c'est le chiffre de 2027 qui compte.
        assert!(with_external("2026-12-31T23:30:00.000Z", 1, 7001).is_ok());
    }

    #[test]
    fn a_page_without_state_or_with_a_broken_state_is_an_error() {
        let now = at("2026-11-14T22:45:00Z");
        let error = parse_open_page("<html><body>Accès refusé</body></html>", now).unwrap_err();
        assert!(
            error.to_string().contains("serverApp-state introuvable"),
            "{error}"
        );

        let broken = "<script id=\"serverApp-state\">{&q;gridsResolver&q;:</script>";
        let error = parse_open_page(broken, now).unwrap_err();
        assert!(error.to_string().contains("illisible"), "{error}");

        let empty = "<script id=\"serverApp-state\">{&q;environment&q;:&q;x&q;}</script>";
        let error = parse_open_page(empty, now).unwrap_err();
        assert!(
            error.to_string().contains("gridsResolver absent"),
            "{error}"
        );
    }

    #[test]
    fn results_give_signs_centimes_as_euros_and_ranks_from_best_to_worst() {
        let events = vec![
            played_event(json!("firstOpponentWinner")),
            played_event(json!("draw")),
            played_event(json!("secondOpponentWinner")),
            played_event(json!("firstOpponentWinner")),
            played_event(json!("draw")),
            played_event(json!("draw")),
            played_event(json!("secondOpponentWinner")),
        ];
        // Rangs donnés dans le désordre : le rang 7 doit passer devant le rang 6.
        let ranks = json!([
            {"rankNumberMarket": 6, "rankNumberGrid": 26943, "rankAmount": 260},
            {"rankNumberMarket": 7, "rankNumberGrid": 1883, "rankAmount": 3100},
        ]);
        let lf7 = grid("lotofoot7", 140, "2026-11-10T17:40:00.000Z", events, ranks);
        let html = page(resolver(vec![lf7]));

        let result = parse_results_page(&html).unwrap();

        assert_eq!((result.total, result.offset, result.limit), (99, 0, 20));
        assert_eq!(result.results.len(), 1);
        let grid = &result.results[0];
        assert_eq!(grid.game, Some(Game::Lf7));
        assert_eq!(grid.number, 140);
        assert_eq!(grid.closes_at, Some(at("2026-11-10T17:40:00Z")));
        assert_eq!(grid.date, chrono::NaiveDate::from_ymd_opt(2026, 11, 10));
        assert_eq!(grid.matches[0].2, MatchResult::Sign(Outcome::Home));
        assert_eq!(grid.matches[1].2, MatchResult::Sign(Outcome::Draw));
        assert_eq!(grid.matches[2].2, MatchResult::Sign(Outcome::Away));
        assert_eq!(grid.matches[0].0, "Domicile");
        assert_eq!(
            grid.ranks,
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
    fn a_canceled_match_is_kept_as_such() {
        let mut events = vec![played_event(json!("draw")); 6];
        events.insert(4, played_event(json!("canceled")));
        let ranks = json!([
            {"rankNumberMarket": 7, "rankNumberGrid": 10, "rankAmount": 5000},
            {"rankNumberMarket": 6, "rankNumberGrid": 100, "rankAmount": 500},
        ]);
        let html = page(resolver(vec![grid(
            "lotofoot7",
            140,
            "2026-11-10T17:40:00.000Z",
            events,
            ranks,
        )]));

        let result = parse_results_page(&html).unwrap();

        assert_eq!(result.results[0].matches.len(), 7);
        assert_eq!(result.results[0].matches[4].2, MatchResult::Canceled);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn a_rank_without_winner_has_no_unit_gain() {
        let ranks = json!([
            {"rankNumberMarket": 7, "rankNumberGrid": 0, "rankAmount": 0},
            {"rankNumberMarket": 6, "rankNumberGrid": 3, "rankAmount": 2_946_500},
        ]);
        let events = vec![played_event(json!("draw")); 7];
        let html = page(resolver(vec![grid(
            "lotofoot7",
            5,
            "2026-11-10T17:40:00.000Z",
            events,
            ranks,
        )]));

        let ranks = &parse_results_page(&html).unwrap().results[0].ranks;

        assert_eq!(
            ranks[0],
            RankLine {
                good: Some(7),
                winners: 0,
                unit_gain: None
            }
        );
        assert_eq!(ranks[1].unit_gain, Euros::new(29_465.0));
    }

    #[test]
    fn ranks_all_at_zero_are_an_unpublished_report_not_a_report() {
        let ranks = json!([
            {"rankNumberMarket": 7, "rankNumberGrid": 0, "rankAmount": 0},
            {"rankNumberMarket": 6, "rankNumberGrid": 0, "rankAmount": 0},
        ]);
        let events = vec![played_event(json!("draw")); 7];
        let html = page(resolver(vec![grid(
            "lotofoot7",
            5,
            "2026-11-10T17:40:00.000Z",
            events,
            ranks,
        )]));

        let result = parse_results_page(&html).unwrap();

        assert_eq!(result.results.len(), 1);
        assert!(result.results[0].ranks.is_empty());
    }

    #[test]
    fn a_grid_with_a_result_still_missing_is_not_returned() {
        let mut events = vec![played_event(json!("draw")); 6];
        events.push(played_event(Value::Null));
        let html = page(resolver(vec![
            grid(
                "lotofoot7",
                142,
                "2026-11-14T16:55:00.000Z",
                events,
                json!([]),
            ),
            grid(
                "lotofoot7",
                141,
                "2026-11-12T16:55:00.000Z",
                vec![played_event(json!("draw")); 7],
                json!([{"rankNumberMarket": 7, "rankNumberGrid": 1, "rankAmount": 100}]),
            ),
        ]));

        let result = parse_results_page(&html).unwrap();

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].number, 141);
        assert!(
            result.warnings.is_empty(),
            "un résultat manquant est normal"
        );
    }

    #[test]
    fn an_unknown_result_label_is_a_warning_and_spares_the_other_grids() {
        let mut odd = vec![played_event(json!("draw")); 6];
        odd.push(played_event(json!("postponed")));
        let fine = vec![played_event(json!("draw")); 7];
        let ranks = json!([{"rankNumberMarket": 7, "rankNumberGrid": 1, "rankAmount": 100}]);
        let html = page(resolver(vec![
            grid("lotofoot7", 142, "2026-11-14T16:55:00.000Z", odd, json!([])),
            grid("lotofoot7", 141, "2026-11-12T16:55:00.000Z", fine, ranks),
        ]));

        let result = parse_results_page(&html).unwrap();

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.warnings.len(), 1);
        assert!(
            result.warnings[0].contains("LF7 n°142") && result.warnings[0].contains("postponed")
        );
    }

    #[test]
    fn a_fully_canceled_draw_is_listed_apart() {
        let mut lf8 = grid(
            "lotofoot8",
            69,
            "2026-11-10T17:40:00.000Z",
            vec![played_event(json!("canceled")); 8],
            json!([
                {"rankNumberMarket": 0, "rankNumberGrid": 0, "rankAmount": 0},
                {"rankNumberMarket": 0, "rankNumberGrid": 0, "rankAmount": 0},
            ]),
        );
        lf8["drawCancelled"] = json!(true);
        let html = page(resolver(vec![lf8]));

        let result = parse_results_page(&html).unwrap();

        assert!(result.results.is_empty());
        assert_eq!(
            result.canceled_draws,
            vec![GridId {
                game: Game::Lf8,
                year: 2026,
                number: 69
            }]
        );
    }

    #[test]
    fn results_of_other_games_and_a_missing_pagination_are_handled() {
        let lf12 = grid(
            "lotofoot12",
            96,
            "2026-11-05T17:40:00.000Z",
            vec![played_event(json!("draw")); 12],
            json!([]),
        );
        let html = page(resolver(vec![lf12]));
        let result = parse_results_page(&html).unwrap();
        assert!(result.results.is_empty() && result.warnings.is_empty());

        let html = page(json!({"grids": []}));
        let error = parse_results_page(&html).unwrap_err();
        assert!(error.to_string().contains("pagination absente"), "{error}");
    }

    #[test]
    fn results_pages_are_numbered_from_one() {
        assert_eq!(results_path(1), "grilles/resultats/loto-foot");
        assert_eq!(results_path(3), "grilles/resultats/loto-foot?page=3");
    }

    /// Contrôle sur de vraies pages, hors dépôt : `EDGE_FDJ_PAGES=<dossier> cargo test -- --ignored fdj::tests::real_pages`.
    /// Le dossier contient des copies de pages FDJ : `fdj_open_*.html` (grilles ouvertes ou annoncées) et
    /// `fdj_results_*.html` (résultats).
    #[test]
    #[ignore = "lit des pages réelles hors dépôt"]
    fn real_pages_are_read_without_error() {
        let Ok(dir) = std::env::var("EDGE_FDJ_PAGES") else {
            return;
        };
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let html = std::fs::read_to_string(&path).unwrap();
            if name.starts_with("fdj_open_") {
                let grids = parse_open_page(&html, at("2026-11-14T22:45:00Z"))
                    .unwrap_or_else(|e| panic!("{name} : {e}"));
                for g in &grids {
                    println!(
                        "{name} : {} n°{} clôture {} accepte {} pactole {:?} {} rencontres, répartition {}",
                        g.game,
                        g.number,
                        g.closes_at,
                        g.accepting,
                        g.jackpot,
                        g.matches.len(),
                        g.split.is_some()
                    );
                }
            } else if name.starts_with("fdj_results_") {
                let page = parse_results_page(&html).unwrap_or_else(|e| panic!("{name} : {e}"));
                println!(
                    "{name} : {} grilles lues sur {} ({} à partir de {}), {} annulées, {:?}",
                    page.results.len(),
                    page.total,
                    page.limit,
                    page.offset,
                    page.canceled_draws.len(),
                    page.warnings
                );
                for r in &page.results {
                    println!(
                        "    {:?} n°{} {:?} {} rencontres, {} rangs, annulées {}",
                        r.game,
                        r.number,
                        r.date,
                        r.matches.len(),
                        r.ranks.len(),
                        r.matches
                            .iter()
                            .filter(|m| m.2 == MatchResult::Canceled)
                            .count()
                    );
                }
            } else {
                continue;
            }
            checked += 1;
        }
        assert!(checked > 0, "aucune page lue");
    }
}
