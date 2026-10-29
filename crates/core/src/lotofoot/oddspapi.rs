//! Repli OddsPapi des séances Loto Foot : cote Pinnacle d'une rencontre que notre archive The Odds API n'a pas
//! (coupes nationales, compétitions non couvertes ; `docs/sources.md` §62).
//!
//! 1. `fixtures` (1 requête décomptée) liste les matchs à venir de la fenêtre. Un match est retenu si ses deux noms
//!    OddsPapi, traduits par la section `LOTOFOOT` de la table d'alias, sont exactement ceux de la rencontre : jamais
//!    d'appariement approximatif.
//! 2. `historical-odds` (non décompté, 5,3 s entre deux appels) donne l'historique Pinnacle du match : on garde le dernier
//!    prix de chaque issue connu au relevé, archivé dans `lotofoot_oddspapi_quote`.
//! 3. Une rencontre introuvable est signalée avec des pistes (matchs OddsPapi dont un nom contient celui cherché), qui
//!    aident à écrire l'alias et ne servent jamais d'appariement.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::Connection;

use super::odds::{ALIAS_SECTION, MatchMarket, Source};
use crate::config::Config;
use crate::domain::Odds1x2;
use crate::error::Result;
use crate::format::kickoff_label;
use crate::ingest::aliases::Aliases;
use crate::ingest::budget;
use crate::ingest::odds_api::api_error;
use crate::ingest::oddspapi::fetch::{Pause, get, read_usage};
use crate::ingest::oddspapi::format::{self, Fixture, History};
use crate::ingest::oddspapi::{HISTORY_COOLDOWN, OTHER_COOLDOWN};
use crate::ingest::snapshot::{Clock, Fetch};
use crate::market::margin::{MarginMethod, remove_margin};
use crate::store::oddspapi::{self as store, LotofootQuote, NewCall};

/// Football chez OddsPapi (`sportId`).
const SOCCER: u32 = 10;

/// Statut d'un match à venir (`statusId` 0).
const STATUS_UPCOMING: u8 = 0;

/// Bookmaker de référence, au nom d'OddsPapi.
const REFERENCE: &str = "pinnacle";

/// Marché 1X2 et ses trois issues (domicile, nul, extérieur) chez OddsPapi.
const MARKET_1X2: u32 = 101;
const OUTCOMES_1X2: [u32; 3] = [101, 102, 103];

/// Écart maximal entre les bornes `from` et `to` de `fixtures` sans `tournamentId` : l'API exige moins de 10 jours
/// (HTTP 400 « must be under 10 days apart », vérifié le 2026-10-29).
const MAX_WINDOW_DAYS: i64 = 9;

/// Pistes affichées au plus par rencontre introuvable.
const HINTS_PER_MATCH: usize = 3;

/// Accès réseau du repli : l'appel à OddsPapi et la pause entre deux appels (simulés dans les tests).
pub struct Access<'a> {
    /// `GET` sur un point d'accès d'OddsPapi, sans la clé.
    pub fetch: Fetch<'a>,
    /// Attente entre deux appels.
    pub pause: Pause<'a>,
}

/// Bilan du repli.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Search {
    /// Requêtes décomptées par OddsPapi.
    pub spent: u32,
    /// Rencontres repêchées : « Benfica – Gil Vicente (SL Benfica – Gil Vicente Barcelos, prix du jeu. 29/10 18:05) ».
    pub found: Vec<String>,
    /// Rencontres ou appels refusés, avec la raison.
    pub refused: Vec<String>,
    /// Pistes pour les rencontres introuvables (jamais utilisées comme appariement).
    pub hints: Vec<String>,
}

/// Cherche chez OddsPapi les rencontres `wanted` (noms canoniques) dont le coup d'envoi tombe dans `window`, et archive
/// la cote Pinnacle de celles qui sont trouvées. Une panne d'OddsPapi n'arrête pas la séance : elle est notée dans le
/// bilan, et les grilles concernées restent écartées.
pub fn refresh(
    conn: &mut Connection,
    config: &Config,
    aliases: &Aliases,
    wanted: &[(String, String)],
    window: (DateTime<Utc>, DateTime<Utc>),
    now: Clock,
    access: Access,
) -> Result<Search> {
    let mut search = Search::default();
    if wanted.is_empty() {
        return Ok(search);
    }
    let Access { fetch, pause } = access;
    // `account` n'est pas décompté : il sert à la garde de budget (une requête pour `fixtures`).
    let usage = match read_usage(conn, now, fetch, pause) {
        Ok(usage) => usage,
        Err(error) => {
            search.refused.push(format!("OddsPapi : {error}"));
            return Ok(search);
        }
    };
    if let Err(refusal) = budget::check(Some(usage.used), 1, config.oddspapi_monthly_cap) {
        search.refused.push(format!("OddsPapi : {refusal}"));
        return Ok(search);
    }
    let fixtures = match upcoming(conn, window, now, fetch, pause) {
        Ok(fixtures) => fixtures,
        Err(error) => {
            search.spent = 1;
            search.refused.push(format!("OddsPapi : {error}"));
            return Ok(search);
        }
    };
    search.spent = 1;

    for (home, away) in wanted {
        let label = format!("{home} – {away}");
        let resolves =
            |name: &str, canonical: &str| aliases.resolve(ALIAS_SECTION, name) == Some(canonical);
        let matching: Vec<&Fixture> = fixtures
            .iter()
            .filter(|f| resolves(&f.home, home) && resolves(&f.away, away))
            .collect();
        let fixture = match matching.as_slice() {
            [fixture] => *fixture,
            [] => {
                search.refused.push(format!(
                    "{label} : introuvable chez OddsPapi (nom OddsPapi absent de la section [{ALIAS_SECTION}], ou match non couvert)"
                ));
                search.hints.extend(hints(&fixtures, home, away));
                continue;
            }
            several => {
                search.refused.push(format!(
                    "{label} : {} matchs OddsPapi possibles dans la fenêtre, aucun retenu",
                    several.len()
                ));
                continue;
            }
        };
        pause(HISTORY_COOLDOWN);
        match download(conn, fixture, (home, away), now, fetch, pause) {
            Ok(Ok(quote)) => search.found.push(format!(
                "{label} ({} – {}, dernier prix Pinnacle : {})",
                fixture.home,
                fixture.away,
                kickoff_label(quote.last_change_at)
            )),
            Ok(Err(reason)) => search.refused.push(format!("{label} : {reason}")),
            // Une erreur sur un match n'arrête pas le repli : les autres rencontres restent à chercher.
            Err(error) => search.refused.push(format!("{label} : {error}")),
        }
    }
    Ok(search)
}

/// Matchs de football à venir dont le coup d'envoi tombe dans la fenêtre, ramenée à [`MAX_WINDOW_DAYS`] jours.
fn upcoming(
    conn: &Connection,
    window: (DateTime<Utc>, DateTime<Utc>),
    now: Clock,
    fetch: Fetch,
    pause: Pause,
) -> Result<Vec<Fixture>> {
    let from = window.0;
    let until = window.1.min(from + Duration::days(MAX_WINDOW_DAYS));
    let params = format!(
        "sportId={SOCCER}&statusId={STATUS_UPCOMING}&from={}&to={}",
        from.to_rfc3339_opts(SecondsFormat::Secs, true),
        until.to_rfc3339_opts(SecondsFormat::Secs, true),
    );
    pause(OTHER_COOLDOWN);
    let response = get(fetch, pause, "fixtures", &params)?;
    store::record_call(
        conn,
        &NewCall {
            called_at: now(),
            endpoint: "fixtures",
            params: &params,
            http_status: response.status,
            request_count: None,
        },
    )?;
    // Aucun match : OddsPapi répond 404 `FIXTURE_NOT_FOUND` au lieu d'une liste vide (§34).
    if response.status == 404 && response.body.contains("FIXTURE_NOT_FOUND") {
        return Ok(Vec::new());
    }
    if response.status != 200 {
        return Err(api_error(response.status, &response.body));
    }
    Ok(format::parse_fixtures(&response.body)?
        .into_iter()
        .filter(|f| f.status_id == Some(STATUS_UPCOMING) && f.kickoff >= from && f.kickoff <= until)
        .collect())
}

/// Télécharge l'historique Pinnacle d'un match et archive son dernier prix. `Ok(Err(raison))` : pas de cote utilisable.
fn download(
    conn: &Connection,
    fixture: &Fixture,
    teams: (&str, &str),
    now: Clock,
    fetch: Fetch,
    pause: Pause,
) -> Result<std::result::Result<LotofootQuote, String>> {
    let params = format!("fixtureId={}&bookmakers={REFERENCE}", fixture.fixture_id);
    let response = get(fetch, pause, "historical-odds", &params)?;
    let fetched_at = now();
    let call_id = store::record_call(
        conn,
        &NewCall {
            called_at: fetched_at,
            endpoint: "historical-odds",
            params: &params,
            http_status: response.status,
            request_count: None,
        },
    )?;
    match response.status {
        200 => {}
        400 | 404 => return Ok(Err("aucun historique chez OddsPapi".to_string())),
        status => return Err(api_error(status, &response.body)),
    }
    let history = format::parse_history(&response.body, fixture.kickoff)?;
    let Some((odds, last_change_at)) = latest_1x2(&history, fetched_at) else {
        return Ok(Err(
            "cote Pinnacle 1X2 absente ou suspendue chez OddsPapi".to_string()
        ));
    };
    let quote = LotofootQuote {
        call_id,
        // `clone` : le relevé possède ses données, alors que `fixture` appartient à la liste des matchs.
        fixture: fixture.clone(),
        teams: (teams.0.to_string(), teams.1.to_string()),
        odds,
        last_change_at,
        fetched_at,
    };
    store::record_lotofoot_quote(conn, &quote)?;
    Ok(Ok(quote))
}

/// Dernier prix Pinnacle de chaque issue du 1X2 connu à `at`, et l'instant du plus récent des trois changements. `None`
/// si une issue n'a aucun prix ou si son dernier état est une suspension.
fn latest_1x2(history: &History, at: DateTime<Utc>) -> Option<(Odds1x2, DateTime<Utc>)> {
    let last = |outcome: u32| {
        history
            .prices
            .iter()
            .filter(|p| {
                p.bookmaker == REFERENCE && p.market_id == MARKET_1X2 && p.outcome_id == outcome
            })
            .filter(|p| p.changed_at <= at)
            .max_by_key(|p| p.changed_at)
            .filter(|p| p.active)
            .and_then(|p| Some((p.price?, p.changed_at)))
    };
    let [home, draw, away] = OUTCOMES_1X2.map(last);
    let ((home, h_at), (draw, d_at), (away, a_at)) = (home?, draw?, away?);
    Some((Odds1x2 { home, draw, away }, h_at.max(d_at).max(a_at)))
}

/// Pistes pour une rencontre introuvable : matchs dont le nom OddsPapi d'une équipe se traduit par le nom cherché, ou le
/// contient (sans tenir compte de la casse). Simple aide à la saisie de l'alias.
fn hints(fixtures: &[Fixture], home: &str, away: &str) -> Vec<String> {
    let close = |raw: &str, canonical: &str| raw.to_lowercase().contains(&canonical.to_lowercase());
    fixtures
        .iter()
        .filter(|f| close(&f.home, home) || close(&f.away, away))
        .take(HINTS_PER_MATCH)
        .map(|f| {
            format!(
                "{home} – {away} : piste OddsPapi « {} » – « {} » ({})",
                f.home,
                f.away,
                kickoff_label(f.kickoff)
            )
        })
        .collect()
}

/// Dernière cote Pinnacle relevée chez OddsPapi pour la rencontre `teams` (noms canoniques), coup d'envoi dans
/// `kickoff`, relevée entre `fresh_after` et `started` (rien de postérieur à la séance). `None` si l'archive n'en a pas.
pub fn archived_market(
    conn: &Connection,
    teams: (&str, &str),
    kickoff: (DateTime<Utc>, DateTime<Utc>),
    fresh_after: DateTime<Utc>,
    started: DateTime<Utc>,
    method: MarginMethod,
) -> Result<Option<MatchMarket>> {
    let Some((quote_id, quote)) =
        store::latest_lotofoot_quote(conn, teams, kickoff, (fresh_after, started))?
    else {
        return Ok(None);
    };
    Ok(remove_margin(&quote.odds, method).map(|probs| MatchMarket {
        probs,
        source: Source::OddsPapi {
            quote_id,
            last_change_at: quote.last_change_at,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Quota;
    use crate::ingest::client::HttpResponse;
    use crate::store::migrate::migrate;

    const ALIASES: &str = r#"
[LOTOFOOT]
"Benfica" = ["SL Benfica"]
"Gil Vicente" = ["Gil Vicente Barcelos"]
"Braga" = []
"Famalicão" = ["Famalicao"]
"#;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().to_utc()
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            quota: Quota::default(),
            body: body.to_string(),
        }
    }

    const FIXTURES: &str = r#"[
        {"fixtureId":"id1","tournamentId":327,"statusId":0,"startTime":"2026-10-29T20:45:00.000Z",
         "participant1Name":"SL Benfica","participant2Name":"Gil Vicente Barcelos"},
        {"fixtureId":"id2","tournamentId":238,"statusId":0,"startTime":"2026-11-01T20:30:00.000Z",
         "participant1Name":"SC Braga","participant2Name":"FC Famalicao"},
        {"fixtureId":"id3","tournamentId":24702,"statusId":null,"startTime":"2026-10-31T09:00:00.000Z",
         "participant1Name":"FC Ingolstadt 04","participant2Name":"VfB Stuttgart"}]"#;

    /// Historique Pinnacle du match id1 : un prix à 15 h, un changement à 17 h 05, et un prix postérieur au relevé.
    const HISTORY: &str = r#"{"bookmakers":{"pinnacle":{"markets":{"101":{"outcomes":{
        "101":{"players":{"0":[{"createdAt":"2026-10-29T15:00:00Z","price":1.30,"active":true},
                               {"createdAt":"2026-10-29T17:05:41Z","price":1.272,"active":true},
                               {"createdAt":"2026-10-29T18:00:00Z","price":1.25,"active":true}]}},
        "102":{"players":{"0":[{"createdAt":"2026-10-29T15:00:00Z","price":6.43,"active":true}]}},
        "103":{"players":{"0":[{"createdAt":"2026-10-29T15:00:00Z","price":10.01,"active":true}]}}}}}}}}"#;

    /// API simulée : consommation `used`, la liste des matchs, l'historique de id1 ; les appels sont notés.
    fn api(
        used: u32,
        calls: &mut Vec<String>,
    ) -> impl FnMut(&str, &str) -> Result<HttpResponse> + '_ {
        move |path: &str, params: &str| {
            calls.push(format!("{path}?{params}"));
            Ok(match path {
                "account" => ok(&format!(
                    r#"{{"subscriptions":[{{"is_active":true,"request_count":{used},"request_limit":250}}]}}"#
                )),
                "fixtures" => ok(FIXTURES),
                "historical-odds" if params.starts_with("fixtureId=id1&") => ok(HISTORY),
                _ => HttpResponse {
                    status: 404,
                    quota: Quota::default(),
                    body: r#"{"error":"not found"}"#.to_string(),
                },
            })
        }
    }

    fn wanted(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(h, a)| (h.to_string(), a.to_string()))
            .collect()
    }

    const WINDOW: (&str, &str) = ("2026-10-29T15:00:00Z", "2026-11-03T17:00:00Z");

    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        (at(WINDOW.0), at(WINDOW.1))
    }

    #[test]
    fn a_match_is_found_by_its_exact_names_and_its_last_price_before_the_fetch_is_archived() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let aliases = Aliases::parse(ALIASES).unwrap();
        let mut calls = Vec::new();
        let mut fetch = api(70, &mut calls);
        let mut clock = || at("2026-10-29T17:16:00Z");
        let search = refresh(
            &mut conn,
            &Config::default(),
            &aliases,
            &wanted(&[("Benfica", "Gil Vicente")]),
            window(),
            &mut clock,
            Access {
                fetch: &mut fetch,
                pause: &mut |_| {},
            },
        )
        .unwrap();
        assert_eq!(search.spent, 1);
        assert_eq!(search.found.len(), 1, "{search:?}");
        assert!(search.found[0].contains("SL Benfica – Gil Vicente Barcelos"));

        let market = archived_market(
            &conn,
            ("Benfica", "Gil Vicente"),
            window(),
            at("2026-10-29T10:00:00Z"),
            at("2026-10-29T17:20:00Z"),
            MarginMethod::Power,
        )
        .unwrap()
        .unwrap();
        // Le prix de 18 h, postérieur au relevé, n'est pas lu : la cote retenue est celle de 17 h 05.
        let Source::OddsPapi { last_change_at, .. } = market.source else {
            panic!("cote attendue d'OddsPapi");
        };
        assert_eq!(last_change_at, at("2026-10-29T17:05:41Z"));
        let p = market.probs.to_array();
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(p[0] > 0.7);

        // Une séance antérieure au relevé ne le voit pas (aucune fuite du futur).
        assert!(
            archived_market(
                &conn,
                ("Benfica", "Gil Vicente"),
                window(),
                at("2026-10-29T10:00:00Z"),
                at("2026-10-29T17:00:00Z"),
                MarginMethod::Power,
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn an_unknown_name_is_refused_with_hints_that_are_never_used() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        // « SC Braga » n'est pas dans la table : la rencontre n'est pas appariée, même si le nom contient « Braga ».
        let aliases = Aliases::parse(ALIASES).unwrap();
        let mut calls = Vec::new();
        let mut fetch = api(70, &mut calls);
        let mut clock = || at("2026-10-29T17:16:00Z");
        let search = refresh(
            &mut conn,
            &Config::default(),
            &aliases,
            &wanted(&[("Braga", "Famalicão")]),
            window(),
            &mut clock,
            Access {
                fetch: &mut fetch,
                pause: &mut |_| {},
            },
        )
        .unwrap();
        assert!(search.found.is_empty());
        assert!(search.refused[0].contains("introuvable chez OddsPapi"));
        assert_eq!(search.hints.len(), 1);
        assert!(search.hints[0].contains("« SC Braga » – « FC Famalicao »"));
        drop(fetch);
        assert!(calls.iter().all(|c| !c.starts_with("historical-odds")));
    }

    #[test]
    fn the_budget_guard_refuses_before_any_counted_request() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let aliases = Aliases::parse(ALIASES).unwrap();
        let config = Config::default();
        let mut calls = Vec::new();
        let mut fetch = api(config.oddspapi_monthly_cap, &mut calls);
        let mut clock = || at("2026-10-29T17:16:00Z");
        let search = refresh(
            &mut conn,
            &config,
            &aliases,
            &wanted(&[("Benfica", "Gil Vicente")]),
            window(),
            &mut clock,
            Access {
                fetch: &mut fetch,
                pause: &mut |_| {},
            },
        )
        .unwrap();
        assert_eq!(search.spent, 0);
        assert_eq!(search.refused.len(), 1);
        drop(fetch);
        assert_eq!(calls.len(), 1, "seul `account`, non décompté : {calls:?}");
    }

    #[test]
    fn the_listing_window_stays_under_ten_days() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let aliases = Aliases::parse(ALIASES).unwrap();
        let mut calls = Vec::new();
        let mut fetch = api(70, &mut calls);
        let mut clock = || at("2026-10-29T17:16:00Z");
        refresh(
            &mut conn,
            &Config::default(),
            &aliases,
            &wanted(&[("Benfica", "Gil Vicente")]),
            (at("2026-10-29T15:00:00Z"), at("2026-12-01T00:00:00Z")),
            &mut clock,
            Access {
                fetch: &mut fetch,
                pause: &mut |_| {},
            },
        )
        .unwrap();
        drop(fetch);
        assert!(
            calls.iter().any(|c| c
                == "fixtures?sportId=10&statusId=0&from=2026-10-29T15:00:00Z&to=2026-11-07T15:00:00Z"),
            "{calls:?}"
        );
    }
}
