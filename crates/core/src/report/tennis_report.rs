//! Chiffres du rapport « tennis » : l'Elo par surface contre le marché sur le **test**.
//!
//! La configuration de l'Elo vient d'une session de réglage figée sur la validation (2022-2023) ; le test (à partir du
//! 2024-01-01) n'a servi à rien d'autre qu'à ce rapport. Elo et marché sont comparés sur exactement les mêmes matchs, avec
//! des intervalles de confiance par blocs de semaines. Le marché est la cote de Pinnacle de tennis-data, « la plus récente
//! avant le match » : quasiment la clôture (voir `docs/sources.md` §21).
//!
//! Orientation : tennis-data publie des cotes de **vainqueur** et de **perdant**. Les métriques sont calculées sur la
//! probabilité attribuée au vainqueur réel, ce qui est correct pour un log loss et un Brier ; pour la calibration on
//! oriente sur le **favori** de chaque prévision, sinon l'événement serait toujours « le vainqueur gagne ».

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;
use serde::Serialize;

use super::svg::reliability_chart;
use crate::domain::{Odds, Probability};
use crate::error::{Error, Result};
use crate::eval::bootstrap::{MeanCi, block_bootstrap_mean};
use crate::eval::metrics::{brier_binary, log_loss_binary};
use crate::ingest::tennis::{Surface, Tour, quality};
use crate::market::calibration::reliability;
use crate::market::margin::{MarginMethod, remove_margin_two_way};
use crate::models::elo::{EloConfig, Prediction, walk_forward};
use crate::models::elo_tuning::{TEST_START, VALIDATION_START, is_scored};
use crate::store::tennis::{self, StoredMatch};

const RESAMPLES: usize = 2000;
const SEED: u64 = 20_261_021;

/// Bookmaker de référence : Pinnacle.
const MARKET: &str = "PS";

/// Marge plausible d'un marché à deux issues (somme des probabilités implicites), comme dans le contrôle qualité.
const IMPLIED_SUM: (f64, f64) = (1.0, 1.30);

/// La configuration gelée, telle qu'affichée.
#[derive(Debug, Clone, Serialize)]
pub struct FrozenView {
    /// Numérateur de `K`.
    pub k_base: f64,
    /// Exposant de la décroissance de `K`.
    pub k_shape: f64,
    /// Poids du classement de surface.
    pub surface_weight: f64,
    /// Log loss de validation.
    pub validation_log_loss: f64,
    /// Matchs de validation.
    pub validation_matches: usize,
    /// Configurations essayées.
    pub configs_tried: usize,
    /// Sessions de réglage faites (à ce jour).
    pub sessions: u32,
}

/// Une ligne de couverture : combien de matchs, combien avec Pinnacle.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageRow {
    /// Circuit.
    pub tour: String,
    /// Année.
    pub season: u16,
    /// Matchs en base.
    pub matches: u32,
    /// Matchs avec une cote Pinnacle.
    pub with_pinnacle: u32,
}

/// Un modèle sur les matchs du test.
#[derive(Debug, Clone, Serialize)]
pub struct ModelRow {
    /// Nom.
    pub label: String,
    /// Matchs évalués.
    pub n: usize,
    /// Log loss moyen.
    pub log_loss: f64,
    /// Brier moyen.
    pub brier: f64,
}

/// Une tranche (surface, circuit, format) : Elo contre marché.
#[derive(Debug, Clone, Serialize)]
pub struct GroupRow {
    /// Étiquette.
    pub label: String,
    /// Matchs.
    pub n: usize,
    /// Log loss de l'Elo.
    pub elo_log_loss: f64,
    /// Log loss du marché.
    pub market_log_loss: f64,
    /// Écart Elo − marché avec son intervalle (positif = l'Elo est moins bon).
    pub delta: Option<MeanCi>,
}

/// Le test d'information : mélange de l'Elo et du marché.
#[derive(Debug, Clone, Serialize)]
pub struct InfoView {
    /// Poids de l'Elo choisi sur la validation (0 = le marché seul).
    pub weight: f64,
    /// Matchs de validation qui ont servi à le choisir.
    pub validation_matches: usize,
    /// Écart de log loss du mélange contre le marché seul, sur le test (négatif = le mélange fait mieux).
    pub delta_log_loss: Option<MeanCi>,
}

/// Tout ce que le rapport affiche.
#[derive(Debug, Clone, Serialize)]
pub struct TennisReport {
    /// Date de génération.
    pub generated_on: String,
    /// Première date du test.
    pub test_start: String,
    /// Configuration gelée.
    pub frozen: FrozenView,
    /// Couverture des données.
    pub coverage: Vec<CoverageRow>,
    /// Matchs du test : tous, prévus et comptés, comparables au marché.
    pub test_matches: usize,
    /// Matchs du test comptés (terminés, joueurs installés).
    pub test_scored: usize,
    /// Matchs du test comparés au marché (cote Pinnacle présente et plausible).
    pub test_with_market: usize,
    /// Matchs du test qui ont une cote Pinnacle mais aberrante : écartés de la comparaison.
    pub test_odds_rejected: usize,
    /// Elo et marché sur les mêmes matchs.
    pub models: Vec<ModelRow>,
    /// Écart de log loss Elo − marché.
    pub delta_log_loss: Option<MeanCi>,
    /// Écart de Brier Elo − marché.
    pub delta_brier: Option<MeanCi>,
    /// Par surface.
    pub by_surface: Vec<GroupRow>,
    /// Par circuit.
    pub by_tour: Vec<GroupRow>,
    /// Par nombre de sets maximal.
    pub by_best_of: Vec<GroupRow>,
    /// Test d'information.
    pub info: Option<InfoView>,
    /// Fiabilité de l'Elo (favori), SVG.
    pub elo_chart: String,
    /// Fiabilité du marché (favori), SVG.
    pub market_chart: String,
}

/// Un match du test, comparable au marché.
struct EvalRow {
    block: u32,
    tour: Tour,
    surface: Surface,
    best_of: u8,
    elo: f64,
    market: f64,
}

/// Les prévisions qui comptent, avec la probabilité du marché pour le vainqueur, dans la fenêtre `[from, to)`.
fn eval_rows(
    predictions: &[Prediction],
    matches: &HashMap<i64, &StoredMatch>,
    method: MarginMethod,
    from: NaiveDate,
    to: Option<NaiveDate>,
) -> Vec<EvalRow> {
    predictions
        .iter()
        .filter(|p| is_scored(p) && p.date >= from && to.is_none_or(|end| p.date < end))
        .filter_map(|p| {
            let stored = matches.get(&p.match_id)?;
            let (winner_odds, loser_odds) = *stored.odds.get(MARKET)?;
            // Marge hors de [1,00 ; 1,30] : cote aberrante, signalée par `edge tennis check`, jamais utilisée ni corrigée.
            let implied = 1.0 / winner_odds + 1.0 / loser_odds;
            if !(IMPLIED_SUM.0..=IMPLIED_SUM.1).contains(&implied) {
                return None;
            }
            let market =
                remove_margin_two_way(Odds::new(winner_odds)?, Odds::new(loser_odds)?, method)?;
            Some(EvalRow {
                // Un bloc = une semaine d'un circuit : les matchs d'un même tournoi ne sont pas indépendants.
                block: u32::try_from(p.date.num_days_from_ce() / 7).unwrap_or(0) * 2
                    + u32::from(p.tour == Tour::Wta),
                tour: p.tour,
                surface: p.surface,
                best_of: p.best_of,
                elo: p.p_winner,
                market: market.get(),
            })
        })
        .collect()
}

fn probability(value: f64) -> Probability {
    // Elo et marché sont strictement entre 0 et 1 ; la borne protège le logarithme de toute dérive numérique.
    Probability::new(value.clamp(1e-9, 1.0 - 1e-9)).unwrap_or_else(half)
}

fn half() -> Probability {
    Probability::new(0.5).expect("0,5 est une probabilité")
}

pub(crate) fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0usize), |(s, c), v| (s + v, c + 1));
    if count == 0 {
        f64::NAN
    } else {
        sum / count as f64
    }
}

pub(crate) fn loss(p: f64) -> f64 {
    log_loss_binary(probability(p), true)
}

fn squared(p: f64) -> f64 {
    brier_binary(probability(p), true)
}

fn delta(rows: &[&EvalRow], f: impl Fn(f64) -> f64) -> Option<MeanCi> {
    let values: Vec<(u32, f64)> = rows
        .iter()
        .map(|r| (r.block, f(r.elo) - f(r.market)))
        .collect();
    block_bootstrap_mean(&values, RESAMPLES, SEED)
}

fn group(label: String, rows: &[&EvalRow]) -> GroupRow {
    GroupRow {
        label,
        n: rows.len(),
        elo_log_loss: mean(rows.iter().map(|r| loss(r.elo))),
        market_log_loss: mean(rows.iter().map(|r| loss(r.market))),
        delta: delta(rows, loss),
    }
}

/// `logit(p) = ln(p / (1 − p))`.
pub(crate) fn logit(p: f64) -> f64 {
    let p = p.clamp(1e-9, 1.0 - 1e-9);
    (p / (1.0 - p)).ln()
}

/// Mélange dans l'espace logit : `σ((1 − w)·logit(marché) + w·logit(Elo))`.
pub(crate) fn blend(market: f64, elo: f64, weight: f64) -> f64 {
    1.0 / (1.0 + (-((1.0 - weight) * logit(market) + weight * logit(elo))).exp())
}

/// Favori d'une prévision : `(probabilité du favori, le favori a gagné)`, pour la calibration.
fn favourite(p_winner: f64) -> (f64, bool) {
    if p_winner >= 0.5 {
        (p_winner, true)
    } else {
        (1.0 - p_winner, false)
    }
}

/// Construit le rapport.
pub fn build(
    conn: &Connection,
    method: MarginMethod,
    generated_on: &str,
    current_year: u16,
) -> Result<TennisReport> {
    let matches = tennis::load_matches(conn)?;
    if matches.is_empty() {
        return Err(Error::Data(
            "aucun match de tennis en base : lance d'abord `edge tennis import`".into(),
        ));
    }
    let frozen = tennis::latest_tuning(conn)?.ok_or_else(|| {
        Error::Data("aucune configuration gelée : lance d'abord `edge tennis tune`".into())
    })?;
    let config = EloConfig {
        k_base: frozen.k_base,
        k_shape: frozen.k_shape,
        surface_weight: frozen.surface_weight,
    };
    let predictions = walk_forward(&matches, config);
    let by_id: HashMap<i64, &StoredMatch> = matches.iter().map(|m| (m.id, m)).collect();

    let test = eval_rows(&predictions, &by_id, method, TEST_START, None);
    let refs: Vec<&EvalRow> = test.iter().collect();
    let validation = eval_rows(
        &predictions,
        &by_id,
        method,
        VALIDATION_START,
        Some(TEST_START),
    );

    let models = vec![
        ModelRow {
            label: "Elo par surface (gelé)".into(),
            n: test.len(),
            log_loss: mean(test.iter().map(|r| loss(r.elo))),
            brier: mean(test.iter().map(|r| squared(r.elo))),
        },
        ModelRow {
            label: "Marché (Pinnacle sans marge)".into(),
            n: test.len(),
            log_loss: mean(test.iter().map(|r| loss(r.market))),
            brier: mean(test.iter().map(|r| squared(r.market))),
        },
    ];

    let by = |key: &dyn Fn(&EvalRow) -> String, labels: Vec<String>| -> Vec<GroupRow> {
        labels
            .into_iter()
            .filter_map(|label| {
                let rows: Vec<&EvalRow> =
                    refs.iter().copied().filter(|r| key(r) == label).collect();
                (!rows.is_empty()).then(|| group(label, &rows))
            })
            .collect()
    };
    let by_surface = by(
        &|r| r.surface.as_str().to_string(),
        Surface::ALL
            .iter()
            .map(|s| s.as_str().to_string())
            .collect(),
    );
    let by_tour = by(
        &|r| r.tour.as_str().to_uppercase(),
        Tour::ALL
            .iter()
            .map(|t| t.as_str().to_uppercase())
            .collect(),
    );
    let by_best_of = by(
        &|r| format!("au meilleur des {}", r.best_of),
        vec![
            "au meilleur des 3".to_string(),
            "au meilleur des 5".to_string(),
        ],
    );

    // Test d'information : le poids de l'Elo est choisi sur la validation, puis lu sur le test.
    let info = (!validation.is_empty()).then(|| {
        let weights: Vec<f64> = (0..=10).map(|i| f64::from(i) / 10.0).collect();
        let best = weights
            .iter()
            .copied()
            .map(|w| {
                (
                    w,
                    mean(validation.iter().map(|r| loss(blend(r.market, r.elo, w)))),
                )
            })
            .reduce(|best, candidate| {
                if candidate.1 < best.1 {
                    candidate
                } else {
                    best
                }
            })
            .map_or(0.0, |(w, _)| w);
        let values: Vec<(u32, f64)> = test
            .iter()
            .map(|r| (r.block, loss(blend(r.market, r.elo, best)) - loss(r.market)))
            .collect();
        InfoView {
            weight: best,
            validation_matches: validation.len(),
            delta_log_loss: block_bootstrap_mean(&values, RESAMPLES, SEED),
        }
    });

    let chart = |pick: &dyn Fn(&EvalRow) -> f64, title: &str| -> Result<String> {
        let pairs: Vec<(f64, bool)> = test.iter().map(|r| favourite(pick(r))).collect();
        reliability_chart(&reliability(&pairs, 0.1), title)
    };

    let qc = quality::run(conn, current_year)?;
    Ok(TennisReport {
        generated_on: generated_on.to_string(),
        test_start: TEST_START.format("%d/%m/%Y").to_string(),
        frozen: FrozenView {
            k_base: frozen.k_base,
            k_shape: frozen.k_shape,
            surface_weight: frozen.surface_weight,
            validation_log_loss: frozen.validation_log_loss,
            validation_matches: frozen.validation_matches,
            configs_tried: frozen.configs_tried,
            sessions: tennis::tuning_sessions(conn)?,
        },
        coverage: qc
            .seasons
            .iter()
            .map(|s| CoverageRow {
                tour: s.tour.as_str().to_uppercase(),
                season: s.season,
                matches: s.matches,
                with_pinnacle: s.with_pinnacle,
            })
            .collect(),
        test_matches: predictions.iter().filter(|p| p.date >= TEST_START).count(),
        test_scored: predictions
            .iter()
            .filter(|p| p.date >= TEST_START && is_scored(p))
            .count(),
        test_with_market: test.len(),
        test_odds_rejected: predictions
            .iter()
            .filter(|p| p.date >= TEST_START && is_scored(p))
            .filter(|p| {
                by_id.get(&p.match_id).is_some_and(|m| {
                    m.odds.get(MARKET).is_some_and(|(w, l)| {
                        !(IMPLIED_SUM.0..=IMPLIED_SUM.1).contains(&(1.0 / w + 1.0 / l))
                    })
                })
            })
            .count(),
        delta_log_loss: delta(&refs, loss),
        delta_brier: delta(&refs, squared),
        models,
        by_surface,
        by_tour,
        by_best_of,
        info,
        elo_chart: chart(
            &|r| r.elo,
            "Elo : le favori gagne-t-il aussi souvent qu'annoncé ?",
        )?,
        market_chart: chart(&|r| r.market, "Marché : même question")?,
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::ingest::tennis::{MatchOdds, ParsedMatch, Status};
    use crate::report::render::render_tennis_report;
    use crate::store::migrate;
    use crate::store::tennis::{FrozenElo, insert_tuning, upsert_match};

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    /// Un match d'un monde simple : « Fort » bat « Faible », Pinnacle l'avait à 1,25 contre 4,10 (marge 4,4 %).
    fn fort_beats_faible(date: NaiveDate) -> ParsedMatch {
        ParsedMatch {
            tour: Tour::Atp,
            season: u16::try_from(date.year()).unwrap(),
            tournament: "Monde".into(),
            location: None,
            date,
            series: None,
            court: None,
            surface: Surface::Hard,
            round: "Final".into(),
            best_of: 3,
            winner: "Fort".into(),
            loser: "Faible".into(),
            winner_rank: None,
            loser_rank: None,
            winner_points: None,
            loser_points: None,
            winner_sets: Some(2),
            loser_sets: Some(0),
            status: Status::Completed,
            odds: vec![MatchOdds {
                bookmaker: "PS",
                winner: Odds::new(1.25).unwrap(),
                loser: Odds::new(4.10).unwrap(),
            }],
        }
    }

    fn world() -> Connection {
        let conn = database();
        let mut day = NaiveDate::from_ymd_opt(2021, 6, 1).unwrap();
        while day < NaiveDate::from_ymd_opt(2025, 1, 1).unwrap() {
            upsert_match(&conn, &fort_beats_faible(day)).unwrap();
            day = day.succ_opt().unwrap();
        }
        conn
    }

    fn freeze(conn: &Connection) {
        insert_tuning(
            conn,
            &FrozenElo {
                created_at: Utc::now(),
                k_base: 250.0,
                k_shape: 0.4,
                surface_weight: 0.5,
                validation_log_loss: 0.30,
                validation_matches: 730,
                configs_tried: 45,
            },
        )
        .unwrap();
    }

    #[test]
    fn the_report_compares_the_frozen_elo_with_the_market_on_the_test_matches() {
        let conn = world();
        freeze(&conn);

        let report = build(&conn, MarginMethod::Power, "2026-10-18", 2025).unwrap();

        // Test : du 2024-01-01 au 2024-12-31, un match par jour.
        assert_eq!(report.test_matches, 366);
        assert_eq!(report.test_with_market, 366);
        assert_eq!(report.models.len(), 2);
        // Dans ce monde, Fort gagne toujours : l'Elo finit par prévoir mieux que le marché prudent à 77 %.
        assert!(report.models[0].log_loss < std::f64::consts::LN_2);
        assert!(report.delta_log_loss.is_some());
        assert_eq!(report.frozen.configs_tried, 45);
        assert_eq!(report.by_surface.len(), 1);
        assert_eq!(report.by_surface[0].label, "hard");
        assert!(report.elo_chart.contains("<svg"));
        let info = report.info.as_ref().unwrap();
        assert!(
            info.weight > 0.0,
            "l'Elo bat ce marché prudent : le poids optimal n'est pas nul"
        );
    }

    #[test]
    fn the_report_renders_in_french_with_the_verdict_and_the_limits() {
        let conn = world();
        freeze(&conn);
        let report = build(&conn, MarginMethod::Power, "2026-10-18", 2025).unwrap();

        let html = render_tennis_report(&report).unwrap();

        assert!(html.contains("Tennis : l'Elo contre le marché"));
        assert!(html.contains("45 configurations"));
        assert!(html.contains("Pas de CLV"));
        assert!(html.contains("<svg"));
    }

    #[test]
    fn without_matches_or_without_a_frozen_configuration_the_report_says_what_to_run() {
        let empty = database();
        assert!(
            build(&empty, MarginMethod::Power, "x", 2025)
                .unwrap_err()
                .to_string()
                .contains("edge tennis import")
        );

        let conn = world();
        assert!(
            build(&conn, MarginMethod::Power, "x", 2025)
                .unwrap_err()
                .to_string()
                .contains("edge tennis tune")
        );
    }

    #[test]
    fn the_blend_is_the_market_at_zero_and_the_elo_at_one() {
        assert!((blend(0.7, 0.4, 0.0) - 0.7).abs() < 1e-12);
        assert!((blend(0.7, 0.4, 1.0) - 0.4).abs() < 1e-12);
        let half = blend(0.7, 0.4, 0.5);
        assert!(half > 0.4 && half < 0.7);
    }

    #[test]
    fn the_favourite_is_the_side_above_one_half() {
        assert_eq!(favourite(0.8), (0.8, true));
        let (p, won) = favourite(0.3);
        assert!((p - 0.7).abs() < 1e-12);
        assert!(
            !won,
            "le favori à 70 % a perdu quand le vainqueur n'avait que 30 %"
        );
    }

    #[test]
    fn log_loss_of_a_certain_win_is_zero_and_of_a_coin_flip_is_ln_two() {
        assert!(loss(1.0 - 1e-12).abs() < 1e-6);
        assert!((loss(0.5) - std::f64::consts::LN_2).abs() < 1e-12);
        assert!((squared(0.5) - 0.25).abs() < 1e-12);
    }
}
