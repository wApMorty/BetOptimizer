//! Onglet « Opportunités » (`/opportunites`) : les prix du dernier relevé qui dépassent la cote juste de Pinnacle, en
//! cartes, puis le journal récent.
//!
//! Une carte dit tout ce qu'il faut pour décider : le match et le moment, l'opérateur et l'issue, le prix contre la cote
//! juste, l'avantage, la cote en dessous de laquelle on ne parie plus, la mise suggérée. Une seule action : « J'ai parié ».

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::live;
use crate::betting::kelly::to_units;
use crate::betting::live::Opportunity;
use crate::betting::recommend::RECOMMEND_MIN_EDGE;
use crate::error::Result;
use crate::format;
use crate::report::recommend_report::league_name;
use crate::store::journal::{self, StoredBet, StoredRecommendation, Tier};
use crate::store::{self};
use crate::web::App;
use crate::web::form::Form;
use crate::web::view::{Flash, Layout, euros, fixture, gauge_width, render, signed_pct, when};

const TEMPLATE: &str = include_str!("../templates/opportunities.html");

/// Avantage au bout de la jauge.
const METER_MAX: f64 = 0.08;
/// Cartes affichées au plus.
const MAX_CARDS: usize = 24;
/// Lignes du journal affichées.
const JOURNAL_ROWS: usize = 10;

/// Une carte d'opportunité.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Card {
    fixture: String,
    meta: String,
    odds: String,
    fair_odds: String,
    edge: String,
    notified: bool,
    minimal_odds: String,
    /// Largeur de la jauge et position du seuil, en pour cent.
    meter: String,
    threshold_at: String,
    bookmaker: String,
    outcome: String,
    /// Pastille du journal (`au journal n° 3`…) et sa tonalité.
    journal: String,
    journal_tone: &'static str,
    /// `Mise suggérée 9,1 u (91,4 €)`, `Mise sous le minimum` ou `Observation`.
    stake: String,
    stake_strong: bool,
    /// Recommandation à suivre avec « J'ai parié », si elle n'a pas déjà de pari.
    follow: Option<i64>,
    followed: bool,
}

/// Une ligne du journal récent.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct JournalRow {
    id: i64,
    notified: bool,
    fixture: String,
    line: String,
    edge: String,
    followed: bool,
    kickoff: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct OpportunitiesPage {
    #[serde(flatten)]
    layout: Layout,
    all: bool,
    threshold: String,
    total: usize,
    cards: Vec<Card>,
    journal: Vec<JournalRow>,
}

/// La page ; `?toutes` montre aussi les prix sous le seuil.
pub fn page(app: &App, query: &Form, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let curve = super::bankroll(app, &conn)?;
    let prices = live(app, &conn, &curve, now)?;
    let mut recommendations = journal::recommendations(&conn)?;
    recommendations.reverse();
    let bets = journal::bets(&conn)?;
    let all = query.has("toutes");

    let mut shown: Vec<(&Opportunity, bool)> = prices.notified.iter().map(|o| (o, true)).collect();
    if all {
        shown.extend(prices.below.iter().map(|o| (o, false)));
    }
    shown.sort_by(|a, b| b.0.selection.edge.total_cmp(&a.0.selection.edge));
    shown.truncate(MAX_CARDS);
    let cards = shown
        .iter()
        .map(|(found, notified)| {
            card(
                found,
                *notified,
                &recommendations,
                &bets,
                curve.available,
                now,
            )
        })
        .collect();
    let journal = recommendations
        .iter()
        .take(JOURNAL_ROWS)
        .map(|found| journal_row(found, &bets))
        .collect();

    render(
        "opportunities.html",
        TEMPLATE,
        &OpportunitiesPage {
            layout: Layout::new("opportunities", now, flash),
            all,
            threshold: format::decimal(RECOMMEND_MIN_EDGE * 100.0, 0),
            total: prices.notified.len() + prices.below.len(),
            cards,
            journal,
        },
    )
}

fn is_followed(id: i64, bets: &[StoredBet]) -> bool {
    bets.iter().any(|b| b.bet.recommendation_id == Some(id))
}

fn card(
    found: &Opportunity,
    notified: bool,
    recommendations: &[StoredRecommendation],
    bets: &[StoredBet],
    available: f64,
    now: DateTime<Utc>,
) -> Card {
    let s = &found.selection;
    // La recommandation du journal qui correspond à ce prix : c'est elle qu'on suit avec « J'ai parié ».
    let recorded = recommendations
        .iter()
        .find(|r| r.record.event_id == found.event_id && r.record.tier == Tier::Recommended);
    let followed = recorded.is_some_and(|r| is_followed(r.id, bets));
    let (journal, journal_tone) = match (recorded, notified) {
        (Some(r), _) => (format!("au journal n° {}", r.id), "t-pos"),
        (None, true) => ("pas encore au journal".to_string(), ""),
        (None, false) => ("journal fantôme".to_string(), ""),
    };
    let (stake, stake_strong) = match found.stake {
        Some(stake) => {
            // L'unité (1 % de la bankroll) se calcule avec la bankroll du jour de la recommandation, si elle existe.
            let bankroll = recorded.map_or(available, |r| r.record.bankroll.get());
            (
                format!(
                    "Mise suggérée {} ({})",
                    format::units(to_units(stake.get() / bankroll)),
                    euros(stake.get(), 1)
                ),
                true,
            )
        }
        None if notified => ("Mise sous le minimum".to_string(), false),
        None => ("Observation".to_string(), false),
    };
    Card {
        fixture: fixture(&found.home_team, &found.away_team),
        meta: format!(
            "{} · {}",
            league_name(&found.sport_key),
            when(found.kickoff, now)
        ),
        odds: format::decimal(s.odds.get(), 2),
        fair_odds: format::decimal(1.0 / s.probability.get(), 2),
        edge: signed_pct(s.edge, 1),
        notified,
        minimal_odds: format::decimal(found.minimal_odds, 2),
        meter: gauge_width(s.edge / METER_MAX),
        threshold_at: gauge_width(RECOMMEND_MIN_EDGE / METER_MAX),
        bookmaker: found.bookmaker.clone(),
        outcome: format::outcome_label(s.outcome).to_string(),
        journal,
        journal_tone,
        stake,
        stake_strong,
        follow: recorded.filter(|_| !followed).map(|r| r.id),
        followed,
    }
}

fn journal_row(found: &StoredRecommendation, bets: &[StoredBet]) -> JournalRow {
    let r = &found.record;
    JournalRow {
        id: found.id,
        notified: r.tier == Tier::Recommended,
        fixture: fixture(&r.home_team, &r.away_team),
        line: format!(
            "{} · {} à {}",
            r.bookmaker,
            format::outcome_label(r.outcome),
            format::decimal(r.odds.get(), 2)
        ),
        edge: signed_pct(r.edge, 1),
        followed: is_followed(found.id, bets),
        kickoff: format::kickoff_label(r.kickoff),
    }
}

#[cfg(test)]
mod tests {
    use crate::web::testing::sample;

    #[test]
    fn a_notified_price_gets_a_card_with_its_stake_and_a_follow_link() {
        let html = sample().html("/opportunites");
        assert_eq!(html.matches("class=\"card opp").count(), 1, "{html}");
        assert!(html.contains("au journal n° 1"));
        assert!(html.contains("cote juste 2,04"), "{html}");
        assert!(html.contains("href=\"/paris/suivre/1\""));
        assert!(html.contains("Mise suggérée"));
        // Le journal récent liste la recommandation, notifiée.
        assert!(html.contains("n° 1"));
    }
}
