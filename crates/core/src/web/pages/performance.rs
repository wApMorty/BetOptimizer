//! Onglet « Performance » (`/performance`) : bankroll, CLV réel, calibration. `?vue=clv` ou `?vue=calibration` choisit
//! la vue ; la bankroll s'affiche par défaut.

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;

use crate::betting::clv;
use crate::betting::overview::{self, BankrollCurve};
use crate::error::Result;
use crate::eval::bootstrap::MeanCi;
use crate::format;
use crate::store;
use crate::web::App;
use crate::web::charts;
use crate::web::form::Form;
use crate::web::view::{Flash, Layout, euros, render, sign_color, signed_euros, signed_pct};

const TEMPLATE: &str = include_str!("../templates/performance.html");

/// Tranches du diagramme de fiabilité : peu de recommandations réglées, donc peu de tranches.
const RELIABILITY_BINS: usize = 5;
/// Effectif minimal d'un groupe pour que son intervalle ne soit pas qu'indicatif.
const MIN_FOR_CONFIDENCE: usize = 30;
/// Étiquettes de dates sous la courbe de bankroll.
const DATE_LABELS: usize = 4;

/// Les tuiles et la courbe de la bankroll.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct BankrollView {
    available: String,
    initial: String,
    profit: String,
    profit_color: &'static str,
    open_stakes: String,
    drawdown: String,
    chart: Option<String>,
}

/// Un groupe de paris du CLV réel.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct ClvGroup {
    label: &'static str,
    detail: String,
    figure: String,
    color: &'static str,
    interval: String,
    tentative: bool,
}

/// Une tranche du diagramme de fiabilité.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Bin {
    range: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct PerformancePage {
    #[serde(flatten)]
    layout: Layout,
    view: &'static str,
    bankroll: Option<BankrollView>,
    groups: Vec<ClvGroup>,
    buckets: Option<String>,
    calibration: Option<String>,
    bins: Vec<Bin>,
}

/// La page.
pub fn page(app: &App, query: &Form, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let view = match query.get("vue") {
        "clv" => "clv",
        "calibration" => "calibration",
        _ => "bankroll",
    };
    let mut page = PerformancePage {
        layout: Layout::new("performance", now, flash),
        view,
        bankroll: None,
        groups: Vec::new(),
        buckets: None,
        calibration: None,
        bins: Vec::new(),
    };
    match view {
        "clv" => {
            let report = clv::report(&conn, &app.aliases, app.config.margin_method)?;
            page.groups = report.groups.iter().map(clv_group).collect();
            let bars: Vec<(String, Option<f64>)> = report
                .buckets
                .iter()
                .map(|b| (b.label.to_string(), b.mean_clv))
                .collect();
            page.buckets = bars
                .iter()
                .any(|(_, v)| v.is_some())
                .then(|| charts::bars(&bars));
        }
        "calibration" => {
            let bins = overview::reliability(&conn, &app.aliases, RELIABILITY_BINS)?;
            if !bins.is_empty() {
                let points: Vec<(f64, f64)> = bins
                    .iter()
                    .map(|b| (b.mean_probability, b.frequency))
                    .collect();
                page.calibration = Some(charts::reliability(&points));
            }
            page.bins = bins
                .iter()
                .map(|bin| Bin {
                    range: format!(
                        "{} – {} %",
                        format::decimal(bin.low * 100.0, 0),
                        format::decimal(bin.high * 100.0, 0)
                    ),
                    text: format!(
                        "{} réglées · annoncé {} % · observé {} %",
                        bin.n,
                        format::decimal(bin.mean_probability * 100.0, 1),
                        format::decimal(bin.frequency * 100.0, 1)
                    ),
                })
                .collect();
        }
        _ => page.bankroll = Some(bankroll_view(&super::bankroll(app, &conn)?)),
    }
    render("performance.html", TEMPLATE, &page)
}

fn bankroll_view(curve: &BankrollCurve) -> BankrollView {
    let chart = (!curve.points.is_empty()).then(|| {
        let values: Vec<f64> = curve.points.iter().map(|p| p.bankroll).collect();
        // Quelques dates réparties sur la courbe : la première, la dernière, et des rangs régulièrement espacés.
        let last = values.len() - 1;
        let mut labels: Vec<(usize, String)> = (0..DATE_LABELS)
            .map(|k| last * k / (DATE_LABELS - 1).max(1))
            .map(|rank| {
                let at = curve.points[rank].at.with_timezone(&Paris);
                (rank, at.format("%d/%m").to_string())
            })
            .collect();
        labels.dedup_by_key(|(rank, _)| *rank);
        charts::line(&values, &labels, Some(curve.initial), "")
    });
    BankrollView {
        available: euros(curve.available, 0),
        initial: euros(curve.initial, 0),
        profit: signed_euros(curve.settled_profit),
        profit_color: sign_color(curve.settled_profit),
        open_stakes: euros(curve.open_stakes, 0),
        drawdown: format!("{} %", format::decimal(curve.max_drawdown * 100.0, 1)),
        chart,
    }
}

fn interval(ci: Option<MeanCi>) -> String {
    match ci {
        Some(c) => format!(
            "[{} ; {}]",
            format::signed(c.low * 100.0, 1),
            format::signed(c.high * 100.0, 1)
        ),
        None => "pas encore de clôture".to_string(),
    }
}

fn clv_group(group: &clv::Group) -> ClvGroup {
    let (figure, color) = match group.clv {
        Some(c) if c.excludes_zero() => (signed_pct(c.mean, 2), sign_color(c.mean)),
        Some(c) => (signed_pct(c.mean, 2), ""),
        None => ("—".to_string(), ""),
    };
    ClvGroup {
        label: group.label,
        detail: format!(
            "{} élément(s) · clôture connue {} · réglés {}",
            group.n, group.with_closing, group.settled
        ),
        figure,
        color,
        interval: interval(group.clv),
        tentative: group.with_closing < MIN_FOR_CONFIDENCE,
    }
}

#[cfg(test)]
mod tests {
    use crate::web::testing::empty;

    #[test]
    fn each_view_says_when_there_is_nothing_to_draw_yet() {
        let sample = empty();
        let page = |target: &str| sample.html(target);
        assert!(page("/performance").contains("Aucun pari réglé"));
        assert!(page("/performance?vue=clv").contains("Aucune clôture pour l'instant"));
        assert!(page("/performance?vue=calibration").contains("Aucune recommandation réglée"));
        assert!(page("/performance?vue=autre").contains("class=\"on\">Bankroll"));
    }
}
