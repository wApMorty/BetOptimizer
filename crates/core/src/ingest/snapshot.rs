//! Relevé des cotes : pour chaque ligue configurée, garde de budget, appel `/odds`, journal, archive.
//!
//! L'appel réseau et l'horloge sont reçus en paramètres (de simples fonctions) : en production ce sont le
//! vrai client et `Utc::now`, dans les tests une API simulée alimentée par des réponses réelles enregistrées.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::budget;
use super::client::HttpResponse;
use super::odds_api::{
    SPORTS_PATH, TENNIS_PREFIX, active_tennis_keys, api_error, bookmakers_without_quotes,
    odds_params, odds_path, parse_odds,
};
use crate::config::Config;
use crate::domain::Quota;
use crate::error::{Error, Result};
use crate::store::archive::{self, Endpoint, NewApiCall, StoredCounts};

/// Fonction qui effectue un `GET` : reçoit le chemin et les paramètres (sans clé).
pub type Fetch<'a> = &'a mut dyn FnMut(&str, &str) -> Result<HttpResponse>;

/// Fonction qui donne l'heure courante (UTC).
pub type Clock<'a> = &'a mut dyn FnMut() -> DateTime<Utc>;

/// Ce qu'un relevé coûterait, sans rien appeler (`edge snapshot --dry-run`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Ligues qui seraient interrogées.
    pub leagues: Vec<String>,
    /// Coût en crédits d'un appel (un par ligue).
    pub cost_per_call: u32,
    /// Coût total du relevé.
    pub total_cost: u32,
}

/// Calcule le plan d'un relevé à partir de la configuration.
pub fn plan(config: &Config) -> Plan {
    let cost_per_call = budget::odds_cost(config.markets.len(), config.bookmakers.len());
    let calls = u32::try_from(config.leagues.len()).unwrap_or(u32::MAX);
    Plan {
        leagues: config.leagues.clone(),
        cost_per_call,
        total_cost: cost_per_call.saturating_mul(calls),
    }
}

/// Une ligue archivée avec succès.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueReport {
    /// Clé de la compétition.
    pub sport_key: String,
    /// Lignes écrites dans l'archive.
    pub stored: StoredCounts,
    /// Matchs déjà commencés, non archivés.
    pub already_started: usize,
    /// Marchés écartés pour cote invalide.
    pub dropped_markets: usize,
    /// Bookmakers demandés mais sans aucune cote dans cette ligue.
    pub missing_bookmakers: Vec<String>,
}

/// Une ligue non archivée, avec la raison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueFailure {
    /// Clé de la compétition.
    pub sport_key: String,
    /// Raison lisible (refus de la garde, erreur de l'API, réseau, réponse illisible).
    pub reason: String,
}

/// Bilan d'un relevé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotReport {
    /// Ligues archivées.
    pub leagues: Vec<LeagueReport>,
    /// Ligues en échec.
    pub failures: Vec<LeagueFailure>,
    /// Crédits consommés avant le relevé (lus à l'API).
    pub used_before: u32,
    /// Crédits consommés après le relevé.
    pub used_after: u32,
    /// `true` si le quota semble avoir été réinitialisé depuis le relevé précédent.
    pub quota_reset: bool,
}

/// Issue d'une ligue. Les erreurs de base de données ne passent pas par ici : elles interrompent tout
/// le relevé (`Err`). Une panne de l'API ne concerne qu'une ligue : on la note et on passe à la suivante.
enum Outcome {
    Stored(LeagueReport),
    Failed(LeagueFailure),
}

/// Effectue le relevé de toutes les ligues configurées, puis des tournois de tennis en cours.
///
/// 1. lit le quota réel par un appel gratuit à `/sports` (qui liste aussi les tournois de tennis actifs) ;
/// 2. pour chaque ligue : la garde de budget autorise ou refuse, puis `/odds` est appelé, le corps brut
///    est journalisé *avant* toute interprétation, et les matchs à venir sont archivés. Le football passe avant
///    le tennis : si le budget manque, c'est le tennis qui est refusé.
pub fn run(
    conn: &mut Connection,
    config: &Config,
    now: Clock,
    fetch: Fetch,
) -> Result<SnapshotReport> {
    let previous_used = archive::last_known_usage(conn)?;
    let (quota, sports_body) = read_sports(conn, now, fetch)?;
    let used_before = quota.used.ok_or(Error::QuotaUnknown)?;
    let quota_reset = budget::reset_detected(previous_used, used_before);
    if quota_reset {
        tracing::info!(
            avant = previous_used,
            apres = used_before,
            "quota réinitialisé"
        );
    }

    let params = odds_params(&config.markets, &config.bookmakers);
    let cost = budget::odds_cost(config.markets.len(), config.bookmakers.len());
    let mut used = used_before;
    let mut leagues = Vec::new();
    let mut failures = Vec::new();

    let mut sport_keys = config.leagues.clone();
    if config.tennis_archive {
        sport_keys.extend(tennis_keys_to_archive(config, &sports_body));
    }

    for sport_key in &sport_keys {
        if let Err(refusal) = budget::check(Some(used), cost, config.monthly_credit_cap) {
            tracing::warn!(ligue = %sport_key, "{refusal}");
            failures.push(LeagueFailure {
                sport_key: sport_key.clone(),
                reason: refusal.to_string(),
            });
            continue;
        }
        match snapshot_league(conn, config, sport_key, &params, now, fetch)? {
            Outcome::Stored(report) => {
                tracing::info!(
                    ligue = %report.sport_key,
                    matchs = report.stored.events,
                    cotes = report.stored.quotes,
                    deja_commences = report.already_started,
                    "relevé archivé"
                );
                leagues.push(report);
            }
            Outcome::Failed(failure) => {
                tracing::error!(ligue = %failure.sport_key, "{}", failure.reason);
                failures.push(failure);
            }
        }
        // La valeur de référence est toujours celle que l'API vient de renvoyer, jamais un compteur local.
        used = archive::last_known_usage(conn)?.unwrap_or(used);
    }

    Ok(SnapshotReport {
        leagues,
        failures,
        used_before,
        used_after: used,
        quota_reset,
    })
}

/// Tournois de tennis à archiver : ceux que `/sports` dit actifs, au plus `tennis_max_tournaments`.
///
/// Une réponse illisible ne bloque pas le football : on le signale et on n'archive pas le tennis ce jour-là.
fn tennis_keys_to_archive(config: &Config, sports_body: &str) -> Vec<String> {
    let mut keys = match active_tennis_keys(sports_body) {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(%error, "liste des sports illisible : pas de tennis dans ce relevé");
            return Vec::new();
        }
    };
    let cap = usize::try_from(config.tennis_max_tournaments).unwrap_or(usize::MAX);
    if keys.len() > cap {
        tracing::warn!(
            actifs = keys.len(),
            archives = cap,
            "plus de tournois de tennis actifs que le plafond : les derniers par ordre alphabétique sont ignorés"
        );
        keys.truncate(cap);
    }
    keys
}

/// Lit le quota par l'appel gratuit à `/sports` et journalise l'appel (`edge quota`, début de chaque relevé).
pub fn read_quota(conn: &Connection, now: Clock, fetch: Fetch) -> Result<Quota> {
    read_sports(conn, now, fetch).map(|(quota, _)| quota)
}

/// Comme [`read_quota`], et renvoie aussi le corps de la réponse (la liste des sports).
fn read_sports(conn: &Connection, now: Clock, fetch: Fetch) -> Result<(Quota, String)> {
    let response = fetch(SPORTS_PATH, "")?;
    archive::record_api_call(
        conn,
        &NewApiCall {
            called_at: now(),
            endpoint: Endpoint::Sports,
            sport_key: None,
            params: "",
            http_status: response.status,
            quota: response.quota,
            body: None,
        },
    )?;
    if response.status != 200 {
        return Err(api_error(response.status, &response.body));
    }
    Ok((response.quota, response.body))
}

fn snapshot_league(
    conn: &mut Connection,
    config: &Config,
    sport_key: &str,
    params: &str,
    now: Clock,
    fetch: Fetch,
) -> Result<Outcome> {
    let failed = |reason: String| {
        Outcome::Failed(LeagueFailure {
            sport_key: sport_key.to_string(),
            reason,
        })
    };

    let response = match fetch(&odds_path(sport_key), params) {
        Ok(response) => response,
        Err(network_error) => return Ok(failed(network_error.to_string())),
    };
    // L'instant de réception sert à la fois au journal et à la frontière « match déjà commencé ».
    let received_at = now();
    // Journal d'abord : même si la suite échoue, le corps brut est conservé et le crédit tracé.
    let api_call_id = archive::record_api_call(
        conn,
        &NewApiCall {
            called_at: received_at,
            endpoint: Endpoint::Odds,
            sport_key: Some(sport_key),
            params,
            http_status: response.status,
            quota: response.quota,
            body: Some(&response.body),
        },
    )?;

    if response.status != 200 {
        return Ok(failed(
            api_error(response.status, &response.body).to_string(),
        ));
    }
    let parsed = match parse_odds(&response.body, received_at) {
        Ok(parsed) => parsed,
        Err(format_error) => return Ok(failed(format_error.to_string())),
    };

    let stored = archive::record_snapshot(conn, api_call_id, &parsed.events)?;
    // Betclic, NetBet et d'autres ne cotent pas le tennis : les signaler chaque jour n'apprendrait rien.
    let missing_bookmakers: Vec<String> = if sport_key.starts_with(TENNIS_PREFIX) {
        Vec::new()
    } else {
        bookmakers_without_quotes(&config.bookmakers, &parsed.events)
            .into_iter()
            .map(str::to_string)
            .collect()
    };
    if !missing_bookmakers.is_empty() {
        tracing::warn!(
            ligue = sport_key,
            bookmakers = %missing_bookmakers.join(", "),
            "aucune cote pour ces bookmakers (ne couvrent pas la ligue, ou clé erronée)"
        );
    }
    Ok(Outcome::Stored(LeagueReport {
        sport_key: sport_key.to_string(),
        stored,
        already_started: parsed.already_started,
        dropped_markets: parsed.dropped_markets,
        missing_bookmakers,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;

    const EPL: &str = include_str!("../../tests/fixtures/odds_epl.json");
    const SPL: &str = include_str!("../../tests/fixtures/odds_spl_pinnacle_only.json");
    const SERIE_A: &str = include_str!("../../tests/fixtures/odds_serie_a_started.json");
    const INVALID_MARKET: &str = include_str!("../../tests/fixtures/error_invalid_market.json");
    const SPORTS_WITH_TENNIS: &str = include_str!("../../tests/fixtures/sports_with_tennis.json");
    const TENNIS: &str = include_str!("../../tests/fixtures/odds_tennis_wta_ningbo.json");

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    fn config(leagues: &[&str], cap: u32) -> Config {
        Config {
            leagues: leagues.iter().map(|l| l.to_string()).collect(),
            monthly_credit_cap: cap,
            ..Config::default()
        }
    }

    /// Instant du relevé réel dont proviennent les fixtures.
    fn clock() -> impl FnMut() -> DateTime<Utc> {
        || "2026-10-18T10:58:30Z".parse().unwrap()
    }

    fn response(status: u16, cost: u32, used: u32, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            quota: Quota {
                cost: Some(cost),
                used: Some(used),
                remaining: Some(500 - used),
            },
            body: body.to_string(),
        }
    }

    fn fixture_for(path: &str) -> &'static str {
        match path {
            "/sports/soccer_epl/odds" => EPL,
            "/sports/soccer_spl/odds" => SPL,
            "/sports/soccer_italy_serie_a/odds" => SERIE_A,
            "/sports/tennis_wta_ningbo_open/odds" => TENNIS,
            other => panic!("chemin inattendu dans le test : {other}"),
        }
    }

    fn count(conn: &Connection, table: &str) -> u32 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    /// Lance un relevé d'une ligue de football avec la réponse de `/sports` donnée ; renvoie le bilan et les chemins appelés.
    fn run_with_sports(sports_body: &str, config: &Config) -> (SnapshotReport, Vec<String>) {
        let mut conn = database();
        let mut used = 8;
        let mut paths = Vec::new();
        let mut fetch = |path: &str, _params: &str| {
            paths.push(path.to_string());
            if path == SPORTS_PATH {
                return Ok(response(200, 0, used, sports_body));
            }
            used += 1;
            Ok(response(200, 1, used, fixture_for(path)))
        };
        let report = run(&mut conn, config, &mut clock(), &mut fetch).unwrap();
        (report, paths)
    }

    #[test]
    fn active_tennis_tournaments_are_archived_after_the_football_leagues() {
        let (report, paths) = run_with_sports(SPORTS_WITH_TENNIS, &config(&["soccer_epl"], 450));

        assert_eq!(
            paths,
            vec![
                "/sports",
                "/sports/soccer_epl/odds",
                "/sports/tennis_wta_ningbo_open/odds"
            ]
        );
        assert!(report.failures.is_empty());
        let tennis = &report.leagues[1];
        assert_eq!(tennis.sport_key, "tennis_wta_ningbo_open");
        assert_eq!(tennis.stored.events, 12);
        // Betclic ne cote pas le tennis : on ne le signale pas chaque jour.
        assert!(tennis.missing_bookmakers.is_empty());
        assert_eq!(report.used_after, 10);
    }

    #[test]
    fn tennis_can_be_switched_off_or_capped_and_an_unreadable_sports_list_spares_football() {
        let mut off = config(&["soccer_epl"], 450);
        off.tennis_archive = false;
        let (_, paths) = run_with_sports(SPORTS_WITH_TENNIS, &off);
        assert_eq!(paths, vec!["/sports", "/sports/soccer_epl/odds"]);

        let mut capped = config(&["soccer_epl"], 450);
        capped.tennis_max_tournaments = 0;
        let (_, paths) = run_with_sports(SPORTS_WITH_TENNIS, &capped);
        assert_eq!(paths, vec!["/sports", "/sports/soccer_epl/odds"]);

        let (report, paths) = run_with_sports("pas du json", &config(&["soccer_epl"], 450));
        assert_eq!(paths, vec!["/sports", "/sports/soccer_epl/odds"]);
        assert!(report.failures.is_empty());
    }

    #[test]
    fn when_the_budget_runs_short_it_is_tennis_that_is_refused_not_football() {
        // Plafond 9 avec 8 crédits déjà utilisés : une seule ligue passe, la première (le football).
        let (report, paths) = run_with_sports(SPORTS_WITH_TENNIS, &config(&["soccer_epl"], 9));

        assert_eq!(report.leagues.len(), 1);
        assert_eq!(report.leagues[0].sport_key, "soccer_epl");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].sport_key, "tennis_wta_ningbo_open");
        assert!(!paths.contains(&"/sports/tennis_wta_ningbo_open/odds".to_string()));
    }

    #[test]
    fn plan_costs_one_credit_per_league_by_default() {
        let plan = plan(&Config::default());
        assert_eq!(plan.leagues.len(), 6);
        assert_eq!(plan.cost_per_call, 1);
        assert_eq!(plan.total_cost, 6);
    }

    #[test]
    fn every_league_is_archived_and_every_call_journaled() {
        let mut conn = database();
        let mut used = 8;
        let mut paths = Vec::new();
        let mut fetch = |path: &str, _params: &str| {
            paths.push(path.to_string());
            if path == SPORTS_PATH {
                return Ok(response(200, 0, used, "[]"));
            }
            used += 1;
            Ok(response(200, 1, used, fixture_for(path)))
        };

        let report = run(
            &mut conn,
            &config(&["soccer_epl", "soccer_spl"], 450),
            &mut clock(),
            &mut fetch,
        )
        .unwrap();

        assert_eq!(report.used_before, 8);
        assert_eq!(report.used_after, 10);
        assert!(report.failures.is_empty());
        assert_eq!(report.leagues.len(), 2);
        // EPL : 3 matchs, 9 + 9 + 6 cotes. Écosse : 2 matchs, Pinnacle seul = 3 cotes chacun.
        assert_eq!(
            report.leagues[0].stored,
            StoredCounts {
                events: 3,
                quotes: 24
            }
        );
        assert_eq!(
            report.leagues[1].stored,
            StoredCounts {
                events: 2,
                quotes: 6
            }
        );
        assert_eq!(count(&conn, "api_call"), 3);
        // Winamax est demandé mais ne cote pas l'Écosse : c'est signalé.
        assert!(
            report.leagues[1]
                .missing_bookmakers
                .contains(&"winamax_fr".to_string())
        );
        assert!(
            !report.leagues[1]
                .missing_bookmakers
                .contains(&"pinnacle".to_string())
        );
        assert_eq!(
            paths,
            [
                "/sports",
                "/sports/soccer_epl/odds",
                "/sports/soccer_spl/odds"
            ]
        );
    }

    #[test]
    fn budget_guard_refuses_the_remaining_leagues_without_calling() {
        let mut conn = database();
        let mut used = 8;
        let mut paid_calls = 0;
        let mut fetch = |path: &str, _params: &str| {
            if path == SPORTS_PATH {
                return Ok(response(200, 0, used, "[]"));
            }
            paid_calls += 1;
            used += 1;
            Ok(response(200, 1, used, fixture_for(path)))
        };

        // Plafond à 9 avec 8 déjà utilisés : une seule ligue passe.
        let report = run(
            &mut conn,
            &config(&["soccer_epl", "soccer_spl"], 9),
            &mut clock(),
            &mut fetch,
        )
        .unwrap();

        assert_eq!(paid_calls, 1);
        assert_eq!(report.leagues.len(), 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].sport_key, "soccer_spl");
        assert!(report.failures[0].reason.contains("garde de budget"));
        assert_eq!(report.used_after, 9);
    }

    #[test]
    fn unknown_quota_prevents_every_paid_call() {
        let mut conn = database();
        let mut paths = Vec::new();
        let mut fetch = |path: &str, _params: &str| {
            paths.push(path.to_string());
            Ok(HttpResponse {
                status: 200,
                quota: Quota::default(),
                body: "[]".to_string(),
            })
        };

        let result = run(
            &mut conn,
            &config(&["soccer_epl"], 450),
            &mut clock(),
            &mut fetch,
        );

        assert!(matches!(result.unwrap_err(), Error::QuotaUnknown));
        assert_eq!(paths, ["/sports"]);
    }

    #[test]
    fn an_api_error_on_one_league_does_not_stop_the_others() {
        let mut conn = database();
        let mut used = 8;
        let mut fetch = |path: &str, _params: &str| {
            if path == SPORTS_PATH {
                return Ok(response(200, 0, used, "[]"));
            }
            if path.contains("epl") {
                // L'API répond sans débiter : `used` ne bouge pas.
                return Ok(response(422, 0, used, INVALID_MARKET));
            }
            used += 1;
            Ok(response(200, 1, used, fixture_for(path)))
        };

        let report = run(
            &mut conn,
            &config(&["soccer_epl", "soccer_spl"], 450),
            &mut clock(),
            &mut fetch,
        )
        .unwrap();

        assert_eq!(report.leagues.len(), 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].sport_key, "soccer_epl");
        assert!(report.failures[0].reason.contains("INVALID_MARKET"));
        // L'appel en échec est tout de même journalisé, avec son statut.
        let failed_rows: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM api_call WHERE http_status = 422",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(failed_rows, 1);
    }

    #[test]
    fn a_network_error_is_reported_not_raised() {
        let mut conn = database();
        let mut fetch = |path: &str, _params: &str| {
            if path == SPORTS_PATH {
                return Ok(response(200, 0, 8, "[]"));
            }
            Err(Error::Http("délai dépassé".to_string()))
        };

        let report = run(
            &mut conn,
            &config(&["soccer_epl"], 450),
            &mut clock(),
            &mut fetch,
        )
        .unwrap();

        assert!(report.leagues.is_empty());
        assert!(report.failures[0].reason.contains("délai dépassé"));
        assert_eq!(
            count(&conn, "api_call"),
            1,
            "seul l'appel /sports est journalisé"
        );
    }

    #[test]
    fn an_unreadable_body_is_kept_raw_and_nothing_else_is_stored() {
        let mut conn = database();
        let mut fetch = |path: &str, _params: &str| {
            if path == SPORTS_PATH {
                return Ok(response(200, 0, 8, "[]"));
            }
            Ok(response(200, 1, 9, "<html>maintenance</html>"))
        };

        let report = run(
            &mut conn,
            &config(&["soccer_epl"], 450),
            &mut clock(),
            &mut fetch,
        )
        .unwrap();

        assert_eq!(report.failures.len(), 1);
        assert_eq!(count(&conn, "event_seen"), 0);
        let body: String = conn
            .query_row(
                "SELECT body FROM api_call WHERE endpoint = 'odds'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body, "<html>maintenance</html>");
    }

    #[test]
    fn matches_already_started_are_not_archived() {
        let mut conn = database();
        let mut used = 8;
        let mut fetch = |path: &str, _params: &str| {
            if path == SPORTS_PATH {
                return Ok(response(200, 0, used, "[]"));
            }
            used += 1;
            Ok(response(200, 1, used, fixture_for(path)))
        };

        let report = run(
            &mut conn,
            &config(&["soccer_italy_serie_a"], 450),
            &mut clock(),
            &mut fetch,
        )
        .unwrap();

        assert_eq!(report.leagues[0].already_started, 1);
        assert_eq!(report.leagues[0].stored.events, 1);
        // Mais le corps brut, lui, contient toujours le match commencé.
        let raw: String = conn
            .query_row(
                "SELECT body FROM api_call WHERE endpoint = 'odds'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(raw.contains("Udinese"));
    }

    #[test]
    fn a_quota_reset_is_reported() {
        let mut conn = database();
        archive::record_api_call(
            &conn,
            &NewApiCall {
                called_at: "2026-09-30T12:00:00Z".parse().unwrap(),
                endpoint: Endpoint::Sports,
                sport_key: None,
                params: "",
                http_status: 200,
                quota: Quota {
                    cost: Some(0),
                    used: Some(300),
                    remaining: Some(200),
                },
                body: None,
            },
        )
        .unwrap();
        let mut fetch = |_path: &str, _params: &str| Ok(response(200, 0, 3, "[]"));

        let report = run(&mut conn, &config(&[], 450), &mut clock(), &mut fetch).unwrap();

        assert!(report.quota_reset);
        assert_eq!(report.used_before, 3);
    }

    #[test]
    fn running_twice_keeps_both_snapshots() {
        let mut conn = database();
        let mut used = 8;
        for _ in 0..2 {
            let mut fetch = |path: &str, _params: &str| {
                if path == SPORTS_PATH {
                    return Ok(response(200, 0, used, "[]"));
                }
                used += 1;
                Ok(response(200, 1, used, fixture_for(path)))
            };
            run(
                &mut conn,
                &config(&["soccer_spl"], 450),
                &mut clock(),
                &mut fetch,
            )
            .unwrap();
        }

        assert_eq!(count(&conn, "event_seen"), 4);
        assert_eq!(count(&conn, "quote"), 12);
    }
}
