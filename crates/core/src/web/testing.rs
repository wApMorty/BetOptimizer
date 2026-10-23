//! Assistants de test : un dossier de données avec un relevé, une recommandation, un match à venir et une séance
//! Loto Foot, et l'application qui le sert.

use chrono::{DateTime, Duration, Utc};

use super::{App, Method, Request, Response, handle};
use crate::betting::recommend::{self, RECOMMEND_MIN_EDGE};
use crate::config::{Config, ServeToken};
use crate::domain::{Euros, Event, Odds, Quota, Quote};
use crate::ingest::aliases::Aliases;
use crate::lotofoot::pronomatch::{GridPage, OpenGrid};
use crate::lotofoot::{Game, Ticket};
use crate::market::margin::MarginMethod;
use crate::paths::Paths;
use crate::store::archive::{Endpoint, NewApiCall, record_api_call, record_snapshot};
use crate::store::lotofoot::{
    NewReco, NewSession, Page, record_fetch, record_grid_page, record_session,
};
use crate::store::{Connection, open_migrated};

/// Un dossier temporaire garni, à garder en vie pendant le test.
pub struct Sample {
    _dir: tempfile::TempDir,
    pub app: App,
    pub now: DateTime<Utc>,
}

/// Secret de connexion des tests.
pub const TOKEN: &str = "jeton-de-test-0123456789abcdef";
/// Cookie d'une session ouverte.
pub const SESSION: &str = "edge_session=jeton-de-test-0123456789abcdef";
/// Hôte des requêtes de test, et l'origine d'un formulaire envoyé par le site lui-même.
pub const HOST: &str = "edge.test:8737";
pub const ORIGIN: &str = "http://edge.test:8737";

impl Sample {
    /// Une requête `GET`, connecté.
    pub fn get(&self, target: &str) -> Response {
        let request = Request {
            cookies: Some(SESSION),
            ..request(Method::Get, target, b"")
        };
        handle(&self.app, &request, self.now)
    }

    /// Le HTML d'une page, entités décodées : les tests cherchent `l'instant` ou `/paris/1`, pas `l&#x27;instant`.
    pub fn html(&self, target: &str) -> String {
        unescape(&String::from_utf8(self.get(target).body).unwrap())
    }

    /// Un formulaire envoyé par le site lui-même, connecté.
    pub fn post(&self, target: &str, body: &str) -> Response {
        let request = Request {
            cookies: Some(SESSION),
            ..request(Method::Post, target, body.as_bytes())
        };
        handle(&self.app, &request, self.now)
    }

    /// Une requête quelconque.
    pub fn send(&self, request: &Request<'_>) -> Response {
        handle(&self.app, request, self.now)
    }
}

/// Décode les entités qu'écrit l'échappement de `minijinja`.
pub fn unescape(html: &str) -> String {
    html.replace("&#x27;", "'")
        .replace("&#x2f;", "/")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Une requête sans cookie, venue du site (`Origin` et `Host` concordants).
pub fn request<'a>(method: Method, target: &'a str, body: &'a [u8]) -> Request<'a> {
    Request {
        method,
        target,
        cookies: None,
        origin: Some(ORIGIN),
        host: Some(HOST),
        body,
    }
}

/// Une application sur une base migrée vide.
pub fn empty() -> Sample {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    open_migrated(&paths.db_file).unwrap();
    Sample {
        app: app(paths),
        _dir: dir,
        now: Utc::now(),
    }
}

fn app(paths: Paths) -> App {
    App {
        paths,
        config: Config::default(),
        aliases: Aliases::bundled().unwrap(),
        token: ServeToken::new(TOKEN).unwrap(),
    }
}

/// Relevé d'il y a une heure : Pinnacle donne 2,00 à la victoire à domicile et Betclic 2,30, pour un match dans deux jours.
/// La recommandation correspondante (n° 1, mise suggérée) est déjà écrite dans le journal, ainsi qu'une séance Loto Foot de
/// deux tickets sur un LF7.
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
        id: "match-web".to_string(),
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
        excluded_bookmakers: Vec::new(),
        kelly_fraction: 1.0,
        min_stake: Euros::new(1.0).unwrap(),
        bankroll: Euros::new(1000.0).unwrap(),
        code_version: "test".to_string(),
    };
    recommend::run(&mut conn, &settings, now).unwrap();
    lotofoot_session(&mut conn, now);
    Sample {
        app: app(paths),
        _dir: dir,
        now,
    }
}

/// Une grille LF7 relevée il y a une heure et une séance de deux tickets dessus, pas encore déclarés joués.
fn lotofoot_session(conn: &mut Connection, now: DateTime<Utc>) {
    let fetched = now - Duration::hours(1);
    let fetch_id = record_fetch(conn, fetched, Page::Grid(Game::Lf7), 200, None).unwrap();
    let page = GridPage {
        open: Some(OpenGrid {
            game: Game::Lf7,
            accepting: true,
            number: 133,
            closes_at: now + Duration::days(1),
            jackpot: None,
            matches: (1..=7)
                .map(|i| (format!("H{i}"), format!("A{i}")))
                .collect(),
            split: Some(vec![[50, 30, 20]; 7]),
            split_at: None,
        }),
        history: vec![],
    };
    record_grid_page(conn, fetch_id, Game::Lf7, &page).unwrap();
    let tickets = [
        Ticket::parse("1N21N21").unwrap(),
        Ticket::parse("2N21N21").unwrap(),
    ];
    let recos: Vec<NewReco<'_>> = tickets
        .iter()
        .enumerate()
        .map(|(i, ticket)| NewReco {
            game: Game::Lf7,
            number: 133,
            ticket,
            position: i + 1,
            ev_base: 1.8,
            ev_pessimistic: 1.4,
            paid_probability: 0.02,
            stakes_estimate: 200_000.0,
            split_fetch_id: fetch_id,
            inputs: "{}",
        })
        .collect();
    let session = NewSession {
        created_at: fetched,
        model_version: "test",
        params: "{}",
        notes: "{}",
    };
    record_session(conn, &session, &recos).unwrap();
}
