//! Enregistrement des paris réels : les règles communes à la ligne de commande et à l'application.
//!
//! Un pari **suit** une recommandation (sa cote n'est pas sous la cote minimale) ou il est **hors modèle**. Ce module
//! valide la saisie, construit le pari et l'écrit dans le journal immuable ; il ne contacte aucun bookmaker.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::live::FOOTBALL_ONLY;
use crate::domain::{Euros, Odds, Outcome};
use crate::error::{Error, Result};
use crate::store::journal::{self, NewBet};

fn refuse<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Input(message.into()))
}

fn valid_odds(value: f64) -> Result<Odds> {
    Odds::new(value).map_or_else(|| refuse("la cote doit être supérieure à 1"), Ok)
}

fn valid_stake(value: f64) -> Result<Euros> {
    match Euros::new(value) {
        Some(stake) if stake.get() > 0.0 => Ok(stake),
        _ => refuse("la mise doit être strictement positive"),
    }
}

/// Ce que l'utilisateur précise pour un pari qui suit une recommandation ; l'absence reprend la recommandation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FollowedInput {
    /// Cote réellement obtenue.
    pub odds: Option<f64>,
    /// Mise réellement placée.
    pub stake: Option<f64>,
    /// Bookmaker chez qui le pari est placé.
    pub bookmaker: Option<String>,
}

/// Construit le pari qui suit la recommandation `recommendation_id`, sans l'écrire.
///
/// Refus : recommandation introuvable ou déjà suivie ; cote sous la cote minimale (le pari ne correspond plus à la
/// recommandation : il doit être saisi hors modèle) ; recommandation sans mise suggérée et mise non précisée.
pub fn followed_bet(
    conn: &Connection,
    recommendation_id: i64,
    input: &FollowedInput,
) -> Result<NewBet> {
    let Some(stored) = journal::recommendation(conn, recommendation_id)? else {
        return refuse(format!("recommandation n° {recommendation_id} introuvable"));
    };
    let rec = stored.record;
    if journal::bets(conn)?
        .iter()
        .any(|b| b.bet.recommendation_id == Some(recommendation_id))
    {
        return refuse(format!(
            "un pari est déjà enregistré pour la recommandation n° {recommendation_id}"
        ));
    }
    let odds_value = input.odds.unwrap_or(rec.odds.get());
    let odds = valid_odds(odds_value)?;
    if odds_value < rec.minimal_odds {
        return refuse(format!(
            "la cote {} est sous la cote minimale {} : ce pari ne suit plus la recommandation. Enregistre-le comme pari hors modèle (match {})",
            crate::format::decimal(odds_value, 2),
            crate::format::decimal(rec.minimal_odds, 2),
            rec.event_id
        ));
    }
    let stake_value = match input.stake.or(rec.stake.map(Euros::get)) {
        Some(value) => value,
        None => return refuse("la recommandation n'a pas de mise suggérée : précise la mise"),
    };
    Ok(NewBet {
        recommendation_id: Some(recommendation_id),
        event_id: rec.event_id,
        sport_key: rec.sport_key,
        home_team: rec.home_team,
        away_team: rec.away_team,
        kickoff: rec.kickoff,
        bookmaker: input.bookmaker.clone().unwrap_or(rec.bookmaker),
        outcome: rec.outcome,
        odds,
        stake: valid_stake(stake_value)?,
    })
}

/// Construit un pari hors modèle sur un match de l'archive, sans l'écrire.
pub fn off_model_bet(
    conn: &Connection,
    event_id: &str,
    bookmaker: &str,
    outcome: Outcome,
    odds: f64,
    stake: f64,
) -> Result<NewBet> {
    let Some(event) = journal::archived_event(conn, event_id)? else {
        return refuse(format!("match {event_id} introuvable dans l'archive"));
    };
    if bookmaker.trim().is_empty() {
        return refuse("le bookmaker est requis");
    }
    Ok(NewBet {
        recommendation_id: None,
        event_id: event_id.to_string(),
        sport_key: event.sport_key,
        home_team: event.home_team,
        away_team: event.away_team,
        kickoff: event.kickoff,
        bookmaker: bookmaker.trim().to_string(),
        outcome,
        odds: valid_odds(odds)?,
        stake: valid_stake(stake)?,
    })
}

/// Écrit le pari dans le journal et renvoie son numéro.
pub fn record(conn: &Connection, bet: &NewBet, now: DateTime<Utc>) -> Result<i64> {
    journal::insert_bet(conn, bet, now)
}

/// Un match à venir connu de l'archive, proposé dans le formulaire des paris hors modèle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpcomingEvent {
    /// Identifiant chez The Odds API.
    pub event_id: String,
    /// Compétition.
    pub sport_key: String,
    /// Équipe à domicile, nom brut.
    pub home_team: String,
    /// Équipe à l'extérieur, nom brut.
    pub away_team: String,
    /// Coup d'envoi (UTC).
    pub kickoff: DateTime<Utc>,
}

/// Les matchs dont le coup d'envoi est dans le futur, vus dans le relevé le plus récent de chacun, du plus proche au plus lointain.
pub fn upcoming_events(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<UpcomingEvent>> {
    let mut statement = conn.prepare(&format!(
        "SELECT e.event_id, e.sport_key, e.home_team_raw, e.away_team_raw, e.commence_time
         FROM event_seen e
         WHERE e.commence_time > ?1 AND {FOOTBALL_ONLY}
           AND e.api_call_id = (SELECT MAX(api_call_id) FROM event_seen WHERE event_id = e.event_id)
         ORDER BY e.commence_time, e.event_id"
    ))?;
    let rows = statement.query_map([journal::timestamp(now)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut events = Vec::new();
    for row in rows {
        let (event_id, sport_key, home_team, away_team, kickoff) = row?;
        events.push(UpcomingEvent {
            event_id,
            sport_key,
            home_team,
            away_team,
            kickoff: journal::parse_timestamp(&kickoff)?,
        });
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::recommend::{RECOMMEND_MIN_EDGE, Settings, run};
    use crate::betting::testing::{NOW, PINNACLE, at, database, event, event_named, store};
    use crate::market::margin::MarginMethod;

    /// Une base avec une recommandation notifiée (Betclic à 2,30 contre Pinnacle à 2,00) : mise 91,4 €, cote minimale 2,12.
    fn with_recommendation() -> Connection {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );
        let settings = Settings {
            method: MarginMethod::Multiplicative,
            min_edge: RECOMMEND_MIN_EDGE,
            kelly_fraction: 1.0,
            min_stake: Euros::new(1.0).unwrap(),
            bankroll: Euros::new(1000.0).unwrap(),
            code_version: "test".into(),
        };
        run(&mut conn, &settings, at(NOW)).unwrap();
        conn
    }

    fn message(error: Error) -> String {
        assert!(matches!(error, Error::Input(_)), "{error:?}");
        error.to_string()
    }

    #[test]
    fn a_followed_bet_takes_the_odds_and_the_stake_of_the_recommendation_by_default() {
        let conn = with_recommendation();

        let bet = followed_bet(&conn, 1, &FollowedInput::default()).unwrap();

        assert_eq!(bet.recommendation_id, Some(1));
        assert_eq!(bet.bookmaker, "betclic_fr");
        assert_eq!(bet.odds.get(), 2.3);
        assert!((bet.stake.get() - 91.4).abs() < 0.2, "{}", bet.stake.get());
    }

    #[test]
    fn the_user_can_override_the_odds_the_stake_and_the_bookmaker() {
        let conn = with_recommendation();
        let input = FollowedInput {
            odds: Some(2.25),
            stake: Some(40.0),
            bookmaker: Some("winamax_fr".into()),
        };

        let bet = followed_bet(&conn, 1, &input).unwrap();

        assert_eq!((bet.odds.get(), bet.stake.get()), (2.25, 40.0));
        assert_eq!(bet.bookmaker, "winamax_fr");
    }

    #[test]
    fn odds_below_the_minimum_are_refused_with_the_way_out() {
        let conn = with_recommendation();
        let input = FollowedInput {
            odds: Some(2.05),
            ..FollowedInput::default()
        };

        let text = message(followed_bet(&conn, 1, &input).unwrap_err());

        assert!(text.contains("cote minimale"));
        assert!(text.contains("hors modèle"));
    }

    #[test]
    fn a_missing_recommendation_a_second_bet_and_a_missing_stake_are_refused() {
        let conn = with_recommendation();
        assert!(
            message(followed_bet(&conn, 99, &FollowedInput::default()).unwrap_err())
                .contains("introuvable")
        );

        let bet = followed_bet(&conn, 1, &FollowedInput::default()).unwrap();
        record(&conn, &bet, at(NOW)).unwrap();
        assert!(
            message(followed_bet(&conn, 1, &FollowedInput::default()).unwrap_err())
                .contains("déjà enregistré")
        );

        // Journal fantôme : aucune mise suggérée, la mise doit être précisée.
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("pmu_fr", 2.08, 3.5, 3.8)])],
        );
        let settings = Settings {
            method: MarginMethod::Multiplicative,
            min_edge: RECOMMEND_MIN_EDGE,
            kelly_fraction: 1.0,
            min_stake: Euros::new(1.0).unwrap(),
            bankroll: Euros::new(1000.0).unwrap(),
            code_version: "test".into(),
        };
        run(&mut conn, &settings, at(NOW)).unwrap();
        assert!(
            message(followed_bet(&conn, 1, &FollowedInput::default()).unwrap_err())
                .contains("mise")
        );
    }

    #[test]
    fn non_positive_stakes_and_odds_are_refused() {
        let conn = with_recommendation();
        for (odds, stake) in [
            (Some(1.0), None),
            (Some(f64::NAN), None),
            (None, Some(0.0)),
            (None, Some(-5.0)),
        ] {
            let input = FollowedInput {
                odds,
                stake,
                bookmaker: None,
            };
            assert!(
                followed_bet(&conn, 1, &input).is_err(),
                "{odds:?} {stake:?}"
            );
        }
    }

    #[test]
    fn an_off_model_bet_is_built_from_the_archived_match() {
        let conn = with_recommendation();

        let bet = off_model_bet(&conn, "match-1", " winamax_fr ", Outcome::Draw, 3.4, 5.0).unwrap();

        assert_eq!(bet.recommendation_id, None);
        assert_eq!(bet.bookmaker, "winamax_fr");
        assert_eq!(
            (bet.home_team.as_str(), bet.outcome),
            ("Falkirk", Outcome::Draw)
        );
        assert!(
            message(off_model_bet(&conn, "inconnu", "x", Outcome::Draw, 3.4, 5.0).unwrap_err())
                .contains("introuvable")
        );
        assert!(
            message(off_model_bet(&conn, "match-1", "  ", Outcome::Draw, 3.4, 5.0).unwrap_err())
                .contains("bookmaker")
        );
    }

    #[test]
    fn tennis_matches_are_not_offered_for_off_model_bets() {
        let mut conn = database();
        let mut tennis = event_named("tennis-1", "2026-10-24T14:00:00Z", &[PINNACLE]);
        tennis.sport_key = "tennis_atp_us_open".into();
        store(&mut conn, "2026-10-18T10:00:00Z", &[tennis]);

        assert!(
            upcoming_events(&conn, at("2026-10-18T15:00:00Z"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn upcoming_events_lists_only_future_matches_from_their_latest_snapshot() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T10:00:00Z",
            &[
                event_named("late", "2026-10-26T14:00:00Z", &[PINNACLE]),
                event_named("soon", "2026-10-24T14:00:00Z", &[PINNACLE]),
                event_named("past", "2026-10-18T14:30:00Z", &[PINNACLE]),
            ],
        );

        let events = upcoming_events(&conn, at("2026-10-18T15:00:00Z")).unwrap();

        let ids: Vec<&str> = events.iter().map(|e| e.event_id.as_str()).collect();
        assert_eq!(ids, vec!["soon", "late"]);
    }
}
