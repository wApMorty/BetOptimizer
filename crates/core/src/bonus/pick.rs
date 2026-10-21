//! Choix du match d'un bonus sur le dernier relevé de l'archive : pour chaque match, l'issue qui maximise le gain garanti,
//! couverte à la meilleure cote des comptes de couverture.
//!
//! Lecture seule. Les cotes du relevé ont souvent plusieurs heures : une opération proposée se recalcule toujours sur les
//! cotes affichées au moment de parier (`edge bonus cover`, ou le calculateur de l'application).

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::OfferKind;
use super::cover::{Cover, cover};
use crate::betting::live::{EventQuotes, latest_quotes, plausible};
use crate::domain::{Euros, Odds, Outcome};
use crate::error::Result;
use crate::store::bonus::{EventRef, Leg, Role};

/// Ce que l'on cherche à jouer.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// Opérateur du bonus.
    pub bookmaker: String,
    /// Forme de l'opération.
    pub kind: OfferKind,
    /// Mise du pari bonus.
    pub amount: Euros,
    /// Cote minimale imposée par l'offre.
    pub min_odds: Option<Odds>,
    /// Valeur supposée d'un euro de freebet (lue pour `FreebetRefund` seulement).
    pub freebet_value: f64,
}

/// Une opération proposée.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Match.
    pub event: EventRef,
    /// Relevé d'où viennent les cotes.
    pub api_call_id: i64,
    /// Plus ancienne mise à jour des prix utilisés : l'âge de la proposition.
    pub priced_at: DateTime<Utc>,
    /// Le pari bonus, puis une couverture par autre issue.
    pub legs: Vec<Leg>,
    /// Mises, gains par issue et gain garanti.
    pub cover: Cover,
}

impl Candidate {
    /// Le pari bonus (toujours le premier de `legs`).
    pub fn bonus_leg(&self) -> &Leg {
        &self.legs[0]
    }
}

/// Les meilleures opérations du dernier relevé, au plus `limit`, de la plus rentable à la moins rentable.
pub fn pick(
    conn: &Connection,
    target: &Target,
    cover_books: &[String],
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<Candidate>> {
    Ok(candidates(
        &latest_quotes(conn, now)?,
        target,
        cover_books,
        limit,
    ))
}

/// Cœur de [`pick`], sans base : une opération au plus par match, gain garanti strictement positif.
pub(crate) fn candidates(
    events: &[EventQuotes],
    target: &Target,
    cover_books: &[String],
    limit: usize,
) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = events
        .iter()
        .filter_map(|event| best_for_event(event, target, cover_books))
        .filter(|candidate| candidate.cover.guaranteed > 0.0)
        .collect();
    // Du plus gros gain garanti au plus petit ; à égalité, le coup d'envoi le plus proche.
    found.sort_by(|a, b| {
        b.cover
            .guaranteed
            .total_cmp(&a.cover.guaranteed)
            .then(a.event.kickoff.cmp(&b.event.kickoff))
    });
    found.truncate(limit);
    found
}

/// La meilleure issue d'un match pour le bonus, ou `None` si l'opérateur du bonus ou une couverture manque.
fn best_for_event(
    event: &EventQuotes,
    target: &Target,
    cover_books: &[String],
) -> Option<Candidate> {
    let bonus_book = event.books.get(&target.bookmaker)?;
    let bonus_odds = bonus_book.complete().filter(plausible)?;
    let mut best: Option<Candidate> = None;
    for outcome in Outcome::ALL {
        let odds = bonus_odds.get(outcome);
        if target.min_odds.is_some_and(|min| odds.get() < min.get()) {
            continue;
        }
        let Some(covers) = best_covers(event, outcome, &target.bookmaker, cover_books) else {
            continue;
        };
        let cover_odds: Vec<Odds> = covers.iter().map(|quote| quote.odds).collect();
        let Ok(result) = cover(
            target.kind,
            target.amount,
            odds,
            target.freebet_value,
            &cover_odds,
        ) else {
            continue;
        };
        if best
            .as_ref()
            .is_some_and(|b| b.cover.guaranteed >= result.guaranteed)
        {
            continue;
        }
        let mut legs = vec![Leg {
            role: Role::Bonus,
            bookmaker: target.bookmaker.clone(),
            outcome,
            odds,
            stake: target.amount,
        }];
        let mut priced_at = bonus_book.oldest_update?;
        // `into_iter()` consomme `covers` : chaque `bookmaker` (un `String`) passe dans son pari sans copie.
        for (quote, stake) in covers.into_iter().zip(&result.stakes) {
            priced_at = priced_at.min(quote.updated);
            legs.push(Leg {
                role: Role::Cover,
                bookmaker: quote.bookmaker,
                outcome: quote.outcome,
                odds: quote.odds,
                stake: *stake,
            });
        }
        best = Some(Candidate {
            event: EventRef {
                event_id: event.event_id.clone(),
                sport_key: event.sport_key.clone(),
                home_team: event.home_team.clone(),
                away_team: event.away_team.clone(),
                kickoff: event.kickoff,
            },
            api_call_id: event.api_call_id,
            priced_at,
            legs,
            cover: result,
        });
    }
    best
}

/// La meilleure cote de couverture d'une issue.
struct CoverQuote {
    bookmaker: String,
    outcome: Outcome,
    odds: Odds,
    /// Plus ancienne mise à jour du 1X2 de cet opérateur.
    updated: DateTime<Utc>,
}

/// Pour chaque autre issue que `bonus_outcome`, la meilleure cote parmi les comptes de couverture (prix complets et
/// plausibles seulement). `None` si une issue n'a aucune couverture.
fn best_covers(
    event: &EventQuotes,
    bonus_outcome: Outcome,
    bonus_book: &str,
    cover_books: &[String],
) -> Option<Vec<CoverQuote>> {
    let mut covers = Vec::new();
    for outcome in Outcome::ALL.into_iter().filter(|o| *o != bonus_outcome) {
        let mut best: Option<CoverQuote> = None;
        for bookmaker in cover_books.iter().filter(|b| b.as_str() != bonus_book) {
            let Some(book) = event.books.get(bookmaker) else {
                continue;
            };
            let (Some(odds), Some(updated)) =
                (book.complete().filter(plausible), book.oldest_update)
            else {
                continue;
            };
            let price = odds.get(outcome);
            if best.as_ref().is_none_or(|b| price.get() > b.odds.get()) {
                best = Some(CoverQuote {
                    bookmaker: bookmaker.clone(),
                    outcome,
                    odds: price,
                    updated,
                });
            }
        }
        covers.push(best?);
    }
    Some(covers)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::betting::live::BookQuotes;

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn book(home: f64, draw: f64, away: f64) -> BookQuotes {
        BookQuotes {
            prices: [Odds::new(home), Odds::new(draw), Odds::new(away)],
            oldest_update: Some(at("2026-10-21T11:55:00Z")),
        }
    }

    fn event(id: &str, books: &[(&str, BookQuotes)]) -> EventQuotes {
        EventQuotes {
            event_id: id.into(),
            api_call_id: 7,
            sport_key: "soccer_epl".into(),
            home_team: "Arsenal".into(),
            away_team: "Chelsea".into(),
            kickoff: at("2026-10-25T14:00:00Z"),
            // `to_string()` : la carte possède ses clés, indépendamment des littéraux du test.
            books: books
                .iter()
                .map(|(key, quotes)| (key.to_string(), quotes.clone()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn target(kind: OfferKind) -> Target {
        Target {
            bookmaker: "betclic_fr".into(),
            kind,
            amount: Euros::new(100.0).unwrap(),
            min_odds: None,
            freebet_value: 0.53,
        }
    }

    fn covers(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn each_other_outcome_is_covered_at_the_best_cover_price() {
        let events = [event(
            "e1",
            &[
                ("betclic_fr", book(1.80, 3.80, 4.40)),
                ("winamax_fr", book(1.85, 3.60, 4.00)),
                ("pmu_fr", book(1.78, 3.95, 4.10)),
            ],
        )];
        let found = candidates(
            &events,
            &target(OfferKind::Freebet),
            &covers(&["winamax_fr", "pmu_fr"]),
            5,
        );
        assert_eq!(found.len(), 1);
        let legs = &found[0].legs;
        // Le freebet va sur l'outsider (4,40) ; domicile couvert chez Winamax (1,85), nul chez PMU (3,95).
        assert_eq!((legs[0].outcome, legs[0].odds.get()), (Outcome::Away, 4.40));
        assert_eq!(
            (legs[1].bookmaker.as_str(), legs[1].outcome),
            ("winamax_fr", Outcome::Home)
        );
        assert_eq!(
            (legs[2].bookmaker.as_str(), legs[2].outcome),
            ("pmu_fr", Outcome::Draw)
        );
        assert!(found[0].cover.guaranteed > 0.0);
    }

    #[test]
    fn the_bonus_operator_never_covers_itself_and_unknown_books_are_ignored() {
        let events = [event(
            "e1",
            &[
                ("betclic_fr", book(1.80, 3.80, 4.40)),
                ("unibet_fr", book(1.95, 4.00, 4.50)),
                ("winamax_fr", book(1.85, 3.60, 4.00)),
            ],
        )];
        // Unibet a de meilleures cotes, mais ce n'est pas un compte de couverture ; Betclic l'est par erreur, mais c'est
        // l'opérateur du bonus.
        let found = candidates(
            &events,
            &target(OfferKind::Freebet),
            &covers(&["winamax_fr", "betclic_fr"]),
            5,
        );
        assert!(
            found[0].legs[1..]
                .iter()
                .all(|leg| leg.bookmaker == "winamax_fr")
        );
    }

    #[test]
    fn no_cover_account_means_no_candidate() {
        let events = [event("e1", &[("betclic_fr", book(1.80, 3.80, 4.40))])];
        assert!(
            candidates(
                &events,
                &target(OfferKind::Freebet),
                &covers(&["winamax_fr"]),
                5
            )
            .is_empty()
        );
    }

    #[test]
    fn the_minimum_odds_of_the_offer_is_respected() {
        let events = [event(
            "e1",
            &[
                ("betclic_fr", book(1.80, 3.80, 4.40)),
                ("winamax_fr", book(1.85, 3.60, 4.00)),
            ],
        )];
        let mut wanted = target(OfferKind::FreebetRefund);
        wanted.min_odds = Odds::new(5.0);
        assert!(candidates(&events, &wanted, &covers(&["winamax_fr"]), 5).is_empty());
        wanted.min_odds = Odds::new(1.5);
        let found = candidates(&events, &wanted, &covers(&["winamax_fr"]), 5);
        assert!(found[0].bonus_leg().odds.get() >= 1.5);
    }

    #[test]
    fn candidates_are_sorted_by_guaranteed_gain_and_limited() {
        let events = [
            event(
                "weak",
                &[
                    ("betclic_fr", book(1.70, 3.50, 4.00)),
                    ("winamax_fr", book(1.75, 3.40, 4.20)),
                ],
            ),
            event(
                "strong",
                &[
                    ("betclic_fr", book(1.90, 3.90, 4.60)),
                    ("winamax_fr", book(1.92, 3.80, 4.50)),
                ],
            ),
        ];
        let found = candidates(
            &events,
            &target(OfferKind::CashRefund),
            &covers(&["winamax_fr"]),
            1,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].event.event_id, "strong");
    }

    #[test]
    fn implausible_prices_are_neither_bonus_nor_cover() {
        // Somme des probabilités implicites de Winamax ≈ 0,61 : prix aberrant, écarté.
        let events = [event(
            "e1",
            &[
                ("betclic_fr", book(1.80, 3.80, 4.40)),
                ("winamax_fr", book(3.0, 6.0, 9.0)),
            ],
        )];
        assert!(
            candidates(
                &events,
                &target(OfferKind::Freebet),
                &covers(&["winamax_fr"]),
                5
            )
            .is_empty()
        );
    }
}
