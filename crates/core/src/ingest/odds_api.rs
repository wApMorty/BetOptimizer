//! Format de The Odds API : construction des requêtes et lecture des réponses.
//!
//! Tout est pur (aucun réseau) pour être testé sur des réponses réelles enregistrées dans
//! `tests/fixtures/`. L'appel HTTP lui-même est dans le module `client`.
//! Source de vérité : `docs/sources.md` (vérifications du 2026-10-18).

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::config::ApiKey;
use crate::domain::{Event, Odds, Quote};
use crate::error::{Error, Result};

/// Racine de l'API.
pub const BASE_URL: &str = "https://api.the-odds-api.com/v4";

/// Chemin du point d'accès gratuit qui sert à lire le quota.
pub const SPORTS_PATH: &str = "/sports";

/// Chemin des cotes d'une compétition.
pub fn odds_path(sport_key: &str) -> String {
    format!("/sports/{sport_key}/odds")
}

/// Chemin des cotes d'un seul match : `/sports/{ligue}/events/{match}/odds`.
///
/// Renvoie `None` si l'identifiant du match contient autre chose que des lettres et des chiffres : il vient de la base
/// mais se glisse dans le chemin d'une URL, donc on refuse tout caractère qui pourrait la détourner.
pub fn event_odds_path(sport_key: &str, event_id: &str) -> Option<String> {
    let safe = !event_id.is_empty() && event_id.chars().all(|c| c.is_ascii_alphanumeric());
    safe.then(|| format!("/sports/{sport_key}/events/{event_id}/odds"))
}

/// Paramètres journalisés d'un appel par match : l'identifiant du match (dans le chemin de la requête) est noté en
/// tête pour que le journal des appels dise de quel match il s'agit, puis viennent les vrais paramètres.
pub fn event_odds_params(event_id: &str, markets: &[String], bookmakers: &[String]) -> String {
    format!("eventId={event_id}&{}", odds_params(markets, bookmakers))
}

/// Paramètres de la requête `/odds`, **sans la clé API** : c'est ce texte qui est journalisé.
///
/// Sans paramètre `regions`, `bookmakers` fait foi (documentation) ; jusqu'à 10 bookmakers comptent
/// pour une seule région. Cotes décimales, dates ISO 8601 : valeurs par défaut, écrites pour être explicites.
pub fn odds_params(markets: &[String], bookmakers: &[String]) -> String {
    format!(
        "markets={}&bookmakers={}&oddsFormat=decimal&dateFormat=iso",
        markets.join(","),
        bookmakers.join(",")
    )
}

/// Paramètres réellement envoyés pour un appel par match : ceux du journal, sans l'identifiant noté en tête.
pub fn event_request_params(logged: &str) -> &str {
    logged.split_once('&').map_or(logged, |(_, rest)| rest)
}

/// URL complète d'une requête, clé API comprise. Cette chaîne ne doit jamais être journalisée.
pub fn request_url(path: &str, params: &str, key: &ApiKey) -> String {
    let separator = if params.is_empty() { "" } else { "&" };
    format!(
        "{BASE_URL}{path}?apiKey={}{separator}{params}",
        key.expose()
    )
}

// Les structures `Raw*` reflètent exactement le JSON de l'API. On les convertit aussitôt en types du
// domaine : le reste du programme ne dépend pas de la forme de la réponse.
#[derive(Deserialize)]
struct RawEvent {
    id: String,
    sport_key: String,
    commence_time: DateTime<Utc>,
    home_team: String,
    away_team: String,
    bookmakers: Vec<RawBookmaker>,
}

#[derive(Deserialize)]
struct RawBookmaker {
    key: String,
    markets: Vec<RawMarket>,
}

#[derive(Deserialize)]
struct RawMarket {
    key: String,
    last_update: DateTime<Utc>,
    outcomes: Vec<RawOutcome>,
}

#[derive(Deserialize)]
struct RawOutcome {
    name: String,
    price: f64,
    point: Option<f64>,
}

/// Résultat de la lecture d'une réponse `/odds`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedOdds {
    /// Matchs à venir, avec leurs cotes.
    pub events: Vec<Event>,
    /// Matchs déjà commencés au moment du relevé, ignorés : leurs cotes sont très probablement
    /// en direct et n'ont pas leur place dans une archive d'avant-match.
    pub already_started: usize,
    /// Marchés écartés parce qu'une de leurs cotes était invalide (≤ 1,0, par exemple).
    pub dropped_markets: usize,
}

/// Lit le corps d'une réponse `/odds` réussie.
///
/// `fetched_at` est l'instant du relevé : tout match dont le coup d'envoi lui est antérieur ou égal
/// est ignoré.
pub fn parse_odds(body: &str, fetched_at: DateTime<Utc>) -> Result<ParsedOdds> {
    let raw: Vec<RawEvent> = serde_json::from_str(body)?;
    let mut parsed = ParsedOdds {
        events: Vec::new(),
        already_started: 0,
        dropped_markets: 0,
    };
    // `for raw_event in raw` consomme le vecteur : chaque `String` est déplacée dans l'`Event`
    // sans copie.
    for raw_event in raw {
        add_event(&mut parsed, raw_event, fetched_at);
    }
    Ok(parsed)
}

/// Lit le corps d'une réponse `/events/{match}/odds` réussie : un seul match, pas une liste.
pub fn parse_event_odds(body: &str, fetched_at: DateTime<Utc>) -> Result<ParsedOdds> {
    let raw: RawEvent = serde_json::from_str(body)?;
    let mut parsed = ParsedOdds {
        events: Vec::new(),
        already_started: 0,
        dropped_markets: 0,
    };
    add_event(&mut parsed, raw, fetched_at);
    Ok(parsed)
}

/// Ajoute un match au résultat, ou le compte comme « déjà commencé ».
fn add_event(parsed: &mut ParsedOdds, raw_event: RawEvent, fetched_at: DateTime<Utc>) {
    if raw_event.commence_time <= fetched_at {
        parsed.already_started += 1;
        return;
    }
    let mut quotes = Vec::new();
    // Le `to_string()` n'est nécessaire que pour la clé du bookmaker, partagée par plusieurs cotes.
    for bookmaker in raw_event.bookmakers {
        for market in bookmaker.markets {
            match market_quotes(&bookmaker.key, market) {
                Some(market_quotes) => quotes.extend(market_quotes),
                None => {
                    parsed.dropped_markets += 1;
                    tracing::warn!(
                        bookmaker = %bookmaker.key,
                        home = %raw_event.home_team,
                        away = %raw_event.away_team,
                        "marché écarté : une cote est invalide"
                    );
                }
            }
        }
    }
    parsed.events.push(Event {
        id: raw_event.id,
        sport_key: raw_event.sport_key,
        commence_time: raw_event.commence_time,
        home_team: raw_event.home_team,
        away_team: raw_event.away_team,
        quotes,
    });
}

/// Convertit les issues d'un marché en cotes. Renvoie `None` si une seule cote est invalide :
/// un marché 1X2 amputé d'une issue ne signifierait rien, on l'écarte en entier.
fn market_quotes(bookmaker_key: &str, market: RawMarket) -> Option<Vec<Quote>> {
    // `collect` vers `Option<Vec<_>>` : le premier `None` produit par la fermeture arrête tout et
    // le résultat est `None` ; sinon on obtient `Some(le vecteur complet)`.
    market
        .outcomes
        .into_iter()
        .map(|outcome| {
            Some(Quote {
                bookmaker_key: bookmaker_key.to_string(),
                market_key: market.key.clone(),
                updated_at: market.last_update,
                outcome: outcome.name,
                point: outcome.point,
                price: Odds::new(outcome.price)?,
            })
        })
        .collect()
}

/// Bookmakers demandés qui n'ont aucune cote dans les matchs à venir.
///
/// Une clé mal orthographiée est facturée par l'API sans erreur (chaque match revient avec une liste
/// de bookmakers vide) : ce contrôle la signale. Un bookmaker qui ne couvre simplement pas la ligue
/// (Winamax et la Premiership écossaise) apparaît aussi, ce qui est une information utile.
/// Sans match à venir (ligue en pause), il n'y a rien à signaler.
///
/// Le `'a` indique que les noms renvoyés sont empruntés à `requested` : aucune copie.
pub fn bookmakers_without_quotes<'a>(requested: &'a [String], events: &[Event]) -> Vec<&'a str> {
    if events.is_empty() {
        return Vec::new();
    }
    requested
        .iter()
        .map(String::as_str)
        .filter(|name| {
            !events.iter().any(|event| {
                event
                    .quotes
                    .iter()
                    .any(|quote| quote.bookmaker_key == *name)
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct RawSport {
    key: String,
    active: bool,
    #[serde(default)]
    has_outrights: bool,
}

/// Préfixe des clés de tennis de The Odds API : une clé **par tournoi**, présente seulement pendant le tournoi.
pub const TENNIS_PREFIX: &str = "tennis_";

/// Clés des tournois de tennis en cours, lues dans la réponse (gratuite) de `/sports`, triées par nom.
///
/// Les marchés « vainqueur du tournoi » (`has_outrights`) n'ont pas de marché 1 contre 1 : ils sont écartés.
pub fn active_tennis_keys(sports_body: &str) -> Result<Vec<String>> {
    let sports: Vec<RawSport> = serde_json::from_str(sports_body)?;
    let mut keys: Vec<String> = sports
        .into_iter()
        .filter(|s| s.active && !s.has_outrights && s.key.starts_with(TENNIS_PREFIX))
        .map(|s| s.key)
        .collect();
    keys.sort();
    Ok(keys)
}

/// Chemin des résultats d'une compétition (la barre finale est celle de la documentation).
pub fn scores_path(sport_key: &str) -> String {
    format!("/sports/{sport_key}/scores/")
}

/// Paramètres des résultats : les `days` derniers jours (de 1 à 3, le maximum de l'API). Coût : 2 crédits.
pub fn scores_params(days: u8) -> String {
    format!("daysFrom={days}&dateFormat=iso")
}

/// Un match terminé, avec son score final.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedGame {
    /// Identifiant du match (le même que dans `/odds`).
    pub event_id: String,
    /// Coup d'envoi.
    pub commence_time: DateTime<Utc>,
    /// Équipe à domicile.
    pub home_team: String,
    /// Équipe à l'extérieur.
    pub away_team: String,
    /// Points de l'équipe à domicile.
    pub home_score: u16,
    /// Points de l'équipe à l'extérieur.
    pub away_score: u16,
}

/// Résultat de la lecture d'une réponse `/scores`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedScores {
    /// Matchs terminés dont les deux scores sont lisibles.
    pub finished: Vec<FinishedGame>,
    /// Matchs à venir ou en cours (pas de score final).
    pub not_finished: usize,
    /// Matchs déclarés terminés mais dont un score manque ou est illisible : signalés, jamais devinés.
    pub malformed: usize,
}

#[derive(Deserialize)]
struct RawScoredGame {
    id: String,
    commence_time: DateTime<Utc>,
    completed: bool,
    home_team: String,
    away_team: String,
    scores: Option<Vec<RawTeamScore>>,
}

#[derive(Deserialize)]
struct RawTeamScore {
    name: String,
    score: String,
}

/// Lit le corps d'une réponse `/scores` réussie.
///
/// Les scores sont des textes (`"112"`) attachés à un **nom d'équipe** : on les rapproche des équipes du match par le nom exact.
pub fn parse_scores(body: &str) -> Result<ParsedScores> {
    let raw: Vec<RawScoredGame> = serde_json::from_str(body)?;
    let mut parsed = ParsedScores {
        finished: Vec::new(),
        not_finished: 0,
        malformed: 0,
    };
    for game in raw {
        if !game.completed {
            parsed.not_finished += 1;
            continue;
        }
        let score_of = |team: &str| -> Option<u16> {
            game.scores
                .as_ref()?
                .iter()
                .find(|s| s.name == team)?
                .score
                .trim()
                .parse()
                .ok()
        };
        match (score_of(&game.home_team), score_of(&game.away_team)) {
            (Some(home_score), Some(away_score)) => parsed.finished.push(FinishedGame {
                event_id: game.id,
                commence_time: game.commence_time,
                home_team: game.home_team,
                away_team: game.away_team,
                home_score,
                away_score,
            }),
            _ => parsed.malformed += 1,
        }
    }
    Ok(parsed)
}

/// Chemin de la liste des matchs à venir d'une compétition (`/events`, gratuit d'après la documentation et vérifié le
/// 2026-10-18 : `docs/sources.md` §2).
pub fn events_path(sport_key: &str) -> String {
    format!("/sports/{sport_key}/events")
}

/// Un match listé par `/events` (sans cotes).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ListedEvent {
    /// Identifiant The Odds API.
    pub id: String,
    /// Clé de la compétition.
    pub sport_key: String,
    /// Coup d'envoi (UTC).
    pub commence_time: DateTime<Utc>,
    /// Équipe à domicile, nom brut.
    pub home_team: String,
    /// Équipe à l'extérieur, nom brut.
    pub away_team: String,
}

/// Lit le corps d'une réponse `/events` réussie.
pub fn parse_events(body: &str) -> Result<Vec<ListedEvent>> {
    Ok(serde_json::from_str(body)?)
}

/// Une compétition est-elle active (en saison) selon la réponse gratuite de `/sports` ?
pub fn sport_is_active(sports_body: &str, sport_key: &str) -> Result<bool> {
    let sports: Vec<RawSport> = serde_json::from_str(sports_body)?;
    Ok(sports.iter().any(|s| s.key == sport_key && s.active))
}

#[derive(Deserialize)]
struct RawApiError {
    message: String,
    error_code: Option<String>,
}

/// Construit l'erreur à partir du corps d'une réponse HTTP en échec (`{"message": …, "error_code": …}`).
/// Si le corps n'est pas ce JSON, on en garde le début tel quel.
pub fn api_error(status: u16, body: &str) -> Error {
    match serde_json::from_str::<RawApiError>(body) {
        Ok(raw) => Error::Api {
            status,
            code: raw.error_code.unwrap_or_else(|| "?".to_string()),
            message: raw.message,
        },
        Err(_) => Error::Api {
            status,
            code: "?".to_string(),
            message: body.chars().take(200).collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn listed_events_are_read_without_odds() {
        let body = r#"[{"id":"e1","sport_key":"soccer_france_ligue_two","sport_title":"Ligue 2 - France",
            "commence_time":"2026-10-23T18:00:00Z","home_team":"Pau FC","away_team":"Grenoble"}]"#;
        let events = super::parse_events(body).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].home_team, "Pau FC");
        assert_eq!(
            super::events_path("soccer_france_ligue_two"),
            "/sports/soccer_france_ligue_two/events"
        );
    }

    use super::*;

    const EPL: &str = include_str!("../../tests/fixtures/odds_epl.json");
    const SERIE_A_STARTED: &str = include_str!("../../tests/fixtures/odds_serie_a_started.json");
    const SPL: &str = include_str!("../../tests/fixtures/odds_spl_pinnacle_only.json");
    const UNKNOWN_BOOKMAKER: &str =
        include_str!("../../tests/fixtures/odds_unknown_bookmaker.json");
    const EMPTY: &str = include_str!("../../tests/fixtures/odds_empty.json");
    const INVALID_KEY: &str = include_str!("../../tests/fixtures/error_invalid_key.json");
    const INVALID_MARKET: &str = include_str!("../../tests/fixtures/error_invalid_market.json");

    /// Instant du relevé réel dont proviennent les fixtures (UTC).
    fn fetched_at() -> DateTime<Utc> {
        "2026-10-18T10:58:30Z".parse().unwrap()
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn price(event: &Event, bookmaker: &str, outcome: &str) -> f64 {
        event
            .quotes
            .iter()
            .find(|quote| quote.bookmaker_key == bookmaker && quote.outcome == outcome)
            .unwrap()
            .price
            .get()
    }

    #[test]
    fn params_never_contain_the_api_key_but_the_url_does_once() {
        let key = ApiKey::new("CLE-SECRETE");
        let params = odds_params(&names(&["h2h"]), &names(&["pinnacle", "winamax_fr"]));

        assert_eq!(
            params,
            "markets=h2h&bookmakers=pinnacle,winamax_fr&oddsFormat=decimal&dateFormat=iso"
        );
        assert!(!params.contains("CLE-SECRETE"));

        let url = request_url(&odds_path("soccer_epl"), &params, &key);
        assert!(url.starts_with(
            "https://api.the-odds-api.com/v4/sports/soccer_epl/odds?apiKey=CLE-SECRETE&markets=h2h"
        ));
        assert_eq!(url.matches("CLE-SECRETE").count(), 1);
    }

    #[test]
    fn url_without_params_has_no_dangling_ampersand() {
        let url = request_url(SPORTS_PATH, "", &ApiKey::new("k"));
        assert_eq!(url, "https://api.the-odds-api.com/v4/sports?apiKey=k");
    }

    #[test]
    fn epl_response_is_parsed_with_every_quote() {
        let parsed = parse_odds(EPL, fetched_at()).unwrap();

        assert_eq!(parsed.events.len(), 3);
        assert_eq!(parsed.already_started, 0);
        assert_eq!(parsed.dropped_markets, 0);
        let first = &parsed.events[0];
        assert_eq!(first.home_team, "Bournemouth");
        assert_eq!(first.commence_time.to_string(), "2026-10-18 13:00:00 UTC");
        // 3 bookmakers × 3 issues.
        assert_eq!(first.quotes.len(), 9);
        assert_eq!(price(first, "winamax_fr", "Bournemouth"), 2.1);
        assert_eq!(price(first, "pinnacle", "Draw"), 3.38);
        // Le match lointain n'a pas encore de Pinnacle : 2 bookmakers × 3 issues.
        let far = parsed
            .events
            .iter()
            .find(|e| e.home_team == "Chelsea")
            .unwrap();
        assert_eq!(far.quotes.len(), 6);
    }

    #[test]
    fn started_match_is_ignored_and_counted() {
        let parsed = parse_odds(SERIE_A_STARTED, fetched_at()).unwrap();

        assert_eq!(parsed.already_started, 1);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].home_team, "Fiorentina");
    }

    #[test]
    fn a_match_starting_exactly_now_counts_as_started() {
        // Deux des trois matchs de la fixture commencent à 13:00:00Z pile : la borne est incluse.
        let parsed = parse_odds(EPL, "2026-10-18T13:00:00Z".parse().unwrap()).unwrap();
        assert_eq!(parsed.already_started, 2);
        assert_eq!(parsed.events.len(), 1);
    }

    #[test]
    fn outcomes_are_matched_by_name_never_by_position() {
        // Piège documenté dans docs/sources.md : l'API classe les issues par ordre alphabétique des
        // équipes (ici « Lecce » avant « Udinese », alors que Udinese reçoit). On vérifie que chaque
        // cote reste attachée au bon nom, quelle que soit sa position.
        let parsed = parse_odds(SERIE_A_STARTED, "2026-10-18T10:00:00Z".parse().unwrap()).unwrap();
        let udinese = parsed
            .events
            .iter()
            .find(|e| e.home_team == "Udinese")
            .unwrap();

        assert_eq!(price(udinese, "pinnacle", "Lecce"), 2.13);
        assert_eq!(price(udinese, "pinnacle", "Udinese"), 3.8);
        assert_eq!(udinese.quotes[0].outcome, "Lecce");
    }

    #[test]
    fn invalid_price_drops_the_whole_market_only() {
        let body = EPL.replacen("\"price\": 2.1", "\"price\": 1.0", 1);

        let parsed = parse_odds(&body, fetched_at()).unwrap();

        assert_eq!(parsed.dropped_markets, 1);
        // Premier match : 9 cotes moins les 3 du marché écarté (Winamax).
        assert_eq!(parsed.events[0].quotes.len(), 6);
        assert!(
            parsed.events[0]
                .quotes
                .iter()
                .all(|quote| quote.bookmaker_key != "winamax_fr")
        );
    }

    #[test]
    fn empty_response_gives_no_event() {
        let parsed = parse_odds(EMPTY, fetched_at()).unwrap();
        assert!(parsed.events.is_empty());
    }

    #[test]
    fn garbled_body_is_an_error() {
        assert!(matches!(
            parse_odds("<html>502</html>", fetched_at()).unwrap_err(),
            Error::ResponseFormat(_)
        ));
    }

    #[test]
    fn scottish_league_has_pinnacle_only() {
        let parsed = parse_odds(SPL, fetched_at()).unwrap();
        let requested = names(&["pinnacle", "winamax_fr", "betclic_fr"]);

        assert_eq!(
            bookmakers_without_quotes(&requested, &parsed.events),
            vec!["winamax_fr", "betclic_fr"]
        );
    }

    #[test]
    fn unknown_bookmaker_is_detected_because_every_event_comes_back_empty() {
        let parsed = parse_odds(UNKNOWN_BOOKMAKER, fetched_at()).unwrap();
        let requested = names(&["bookmaker_bidon"]);

        assert_eq!(parsed.events.len(), 2);
        assert_eq!(
            bookmakers_without_quotes(&requested, &parsed.events),
            vec!["bookmaker_bidon"]
        );
    }

    #[test]
    fn nothing_is_reported_when_the_league_has_no_upcoming_match() {
        let requested = names(&["pinnacle"]);
        assert!(bookmakers_without_quotes(&requested, &[]).is_empty());
    }

    #[test]
    fn real_error_bodies_are_turned_into_readable_errors() {
        let message = api_error(401, INVALID_KEY).to_string();
        assert!(message.contains("401"));
        assert!(message.contains("INVALID_KEY"));

        let message = api_error(422, INVALID_MARKET).to_string();
        assert!(message.contains("INVALID_MARKET"));
        assert!(message.contains("Invalid markets"));
    }

    #[test]
    fn non_json_error_body_is_kept_truncated() {
        let long = "x".repeat(500);
        let message = api_error(502, &long).to_string();
        assert!(message.contains("502"));
        assert!(message.len() < 400);
    }

    #[test]
    fn a_recorded_single_event_response_is_read() {
        // Réponse réelle de `/sports/soccer_spain_la_liga/events/{match}/odds?markets=h2h&bookmakers=pinnacle`
        // (2026-10-18, coût mesuré : 1 crédit). Voir docs/sources.md.
        let body = include_str!("../../tests/fixtures/odds_event_pinnacle.json");

        let parsed = parse_event_odds(body, "2026-10-18T12:00:00Z".parse().unwrap()).unwrap();

        assert_eq!(parsed.events.len(), 1);
        let event = &parsed.events[0];
        assert_eq!(
            (event.home_team.as_str(), event.away_team.as_str()),
            ("Levante", "Athletic Bilbao")
        );
        let pinnacle: Vec<_> = event
            .quotes
            .iter()
            .filter(|q| q.bookmaker_key == "pinnacle")
            .collect();
        assert_eq!(pinnacle.len(), 3);
        assert!(pinnacle.iter().any(|q| q.outcome == "Draw"));
    }

    #[test]
    fn only_active_tennis_tournaments_are_listed_from_the_sports_response() {
        // Réponse réelle de `GET /v4/sports` du 2026-10-18, réduite à quelques sports.
        let body = include_str!("../../tests/fixtures/sports_with_tennis.json");

        assert_eq!(
            active_tennis_keys(body).unwrap(),
            vec!["tennis_wta_ningbo_open".to_string()]
        );
        assert!(active_tennis_keys("[]").unwrap().is_empty());
        let inactive = r#"[{"key":"tennis_atp_x","active":false},{"key":"tennis_atp_y_winner","active":true,"has_outrights":true}]"#;
        assert!(active_tennis_keys(inactive).unwrap().is_empty());
        assert!(active_tennis_keys("pas du json").is_err());
    }

    #[test]
    fn finished_games_are_read_with_their_scores_matched_by_team_name() {
        // Réponse réelle de `/scores` (2026-10-19, matchs à venir : `scores: null`) dont le premier match a été complété
        // à la main d'après la documentation (`"scores": [{"name": …, "score": "112"}, …]`).
        let body = include_str!("../../tests/fixtures/nba_scores.json");

        let parsed = parse_scores(body).unwrap();

        assert_eq!(parsed.finished.len(), 1);
        assert_eq!(parsed.not_finished, 2);
        assert_eq!(parsed.malformed, 0);
        let game = &parsed.finished[0];
        assert_eq!(
            (game.home_team.as_str(), game.away_team.as_str()),
            ("Detroit Pistons", "Boston Celtics")
        );
        assert_eq!((game.home_score, game.away_score), (112, 105));
    }

    #[test]
    fn a_completed_game_with_a_missing_or_garbled_score_is_counted_not_guessed() {
        let body = r#"[
          {"id":"a","commence_time":"2026-10-20T23:00:00Z","completed":true,"home_team":"A","away_team":"B","scores":null},
          {"id":"b","commence_time":"2026-10-20T23:00:00Z","completed":true,"home_team":"A","away_team":"B",
           "scores":[{"name":"A","score":"100"},{"name":"Autre","score":"90"}]},
          {"id":"c","commence_time":"2026-10-20T23:00:00Z","completed":true,"home_team":"A","away_team":"B",
           "scores":[{"name":"A","score":"cent"},{"name":"B","score":"90"}]}]"#;

        let parsed = parse_scores(body).unwrap();

        assert!(parsed.finished.is_empty());
        assert_eq!(parsed.malformed, 3);
    }

    #[test]
    fn a_sport_is_active_only_when_listed_as_such() {
        let body = include_str!("../../tests/fixtures/sports_with_tennis.json");

        assert!(sport_is_active(body, "soccer_epl").unwrap());
        assert!(
            !sport_is_active(body, "basketball_nba").unwrap(),
            "absent de cette liste réduite"
        );
        assert!(!sport_is_active("[]", "basketball_nba").unwrap());
        assert!(sport_is_active("pas du json", "x").is_err());
        assert_eq!(
            scores_path("basketball_nba"),
            "/sports/basketball_nba/scores/"
        );
        assert_eq!(scores_params(3), "daysFrom=3&dateFormat=iso");
    }

    #[test]
    fn the_event_path_refuses_anything_but_letters_and_digits() {
        assert_eq!(
            event_odds_path("soccer_epl", "f0153980fbafeb84"),
            Some("/sports/soccer_epl/events/f0153980fbafeb84/odds".to_string())
        );
        assert_eq!(event_odds_path("soccer_epl", "a/../b"), None);
        assert_eq!(event_odds_path("soccer_epl", "a?x=1"), None);
        assert_eq!(event_odds_path("soccer_epl", ""), None);
    }

    #[test]
    fn the_logged_params_name_the_match_and_the_sent_params_do_not() {
        let markets = vec!["h2h".to_string()];
        let bookmakers = vec!["pinnacle".to_string()];

        let logged = event_odds_params("abc123", &markets, &bookmakers);

        assert_eq!(
            logged,
            "eventId=abc123&markets=h2h&bookmakers=pinnacle&oddsFormat=decimal&dateFormat=iso"
        );
        assert_eq!(
            event_request_params(&logged),
            "markets=h2h&bookmakers=pinnacle&oddsFormat=decimal&dateFormat=iso"
        );
    }

    #[test]
    fn a_single_event_response_is_read_like_one_element_of_the_list() {
        let body = r#"{"id":"e1","sport_key":"soccer_spl","commence_time":"2026-10-18T14:00:00Z",
            "home_team":"Celtic","away_team":"Hearts","bookmakers":[{"key":"pinnacle","title":"Pinnacle",
            "markets":[{"key":"h2h","last_update":"2026-10-18T10:58:27Z","outcomes":[
            {"name":"Celtic","price":1.56},{"name":"Hearts","price":5.21},{"name":"Draw","price":4.92}]}]}]}"#;

        let before = parse_event_odds(body, "2026-10-18T13:30:00Z".parse().unwrap()).unwrap();
        let after = parse_event_odds(body, "2026-10-18T14:30:00Z".parse().unwrap()).unwrap();

        assert_eq!(before.events.len(), 1);
        assert_eq!(before.events[0].quotes.len(), 3);
        assert_eq!(before.events[0].id, "e1");
        assert!(
            after.events.is_empty(),
            "un match commencé n'est pas archivé"
        );
        assert_eq!(after.already_started, 1);
    }
}
