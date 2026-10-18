//! Opportunités courantes : le dernier relevé de l'archive, chaque prix confronté à Pinnacle sans marge.
//!
//! Lecture seule : rien n'est écrit en base. Pour chaque match à venir, la probabilité de référence est celle de
//! Pinnacle débarrassée de sa marge **au même relevé** ; chaque autre bookmaker est comparé à elle. Au plus une
//! sélection par match (règle du projet) : l'issue et le bookmaker de plus forte espérance, si elle atteint le seuil.
//!
//! Ce n'est pas un modèle : la valeur vient d'un prix en retard sur le marché le plus précis, pas d'une prévision.
//! Aucun seuil n'est validé pour les opérateurs français (voir `edge backtest report`) : les résultats sont donc
//! informatifs, et c'est à l'appelant de le dire.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};

use super::kelly::stake;
use super::simultaneous::{Bet, fractions};
use super::value::{Selection, best_selection, minimal_odds};
use crate::domain::{Euros, Odds, Odds1x2, Outcome};
use crate::error::Result;
use crate::market::margin::{MarginMethod, remove_margin};
use crate::store::journal::{parse_timestamp, timestamp};

/// Bookmaker de référence : ses probabilités, sans marge, tiennent lieu de probabilités vraies.
pub const SHARP_BOOKMAKER: &str = "pinnacle";

/// Âge maximal (en heures) du relevé d'un match. Au-delà, le prix n'est plus celui du marché : le match est ignoré.
/// Le timer prend un relevé par jour ; 30 h tolère un retard sans laisser passer un relevé manqué.
pub const MAX_SNAPSHOT_AGE_HOURS: i64 = 30;

/// Condition SQL qui ne garde que le football : le 1X2 suppose un nul, que le tennis n'a pas. Les tournois de tennis sont
/// archivés (pour juger plus tard un avantage), mais ni lus ni recommandés ici. `\_` : le tiret bas, joker de `LIKE`, est littéral.
pub(crate) const FOOTBALL_ONLY: &str = "e.sport_key LIKE 'soccer\\_%' ESCAPE '\\'";

/// Marge totale plausible d'un 1X2 (somme des probabilités implicites) ; hors de cet intervalle, le prix est signalé
/// et ignoré, jamais corrigé (CLAUDE.md, « cotes aberrantes »).
const IMPLIED_SUM_RANGE: (f64, f64) = (1.0, 1.30);

/// Un pari à étudier : un prix qui dépasse la cote juste de Pinnacle d'au moins le seuil demandé.
#[derive(Debug, Clone, PartialEq)]
pub struct Opportunity {
    /// Identifiant du match chez The Odds API.
    pub event_id: String,
    /// Relevé (ligne de `api_call`) d'où viennent les prix.
    pub api_call_id: i64,
    /// Compétition (clé The Odds API).
    pub sport_key: String,
    /// Équipe à domicile, nom brut de l'API.
    pub home_team: String,
    /// Équipe à l'extérieur, nom brut de l'API.
    pub away_team: String,
    /// Coup d'envoi (UTC).
    pub kickoff: DateTime<Utc>,
    /// Bookmaker qui propose le prix.
    pub bookmaker: String,
    /// Issue retenue, cote, probabilité (Pinnacle sans marge) et espérance.
    pub selection: Selection,
    /// Cote en dessous de laquelle l'espérance tombe sous le seuil.
    pub minimal_odds: f64,
    /// Plus ancienne mise à jour du prix parmi les trois issues, telle que l'API la rapporte (UTC) : l'âge du prix.
    pub priced_at: DateTime<Utc>,
    /// Mise suggérée par [`size_stakes`] ; `None` tant qu'elle n'est pas calculée ou si elle tombe sous la mise minimale.
    pub stake: Option<Euros>,
}

/// Résultat d'une lecture de l'archive.
#[derive(Debug, Clone, PartialEq)]
pub struct Scan {
    /// Matchs à venir présents dans un relevé assez récent.
    pub events: usize,
    /// Matchs ignorés faute d'un 1X2 complet et plausible de Pinnacle dans le relevé.
    pub without_reference: usize,
    /// Prix ignorés parce que la somme des probabilités implicites est hors de `[1,00 ; 1,30]`.
    pub implausible_quotes: usize,
    /// Un pari au plus par match, du plus proche coup d'envoi au plus lointain.
    pub opportunities: Vec<Opportunity>,
}

/// Les cotes d'un bookmaker pour un match, à mesure qu'on lit les lignes de la table `quote`.
#[derive(Default)]
struct BookQuotes {
    prices: [Option<Odds>; 3],
    /// Plus ancienne mise à jour parmi les trois issues : le prix vaut ce que vaut son plus vieux élément.
    oldest_update: Option<DateTime<Utc>>,
}

impl BookQuotes {
    fn complete(&self) -> Option<Odds1x2> {
        match self.prices {
            [Some(home), Some(draw), Some(away)] => Some(Odds1x2 { home, draw, away }),
            _ => None,
        }
    }
}

/// Lit les derniers relevés et cherche les prix qui dépassent la cote juste de Pinnacle d'au moins `min_edge`.
///
/// Un match n'est lu que dans son relevé le plus récent, et seulement si ce relevé a moins de
/// [`MAX_SNAPSHOT_AGE_HOURS`] heures et si le coup d'envoi est dans le futur.
pub fn scan(
    conn: &Connection,
    method: MarginMethod,
    min_edge: f64,
    now: DateTime<Utc>,
) -> Result<Scan> {
    let oldest_snapshot = now - Duration::hours(MAX_SNAPSHOT_AGE_HOURS);
    let mut events = conn.prepare(&format!(
        "SELECT e.event_id, e.api_call_id, e.sport_key, e.commence_time, e.home_team_raw, e.away_team_raw
         FROM event_seen e JOIN api_call a ON a.id = e.api_call_id
         WHERE e.commence_time > ?1 AND a.called_at >= ?2 AND {FOOTBALL_ONLY}
           AND e.api_call_id = (SELECT MAX(api_call_id) FROM event_seen WHERE event_id = e.event_id)
         ORDER BY e.commence_time, e.event_id"
    ))?;
    let mut quotes = conn.prepare(
        "SELECT bookmaker_key, outcome_raw, price, quote_updated_at FROM quote
         WHERE api_call_id = ?1 AND event_id = ?2 AND market_key = 'h2h'",
    )?;

    let mut scan = Scan {
        events: 0,
        without_reference: 0,
        implausible_quotes: 0,
        opportunities: Vec::new(),
    };
    let rows = events.query_map(params![timestamp(now), timestamp(oldest_snapshot)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (event_id, call_id, sport_key, kickoff, home, away) = row?;
        scan.events += 1;

        // BTreeMap : itération par clé de bookmaker, donc départage déterministe des égalités.
        let mut books: BTreeMap<String, BookQuotes> = BTreeMap::new();
        let lines = quotes.query_map(params![call_id, event_id], |line| {
            Ok((
                line.get::<_, String>(0)?,
                line.get::<_, String>(1)?,
                line.get::<_, f64>(2)?,
                line.get::<_, String>(3)?,
            ))
        })?;
        for line in lines {
            let (bookmaker, outcome_name, price, updated_at) = line?;
            // Appariement par nom exact : l'API renvoie les issues dans l'ordre alphabétique, jamais domicile/nul/extérieur.
            let outcome = if outcome_name == home {
                Outcome::Home
            } else if outcome_name == away {
                Outcome::Away
            } else if outcome_name == "Draw" {
                Outcome::Draw
            } else {
                continue;
            };
            let book = books.entry(bookmaker).or_default();
            book.prices[outcome.index()] = Odds::new(price);
            let updated = parse_timestamp(&updated_at)?;
            book.oldest_update = Some(book.oldest_update.map_or(updated, |t| t.min(updated)));
        }

        let reference = books
            .get(SHARP_BOOKMAKER)
            .and_then(BookQuotes::complete)
            .filter(plausible);
        let Some(fair) = reference.and_then(|odds| remove_margin(&odds, method)) else {
            scan.without_reference += 1;
            continue;
        };

        let mut best: Option<Opportunity> = None;
        for (bookmaker, book) in &books {
            if bookmaker == SHARP_BOOKMAKER {
                continue;
            }
            let (Some(odds), Some(priced_at)) = (book.complete(), book.oldest_update) else {
                continue;
            };
            if !plausible(&odds) {
                scan.implausible_quotes += 1;
                continue;
            }
            let Some(selection) = best_selection(&fair, &odds, min_edge) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|b| selection.edge > b.selection.edge)
            {
                best = Some(Opportunity {
                    event_id: event_id.clone(),
                    api_call_id: call_id,
                    sport_key: sport_key.clone(),
                    home_team: home.clone(),
                    away_team: away.clone(),
                    kickoff: parse_timestamp(&kickoff)?,
                    bookmaker: bookmaker.clone(),
                    minimal_odds: minimal_odds(selection.probability, min_edge),
                    selection,
                    priced_at,
                    stake: None,
                });
            }
        }
        scan.opportunities.extend(best);
    }
    Ok(scan)
}

/// Calcule les mises suggérées d'un lot : Kelly simultané (les paris partagent la même bankroll), multiplié par la
/// fraction de Kelly choisie, arrondi à la baisse au dixième d'euro. Sous `min_stake`, le pari n'est pas suggéré.
pub fn size_stakes(
    opportunities: &mut [Opportunity],
    bankroll: Euros,
    kelly_fraction: f64,
    min_stake: Euros,
) -> Result<()> {
    let bets: Vec<Bet> = opportunities
        .iter()
        .map(|o| Bet {
            probability: o.selection.probability,
            odds: o.selection.odds,
        })
        .collect();
    let shares = fractions(&bets)?;
    for (opportunity, share) in opportunities.iter_mut().zip(shares) {
        opportunity.stake = stake(bankroll, share * kelly_fraction, min_stake);
    }
    Ok(())
}

fn plausible(odds: &Odds1x2) -> bool {
    (IMPLIED_SUM_RANGE.0..=IMPLIED_SUM_RANGE.1).contains(&odds.implied_sum())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::testing::{NOW, PINNACLE, at, database, event, store};

    #[test]
    fn tennis_events_in_the_archive_are_neither_read_nor_counted_as_football_without_reference() {
        let mut conn = database();
        let mut tennis = crate::betting::testing::event_named(
            "tennis-1",
            "2026-10-24T14:00:00Z",
            &[PINNACLE, ("winamax_fr", 2.3, 3.2, 3.4)],
        );
        tennis.sport_key = "tennis_wta_ningbo_open".into();
        store(&mut conn, "2026-10-18T14:00:00Z", &[tennis]);

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, at(NOW)).unwrap();

        assert_eq!(scan.events, 0);
        assert_eq!(scan.without_reference, 0);
        assert!(scan.opportunities.is_empty());
    }

    #[test]
    fn a_price_above_the_fair_odds_is_found_and_the_minimal_odds_follow() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.03, at(NOW)).unwrap();

        assert_eq!(scan.events, 1);
        assert_eq!(scan.opportunities.len(), 1);
        let found = &scan.opportunities[0];
        assert_eq!(found.bookmaker, "betclic_fr");
        assert_eq!(found.selection.outcome, Outcome::Home);
        // p = (1/2,00) / 1,02778 ≈ 0,486486 ; espérance = p × 2,30 − 1 ≈ +11,9 %.
        assert!((found.selection.probability.get() - 0.486_486).abs() < 1e-5);
        assert!((found.selection.edge - 0.118_92).abs() < 1e-4);
        // Cote minimale = 1,03 / p ≈ 2,117.
        assert!((found.minimal_odds - 1.03 / 0.486_486).abs() < 1e-3);
    }

    #[test]
    fn only_the_best_bookmaker_and_outcome_of_a_match_is_kept() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[
                PINNACLE,
                ("betclic_fr", 2.3, 3.2, 3.4),
                ("winamax_fr", 2.4, 3.2, 3.4),
            ])],
        );

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, at(NOW)).unwrap();

        assert_eq!(scan.opportunities.len(), 1);
        assert_eq!(scan.opportunities[0].bookmaker, "winamax_fr");
    }

    #[test]
    fn nothing_is_recommended_below_the_threshold_or_for_pinnacle_itself() {
        let mut conn = database();
        // Betclic légèrement sous Pinnacle partout : aucune espérance positive.
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 1.95, 3.5, 3.9)])],
        );

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, at(NOW)).unwrap();

        assert_eq!(scan.events, 1);
        assert!(scan.opportunities.is_empty());
    }

    #[test]
    fn a_match_without_a_complete_pinnacle_line_has_no_reference_and_is_counted() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[("betclic_fr", 2.3, 3.2, 3.4)])],
        );

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, at(NOW)).unwrap();

        assert_eq!(scan.without_reference, 1);
        assert!(scan.opportunities.is_empty());
    }

    #[test]
    fn an_implausible_price_is_flagged_and_never_used() {
        let mut conn = database();
        // Somme des probabilités implicites ≈ 0,54 : impossible (pari sans risque), donc probablement faux.
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 4.0, 6.0, 8.0)])],
        );

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, at(NOW)).unwrap();

        assert_eq!(scan.implausible_quotes, 1);
        assert!(scan.opportunities.is_empty());
    }

    #[test]
    fn a_stale_snapshot_and_a_match_already_started_are_ignored() {
        let mut conn = database();
        // Relevé vieux de plus de 30 h.
        store(
            &mut conn,
            "2026-10-17T06:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );

        let stale = scan(&conn, MarginMethod::Multiplicative, 0.0, at(NOW)).unwrap();
        assert_eq!(stale.events, 0);

        // Relevé récent, mais on se place après le coup d'envoi.
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-24T13:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );
        let started = scan(
            &conn,
            MarginMethod::Multiplicative,
            0.0,
            at("2026-10-24T14:30:00Z"),
        )
        .unwrap();
        assert_eq!(started.events, 0);
    }

    #[test]
    fn only_the_latest_snapshot_of_a_match_counts() {
        let mut conn = database();
        // Premier relevé : valeur. Second relevé (plus récent) : le prix de Betclic a été corrigé.
        store(
            &mut conn,
            "2026-10-18T10:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 1.95, 3.5, 3.9)])],
        );

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, at(NOW)).unwrap();

        assert_eq!(scan.events, 1);
        assert!(scan.opportunities.is_empty());
    }

    #[test]
    fn stakes_are_simultaneous_rounded_down_and_scaled_by_the_kelly_fraction() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );
        let mut found = scan(&conn, MarginMethod::Multiplicative, 0.03, at(NOW))
            .unwrap()
            .opportunities;
        let bankroll = Euros::new(1000.0).unwrap();
        let min_stake = Euros::new(1.0).unwrap();

        size_stakes(&mut found, bankroll, 1.0, min_stake).unwrap();
        let full = found[0].stake.unwrap().get();
        size_stakes(&mut found, bankroll, 0.5, min_stake).unwrap();
        let half = found[0].stake.unwrap().get();

        // Kelly d'un pari seul : (p·cote − 1)/(cote − 1) ≈ 0,1189/1,30 ≈ 9,1 % ; un seul pari, donc le lot ne change rien.
        assert!((full - 91.4).abs() < 1.0, "{full}");
        assert!((half - full / 2.0).abs() <= 0.1 + 1e-9, "{half} vs {full}");
        // Arrondi à la baisse au dixième.
        assert!(((full * 10.0).round() - full * 10.0).abs() < 1e-6);

        // Sous la mise minimale : pas de mise suggérée.
        size_stakes(&mut found, bankroll, 0.001, min_stake).unwrap();
        assert_eq!(found[0].stake, None);
    }
}
