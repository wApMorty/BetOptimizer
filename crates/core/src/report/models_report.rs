//! Chiffres du rapport « modèles » : comment Poisson et Dixon-Coles se comparent au marché sur le **test**.
//!
//! Le test (2025/26, puis 2026/27 en avant) n'a servi ni à choisir un hyperparamètre ni à régler un seuil : les
//! configurations viennent des sessions de réglage figées sur la validation 2024/25. Toutes les comparaisons se
//! font sur exactement les mêmes matchs, avec des intervalles de confiance par blocs de journées.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;
use serde::Serialize;

use super::svg::reliability_chart;
use crate::domain::{Moment, Outcome, Probability, Probs1x2};
use crate::error::{Error, Result};
use crate::eval::bootstrap::{MeanCi, block_bootstrap_mean};
use crate::eval::metrics::{brier, brier_binary, log_loss, log_loss_binary, rps};
use crate::ingest::football_data::LEAGUES;
use crate::market::calibration::reliability;
use crate::market::dataset::{self, MarketGame};
use crate::market::margin::{MarginMethod, remove_margin, remove_margin_two_way};
use crate::models::MatchModel;
use crate::models::data::{LeagueData, training_games};
use crate::models::dixon_coles::DixonColes;
use crate::models::tuning::{self, candidate_grid, validation_window};
use crate::models::walk_forward::{ModelConfig, ModelKind, Prediction, fit_proxy};
use crate::models::xg_proxy::{ShotObservation, XgProxy, agreement};
use crate::store::predictions;

const RESAMPLES: usize = 2000;
const SEED: u64 = 20_261_019;

/// Une configuration figée sur la validation.
#[derive(Debug, Clone, Serialize)]
pub struct FrozenRow {
    /// Étiquette (`poisson`, `dixon_coles`, `dixon_coles_goals`).
    pub label: String,
    /// Décroissance `ξ` par jour.
    pub decay: f64,
    /// Demi-vie correspondante, en jours : `ln 2 / ξ`.
    pub half_life_days: f64,
    /// Régularisation `κ`.
    pub ridge: f64,
    /// Mélange de buts attendus issus des tirs.
    pub blend: f64,
    /// Log loss sur la validation.
    pub validation_log_loss: f64,
}

/// Une configuration de la grille avec son log loss de validation.
#[derive(Debug, Clone, Serialize)]
pub struct GridRow {
    /// Famille.
    pub model: &'static str,
    /// `ξ`.
    pub decay: f64,
    /// `κ`.
    pub ridge: f64,
    /// Mélange de tirs.
    pub blend: f64,
    /// Log loss de validation.
    pub log_loss: f64,
}

/// Un pronostiqueur évalué sur le 1X2.
#[derive(Debug, Clone, Serialize)]
pub struct ForecastRow {
    /// Libellé.
    pub label: String,
    /// Matchs communs.
    pub n: usize,
    /// Log loss moyen.
    pub log_loss: MeanCi,
    /// Brier moyen.
    pub brier: MeanCi,
    /// RPS moyen.
    pub rps: MeanCi,
    /// Différence de log loss avec la référence (positif = moins bon que la référence).
    pub log_loss_vs_reference: Option<MeanCi>,
    /// Différence de RPS avec la référence.
    pub rps_vs_reference: Option<MeanCi>,
}

/// Écart du modèle au marché, par ligue.
#[derive(Debug, Clone, Serialize)]
pub struct LeagueRow {
    /// Nom de la ligue.
    pub name: String,
    /// Matchs.
    pub games: usize,
    /// Δ log loss Dixon-Coles − moyenne du marché (clôture).
    pub log_loss_vs_market: MeanCi,
}

/// Un pronostiqueur évalué sur un événement binaire.
#[derive(Debug, Clone, Serialize)]
pub struct BinaryRow {
    /// Libellé.
    pub label: String,
    /// Matchs.
    pub n: usize,
    /// Log loss moyen.
    pub log_loss: MeanCi,
    /// Brier moyen.
    pub brier: MeanCi,
    /// Différence de log loss avec la référence.
    pub log_loss_vs_reference: Option<MeanCi>,
}

/// Force d'une équipe.
#[derive(Debug, Clone, Serialize)]
pub struct TeamRating {
    /// Nom canonique.
    pub name: String,
    /// Force offensive (log, 0 = moyenne).
    pub attack: f64,
    /// Faiblesse défensive (log, 0 = moyenne ; positif = encaisse plus que la moyenne).
    pub defence: f64,
    /// Force nette : `attaque − défense`.
    pub net: f64,
}

/// Forces des équipes d'une ligue.
#[derive(Debug, Clone, Serialize)]
pub struct LeagueRatings {
    /// Nom de la ligue.
    pub name: String,
    /// Équipes ayant joué dans les 12 derniers mois, de la plus forte à la plus faible.
    pub teams: Vec<TeamRating>,
}

/// Contrôle du proxy de buts attendus contre les xG publiés par football-data (2026/27).
#[derive(Debug, Clone, Serialize)]
pub struct ProxyRow {
    /// Nom de la ligue.
    pub league: String,
    /// Buts par tir cadré.
    pub on_target: f64,
    /// Buts par tir non cadré.
    pub off_target: f64,
    /// Équipes-matchs avec xG publié.
    pub pairs: usize,
    /// Corrélation de Pearson entre le proxy et le xG publié.
    pub correlation: Option<f64>,
    /// Rapport des moyennes proxy / publié.
    pub ratio: Option<f64>,
}

/// Tout ce que le rapport affiche.
#[derive(Debug, Clone, Serialize)]
pub struct ModelsReport {
    /// Date de génération.
    pub generated_on: String,
    /// Validation (début, fin).
    pub validation: (String, String),
    /// Test (début, fin).
    pub test: (String, String),
    /// Sessions de réglage enregistrées.
    pub tuning_sessions: u32,
    /// Configurations essayées pendant la dernière session.
    pub configs_tried: usize,
    /// Configurations figées.
    pub frozen: Vec<FrozenRow>,
    /// Grille complète, du meilleur au pire log loss de validation.
    pub grid: Vec<GridRow>,
    /// Meilleur log loss de validation de Dixon-Coles selon le mélange de tirs.
    pub blend_effect: Vec<(f64, f64)>,
    /// Matchs du test prédits par les trois configurations.
    pub test_games: usize,
    /// Comparaison sur les matchs de l'échantillon commun (référence : moyenne du marché, clôture).
    pub comparison: Vec<ForecastRow>,
    /// Comparaison sur le sous-ensemble ayant une clôture Pinnacle (référence : Pinnacle).
    pub comparison_pinnacle: Vec<ForecastRow>,
    /// Matchs du sous-ensemble Pinnacle.
    pub pinnacle_games: usize,
    /// Test d'information, référence clôture : le modèle ajoute-t-il quelque chose à la clôture du marché ?
    pub blend_close: Vec<ForecastRow>,
    /// Poids du modèle dans la combinaison avec la clôture (réglé sur la validation).
    pub blend_weight_close: f64,
    /// Test d'information, référence avant-match : le modèle améliore-t-il les cotes du vendredi ?
    pub blend_pre: Vec<ForecastRow>,
    /// Poids du modèle dans la combinaison avec l'avant-match (réglé sur la validation).
    pub blend_weight_pre: f64,
    /// Écart au marché par ligue.
    pub by_league: Vec<LeagueRow>,
    /// Plus/moins 2,5 buts.
    pub totals: Vec<BinaryRow>,
    /// Les deux équipes marquent (sans cote de marché : calibration et log loss contre la fréquence passée).
    pub btts: Vec<BinaryRow>,
    /// Diagramme de fiabilité du 1X2 de Dixon-Coles.
    pub calibration_svg: String,
    /// Diagramme de fiabilité de « les deux marquent ».
    pub btts_calibration_svg: String,
    /// Forces d'équipe actuelles.
    pub ratings: Vec<LeagueRatings>,
    /// Contrôle du proxy de buts attendus.
    pub proxy: Vec<ProxyRow>,
    /// Prédictions où ρ a été ramené dans le domaine faisable (test).
    pub rho_clamped: usize,
}

/// Informations d'un match du test.
struct TestGame {
    block: u32,
    div: String,
    outcome: Outcome,
}

/// Intervalle de confiance d'une série `(bloc, valeur)`.
fn ci(values: Vec<(u32, f64)>) -> Result<MeanCi> {
    block_bootstrap_mean(&values, RESAMPLES, SEED)
        .ok_or_else(|| Error::Data("échantillon vide".to_string()))
}

/// Un pronostiqueur : ses probabilités 1X2 par identifiant de match.
struct Forecaster {
    label: String,
    forecasts: HashMap<i64, Probs1x2>,
}

fn forecast_rows(
    forecasters: &[Forecaster],
    games: &HashMap<i64, TestGame>,
    ids: &[i64],
    reference: usize,
) -> Result<Vec<ForecastRow>> {
    let metric = |f: &Forecaster, id: i64| -> Option<(u32, f64, f64, f64)> {
        let game = games.get(&id)?;
        let forecast = f.forecasts.get(&id)?;
        Some((
            game.block,
            log_loss(forecast, game.outcome),
            brier(forecast, game.outcome),
            rps(forecast, game.outcome),
        ))
    };
    let reference_scores: HashMap<i64, (u32, f64, f64, f64)> = ids
        .iter()
        .filter_map(|id| metric(&forecasters[reference], *id).map(|m| (*id, m)))
        .collect();
    let mut rows = Vec::new();
    for (index, forecaster) in forecasters.iter().enumerate() {
        let scored: Vec<(i64, (u32, f64, f64, f64))> = ids
            .iter()
            .filter_map(|id| metric(forecaster, *id).map(|m| (*id, m)))
            .collect();
        let (vs_ll, vs_rps) = if index == reference {
            (None, None)
        } else {
            let diff = |pick: fn(&(u32, f64, f64, f64)) -> f64| -> Result<MeanCi> {
                ci(scored
                    .iter()
                    .filter_map(|(id, m)| {
                        reference_scores.get(id).map(|r| (m.0, pick(m) - pick(r)))
                    })
                    .collect())
            };
            (Some(diff(|m| m.1)?), Some(diff(|m| m.3)?))
        };
        rows.push(ForecastRow {
            label: forecaster.label.clone(),
            n: scored.len(),
            log_loss: ci(scored.iter().map(|(_, m)| (m.0, m.1)).collect())?,
            brier: ci(scored.iter().map(|(_, m)| (m.0, m.2)).collect())?,
            rps: ci(scored.iter().map(|(_, m)| (m.0, m.3)).collect())?,
            log_loss_vs_reference: vs_ll,
            rps_vs_reference: vs_rps,
        });
    }
    Ok(rows)
}

/// Probabilités du marché pour un jeu de cotes 1X2.
fn market_forecasts(games: &[MarketGame], method: MarginMethod) -> HashMap<i64, Probs1x2> {
    games
        .iter()
        .filter_map(|game| remove_margin(&game.odds, method).map(|p| (game.game_id, p)))
        .collect()
}

fn model_forecasts(predictions: &[Prediction]) -> HashMap<i64, Probs1x2> {
    predictions
        .iter()
        .filter_map(|p| Probs1x2::new(p.p_home, p.p_draw, p.p_away).map(|probs| (p.game_id, probs)))
        .collect()
}

/// Fréquences passées de chaque résultat par ligue (strictement avant `before`) : le pronostiqueur le plus naïf.
fn climatology(
    leagues: &[LeagueData],
    before: NaiveDate,
    ids: &HashSet<i64>,
) -> HashMap<i64, Probs1x2> {
    let mut forecasts = HashMap::new();
    for league in leagues {
        let (mut home, mut draw, mut away) = (0.0, 0.0, 0.0);
        for game in league.games.iter().filter(|g| g.date < before) {
            match game.goals_home.cmp(&game.goals_away) {
                std::cmp::Ordering::Greater => home += 1.0,
                std::cmp::Ordering::Equal => draw += 1.0,
                std::cmp::Ordering::Less => away += 1.0,
            }
        }
        let total = home + draw + away;
        if total == 0.0 {
            continue;
        }
        let Some(probs) = Probs1x2::new(home / total, draw / total, away / total) else {
            continue;
        };
        for game in league.games.iter().filter(|g| ids.contains(&g.game_id)) {
            forecasts.insert(game.game_id, probs);
        }
    }
    forecasts
}

/// Construit le rapport. `test_last` est la dernière date du test (en pratique, le dernier match en base).
pub fn build(
    conn: &Connection,
    leagues: &[LeagueData],
    method: MarginMethod,
    test_last: NaiveDate,
    generated_on: &str,
) -> Result<ModelsReport> {
    let validation = validation_window();
    let test = tuning::test_window(test_last);

    // --- Réglage : configurations figées et grille de validation ---------------------------------------------
    let outcomes = tuning::outcomes(leagues);
    let mut frozen = Vec::new();
    let mut frozen_configs: Vec<(&'static str, ModelConfig)> = Vec::new();
    let mut configs_tried = 0;
    for label in ["poisson", "dixon_coles", "dixon_coles_goals"] {
        let record = predictions::latest_tuning(conn, label)?.ok_or_else(|| {
            Error::Data("aucun réglage figé : lance `edge model tune`".to_string())
        })?;
        configs_tried = record.configs_tried;
        frozen.push(FrozenRow {
            label: label.to_string(),
            decay: record.config.decay,
            half_life_days: std::f64::consts::LN_2 / record.config.decay,
            ridge: record.config.ridge,
            blend: record.config.blend,
            validation_log_loss: record.validation_log_loss,
        });
        frozen_configs.push((label, record.config));
    }
    let mut grid = Vec::new();
    for config in candidate_grid() {
        if let Some(run_id) = predictions::find_run(conn, &config, validation)?
            && let Some(loss) =
                tuning::mean_log_loss(&predictions::load_predictions(conn, run_id)?, &outcomes)
        {
            grid.push(GridRow {
                model: config.model_name(),
                decay: config.decay,
                ridge: config.ridge,
                blend: config.blend,
                log_loss: loss,
            });
        }
    }
    grid.sort_by(|a, b| a.log_loss.total_cmp(&b.log_loss));
    let mut blend_effect: Vec<(f64, f64)> = Vec::new();
    for blend in [0.0, 0.5, 1.0] {
        if let Some(best) = grid
            .iter()
            .find(|row| row.model == "dixon_coles" && row.blend == blend)
        {
            blend_effect.push((blend, best.log_loss));
        }
    }

    // --- Prédictions du test ------------------------------------------------------------------------------------
    let mut model_predictions: HashMap<&'static str, Vec<Prediction>> = HashMap::new();
    let mut rho_clamped = 0;
    for (label, config) in &frozen_configs {
        let run_id = predictions::find_run(conn, config, test)?.ok_or_else(|| {
            Error::Data("pas d'exécution de test : lance `edge model evaluate`".to_string())
        })?;
        model_predictions.insert(label, predictions::load_predictions(conn, run_id)?);
    }
    for (_, config) in &frozen_configs {
        if let Some(run_id) = predictions::find_run(conn, config, test)? {
            rho_clamped = rho_clamped.max(conn.query_row(
                "SELECT rho_clamped FROM model_run WHERE id = ?1",
                [run_id],
                |row| row.get::<_, i64>(0),
            )? as usize);
        }
    }

    // Blocs de bootstrap : une ligue et une semaine.
    let mut blocks: HashMap<(String, i32), u32> = HashMap::new();
    let mut games: HashMap<i64, TestGame> = HashMap::new();
    for league in leagues {
        for game in league
            .games
            .iter()
            .filter(|g| g.date >= test.first && g.date <= test.last)
        {
            let week = (game.date.num_days_from_ce() - 1) / 7;
            let next = blocks.len() as u32;
            let block = *blocks.entry((league.div.clone(), week)).or_insert(next);
            games.insert(
                game.game_id,
                TestGame {
                    block,
                    div: league.div.clone(),
                    outcome: outcomes[&game.game_id],
                },
            );
        }
    }

    // --- 1X2 : modèles contre marché ---------------------------------------------------------------------------
    let avg_close = market_forecasts(&dataset::load(conn, "Avg", Moment::Close)?, method);
    let avg_pre = market_forecasts(&dataset::load(conn, "Avg", Moment::Pre)?, method);
    let avg_pre_for_blend = avg_pre.clone();
    let ps_close = market_forecasts(&dataset::load(conn, "PS", Moment::Close)?, method);
    let poisson = model_forecasts(&model_predictions["poisson"]);
    let dixon_coles = model_forecasts(&model_predictions["dixon_coles"]);
    let goals_only = model_forecasts(&model_predictions["dixon_coles_goals"]);

    // Échantillon commun : matchs du test prédits par les modèles et cotés par la moyenne du marché.
    let ids: Vec<i64> = {
        let mut ids: Vec<i64> = games
            .keys()
            .copied()
            .filter(|id| {
                avg_close.contains_key(id)
                    && avg_pre.contains_key(id)
                    && poisson.contains_key(id)
                    && dixon_coles.contains_key(id)
                    && goals_only.contains_key(id)
            })
            .collect();
        ids.sort_unstable();
        ids
    };
    let id_set: HashSet<i64> = ids.iter().copied().collect();
    let baseline = climatology(leagues, test.first, &id_set);

    let forecasters = vec![
        Forecaster {
            label: "Moyenne du marché, clôture (référence)".into(),
            forecasts: avg_close.clone(),
        },
        Forecaster {
            label: "Moyenne du marché, avant-match".into(),
            forecasts: avg_pre,
        },
        Forecaster {
            label: "Dixon-Coles (réglage figé)".into(),
            forecasts: dixon_coles.clone(),
        },
        Forecaster {
            label: "Dixon-Coles à buts seuls".into(),
            forecasts: goals_only,
        },
        Forecaster {
            label: "Poisson".into(),
            forecasts: poisson,
        },
        Forecaster {
            label: "Fréquences passées de la ligue".into(),
            forecasts: baseline,
        },
    ];
    let comparison = forecast_rows(&forecasters, &games, &ids, 0)?;

    // Sous-ensemble avec Pinnacle : référence Pinnacle (clôture).
    let pinnacle_ids: Vec<i64> = ids
        .iter()
        .copied()
        .filter(|id| ps_close.contains_key(id))
        .collect();
    let comparison_pinnacle = if pinnacle_ids.is_empty() {
        Vec::new()
    } else {
        let mut set = vec![Forecaster {
            label: "Pinnacle, clôture (référence)".into(),
            forecasts: ps_close,
        }];
        set.push(Forecaster {
            label: "Moyenne du marché, clôture".into(),
            forecasts: forecasters[0].forecasts.clone(),
        });
        set.extend(forecasters.into_iter().skip(2).take(2));
        forecast_rows(&set, &games, &pinnacle_ids, 0)?
    };

    // --- Test d'information : combinaison géométrique marché / modèle ------------------------------------------
    // p ∝ p_marché^(1−w) · p_modèle^w. Le poids w est choisi sur la VALIDATION puis évalué sur le TEST : si le
    // modèle n'apportait aucune information, le meilleur w serait 0 et la combinaison ne battrait pas le marché.
    let dc_config_for_blend = frozen_configs
        .iter()
        .find(|(label, _)| *label == "dixon_coles")
        .map(|(_, config)| *config);
    let validation_dixon_coles = match dc_config_for_blend {
        Some(config) => match predictions::find_run(conn, &config, validation)? {
            Some(run_id) => model_forecasts(&predictions::load_predictions(conn, run_id)?),
            None => HashMap::new(),
        },
        None => HashMap::new(),
    };
    let validation_ids = |market: &HashMap<i64, Probs1x2>| -> Vec<i64> {
        let mut ids: Vec<i64> = validation_dixon_coles
            .keys()
            .copied()
            .filter(|id| market.contains_key(id))
            .collect();
        ids.sort_unstable();
        ids
    };
    let weight_close = best_weight(
        &avg_close,
        &validation_dixon_coles,
        &outcomes,
        &validation_ids(&avg_close),
    );
    let weight_pre = best_weight(
        &avg_pre_for_blend,
        &validation_dixon_coles,
        &outcomes,
        &validation_ids(&avg_pre_for_blend),
    );
    let blended_close = blend_forecasts(&avg_close, &dixon_coles, weight_close);
    let blended_pre = blend_forecasts(&avg_pre_for_blend, &dixon_coles, weight_pre);
    let blend_close = forecast_rows(
        &[
            Forecaster {
                label: "Moyenne du marché, clôture seule (référence)".into(),
                forecasts: avg_close.clone(),
            },
            Forecaster {
                label: format!(
                    "Clôture + modèle (poids du modèle {})",
                    weight_label(weight_close)
                ),
                forecasts: blended_close.clone(),
            },
        ],
        &games,
        &ids,
        0,
    )?;
    let blend_pre = forecast_rows(
        &[
            Forecaster {
                label: "Moyenne du marché, avant-match seul (référence)".into(),
                forecasts: avg_pre_for_blend.clone(),
            },
            Forecaster {
                label: format!(
                    "Avant-match + modèle (poids du modèle {})",
                    weight_label(weight_pre)
                ),
                forecasts: blended_pre,
            },
            Forecaster {
                label: "Moyenne du marché, clôture seule".into(),
                forecasts: avg_close.clone(),
            },
        ],
        &games,
        &ids,
        0,
    )?;

    // --- Écart au marché par ligue --------------------------------------------------------------------------------
    let mut by_league = Vec::new();
    for league in LEAGUES {
        let league_ids: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|id| games[id].div == league.div)
            .collect();
        if league_ids.is_empty() {
            continue;
        }
        let diffs: Vec<(u32, f64)> = league_ids
            .iter()
            .filter_map(|id| {
                let game = games.get(id)?;
                let model = log_loss(dixon_coles.get(id)?, game.outcome);
                let market = log_loss(avg_close.get(id)?, game.outcome);
                Some((game.block, model - market))
            })
            .collect();
        by_league.push(LeagueRow {
            name: league.name.to_string(),
            games: league_ids.len(),
            log_loss_vs_market: ci(diffs)?,
        });
    }

    // --- Plus/moins 2,5 et « les deux marquent » -----------------------------------------------------------------
    let over_actual: HashMap<i64, bool> = leagues
        .iter()
        .flat_map(|l| l.games.iter())
        .map(|g| {
            (
                g.game_id,
                u16::from(g.goals_home) + u16::from(g.goals_away) >= 3,
            )
        })
        .collect();
    let btts_actual: HashMap<i64, bool> = leagues
        .iter()
        .flat_map(|l| l.games.iter())
        .map(|g| (g.game_id, g.goals_home > 0 && g.goals_away > 0))
        .collect();
    let totals_market = |bookmaker: &str, moment: Moment| -> Result<HashMap<i64, Probability>> {
        Ok(dataset::load_totals(conn, bookmaker, moment)?
            .into_iter()
            .filter_map(|t| remove_margin_two_way(t.over, t.under, method).map(|p| (t.game_id, p)))
            .collect())
    };
    let totals_avg_close = totals_market("Avg", Moment::Close)?;
    let totals_avg_pre = totals_market("Avg", Moment::Pre)?;
    let model_over = |name: &str| -> HashMap<i64, Probability> {
        model_predictions[name]
            .iter()
            .filter_map(|p| Probability::new(p.p_over25).map(|q| (p.game_id, q)))
            .collect()
    };
    let past_rate = |actual: &HashMap<i64, bool>, before: NaiveDate| -> Option<f64> {
        let past: Vec<bool> = leagues
            .iter()
            .flat_map(|l| l.games.iter())
            .filter(|g| g.date < before)
            .filter_map(|g| actual.get(&g.game_id).copied())
            .collect();
        (!past.is_empty()).then(|| past.iter().filter(|b| **b).count() as f64 / past.len() as f64)
    };
    let over_rate = past_rate(&over_actual, test.first).and_then(Probability::new);
    let totals_forecasters: Vec<(String, HashMap<i64, Probability>)> = vec![
        (
            "Moyenne du marché, clôture (référence)".to_string(),
            totals_avg_close,
        ),
        ("Moyenne du marché, avant-match".to_string(), totals_avg_pre),
        (
            "Dixon-Coles (réglage figé)".to_string(),
            model_over("dixon_coles"),
        ),
        (
            "Dixon-Coles à buts seuls".to_string(),
            model_over("dixon_coles_goals"),
        ),
        ("Poisson".to_string(), model_over("poisson")),
        (
            "Fréquence passée de plus de 2,5 buts".to_string(),
            over_rate
                .map(|p| games.keys().map(|id| (*id, p)).collect())
                .unwrap_or_default(),
        ),
    ];
    let totals_ids: Vec<i64> = {
        let mut ids: Vec<i64> = games
            .keys()
            .copied()
            .filter(|id| {
                totals_forecasters
                    .iter()
                    .take(5)
                    .all(|(_, f)| f.contains_key(id))
            })
            .collect();
        ids.sort_unstable();
        ids
    };
    let totals = binary_rows(&totals_forecasters, &games, &over_actual, &totals_ids)?;

    let btts_rate = past_rate(&btts_actual, test.first).and_then(Probability::new);
    let model_btts = |name: &str| -> HashMap<i64, Probability> {
        model_predictions[name]
            .iter()
            .filter_map(|p| Probability::new(p.p_btts).map(|q| (p.game_id, q)))
            .collect()
    };
    let btts_forecasters: Vec<(String, HashMap<i64, Probability>)> = vec![
        (
            "Fréquence passée des deux équipes marquent (référence)".to_string(),
            btts_rate
                .map(|p| games.keys().map(|id| (*id, p)).collect())
                .unwrap_or_default(),
        ),
        (
            "Dixon-Coles (réglage figé)".to_string(),
            model_btts("dixon_coles"),
        ),
        (
            "Dixon-Coles à buts seuls".to_string(),
            model_btts("dixon_coles_goals"),
        ),
        ("Poisson".to_string(), model_btts("poisson")),
    ];
    let btts_ids: Vec<i64> = {
        let mut ids: Vec<i64> = games
            .keys()
            .copied()
            .filter(|id| btts_forecasters.iter().all(|(_, f)| f.contains_key(id)))
            .collect();
        ids.sort_unstable();
        ids
    };
    let btts = binary_rows(&btts_forecasters, &games, &btts_actual, &btts_ids)?;

    // --- Calibration du Dixon-Coles --------------------------------------------------------------------------------
    let calibration_pairs: Vec<(f64, bool)> = ids
        .iter()
        .filter_map(|id| {
            let forecast = dixon_coles.get(id)?;
            let outcome = games.get(id)?.outcome;
            Some(Outcome::ALL.map(|o| (forecast.get(o).get(), o == outcome)))
        })
        .flatten()
        .collect();
    let calibration_svg = reliability_chart(
        &reliability(&calibration_pairs, 0.05),
        "Dixon-Coles, 1X2 (test)",
    )?;
    let btts_pairs: Vec<(f64, bool)> = btts_ids
        .iter()
        .filter_map(|id| Some((model_btts("dixon_coles").get(id)?.get(), btts_actual[id])))
        .collect();
    let btts_calibration_svg = reliability_chart(
        &reliability(&btts_pairs, 0.05),
        "Dixon-Coles, les deux marquent (test)",
    )?;

    // --- Forces d'équipe actuelles et contrôle du proxy -------------------------------------------------------------
    let dc_config = frozen_configs
        .iter()
        .find(|(label, _)| *label == "dixon_coles")
        .map(|(_, c)| *c)
        .ok_or_else(|| Error::Data("réglage manquant".into()))?;
    let (ratings, proxy) = ratings_and_proxy(leagues, dc_config, test_last)?;

    Ok(ModelsReport {
        generated_on: generated_on.to_string(),
        validation: (validation.first.to_string(), validation.last.to_string()),
        test: (test.first.to_string(), test.last.to_string()),
        tuning_sessions: predictions::tuning_session_count(conn)?,
        configs_tried,
        frozen,
        grid,
        blend_effect,
        test_games: model_predictions["dixon_coles"].len(),
        comparison,
        comparison_pinnacle,
        pinnacle_games: pinnacle_ids.len(),
        blend_close,
        blend_weight_close: weight_close,
        blend_pre,
        blend_weight_pre: weight_pre,
        by_league,
        totals,
        btts,
        calibration_svg,
        btts_calibration_svg,
        ratings,
        proxy,
        rho_clamped,
    })
}

/// Combinaison géométrique de deux prévisions : `q_i ∝ a_i^(1−w) · b_i^w`, renormalisée. `w = 0` redonne `a`, `w = 1` donne `b`.
///
/// C'est la forme classique (« log-linéaire ») pour mélanger deux pronostiqueurs : elle revient à une régression
/// logistique multinomiale sur les logarithmes des probabilités, avec un seul poids à régler.
pub(crate) fn geometric_blend(a: &Probs1x2, b: &Probs1x2, weight: f64) -> Option<Probs1x2> {
    let (pa, pb) = (a.to_array(), b.to_array());
    let raw: Vec<f64> = (0..3)
        .map(|i| pa[i].powf(1.0 - weight) * pb[i].powf(weight))
        .collect();
    let total: f64 = raw.iter().sum();
    Probs1x2::new(raw[0] / total, raw[1] / total, raw[2] / total)
}

fn blend_forecasts(
    market: &HashMap<i64, Probs1x2>,
    model: &HashMap<i64, Probs1x2>,
    weight: f64,
) -> HashMap<i64, Probs1x2> {
    market
        .iter()
        .filter_map(|(id, a)| geometric_blend(a, model.get(id)?, weight).map(|q| (*id, q)))
        .collect()
}

/// Poids du modèle (0 à 1 par pas de 0,05) qui minimise le log loss de la combinaison sur `ids`.
pub(crate) fn best_weight(
    market: &HashMap<i64, Probs1x2>,
    model: &HashMap<i64, Probs1x2>,
    outcomes: &HashMap<i64, Outcome>,
    ids: &[i64],
) -> f64 {
    let mean_loss = |weight: f64| -> f64 {
        let mut total = 0.0;
        let mut count = 0usize;
        for id in ids {
            let (Some(a), Some(b), Some(outcome)) =
                (market.get(id), model.get(id), outcomes.get(id))
            else {
                continue;
            };
            if let Some(q) = geometric_blend(a, b, weight) {
                total += log_loss(&q, *outcome);
                count += 1;
            }
        }
        if count == 0 {
            f64::INFINITY
        } else {
            total / count as f64
        }
    };
    (0..=20)
        .map(|step| f64::from(step) * 0.05)
        .min_by(|a, b| mean_loss(*a).total_cmp(&mean_loss(*b)))
        .unwrap_or(0.0)
}

fn weight_label(weight: f64) -> String {
    format!("{weight:.2}").replace('.', ",")
}

/// Lignes d'un événement binaire ; la référence est la première série.
fn binary_rows(
    series: &[(String, HashMap<i64, Probability>)],
    games: &HashMap<i64, TestGame>,
    actual: &HashMap<i64, bool>,
    ids: &[i64],
) -> Result<Vec<BinaryRow>> {
    let scores = |forecasts: &HashMap<i64, Probability>| -> Vec<(i64, u32, f64, f64)> {
        ids.iter()
            .filter_map(|id| {
                let p = *forecasts.get(id)?;
                let happened = *actual.get(id)?;
                Some((
                    *id,
                    games.get(id)?.block,
                    log_loss_binary(p, happened),
                    brier_binary(p, happened),
                ))
            })
            .collect()
    };
    let reference: HashMap<i64, f64> = scores(&series[0].1)
        .into_iter()
        .map(|(id, _, ll, _)| (id, ll))
        .collect();
    let mut rows = Vec::new();
    for (index, (label, forecasts)) in series.iter().enumerate() {
        let scored = scores(forecasts);
        if scored.is_empty() {
            continue;
        }
        let vs = if index == 0 {
            None
        } else {
            Some(ci(scored
                .iter()
                .filter_map(|(id, block, ll, _)| reference.get(id).map(|r| (*block, ll - r)))
                .collect())?)
        };
        rows.push(BinaryRow {
            label: label.clone(),
            n: scored.len(),
            log_loss: ci(scored.iter().map(|(_, b, ll, _)| (*b, *ll)).collect())?,
            brier: ci(scored.iter().map(|(_, b, _, br)| (*b, *br)).collect())?,
            log_loss_vs_reference: vs,
        });
    }
    Ok(rows)
}

/// Forces d'équipe à la date du dernier match (modèle figé) et contrôle du proxy contre les xG publiés.
fn ratings_and_proxy(
    leagues: &[LeagueData],
    config: ModelConfig,
    last: NaiveDate,
) -> Result<(Vec<LeagueRatings>, Vec<ProxyRow>)> {
    let as_of = last + chrono::Days::new(1);
    let mut ratings = Vec::new();
    let mut proxy_rows = Vec::new();
    for league_info in LEAGUES {
        let Some(league) = leagues.iter().find(|l| l.div == league_info.div) else {
            continue;
        };
        let proxy = if config.blend > 0.0 {
            fit_proxy(league, as_of)
        } else {
            None
        };
        let training = training_games(league, as_of, config.decay, config.blend, proxy.as_ref());
        let fitted = DixonColes {
            ridge: config.ridge,
        }
        .fit(league.teams.len(), &training, None)?;
        let recent_cutoff = last - chrono::Days::new(365);
        let mut active: HashSet<usize> = HashSet::new();
        for game in league.games.iter().filter(|g| g.date > recent_cutoff) {
            active.insert(game.home);
            active.insert(game.away);
        }
        let mut teams: Vec<TeamRating> = active
            .into_iter()
            .map(|index| TeamRating {
                name: league.teams[index].clone(),
                attack: fitted.attack[index],
                defence: fitted.defence[index],
                net: fitted.attack[index] - fitted.defence[index],
            })
            .collect();
        teams.sort_by(|a, b| b.net.total_cmp(&a.net));
        ratings.push(LeagueRatings {
            name: league_info.name.to_string(),
            teams,
        });

        // Proxy ajusté sur le passé (avant 2026/27), évalué sur les xG publiés de 2026/27 : hors échantillon.
        let cutoff = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap_or(as_of);
        let mut observations = Vec::new();
        for game in league.games.iter().filter(|g| g.date < cutoff) {
            if let Some([(shots_home, on_home), (shots_away, on_away)]) = game.shots {
                observations.push(ShotObservation {
                    on_target: on_home,
                    shots: shots_home,
                    goals: game.goals_home,
                });
                observations.push(ShotObservation {
                    on_target: on_away,
                    shots: shots_away,
                    goals: game.goals_away,
                });
            }
        }
        if let Some(fit) = XgProxy::fit(&observations) {
            let mut pairs = Vec::new();
            for game in league.games.iter().filter(|g| g.date >= cutoff) {
                if let (
                    Some([(shots_home, on_home), (shots_away, on_away)]),
                    Some((xg_home, xg_away)),
                ) = (game.shots, game.published_xg)
                {
                    pairs.push((fit.expected(shots_home, on_home), xg_home));
                    pairs.push((fit.expected(shots_away, on_away), xg_away));
                }
            }
            let agree = agreement(&pairs);
            proxy_rows.push(ProxyRow {
                league: league_info.name.to_string(),
                on_target: fit.on_target,
                off_target: fit.off_target,
                pairs: pairs.len(),
                correlation: agree.map(|a| a.0),
                ratio: agree.map(|a| a.1),
            });
        }
    }
    Ok((ratings, proxy_rows))
}

/// Libellé d'une famille pour l'affichage.
pub fn kind_label(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Poisson => "Poisson",
        ModelKind::DixonColes => "Dixon-Coles",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probs(h: f64, d: f64, a: f64) -> Probs1x2 {
        Probs1x2::new(h, d, a).unwrap()
    }

    /// Parcours complet sur une ligue simulée à paramètres connus : cotes de marché fabriquées à partir de la
    /// vérité (marge de 5 %), réglage sur une petite grille, évaluation figée, rapport puis HTML.
    #[test]
    fn the_report_is_built_end_to_end_on_a_simulated_league() {
        use chrono::Utc;

        use crate::models::testing::{insert_league, simulated_league_from, truth};
        use crate::report::render::render_models_report;
        use crate::store::migrate;

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        // 11 saisons depuis 2016 : la 9e (2024/25) sert de validation, les deux suivantes de test.
        let league = simulated_league_from(NaiveDate::from_ymd_opt(2016, 8, 1).unwrap(), 11, 5);
        insert_league(&conn, &league);
        let truth = truth();
        for game in &league.games {
            let grid = truth.grid(game.home, game.away).unwrap();
            let [h, d, a] = grid.home_draw_away().unwrap().to_array();
            let over = grid.over(2.5);
            for (bookmaker, moment) in [("Avg", "close"), ("Avg", "pre"), ("PS", "close")] {
                conn.execute(
                    "INSERT INTO game_odds VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        game.game_id,
                        bookmaker,
                        moment,
                        1.0 / (h * 1.05),
                        1.0 / (d * 1.05),
                        1.0 / (a * 1.05)
                    ],
                )
                .unwrap();
            }
            for moment in ["close", "pre"] {
                conn.execute(
                    "INSERT INTO game_totals_odds VALUES (?1, 'Avg', ?2, 2.5, ?3, ?4)",
                    rusqlite::params![
                        game.game_id,
                        moment,
                        1.0 / (over * 1.05),
                        1.0 / ((1.0 - over) * 1.05)
                    ],
                )
                .unwrap();
            }
        }
        let leagues = vec![league];
        let grid = [
            ModelConfig {
                kind: ModelKind::Poisson,
                decay: 0.002,
                ridge: 2.0,
                blend: 0.0,
            },
            ModelConfig {
                kind: ModelKind::DixonColes,
                decay: 0.002,
                ridge: 2.0,
                blend: 0.0,
            },
            ModelConfig {
                kind: ModelKind::DixonColes,
                decay: 0.002,
                ridge: 2.0,
                blend: 0.5,
            },
        ];
        tuning::tune(
            &mut conn,
            &leagues,
            &grid,
            validation_window(),
            Utc::now(),
            &mut |_, _, _| {},
        )
        .unwrap();
        let last = leagues[0].games.iter().map(|g| g.date).max().unwrap();
        tuning::evaluate(&mut conn, &leagues, tuning::test_window(last), Utc::now()).unwrap();

        let report = build(&conn, &leagues, MarginMethod::Power, last, "2026-10-19").unwrap();

        assert!(report.test_games > 50, "{}", report.test_games);
        assert_eq!(report.frozen.len(), 3);
        assert_eq!(report.comparison.len(), 6);
        assert!(
            report
                .comparison
                .iter()
                .all(|row| row.n == report.comparison[0].n)
        );
        assert_eq!(
            report.comparison[0].label,
            "Moyenne du marché, clôture (référence)"
        );
        // Le marché fabriqué à partir de la vérité ne peut pas être battu par un modèle ajusté sur du bruit.
        let poisson = report
            .comparison
            .iter()
            .find(|r| r.label == "Poisson")
            .unwrap();
        assert!(poisson.log_loss_vs_reference.unwrap().mean > -0.02);
        // Le modèle est meilleur que les fréquences passées de la ligue.
        let baseline = report.comparison.last().unwrap();
        assert!(poisson.log_loss.mean < baseline.log_loss.mean + 0.02);
        assert!((0.0..=1.0).contains(&report.blend_weight_close));
        assert!((0.0..=1.0).contains(&report.blend_weight_pre));
        assert_eq!(report.blend_close.len(), 2);
        assert_eq!(report.blend_pre.len(), 3);
        assert!(!report.totals.is_empty() && !report.btts.is_empty());
        assert_eq!(report.by_league.len(), 1);
        assert_eq!(report.ratings[0].teams.len(), 8);
        assert_eq!(report.tuning_sessions, 1);
        assert_eq!(report.configs_tried, 3);

        let html = render_models_report(&report).unwrap();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Les modèles face au marché"));
        assert!(html.contains("Dixon-Coles"));
        for forbidden in ["<script", "<link", "src=", "href=", "@import"] {
            assert!(
                !html.contains(forbidden),
                "{forbidden} ne doit pas apparaître"
            );
        }
    }

    #[test]
    fn the_geometric_blend_interpolates_between_the_two_forecasts() {
        let (a, b) = (probs(0.5, 0.3, 0.2), probs(0.2, 0.3, 0.5));
        let zero = geometric_blend(&a, &b, 0.0).unwrap().to_array();
        let one = geometric_blend(&a, &b, 1.0).unwrap().to_array();
        let half = geometric_blend(&a, &b, 0.5).unwrap().to_array();
        for i in 0..3 {
            assert!((zero[i] - a.to_array()[i]).abs() < 1e-12);
            assert!((one[i] - b.to_array()[i]).abs() < 1e-12);
        }
        // Moyenne géométrique de deux prévisions symétriques : le nul est favorisé et la somme vaut 1.
        assert!((half.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!((half[0] - half[2]).abs() < 1e-12);
        assert!(half[1] > 0.3);
    }

    #[test]
    fn the_best_weight_is_zero_when_the_model_is_pure_noise_and_one_when_the_market_is() {
        // Résultats : une victoire à domicile sur deux.
        let outcomes: HashMap<i64, Outcome> = (0..200)
            .map(|i| {
                (
                    i,
                    if i % 2 == 0 {
                        Outcome::Home
                    } else {
                        Outcome::Away
                    },
                )
            })
            .collect();
        let ids: Vec<i64> = (0..200).collect();
        // Un pronostiqueur informé : favori du bon côté. Un bruité : uniforme.
        let informed: HashMap<i64, Probs1x2> = ids
            .iter()
            .map(|i| {
                (
                    *i,
                    if i % 2 == 0 {
                        probs(0.7, 0.1, 0.2)
                    } else {
                        probs(0.2, 0.1, 0.7)
                    },
                )
            })
            .collect();
        let noise: HashMap<i64, Probs1x2> = ids
            .iter()
            .map(|i| (*i, probs(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0)))
            .collect();

        // Marché informé + modèle bruité : on ne doit pas faire confiance au modèle.
        assert_eq!(best_weight(&informed, &noise, &outcomes, &ids), 0.0);
        // Marché bruité + modèle informé : tout le poids va au modèle.
        assert_eq!(best_weight(&noise, &informed, &outcomes, &ids), 1.0);
    }
}
