//! Générateurs de paris candidats : où une règle de sélection voit de la valeur.
//!
//! Deux sources de valeur, évaluées par le même protocole :
//!
//! 1. **prix mou contre Pinnacle au même instant** : un bookmaker dont le prix d'avant-match dépasse la cote juste
//!    de Pinnacle (même relevé, donc décidable) ;
//! 2. **modèle contre prix** : la probabilité du modèle contre le prix d'un bookmaker.
//!
//! Dans les deux cas, seules des informations d'avant-match décident ; la clôture ne sert qu'au CLV.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;
use serde::Serialize;

use super::engine::Candidate;
use crate::betting::value::best_selection;
use crate::domain::{Moment, Odds, Odds1x2, Outcome, Probs1x2};
use crate::error::Result;
use crate::market::margin::{MarginMethod, remove_margin};

/// Bookmakers dont les cotes sont chargées : Pinnacle, la moyenne du marché, et les bookmakers « mous » candidats.
pub const SOFT_BOOKMAKERS: [&str; 8] = ["B365", "WH", "BW", "IW", "VC", "LB", "BFD", "1XB"];

/// Période d'un match dans le découpage walk-forward du projet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    /// Avant le 1er juillet 2024 : saisons 2016/17 à 2023/24.
    Train,
    /// Saison 2024/25 : choix des seuils.
    Validation,
    /// À partir du 1er juillet 2025 : 2025/26 puis 2026/27 en avant. Évalué une seule fois.
    Test,
}

impl Period {
    /// Période d'une date de match.
    pub fn of(date: NaiveDate) -> Period {
        let validation_start = NaiveDate::from_ymd_opt(2024, 7, 1).unwrap_or_default();
        let test_start = NaiveDate::from_ymd_opt(2025, 7, 1).unwrap_or_default();
        if date < validation_start {
            Period::Train
        } else if date < test_start {
            Period::Validation
        } else {
            Period::Test
        }
    }
}

/// Un match avec tout ce qu'il faut pour générer des candidats.
#[derive(Debug, Clone, PartialEq)]
pub struct GameInputs {
    /// Identifiant du match en base.
    pub game_id: i64,
    /// Date du match.
    pub date: NaiveDate,
    /// Bloc de bootstrap : une ligue et une semaine.
    pub block: u32,
    /// Résultat réel.
    pub result: Outcome,
    /// Cotes 1X2 par bookmaker et moment.
    pub odds: HashMap<(String, Moment), Odds1x2>,
}

impl GameInputs {
    /// Cotes d'un bookmaker à un moment donné, si le fichier les a.
    pub(crate) fn price(&self, bookmaker: &str, moment: Moment) -> Option<&Odds1x2> {
        self.odds.get(&(bookmaker.to_string(), moment))
    }

    /// Référence d'avant-match : Pinnacle, ou la moyenne du marché quand Pinnacle manque (après janvier 2026).
    pub(crate) fn reference_pre(&self) -> Option<&Odds1x2> {
        self.price("PS", Moment::Pre)
            .or_else(|| self.price("Avg", Moment::Pre))
    }
}

/// Charge tous les matchs et leurs cotes (bookmakers mous, Pinnacle, moyenne du marché).
pub fn load_inputs(conn: &Connection) -> Result<Vec<GameInputs>> {
    let mut games: HashMap<i64, GameInputs> = HashMap::new();
    let mut blocks: HashMap<(String, i32), u32> = HashMap::new();
    let mut statement =
        conn.prepare("SELECT id, div, match_date, ft_result FROM game ORDER BY match_date, id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (game_id, div, date, result) = row?;
        let Ok(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") else {
            continue;
        };
        let week = (date.num_days_from_ce() - 1) / 7;
        let next = blocks.len() as u32;
        let block = *blocks.entry((div, week)).or_insert(next);
        let result = match result.as_str() {
            "H" => Outcome::Home,
            "D" => Outcome::Draw,
            _ => Outcome::Away,
        };
        games.insert(
            game_id,
            GameInputs {
                game_id,
                date,
                block,
                result,
                odds: HashMap::new(),
            },
        );
    }

    let wanted: Vec<&str> = SOFT_BOOKMAKERS
        .iter()
        .copied()
        .chain(["PS", "Avg"])
        .collect();
    let mut statement =
        conn.prepare("SELECT game_id, bookmaker, moment, home, draw, away FROM game_odds")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            [
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
            ],
        ))
    })?;
    for row in rows {
        let (game_id, bookmaker, moment, prices) = row?;
        if !wanted.contains(&bookmaker.as_str()) {
            continue;
        }
        let moment = if moment == "pre" {
            Moment::Pre
        } else {
            Moment::Close
        };
        let [Some(home), Some(draw), Some(away)] = prices.map(Odds::new) else {
            continue;
        };
        if let Some(game) = games.get_mut(&game_id) {
            game.odds
                .insert((bookmaker, moment), Odds1x2 { home, draw, away });
        }
    }
    let mut all: Vec<GameInputs> = games.into_values().collect();
    all.sort_by_key(|g| (g.date, g.game_id));
    Ok(all)
}

/// Probabilité de clôture sans marge de l'issue : Pinnacle si présent, sinon la moyenne du marché.
fn close_fair(
    game: &GameInputs,
    outcome: Outcome,
    method: MarginMethod,
) -> Option<crate::domain::Probability> {
    let close = game
        .price("PS", Moment::Close)
        .or_else(|| game.price("Avg", Moment::Close))?;
    remove_margin(close, method).map(|p| p.get(outcome))
}

/// Applique une décote multiplicative aux trois cotes (bookmaker plus généreux ou moins généreux que le marché).
fn discounted(odds: &Odds1x2, factor: f64) -> Option<Odds1x2> {
    Some(Odds1x2 {
        home: Odds::new(odds.home.get() * factor)?,
        draw: Odds::new(odds.draw.get() * factor)?,
        away: Odds::new(odds.away.get() * factor)?,
    })
}

/// Paramètres d'un générateur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Generator {
    /// Avantage minimal exigé (espérance d'un euro misé).
    pub min_edge: f64,
    /// Décote appliquée aux cotes du bookmaker : 1 = cotes telles quelles, 0,957 = ce que mesure l'archive pour Betclic.
    pub price_factor: f64,
    /// Méthode de retrait de marge de la référence.
    pub method: MarginMethod,
}

/// Source 1 : le prix d'avant-match de `book` contre la cote juste de Pinnacle au **même instant** (relevé
/// d'avant-match). Quand Pinnacle est absent des fichiers (après janvier 2026), la moyenne du marché d'avant-match
/// tient lieu de référence.
pub fn soft_vs_sharp(games: &[&GameInputs], book: &str, generator: &Generator) -> Vec<Candidate> {
    games
        .iter()
        .filter_map(|game| {
            let sharp = game.reference_pre()?;
            let soft = discounted(game.price(book, Moment::Pre)?, generator.price_factor)?;
            let probs = remove_margin(sharp, generator.method)?;
            candidate(game, &probs, &soft, generator)
        })
        .collect()
}

/// Source 2 : la probabilité du modèle contre le prix d'avant-match de `book`.
pub fn model_vs_price(
    games: &[&GameInputs],
    predictions: &HashMap<i64, Probs1x2>,
    book: &str,
    generator: &Generator,
) -> Vec<Candidate> {
    games
        .iter()
        .filter_map(|game| {
            let probs = predictions.get(&game.game_id)?;
            let soft = discounted(game.price(book, Moment::Pre)?, generator.price_factor)?;
            candidate(game, probs, &soft, generator)
        })
        .collect()
}

fn candidate(
    game: &GameInputs,
    probs: &Probs1x2,
    odds: &Odds1x2,
    generator: &Generator,
) -> Option<Candidate> {
    let selection = best_selection(probs, odds, generator.min_edge)?;
    Some(Candidate {
        game_id: game.game_id,
        date: game.date,
        block: game.block,
        outcome: selection.outcome,
        odds: selection.odds,
        probability: selection.probability,
        close_fair: close_fair(game, selection.outcome, generator.method),
        won: game.result == selection.outcome,
    })
}

/// Les matchs d'une période.
pub fn in_period(games: &[GameInputs], period: Period) -> Vec<&GameInputs> {
    games
        .iter()
        .filter(|g| Period::of(g.date) == period)
        .collect()
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn odds(h: f64, d: f64, a: f64) -> Odds1x2 {
        Odds1x2 {
            home: Odds::new(h).unwrap(),
            draw: Odds::new(d).unwrap(),
            away: Odds::new(a).unwrap(),
        }
    }

    fn game(
        id: i64,
        date: NaiveDate,
        result: Outcome,
        prices: &[(&str, Moment, Odds1x2)],
    ) -> GameInputs {
        GameInputs {
            game_id: id,
            date,
            block: 0,
            result,
            odds: prices
                .iter()
                .map(|(b, m, o)| ((b.to_string(), *m), *o))
                .collect(),
        }
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn generator(min_edge: f64, factor: f64) -> Generator {
        Generator {
            min_edge,
            price_factor: factor,
            method: MarginMethod::Multiplicative,
        }
    }

    #[test]
    fn the_periods_follow_the_walk_forward_split() {
        assert_eq!(Period::of(day(2016, 8, 13)), Period::Train);
        assert_eq!(Period::of(day(2024, 6, 30)), Period::Train);
        assert_eq!(Period::of(day(2024, 7, 1)), Period::Validation);
        assert_eq!(Period::of(day(2025, 6, 30)), Period::Validation);
        assert_eq!(Period::of(day(2025, 7, 1)), Period::Test);
        assert_eq!(Period::of(day(2026, 10, 12)), Period::Test);
    }

    #[test]
    fn a_soft_price_above_the_sharp_fair_odds_is_selected_on_the_best_outcome_only() {
        // Pinnacle sans marge (somme 1) : 50 % / 30 % / 20 % → cotes justes 2,0 / 3,33 / 5,0.
        let sharp = odds(2.0, 1.0 / 0.3, 5.0);
        // Le bookmaker mou paie 2,20 le domicile (+10 %), 3,40 le nul (+2 %) et 4,80 l'extérieur (−4 %).
        let soft = odds(2.20, 3.40, 4.80);
        let matches = [game(
            1,
            day(2025, 8, 16),
            Outcome::Home,
            &[("PS", Moment::Pre, sharp), ("B365", Moment::Pre, soft)],
        )];
        let refs: Vec<&GameInputs> = matches.iter().collect();

        let candidates = soft_vs_sharp(&refs, "B365", &generator(0.0, 1.0));

        assert_eq!(candidates.len(), 1, "une seule sélection par match");
        assert_eq!(candidates[0].outcome, Outcome::Home);
        assert_abs_diff_eq!(candidates[0].odds.get(), 2.20, epsilon = 1e-12);
        assert_abs_diff_eq!(candidates[0].probability.get(), 0.5, epsilon = 1e-9);
        assert!(candidates[0].won);
    }

    #[test]
    fn the_threshold_and_the_price_factor_remove_selections() {
        let sharp = odds(2.0, 1.0 / 0.3, 5.0);
        let soft = odds(2.20, 3.40, 4.80);
        let matches = [game(
            1,
            day(2025, 8, 16),
            Outcome::Draw,
            &[("PS", Moment::Pre, sharp), ("B365", Moment::Pre, soft)],
        )];
        let refs: Vec<&GameInputs> = matches.iter().collect();

        // Avantage exigé de 12 % : le meilleur (+10 %) ne suffit pas.
        assert!(soft_vs_sharp(&refs, "B365", &generator(0.12, 1.0)).is_empty());
        // Décote de 10 % : 2,20 × 0,90 = 1,98 < cote juste 2,0 → plus aucune valeur (et 0,91 laisserait +0,1 %).
        assert!(soft_vs_sharp(&refs, "B365", &generator(0.0, 0.90)).is_empty());
        // Sans décote ni exigence : une sélection.
        assert_eq!(soft_vs_sharp(&refs, "B365", &generator(0.0, 1.0)).len(), 1);
    }

    #[test]
    fn the_average_market_replaces_pinnacle_when_it_is_missing() {
        let avg = odds(2.0, 1.0 / 0.3, 5.0);
        let soft = odds(2.30, 3.0, 4.0);
        let matches = [game(
            1,
            day(2026, 3, 1),
            Outcome::Home,
            &[("Avg", Moment::Pre, avg), ("B365", Moment::Pre, soft)],
        )];
        let refs: Vec<&GameInputs> = matches.iter().collect();

        assert_eq!(soft_vs_sharp(&refs, "B365", &generator(0.0, 1.0)).len(), 1);
        // Ni Pinnacle ni moyenne : pas de référence, pas de candidat.
        let none = [game(
            2,
            day(2026, 3, 1),
            Outcome::Home,
            &[("B365", Moment::Pre, soft)],
        )];
        let refs: Vec<&GameInputs> = none.iter().collect();
        assert!(soft_vs_sharp(&refs, "B365", &generator(0.0, 1.0)).is_empty());
    }

    #[test]
    fn the_closing_price_only_feeds_the_clv_never_the_decision() {
        let sharp = odds(2.0, 1.0 / 0.3, 5.0);
        let soft = odds(2.20, 3.40, 4.80);
        let build = |close: Odds1x2| {
            vec![game(
                1,
                day(2025, 8, 16),
                Outcome::Home,
                &[
                    ("PS", Moment::Pre, sharp),
                    ("B365", Moment::Pre, soft),
                    ("PS", Moment::Close, close),
                ],
            )]
        };
        let (a, b) = (build(odds(1.5, 4.0, 8.0)), build(odds(3.0, 3.0, 3.0)));
        let (a_refs, b_refs): (Vec<&GameInputs>, Vec<&GameInputs>) =
            (a.iter().collect(), b.iter().collect());

        let first = &soft_vs_sharp(&a_refs, "B365", &generator(0.0, 1.0))[0];
        let second = &soft_vs_sharp(&b_refs, "B365", &generator(0.0, 1.0))[0];

        // Même décision, même cote, même probabilité : seule la clôture change.
        assert_eq!(
            (first.outcome, first.odds, first.probability),
            (second.outcome, second.odds, second.probability)
        );
        assert_ne!(first.close_fair, second.close_fair);
        // 1,5 / 4 / 8 : probabilités implicites 0,667 / 0,25 / 0,125, somme 1,0417 → domicile 0,640 sans marge.
        assert_abs_diff_eq!(
            first.close_fair.unwrap().get(),
            (1.0 / 1.5) / (1.0 / 1.5 + 0.25 + 0.125),
            epsilon = 1e-9
        );
    }

    #[test]
    fn the_model_generator_uses_the_model_probabilities_and_skips_unpredicted_games() {
        let soft = odds(2.2, 3.4, 4.8);
        let matches = [
            game(
                1,
                day(2025, 8, 16),
                Outcome::Away,
                &[("B365", Moment::Pre, soft)],
            ),
            game(
                2,
                day(2025, 8, 16),
                Outcome::Home,
                &[("B365", Moment::Pre, soft)],
            ),
        ];
        let refs: Vec<&GameInputs> = matches.iter().collect();
        // Le modèle ne prédit que le match 1 et y voit de la valeur sur l'extérieur (0,28 × 4,8 = 1,34).
        let predictions: HashMap<i64, Probs1x2> = [(1, Probs1x2::new(0.45, 0.27, 0.28).unwrap())]
            .into_iter()
            .collect();

        let candidates = model_vs_price(&refs, &predictions, "B365", &generator(0.0, 1.0));

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].game_id, 1);
        assert_eq!(candidates[0].outcome, Outcome::Away);
        assert!(candidates[0].won);
    }

    #[test]
    fn a_negative_threshold_returns_the_best_outcome_of_every_match_for_selection_diagnostics() {
        let sharp = odds(2.0, 1.0 / 0.3, 5.0);
        let tight = odds(1.9, 3.1, 4.6); // aucune valeur nulle part
        let matches = [game(
            1,
            day(2025, 8, 16),
            Outcome::Home,
            &[("PS", Moment::Pre, sharp), ("B365", Moment::Pre, tight)],
        )];
        let refs: Vec<&GameInputs> = matches.iter().collect();

        let all = soft_vs_sharp(&refs, "B365", &generator(-1.0, 1.0));

        assert_eq!(all.len(), 1);
        let edge = all[0].probability.get() * all[0].odds.get() - 1.0;
        assert!(edge < 0.0 && edge > -0.15);
    }

    #[test]
    fn periods_filter_games_by_date() {
        let games = vec![
            game(1, day(2020, 1, 1), Outcome::Home, &[]),
            game(2, day(2024, 10, 1), Outcome::Home, &[]),
            game(3, day(2026, 1, 1), Outcome::Home, &[]),
        ];
        assert_eq!(in_period(&games, Period::Train).len(), 1);
        assert_eq!(in_period(&games, Period::Validation)[0].game_id, 2);
        assert_eq!(in_period(&games, Period::Test)[0].game_id, 3);
    }
}
