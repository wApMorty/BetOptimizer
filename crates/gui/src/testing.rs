//! Assistants de test : un dossier de données avec un relevé, une recommandation et un match à venir.

use chrono::{DateTime, Duration, Utc};
use edge_core::betting::recommend::{self, RECOMMEND_MIN_EDGE};
use edge_core::domain::{Euros, Event, Odds, Quota, Quote};
use edge_core::market::margin::MarginMethod;
use edge_core::paths::Paths;
use edge_core::store::archive::{Endpoint, NewApiCall, record_api_call, record_snapshot};
use edge_core::store::open_migrated;

/// Un dossier temporaire garni, à garder en vie pendant le test.
pub struct Sample {
    _dir: tempfile::TempDir,
    pub paths: Paths,
    pub now: DateTime<Utc>,
}

/// Relevé d'il y a une heure : Pinnacle donne 2,00 à la victoire à domicile et Betclic 2,30, pour un match dans deux jours.
/// La recommandation correspondante (n° 1, mise suggérée) est déjà écrite dans le journal.
pub fn sample() -> Sample {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let now = Utc::now();
    let called_at = now - Duration::hours(1);

    let mut conn = open_migrated(&paths.db_file).unwrap();
    let mut quotes = Vec::new();
    for (bookmaker, home, draw, away) in
        [("pinnacle", 2.0, 3.6, 4.0), ("betclic_fr", 2.3, 3.2, 3.4)]
    {
        for (name, price) in [("Falkirk", home), ("Draw", draw), ("Kilmarnock", away)] {
            quotes.push(Quote {
                bookmaker_key: bookmaker.to_string(),
                market_key: "h2h".to_string(),
                updated_at: called_at,
                outcome: name.to_string(),
                point: None,
                price: Odds::new(price).unwrap(),
            });
        }
    }
    let event = Event {
        id: "match-gui".to_string(),
        sport_key: "soccer_spl".to_string(),
        commence_time: now + Duration::days(2),
        home_team: "Falkirk".to_string(),
        away_team: "Kilmarnock".to_string(),
        quotes,
    };
    let call = NewApiCall {
        called_at,
        endpoint: Endpoint::Odds,
        sport_key: Some("soccer_spl"),
        params: "markets=h2h",
        http_status: 200,
        quota: Quota::default(),
        body: None,
    };
    let id = record_api_call(&conn, &call).unwrap();
    record_snapshot(&mut conn, id, &[event]).unwrap();

    let settings = recommend::Settings {
        method: MarginMethod::Power,
        min_edge: RECOMMEND_MIN_EDGE,
        kelly_fraction: 1.0,
        min_stake: Euros::new(1.0).unwrap(),
        bankroll: Euros::new(1000.0).unwrap(),
        code_version: "test".to_string(),
    };
    recommend::run(&mut conn, &settings, now).unwrap();
    Sample {
        _dir: dir,
        paths,
        now,
    }
}
