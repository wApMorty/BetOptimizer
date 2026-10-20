//! Chiffres du rapport « abaque des seuils » : quel avantage minimal exiger d'un prix mou contre Pinnacle.

use rusqlite::Connection;
use serde::Serialize;

use super::svg::{threshold_clv_chart, threshold_growth_chart};
use crate::backtest::candidates::load_inputs;
use crate::backtest::curve::{self, CurvePoint, FREEZE_KELLY, FREEZE_KELLY_INDEX, SourceCurve};
use crate::backtest::study::{Criterion, KELLY_FRACTIONS, MIN_SELECTION_BETS};
use crate::betting::clv::{self, Split};
use crate::betting::recommend::RECOMMEND_MIN_EDGE;
use crate::error::Result;
use crate::ingest::aliases::Aliases;
use crate::market::margin::MarginMethod;
use crate::store::backtests;

/// Indices des seuils repris dans les tableaux (en dixièmes de point) : 0 ; 0,2 ; 0,5 ; 1 ; 1,5 ; 2 ; 3 ; 4 ; 5 ; 6 ; 8 %.
const TABLE_STEPS: [usize; 11] = [0, 2, 5, 10, 15, 20, 30, 40, 50, 60, 80];

/// Une ligne de tableau : un point de l'abaque et sa croissance traduite en multiplicateur de bankroll.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// Le point.
    pub point: CurvePoint,
    /// `exp(G)` en demi-Kelly (la fraction du gel) : par combien la bankroll serait multipliée sur toute la période si la
    /// clôture dit vrai.
    pub multiplier: f64,
    /// Même chose, ramenée à une saison : `exp(G / saisons)`.
    pub per_season: f64,
}

/// Seuil gelé d'une source au moment du rapport.
#[derive(Debug, Clone, Serialize)]
pub struct Frozen {
    /// Avantage minimal gelé.
    pub threshold: f64,
    /// Critère qui l'a choisi.
    pub criterion: Criterion,
}

/// Une source : son abaque, ses graphiques, ses lignes de tableau.
#[derive(Debug, Clone, Serialize)]
pub struct SourceView {
    /// L'abaque complète.
    pub curve: SourceCurve,
    /// Points repris dans le tableau.
    pub rows: Vec<Row>,
    /// Le point de plus forte croissance, s'il existe.
    pub best: Option<Row>,
    /// Graphique de la croissance attendue (SVG), vide sans pari.
    pub growth_chart: String,
    /// Graphique du CLV moyen (SVG), vide sans pari.
    pub clv_chart: String,
    /// Dernier seuil gelé de la source.
    pub frozen: Option<Frozen>,
}

/// Tout ce que le rapport affiche.
#[derive(Debug, Clone, Serialize)]
pub struct ThresholdsReport {
    /// Date de génération.
    pub generated_on: String,
    /// Méthode de retrait de marge de la configuration.
    pub method: &'static str,
    /// Configurations d'une session de l'abaque.
    pub configs_tried: usize,
    /// Sessions de réglage déjà enregistrées.
    pub sessions: u32,
    /// Configurations essayées, toutes sessions enregistrées confondues.
    pub configs_all_sessions: usize,
    /// Paris minimum pour qu'un seuil soit retenu.
    pub min_selection_bets: usize,
    /// Fractions de Kelly tracées.
    pub kelly_fractions: Vec<f64>,
    /// Fraction de Kelly du critère de gel.
    pub freeze_kelly: f64,
    /// Seuil des recommandations notifiées en production.
    pub production_threshold: f64,
    /// Une vue par bookmaker de l'historique.
    pub sources: Vec<SourceView>,
    /// CLV réel par opérateur français (journal des recommandations).
    pub live_by_bookmaker: Vec<Split>,
    /// CLV réel par issue (journal des recommandations).
    pub live_by_outcome: Vec<Split>,
}

fn row(point: &CurvePoint, seasons: usize) -> Row {
    let growth = point.growth[FREEZE_KELLY_INDEX];
    Row {
        point: point.clone(),
        multiplier: growth.exp(),
        per_season: (growth / seasons.max(1) as f64).exp(),
    }
}

fn view(conn: &Connection, curve: SourceCurve) -> Result<SourceView> {
    let rows = TABLE_STEPS
        .iter()
        .filter_map(|k| curve.points.get(*k))
        .map(|p| row(p, curve.seasons))
        .collect();
    let best = curve.best_point().map(|p| row(p, curve.seasons));
    let has_bets = curve.points.first().is_some_and(|p| p.bets > 0);
    let (growth_chart, clv_chart) = if has_bets {
        (
            threshold_growth_chart(
                &curve.points,
                &KELLY_FRACTIONS,
                curve.best,
                &format!("{} : croissance attendue", curve.label),
            )?,
            threshold_clv_chart(&curve.points, &format!("{} : CLV moyen", curve.label))?,
        )
    } else {
        (String::new(), String::new())
    };
    let frozen = backtests::latest_selection(conn, &curve.label)?.map(|f| Frozen {
        threshold: f.threshold,
        criterion: f.criterion,
    });
    Ok(SourceView {
        curve,
        rows,
        best,
        growth_chart,
        clv_chart,
        frozen,
    })
}

/// Construit le rapport : abaques de l'entraînement et de la validation, et CLV réel du journal.
pub fn build(
    conn: &Connection,
    aliases: &Aliases,
    method: MarginMethod,
    generated_on: &str,
) -> Result<ThresholdsReport> {
    let inputs = load_inputs(conn)?;
    let sources = curve::curves(&inputs, method)?
        .into_iter()
        .map(|c| view(conn, c))
        .collect::<Result<Vec<_>>>()?;
    let live = clv::report(conn, aliases, method)?;
    Ok(ThresholdsReport {
        generated_on: generated_on.to_string(),
        method: method.name(),
        configs_tried: curve::configs_tried(),
        sessions: backtests::selection_session_count(conn)?,
        configs_all_sessions: backtests::configs_tried_all_sessions(conn)?,
        min_selection_bets: MIN_SELECTION_BETS,
        kelly_fractions: KELLY_FRACTIONS.to_vec(),
        freeze_kelly: FREEZE_KELLY,
        production_threshold: RECOMMEND_MIN_EDGE,
        sources,
        live_by_bookmaker: live.by_bookmaker,
        live_by_outcome: live.by_outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::curve::{MarginSpread, OutcomeCell, OutcomeSplit};
    use crate::eval::bootstrap::MeanCi;
    use crate::report::render::render_thresholds_report;

    fn point(k: u32, bets: usize, growth: f64) -> CurvePoint {
        CurvePoint {
            threshold: f64::from(k) * 0.001,
            bets,
            clv: Some(MeanCi {
                mean: 0.004,
                low: -0.001,
                high: 0.009,
            }),
            flat_roi: Some(0.01),
            growth: vec![growth, growth * 0.75, growth * 0.44],
            growth_ci: Some((growth - 0.1, growth + 0.1)),
        }
    }

    fn sample() -> ThresholdsReport {
        let points: Vec<CurvePoint> = (0..=80)
            .map(|k| point(k, 900 - 10 * k as usize, 0.30 - 0.003 * f64::from(k)))
            .collect();
        let cell = OutcomeCell {
            bets: 10,
            clv: Some(0.002),
        };
        let curve = SourceCurve {
            label: "soft:B365".into(),
            batches: 400,
            seasons: 9,
            best: Some(0.005),
            by_outcome: vec![OutcomeSplit {
                threshold: 0.0,
                home: cell,
                draw: cell,
                away: cell,
            }],
            margin_spread: Some(MarginSpread {
                compared: 1800,
                median: 0.0021,
                p90: 0.0084,
            }),
            points,
        };
        let rows = TABLE_STEPS
            .iter()
            .map(|k| row(&curve.points[*k], 9))
            .collect();
        ThresholdsReport {
            generated_on: "2026-10-20".into(),
            method: "power",
            configs_tried: 648,
            sessions: 1,
            configs_all_sessions: 54,
            min_selection_bets: 50,
            kelly_fractions: KELLY_FRACTIONS.to_vec(),
            freeze_kelly: 0.5,
            production_threshold: 0.03,
            sources: vec![SourceView {
                best: curve.best_point().map(|p| row(p, 9)),
                rows,
                growth_chart: "<svg id=\"g\"></svg>".into(),
                clv_chart: "<svg id=\"c\"></svg>".into(),
                frozen: None,
                curve,
            }],
            live_by_bookmaker: vec![Split {
                key: "winamax_fr".into(),
                n: 2,
                with_closing: 1,
                mean_edge: 0.007,
                clv: None,
            }],
            live_by_outcome: Vec::new(),
        }
    }

    #[test]
    fn the_report_shows_the_curve_the_chosen_threshold_and_the_session_count() {
        let html = render_thresholds_report(&sample()).unwrap();

        assert!(html.contains("soft:B365"));
        assert!(html.contains("<svg id=\"g\">"), "graphique inséré tel quel");
        assert!(html.contains("0,5 %"), "seuil retenu au format français");
        assert!(html.contains("648 configurations"));
        assert!(html.contains("winamax_fr"));
        assert!(html.contains("Limites"));
        assert!(!html.contains("netbet"), "NetBet n'apparaît jamais");
    }

    #[test]
    fn the_multiplier_is_the_exponential_of_the_total_growth() {
        // Demi-Kelly : `point` y met 0,75 × 0,3 = 0,225.
        let r = row(&point(0, 100, 0.3), 3);
        assert!((r.multiplier - 0.225_f64.exp()).abs() < 1e-12);
        assert!((r.per_season - 0.075_f64.exp()).abs() < 1e-12);
    }
}
