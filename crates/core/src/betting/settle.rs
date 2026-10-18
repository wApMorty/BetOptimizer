//! Règlement des paris et des recommandations : le résultat d'un match vient de l'historique football-data importé.
//!
//! Le rapprochement entre un match de The Odds API et un match de football-data passe par la table d'alias, ligue par
//! ligue : un nom inconnu laisse le pari **non réglé** (jamais d'appariement approximatif). football-data publie ses
//! fichiers avec quelques jours de retard : un pari récent reste ouvert tant que `edge fd fetch` puis `edge fd import`
//! n'ont pas ramené le résultat, ou jusqu'à un règlement manuel.

use chrono::{DateTime, Days, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::Outcome;
use crate::error::Result;
use crate::ingest::aliases::Aliases;
use crate::ingest::football_data::LEAGUES;
use crate::store::journal::{self, Settlement, SettlementSource};

/// Résultat d'un match (`H`, `D`, `A`), ou `None` s'il n'est pas (encore) dans l'historique ou si un nom d'équipe est inconnu.
///
/// La date de football-data est la date locale publiée : on accepte un jour d'écart avec la date UTC du coup d'envoi.
pub fn result_of(
    conn: &Connection,
    aliases: &Aliases,
    sport_key: &str,
    home_raw: &str,
    away_raw: &str,
    kickoff: DateTime<Utc>,
) -> Result<Option<Outcome>> {
    let Some(league) = LEAGUES.iter().find(|league| league.sport_key == sport_key) else {
        return Ok(None);
    };
    let (Some(home), Some(away)) = (
        aliases.resolve(league.div, home_raw),
        aliases.resolve(league.div, away_raw),
    ) else {
        return Ok(None);
    };
    let day = kickoff.date_naive();
    let (Some(from), Some(to)) = (
        day.checked_sub_days(Days::new(1)),
        day.checked_add_days(Days::new(1)),
    ) else {
        return Ok(None);
    };
    let code: Option<String> = conn
        .query_row(
            "SELECT g.ft_result FROM game g
               JOIN team h ON h.id = g.home_id
               JOIN team a ON a.id = g.away_id
             WHERE g.div = ?1 AND h.name = ?2 AND a.name = ?3 AND g.match_date BETWEEN ?4 AND ?5",
            params![
                league.div,
                home,
                away,
                from.format("%Y-%m-%d").to_string(),
                to.format("%Y-%m-%d").to_string()
            ],
            |row| row.get(0),
        )
        .optional()?;
    Ok(code.as_deref().and_then(Outcome::from_code))
}

/// Bilan d'un règlement automatique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettleReport {
    /// Paris réglés pendant cette exécution.
    pub settled: usize,
    /// Paris encore ouverts : résultat absent de l'historique, ou nom d'équipe inconnu.
    pub still_open: usize,
}

/// Règle tous les paris ouverts dont le résultat est connu.
pub fn settle_open_bets(
    conn: &Connection,
    aliases: &Aliases,
    now: DateTime<Utc>,
) -> Result<SettleReport> {
    let mut report = SettleReport {
        settled: 0,
        still_open: 0,
    };
    for bet in journal::bets(conn)?.iter().filter(|b| b.result.is_none()) {
        let b = &bet.bet;
        match result_of(
            conn,
            aliases,
            &b.sport_key,
            &b.home_team,
            &b.away_team,
            b.kickoff,
        )? {
            Some(actual) => {
                let settlement = if actual == b.outcome {
                    Settlement::Won
                } else {
                    Settlement::Lost
                };
                if journal::insert_result(
                    conn,
                    bet,
                    settlement,
                    SettlementSource::FootballData,
                    now,
                )? {
                    report.settled += 1;
                }
            }
            None => report.still_open += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::testing::{at, database};
    use crate::domain::{Euros, Odds};
    use crate::store::journal::{NewBet, bets, insert_bet};

    fn aliases() -> Aliases {
        Aliases::parse("[SC0]\n\"Falkirk\" = [\"Falkirk FC\"]\n\"Kilmarnock\" = []\n").unwrap()
    }

    /// Une base avec le match Falkirk – Kilmarnock du 24 octobre (2-1, victoire à domicile).
    fn with_result() -> Connection {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'Falkirk'), (2, 'Kilmarnock');
             INSERT INTO game (div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
             VALUES ('SC0', '2026/27', '2026-10-24', 1, 2, 2, 1, 'H');",
        )
        .unwrap();
        conn
    }

    fn bet(outcome: Outcome, home: &str) -> NewBet {
        NewBet {
            recommendation_id: None,
            event_id: "m1".into(),
            sport_key: "soccer_spl".into(),
            home_team: home.into(),
            away_team: "Kilmarnock".into(),
            kickoff: at("2026-10-24T14:00:00Z"),
            bookmaker: "betclic_fr".into(),
            outcome,
            odds: Odds::new(2.2).unwrap(),
            stake: Euros::new(50.0).unwrap(),
        }
    }

    #[test]
    fn the_result_is_found_through_the_alias_table_and_a_one_day_tolerance() {
        let conn = with_result();

        // Coup d'envoi tard le 24 au soir UTC, date locale publiée le 25 : un jour d'écart toléré.
        let found = result_of(
            &conn,
            &aliases(),
            "soccer_spl",
            "Falkirk FC",
            "Kilmarnock",
            at("2026-10-25T00:30:00Z"),
        )
        .unwrap();

        assert_eq!(found, Some(Outcome::Home));
    }

    #[test]
    fn an_unknown_team_or_league_or_a_missing_game_gives_none() {
        let conn = with_result();
        let aliases = aliases();
        let kickoff = at("2026-10-24T14:00:00Z");

        assert_eq!(
            result_of(
                &conn,
                &aliases,
                "soccer_spl",
                "Inconnu",
                "Kilmarnock",
                kickoff
            )
            .unwrap(),
            None
        );
        assert_eq!(
            result_of(
                &conn,
                &aliases,
                "soccer_unknown",
                "Falkirk",
                "Kilmarnock",
                kickoff
            )
            .unwrap(),
            None
        );
        assert_eq!(
            result_of(
                &conn,
                &aliases,
                "soccer_spl",
                "Falkirk",
                "Kilmarnock",
                at("2026-11-30T14:00:00Z")
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn open_bets_are_settled_won_or_lost_from_the_result() {
        let conn = with_result();
        insert_bet(
            &conn,
            &bet(Outcome::Home, "Falkirk"),
            at("2026-10-24T10:00:00Z"),
        )
        .unwrap();
        insert_bet(
            &conn,
            &bet(Outcome::Draw, "Falkirk"),
            at("2026-10-24T10:00:00Z"),
        )
        .unwrap();

        let report = settle_open_bets(&conn, &aliases(), at("2026-10-26T09:00:00Z")).unwrap();

        assert_eq!(
            report,
            SettleReport {
                settled: 2,
                still_open: 0
            }
        );
        let all = bets(&conn).unwrap();
        let won = all[0].result.as_ref().unwrap();
        assert_eq!(won.settlement, Settlement::Won);
        assert!((won.profit - 60.0).abs() < 1e-9);
        assert_eq!(all[1].result.as_ref().unwrap().settlement, Settlement::Lost);
        assert_eq!(won.source, SettlementSource::FootballData);
    }

    #[test]
    fn a_bet_without_a_known_result_stays_open_and_a_second_pass_changes_nothing() {
        let conn = database();
        insert_bet(
            &conn,
            &bet(Outcome::Home, "Falkirk"),
            at("2026-10-24T10:00:00Z"),
        )
        .unwrap();

        let first = settle_open_bets(&conn, &aliases(), at("2026-10-26T09:00:00Z")).unwrap();

        assert_eq!(
            first,
            SettleReport {
                settled: 0,
                still_open: 1
            }
        );
        assert!(bets(&conn).unwrap()[0].result.is_none());

        // Le résultat arrive, puis on repasse deux fois : le règlement n'est écrit qu'une fois.
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'Falkirk'), (2, 'Kilmarnock');
             INSERT INTO game (div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
             VALUES ('SC0', '2026/27', '2026-10-24', 1, 2, 0, 0, 'D');",
        )
        .unwrap();
        let second = settle_open_bets(&conn, &aliases(), at("2026-10-27T09:00:00Z")).unwrap();
        let third = settle_open_bets(&conn, &aliases(), at("2026-10-28T09:00:00Z")).unwrap();

        assert_eq!(second.settled, 1);
        assert_eq!(
            third,
            SettleReport {
                settled: 0,
                still_open: 0
            }
        );
    }
}
