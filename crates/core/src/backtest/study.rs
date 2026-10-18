//! L'étude complète d'une règle de sélection : réglage du seuil, gel, évaluation sur le test.
//!
//! Protocole (invariants 4, 5 et 8) : le **seuil d'avantage** de chaque source est choisi sur les périodes
//! d'entraînement et de validation (le CLV moyen le plus élevé, avec un nombre minimal de paris), puis **gelé**
//! avant que le test ne soit regardé. Le test n'est évalué qu'avec ces seuils gelés.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

use super::candidates::{
    GameInputs, Generator, Period, SOFT_BOOKMAKERS, in_period, model_vs_price, soft_vs_sharp,
};
use super::engine::{Candidate, StakingRule, run};
use super::metrics::{
    RiskReport, Scenario, SelectionCalibration, Summary, clv_ci, flat_roi_ci, monte_carlo_risk,
    selection_calibration, summarize,
};
use crate::domain::Probs1x2;
use crate::error::{Error, Result};
use crate::eval::bootstrap::MeanCi;
use crate::market::margin::MarginMethod;
use crate::store::backtests::{self, FrozenThreshold, RunKey};

/// Avantages minimaux essayés.
pub const THRESHOLDS: [f64; 6] = [0.0, 0.01, 0.02, 0.03, 0.05, 0.08];

/// Fractions de Kelly comparées côte à côte.
pub const KELLY_FRACTIONS: [f64; 3] = [1.0, 0.5, 0.25];

/// Nombre minimal de paris pour qu'un seuil soit éligible au gel : en dessous, le CLV est trop bruité pour guider.
pub const MIN_SELECTION_BETS: usize = 50;

/// Trajectoires simulées pour estimer le risque de ruine.
const MONTE_CARLO_PATHS: usize = 2000;

/// Décotes de prix testées en sensibilité : cotes telles quelles (courtier proche de Pinnacle), puis ce que
/// l'archive mesure pour Betclic (0,957) et Winamax (0,910) par rapport à Pinnacle.
pub const PRICE_FACTORS: [f64; 3] = [1.0, 0.957, 0.910];

/// Une source de paris candidats.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// Prix d'avant-match d'un bookmaker contre la cote juste de Pinnacle au même instant.
    Soft(&'static str),
    /// Probabilité du modèle (Dixon-Coles gelé) contre le prix d'avant-match de Bet365.
    Model,
}

impl Source {
    /// Étiquette stockée en base et affichée.
    pub fn label(&self) -> String {
        match self {
            Source::Soft(book) => format!("soft:{book}"),
            Source::Model => "model:dixon_coles".to_string(),
        }
    }

    /// Périodes sur lesquelles le seuil est réglé : les prédictions du modèle n'existent que depuis la validation.
    fn selection_periods(&self) -> &'static [Period] {
        match self {
            Source::Soft(_) => &[Period::Train, Period::Validation],
            Source::Model => &[Period::Validation],
        }
    }
}

/// Toutes les sources étudiées : huit bookmakers « mous » et le modèle.
pub fn sources() -> Vec<Source> {
    SOFT_BOOKMAKERS
        .iter()
        .map(|book| Source::Soft(book))
        .chain([Source::Model])
        .collect()
}

/// Génère les candidats d'une source sur un jeu de matchs.
pub fn generate(
    source: &Source,
    games: &[&GameInputs],
    predictions: &HashMap<i64, Probs1x2>,
    generator: &Generator,
) -> Vec<Candidate> {
    match source {
        Source::Soft(book) => soft_vs_sharp(games, book, generator),
        Source::Model => model_vs_price(games, predictions, "B365", generator),
    }
}

fn candidates_in(
    source: &Source,
    all: &[GameInputs],
    periods: &[Period],
    predictions: &HashMap<i64, Probs1x2>,
    generator: &Generator,
) -> Vec<Candidate> {
    let games: Vec<&GameInputs> = periods.iter().flat_map(|p| in_period(all, *p)).collect();
    generate(source, &games, predictions, generator)
}

/// Résultat d'un seuil sur la période de réglage.
#[derive(Debug, Clone, PartialEq)]
pub struct ThresholdResult {
    /// Avantage minimal.
    pub threshold: f64,
    /// Paris retenus.
    pub bets: usize,
    /// CLV moyen.
    pub clv: Option<f64>,
    /// Rendement moyen à mise fixe.
    pub roi: Option<f64>,
}

/// Réglage d'une source.
#[derive(Debug, Clone, PartialEq)]
pub struct TuneRow {
    /// Source.
    pub source: String,
    /// Un résultat par seuil essayé.
    pub results: Vec<ThresholdResult>,
    /// Seuil gelé, `None` si aucun seuil n'est éligible.
    pub chosen: Option<f64>,
}

fn mean_flat_roi(candidates: &[Candidate]) -> Option<f64> {
    if candidates.is_empty() {
        return None;
    }
    Some(
        candidates
            .iter()
            .map(|c| if c.won { c.odds.get() - 1.0 } else { -1.0 })
            .sum::<f64>()
            / candidates.len() as f64,
    )
}

fn mean_clv(candidates: &[Candidate]) -> Option<f64> {
    let values: Vec<f64> = candidates
        .iter()
        .filter_map(|c| c.close_fair.map(|p| c.odds.get() * p.get() - 1.0))
        .collect();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// Choisit, pour chaque source, le seuil au CLV moyen le plus élevé parmi ceux qui retiennent assez de paris, et le gèle.
pub fn tune(
    conn: &Connection,
    all: &[GameInputs],
    predictions: &HashMap<i64, Probs1x2>,
    method: MarginMethod,
    now: DateTime<Utc>,
) -> Result<Vec<TuneRow>> {
    let sources: Vec<Source> = sources()
        .into_iter()
        .filter(|s| *s != Source::Model || !predictions.is_empty())
        .collect();
    let configs_tried = sources.len() * THRESHOLDS.len();
    let mut rows = Vec::new();
    for source in &sources {
        let mut results = Vec::new();
        for threshold in THRESHOLDS {
            let generator = Generator {
                min_edge: threshold,
                price_factor: 1.0,
                method,
            };
            let candidates = candidates_in(
                source,
                all,
                source.selection_periods(),
                predictions,
                &generator,
            );
            results.push(ThresholdResult {
                threshold,
                bets: candidates.len(),
                clv: mean_clv(&candidates),
                roi: mean_flat_roi(&candidates),
            });
        }
        let chosen = results
            .iter()
            .filter(|r| r.bets >= MIN_SELECTION_BETS && r.clv.is_some())
            .max_by(|a, b| {
                a.clv
                    .unwrap_or(f64::MIN)
                    .total_cmp(&b.clv.unwrap_or(f64::MIN))
            })
            .cloned();
        if let Some(best) = &chosen {
            backtests::insert_selection(
                conn,
                &FrozenThreshold {
                    source: source.label(),
                    threshold: best.threshold,
                    selection_clv: best.clv,
                    selection_bets: best.bets,
                    configs_tried,
                },
                now,
            )?;
        }
        rows.push(TuneRow {
            source: source.label(),
            results,
            chosen: chosen.map(|r| r.threshold),
        });
    }
    Ok(rows)
}

/// Statistiques d'une période pour la règle gelée.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeriodRow {
    /// Période.
    pub period: Period,
    /// Paris retenus.
    pub bets: usize,
    /// CLV moyen avec intervalle à 95 %.
    pub clv: Option<MeanCi>,
    /// Rendement à mise fixe avec intervalle à 95 %.
    pub roi: Option<MeanCi>,
}

/// Résultat des mises d'une fraction de Kelly.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StakingRow {
    /// Fraction de Kelly.
    pub kelly_fraction: f64,
    /// Résumé du backtest.
    pub summary: Summary,
    /// Risque si les probabilités annoncées sont justes.
    pub risk_if_announced: Option<RiskReport>,
    /// Risque si la clôture du marché dit vrai (l'avantage supposé n'existe pas).
    pub risk_if_close_is_true: Option<RiskReport>,
}

/// Espérance annoncée contre rendement réalisé, par tranche d'espérance annoncée.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvBucket {
    /// Libellé de la tranche.
    pub label: &'static str,
    /// Matchs (meilleure issue de chaque match) dans la tranche.
    pub n: usize,
    /// Espérance moyenne annoncée.
    pub announced: f64,
    /// Rendement moyen réalisé à mise fixe.
    pub realized: f64,
    /// CLV moyen.
    pub clv: Option<f64>,
}

const EV_BUCKETS: [(&str, f64, f64); 6] = [
    ("< −5 %", f64::NEG_INFINITY, -0.05),
    ("−5 % à 0 %", -0.05, 0.0),
    ("0 % à 2 %", 0.0, 0.02),
    ("2 % à 5 %", 0.02, 0.05),
    ("5 % à 10 %", 0.05, 0.10),
    ("≥ 10 %", 0.10, f64::INFINITY),
];

/// Regroupe la meilleure issue de chaque match par espérance annoncée : si l'espérance annoncée prédisait le
/// rendement réalisé, les deux colonnes se suivraient. C'est le diagnostic central d'une règle de sélection.
pub fn ev_buckets(candidates: &[Candidate]) -> Vec<EvBucket> {
    EV_BUCKETS
        .iter()
        .filter_map(|(label, low, high)| {
            let selected: Vec<Candidate> = candidates
                .iter()
                .filter(|c| {
                    let edge = c.probability.get() * c.odds.get() - 1.0;
                    edge >= *low && edge < *high
                })
                .cloned()
                .collect();
            if selected.is_empty() {
                return None;
            }
            let announced = selected
                .iter()
                .map(|c| c.probability.get() * c.odds.get() - 1.0)
                .sum::<f64>()
                / selected.len() as f64;
            Some(EvBucket {
                label,
                n: selected.len(),
                announced,
                realized: mean_flat_roi(&selected).unwrap_or(0.0),
                clv: mean_clv(&selected),
            })
        })
        .collect()
}

/// Verdict sur le test : ce que l'on peut honnêtement conclure d'une règle gelée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Moins de 30 paris au test : aucune conclusion possible.
    NoPower,
    /// L'intervalle du CLV au test est entièrement négatif : la règle perd face à la clôture du marché.
    Rejected,
    /// L'intervalle du CLV au test est entièrement positif.
    Positive,
    /// L'intervalle du CLV au test contient 0.
    Inconclusive,
}

/// Verdict d'une règle d'après sa ligne « test ».
pub fn verdict(test: Option<&PeriodRow>) -> Verdict {
    let Some(row) = test else {
        return Verdict::NoPower;
    };
    match (row.bets, row.clv) {
        (bets, _) if bets < 30 => Verdict::NoPower,
        (_, Some(clv)) if clv.high < 0.0 => Verdict::Rejected,
        (_, Some(clv)) if clv.low > 0.0 => Verdict::Positive,
        _ => Verdict::Inconclusive,
    }
}

/// CLV d'une saison pour le seuil gelé : montre si l'effet est stable d'une saison à l'autre ou concentré sur une seule.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeasonRow {
    /// Saison (`2019/20`).
    pub season: String,
    /// Paris retenus.
    pub bets: usize,
    /// CLV moyen.
    pub clv: Option<f64>,
    /// Rendement moyen à mise fixe.
    pub roi: Option<f64>,
}

/// Étude d'une source avec son seuil gelé.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceStudy {
    /// Source.
    pub label: String,
    /// Seuil gelé.
    pub threshold: f64,
    /// CLV de la période de réglage au moment du gel.
    pub frozen_clv: Option<f64>,
    /// Paris de la période de réglage au moment du gel.
    pub frozen_bets: usize,
    /// Une ligne par période (entraînement, validation, test).
    pub periods: Vec<PeriodRow>,
    /// CLV et rendement saison par saison, sur toute l'histoire, au seuil gelé.
    pub by_season: Vec<SeasonRow>,
    /// Ce que l'on peut conclure du test.
    pub verdict: Verdict,
    /// Les paris du test : annoncé contre réalisé.
    pub calibration: Option<SelectionCalibration>,
    /// Mises du test, une ligne par fraction de Kelly.
    pub staking: Vec<StakingRow>,
    /// Espérance annoncée contre réalisée, sur toutes les meilleures issues du test.
    pub ev_buckets: Vec<EvBucket>,
    /// Sensibilité à la décote de prix : `(facteur, paris, CLV moyen, rendement moyen)`.
    pub price_sensitivity: Vec<(f64, usize, Option<f64>, Option<f64>)>,
}

/// Étudie une source avec le seuil gelé.
pub fn study_source(
    source: &Source,
    frozen: &FrozenThreshold,
    all: &[GameInputs],
    predictions: &HashMap<i64, Probs1x2>,
    method: MarginMethod,
) -> Result<SourceStudy> {
    let generator = Generator {
        min_edge: frozen.threshold,
        price_factor: 1.0,
        method,
    };
    let mut periods = Vec::new();
    for period in [Period::Train, Period::Validation, Period::Test] {
        let candidates = candidates_in(source, all, &[period], predictions, &generator);
        periods.push(PeriodRow {
            period,
            bets: candidates.len(),
            clv: clv_ci(&candidates),
            roi: flat_roi_ci(&candidates),
        });
    }
    let test = candidates_in(source, all, &[Period::Test], predictions, &generator);
    let by_season = {
        let everything = candidates_in(
            source,
            all,
            &[Period::Train, Period::Validation, Period::Test],
            predictions,
            &generator,
        );
        let mut seasons: std::collections::BTreeMap<String, Vec<Candidate>> =
            std::collections::BTreeMap::new();
        for candidate in everything {
            let label = crate::ingest::football_data::Season::containing(candidate.date).label();
            seasons.entry(label).or_default().push(candidate);
        }
        seasons
            .into_iter()
            .map(|(season, candidates)| SeasonRow {
                season,
                bets: candidates.len(),
                clv: mean_clv(&candidates),
                roi: mean_flat_roi(&candidates),
            })
            .collect::<Vec<_>>()
    };

    let mut staking = Vec::new();
    for fraction in KELLY_FRACTIONS {
        let rule = StakingRule::standard(fraction);
        let result = run(&test, &rule)?;
        staking.push(StakingRow {
            kelly_fraction: fraction,
            summary: summarize(&result),
            risk_if_announced: monte_carlo_risk(
                &test,
                &rule,
                Scenario::Announced,
                MONTE_CARLO_PATHS,
            )?,
            risk_if_close_is_true: monte_carlo_risk(
                &test,
                &rule,
                Scenario::CloseFair,
                MONTE_CARLO_PATHS,
            )?,
        });
    }

    // Toutes les meilleures issues du test (sans seuil) pour le diagnostic espérance annoncée / réalisée.
    let everything = Generator {
        min_edge: f64::NEG_INFINITY,
        price_factor: 1.0,
        method,
    };
    let all_best = candidates_in(source, all, &[Period::Test], predictions, &everything);

    let mut price_sensitivity = Vec::new();
    for factor in PRICE_FACTORS {
        let generator = Generator {
            min_edge: frozen.threshold,
            price_factor: factor,
            method,
        };
        let candidates = candidates_in(source, all, &[Period::Test], predictions, &generator);
        price_sensitivity.push((
            factor,
            candidates.len(),
            mean_clv(&candidates),
            mean_flat_roi(&candidates),
        ));
    }

    let test_verdict = verdict(periods.last());
    Ok(SourceStudy {
        label: source.label(),
        threshold: frozen.threshold,
        frozen_clv: frozen.selection_clv,
        frozen_bets: frozen.selection_bets,
        periods,
        by_season,
        verdict: test_verdict,
        calibration: selection_calibration(&test),
        staking,
        ev_buckets: ev_buckets(&all_best),
        price_sensitivity,
    })
}

/// Probabilités du Dixon-Coles gelé sur les périodes de validation et de test (vide sans réglage ni évaluation du
/// modèle : la source « modèle » est alors simplement absente).
pub fn load_model_predictions(conn: &Connection) -> Result<HashMap<i64, Probs1x2>> {
    use crate::models::tuning::{test_window, validation_window};
    use crate::models::walk_forward::Prediction;
    use crate::store::predictions;

    let Some(frozen) = predictions::latest_tuning(conn, "dixon_coles")? else {
        return Ok(HashMap::new());
    };
    let last: Option<String> =
        conn.query_row("SELECT MAX(match_date) FROM game", [], |row| row.get(0))?;
    let Some(last) =
        last.and_then(|text| chrono::NaiveDate::parse_from_str(&text, "%Y-%m-%d").ok())
    else {
        return Ok(HashMap::new());
    };
    let mut merged: HashMap<i64, Probs1x2> = HashMap::new();
    for window in [validation_window(), test_window(last)] {
        if let Some(run_id) = predictions::find_run(conn, &frozen.config, window)? {
            let rows: Vec<Prediction> = predictions::load_predictions(conn, run_id)?;
            merged.extend(rows.iter().filter_map(|p| {
                Probs1x2::new(p.p_home, p.p_draw, p.p_away).map(|q| (p.game_id, q))
            }));
        }
    }
    Ok(merged)
}

/// Étudie toutes les sources qui ont un seuil gelé. Refuse de s'exécuter sans aucun gel.
pub fn study_all(
    conn: &Connection,
    all: &[GameInputs],
    predictions: &HashMap<i64, Probs1x2>,
    method: MarginMethod,
) -> Result<Vec<SourceStudy>> {
    let mut studies = Vec::new();
    for source in sources() {
        if let Some(frozen) = backtests::latest_selection(conn, &source.label())? {
            studies.push(study_source(&source, &frozen, all, predictions, method)?);
        }
    }
    if studies.is_empty() {
        return Err(Error::Data(
            "aucun seuil gelé : lance d'abord `edge backtest tune`".to_string(),
        ));
    }
    Ok(studies)
}

/// Enregistre dans le journal les exécutions du **test** de chaque source gelée, pour chaque fraction de Kelly.
/// Renvoie le nombre d'exécutions nouvellement enregistrées.
pub fn persist_test_runs(
    conn: &mut Connection,
    all: &[GameInputs],
    predictions: &HashMap<i64, Probs1x2>,
    method: MarginMethod,
    now: DateTime<Utc>,
) -> Result<usize> {
    let mut stored = 0;
    for source in sources() {
        let Some(frozen) = backtests::latest_selection(conn, &source.label())? else {
            continue;
        };
        let generator = Generator {
            min_edge: frozen.threshold,
            price_factor: 1.0,
            method,
        };
        let candidates = candidates_in(&source, all, &[Period::Test], predictions, &generator);
        for fraction in KELLY_FRACTIONS {
            let result = run(&candidates, &StakingRule::standard(fraction))?;
            let key = RunKey {
                source: source.label(),
                period: Period::Test,
                threshold: frozen.threshold,
                kelly_fraction: fraction,
                price_factor: 1.0,
            };
            if backtests::insert_run(conn, &key, &candidates, &result, now)?.is_some() {
                stored += 1;
            }
        }
    }
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap as Map;

    use chrono::{Days, NaiveDate};

    use super::*;
    use crate::domain::{Moment, Odds, Odds1x2, Outcome};
    use crate::models::testing::Lcg;
    use crate::store::migrate;

    fn odds(p: [f64; 3], margin: f64) -> Odds1x2 {
        let price = |q: f64| Odds::new(1.0 / (q * (1.0 + margin))).unwrap();
        Odds1x2 {
            home: price(p[0]),
            draw: price(p[1]),
            away: price(p[2]),
        }
    }

    /// Monde simulé : Pinnacle dit vrai (les résultats sont tirés selon ses probabilités) ; Bet365 se trompe de
    /// ± `lag` autour du prix de Pinnacle (un bookmaker lent, bruit symétrique) ; la clôture est la vérité.
    fn world(games: usize, lag: f64, seed: u64) -> Vec<GameInputs> {
        let mut rng = Lcg(seed);
        let start = NaiveDate::from_ymd_opt(2022, 8, 6).unwrap();
        (0..games)
            .map(|i| {
                let home = 0.25 + 0.45 * rng.next();
                let draw = (1.0 - home) * (0.25 + 0.2 * rng.next());
                let p = [home, draw, 1.0 - home - draw];
                let roll = rng.next();
                let result = if roll < p[0] {
                    Outcome::Home
                } else if roll < p[0] + p[1] {
                    Outcome::Draw
                } else {
                    Outcome::Away
                };
                let sharp = odds(p, 0.02);
                let jitter = |rng: &mut Lcg| 1.0 + lag * (2.0 * rng.next() - 1.0);
                let soft = Odds1x2 {
                    home: Odds::new(sharp.home.get() * jitter(&mut rng)).unwrap(),
                    draw: Odds::new(sharp.draw.get() * jitter(&mut rng)).unwrap(),
                    away: Odds::new(sharp.away.get() * jitter(&mut rng)).unwrap(),
                };
                let mut prices = Map::new();
                prices.insert(("PS".to_string(), Moment::Pre), sharp);
                prices.insert(("PS".to_string(), Moment::Close), sharp);
                prices.insert(("B365".to_string(), Moment::Pre), soft);
                GameInputs {
                    game_id: i as i64 + 1,
                    date: start + Days::new(3 * i as u64),
                    block: (i / 4) as u32,
                    result,
                    odds: prices,
                }
            })
            .collect()
    }

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn a_laggard_bookmaker_gives_a_positive_clv_and_a_threshold_is_frozen() {
        // 6 000 matchs répartis sur toutes les périodes ; la vérité est Pinnacle, Bet365 le bruite de ± 8 %.
        let all = world(6000, 0.08, 1);
        let conn = database();

        let rows = tune(
            &conn,
            &all,
            &HashMap::new(),
            MarginMethod::Multiplicative,
            Utc::now(),
        )
        .unwrap();

        let b365 = rows.iter().find(|r| r.source == "soft:B365").unwrap();
        assert!(b365.chosen.is_some());
        let best = b365
            .results
            .iter()
            .find(|r| Some(r.threshold) == b365.chosen)
            .unwrap();
        assert!(best.clv.unwrap() > 0.0, "{best:?}");
        // Le modèle n'a pas de prédictions : aucune ligne pour lui ; les bookmakers absents n'ont aucun pari.
        assert!(rows.iter().all(|r| r.source != "model:dixon_coles"));
        let others = rows.iter().find(|r| r.source == "soft:WH").unwrap();
        assert!(others.chosen.is_none(), "sans cotes, aucun seuil éligible");
        // Un seul gel enregistré, avec le nombre total de configurations essayées (8 sources × 6 seuils).
        let frozen = backtests::latest_selection(&conn, "soft:B365")
            .unwrap()
            .unwrap();
        assert_eq!(frozen.configs_tried, 8 * 6);
        assert_eq!(backtests::selection_session_count(&conn).unwrap(), 1);
    }

    #[test]
    fn a_higher_threshold_selects_fewer_bets() {
        let all = world(6000, 0.08, 2);
        let conn = database();
        let rows = tune(
            &conn,
            &all,
            &HashMap::new(),
            MarginMethod::Multiplicative,
            Utc::now(),
        )
        .unwrap();
        let b365 = rows.iter().find(|r| r.source == "soft:B365").unwrap();
        let counts: Vec<usize> = b365.results.iter().map(|r| r.bets).collect();
        assert!(counts.windows(2).all(|w| w[0] >= w[1]), "{counts:?}");
    }

    #[test]
    fn the_study_uses_the_frozen_threshold_and_reports_each_period_and_each_kelly_fraction() {
        let all = world(6000, 0.08, 3);
        let conn = database();
        tune(
            &conn,
            &all,
            &HashMap::new(),
            MarginMethod::Multiplicative,
            Utc::now(),
        )
        .unwrap();

        let studies =
            study_all(&conn, &all, &HashMap::new(), MarginMethod::Multiplicative).unwrap();

        assert_eq!(studies.len(), 1);
        let study = &studies[0];
        assert_eq!(study.label, "soft:B365");
        assert_eq!(study.periods.len(), 3);
        assert_eq!(study.staking.len(), 3);
        assert_eq!(study.staking[1].kelly_fraction, 0.5);
        assert_eq!(study.price_sensitivity.len(), 3);
        // Une décote de prix de 9 % détruit l'avantage d'un bookmaker qui ne se trompe que de ± 8 %.
        let (_, _, undiscounted_clv, _) = study.price_sensitivity[0];
        let (_, _, discounted_clv, _) = study.price_sensitivity[2];
        assert!(discounted_clv.unwrap_or(-1.0) < undiscounted_clv.unwrap() - 0.05);
        // Les paris du test se jouent à la fréquence annoncée, car dans ce monde Pinnacle dit vrai.
        let calibration = study.calibration.as_ref().unwrap();
        assert!(
            (calibration.realized - calibration.announced).abs() < 0.06,
            "{calibration:?}"
        );
    }

    #[test]
    fn where_the_edge_is_real_the_realised_return_follows_the_announced_one() {
        // Dans ce monde, l'espérance annoncée (contre Pinnacle, qui dit vrai) est réelle : le rendement réalisé
        // doit croître avec elle, et rester négatif quand elle est négative.
        let all = world(8000, 0.10, 4);
        let games: Vec<&GameInputs> = all.iter().collect();
        let generator = Generator {
            min_edge: f64::NEG_INFINITY,
            price_factor: 1.0,
            method: MarginMethod::Multiplicative,
        };
        let candidates = soft_vs_sharp(&games, "B365", &generator);

        let buckets = ev_buckets(&candidates);

        assert!(buckets.len() >= 4);
        assert_eq!(buckets.iter().map(|b| b.n).sum::<usize>(), candidates.len());
        let top = buckets.last().unwrap();
        assert!(top.realized > buckets[0].realized, "{buckets:?}");
        assert!(top.realized > 0.0, "{top:?}");
    }

    #[test]
    fn evaluation_refuses_to_run_without_a_frozen_threshold() {
        let all = world(500, 0.05, 5);
        let err = study_all(
            &database(),
            &all,
            &HashMap::new(),
            MarginMethod::Multiplicative,
        )
        .unwrap_err();
        assert!(err.to_string().contains("edge backtest tune"));
    }

    #[test]
    fn test_runs_are_journaled_once_per_source_and_fraction() {
        let all = world(6000, 0.08, 6);
        let mut conn = database();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        tune(
            &conn,
            &all,
            &HashMap::new(),
            MarginMethod::Multiplicative,
            Utc::now(),
        )
        .unwrap();

        let first = persist_test_runs(
            &mut conn,
            &all,
            &HashMap::new(),
            MarginMethod::Multiplicative,
            Utc::now(),
        )
        .unwrap();
        let second = persist_test_runs(
            &mut conn,
            &all,
            &HashMap::new(),
            MarginMethod::Multiplicative,
            Utc::now(),
        )
        .unwrap();

        assert_eq!(first, 3, "une source gelée × trois fractions de Kelly");
        assert_eq!(second, 0, "relancer n'ajoute rien");
    }

    #[test]
    fn the_verdict_depends_on_the_test_sample_and_the_clv_interval() {
        let row = |bets: usize, low: f64, high: f64| PeriodRow {
            period: Period::Test,
            bets,
            clv: Some(MeanCi {
                mean: (low + high) / 2.0,
                low,
                high,
            }),
            roi: None,
        };
        assert_eq!(verdict(None), Verdict::NoPower);
        assert_eq!(verdict(Some(&row(8, -0.05, 0.05))), Verdict::NoPower);
        assert_eq!(verdict(Some(&row(200, -0.08, -0.07))), Verdict::Rejected);
        assert_eq!(verdict(Some(&row(200, 0.01, 0.03))), Verdict::Positive);
        assert_eq!(verdict(Some(&row(200, -0.01, 0.03))), Verdict::Inconclusive);
    }

    #[test]
    fn ev_buckets_follow_a_hand_calculation() {
        use crate::domain::Probability;
        let candidate = |p: f64, odds: f64, won: bool| Candidate {
            game_id: 1,
            date: NaiveDate::from_ymd_opt(2025, 8, 16).unwrap(),
            block: 0,
            outcome: Outcome::Home,
            odds: Odds::new(odds).unwrap(),
            probability: Probability::new(p).unwrap(),
            close_fair: Probability::new(p),
            won,
        };
        // EV annoncée +3 % (p = 0,5, cote 2,06) deux fois : une gagnée, une perdue → rendement réalisé +3 %.
        let rows = ev_buckets(&[candidate(0.5, 2.06, true), candidate(0.5, 2.06, false)]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "2 % à 5 %");
        assert!((rows[0].announced - 0.03).abs() < 1e-12);
        assert!((rows[0].realized - 0.03).abs() < 1e-12);
    }
}
