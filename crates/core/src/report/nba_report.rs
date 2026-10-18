//! Chiffres du rapport « NBA » : le modèle contre Pinnacle sur la victoire, le handicap et le total.
//!
//! Deux modes, annoncés en tête du rapport :
//! - **gelé** : la configuration vient d'une session de réglage sur la validation (2026-11-15 au 2027-02-01) ; seuls les
//!   matchs du test (à partir du 2027-02-01) sont évalués ;
//! - **exploratoire** : aucune configuration gelée, on évalue avec le point de départ sur toute l'archive. Ces chiffres
//!   décrivent, ils ne valident rien et ne doivent servir à aucun réglage.
//!
//! Le marché est le dernier relevé de Pinnacle antérieur au coup d'envoi (vers 18 h à New York : 1 à 4 h avant les matchs du soir).

use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;
use serde::Serialize;

use super::svg::reliability_chart;
use super::tennis_report::{blend, loss, mean};
use crate::domain::{Odds, Probability};
use crate::error::{Error, Result};
use crate::eval::bootstrap::{MeanCi, block_bootstrap_mean};
use crate::market::calibration::reliability;
use crate::market::margin::{MarginMethod, remove_margin_two_way};
use crate::models::nba::{
    EloConfig, Prediction, cover_probability, over_probability, walk_forward,
};
use crate::models::nba_tuning::{TEST_START, VALIDATION_START, window};
use crate::store::nba::{self, PinnacleQuote, StoredGame};

const RESAMPLES: usize = 2000;
const SEED: u64 = 20_261_022;

/// Marge plausible d'un marché à deux issues.
const IMPLIED_SUM: (f64, f64) = (1.0, 1.30);

/// Réglages du modèle utilisés pour le rapport.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigView {
    /// `K`.
    pub k: f64,
    /// Avantage du terrain (points d'Elo).
    pub home_advantage: f64,
    /// Pénalité du dos à dos (points d'Elo).
    pub back_to_back: f64,
    /// Part de classement conservée entre deux saisons.
    pub carry_over: f64,
    /// Pas d'apprentissage du total.
    pub totals_rate: f64,
    /// Points de marge par point d'Elo.
    pub margin_per_elo: f64,
    /// Écart-type de la marge.
    pub margin_sigma: f64,
    /// Écart-type du total.
    pub total_sigma: f64,
}

/// Ce qui n'existe que pour une configuration gelée.
#[derive(Debug, Clone, Serialize)]
pub struct FrozenView {
    /// Log loss de validation.
    pub validation_log_loss: f64,
    /// Matchs de validation.
    pub validation_games: usize,
    /// Configurations essayées.
    pub configs_tried: usize,
    /// Sessions de réglage faites.
    pub sessions: u32,
}

/// Une comparaison de probabilités : modèle contre marché sur les mêmes matchs.
#[derive(Debug, Clone, Serialize)]
pub struct ProbabilityRow {
    /// Marché.
    pub label: String,
    /// Matchs comparés.
    pub n: usize,
    /// Log loss du modèle.
    pub model_log_loss: f64,
    /// Log loss de Pinnacle.
    pub market_log_loss: f64,
    /// Écart modèle − marché (positif = le modèle est moins bon).
    pub delta: Option<MeanCi>,
}

/// Une comparaison de prévisions en points : modèle contre ligne de Pinnacle.
#[derive(Debug, Clone, Serialize)]
pub struct PointsRow {
    /// Grandeur (marge, total).
    pub label: String,
    /// Matchs comparés.
    pub n: usize,
    /// Racine de l'erreur quadratique moyenne du modèle, en points.
    pub model_rmse: f64,
    /// Idem pour la ligne de Pinnacle.
    pub market_rmse: f64,
    /// Écart des erreurs quadratiques modèle − marché (positif = le modèle est moins bon).
    pub delta_squared: Option<MeanCi>,
}

/// Test d'information : mélange du modèle et du marché sur la victoire.
#[derive(Debug, Clone, Serialize)]
pub struct InfoView {
    /// Poids du modèle choisi sur la validation.
    pub weight: f64,
    /// Matchs de validation.
    pub validation_games: usize,
    /// Écart de log loss du mélange contre le marché seul, sur le test.
    pub delta: Option<MeanCi>,
}

/// Tout ce que le rapport affiche.
#[derive(Debug, Clone, Serialize)]
pub struct NbaReport {
    /// Date de génération.
    pub generated_on: String,
    /// Mode exploratoire (aucune configuration gelée).
    pub exploratory: bool,
    /// Fenêtre évaluée, en texte.
    pub window: String,
    /// Matchs terminés en archive.
    pub games: usize,
    /// Premier et dernier coup d'envoi archivés.
    pub period: String,
    /// Matchs de la fenêtre dont les deux équipes ont déjà joué assez de matchs.
    pub scored: usize,
    /// Réglages utilisés.
    pub config: ConfigView,
    /// Réglage gelé, s'il existe.
    pub frozen: Option<FrozenView>,
    /// Vainqueur, handicap et plus/moins : log loss.
    pub probabilities: Vec<ProbabilityRow>,
    /// Marge et total : erreur en points.
    pub points: Vec<PointsRow>,
    /// Test d'information (mode gelé seulement).
    pub info: Option<InfoView>,
    /// Fiabilité de la victoire à domicile prévue par le modèle, SVG.
    pub chart: String,
}

/// Lignes de Pinnacle d'un match, probabilités sans marge comprises.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lines {
    /// Probabilité sans marge que l'équipe à domicile gagne.
    pub p_home: Option<f64>,
    /// Handicap de l'équipe à domicile (−5,5 : elle donne 5,5 points) et probabilité sans marge qu'elle le couvre.
    pub spread: Option<(f64, f64)>,
    /// Ligne du total et probabilité sans marge du « plus ».
    pub total: Option<(f64, f64)>,
}

fn fair_two_way(first: f64, second: f64, method: MarginMethod) -> Option<f64> {
    let implied = 1.0 / first + 1.0 / second;
    if !(IMPLIED_SUM.0..=IMPLIED_SUM.1).contains(&implied) {
        return None;
    }
    remove_margin_two_way(Odds::new(first)?, Odds::new(second)?, method).map(Probability::get)
}

/// Lit les trois marchés de Pinnacle dans ses cotes brutes. Un marché incomplet ou aberrant est simplement absent.
pub fn lines(quotes: &[PinnacleQuote], home: &str, away: &str, method: MarginMethod) -> Lines {
    let find = |market: &str, outcome: &str| {
        quotes
            .iter()
            .find(|q| q.market == market && q.outcome == outcome)
    };
    let mut lines = Lines::default();
    if let (Some(h), Some(a)) = (find("h2h", home), find("h2h", away)) {
        lines.p_home = fair_two_way(h.price, a.price, method);
    }
    if let (Some(h), Some(a)) = (find("spreads", home), find("spreads", away))
        && let (Some(point_home), Some(point_away)) = (h.point, a.point)
        && (point_home + point_away).abs() < 1e-9
        && let Some(p) = fair_two_way(h.price, a.price, method)
    {
        lines.spread = Some((point_home, p));
    }
    if let (Some(over), Some(under)) = (find("totals", "Over"), find("totals", "Under"))
        && let (Some(point_over), Some(point_under)) = (over.point, under.point)
        && (point_over - point_under).abs() < 1e-9
        && let Some(p) = fair_two_way(over.price, under.price, method)
    {
        lines.total = Some((point_over, p));
    }
    lines
}

/// Un match évalué : prévisions, marché et résultat.
struct Row<'a> {
    prediction: &'a Prediction,
    lines: Lines,
    block: u32,
}

fn rows_in<'a>(
    conn: &Connection,
    predictions: &[&'a Prediction],
    method: MarginMethod,
) -> Result<Vec<Row<'a>>> {
    predictions
        .iter()
        .map(|&prediction| {
            let quotes =
                nba::pinnacle_quotes_before(conn, &prediction.event_id, prediction.commence_time)?;
            Ok(Row {
                prediction,
                lines: lines(
                    &quotes,
                    &prediction.home_team,
                    &prediction.away_team,
                    method,
                ),
                // Un bloc = une semaine : les matchs d'une même semaine ne sont pas indépendants (blessures, séries).
                block: u32::try_from(prediction.commence_time.date_naive().num_days_from_ce() / 7)
                    .unwrap_or(0),
            })
        })
        .collect()
}

fn ci(values: Vec<(u32, f64)>) -> Option<MeanCi> {
    block_bootstrap_mean(&values, RESAMPLES, SEED)
}

fn win_row(rows: &[Row<'_>]) -> ProbabilityRow {
    let used: Vec<(&Row<'_>, f64)> = rows
        .iter()
        .filter_map(|r| r.lines.p_home.map(|m| (r, m)))
        .collect();
    let outcome = |r: &Row<'_>, p: f64| {
        if r.prediction.actual_margin > 0 {
            p
        } else {
            1.0 - p
        }
    };
    ProbabilityRow {
        label: "Vainqueur".into(),
        n: used.len(),
        model_log_loss: mean(
            used.iter()
                .map(|(r, _)| loss(outcome(r, r.prediction.p_home))),
        ),
        market_log_loss: mean(used.iter().map(|(r, m)| loss(outcome(r, *m)))),
        delta: ci(used
            .iter()
            .map(|(r, m)| {
                (
                    r.block,
                    loss(outcome(r, r.prediction.p_home)) - loss(outcome(r, *m)),
                )
            })
            .collect()),
    }
}

fn cover_row(rows: &[Row<'_>], config: &ConfigView) -> ProbabilityRow {
    // Handicap couvert : la marge réelle dépasse `−handicap`. Une égalité (ligne entière) est un remboursement : exclue.
    let used: Vec<(&Row<'_>, f64, f64, bool)> = rows
        .iter()
        .filter_map(|r| {
            let (point, market) = r.lines.spread?;
            let result = f64::from(r.prediction.actual_margin) + point;
            (result != 0.0).then_some((r, point, market, result > 0.0))
        })
        .collect();
    let model = |r: &Row<'_>, point: f64| {
        cover_probability(
            r.prediction.gap * config.margin_per_elo,
            config.margin_sigma,
            point,
        )
    };
    let outcome = |p: f64, covered: bool| if covered { p } else { 1.0 - p };
    ProbabilityRow {
        label: "Handicap".into(),
        n: used.len(),
        model_log_loss: mean(
            used.iter()
                .map(|(r, point, _, c)| loss(outcome(model(r, *point), *c))),
        ),
        market_log_loss: mean(used.iter().map(|(_, _, m, c)| loss(outcome(*m, *c)))),
        delta: ci(used
            .iter()
            .map(|(r, point, m, c)| {
                (
                    r.block,
                    loss(outcome(model(r, *point), *c)) - loss(outcome(*m, *c)),
                )
            })
            .collect()),
    }
}

fn over_row(rows: &[Row<'_>], config: &ConfigView) -> ProbabilityRow {
    let used: Vec<(&Row<'_>, f64, f64, bool)> = rows
        .iter()
        .filter_map(|r| {
            let (line, market) = r.lines.total?;
            let diff = f64::from(r.prediction.actual_total) - line;
            (diff != 0.0).then_some((r, line, market, diff > 0.0))
        })
        .collect();
    let model =
        |r: &Row<'_>, line: f64| over_probability(r.prediction.total, config.total_sigma, line);
    let outcome = |p: f64, over: bool| if over { p } else { 1.0 - p };
    ProbabilityRow {
        label: "Plus/moins".into(),
        n: used.len(),
        model_log_loss: mean(
            used.iter()
                .map(|(r, line, _, o)| loss(outcome(model(r, *line), *o))),
        ),
        market_log_loss: mean(used.iter().map(|(_, _, m, o)| loss(outcome(*m, *o)))),
        delta: ci(used
            .iter()
            .map(|(r, line, m, o)| {
                (
                    r.block,
                    loss(outcome(model(r, *line), *o)) - loss(outcome(*m, *o)),
                )
            })
            .collect()),
    }
}

fn margin_row(rows: &[Row<'_>], config: &ConfigView) -> PointsRow {
    // La ligne de Pinnacle est sa médiane de la marge de l'équipe à domicile : `−handicap`.
    let used: Vec<(&Row<'_>, f64)> = rows
        .iter()
        .filter_map(|r| r.lines.spread.map(|(point, _)| (r, -point)))
        .collect();
    let actual = |r: &Row<'_>| f64::from(r.prediction.actual_margin);
    let model = |r: &Row<'_>| r.prediction.gap * config.margin_per_elo;
    PointsRow {
        label: "Marge de l'équipe à domicile".into(),
        n: used.len(),
        model_rmse: mean(used.iter().map(|(r, _)| (actual(r) - model(r)).powi(2))).sqrt(),
        market_rmse: mean(used.iter().map(|(r, line)| (actual(r) - line).powi(2))).sqrt(),
        delta_squared: ci(used
            .iter()
            .map(|(r, line)| {
                (
                    r.block,
                    (actual(r) - model(r)).powi(2) - (actual(r) - line).powi(2),
                )
            })
            .collect()),
    }
}

fn total_row(rows: &[Row<'_>]) -> PointsRow {
    let used: Vec<(&Row<'_>, f64)> = rows
        .iter()
        .filter_map(|r| r.lines.total.map(|(line, _)| (r, line)))
        .collect();
    let actual = |r: &Row<'_>| f64::from(r.prediction.actual_total);
    PointsRow {
        label: "Total de points".into(),
        n: used.len(),
        model_rmse: mean(
            used.iter()
                .map(|(r, _)| (actual(r) - r.prediction.total).powi(2)),
        )
        .sqrt(),
        market_rmse: mean(used.iter().map(|(r, line)| (actual(r) - line).powi(2))).sqrt(),
        delta_squared: ci(used
            .iter()
            .map(|(r, line)| {
                (
                    r.block,
                    (actual(r) - r.prediction.total).powi(2) - (actual(r) - line).powi(2),
                )
            })
            .collect()),
    }
}

/// Construit le rapport. En mode gelé, évalue le test ; sans configuration gelée, tout l'archive en mode exploratoire.
pub fn build(conn: &Connection, method: MarginMethod, generated_on: &str) -> Result<NbaReport> {
    let games: Vec<StoredGame> = nba::load_games(conn)?;
    let (Some(first), Some(last)) = (games.first(), games.last()) else {
        return Err(Error::Data(
            "aucun match de NBA terminé en base : `edge nba snapshot` récupère les résultats (le premier appel suit l'ouverture de la saison)".into(),
        ));
    };
    let period = format!(
        "{} → {}",
        first.commence_time.format("%d/%m/%Y"),
        last.commence_time.format("%d/%m/%Y")
    );

    let frozen = nba::latest_tuning(conn)?;
    let (elo, rate, config) = match &frozen {
        Some(f) => (
            EloConfig {
                k: f.k,
                home_advantage: f.home_advantage,
                back_to_back: f.back_to_back,
                carry_over: f.carry_over,
            },
            f.totals_rate,
            ConfigView {
                k: f.k,
                home_advantage: f.home_advantage,
                back_to_back: f.back_to_back,
                carry_over: f.carry_over,
                totals_rate: f.totals_rate,
                margin_per_elo: f.margin_per_elo,
                margin_sigma: f.margin_sigma,
                total_sigma: f.total_sigma,
            },
        ),
        None => {
            let start = EloConfig::starting_point();
            (
                start,
                0.06,
                ConfigView {
                    k: start.k,
                    home_advantage: start.home_advantage,
                    back_to_back: start.back_to_back,
                    carry_over: start.carry_over,
                    totals_rate: 0.06,
                    margin_per_elo: 1.0 / 28.0,
                    margin_sigma: 12.0,
                    total_sigma: 18.0,
                },
            )
        }
    };
    let predictions = walk_forward(&games, elo, rate);

    let (from, label): (NaiveDate, String) = if frozen.is_some() {
        (
            TEST_START,
            format!("test : à partir du {}", TEST_START.format("%d/%m/%Y")),
        )
    } else {
        (
            first.commence_time.date_naive(),
            "toute l'archive (exploratoire : configuration par défaut, non gelée)".to_string(),
        )
    };
    let evaluated = window(&predictions, from, None);
    let rows = rows_in(conn, &evaluated, method)?;

    let info = match &frozen {
        Some(_) => {
            let validation = window(&predictions, VALIDATION_START, Some(TEST_START));
            let validation_rows = rows_in(conn, &validation, method)?;
            let usable: Vec<(&Row<'_>, f64)> = validation_rows
                .iter()
                .filter_map(|r| r.lines.p_home.map(|m| (r, m)))
                .collect();
            let outcome = |r: &Row<'_>, p: f64| {
                if r.prediction.actual_margin > 0 {
                    p
                } else {
                    1.0 - p
                }
            };
            let best = (0..=10)
                .map(|i| f64::from(i) / 10.0)
                .map(|w| {
                    (
                        w,
                        mean(
                            usable
                                .iter()
                                .map(|(r, m)| loss(outcome(r, blend(*m, r.prediction.p_home, w)))),
                        ),
                    )
                })
                .filter(|(_, l)| l.is_finite())
                .reduce(|best, c| if c.1 < best.1 { c } else { best })
                .map(|(w, _)| w);
            best.map(|weight| {
                let values: Vec<(u32, f64)> = rows
                    .iter()
                    .filter_map(|r| {
                        let m = r.lines.p_home?;
                        Some((
                            r.block,
                            loss(outcome(r, blend(m, r.prediction.p_home, weight)))
                                - loss(outcome(r, m)),
                        ))
                    })
                    .collect();
                InfoView {
                    weight,
                    validation_games: usable.len(),
                    delta: ci(values),
                }
            })
        }
        None => None,
    };

    let pairs: Vec<(f64, bool)> = evaluated
        .iter()
        .map(|p| (p.p_home, p.actual_margin > 0))
        .collect();
    let chart = if pairs.is_empty() {
        String::new()
    } else {
        reliability_chart(
            &reliability(&pairs, 0.1),
            "Victoire à domicile : prévue contre observée",
        )?
    };

    Ok(NbaReport {
        generated_on: generated_on.to_string(),
        exploratory: frozen.is_none(),
        window: label,
        games: games.len(),
        period,
        scored: evaluated.len(),
        frozen: match &frozen {
            Some(f) => Some(FrozenView {
                validation_log_loss: f.validation_log_loss,
                validation_games: f.validation_games,
                configs_tried: f.configs_tried,
                sessions: nba::tuning_sessions(conn)?,
            }),
            None => None,
        },
        probabilities: vec![
            win_row(&rows),
            cover_row(&rows, &config),
            over_row(&rows, &config),
        ],
        points: vec![margin_row(&rows, &config), total_row(&rows)],
        info,
        chart,
        config,
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::domain::{Event, Odds, Quota, Quote};
    use crate::models::nba_tuning::{simulation::simulated_season, tune};
    use crate::report::render::render_nba_report;
    use crate::store::archive::{Endpoint, NewApiCall, record_api_call, record_snapshot};
    use crate::store::migrate;
    use crate::store::nba::{FrozenNba, insert_tuning, upsert_game};

    fn quote(market: &str, outcome: &str, point: Option<f64>, price: f64) -> PinnacleQuote {
        PinnacleQuote {
            market: market.into(),
            outcome: outcome.into(),
            point,
            price,
        }
    }

    fn full_market() -> Vec<PinnacleQuote> {
        // Pinnacle : domicile à 60 % avec 3 % de marge, handicap −3,5, total 226,5.
        vec![
            quote("h2h", "Dom", None, 1.0 / (0.6 * 1.03)),
            quote("h2h", "Ext", None, 1.0 / (0.4 * 1.03)),
            quote("spreads", "Dom", Some(-3.5), 1.95),
            quote("spreads", "Ext", Some(3.5), 1.95),
            quote("totals", "Over", Some(226.5), 1.95),
            quote("totals", "Under", Some(226.5), 1.95),
        ]
    }

    #[test]
    fn the_three_pinnacle_markets_are_read_with_their_fair_probabilities() {
        let read = lines(&full_market(), "Dom", "Ext", MarginMethod::Multiplicative);

        assert!((read.p_home.unwrap() - 0.6).abs() < 1e-9);
        let (spread, cover) = read.spread.unwrap();
        assert_eq!(spread, -3.5);
        assert!((cover - 0.5).abs() < 1e-9);
        let (line, over) = read.total.unwrap();
        assert_eq!(line, 226.5);
        assert!((over - 0.5).abs() < 1e-9);
    }

    #[test]
    fn an_incomplete_inconsistent_or_aberrant_market_is_simply_absent() {
        // Handicaps qui ne se répondent pas (−3,5 et +4,5), total sans « Under », vainqueur à une marge de −10 %.
        let quotes = vec![
            quote("h2h", "Dom", None, 2.2),
            quote("h2h", "Ext", None, 2.2),
            quote("spreads", "Dom", Some(-3.5), 1.95),
            quote("spreads", "Ext", Some(4.5), 1.95),
            quote("totals", "Over", Some(226.5), 1.95),
        ];

        let read = lines(&quotes, "Dom", "Ext", MarginMethod::Multiplicative);

        assert_eq!(read, Lines::default());
    }

    /// Une base avec la saison simulée, les résultats, et un relevé de Pinnacle 3 h avant chaque match.
    fn season_with_market() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        for game in simulated_season() {
            upsert_game(
                &conn,
                &crate::ingest::odds_api::FinishedGame {
                    event_id: game.event_id.clone(),
                    commence_time: game.commence_time,
                    home_team: game.home_team.clone(),
                    away_team: game.away_team.clone(),
                    home_score: game.home_score,
                    away_score: game.away_score,
                },
                game.commence_time,
            )
            .unwrap();
            let called_at = game.commence_time - chrono::Duration::hours(3);
            let quotes = full_market()
                .into_iter()
                .map(|q| Quote {
                    bookmaker_key: "pinnacle".into(),
                    market_key: q.market,
                    updated_at: called_at,
                    outcome: match q.outcome.as_str() {
                        "Dom" => game.home_team.clone(),
                        "Ext" => game.away_team.clone(),
                        other => other.to_string(),
                    },
                    point: q.point,
                    price: Odds::new(q.price).unwrap(),
                })
                .collect();
            let event = Event {
                id: game.event_id.clone(),
                sport_key: "basketball_nba".into(),
                commence_time: game.commence_time,
                home_team: game.home_team.clone(),
                away_team: game.away_team.clone(),
                quotes,
            };
            let id = record_api_call(
                &conn,
                &NewApiCall {
                    called_at,
                    endpoint: Endpoint::Odds,
                    sport_key: Some("basketball_nba"),
                    params: "markets=h2h,spreads,totals",
                    http_status: 200,
                    quota: Quota::default(),
                    body: None,
                },
            )
            .unwrap();
            record_snapshot(&mut conn, id, &[event]).unwrap();
        }
        conn
    }

    #[test]
    fn without_a_frozen_configuration_the_report_is_exploratory_on_the_whole_archive() {
        let conn = season_with_market();

        let report = build(&conn, MarginMethod::Multiplicative, "2026-10-19").unwrap();

        assert!(report.exploratory);
        assert!(report.frozen.is_none() && report.info.is_none());
        assert!(report.games > 1000);
        let labels: Vec<&str> = report
            .probabilities
            .iter()
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Vainqueur", "Handicap", "Plus/moins"]);
        assert!(
            report
                .probabilities
                .iter()
                .all(|r| r.n > 500 && r.delta.is_some())
        );
        assert!(
            report
                .points
                .iter()
                .all(|r| r.n > 500 && r.model_rmse.is_finite() && r.market_rmse.is_finite())
        );
        assert!(report.chart.contains("<svg"));
        let html = render_nba_report(&report).unwrap();
        assert!(html.contains("Exploratoire"));
        assert!(html.contains("NBA : le modèle contre Pinnacle"));
    }

    #[test]
    fn with_a_frozen_configuration_only_the_test_window_is_evaluated_and_the_info_test_appears() {
        let conn = season_with_market();
        let games = crate::store::nba::load_games(&conn).unwrap();
        let tuned = tune(&games).unwrap();
        insert_tuning(
            &conn,
            &FrozenNba {
                created_at: Utc::now(),
                k: tuned.elo.k,
                home_advantage: tuned.elo.home_advantage,
                back_to_back: tuned.elo.back_to_back,
                carry_over: tuned.elo.carry_over,
                totals_rate: tuned.totals_rate,
                margin_per_elo: tuned.margin_per_elo,
                margin_sigma: tuned.margin_sigma,
                total_sigma: tuned.total_sigma,
                validation_log_loss: tuned.validation_log_loss,
                validation_games: tuned.validation_games,
                configs_tried: tuned.configs_tried,
            },
        )
        .unwrap();

        let report = build(&conn, MarginMethod::Multiplicative, "2027-02-02").unwrap();

        assert!(!report.exploratory);
        assert_eq!(report.frozen.as_ref().unwrap().configs_tried, 57);
        assert!(report.window.contains("01/02/2027"));
        assert!(report.scored > 300 && report.scored < report.games);
        assert!(report.info.is_some());
        let html = render_nba_report(&report).unwrap();
        assert!(!html.contains("Exploratoire."));
        assert!(html.contains("57 configurations"));
    }

    #[test]
    fn an_empty_archive_says_how_results_arrive() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();

        let error = build(&conn, MarginMethod::Power, "x")
            .unwrap_err()
            .to_string();

        assert!(error.contains("edge nba snapshot"));
    }
}
