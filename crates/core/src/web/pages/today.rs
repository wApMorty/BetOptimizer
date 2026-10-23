//! Onglet « Aujourd'hui » (`/`) : le verdict du dernier relevé, bankroll, CLV et crédits, la fraîcheur des relevés et les
//! paris ouverts.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{Feed, feeds, live};
use crate::betting::clv;
use crate::betting::recommend::RECOMMEND_MIN_EDGE;
use crate::error::Result;
use crate::format;
use crate::report::daily;
use crate::report::recommend_report::league_name;
use crate::store::{self, archive, journal};
use crate::web::App;
use crate::web::charts;
use crate::web::view::{
    Flash, Layout, euros, fixture, gauge_width, render, signed_euros, signed_pct, when,
};

const TEMPLATE: &str = include_str!("../templates/today.html");

/// Clôtures à réunir avant que le CLV réel dise quelque chose.
const CLOSINGS_NEEDED: usize = 30;

/// Le meilleur prix du relevé.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Best {
    edge: String,
    line: String,
    when: String,
}

/// Une tuile : intitulé, chiffre, couleur du chiffre, jauge éventuelle, note.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Tile {
    label: &'static str,
    figure: String,
    color: &'static str,
    /// Largeur de la jauge en pour cent, s'il y en a une.
    gauge: Option<String>,
    /// Tonalité de la jauge (`t-warn`…).
    gauge_tone: &'static str,
    note: String,
}

/// Un pari ouvert.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct OpenBet {
    fixture: String,
    meta: String,
    stake: String,
    side: String,
}

/// Le dernier rapport quotidien.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct LatestReport {
    name: String,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct TodayPage {
    #[serde(flatten)]
    layout: Layout,
    title: String,
    subtitle: String,
    best: Option<Best>,
    any: bool,
    threshold: String,
    bankroll: Tile,
    spark: Option<String>,
    clv: Tile,
    credits: Tile,
    feeds: Vec<Feed>,
    open_bets: Vec<OpenBet>,
    latest: Option<LatestReport>,
}

/// La page.
pub fn page(app: &App, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let curve = super::bankroll(app, &conn)?;
    let prices = live(app, &conn, &curve, now)?;

    let title = match prices.notified.len() {
        0 => "Rien à parier pour l'instant".to_string(),
        1 => "1 opportunité".to_string(),
        n => format!("{n} opportunités"),
    };
    let date = format::long_date(now);
    let subtitle = if prices.events == 0 {
        format!("{date} · aucun relevé récent : le prochain arrive à 14 h")
    } else if prices.notified.is_empty() {
        format!(
            "{date} · {} matchs examinés, aucun prix n'atteint le seuil (c'est le cas la plupart des jours)",
            prices.events
        )
    } else {
        format!(
            "{date} · {} matchs examinés · {} autre(s) prix positif(s) sous le seuil",
            prices.events,
            prices.below.len()
        )
    };
    let best = prices
        .notified
        .iter()
        .max_by(|a, b| a.selection.edge.total_cmp(&b.selection.edge))
        .map(|top| Best {
            edge: signed_pct(top.selection.edge, 1),
            line: format!(
                "{} · {} · {} à {}",
                fixture(&top.home_team, &top.away_team),
                top.bookmaker,
                format::outcome_label(top.selection.outcome),
                format::decimal(top.selection.odds.get(), 2)
            ),
            when: when(top.kickoff, now),
        });

    let mut series = vec![curve.initial];
    series.extend(curve.points.iter().map(|p| p.bankroll));
    let tone = match curve.settled_profit {
        p if p > 0.0 => "pos",
        p if p < 0.0 => "neg",
        _ => "acc",
    };
    let spark = (series.len() > 1).then(|| charts::sparkline(&series, tone));
    let bankroll = Tile {
        label: "Bankroll disponible",
        figure: euros(curve.available, 0),
        color: "",
        gauge: None,
        gauge_tone: "",
        note: format!("gains réglés {}", signed_euros(curve.settled_profit)),
    };

    let report = clv::report(&conn, &app.aliases, app.config.margin_method)?;
    let clv = clv_tile(&report);
    let credits = credits_tile(
        archive::last_known_usage(&conn)?,
        app.config.monthly_credit_cap,
    );

    let mut open: Vec<_> = journal::bets(&conn)?
        .into_iter()
        .filter(|b| b.result.is_none())
        .collect();
    open.sort_by_key(|b| b.bet.kickoff);
    let open_bets = open
        .iter()
        .take(3)
        .map(|stored| {
            let b = &stored.bet;
            OpenBet {
                fixture: fixture(&b.home_team, &b.away_team),
                meta: format!("{} · {}", league_name(&b.sport_key), when(b.kickoff, now)),
                stake: euros(b.stake.get(), 1),
                side: format!(
                    "{} à {}",
                    format::outcome_label(b.outcome),
                    format::decimal(b.odds.get(), 2)
                ),
            }
        })
        .collect();
    let latest = daily::list(&app.paths.daily_reports_dir)?
        .into_iter()
        .next()
        .map(|report| LatestReport {
            label: format::long_date(report.at),
            name: report.name,
        });

    render(
        "today.html",
        TEMPLATE,
        &TodayPage {
            layout: Layout::new("today", now, flash),
            title,
            subtitle,
            any: best.is_some(),
            best,
            threshold: format::decimal(RECOMMEND_MIN_EDGE * 100.0, 0),
            bankroll,
            spark,
            clv,
            credits,
            feeds: feeds(&conn, now)?,
            open_bets,
            latest,
        },
    )
}

/// Le CLV réel du groupe qui a le plus de clôtures parmi les deux premiers (recommandations suivies, hors modèle).
fn clv_tile(report: &clv::ClvReport) -> Tile {
    let group = report
        .groups
        .iter()
        .take(2)
        .max_by_key(|g| g.with_closing)
        .filter(|g| g.with_closing > 0);
    let closings = group.map_or(0, |g| g.with_closing);
    let ci = group.and_then(|g| g.clv);
    let (figure, color) = match ci {
        Some(ci) if ci.excludes_zero() => (
            signed_pct(ci.mean, 2),
            if ci.mean > 0.0 { "pos" } else { "neg" },
        ),
        Some(ci) => (signed_pct(ci.mean, 2), ""),
        None => ("—".to_string(), ""),
    };
    let note = if closings >= CLOSINGS_NEEDED {
        match ci {
            Some(ci) if ci.excludes_zero() => format!("{closings} clôtures : différence nette"),
            _ => format!("{closings} clôtures : pas distinct de zéro"),
        }
    } else {
        format!("{closings} / {CLOSINGS_NEEDED} clôtures : trop peu")
    };
    Tile {
        label: "CLV réel",
        figure,
        color,
        gauge: Some(gauge_width(closings as f64 / CLOSINGS_NEEDED as f64)),
        gauge_tone: "",
        note,
    }
}

/// Les crédits de l'API de cotes consommés dans la période, contre le plafond.
fn credits_tile(used: Option<u32>, cap: u32) -> Tile {
    match used {
        Some(used) => Tile {
            label: "Crédits API ce mois",
            figure: format!("{used} / {cap}"),
            color: "",
            gauge: Some(gauge_width(f64::from(used) / f64::from(cap.max(1)))),
            gauge_tone: if used * 10 >= cap * 9 {
                "t-neg"
            } else if used * 10 >= cap * 7 {
                "t-warn"
            } else {
                ""
            },
            note: format!("{} avant le plafond", cap.saturating_sub(used)),
        },
        None => Tile {
            label: "Crédits API ce mois",
            figure: "—".to_string(),
            color: "",
            gauge: None,
            gauge_tone: "",
            note: "pas encore lus : le premier relevé les lit".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::web::testing::{empty, sample};

    #[test]
    fn the_verdict_counts_the_opportunities_or_says_there_is_nothing() {
        let html = sample().html("/");
        assert!(html.contains("1 opportunité"), "{html}");
        assert!(html.contains("Meilleur avantage"));
        assert!(html.contains("Falkirk – Kilmarnock · betclic_fr · domicile à 2,30"));
        assert!(html.contains("Football · il y a 1 h 00"));

        let html = empty().html("/");
        assert!(html.contains("Rien à parier pour l'instant"));
        assert!(html.contains("aucun relevé récent"));
        assert!(html.contains("Aucun pari ouvert"));
    }
}
