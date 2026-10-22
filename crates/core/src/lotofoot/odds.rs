//! Probabilités du marché des rencontres d'une grille : cote Pinnacle de notre archive The Odds API, marge retirée.
//!
//! 1. Les noms de prono-match sont traduits en noms de The Odds API par la section `LOTOFOOT` de `config/aliases.toml` :
//!    un nom inconnu bloque la grille (règle du projet : jamais d'appariement approximatif).
//! 2. On cherche dans l'archive le dernier relevé Pinnacle du match, assez récent.
//! 3. Pour les matchs absents, on trouve leur compétition avec `/events` (gratuit) parmi les compétitions actives, puis on
//!    relève ces compétitions avec `/odds` (1 crédit chacune), sous la garde de budget et un plafond par séance.

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::config::Config;
use crate::domain::{Odds, Odds1x2, Probs1x2};
use crate::error::Result;
use crate::ingest::aliases::Aliases;
use crate::ingest::budget;
use crate::ingest::odds_api::{events_path, odds_params, parse_events, sport_is_active};
use crate::ingest::snapshot::{self, Clock, Fetch, Outcome as LeagueOutcome};
use crate::market::margin::{MarginMethod, remove_margin};
use crate::store::archive::{self, Endpoint, NewApiCall};

/// Section de la table d'alias : noms de prono-match → noms de The Odds API.
pub const ALIAS_SECTION: &str = "LOTOFOOT";

/// Bookmaker de référence.
const REFERENCE: &str = "pinnacle";

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Noms de The Odds API des rencontres d'une grille, ou la liste des noms inconnus (au format `LOTOFOOT: nom`).
pub fn canonical_matches(
    aliases: &Aliases,
    matches: &[(String, String)],
) -> std::result::Result<Vec<(String, String)>, Vec<String>> {
    let mut unknown = Vec::new();
    let mut out = Vec::new();
    for (home, away) in matches {
        let h = aliases.resolve(ALIAS_SECTION, home);
        let a = aliases.resolve(ALIAS_SECTION, away);
        if h.is_none() {
            unknown.push(format!("{ALIAS_SECTION}: {home}"));
        }
        if a.is_none() {
            unknown.push(format!("{ALIAS_SECTION}: {away}"));
        }
        if let (Some(h), Some(a)) = (h, a) {
            out.push((h.to_string(), a.to_string()));
        }
    }
    if unknown.is_empty() {
        Ok(out)
    } else {
        Err(unknown)
    }
}

/// Probabilités du marché d'une rencontre et l'appel d'où vient la cote.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchMarket {
    /// Probabilités sans marge (méthode puissance).
    pub probs: Probs1x2,
    /// Appel `/odds` d'où vient la cote.
    pub api_call_id: i64,
    /// Compétition.
    pub sport_key: String,
}

/// Dernière cote Pinnacle 1X2 archivée du match `home – away` (noms de The Odds API) dont le coup d'envoi tombe dans
/// `[from ; until]` et relevée après `fresh_after`. `None` si l'archive n'en a pas.
pub fn archived_market(
    conn: &Connection,
    teams: (&str, &str),
    from: DateTime<Utc>,
    until: DateTime<Utc>,
    fresh_after: DateTime<Utc>,
    method: MarginMethod,
) -> Result<Option<MatchMarket>> {
    let (home, away) = teams;
    let found: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT e.api_call_id, e.event_id, e.sport_key FROM event_seen e JOIN api_call a ON a.id = e.api_call_id
             WHERE e.home_team_raw = ?1 AND e.away_team_raw = ?2 AND e.commence_time BETWEEN ?3 AND ?4
               AND a.called_at >= ?5
               AND EXISTS (SELECT 1 FROM quote q WHERE q.api_call_id = e.api_call_id AND q.event_id = e.event_id
                           AND q.bookmaker_key = ?6 AND q.market_key = 'h2h')
             ORDER BY a.called_at DESC LIMIT 1",
            params![home, away, timestamp(from), timestamp(until), timestamp(fresh_after), REFERENCE],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((api_call_id, event_id, sport_key)) = found else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT outcome_raw, price FROM quote WHERE api_call_id = ?1 AND event_id = ?2 AND bookmaker_key = ?3
         AND market_key = 'h2h'",
    )?;
    let prices = stmt
        .query_map(params![api_call_id, event_id, REFERENCE], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let price = |name: &str| {
        prices
            .iter()
            .find(|(o, _)| o == name)
            .and_then(|(_, p)| Odds::new(*p))
    };
    let (Some(h), Some(d), Some(a)) = (price(home), price("Draw"), price(away)) else {
        return Ok(None);
    };
    let odds = Odds1x2 {
        home: h,
        draw: d,
        away: a,
    };
    Ok(remove_margin(&odds, method).map(|probs| MatchMarket {
        probs,
        api_call_id,
        sport_key,
    }))
}

/// Bilan de la recherche des compétitions manquantes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeagueSearch {
    /// Compétitions listées par `/events`.
    pub listed: usize,
    /// Compétitions relevées (`/odds`).
    pub fetched: Vec<String>,
    /// Compétitions nécessaires mais non relevées, avec la raison (plafond de la séance, garde de budget, erreur).
    pub refused: Vec<String>,
    /// Crédits dépensés.
    pub spent: u32,
}

/// Cherche et relève les compétitions des matchs `wanted` (noms de The Odds API) dont le coup d'envoi tombe dans
/// `[from ; until]`. `/sports` et `/events` sont gratuits ; chaque `/odds` coûte un crédit.
pub fn refresh_leagues(
    conn: &mut Connection,
    config: &Config,
    wanted: &[(String, String)],
    window: (DateTime<Utc>, DateTime<Utc>),
    now: Clock,
    fetch: Fetch,
) -> Result<LeagueSearch> {
    let mut search = LeagueSearch::default();
    if wanted.is_empty() {
        return Ok(search);
    }
    let (_, sports_body) = snapshot::read_sports(conn, now, fetch)?;
    // Ordre conservé (les ligues archivées d'abord), doublons retirés.
    let mut keys: Vec<&String> = Vec::new();
    for key in config.leagues.iter().chain(&config.lotofoot_leagues) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let mut needed: Vec<String> = Vec::new();
    let mut found = vec![false; wanted.len()];
    for key in keys {
        if found.iter().all(|f| *f) {
            break;
        }
        if !sport_is_active(&sports_body, key).unwrap_or(false) {
            continue;
        }
        let path = events_path(key);
        let params = "dateFormat=iso";
        let response = match fetch(&path, params) {
            Ok(response) => response,
            Err(error) => {
                search.refused.push(format!("{key} : {error}"));
                continue;
            }
        };
        archive::record_api_call(
            conn,
            &NewApiCall {
                called_at: now(),
                endpoint: Endpoint::Events,
                sport_key: Some(key),
                params,
                http_status: response.status,
                quota: response.quota,
                body: None,
            },
        )?;
        if response.status != 200 {
            continue;
        }
        search.listed += 1;
        let Ok(events) = parse_events(&response.body) else {
            search
                .refused
                .push(format!("{key} : liste des matchs illisible"));
            continue;
        };
        let mut useful = false;
        for event in &events {
            if event.commence_time < window.0 || event.commence_time > window.1 {
                continue;
            }
            for (i, (home, away)) in wanted.iter().enumerate() {
                if !found[i] && event.home_team == *home && event.away_team == *away {
                    found[i] = true;
                    useful = true;
                }
            }
        }
        if useful {
            needed.push(key.clone());
        }
    }

    let params = odds_params(&config.markets, &config.bookmakers);
    let cost = budget::odds_cost(config.markets.len(), config.bookmakers.len());
    for key in needed {
        if search.spent.saturating_add(cost) > config.lotofoot_session_credits {
            search.refused.push(format!(
                "{key} : plafond de {} crédits de la séance atteint",
                config.lotofoot_session_credits
            ));
            continue;
        }
        if let Err(refusal) = budget::check(
            archive::last_known_usage(conn)?,
            cost,
            config.monthly_credit_cap,
        ) {
            search.refused.push(format!("{key} : {refusal}"));
            continue;
        }
        match snapshot::snapshot_league(conn, config, &key, &params, now, fetch)? {
            LeagueOutcome::Stored(_) => {
                search.spent += cost;
                search.fetched.push(key);
            }
            LeagueOutcome::Failed(failure) => {
                search.refused.push(format!("{key} : {}", failure.reason))
            }
        }
    }
    Ok(search)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Quota;
    use crate::ingest::client::HttpResponse;
    use crate::store::migrate::migrate;

    const ALIASES: &str = r#"
[LOTOFOOT]
"Paris Saint Germain" = ["Paris SG"]
"Lyon" = []
"Pau FC" = ["Pau"]
"Grenoble" = []
"#;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn quota(used: u32) -> Quota {
        Quota {
            remaining: Some(500 - used),
            used: Some(used),
            cost: Some(0),
        }
    }

    #[test]
    fn unknown_names_block_the_grid_with_the_names() {
        let aliases = Aliases::parse(ALIASES).unwrap();
        let ok = canonical_matches(&aliases, &[("Paris SG".into(), "Lyon".into())]).unwrap();
        assert_eq!(
            ok,
            vec![("Paris Saint Germain".to_string(), "Lyon".to_string())]
        );
        let err = canonical_matches(
            &aliases,
            &[
                ("Paris SG".into(), "Lyon".into()),
                ("Inconnu FC".into(), "Lyon".into()),
            ],
        );
        assert_eq!(err.unwrap_err(), vec!["LOTOFOOT: Inconnu FC".to_string()]);
    }

    /// Le match de Ligue 2 est absent de l'archive : `/events` le trouve, `/odds` le relève (1 crédit), puis l'archive
    /// fournit sa cote sans marge.
    #[test]
    fn a_missing_league_is_found_then_fetched_under_the_budget() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let config = Config::default();
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut calls: Vec<String> = Vec::new();
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            calls.push(path.to_string());
            let body = match path {
                "/sports" => r#"[{"key":"soccer_france_ligue_two","group":"Soccer","title":"Ligue 2","description":"","active":true,"has_outrights":false},
                                {"key":"soccer_epl","group":"Soccer","title":"EPL","description":"","active":true,"has_outrights":false}]"#.to_string(),
                "/sports/soccer_france_ligue_two/events" => r#"[{"id":"e1","sport_key":"soccer_france_ligue_two",
                    "commence_time":"2026-10-24T18:00:00Z","home_team":"Pau FC","away_team":"Grenoble"}]"#.to_string(),
                "/sports/soccer_france_ligue_two/odds" => r#"[{"id":"e1","sport_key":"soccer_france_ligue_two",
                    "commence_time":"2026-10-24T18:00:00Z","home_team":"Pau FC","away_team":"Grenoble",
                    "bookmakers":[{"key":"pinnacle","markets":[{"key":"h2h","last_update":"2026-10-24T09:58:00Z",
                    "outcomes":[{"name":"Pau FC","price":1.8},{"name":"Draw","price":3.47},{"name":"Grenoble","price":4.66}]}]}]}]"#.to_string(),
                _ => "[]".to_string(),
            };
            Ok(HttpResponse {
                status: 200,
                quota: quota(80),
                body,
            })
        };
        let wanted = vec![("Pau FC".to_string(), "Grenoble".to_string())];
        let window = (at("2026-10-24T10:00:00Z"), at("2026-10-28T00:00:00Z"));
        let search =
            refresh_leagues(&mut conn, &config, &wanted, window, &mut clock, &mut fetch).unwrap();
        assert_eq!(search.fetched, vec!["soccer_france_ligue_two".to_string()]);
        assert_eq!(search.spent, 1);
        // Les compétitions actives sont parcourues dans l'ordre de `leagues` puis de `lotofoot_leagues` : la Premier League
        // (vide ici) passe avant la Ligue 2 ; la recherche s'arrête dès que tous les matchs sont trouvés.
        assert_eq!(
            calls,
            [
                "/sports",
                "/sports/soccer_epl/events",
                "/sports/soccer_france_ligue_two/events",
                "/sports/soccer_france_ligue_two/odds"
            ]
        );
        let market = archived_market(
            &conn,
            ("Pau FC", "Grenoble"),
            window.0,
            window.1,
            at("2026-10-23T10:00:00Z"),
            MarginMethod::Power,
        )
        .unwrap()
        .unwrap();
        assert_eq!(market.sport_key, "soccer_france_ligue_two");
        let p = market.probs.to_array();
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(p[0] > p[2]);
        // Une cote trop ancienne ne sert pas.
        assert!(
            archived_market(
                &conn,
                ("Pau FC", "Grenoble"),
                window.0,
                window.1,
                at("2026-10-24T11:00:00Z"),
                MarginMethod::Power
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn the_session_cap_refuses_extra_leagues() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let config = Config {
            lotofoot_session_credits: 0,
            ..Config::default()
        };
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            let body = match path {
                "/sports" => r#"[{"key":"soccer_france_ligue_two","group":"Soccer","title":"Ligue 2","description":"","active":true,"has_outrights":false}]"#.to_string(),
                "/sports/soccer_france_ligue_two/events" => r#"[{"id":"e1","sport_key":"soccer_france_ligue_two",
                    "commence_time":"2026-10-24T18:00:00Z","home_team":"Pau FC","away_team":"Grenoble"}]"#.to_string(),
                _ => panic!("aucun /odds ne doit partir : {path}"),
            };
            Ok(HttpResponse {
                status: 200,
                quota: quota(80),
                body,
            })
        };
        let wanted = vec![("Pau FC".to_string(), "Grenoble".to_string())];
        let window = (at("2026-10-24T10:00:00Z"), at("2026-10-28T00:00:00Z"));
        let search =
            refresh_leagues(&mut conn, &config, &wanted, window, &mut clock, &mut fetch).unwrap();
        assert!(search.fetched.is_empty());
        assert_eq!(search.spent, 0);
        assert_eq!(search.refused.len(), 1);
    }
}
