//! Chargement des matchs et des cotes 1X2 d'un bookmaker, prêts pour l'évaluation du marché.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;

use crate::domain::{Moment, Odds, Odds1x2, Outcome};
use crate::error::Result;

/// Un match terminé avec les cotes 1X2 d'un bookmaker à un moment donné.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketGame {
    /// Identifiant du match en base.
    pub game_id: i64,
    /// Code football-data de la ligue.
    pub div: String,
    /// Saison (`2025/26`).
    pub season: String,
    /// Bloc de bootstrap : une ligue et une semaine (la « journée » des intervalles de confiance).
    pub block: u32,
    /// Résultat réel.
    pub outcome: Outcome,
    /// Cotes du bookmaker.
    pub odds: Odds1x2,
}

/// Charge tous les matchs pour lesquels `bookmaker` a des cotes au `moment` donné, par ordre chronologique.
pub fn load(conn: &Connection, bookmaker: &str, moment: Moment) -> Result<Vec<MarketGame>> {
    let mut statement = conn.prepare(
        "SELECT g.id, g.div, g.season, g.match_date, g.ft_result, o.home, o.draw, o.away
           FROM game g
           JOIN game_odds o ON o.game_id = g.id AND o.bookmaker = ?1 AND o.moment = ?2
          ORDER BY g.match_date, g.id",
    )?;
    let rows = statement.query_map([bookmaker, moment.as_str()], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            [
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
            ],
        ))
    })?;

    let mut blocks: HashMap<(String, i32), u32> = HashMap::new();
    let mut games = Vec::new();
    for row in rows {
        let (game_id, div, season, date, result, prices) = row?;
        let outcome = match result.as_str() {
            "H" => Outcome::Home,
            "D" => Outcome::Draw,
            _ => Outcome::Away,
        };
        // `let … else` : une cote invalide (impossible, la base l'interdit) écarterait simplement ce match.
        let [Some(home), Some(draw), Some(away)] = prices.map(Odds::new) else {
            continue;
        };
        // Semaine du lundi au dimanche : `num_days_from_ce` vaut 1 pour le lundi 1er janvier de l'an 1,
        // donc `(jours − 1) / 7` change de valeur chaque lundi. Samedi et dimanche partagent le même bloc.
        let week = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map(|d| (d.num_days_from_ce() - 1) / 7)
            .unwrap_or(0);
        let next_block = blocks.len() as u32;
        let block = *blocks.entry((div.clone(), week)).or_insert(next_block);
        games.push(MarketGame {
            game_id,
            div,
            season,
            block,
            outcome,
            odds: Odds1x2 { home, draw, away },
        });
    }
    Ok(games)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;

    #[test]
    fn games_are_loaded_with_outcome_odds_and_weekly_blocks() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'A'), (2, 'B');
             INSERT INTO game (id, div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result) VALUES
               (1, 'E0', '2025/26', '2025-08-16', 1, 2, 2, 0, 'H'),
               (2, 'E0', '2025/26', '2025-08-17', 2, 1, 1, 1, 'D'),
               (3, 'E0', '2025/26', '2025-08-30', 1, 2, 0, 1, 'A'),
               (4, 'SC0', '2025/26', '2025-08-16', 1, 2, 0, 1, 'A');
             INSERT INTO game_odds VALUES
               (1, 'PS', 'close', 1.5, 4.0, 7.0), (2, 'PS', 'close', 2.0, 3.5, 4.0),
               (3, 'PS', 'close', 2.2, 3.4, 3.6), (4, 'PS', 'close', 2.2, 3.4, 3.6),
               (1, 'PS', 'pre',   1.6, 4.0, 6.0);",
        )
        .unwrap();

        let games = load(&conn, "PS", Moment::Close).unwrap();
        let by_id = |id: i64| games.iter().find(|g| g.game_id == id).unwrap();

        assert_eq!(games.len(), 4);
        assert_eq!(by_id(1).outcome, Outcome::Home);
        assert_eq!(by_id(1).odds.home.get(), 1.5);
        assert_eq!(by_id(2).outcome, Outcome::Draw);
        assert_eq!(by_id(3).outcome, Outcome::Away);
        // Ordre chronologique : 16/08 (deux ligues), 17/08, 30/08.
        assert_eq!(games.last().unwrap().game_id, 3);
        // Jeu de données par moment : un seul match a une cote d'avant-match.
        assert_eq!(load(&conn, "PS", Moment::Pre).unwrap().len(), 1);
        // Même ligue, même semaine : même bloc ; autre semaine ou autre ligue : autre bloc.
        assert_eq!(by_id(1).block, by_id(2).block);
        assert_ne!(by_id(1).block, by_id(3).block);
        assert_ne!(by_id(1).block, by_id(4).block);
    }
}
