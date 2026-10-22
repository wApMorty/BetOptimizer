//! Téléchargement de l'historique OddsPapi des matchs terminés depuis le début du test (`edge latency fetch`).
//!
//! Déroulé : consommation du compte (`account`, non décompté), garde de budget, liste des matchs terminés de chaque
//! championnat (`fixtures`, 1 requête décomptée chacun), puis historique de chaque match encore inconnu
//! (`historical-odds`, non décompté). L'appel réseau, l'horloge et les pauses sont reçus en paramètres : en production
//! le vrai client, `Utc::now` et `thread::sleep`, dans les tests une API simulée.

use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::Connection;

use super::format::{self, Fixture, STATUS_FINISHED};
use super::{BOOKMAKERS, HISTORY_COOLDOWN, LEAGUES, OTHER_COOLDOWN};
use crate::error::{Error, Result};
use crate::eval::latency::test_start;
use crate::ingest::budget;
use crate::ingest::client::HttpResponse;
use crate::ingest::odds_api::api_error;
use crate::ingest::snapshot::{Clock, Fetch};
use crate::store::oddspapi::{self as store, NewCall};

/// Fonction qui attend la durée donnée (pause entre deux appels).
pub type Pause<'a> = &'a mut dyn FnMut(Duration);

/// Nombre de nouvelles tentatives après un refus pour cause de débit (HTTP 429 sans quota épuisé).
const RATE_RETRIES: u32 = 4;

/// Requêtes décomptées par passage : une liste de matchs par championnat.
pub fn cost() -> u32 {
    // Six championnats : la conversion ne peut pas échouer.
    u32::try_from(LEAGUES.len()).unwrap_or(u32::MAX)
}

/// Bilan d'un passage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchReport {
    /// Requêtes décomptées avant le passage, et allocation mensuelle.
    pub used_before: u32,
    /// Allocation mensuelle de l'offre.
    pub limit: u32,
    /// Matchs terminés listés depuis le début du test.
    pub listed: usize,
    /// Matchs déjà archivés (ignorés).
    pub already_known: usize,
    /// Historiques téléchargés et archivés.
    pub downloaded: usize,
    /// Matchs sans historique chez OddsPapi (archivés comme tels, jamais redemandés).
    pub missing: usize,
    /// Changements de prix archivés.
    pub prices: usize,
    /// Entrées écartées (actives sans cote valide).
    pub discarded: usize,
    /// Échecs (erreur serveur, réseau) : le match sera redemandé au passage suivant.
    pub failures: Vec<String>,
}

/// Lance un passage complet. `cap` : plafond de requêtes décomptées (`oddspapi_monthly_cap`).
pub fn run(
    conn: &mut Connection,
    cap: u32,
    now: Clock,
    fetch: Fetch,
    pause: Pause,
) -> Result<FetchReport> {
    let usage = read_usage(conn, now, fetch, pause)?;
    // Garde de coût : le message parle de « crédits », ce sont ici des requêtes décomptées.
    budget::check(Some(usage.used), cost(), cap)?;
    let mut report = FetchReport {
        used_before: usage.used,
        limit: usage.limit,
        ..FetchReport::default()
    };

    let until = now();
    let mut fixtures = Vec::new();
    for (tournament_id, _) in LEAGUES {
        pause(OTHER_COOLDOWN);
        fixtures.extend(list_finished(
            conn,
            tournament_id,
            until,
            now,
            fetch,
            pause,
        )?);
    }
    report.listed = fixtures.len();

    for fixture in &fixtures {
        if store::known_fixture(conn, &fixture.fixture_id)? {
            report.already_known += 1;
            continue;
        }
        pause(HISTORY_COOLDOWN);
        match download(conn, fixture, now, fetch, pause) {
            Ok(Some((prices, discarded))) => {
                report.downloaded += 1;
                report.prices += prices;
                report.discarded += discarded;
            }
            Ok(None) => report.missing += 1,
            // Une erreur sur un match n'arrête pas le passage : les autres restent à archiver.
            Err(error) => report.failures.push(format!(
                "{} ({} – {}) : {error}",
                fixture.fixture_id, fixture.home, fixture.away
            )),
        }
    }
    Ok(report)
}

/// Lit la consommation du compte. Le corps de la réponse n'est jamais journalisé : il contient la clé.
fn read_usage(conn: &Connection, now: Clock, fetch: Fetch, pause: Pause) -> Result<format::Usage> {
    let response = get(fetch, pause, "account", "")?;
    let usage = if response.status == 200 {
        Some(format::parse_usage(&response.body)?)
    } else {
        None
    };
    store::record_call(
        conn,
        &NewCall {
            called_at: now(),
            endpoint: "account",
            params: "",
            http_status: response.status,
            request_count: usage.map(|u| u.used),
        },
    )?;
    usage.ok_or_else(|| api_error(response.status, &response.body))
}

/// Matchs terminés d'un championnat depuis le début du test. `tournamentId` dispense des bornes de 10 jours de l'API :
/// toute la période est relue chaque semaine pour 1 requête, si bien qu'un lundi manqué ne perd aucun match.
fn list_finished(
    conn: &Connection,
    tournament_id: u32,
    until: DateTime<Utc>,
    now: Clock,
    fetch: Fetch,
    pause: Pause,
) -> Result<Vec<Fixture>> {
    let params = format!(
        "tournamentId={tournament_id}&statusId={STATUS_FINISHED}&from={}&to={}",
        test_start().to_rfc3339_opts(SecondsFormat::Secs, true),
        until.to_rfc3339_opts(SecondsFormat::Secs, true),
    );
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
    // OddsPapi répond 404 (`FIXTURE_NOT_FOUND`) quand aucun match ne correspond, au lieu d'une liste vide (vu le
    // 2026-10-22, au premier jour du test).
    if response.status == 404 && response.body.contains("FIXTURE_NOT_FOUND") {
        return Ok(Vec::new());
    }
    if response.status != 200 {
        return Err(api_error(response.status, &response.body));
    }
    Ok(format::parse_fixtures(&response.body)?
        .into_iter()
        .filter(|f| f.status_id == STATUS_FINISHED && f.kickoff >= test_start())
        .collect())
}

/// Télécharge et archive l'historique d'un match. `Ok(None)` : OddsPapi n'a pas d'historique (400 ou 404).
fn download(
    conn: &mut Connection,
    fixture: &Fixture,
    now: Clock,
    fetch: Fetch,
    pause: Pause,
) -> Result<Option<(usize, usize)>> {
    let params = format!("fixtureId={}&bookmakers={BOOKMAKERS}", fixture.fixture_id);
    let response = get(fetch, pause, "historical-odds", &params)?;
    let called_at = now();
    store::record_call(
        conn,
        &NewCall {
            called_at,
            endpoint: "historical-odds",
            params: &params,
            http_status: response.status,
            request_count: None,
        },
    )?;
    match response.status {
        200 => {
            let history = format::parse_history(&response.body, fixture.kickoff)?;
            let written =
                store::record_fixture(conn, fixture, called_at, BOOKMAKERS, Some(&history))?;
            Ok(Some((written, history.discarded)))
        }
        400 | 404 => {
            store::record_fixture(conn, fixture, called_at, BOOKMAKERS, None)?;
            Ok(None)
        }
        status => Err(api_error(status, &response.body)),
    }
}

/// `GET` avec nouvelles tentatives sur un refus de débit. Un quota mensuel épuisé (`REQUEST_LIMIT_EXCEEDED`) n'est
/// pas réessayé : il est renvoyé tel quel et arrête le passage.
fn get(fetch: Fetch, pause: Pause, path: &str, params: &str) -> Result<HttpResponse> {
    let mut attempt = 0;
    loop {
        let response = fetch(path, params)?;
        let rate_limited = response.status == 429 && !response.body.contains("REQUEST_LIMIT");
        if !rate_limited || attempt >= RATE_RETRIES {
            if response.status == 429 {
                return Err(api_error(response.status, &response.body));
            }
            return Ok(response);
        }
        attempt += 1;
        pause(Duration::from_secs(10 * u64::from(attempt)));
    }
}

/// Erreur affichée quand la garde refuse le passage (pour la CLI, qui l'explique).
pub fn is_budget_refusal(error: &Error) -> bool {
    matches!(error, Error::BudgetExceeded { .. })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::domain::Quota;
    use crate::store::open_migrated;

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            quota: Quota::default(),
            body: body.to_string(),
        }
    }

    const ACCOUNT: &str = r#"{"api_key":"SECRET","subscriptions":[{"is_active":true,"request_count":13,"request_limit":250}]}"#;

    fn fixture_list(id: &str, start: &str) -> String {
        format!(
            r#"[{{"fixtureId":"{id}","tournamentId":17,"statusId":2,"startTime":"{start}",
                "participant1Name":"Arsenal FC","participant2Name":"Chelsea FC"}}]"#
        )
    }

    const HISTORY: &str = r#"{"bookmakers":{"winamax.fr":{"markets":{"101":{"outcomes":{"101":{"players":{"0":[
        {"createdAt":"2026-10-25T14:00:00+00:00","price":2.1,"active":true}]}}}}}}}}"#;

    /// Exécute un passage contre une API simulée qui répond, dans l'ordre, par les réponses données.
    fn run_with(
        responses: Vec<HttpResponse>,
        cap: u32,
    ) -> (Result<FetchReport>, Vec<String>, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_migrated(&dir.path().join("edge.db")).unwrap();
        let mut queue: VecDeque<HttpResponse> = responses.into();
        let mut calls = Vec::new();
        let mut clock = || {
            DateTime::parse_from_rfc3339("2026-10-27T08:00:00Z")
                .unwrap()
                .to_utc()
        };
        let mut fetch = |path: &str, params: &str| {
            calls.push(format!("{path}?{params}"));
            Ok(queue.pop_front().expect("réponse simulée manquante"))
        };
        let mut pause = |_: Duration| {};
        let result = run(&mut conn, cap, &mut clock, &mut fetch, &mut pause);
        (result, calls, conn)
    }

    #[test]
    fn a_pass_lists_the_six_leagues_then_downloads_each_new_fixture_once() {
        let mut responses = vec![response(200, ACCOUNT)];
        responses.push(response(
            200,
            &fixture_list("id1", "2026-10-25T15:00:00.000Z"),
        ));
        // Le même match apparaît aussi dans la deuxième liste : il ne doit être téléchargé qu'une fois.
        responses.push(response(
            200,
            &fixture_list("id1", "2026-10-25T15:00:00.000Z"),
        ));
        for _ in 2..6 {
            responses.push(response(200, "[]"));
        }
        responses.push(response(200, HISTORY));
        let (result, calls, conn) = run_with(responses, 200);
        let report = result.unwrap();
        assert_eq!(
            (report.listed, report.downloaded, report.already_known),
            (2, 1, 1)
        );
        assert_eq!(report.prices, 1);
        assert_eq!(
            calls
                .iter()
                .filter(|c| c.starts_with("historical-odds"))
                .count(),
            1
        );
        assert!(calls.iter().all(|c| !c.contains("apiKey")));
        let logged: i64 = conn
            .query_row("SELECT COUNT(*) FROM oddspapi_call", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logged, 8);
    }

    #[test]
    fn matches_before_the_test_start_are_ignored() {
        let mut responses = vec![response(200, ACCOUNT)];
        responses.push(response(
            200,
            &fixture_list("id0", "2026-10-21T19:00:00.000Z"),
        ));
        for _ in 1..6 {
            responses.push(response(200, "[]"));
        }
        let (result, calls, _) = run_with(responses, 200);
        assert_eq!(result.unwrap().listed, 0);
        assert!(calls.iter().all(|c| !c.starts_with("historical-odds")));
    }

    #[test]
    fn no_fixture_found_is_an_empty_list() {
        let mut responses = vec![response(200, ACCOUNT)];
        for _ in 0..6 {
            responses.push(response(
                404,
                r#"{"error":{"message":"No fixtures found for the specified criteria.","code":"FIXTURE_NOT_FOUND"}}"#,
            ));
        }
        let (result, _, _) = run_with(responses, 200);
        assert_eq!(result.unwrap().listed, 0);
    }

    #[test]
    fn the_budget_guard_refuses_a_pass_that_would_exceed_the_cap() {
        // 13 requêtes déjà décomptées + 6 > plafond de 18 : refus avant toute requête décomptée.
        let (result, calls, _) = run_with(vec![response(200, ACCOUNT)], 18);
        assert!(is_budget_refusal(&result.unwrap_err()));
        assert_eq!(calls, vec!["account?".to_string()]);
    }

    #[test]
    fn a_missing_history_is_archived_and_a_server_error_is_retried_next_time() {
        let mut responses = vec![response(200, ACCOUNT)];
        let two = r#"[{"fixtureId":"a","tournamentId":17,"statusId":2,"startTime":"2026-10-25T15:00:00.000Z",
            "participant1Name":"A","participant2Name":"B"},{"fixtureId":"b","tournamentId":17,"statusId":2,
            "startTime":"2026-10-25T17:30:00.000Z","participant1Name":"C","participant2Name":"D"}]"#;
        responses.push(response(200, two));
        for _ in 1..6 {
            responses.push(response(200, "[]"));
        }
        responses.push(response(
            404,
            r#"{"error":{"message":"No historical odds found."}}"#,
        ));
        responses.push(response(522, "error code: 522"));
        let (result, _, conn) = run_with(responses, 200);
        let report = result.unwrap();
        assert_eq!((report.missing, report.failures.len()), (1, 1));
        assert!(store::known_fixture(&conn, "a").unwrap());
        assert!(!store::known_fixture(&conn, "b").unwrap());
    }

    #[test]
    fn a_rate_limit_is_retried_but_an_exhausted_quota_is_not() {
        let mut fetches = 0;
        let mut fetch = |_: &str, _: &str| {
            fetches += 1;
            Ok(if fetches < 3 {
                response(429, r#"{"code":"rate_limited"}"#)
            } else {
                response(200, "ok")
            })
        };
        let mut pause = |_: Duration| {};
        assert_eq!(
            get(&mut fetch, &mut pause, "fixtures", "").unwrap().status,
            200
        );

        let mut fetch =
            |_: &str, _: &str| Ok(response(429, r#"{"code":"REQUEST_LIMIT_EXCEEDED"}"#));
        assert!(get(&mut fetch, &mut pause, "fixtures", "").is_err());
    }
}
