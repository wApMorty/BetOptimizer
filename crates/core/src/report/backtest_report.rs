//! Chiffres du rapport « backtest » : où une règle de sélection trouve de la valeur, et si cette valeur est réelle.

use rusqlite::Connection;
use serde::Serialize;

use crate::backtest::candidates::load_inputs;
use crate::backtest::study::{
    KELLY_FRACTIONS, MIN_SELECTION_BETS, PRICE_FACTORS, SourceStudy, THRESHOLDS,
    load_model_predictions, study_all,
};
use crate::error::Result;
use crate::market::margin::MarginMethod;
use crate::store::backtests;

/// Tout ce que le rapport affiche.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestReport {
    /// Date de génération.
    pub generated_on: String,
    /// Sessions de réglage des seuils.
    pub sessions: u32,
    /// Configurations essayées pendant la dernière session.
    pub configs_tried: usize,
    /// Seuils essayés.
    pub thresholds: Vec<f64>,
    /// Fractions de Kelly comparées.
    pub kelly_fractions: Vec<f64>,
    /// Décotes de prix testées.
    pub price_factors: Vec<f64>,
    /// Paris minimum pour geler un seuil.
    pub min_selection_bets: usize,
    /// Une étude par source ayant un seuil gelé.
    pub studies: Vec<SourceStudy>,
}

/// Construit le rapport à partir de la base et des seuils gelés.
pub fn build(
    conn: &Connection,
    method: MarginMethod,
    generated_on: &str,
) -> Result<BacktestReport> {
    let inputs = load_inputs(conn)?;
    let predictions = load_model_predictions(conn)?;
    let studies = study_all(conn, &inputs, &predictions, method)?;
    let configs_tried = studies
        .first()
        .map(|s| backtests::latest_selection(conn, &s.label))
        .transpose()?
        .flatten()
        .map_or(0, |frozen| frozen.configs_tried);
    Ok(BacktestReport {
        generated_on: generated_on.to_string(),
        sessions: backtests::selection_session_count(conn)?,
        configs_tried,
        thresholds: THRESHOLDS.to_vec(),
        kelly_fractions: KELLY_FRACTIONS.to_vec(),
        price_factors: PRICE_FACTORS.to_vec(),
        min_selection_bets: MIN_SELECTION_BETS,
        studies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::candidates::Period;
    use crate::backtest::metrics::{RiskReport, SelectionCalibration, Summary};
    use crate::backtest::study::{EvBucket, PeriodRow, SeasonRow, StakingRow, Verdict};
    use crate::eval::bootstrap::MeanCi;
    use crate::report::render::render_backtest_report;

    fn report(studies: Vec<SourceStudy>) -> BacktestReport {
        BacktestReport {
            generated_on: "2026-10-18".into(),
            sessions: 1,
            configs_tried: 54,
            thresholds: vec![0.0, 0.03],
            kelly_fractions: vec![1.0, 0.5, 0.25],
            price_factors: vec![1.0, 0.957, 0.91],
            min_selection_bets: 50,
            studies,
        }
    }

    fn rejected_study() -> SourceStudy {
        let ci = |mean: f64| MeanCi {
            mean,
            low: mean - 0.005,
            high: mean + 0.005,
        };
        SourceStudy {
            label: "model:test".into(),
            threshold: 0.0,
            frozen_clv: Some(-0.077),
            frozen_bets: 1648,
            periods: vec![
                PeriodRow {
                    period: Period::Validation,
                    bets: 1648,
                    clv: Some(ci(-0.077)),
                    roi: Some(ci(-0.099)),
                },
                PeriodRow {
                    period: Period::Test,
                    bets: 1962,
                    clv: Some(ci(-0.0757)),
                    roi: None,
                },
            ],
            by_season: vec![SeasonRow {
                season: "2025/26".into(),
                bets: 1659,
                clv: Some(-0.0761),
                roi: Some(-0.124),
            }],
            verdict: Verdict::Rejected,
            calibration: Some(SelectionCalibration {
                n: 1962,
                announced: 0.325,
                realized: 0.255,
                close_fair: Some(0.269),
                announced_edge: 0.166,
            }),
            staking: vec![StakingRow {
                kelly_fraction: 0.5,
                summary: Summary {
                    bets: 694,
                    staked: 4571.0,
                    profit: -981.0,
                    roi_on_stakes: -0.21,
                    final_bankroll: 19.0,
                    log_growth_per_batch: -0.1,
                    max_drawdown: 0.98,
                },
                risk_if_announced: None,
                risk_if_close_is_true: Some(RiskReport {
                    ruin_probability: 0.83,
                    median_final: 0.02,
                    p95_drawdown: 1.0,
                }),
            }],
            ev_buckets: vec![EvBucket {
                label: "≥ 10 %",
                n: 1050,
                announced: 0.27,
                realized: -0.148,
                clv: Some(-0.0852),
            }],
            price_sensitivity: vec![
                (1.0, 1962, Some(-0.0757), Some(-0.124)),
                (0.91, 1049, None, None),
            ],
        }
    }

    #[test]
    fn the_report_shows_the_verdict_the_numbers_and_the_limits() {
        let html = render_backtest_report(&report(vec![rejected_study()])).unwrap();

        assert!(html.contains("model:test"));
        assert!(html.contains("rejetée"), "le verdict doit apparaître");
        assert!(
            html.contains("-7,57 %"),
            "CLV du test au format français signé"
        );
        assert!(
            html.contains("54 configurations"),
            "nombre de configurations essayées"
        );
        assert!(html.contains("Limites"));
        // Pas de séparateur décimal anglais dans les pourcentages.
        assert!(!html.contains("-7.57"));
    }

    #[test]
    fn without_a_frozen_threshold_the_report_explains_what_to_run() {
        let html = render_backtest_report(&report(Vec::new())).unwrap();

        assert!(html.contains("edge backtest tune"));
        assert!(html.contains("Limites"));
    }
}
