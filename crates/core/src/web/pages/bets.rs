//! Onglet « Paris » (`/paris`) : tes paris réels, leur état, et leur règlement.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::Result;
use crate::format;
use crate::report::recommend_report::league_name;
use crate::store::journal::{self, Settlement, StoredBet};
use crate::store::{self};
use crate::web::App;
use crate::web::view::{Flash, Layout, euros, fixture, render, signed_euros, when};

const TEMPLATE: &str = include_str!("../templates/bets.html");

/// Une carte de pari.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct BetCard {
    id: i64,
    fixture: String,
    meta: String,
    bookmaker: String,
    side: String,
    stake: String,
    /// `recommandation n° 9`, ou `hors modèle`.
    origin: String,
    off_model: bool,
    /// Tonalité de la bande et de l'état (`t-acc` ouvert, `t-pos` gagné…).
    tone: &'static str,
    state: &'static str,
    open: bool,
    /// Gain net d'un pari gagné ou perdu (absent pour un remboursé).
    profit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BetsPage {
    #[serde(flatten)]
    layout: Layout,
    subtitle: String,
    bets: Vec<BetCard>,
}

/// La page.
pub fn page(app: &App, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let curve = super::bankroll(app, &conn)?;
    let mut bets = journal::bets(&conn)?;
    bets.reverse();
    let open = bets.iter().filter(|b| b.result.is_none()).count();
    let subtitle = format!(
        "{} pari(s) · {} ouvert(s) · bankroll disponible {}",
        bets.len(),
        open,
        euros(curve.available, 0)
    );
    render(
        "bets.html",
        TEMPLATE,
        &BetsPage {
            layout: Layout::new("bets", now, flash),
            subtitle,
            bets: bets.iter().map(|stored| card(stored, now)).collect(),
        },
    )
}

fn card(stored: &StoredBet, now: DateTime<Utc>) -> BetCard {
    let b = &stored.bet;
    let (tone, state) = match &stored.result {
        None => ("t-acc", "ouvert"),
        Some(r) => match r.settlement {
            Settlement::Won => ("t-pos", "gagné"),
            Settlement::Lost => ("t-neg", "perdu"),
            Settlement::Void => ("", "remboursé"),
        },
    };
    BetCard {
        id: stored.id,
        fixture: fixture(&b.home_team, &b.away_team),
        meta: format!("{} · {}", league_name(&b.sport_key), when(b.kickoff, now)),
        bookmaker: b.bookmaker.clone(),
        side: format!(
            "{} à {}",
            format::outcome_label(b.outcome),
            format::decimal(b.odds.get(), 2)
        ),
        stake: format!("mise {}", euros(b.stake.get(), 1)),
        origin: b.recommendation_id.map_or("hors modèle".to_string(), |id| {
            format!("recommandation n° {id}")
        }),
        off_model: b.recommendation_id.is_none(),
        tone,
        state,
        open: stored.result.is_none(),
        profit: stored
            .result
            .as_ref()
            .filter(|r| r.settlement != Settlement::Void)
            .map(|r| signed_euros(r.profit)),
    }
}
