//! Écriture de l'archive des cotes.
//!
//! Uniquement des `INSERT` : les triggers de la migration 2 interdisent toute modification ou
//! suppression. Les horodatages sont stockés en UTC au format RFC 3339 (« 2026-10-18T12:00:00Z »).

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{Event, Quota};
use crate::error::Result;

/// Point d'accès de l'API appelé (colonne `api_call.endpoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    /// `GET /v4/sports` : gratuit, sert à lire le quota.
    Sports,
    /// `GET /v4/sports/{clé}/odds` : les cotes d'une compétition.
    Odds,
    /// `GET /v4/sports/{clé}/scores` : les résultats récents. La contrainte de la table `api_call` (migration 1, immuable)
    /// ne connaît ni `scores` ni de quoi la modifier : ces appels portent le libellé `events`.
    Events,
}

impl Endpoint {
    fn as_str(self) -> &'static str {
        match self {
            Endpoint::Sports => "sports",
            Endpoint::Odds => "odds",
            Endpoint::Events => "events",
        }
    }
}

/// Un appel à l'API à journaliser.
///
/// Les références `&'a str` évitent de copier les textes : la structure vit le temps d'un appel de
/// fonction, pendant lequel les textes empruntés restent valides (`'a` relie les deux durées de vie).
#[derive(Debug)]
pub struct NewApiCall<'a> {
    /// Instant de l'appel (UTC).
    pub called_at: DateTime<Utc>,
    /// Point d'accès appelé.
    pub endpoint: Endpoint,
    /// Compétition concernée (`None` pour `/sports`).
    pub sport_key: Option<&'a str>,
    /// Paramètres envoyés, **sans la clé API**.
    pub params: &'a str,
    /// Code de statut HTTP de la réponse.
    pub http_status: u16,
    /// Compteurs de crédits lus dans les en-têtes.
    pub quota: Quota,
    /// Corps brut de la réponse (conservé pour `/odds`).
    pub body: Option<&'a str>,
}

/// Nombre de lignes écrites par [`record_snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredCounts {
    /// Matchs enregistrés.
    pub events: usize,
    /// Cotes enregistrées.
    pub quotes: usize,
}

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Journalise un appel et renvoie son identifiant.
pub fn record_api_call(conn: &Connection, call: &NewApiCall<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO api_call
             (called_at, endpoint, sport_key, params, http_status, cost, used, remaining, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            timestamp(call.called_at),
            call.endpoint.as_str(),
            call.sport_key,
            call.params,
            call.http_status,
            call.quota.cost,
            call.quota.used,
            call.quota.remaining,
            call.body,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Enregistre les matchs et les cotes d'une réponse `/odds`, rattachés à l'appel `api_call_id`.
///
/// Tout est écrit dans une seule transaction : un relevé est enregistré en entier ou pas du tout.
pub fn record_snapshot(
    conn: &mut Connection,
    api_call_id: i64,
    events: &[Event],
) -> Result<StoredCounts> {
    let tx = conn.transaction()?;
    let mut counts = StoredCounts {
        events: 0,
        quotes: 0,
    };
    // Les requêtes préparées empruntent `tx`. Or `tx.commit()` prend `tx` par valeur : il faut donc
    // que ces emprunts soient terminés avant. Le bloc `{ … }` les détruit à sa fermeture.
    {
        let mut insert_event = tx.prepare_cached(
            "INSERT INTO event_seen
                 (api_call_id, event_id, sport_key, commence_time, home_team_raw, away_team_raw)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut insert_quote = tx.prepare_cached(
            "INSERT INTO quote
                 (api_call_id, event_id, bookmaker_key, market_key, quote_updated_at,
                  outcome_raw, point, price)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for event in events {
            insert_event.execute(params![
                api_call_id,
                event.id,
                event.sport_key,
                timestamp(event.commence_time),
                event.home_team,
                event.away_team,
            ])?;
            counts.events += 1;
            for quote in &event.quotes {
                insert_quote.execute(params![
                    api_call_id,
                    event.id,
                    quote.bookmaker_key,
                    quote.market_key,
                    timestamp(quote.updated_at),
                    quote.outcome,
                    quote.point,
                    quote.price.get(),
                ])?;
                counts.quotes += 1;
            }
        }
    }
    tx.commit()?;
    Ok(counts)
}

/// Instant du dernier relevé de cotes réussi d'une famille de compétitions (`soccer_`, `tennis_`, `basketball_`).
///
/// Les appels « un match » (clôtures, paramètres `eventId=…`) ne comptent pas : ils ne disent rien de la fraîcheur du relevé.
pub fn last_odds_call(conn: &Connection, prefix: &str) -> Result<Option<DateTime<Utc>>> {
    let at: Option<String> = conn.query_row(
        "SELECT MAX(called_at) FROM api_call
         WHERE endpoint = 'odds' AND http_status = 200 AND sport_key IS NOT NULL
           AND substr(sport_key, 1, length(?1)) = ?1 AND params NOT LIKE 'eventId=%'",
        [prefix],
        |row| row.get(0),
    )?;
    at.map(|text| super::journal::parse_timestamp(&text))
        .transpose()
}

/// Dernier nombre de crédits consommés (`x-requests-used`) connu, tous appels confondus.
///
/// `None` si aucun appel n'a encore renvoyé cet en-tête. Les appels sans en-tête (401) sont ignorés.
pub fn last_known_usage(conn: &Connection) -> Result<Option<u32>> {
    // `optional()` transforme « aucune ligne » (une erreur pour `query_row`) en `Ok(None)`.
    let used = conn
        .query_row(
            "SELECT used FROM api_call WHERE used IS NOT NULL ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Odds, Quote};
    use crate::store::migrate;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn api_call(quota: Quota) -> NewApiCall<'static> {
        NewApiCall {
            called_at: at("2026-10-18T12:00:00Z"),
            endpoint: Endpoint::Odds,
            sport_key: Some("soccer_spl"),
            params: "markets=h2h",
            http_status: 200,
            quota,
            body: Some("[]"),
        }
    }

    fn quote(bookmaker: &str, outcome: &str, price: f64) -> Quote {
        Quote {
            bookmaker_key: bookmaker.to_string(),
            market_key: "h2h".to_string(),
            updated_at: at("2026-10-18T11:59:30Z"),
            outcome: outcome.to_string(),
            point: None,
            price: Odds::new(price).unwrap(),
        }
    }

    fn event() -> Event {
        Event {
            id: "abc123".to_string(),
            sport_key: "soccer_spl".to_string(),
            commence_time: at("2026-10-24T14:00:00Z"),
            home_team: "Falkirk F.C.".to_string(),
            away_team: "Kilmarnock".to_string(),
            quotes: vec![
                quote("pinnacle", "Falkirk F.C.", 2.5),
                quote("pinnacle", "Kilmarnock", 2.9),
                quote("pinnacle", "Draw", 3.4),
            ],
        }
    }

    fn count(conn: &Connection, table: &str) -> u32 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn the_last_odds_call_of_a_family_ignores_other_families_failures_and_single_match_calls() {
        let conn = database();
        assert_eq!(last_odds_call(&conn, "soccer_").unwrap(), None);
        conn.execute_batch(
            "INSERT INTO api_call (called_at, endpoint, sport_key, params, http_status) VALUES
               ('2026-10-18T12:00:00Z', 'odds', 'soccer_epl', 'markets=h2h', 200),
               ('2026-10-19T12:00:00Z', 'odds', 'soccer_spl', 'markets=h2h', 500),
               ('2026-10-20T12:00:00Z', 'odds', 'soccer_epl', 'eventId=abc&markets=h2h', 200),
               ('2026-10-19T14:00:00Z', 'odds', 'tennis_wta_x', 'markets=h2h', 200),
               ('2026-10-17T12:00:00Z', 'odds', 'soccer_spl', 'markets=h2h', 200)",
        )
        .unwrap();

        let football = last_odds_call(&conn, "soccer_").unwrap().unwrap();
        let tennis = last_odds_call(&conn, "tennis_").unwrap().unwrap();

        assert_eq!(football, at("2026-10-18T12:00:00Z"));
        assert_eq!(tennis, at("2026-10-19T14:00:00Z"));
        assert_eq!(last_odds_call(&conn, "basketball_").unwrap(), None);
    }

    #[test]
    fn last_known_usage_ignores_calls_without_headers() {
        let conn = database();
        assert_eq!(last_known_usage(&conn).unwrap(), None);

        let with_headers = Quota {
            cost: Some(1),
            used: Some(8),
            remaining: Some(492),
        };
        record_api_call(&conn, &api_call(with_headers)).unwrap();
        // Un appel plus récent sans en-têtes (cas du 401) ne doit pas masquer la valeur connue.
        record_api_call(&conn, &api_call(Quota::default())).unwrap();

        assert_eq!(last_known_usage(&conn).unwrap(), Some(8));
    }

    #[test]
    fn snapshot_stores_events_and_quotes_with_raw_names_and_utc_times() {
        let mut conn = database();
        let call_id = record_api_call(&conn, &api_call(Quota::default())).unwrap();

        let counts = record_snapshot(&mut conn, call_id, &[event()]).unwrap();

        assert_eq!(
            counts,
            StoredCounts {
                events: 1,
                quotes: 3
            }
        );
        let (home, commence): (String, String) = conn
            .query_row(
                "SELECT home_team_raw, commence_time FROM event_seen",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(home, "Falkirk F.C.");
        assert_eq!(commence, "2026-10-24T14:00:00Z");
    }

    #[test]
    fn two_snapshots_of_the_same_match_are_both_kept() {
        let mut conn = database();
        for _ in 0..2 {
            let call_id = record_api_call(&conn, &api_call(Quota::default())).unwrap();
            record_snapshot(&mut conn, call_id, &[event()]).unwrap();
        }

        assert_eq!(count(&conn, "event_seen"), 2);
        assert_eq!(count(&conn, "quote"), 6);
    }

    #[test]
    fn archive_tables_refuse_update_and_delete() {
        let mut conn = database();
        let call_id = record_api_call(&conn, &api_call(Quota::default())).unwrap();
        record_snapshot(&mut conn, call_id, &[event()]).unwrap();

        for statement in [
            "UPDATE api_call SET http_status = 500",
            "DELETE FROM api_call",
            "UPDATE event_seen SET home_team_raw = 'x'",
            "DELETE FROM event_seen",
            "UPDATE quote SET price = 9.9",
            "DELETE FROM quote",
        ] {
            let err = conn.execute(statement, []).unwrap_err();
            assert!(
                err.to_string().contains("archive immuable"),
                "{statement} doit être refusée : {err}"
            );
        }
        assert_eq!(count(&conn, "quote"), 3);
    }

    #[test]
    fn a_failing_snapshot_leaves_nothing_behind() {
        let mut conn = database();
        let call_id = record_api_call(&conn, &api_call(Quota::default())).unwrap();
        // Deux fois le même match dans une réponse : la clé primaire refuse le second.
        let result = record_snapshot(&mut conn, call_id, &[event(), event()]);

        assert!(result.is_err());
        assert_eq!(count(&conn, "event_seen"), 0);
        assert_eq!(count(&conn, "quote"), 0);
    }
}
