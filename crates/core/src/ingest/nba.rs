//! Relevé de la NBA : les cotes du soir (vainqueur, handicaps, totaux) et, tous les deux jours, les résultats.
//!
//! Lancé par un timer du soir (22 h UTC, soit 18 h à New York) : les cotes sont prises 1 à 4 heures avant les matchs du soir
//! américain, donc proches de la clôture. Comme pour le relevé quotidien, le réseau et l'horloge sont reçus en paramètres.
//!
//! Les résultats viennent de `/scores` (2 crédits, jusqu'à 3 jours en arrière). Un appel tous les deux jours laisse un jour de
//! recouvrement : un appel manqué ne perd aucun match, et l'écriture est idempotente.

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;

use super::budget;
use super::odds_api::{
    api_error, odds_params, parse_scores, scores_params, scores_path, sport_is_active,
};
use super::snapshot::{Clock, Fetch, LeagueReport, Outcome, read_sports, snapshot_league};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::store::archive::{self, Endpoint, NewApiCall};
use crate::store::nba::{last_scores_call, upsert_game};

/// Clé de The Odds API pour la NBA.
pub const SPORT_KEY: &str = "basketball_nba";

/// Jours en arrière demandés à `/scores` (le maximum de l'API).
pub const SCORES_DAYS: u8 = 3;

/// Coût d'un appel à `/scores` avec `daysFrom`.
pub const SCORES_COST: u32 = 2;

/// Délai minimal entre deux appels à `/scores`, en heures : 40 h, soit un appel un jour sur deux malgré un timer quotidien
/// légèrement décalé.
pub const SCORES_EVERY_HOURS: i64 = 40;

/// Ce que l'appel aux résultats a donné.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScoresReport {
    /// L'API a-t-elle été appelée ?
    pub fetched: bool,
    /// Pourquoi pas (trop récent, budget refusé, erreur de l'API…).
    pub skipped: Option<String>,
    /// Matchs terminés écrits ou mis à jour.
    pub stored: usize,
    /// Matchs à venir ou en cours dans la réponse.
    pub not_finished: usize,
    /// Matchs terminés dont un score manque ou est illisible.
    pub malformed: usize,
}

/// Bilan du relevé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NbaReport {
    /// La NBA est-elle en saison ? Sinon, aucun crédit n'est dépensé.
    pub in_season: bool,
    /// Cotes archivées.
    pub odds: Option<LeagueReport>,
    /// Pourquoi les cotes n'ont pas été archivées.
    pub odds_failure: Option<String>,
    /// Résultats.
    pub scores: ScoresReport,
    /// Crédits consommés avant le relevé.
    pub used_before: u32,
    /// Crédits consommés après.
    pub used_after: u32,
}

/// Coût des cotes d'un relevé : un crédit par marché et par groupe de 10 bookmakers.
pub fn odds_cost(config: &Config) -> u32 {
    budget::odds_cost(config.nba_markets.len(), config.bookmakers.len())
}

/// Effectue le relevé de la NBA.
pub fn run(conn: &mut Connection, config: &Config, now: Clock, fetch: Fetch) -> Result<NbaReport> {
    // L'appel à `/sports` est gratuit : il donne le quota et dit si la NBA est en saison.
    let (quota, sports_body) = read_sports(conn, now, fetch)?;
    let used_before = quota.used.ok_or(Error::QuotaUnknown)?;
    let mut report = NbaReport {
        in_season: false,
        odds: None,
        odds_failure: None,
        scores: ScoresReport::default(),
        used_before,
        used_after: used_before,
    };
    report.in_season = match sport_is_active(&sports_body, SPORT_KEY) {
        Ok(active) => active,
        Err(error) => {
            tracing::warn!(%error, "liste des sports illisible : pas de relevé NBA");
            false
        }
    };
    if !report.in_season {
        return Ok(report);
    }

    let cap = config.monthly_credit_cap;
    let mut used = used_before;

    match budget::check(Some(used), odds_cost(config), cap) {
        Err(refusal) => report.odds_failure = Some(refusal.to_string()),
        Ok(()) => {
            let params = odds_params(&config.nba_markets, &config.bookmakers);
            match snapshot_league(conn, config, SPORT_KEY, &params, now, fetch)? {
                Outcome::Stored(league) => report.odds = Some(league),
                Outcome::Failed(failure) => report.odds_failure = Some(failure.reason),
            }
            used = archive::last_known_usage(conn)?.unwrap_or(used);
        }
    }

    report.scores = fetch_scores(conn, used, cap, now, fetch)?;
    report.used_after = archive::last_known_usage(conn)?.unwrap_or(used);
    Ok(report)
}

/// Appelle `/scores` s'il est temps et si le budget le permet, puis écrit les matchs terminés.
fn fetch_scores(
    conn: &mut Connection,
    used: u32,
    cap: u32,
    now: Clock,
    fetch: Fetch,
) -> Result<ScoresReport> {
    let mut report = ScoresReport::default();
    let current = now();
    if let Some(last) = last_scores_call(conn)?
        && current - last < Duration::hours(SCORES_EVERY_HOURS)
    {
        report.skipped = Some(format!(
            "dernier appel il y a moins de {SCORES_EVERY_HOURS} h : un appel tous les deux jours suffit (recouvrement d'un jour)"
        ));
        return Ok(report);
    }
    if let Err(refusal) = budget::check(Some(used), SCORES_COST, cap) {
        report.skipped = Some(refusal.to_string());
        return Ok(report);
    }

    let params = scores_params(SCORES_DAYS);
    let response = match fetch(&scores_path(SPORT_KEY), &params) {
        Ok(response) => response,
        Err(network_error) => {
            report.skipped = Some(network_error.to_string());
            return Ok(report);
        }
    };
    report.fetched = true;
    let received_at = now();
    archive::record_api_call(
        conn,
        &NewApiCall {
            called_at: received_at,
            endpoint: Endpoint::Events,
            sport_key: Some(SPORT_KEY),
            params: &params,
            http_status: response.status,
            quota: response.quota,
            body: Some(&response.body),
        },
    )?;
    if response.status != 200 {
        report.skipped = Some(api_error(response.status, &response.body).to_string());
        return Ok(report);
    }
    let parsed = match parse_scores(&response.body) {
        Ok(parsed) => parsed,
        Err(format_error) => {
            report.skipped = Some(format_error.to_string());
            return Ok(report);
        }
    };
    let tx = conn.transaction()?;
    for game in &parsed.finished {
        upsert_game(&tx, game, received_at)?;
    }
    tx.commit()?;
    report.stored = parsed.finished.len();
    report.not_finished = parsed.not_finished;
    report.malformed = parsed.malformed;
    if parsed.malformed > 0 {
        tracing::warn!(
            matchs = parsed.malformed,
            "matchs terminés dont un score manque : non écrits"
        );
    }
    Ok(report)
}

/// Quand les résultats seront-ils demandés au prochain relevé ? `None` : maintenant.
pub fn next_scores_call(conn: &Connection, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>> {
    Ok(last_scores_call(conn)?
        .map(|last| last + Duration::hours(SCORES_EVERY_HOURS))
        .filter(|due| *due > now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Quota;
    use crate::ingest::client::HttpResponse;
    use crate::ingest::odds_api::SPORTS_PATH;
    use crate::store::migrate;
    use crate::store::nba::{count_games, load_games};

    const SPORTS: &str = include_str!("../../tests/fixtures/sports_with_nba.json");
    const ODDS: &str = include_str!("../../tests/fixtures/odds_nba.json");
    const SCORES: &str = include_str!("../../tests/fixtures/nba_scores.json");

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
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

    /// Lance un relevé au `moment` donné ; renvoie le bilan et les `(chemin, paramètres)` appelés.
    fn run_at(
        conn: &mut Connection,
        config: &Config,
        moment: &str,
        sports: &str,
        used_start: u32,
    ) -> (NbaReport, Vec<(String, String)>) {
        let mut calls = Vec::new();
        let mut used = used_start;
        let mut fetch = |path: &str, params: &str| {
            calls.push((path.to_string(), params.to_string()));
            if path == SPORTS_PATH {
                return Ok(response(200, 0, used, sports));
            }
            if path.ends_with("/scores/") {
                used += 2;
                return Ok(response(200, 2, used, SCORES));
            }
            used += 3;
            Ok(response(200, 3, used, ODDS))
        };
        let instant = at(moment);
        let report = run(conn, config, &mut || instant, &mut fetch).unwrap();
        (report, calls)
    }

    #[test]
    fn the_evening_snapshot_archives_three_markets_and_fetches_the_scores() {
        let mut conn = database();

        let (report, calls) = run_at(
            &mut conn,
            &Config::default(),
            "2026-10-19T22:00:00Z",
            SPORTS,
            20,
        );

        assert!(report.in_season);
        let paths: Vec<&str> = calls.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "/sports",
                "/sports/basketball_nba/odds",
                "/sports/basketball_nba/scores/"
            ]
        );
        assert_eq!(
            calls[1].1,
            "markets=h2h,spreads,totals&bookmakers=pinnacle,winamax_fr,betclic_fr,unibet_fr,pmu_fr,netbet_fr&oddsFormat=decimal&dateFormat=iso"
        );
        assert_eq!(calls[2].1, "daysFrom=3&dateFormat=iso");
        let odds = report.odds.unwrap();
        assert_eq!(odds.stored.events, 3);
        assert!(
            odds.stored.quotes > 30,
            "trois marchés par bookmaker : {}",
            odds.stored.quotes
        );
        assert!(report.scores.fetched);
        assert_eq!((report.scores.stored, report.scores.not_finished), (1, 2));
        assert_eq!(count_games(&conn).unwrap(), 1);
        assert_eq!(load_games(&conn).unwrap()[0].home_score, 112);
        assert_eq!((report.used_before, report.used_after), (20, 25));
    }

    #[test]
    fn the_scores_are_fetched_every_other_day_not_every_day() {
        let mut conn = database();
        let config = Config::default();
        run_at(&mut conn, &config, "2026-10-19T22:00:00Z", SPORTS, 20);

        let (next_day, calls) = run_at(&mut conn, &config, "2026-10-20T22:00:00Z", SPORTS, 25);
        let (two_days, _) = run_at(&mut conn, &config, "2026-10-21T22:30:00Z", SPORTS, 28);

        assert!(!next_day.scores.fetched);
        assert!(next_day.scores.skipped.unwrap().contains("40 h"));
        assert_eq!(calls.len(), 2, "les cotes seulement : /sports et /odds");
        assert!(two_days.scores.fetched, "48 h après : l'appel est dû");
        assert_eq!(
            count_games(&conn).unwrap(),
            1,
            "le même match rejoué n'est pas dupliqué"
        );
    }

    #[test]
    fn out_of_season_nothing_is_spent() {
        let mut conn = database();
        // Liste de sports sans la NBA (le fixture de football et de tennis).
        let off_season = include_str!("../../tests/fixtures/sports_with_tennis.json");

        let (report, calls) = run_at(
            &mut conn,
            &Config::default(),
            "2026-07-15T22:00:00Z",
            off_season,
            20,
        );

        assert!(!report.in_season);
        assert_eq!(calls.len(), 1, "seul l'appel gratuit à /sports");
        assert!(report.odds.is_none());
        assert!(!report.scores.fetched);
    }

    #[test]
    fn the_budget_guard_refuses_the_odds_then_the_scores_in_order() {
        let mut conn = database();
        // Plafond 22 avec 20 crédits utilisés : les cotes (3) ne passent pas, les résultats (2) passent.
        let config = Config {
            monthly_credit_cap: 22,
            ..Config::default()
        };

        let (report, calls) = run_at(&mut conn, &config, "2026-10-19T22:00:00Z", SPORTS, 20);

        assert!(report.odds.is_none());
        assert!(report.odds_failure.unwrap().contains("garde de budget"));
        assert!(report.scores.fetched);
        assert!(!calls.iter().any(|(p, _)| p.ends_with("/odds")));
    }

    #[test]
    fn next_scores_call_tells_when_the_results_are_due() {
        let mut conn = database();
        assert_eq!(
            next_scores_call(&conn, at("2026-10-19T22:00:00Z")).unwrap(),
            None
        );
        run_at(
            &mut conn,
            &Config::default(),
            "2026-10-19T22:00:00Z",
            SPORTS,
            20,
        );

        let due = next_scores_call(&conn, at("2026-10-20T10:00:00Z")).unwrap();

        assert_eq!(due, Some(at("2026-10-21T14:00:00Z")));
        assert_eq!(
            next_scores_call(&conn, at("2026-10-22T00:00:00Z")).unwrap(),
            None
        );
    }
}
