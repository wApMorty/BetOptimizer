//! Les pages de l'application, une par onglet. Chacune lit la base, prépare une vue (`#[derive(Serialize)]`) et la rend
//! avec son gabarit (`web/templates/`). Ce module garde les lectures partagées par plusieurs onglets.

pub mod bets;
pub mod bonus;
pub mod latency;
pub mod lotofoot;
pub mod opportunities;
pub mod paper;
pub mod performance;
pub mod reports;
pub mod today;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

use super::App;
use crate::betting::live::{self, Opportunity};
use crate::betting::overview::{self, BankrollCurve};
use crate::betting::recommend::RECOMMEND_MIN_EDGE;
use crate::domain::Euros;
use crate::error::{Error, Result};
use crate::format;
use crate::store::{archive, history, nba, tennis};

/// Les prix du dernier relevé, partagés en notifiés (avantage ≥ seuil, avec mise suggérée) et en dessous.
pub struct Live {
    /// Matchs à venir dans un relevé assez récent.
    pub events: usize,
    /// Prix qui atteignent le seuil de notification.
    pub notified: Vec<Opportunity>,
    /// Prix d'avantage positif sous le seuil (journal fantôme), sans mise.
    pub below: Vec<Opportunity>,
}

/// Lit le dernier relevé, comme `edge recommend` mais sans rien écrire, et dimensionne les mises sur la bankroll
/// disponible.
pub fn live(
    app: &App,
    conn: &Connection,
    curve: &BankrollCurve,
    now: DateTime<Utc>,
) -> Result<Live> {
    let config = &app.config;
    let scan = live::scan(
        conn,
        config.margin_method,
        0.0,
        &config.excluded_bookmakers,
        now,
    )?;
    let (mut notified, below): (Vec<Opportunity>, Vec<Opportunity>) = scan
        .opportunities
        .into_iter()
        .partition(|o| o.selection.edge >= RECOMMEND_MIN_EDGE);
    let available = euros(curve.available, "bankroll disponible")?;
    let min_stake = euros(config.min_stake, "min_stake")?;
    live::size_stakes(&mut notified, available, config.kelly_fraction, min_stake)?;
    Ok(Live {
        events: scan.events,
        notified,
        below,
    })
}

/// La courbe de bankroll des paris réels.
pub fn bankroll(app: &App, conn: &Connection) -> Result<BankrollCurve> {
    overview::bankroll_curve(
        conn,
        euros(app.config.initial_bankroll, "initial_bankroll")?,
    )
}

fn euros(value: f64, what: &str) -> Result<Euros> {
    Euros::new(value).ok_or_else(|| Error::Data(format!("{what} invalide : {value}")))
}

/// Fraîcheur d'un flux de l'archive (football, tennis, NBA).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Feed {
    /// Nom du sport.
    pub label: &'static str,
    /// `il y a 2 h 00`, ou `aucun relevé`.
    pub age: String,
    /// Classe de la pastille : vert sous 30 h, ambre sous 72 h, rouge au-delà, neutre sans relevé.
    pub tone: &'static str,
    /// `12 480 matchs d'historique`.
    pub archived: String,
}

/// Le dernier relevé réussi de chaque sport, et ce que l'archive contient.
pub fn feeds(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<Feed>> {
    let rows = [
        (
            "Football",
            "soccer_",
            history::count_games(conn)?,
            "matchs d'historique",
        ),
        (
            "Tennis",
            "tennis_",
            tennis::count_matches(conn)?,
            "matchs importés",
        ),
        (
            "NBA",
            "basketball_",
            nba::count_games(conn)?,
            "matchs terminés",
        ),
    ];
    let mut feeds = Vec::new();
    for (label, prefix, count, unit) in rows {
        let last = archive::last_odds_call(conn, prefix)?;
        let (age, tone) = match last {
            Some(at) => {
                let hours = (now - at).num_hours();
                let tone = if hours < 30 {
                    "t-pos"
                } else if hours < 72 {
                    "t-warn"
                } else {
                    "t-neg"
                };
                (format::age(at, now), tone)
            }
            None => ("aucun relevé".to_string(), ""),
        };
        feeds.push(Feed {
            label,
            age,
            tone,
            archived: format!("{count} {unit}"),
        });
    }
    Ok(feeds)
}
