//! Construction des chiffres du rapport « marché » : marges, méthodes de retrait de marge, sources de
//! référence, calibration et tranches de cote. Rien ici n'écrit de HTML : voir [`super::render`].
//!
//! Question posée : à quel point le marché est-il un bon pronostiqueur, et quelle méthode de retrait de marge
//! lui rend ses probabilités les plus justes ? Réponse mesurée sur les scores propres (log loss, Brier, RPS)
//! avec intervalles de confiance par blocs de journées, jamais sur le taux de bons pronostics.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use serde::Serialize;

use super::svg::reliability_chart;
use crate::domain::{Moment, Outcome, Probs1x2};
use crate::error::{Error, Result};
use crate::eval::bootstrap::{MeanCi, block_bootstrap_mean};
use crate::eval::metrics::{brier, log_loss, rps};
use crate::ingest::football_data::LEAGUES;
use crate::ingest::football_data::quality::{self, QcReport};
use crate::market::calibration::{Bin, BucketInput, OddsBucket, odds_buckets, reliability};
use crate::market::dataset::{self, MarketGame};
use crate::market::margin::{MarginMethod, remove_margin};

/// Rééchantillonnages du bootstrap par blocs.
const RESAMPLES: usize = 2000;
/// Graine fixe : le rapport est reproductible à l'identique (invariant 9).
const SEED: u64 = 20_261_018;
/// Largeur des classes du diagramme de fiabilité.
const BIN_WIDTH: f64 = 0.05;

/// Scores d'une méthode de retrait de marge sur l'échantillon commun.
#[derive(Debug, Clone, Serialize)]
pub struct MethodRow {
    /// Nom de la méthode.
    pub name: &'static str,
    /// Nombre de matchs.
    pub n: usize,
    /// Log loss moyen et intervalle à 95 %.
    pub log_loss: MeanCi,
    /// Score de Brier moyen.
    pub brier: MeanCi,
    /// RPS moyen.
    pub rps: MeanCi,
    /// Différence de log loss avec la méthode multiplicative (négatif = meilleur). `None` pour la référence.
    pub log_loss_vs_multiplicative: Option<MeanCi>,
    /// Différence de RPS avec la méthode multiplicative.
    pub rps_vs_multiplicative: Option<MeanCi>,
}

/// Qualité d'une source de cotes (bookmaker et moment) sur l'échantillon commun à toutes les sources.
#[derive(Debug, Clone, Serialize)]
pub struct SourceRow {
    /// Libellé (« Pinnacle, clôture »).
    pub label: String,
    /// Marge moyenne de la source sur l'échantillon commun.
    pub mean_margin: f64,
    /// Log loss moyen.
    pub log_loss: MeanCi,
    /// RPS moyen.
    pub rps: MeanCi,
    /// Différence de log loss avec la référence (Pinnacle, clôture). `None` pour la référence elle-même.
    pub log_loss_vs_reference: Option<MeanCi>,
}

/// Les méthodes de retrait de marge comparées sur un bookmaker à marge plus élevée (cotes de clôture).
#[derive(Debug, Clone, Serialize)]
pub struct BookmakerMethodRow {
    /// Nom du bookmaker.
    pub label: String,
    /// Nombre de matchs.
    pub n: usize,
    /// Marge moyenne.
    pub mean_margin: f64,
    /// Δ log loss de la méthode `power` contre la multiplicative.
    pub power_log_loss: MeanCi,
    /// Δ log loss de la méthode `shin` contre la multiplicative.
    pub shin_log_loss: MeanCi,
    /// Δ RPS de la méthode `power` contre la multiplicative.
    pub power_rps: MeanCi,
    /// Δ RPS de la méthode `shin` contre la multiplicative.
    pub shin_rps: MeanCi,
}

/// Marge moyenne d'un bookmaker à un moment donné.
#[derive(Debug, Clone, Serialize)]
pub struct MarginRow {
    /// Préfixe de colonne football-data.
    pub bookmaker: String,
    /// `pre` ou `close`.
    pub moment: String,
    /// Nombre de matchs.
    pub games: u32,
    /// Marge moyenne : somme des probabilités implicites − 1.
    pub mean: f64,
    /// Marge minimale.
    pub min: f64,
    /// Marge maximale.
    pub max: f64,
}

/// Marge moyenne de trois sources de référence, saison par saison (cotes de clôture).
#[derive(Debug, Clone, Serialize)]
pub struct SeasonMargin {
    /// Saison.
    pub season: String,
    /// Pinnacle (absent après janvier 2026).
    pub pinnacle: Option<f64>,
    /// Moyenne du marché.
    pub average: Option<f64>,
    /// Bet365.
    pub bet365: Option<f64>,
}

/// Calibration d'une ligue.
#[derive(Debug, Clone, Serialize)]
pub struct LeagueView {
    /// Code football-data.
    pub div: String,
    /// Nom.
    pub name: String,
    /// Nombre de matchs.
    pub games: usize,
    /// Diagramme de fiabilité (SVG).
    pub svg: String,
    /// Tranches de cote de la ligue.
    pub buckets: Vec<OddsBucket>,
}

/// Tout ce que le rapport affiche.
#[derive(Debug, Clone, Serialize)]
pub struct MarketReport {
    /// Date de génération.
    pub generated_on: String,
    /// Matchs en base.
    pub games_total: u32,
    /// Matchs avec une clôture Pinnacle.
    pub games_with_pinnacle: usize,
    /// Contrôle qualité des données.
    pub qc: QcReport,
    /// Marges des bookmakers.
    pub margins: Vec<MarginRow>,
    /// Marges par saison.
    pub season_margins: Vec<SeasonMargin>,
    /// Comparaison des méthodes de retrait de marge.
    pub methods: Vec<MethodRow>,
    /// Matchs écartés de la comparaison (une méthode sans solution).
    pub methods_excluded: usize,
    /// Méthode au log loss le plus bas.
    pub best_method: &'static str,
    /// Cette différence est-elle distinguable du bruit ? (intervalle de la meilleure méthode contre la multiplicative)
    pub best_method_is_distinguishable: bool,
    /// Les méthodes sur des bookmakers à marge plus élevée.
    pub bookmaker_methods: Vec<BookmakerMethodRow>,
    /// Qualité des sources de cotes.
    pub sources: Vec<SourceRow>,
    /// Matchs de l'échantillon commun aux sources.
    pub sources_common_games: usize,
    /// Diagramme de fiabilité de toutes les ligues.
    pub pooled_svg: String,
    /// Classes du diagramme (toutes ligues).
    pub pooled_bins: Vec<Bin>,
    /// Tranches de cote, toutes ligues.
    pub pooled_buckets: Vec<OddsBucket>,
    /// Une vue par ligue.
    pub leagues: Vec<LeagueView>,
}

/// Construit le rapport à partir de la base.
pub fn build(conn: &Connection, current_season: &str, generated_on: &str) -> Result<MarketReport> {
    let ps_close = dataset::load(conn, "PS", Moment::Close)?;
    if ps_close.is_empty() {
        return Err(Error::Data(
            "aucune cote de clôture Pinnacle en base : lance `edge fd fetch` puis `edge fd import`"
                .to_string(),
        ));
    }

    // Échantillon commun aux trois méthodes : un match que l'une d'elles ne sait pas traiter est écarté de
    // toutes, pour comparer les méthodes sur exactement les mêmes matchs.
    let usable: Vec<&MarketGame> = ps_close
        .iter()
        .filter(|game| {
            MarginMethod::ALL
                .iter()
                .all(|m| remove_margin(&game.odds, *m).is_some())
        })
        .collect();
    let methods_excluded = ps_close.len() - usable.len();
    let methods = method_rows(&usable)?;
    let best = best_method(&methods);
    let best_row = methods
        .iter()
        .find(|row| row.name == best.name())
        .ok_or_else(|| Error::Data("méthode introuvable".into()))?;
    let best_method_is_distinguishable = best_row
        .log_loss_vs_multiplicative
        .is_some_and(|ci| ci.excludes_zero());

    let (sources, sources_common_games) = source_rows(conn, best)?;

    let pooled_pairs = reliability_pairs(&usable, best);
    let pooled_bins = reliability(&pooled_pairs, BIN_WIDTH);
    let pooled_svg = reliability_chart(&pooled_bins, "Toutes ligues")?;
    let pooled_buckets = odds_buckets(&bucket_inputs(&usable, best));

    let mut leagues = Vec::new();
    for league in LEAGUES {
        let subset: Vec<&MarketGame> = usable
            .iter()
            .copied()
            .filter(|g| g.div == league.div)
            .collect();
        let bins = reliability(&reliability_pairs(&subset, best), BIN_WIDTH);
        leagues.push(LeagueView {
            div: league.div.to_string(),
            name: league.name.to_string(),
            games: subset.len(),
            svg: reliability_chart(&bins, league.name)?,
            buckets: odds_buckets(&bucket_inputs(&subset, best)),
        });
    }

    let games_total = conn.query_row("SELECT COUNT(*) FROM game", [], |row| row.get(0))?;
    Ok(MarketReport {
        generated_on: generated_on.to_string(),
        games_total,
        games_with_pinnacle: ps_close.len(),
        qc: quality::run(conn, current_season)?,
        margins: margin_rows(conn)?,
        season_margins: season_margin_rows(conn)?,
        methods,
        methods_excluded,
        bookmaker_methods: bookmaker_method_rows(conn)?,
        best_method: best.name(),
        best_method_is_distinguishable,
        sources,
        sources_common_games,
        pooled_svg,
        pooled_bins,
        pooled_buckets,
        leagues,
    })
}

/// Compare les trois méthodes sur des bookmakers dont la marge est plus élevée que celle de Pinnacle : c'est
/// là que le choix de méthode compte, et ce sont les marges des bookmakers français qui nous intéressent.
fn bookmaker_method_rows(conn: &Connection) -> Result<Vec<BookmakerMethodRow>> {
    let bookmakers = [
        ("Bet365", "B365"),
        ("William Hill", "WH"),
        ("Bet&Win", "BW"),
        ("Interwetten", "IW"),
        ("VC Bet", "VC"),
    ];
    let mut rows = Vec::new();
    for (label, code) in bookmakers {
        let games = dataset::load(conn, code, Moment::Close)?;
        let usable: Vec<&MarketGame> = games
            .iter()
            .filter(|game| {
                MarginMethod::ALL
                    .iter()
                    .all(|m| remove_margin(&game.odds, *m).is_some())
            })
            .collect();
        if usable.len() < 1000 {
            continue;
        }
        let methods = method_rows(&usable)?;
        let (Some(power_ll), Some(shin_ll), Some(power_rps), Some(shin_rps)) = (
            methods[1].log_loss_vs_multiplicative,
            methods[2].log_loss_vs_multiplicative,
            methods[1].rps_vs_multiplicative,
            methods[2].rps_vs_multiplicative,
        ) else {
            continue;
        };
        rows.push(BookmakerMethodRow {
            label: label.to_string(),
            n: usable.len(),
            mean_margin: usable
                .iter()
                .map(|g| g.odds.implied_sum() - 1.0)
                .sum::<f64>()
                / usable.len() as f64,
            power_log_loss: power_ll,
            shin_log_loss: shin_ll,
            power_rps,
            shin_rps,
        });
    }
    Ok(rows)
}

/// Intervalle de confiance d'une série de valeurs `(bloc, valeur)`.
fn ci(values: Vec<(u32, f64)>) -> Result<MeanCi> {
    block_bootstrap_mean(&values, RESAMPLES, SEED)
        .ok_or_else(|| Error::Data("échantillon vide".to_string()))
}

/// Scores d'un match pour une méthode : `(bloc, log loss, Brier, RPS)`.
fn score(game: &MarketGame, method: MarginMethod) -> Option<(u32, f64, f64, f64)> {
    let forecast = remove_margin(&game.odds, method)?;
    Some((
        game.block,
        log_loss(&forecast, game.outcome),
        brier(&forecast, game.outcome),
        rps(&forecast, game.outcome),
    ))
}

fn method_rows(games: &[&MarketGame]) -> Result<Vec<MethodRow>> {
    // Une liste de scores par méthode, dans l'ordre de `MarginMethod::ALL` (la première est la référence).
    let scored: Vec<Vec<(u32, f64, f64, f64)>> = MarginMethod::ALL
        .iter()
        .map(|method| {
            games
                .iter()
                .filter_map(|game| score(game, *method))
                .collect()
        })
        .collect();
    let mut rows = Vec::new();
    for (index, method) in MarginMethod::ALL.iter().enumerate() {
        let series = &scored[index];
        let (vs_ll, vs_rps) = if index == 0 {
            (None, None)
        } else {
            let reference = &scored[0];
            let diff = |pick: fn(&(u32, f64, f64, f64)) -> f64| -> Result<MeanCi> {
                ci(series
                    .iter()
                    .zip(reference)
                    .map(|(a, b)| (a.0, pick(a) - pick(b)))
                    .collect())
            };
            (Some(diff(|s| s.1)?), Some(diff(|s| s.3)?))
        };
        rows.push(MethodRow {
            name: method.name(),
            n: series.len(),
            log_loss: ci(series.iter().map(|s| (s.0, s.1)).collect())?,
            brier: ci(series.iter().map(|s| (s.0, s.2)).collect())?,
            rps: ci(series.iter().map(|s| (s.0, s.3)).collect())?,
            log_loss_vs_multiplicative: vs_ll,
            rps_vs_multiplicative: vs_rps,
        });
    }
    Ok(rows)
}

/// Méthode au log loss moyen le plus bas.
fn best_method(rows: &[MethodRow]) -> MarginMethod {
    MarginMethod::ALL
        .into_iter()
        .zip(rows)
        .min_by(|a, b| a.1.log_loss.mean.total_cmp(&b.1.log_loss.mean))
        .map(|(method, _)| method)
        .unwrap_or(MarginMethod::Multiplicative)
}

/// Compare les sources de cotes sur les matchs où toutes existent (donc la même période, les mêmes matchs).
fn source_rows(conn: &Connection, method: MarginMethod) -> Result<(Vec<SourceRow>, usize)> {
    let definitions = [
        ("Pinnacle, clôture", "PS", Moment::Close),
        ("Moyenne du marché, clôture", "Avg", Moment::Close),
        ("Bet365, clôture", "B365", Moment::Close),
        ("Pinnacle, avant-match", "PS", Moment::Pre),
        ("Moyenne du marché, avant-match", "Avg", Moment::Pre),
        ("Bet365, avant-match", "B365", Moment::Pre),
    ];
    let mut sets = Vec::new();
    for (label, bookmaker, moment) in definitions {
        sets.push((label, dataset::load(conn, bookmaker, moment)?));
    }
    // Une source sans aucune cote (bookmaker absent de la base) est ignorée plutôt que de vider l'échantillon commun.
    sets.retain(|(_, games)| !games.is_empty());
    // Identifiants présents dans toutes les sources et exploitables par la méthode.
    let mut common: Option<HashSet<i64>> = None;
    for (_, games) in &sets {
        let ids: HashSet<i64> = games
            .iter()
            .filter(|game| remove_margin(&game.odds, method).is_some())
            .map(|game| game.game_id)
            .collect();
        common = Some(match common {
            None => ids,
            Some(previous) => previous.intersection(&ids).copied().collect(),
        });
    }
    let common = common.unwrap_or_default();

    // Log loss de la référence par match, pour les différences appariées.
    let mut reference: HashMap<i64, f64> = HashMap::new();
    let mut rows = Vec::new();
    for (index, (label, games)) in sets.iter().enumerate() {
        let selected: Vec<&MarketGame> = games
            .iter()
            .filter(|g| common.contains(&g.game_id))
            .collect();
        if selected.is_empty() {
            continue;
        }
        let scored: Vec<(i64, u32, f64, f64, f64)> = selected
            .iter()
            .filter_map(|game| {
                score(game, method)
                    .map(|s| (game.game_id, s.0, s.1, s.3, game.odds.implied_sum() - 1.0))
            })
            .collect();
        if index == 0 {
            reference = scored.iter().map(|s| (s.0, s.2)).collect();
        }
        let vs_reference = if index == 0 {
            None
        } else {
            Some(ci(scored
                .iter()
                .filter_map(|s| reference.get(&s.0).map(|r| (s.1, s.2 - r)))
                .collect())?)
        };
        rows.push(SourceRow {
            label: (*label).to_string(),
            mean_margin: scored.iter().map(|s| s.4).sum::<f64>() / scored.len() as f64,
            log_loss: ci(scored.iter().map(|s| (s.1, s.2)).collect())?,
            rps: ci(scored.iter().map(|s| (s.1, s.3)).collect())?,
            log_loss_vs_reference: vs_reference,
        });
    }
    Ok((rows, common.len()))
}

/// Paires (probabilité prévue, s'est produit) pour les trois issues de chaque match.
fn reliability_pairs(games: &[&MarketGame], method: MarginMethod) -> Vec<(f64, bool)> {
    let mut pairs = Vec::with_capacity(games.len() * 3);
    for game in games {
        if let Some(forecast) = remove_margin(&game.odds, method) {
            for outcome in Outcome::ALL {
                pairs.push((forecast.get(outcome).get(), outcome == game.outcome));
            }
        }
    }
    pairs
}

/// Sélections pour l'analyse par tranche de cote : probabilité sans marge, cote brute, issue gagnante ou non.
fn bucket_inputs(games: &[&MarketGame], method: MarginMethod) -> Vec<BucketInput> {
    let mut inputs = Vec::with_capacity(games.len() * 3);
    for game in games {
        let Some(forecast): Option<Probs1x2> = remove_margin(&game.odds, method) else {
            continue;
        };
        for outcome in Outcome::ALL {
            inputs.push(BucketInput {
                fair_probability: forecast.get(outcome).get(),
                raw_odds: game.odds.get(outcome).get(),
                won: outcome == game.outcome,
            });
        }
    }
    inputs
}

/// Marge moyenne, minimale et maximale par bookmaker et moment. Sont exclus : `Max` (meilleure cote de chaque
/// issue, sa somme passe sous 1 par construction), `BFE` (exchange Betfair, cotes peu liquides) et les
/// bookmakers à moins de 500 matchs.
fn margin_rows(conn: &Connection) -> Result<Vec<MarginRow>> {
    let mut statement = conn.prepare(
        "SELECT bookmaker, moment, COUNT(*),
                AVG(1.0 / home + 1.0 / draw + 1.0 / away) - 1.0,
                MIN(1.0 / home + 1.0 / draw + 1.0 / away) - 1.0,
                MAX(1.0 / home + 1.0 / draw + 1.0 / away) - 1.0
           FROM game_odds
          WHERE bookmaker NOT IN ('Max', 'BFE')
          GROUP BY bookmaker, moment
         HAVING COUNT(*) >= 500
          ORDER BY moment DESC, 4",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(MarginRow {
                bookmaker: row.get(0)?,
                moment: row.get(1)?,
                games: row.get(2)?,
                mean: row.get(3)?,
                min: row.get(4)?,
                max: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn season_margin_rows(conn: &Connection) -> Result<Vec<SeasonMargin>> {
    let mut statement = conn.prepare(
        "SELECT g.season, o.bookmaker, AVG(1.0 / o.home + 1.0 / o.draw + 1.0 / o.away) - 1.0
           FROM game_odds o JOIN game g ON g.id = o.game_id
          WHERE o.moment = 'close' AND o.bookmaker IN ('PS', 'Avg', 'B365')
          GROUP BY g.season, o.bookmaker
          ORDER BY g.season",
    )?;
    let mut by_season: Vec<SeasonMargin> = Vec::new();
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;
    for row in rows {
        let (season, bookmaker, margin) = row?;
        // Les lignes arrivent triées par saison : on complète la dernière ligne ou on en ouvre une nouvelle.
        if by_season.last().is_none_or(|last| last.season != season) {
            by_season.push(SeasonMargin {
                season,
                pinnacle: None,
                average: None,
                bet365: None,
            });
        }
        if let Some(last) = by_season.last_mut() {
            match bookmaker.as_str() {
                "PS" => last.pinnacle = Some(margin),
                "Avg" => last.average = Some(margin),
                _ => last.bet365 = Some(margin),
            }
        }
    }
    Ok(by_season)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;

    /// Base synthétique : `n` matchs d'un marché parfaitement calibré (les issues sont tirées au sort selon
    /// les probabilités publiées) pour une ligue, deux bookmakers.
    fn synthetic_database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn.execute_batch("INSERT INTO team (id, name) VALUES (1, 'A'), (2, 'B');")
            .unwrap();
        // Générateur congruentiel simple : suffisant pour fabriquer des données de test déterministes.
        let mut state: u64 = 12345;
        let mut uniform = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f64) / ((1u64 << 31) as f64)
        };
        let tx = conn.transaction().unwrap();
        for i in 0..600 {
            // Probabilités vraies variables d'un match à l'autre.
            let home = 0.25 + 0.5 * uniform();
            let draw = (1.0 - home) * (0.3 + 0.2 * uniform());
            let away = 1.0 - home - draw;
            let draw_roll = uniform();
            let (result, ft) = if draw_roll < home {
                ("H", (1, 0))
            } else if draw_roll < home + draw {
                ("D", (0, 0))
            } else {
                ("A", (0, 1))
            };
            // Un match par jour : la clé (ligue, date, équipes) reste unique.
            let date =
                chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap() + chrono::Days::new(i as u64);
            tx.execute(
                "INSERT INTO game (id, div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
                 VALUES (?1, 'E0', '2024/25', ?2, 1, 2, ?3, ?4, ?5)",
                rusqlite::params![i + 1, date.format("%Y-%m-%d").to_string(), ft.0, ft.1, result],
            )
            .unwrap();
            // Marge de 5 % répartie proportionnellement.
            for (bookmaker, margin) in [("PS", 1.05), ("Avg", 1.08)] {
                for moment in ["pre", "close"] {
                    tx.execute(
                        "INSERT INTO game_odds VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![
                            i + 1,
                            bookmaker,
                            moment,
                            1.0 / (home * margin),
                            1.0 / (draw * margin),
                            1.0 / (away * margin)
                        ],
                    )
                    .unwrap();
                }
            }
        }
        tx.commit().unwrap();
        conn
    }

    #[test]
    fn the_report_is_built_from_a_calibrated_synthetic_market() {
        let conn = synthetic_database();

        let report = build(&conn, "2026/27", "2026-10-18").unwrap();

        assert_eq!(report.games_total, 600);
        assert_eq!(report.methods.len(), 3);
        assert!(report.methods.iter().all(|m| m.n == 600));
        // Un marché calibré a un log loss proche de l'entropie : bien en dessous de ln 3 ≈ 1,0986.
        assert!(
            report.methods[0].log_loss.mean < 1.0986,
            "{:?}",
            report.methods[0].log_loss
        );
        assert!(report.methods[0].log_loss.low <= report.methods[0].log_loss.mean);
        // La marge synthétique de 5 % est retrouvée.
        let ps = report
            .margins
            .iter()
            .find(|m| m.bookmaker == "PS" && m.moment == "close")
            .unwrap();
        assert!((ps.mean - 0.05).abs() < 1e-9, "{}", ps.mean);
        // Calibration : la probabilité prévue et la fréquence observée se suivent classe par classe.
        let big_bins: Vec<&Bin> = report.pooled_bins.iter().filter(|b| b.n >= 100).collect();
        assert!(!big_bins.is_empty());
        for bin in big_bins {
            assert!((bin.mean_predicted - bin.observed).abs() < 0.12, "{bin:?}");
        }
        assert_eq!(report.leagues.len(), 6);
        assert_eq!(report.leagues[0].games, 600);
        assert_eq!(report.leagues[1].games, 0);
        assert!(report.pooled_svg.contains("<svg"));
    }

    #[test]
    fn sources_are_compared_on_the_same_matches_with_the_reference_first() {
        let conn = synthetic_database();

        let report = build(&conn, "2026/27", "2026-10-18").unwrap();

        assert_eq!(report.sources_common_games, 600);
        assert_eq!(report.sources[0].label, "Pinnacle, clôture");
        assert!(report.sources[0].log_loss_vs_reference.is_none());
        // Bet365 est absent de la base synthétique : sa source est ignorée, les quatre autres restent.
        assert_eq!(report.sources.len(), 4);
        assert!(
            report
                .sources
                .iter()
                .all(|s| !s.label.starts_with("Bet365"))
        );
        // La moyenne du marché (marge de 8 %) est moins bien que Pinnacle (5 %) : sa marge se retrouve ici.
        let average = report
            .sources
            .iter()
            .find(|s| s.label == "Moyenne du marché, clôture")
            .unwrap();
        assert!((average.mean_margin - 0.08).abs() < 1e-9);
        assert!(average.log_loss_vs_reference.is_some());
    }

    #[test]
    fn the_html_is_self_contained_french_and_complete() {
        let conn = synthetic_database();
        let report = build(&conn, "2026/27", "2026-10-18").unwrap();

        let html = crate::report::render::render_market_report(&report).unwrap();

        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Le marché comme pronostiqueur"));
        assert!(html.contains("2026-10-18"));
        // Tableaux et graphiques présents ; nombres au format français (virgule), pas de point décimal brut.
        assert!(html.contains("<svg"));
        assert!(html.contains("multiplicative"));
        assert!(
            html.contains("4,9 %") || html.contains("5,0 %"),
            "marge de 5 % attendue"
        );
        // Autonome : aucune ressource externe (le seul « http » admis est l'espace de noms SVG).
        for forbidden in ["<script", "<link", "src=", "href=", "@import"] {
            assert!(
                !html.contains(forbidden),
                "{forbidden} ne doit pas apparaître"
            );
        }
    }

    #[test]
    fn an_empty_database_gives_an_actionable_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        let err = build(&conn, "2026/27", "2026-10-18").unwrap_err();
        assert!(err.to_string().contains("edge fd fetch"));
    }

    #[test]
    fn the_report_is_reproducible() {
        let conn = synthetic_database();
        let a = build(&conn, "2026/27", "2026-10-18").unwrap();
        let b = build(&conn, "2026/27", "2026-10-18").unwrap();
        assert_eq!(a.methods[1].log_loss, b.methods[1].log_loss);
        assert_eq!(a.best_method, b.best_method);
    }
}
