//! Cotes de clôture : un relevé de Pinnacle juste avant le coup d'envoi des matchs qui ont une recommandation ou un pari.
//!
//! C'est ce relevé qui donne le **CLV** (cote obtenue contre probabilité de clôture de Pinnacle sans marge), la mesure
//! principale du projet. Il coûte un crédit par match, et seulement pour les matchs suivis (recommandations des deux
//! paliers et paris réels) : la commande est lancée toutes les dix minutes mais n'appelle l'API que si un match suivi
//! commence dans les 30 minutes et n'a pas encore sa clôture.
//!
//! Comme pour le relevé quotidien, le réseau et l'horloge sont reçus en paramètres.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};

use super::budget;
use super::odds_api::{
    api_error, event_odds_params, event_odds_path, event_request_params, parse_event_odds,
};
use super::snapshot::{Clock, Fetch};
use crate::config::Config;
use crate::error::Result;
use crate::store::archive::{self, Endpoint, NewApiCall};
use crate::store::journal::timestamp;

/// Fenêtre avant le coup d'envoi : un match entre en jeu 30 minutes avant, et un relevé de Pinnacle pris dans cette
/// fenêtre est sa clôture (le relevé quotidien de 14 h peut en tenir lieu pour un match de 14 h 15).
pub const WINDOW_MINUTES: i64 = 30;

/// Nombre maximal d'appels par match : un échec (match retiré, panne) ne se rejoue qu'une fois, pas toutes les dix minutes.
pub const MAX_ATTEMPTS: i64 = 2;

/// Bookmaker dont on relève la clôture.
const CLOSING_BOOKMAKER: &str = "pinnacle";

/// Un match à relever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Identifiant du match chez The Odds API.
    pub event_id: String,
    /// Compétition.
    pub sport_key: String,
    /// Coup d'envoi (UTC).
    pub kickoff: DateTime<Utc>,
}

/// Matchs suivis qui commencent dans les 30 minutes, sans clôture de Pinnacle, et dont les tentatives ne sont pas épuisées.
pub fn pending(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<Target>> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT event_id, sport_key, kickoff FROM (
             SELECT event_id, sport_key, kickoff FROM recommendation
             UNION
             SELECT event_id, sport_key, kickoff FROM bet
         ) WHERE kickoff > ?1 AND kickoff <= ?2 ORDER BY kickoff, event_id",
    )?;
    let horizon = now + Duration::minutes(WINDOW_MINUTES);
    let rows = statement.query_map(params![timestamp(now), timestamp(horizon)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut targets = Vec::new();
    for row in rows {
        let (event_id, sport_key, kickoff) = row?;
        let kickoff = crate::store::journal::parse_timestamp(&kickoff)?;
        if has_closing(conn, &event_id, kickoff)? || attempts(conn, &event_id)? >= MAX_ATTEMPTS {
            continue;
        }
        targets.push(Target {
            event_id,
            sport_key,
            kickoff,
        });
    }
    Ok(targets)
}

/// Un relevé de Pinnacle pris dans les 30 minutes avant le coup d'envoi existe-t-il déjà ?
pub fn has_closing(conn: &Connection, event_id: &str, kickoff: DateTime<Utc>) -> Result<bool> {
    let since = timestamp(kickoff - Duration::minutes(WINDOW_MINUTES));
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM quote q JOIN api_call a ON a.id = q.api_call_id
         WHERE q.event_id = ?1 AND q.bookmaker_key = ?2 AND q.market_key = 'h2h' AND a.called_at >= ?3",
        params![event_id, CLOSING_BOOKMAKER, since],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Nombre d'appels « un match » déjà faits pour ce match (réussis ou non).
fn attempts(conn: &Connection, event_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM api_call WHERE endpoint = 'odds' AND params LIKE ?1",
        [format!("eventId={event_id}&%")],
        |row| row.get(0),
    )?)
}

/// Un match dont la clôture a été relevée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// Le match.
    pub target: Target,
    /// Pinnacle avait-il une cote ? Sinon la clôture est inutilisable (et un seul nouvel essai est permis).
    pub has_pinnacle: bool,
}

/// Un match dont la clôture n'a pas été relevée, avec la raison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Le match.
    pub target: Target,
    /// Raison lisible (garde de budget, erreur de l'API, réseau, réponse illisible).
    pub reason: String,
}

/// Bilan d'une exécution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClosingReport {
    /// Clôtures relevées.
    pub captured: Vec<Captured>,
    /// Clôtures manquées.
    pub failures: Vec<Failure>,
}

/// Relève la clôture de chaque match en attente.
///
/// Sans match en attente, aucun appel n'est fait (pas même le quota, qui est gratuit mais inutile).
pub fn run(
    conn: &mut Connection,
    config: &Config,
    now: Clock,
    fetch: Fetch,
) -> Result<ClosingReport> {
    let mut report = ClosingReport::default();
    let targets = pending(conn, now())?;
    if targets.is_empty() {
        return Ok(report);
    }
    let bookmakers = [CLOSING_BOOKMAKER.to_string()];
    let cost = budget::odds_cost(config.markets.len(), bookmakers.len());

    for target in targets {
        let used = archive::last_known_usage(conn)?;
        if let Err(refusal) = budget::check(used, cost, config.monthly_credit_cap) {
            tracing::warn!(match_ = %target.event_id, "{refusal}");
            report.failures.push(Failure {
                target,
                reason: refusal.to_string(),
            });
            continue;
        }
        let Some(path) = event_odds_path(&target.sport_key, &target.event_id) else {
            report.failures.push(Failure {
                reason: "identifiant de match inattendu : appel refusé".to_string(),
                target,
            });
            continue;
        };
        let logged = event_odds_params(&target.event_id, &config.markets, &bookmakers);

        let response = match fetch(&path, event_request_params(&logged)) {
            Ok(response) => response,
            Err(network_error) => {
                report.failures.push(Failure {
                    reason: network_error.to_string(),
                    target,
                });
                continue;
            }
        };
        let received_at = now();
        // Journal d'abord : le corps brut est conservé et le crédit tracé même si la suite échoue.
        let api_call_id = archive::record_api_call(
            conn,
            &NewApiCall {
                called_at: received_at,
                endpoint: Endpoint::Odds,
                sport_key: Some(&target.sport_key),
                params: &logged,
                http_status: response.status,
                quota: response.quota,
                body: Some(&response.body),
            },
        )?;
        if response.quota.cost.is_some_and(|actual| actual != cost) {
            tracing::warn!(
                attendu = cost,
                reel = ?response.quota.cost,
                "un appel par match a coûté autre chose que prévu : à consigner dans docs/sources.md"
            );
        }
        if response.status != 200 {
            report.failures.push(Failure {
                reason: api_error(response.status, &response.body).to_string(),
                target,
            });
            continue;
        }
        let parsed = match parse_event_odds(&response.body, received_at) {
            Ok(parsed) => parsed,
            Err(format_error) => {
                report.failures.push(Failure {
                    reason: format_error.to_string(),
                    target,
                });
                continue;
            }
        };
        archive::record_snapshot(conn, api_call_id, &parsed.events)?;
        let has_pinnacle = parsed
            .events
            .iter()
            .flat_map(|event| &event.quotes)
            .any(|quote| quote.bookmaker_key == CLOSING_BOOKMAKER);
        tracing::info!(match_ = %target.event_id, pinnacle = has_pinnacle, "clôture relevée");
        report.captured.push(Captured {
            target,
            has_pinnacle,
        });
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::testing::{at, database};
    use crate::domain::{Euros, Odds, Outcome, Probability, Quota};
    use crate::ingest::client::HttpResponse;
    use crate::store::journal::{
        NewBet, NewRecommendation, Tier, insert_bet, insert_recommendation,
    };

    const NOW: &str = "2026-10-24T13:50:00Z";
    const KICKOFF_SOON: &str = "2026-10-24T14:10:00Z";

    fn config() -> Config {
        Config::default()
    }

    fn recommendation(event_id: &str, kickoff: &str) -> NewRecommendation {
        NewRecommendation {
            rule: "price_vs_pinnacle".into(),
            tier: Tier::Recommended,
            event_id: event_id.into(),
            sport_key: "soccer_spl".into(),
            home_team: "Falkirk".into(),
            away_team: "Kilmarnock".into(),
            kickoff: at(kickoff),
            bookmaker: "betclic_fr".into(),
            outcome: Outcome::Home,
            odds: Odds::new(2.3).unwrap(),
            probability: Probability::new(0.4865).unwrap(),
            edge: 0.1189,
            min_edge: 0.03,
            minimal_odds: 2.117,
            stake: Euros::new(91.4),
            kelly_fraction: 1.0,
            bankroll: Euros::new(1000.0).unwrap(),
            margin_method: "power".into(),
            api_call_id: 1,
            code_version: "test".into(),
        }
    }

    fn conn_with_recommendation(event_id: &str, kickoff: &str) -> Connection {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO api_call (id, called_at, endpoint, sport_key, params, http_status, used)
             VALUES (1, '2026-10-24T12:00:00Z', 'odds', 'soccer_spl', 'markets=h2h', 200, 20)",
        )
        .unwrap();
        insert_recommendation(
            &conn,
            &recommendation(event_id, kickoff),
            at("2026-10-24T12:05:00Z"),
        )
        .unwrap();
        conn
    }

    /// Réponse d'un appel par match : Pinnacle seul, 1X2.
    fn event_body(id: &str, kickoff: &str) -> String {
        format!(
            r#"{{"id":"{id}","sport_key":"soccer_spl","sport_title":"x","commence_time":"{kickoff}",
            "home_team":"Falkirk","away_team":"Kilmarnock","bookmakers":[{{"key":"pinnacle","title":"Pinnacle",
            "last_update":"2026-10-24T13:49:00Z","markets":[{{"key":"h2h","last_update":"2026-10-24T13:49:00Z",
            "outcomes":[{{"name":"Falkirk","price":2.1}},{{"name":"Kilmarnock","price":3.9}},{{"name":"Draw","price":3.5}}]}}]}}]}}"#
        )
    }

    fn ok(body: String, used: u32) -> HttpResponse {
        HttpResponse {
            status: 200,
            quota: Quota {
                cost: Some(1),
                used: Some(used),
                remaining: Some(500 - used),
            },
            body,
        }
    }

    #[test]
    fn a_followed_match_starting_soon_without_a_closing_is_pending() {
        let conn = conn_with_recommendation("m1", KICKOFF_SOON);

        let targets = pending(&conn, at(NOW)).unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].event_id, "m1");
    }

    #[test]
    fn matches_too_far_or_already_started_are_not_pending() {
        let far = conn_with_recommendation("m1", "2026-10-24T16:00:00Z");
        let started = conn_with_recommendation("m1", "2026-10-24T13:40:00Z");

        assert!(pending(&far, at(NOW)).unwrap().is_empty());
        assert!(pending(&started, at(NOW)).unwrap().is_empty());
    }

    #[test]
    fn a_bet_alone_also_makes_its_match_followed() {
        let conn = database();
        insert_bet(
            &conn,
            &NewBet {
                recommendation_id: None,
                event_id: "off".into(),
                sport_key: "soccer_spl".into(),
                home_team: "Falkirk".into(),
                away_team: "Kilmarnock".into(),
                kickoff: at(KICKOFF_SOON),
                bookmaker: "winamax_fr".into(),
                outcome: Outcome::Draw,
                odds: Odds::new(3.4).unwrap(),
                stake: Euros::new(5.0).unwrap(),
            },
            at("2026-10-24T12:00:00Z"),
        )
        .unwrap();

        assert_eq!(pending(&conn, at(NOW)).unwrap().len(), 1);
    }

    #[test]
    fn nothing_is_pending_when_no_followed_match_starts_soon_and_no_call_is_made() {
        let mut conn = database();
        let mut calls = 0;

        let report = run(&mut conn, &config(), &mut || at(NOW), &mut |_, _| {
            calls += 1;
            Err(crate::error::Error::Http("ne doit pas être appelé".into()))
        })
        .unwrap();

        assert_eq!(report, ClosingReport::default());
        assert_eq!(calls, 0);
    }

    #[test]
    fn a_capture_is_archived_journaled_with_the_match_id_and_closes_the_match() {
        let mut conn = conn_with_recommendation("m1", KICKOFF_SOON);
        let mut seen = None;

        let report = run(
            &mut conn,
            &config(),
            &mut || at(NOW),
            &mut |path, params| {
                seen = Some((path.to_string(), params.to_string()));
                Ok(ok(event_body("m1", KICKOFF_SOON), 20))
            },
        )
        .unwrap();

        assert_eq!(report.captured.len(), 1);
        assert!(report.captured[0].has_pinnacle);
        let (path, params) = seen.unwrap();
        assert_eq!(path, "/sports/soccer_spl/events/m1/odds");
        assert_eq!(
            params,
            "markets=h2h&bookmakers=pinnacle&oddsFormat=decimal&dateFormat=iso"
        );
        let logged: String = conn
            .query_row(
                "SELECT params FROM api_call WHERE params LIKE 'eventId=%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(logged.starts_with("eventId=m1&markets=h2h"));
        assert!(has_closing(&conn, "m1", at(KICKOFF_SOON)).unwrap());
        assert!(pending(&conn, at(NOW)).unwrap().is_empty());
    }

    #[test]
    fn the_budget_guard_stops_the_call() {
        let mut conn = conn_with_recommendation("m1", KICKOFF_SOON);
        conn.execute_batch(
            "INSERT INTO api_call (called_at, endpoint, params, http_status, used) VALUES ('2026-10-24T13:00:00Z', 'sports', '', 200, 450)",
        )
        .unwrap();
        let mut calls = 0;

        let report = run(&mut conn, &config(), &mut || at(NOW), &mut |_, _| {
            calls += 1;
            Ok(ok(event_body("m1", KICKOFF_SOON), 451))
        })
        .unwrap();

        assert_eq!(calls, 0);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].reason.contains("garde de budget"));
    }

    #[test]
    fn an_api_error_is_journaled_and_retried_only_once() {
        let mut conn = conn_with_recommendation("m1", KICKOFF_SOON);
        let mut calls = 0;
        let mut failing = |_: &str, _: &str| {
            calls += 1;
            Ok(HttpResponse {
                status: 404,
                quota: Quota {
                    cost: Some(0),
                    used: Some(20),
                    remaining: Some(480),
                },
                body: r#"{"message":"Event not found","error_code":"EVENT_NOT_FOUND"}"#.to_string(),
            })
        };

        for _ in 0..4 {
            run(&mut conn, &config(), &mut || at(NOW), &mut failing).unwrap();
        }

        assert_eq!(
            calls as i64, MAX_ATTEMPTS,
            "pas de boucle d'appels sur un match introuvable"
        );
    }

    #[test]
    fn a_response_without_pinnacle_is_stored_but_does_not_count_as_a_closing() {
        let mut conn = conn_with_recommendation("m1", KICKOFF_SOON);
        let empty = format!(
            r#"{{"id":"m1","sport_key":"soccer_spl","commence_time":"{KICKOFF_SOON}","home_team":"Falkirk","away_team":"Kilmarnock","bookmakers":[]}}"#
        );

        let report = run(&mut conn, &config(), &mut || at(NOW), &mut |_, _| {
            Ok(ok(empty.clone(), 20))
        })
        .unwrap();

        assert!(!report.captured[0].has_pinnacle);
        assert!(!has_closing(&conn, "m1", at(KICKOFF_SOON)).unwrap());
    }

    #[test]
    fn the_daily_snapshot_taken_close_enough_to_kickoff_already_counts() {
        let mut conn = conn_with_recommendation("m1", "2026-10-24T14:15:00Z");
        crate::betting::testing::store(
            &mut conn,
            "2026-10-24T13:50:00Z",
            &[crate::betting::testing::event_named(
                "m1",
                "2026-10-24T14:15:00Z",
                &[crate::betting::testing::PINNACLE],
            )],
        );

        assert!(
            pending(&conn, at("2026-10-24T13:55:00Z"))
                .unwrap()
                .is_empty()
        );
    }
}
