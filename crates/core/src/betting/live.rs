//! Opportunités courantes : le dernier relevé de l'archive, chaque prix confronté à Pinnacle sans marge.
//!
//! Lecture seule : rien n'est écrit en base. Pour chaque match à venir, la probabilité de référence est celle de
//! Pinnacle débarrassée de sa marge **au même relevé** ; chaque autre bookmaker est comparé à elle. Au plus une
//! sélection par match (règle du projet) : l'issue et le bookmaker de plus forte espérance, si elle atteint le seuil.
//!
//! Deux gardes sur la référence, ajoutées après l'audit du 2026-11-12 (`docs/sources.md` §73) : un match n'est examiné
//! qu'à [`MAX_HORIZON_HOURS`] heures au plus de son coup d'envoi, et la référence est écartée quand sa propre marge
//! dépasse [`SHARP_IMPLIED_SUM_MAX`]. Plus loin du match, Pinnacle prend plus de marge, bouge rarement puis saute : son
//! prix n'y tient plus lieu de probabilité vraie.
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

/// Horizon maximal (en heures) d'un match examiné. Au-delà, le match est compté ([`Scan::too_far`]) mais ni comparé ni
/// recommandé, quel que soit le palier du journal.
///
/// Mesure sur l'archive (2 850 relevés de Pinnacle, `docs/sources.md` §73) : surronde médiane de 1,030 à moins de 24 h,
/// 1,035 de 1 à 3 jours, 1,051 de 7 à 14 jours ; au-delà de 7 jours le prix ne bouge pas un jour sur deux, puis saute
/// (99e centile de la variation quotidienne : 5,9 points de probabilité). Les deux seules recommandations pariées avant
/// cette garde avaient 15 et 19 jours d'avance ; leur avantage annoncé s'est retourné en quelques jours.
pub const MAX_HORIZON_HOURS: i64 = 72;

/// Somme maximale des probabilités implicites de la **référence**. Plus stricte que [`IMPLIED_SUM_RANGE`], qui vaut pour
/// un opérateur : à moins de trois jours, 98 % des relevés de Pinnacle sont sous 1,06 (§73). Au-dessus, la marge à retirer
/// est trop grande pour que la probabilité obtenue serve de référence (le relevé du 2026-11-10 de Deportivo – Barcelone,
/// à 1,085, annonçait +4,2 % sur un prix qui valait −17 à −26 % le lendemain).
pub const SHARP_IMPLIED_SUM_MAX: f64 = 1.06;

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
    /// Matchs examinés : à venir, dans un relevé assez récent, à [`MAX_HORIZON_HOURS`] heures au plus du coup d'envoi.
    pub events: usize,
    /// Matchs d'un relevé récent laissés de côté parce qu'ils commencent dans plus de [`MAX_HORIZON_HOURS`] heures. Ils
    /// ne comptent pas dans `events` : `events + too_far` nul veut dire que le relevé lui-même manque.
    pub too_far: usize,
    /// Matchs ignorés faute d'un 1X2 complet et plausible de Pinnacle dans le relevé.
    pub without_reference: usize,
    /// Matchs ignorés parce que la marge de Pinnacle dépasse [`SHARP_IMPLIED_SUM_MAX`] : la référence existe, elle n'est
    /// pas assez précise.
    pub wide_reference: usize,
    /// Prix ignorés parce que la somme des probabilités implicites est hors de `[1,00 ; 1,30]`.
    pub implausible_quotes: usize,
    /// Un pari au plus par match, du plus proche coup d'envoi au plus lointain.
    pub opportunities: Vec<Opportunity>,
}

/// Les cotes d'un bookmaker pour un match, à mesure qu'on lit les lignes de la table `quote`.
#[derive(Debug, Default, Clone)]
pub(crate) struct BookQuotes {
    pub(crate) prices: [Option<Odds>; 3],
    /// Plus ancienne mise à jour parmi les trois issues : le prix vaut ce que vaut son plus vieux élément.
    pub(crate) oldest_update: Option<DateTime<Utc>>,
}

impl BookQuotes {
    /// Le 1X2 complet, ou `None` s'il manque une issue.
    pub(crate) fn complete(&self) -> Option<Odds1x2> {
        match self.prices {
            [Some(home), Some(draw), Some(away)] => Some(Odds1x2 { home, draw, away }),
            _ => None,
        }
    }
}

/// Un match à venir lu dans son relevé le plus récent, avec les cotes 1X2 de chaque bookmaker.
#[derive(Debug, Clone)]
pub(crate) struct EventQuotes {
    pub(crate) event_id: String,
    pub(crate) api_call_id: i64,
    pub(crate) sport_key: String,
    pub(crate) home_team: String,
    pub(crate) away_team: String,
    pub(crate) kickoff: DateTime<Utc>,
    /// BTreeMap : itération par clé de bookmaker, donc départage déterministe des égalités.
    pub(crate) books: BTreeMap<String, BookQuotes>,
}

/// Les matchs de football à venir, chacun dans son relevé le plus récent si ce relevé a moins de
/// [`MAX_SNAPSHOT_AGE_HOURS`] heures, du plus proche coup d'envoi au plus lointain. Lecture seule.
pub(crate) fn latest_quotes(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<EventQuotes>> {
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
    let mut all = Vec::new();
    for row in rows {
        let (event_id, call_id, sport_key, kickoff, home, away) = row?;
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
        all.push(EventQuotes {
            event_id,
            api_call_id: call_id,
            sport_key,
            home_team: home,
            away_team: away,
            kickoff: parse_timestamp(&kickoff)?,
            books,
        });
    }
    Ok(all)
}

/// Lit les derniers relevés et cherche les prix qui dépassent la cote juste de Pinnacle d'au moins `min_edge`.
///
/// Un match n'est lu que dans son relevé le plus récent, et seulement si ce relevé a moins de
/// [`MAX_SNAPSHOT_AGE_HOURS`] heures et si le coup d'envoi est dans le futur, à [`MAX_HORIZON_HOURS`] heures au plus.
/// Les bookmakers de `excluded` (`excluded_bookmakers` de la configuration) ne sont jamais retenus : leurs prix restent
/// dans l'archive, c'est tout.
///
/// L'horizon est appliqué ici et non dans [`latest_quotes`] : les bonus de bienvenue lisent les mêmes relevés pour
/// choisir un match à couvrir, et un match lointain leur convient.
pub fn scan(
    conn: &Connection,
    method: MarginMethod,
    min_edge: f64,
    excluded: &[String],
    now: DateTime<Utc>,
) -> Result<Scan> {
    let mut scan = Scan {
        events: 0,
        too_far: 0,
        without_reference: 0,
        wide_reference: 0,
        implausible_quotes: 0,
        opportunities: Vec::new(),
    };
    let horizon = now + Duration::hours(MAX_HORIZON_HOURS);
    for event in latest_quotes(conn, now)? {
        if event.kickoff > horizon {
            scan.too_far += 1;
            continue;
        }
        scan.events += 1;
        let reference = event
            .books
            .get(SHARP_BOOKMAKER)
            .and_then(BookQuotes::complete)
            .filter(plausible);
        let Some(reference) = reference else {
            scan.without_reference += 1;
            continue;
        };
        if reference.implied_sum() > SHARP_IMPLIED_SUM_MAX {
            scan.wide_reference += 1;
            continue;
        }
        let Some(fair) = remove_margin(&reference, method) else {
            scan.without_reference += 1;
            continue;
        };

        let mut best: Option<Opportunity> = None;
        for (bookmaker, book) in &event.books {
            if bookmaker == SHARP_BOOKMAKER || excluded.contains(bookmaker) {
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
                    event_id: event.event_id.clone(),
                    api_call_id: event.api_call_id,
                    sport_key: event.sport_key.clone(),
                    home_team: event.home_team.clone(),
                    away_team: event.away_team.clone(),
                    kickoff: event.kickoff,
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

/// Vrai si la somme des probabilités implicites est plausible (voir [`IMPLIED_SUM_RANGE`]).
pub(crate) fn plausible(odds: &Odds1x2) -> bool {
    (IMPLIED_SUM_RANGE.0..=IMPLIED_SUM_RANGE.1).contains(&odds.implied_sum())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::testing::{
        KICKOFF, NOW, PINNACLE, at, database, event, event_named, store,
    };

    #[test]
    fn a_match_beyond_the_horizon_is_counted_but_never_examined() {
        let mut conn = database();
        let books = [PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)];
        // « Maintenant » : le 18 à 15 h. 72 h plus tard, pile : le 21 à 15 h (examiné) ; une heure de plus : trop loin.
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[
                event_named("at-the-limit", "2026-10-21T15:00:00Z", &books),
                event_named("too-far", "2026-10-21T16:00:00Z", &books),
                event_named("weeks-away", "2026-11-06T14:00:00Z", &books),
            ],
        );

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();

        assert_eq!(scan.events, 1);
        assert_eq!(scan.too_far, 2);
        assert_eq!(scan.opportunities.len(), 1);
        assert_eq!(scan.opportunities[0].event_id, "at-the-limit");
    }

    #[test]
    fn a_reference_with_too_much_margin_is_set_aside_and_counted() {
        let mut conn = database();
        // Pinnacle à 1,90 / 3,40 / 4,00 : somme 0,526 + 0,294 + 0,250 = 1,070, au-dessus de 1,06. À 1,95 / 3,50 / 4,00 :
        // 0,513 + 0,286 + 0,250 = 1,049, gardée. Le prix de Betclic dépasse la cote juste dans les deux cas.
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[
                event_named(
                    "wide",
                    KICKOFF,
                    &[
                        ("pinnacle", 1.90, 3.40, 4.00),
                        ("betclic_fr", 2.3, 3.2, 3.4),
                    ],
                ),
                event_named(
                    "tight",
                    KICKOFF,
                    &[
                        ("pinnacle", 1.95, 3.50, 4.00),
                        ("betclic_fr", 2.3, 3.2, 3.4),
                    ],
                ),
            ],
        );

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();

        assert_eq!(scan.events, 2);
        assert_eq!(scan.wide_reference, 1);
        assert_eq!(
            scan.without_reference, 0,
            "une référence trop large n'est pas une référence absente"
        );
        assert_eq!(scan.opportunities.len(), 1);
        assert_eq!(scan.opportunities[0].event_id, "tight");
    }

    #[test]
    fn tennis_events_in_the_archive_are_neither_read_nor_counted_as_football_without_reference() {
        let mut conn = database();
        let mut tennis = event_named(
            "tennis-1",
            KICKOFF,
            &[PINNACLE, ("winamax_fr", 2.3, 3.2, 3.4)],
        );
        tennis.sport_key = "tennis_wta_ningbo_open".into();
        store(&mut conn, "2026-10-18T14:00:00Z", &[tennis]);

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();

        assert_eq!(scan.events, 0);
        assert_eq!(scan.too_far, 0);
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

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.03, &[], at(NOW)).unwrap();

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

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();

        assert_eq!(scan.opportunities.len(), 1);
        assert_eq!(scan.opportunities[0].bookmaker, "winamax_fr");
    }

    #[test]
    fn an_excluded_bookmaker_is_never_retained_even_with_the_best_price() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[
                PINNACLE,
                ("betclic_fr", 2.3, 3.2, 3.4),
                ("netbet_fr", 2.6, 3.2, 3.4),
            ])],
        );

        let excluded = ["netbet_fr".to_string()];
        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &excluded, at(NOW)).unwrap();

        assert_eq!(scan.opportunities.len(), 1);
        assert_eq!(scan.opportunities[0].bookmaker, "betclic_fr");
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

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();

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

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();

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

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();

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

        let stale = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();
        assert_eq!(stale.events, 0);

        // Relevé récent (13 h) d'un match de 14 h, mais on se place à 14 h 30 : après le coup d'envoi.
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-24T13:00:00Z",
            &[event_named(
                "match-1",
                "2026-10-24T14:00:00Z",
                &[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)],
            )],
        );
        let started = scan(
            &conn,
            MarginMethod::Multiplicative,
            0.0,
            &[],
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

        let scan = scan(&conn, MarginMethod::Multiplicative, 0.0, &[], at(NOW)).unwrap();

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
        let mut found = scan(&conn, MarginMethod::Multiplicative, 0.03, &[], at(NOW))
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
