//! Assistants de test partagés : base en mémoire et relevés simulés (le match de Falkirk contre Kilmarnock).

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::domain::{Event, Odds, Quota, Quote};
use crate::store::archive::{Endpoint, NewApiCall, record_api_call, record_snapshot};
use crate::store::migrate;

/// « Maintenant » des tests : trois heures après le relevé de 14 h.
pub(crate) const NOW: &str = "2026-10-18T15:00:00Z";

/// Pinnacle : 2,00 / 3,60 / 4,00, soit des probabilités implicites 0,500 / 0,278 / 0,250 (somme 1,028).
pub(crate) const PINNACLE: (&str, f64, f64, f64) = ("pinnacle", 2.0, 3.6, 4.0);

pub(crate) fn database() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    migrate::migrate(&mut conn).unwrap();
    conn
}

pub(crate) fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

/// Un match à Falkirk le 24 octobre, avec les 1X2 `(bookmaker, domicile, nul, extérieur)`.
pub(crate) fn event(books: &[(&str, f64, f64, f64)]) -> Event {
    event_named("match-1", "2026-10-24T14:00:00Z", books)
}

/// Comme [`event`], avec un identifiant et un coup d'envoi choisis.
pub(crate) fn event_named(id: &str, kickoff: &str, books: &[(&str, f64, f64, f64)]) -> Event {
    let mut quotes = Vec::new();
    for (bookmaker, home, draw, away) in books {
        for (name, price) in [("Falkirk", home), ("Draw", draw), ("Kilmarnock", away)] {
            quotes.push(Quote {
                bookmaker_key: (*bookmaker).to_string(),
                market_key: "h2h".to_string(),
                updated_at: at("2026-10-18T13:59:00Z"),
                outcome: name.to_string(),
                point: None,
                price: Odds::new(*price).unwrap(),
            });
        }
    }
    Event {
        id: id.to_string(),
        sport_key: "soccer_spl".to_string(),
        commence_time: at(kickoff),
        home_team: "Falkirk".to_string(),
        away_team: "Kilmarnock".to_string(),
        quotes,
    }
}

/// Archive un relevé pris à `called_at` ; renvoie l'identifiant de l'appel.
pub(crate) fn store(conn: &mut Connection, called_at: &str, events: &[Event]) -> i64 {
    let call = NewApiCall {
        called_at: at(called_at),
        endpoint: Endpoint::Odds,
        sport_key: Some("soccer_spl"),
        params: "markets=h2h",
        http_status: 200,
        quota: Quota::default(),
        body: None,
    };
    let id = record_api_call(conn, &call).unwrap();
    record_snapshot(conn, id, events).unwrap();
    id
}
