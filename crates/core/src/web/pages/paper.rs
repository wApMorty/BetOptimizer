//! Onglet « Paper trading » (`/paper`) : chaque recommandation notifiée et misée, suivie comme si elle avait été jouée à
//! la cote et à la mise annoncées.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::betting::paper::{self, PaperBet, PaperTrading, Status};
use crate::error::Result;
use crate::eval::bootstrap::MeanCi;
use crate::format::{decimal, kickoff_label, outcome_label, signed, units};
use crate::report::recommend_report::league_name;
use crate::store;
use crate::web::App;
use crate::web::charts;
use crate::web::view::{Flash, Layout, render};

const TEMPLATE: &str = include_str!("../templates/paper.html");

/// Sous ce nombre de clôtures, la CLV est signalée comme indicative (même seuil que `edge clv`).
const FEW_CLOSINGS: usize = 30;

/// Une ligne du tableau.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct PaperRow {
    id: i64,
    kickoff: String,
    fixture: String,
    league: String,
    bookmaker: String,
    outcome: String,
    odds: String,
    edge: String,
    stake: String,
    clv: String,
    result: String,
    /// Classe CSS du résultat (`won`, `lost`, `open`).
    css: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct PaperPage {
    #[serde(flatten)]
    layout: Layout,
    n: usize,
    settled: usize,
    pending: usize,
    upcoming: usize,
    profit: String,
    staked: String,
    roi: String,
    clv: String,
    clv_ci: String,
    with_closing: usize,
    flat_roi: String,
    flat_roi_ci: String,
    drawdown: String,
    few_closings: bool,
    chart: Option<String>,
    rows: Vec<PaperRow>,
}

/// La page.
pub fn page(app: &App, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let paper = paper::paper_trading(&conn, &app.aliases, app.config.margin_method, now)?;
    render(
        "paper.html",
        TEMPLATE,
        &view(&paper, Layout::new("paper", now, flash)),
    )
}

/// Pourcentage signé : `0,0123` → `+1,2 %`.
fn signed_percent(value: f64) -> String {
    format!("{} %", signed(value * 100.0, 1))
}

/// Moyenne et intervalle d'un `MeanCi` : (`+1,2 %`, `[−0,8 ; +3,1] %`), ou des tirets.
fn ci_parts(ci: Option<MeanCi>) -> (String, String) {
    ci.map_or(("—".into(), "—".into()), |c| {
        (
            signed_percent(c.mean),
            format!(
                "[{} ; {}] %",
                signed(c.low * 100.0, 1),
                signed(c.high * 100.0, 1)
            ),
        )
    })
}

fn view(paper: &PaperTrading, layout: Layout) -> PaperPage {
    let (clv, clv_ci) = ci_parts(paper.clv);
    let (flat_roi, flat_roi_ci) = ci_parts(paper.flat_roi);
    // Le gain cumulé part de zéro : le premier point est l'état avant le premier règlement.
    let chart = (!paper.curve.is_empty()).then(|| {
        let mut values = vec![0.0];
        values.extend(&paper.curve);
        charts::line(&values, &[], Some(0.0), " u")
    });
    PaperPage {
        layout,
        n: paper.bets.len(),
        settled: paper.count(Status::Won) + paper.count(Status::Lost),
        pending: paper.count(Status::Pending),
        upcoming: paper.count(Status::Upcoming),
        profit: format!("{} u", signed(paper.profit_units, 1)),
        staked: units(paper.staked_units),
        roi: paper.roi.map_or("—".into(), signed_percent),
        clv,
        clv_ci,
        with_closing: paper.with_closing,
        flat_roi,
        flat_roi_ci,
        drawdown: units(paper.max_drawdown_units),
        few_closings: paper.with_closing < FEW_CLOSINGS,
        chart,
        // Du plus récent au plus ancien : la dernière recommandation en haut.
        rows: paper.bets.iter().rev().map(row).collect(),
    }
}

fn row(bet: &PaperBet) -> PaperRow {
    let stored = &bet.recommendation;
    let r = &stored.record;
    let (result, css) = match (bet.status, bet.profit_units) {
        (Status::Won, Some(profit)) => (format!("gagné {} u", signed(profit, 1)), "won"),
        (Status::Lost, Some(profit)) => (format!("perdu {} u", signed(profit, 1)), "lost"),
        (Status::Pending, _) => ("résultat attendu".to_string(), "open"),
        // `Upcoming`, et par prudence un gagné ou perdu sans gain calculé (impossible : `paper_trading` le calcule toujours).
        _ => ("à venir".to_string(), "open"),
    };
    PaperRow {
        id: stored.id,
        kickoff: kickoff_label(r.kickoff),
        fixture: format!("{} – {}", r.home_team, r.away_team),
        league: league_name(&r.sport_key),
        bookmaker: r.bookmaker.clone(),
        outcome: outcome_label(r.outcome).to_string(),
        odds: decimal(r.odds.get(), 2),
        edge: signed_percent(r.edge),
        stake: units(bet.stake_units),
        clv: bet.clv.map_or("—".into(), signed_percent),
        result,
        css,
    }
}

#[cfg(test)]
mod tests {
    use crate::web::testing::sample;

    #[test]
    fn the_paper_trading_page_lists_staked_recommendations_only() {
        let html = sample().html("/paper");
        assert!(html.contains("n° 1"), "{html}");
        assert!(html.contains("à venir"));
        assert!(html.contains("Moins de 30 clôtures"));
        assert!(!html.contains("<polyline"), "rien de réglé, pas de courbe");
    }
}
