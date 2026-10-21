//! Suivi des offres : l'état de chaque opérateur se déduit du journal, et rien n'est jamais proposé deux fois.
//!
//! Un opérateur est *à vérifier* (aucune offre enregistrée), *inéligible*, *disponible* ou *consommé* (dès que son pari
//! bonus est enregistré). Les comptes de couverture sont les opérateurs inéligibles ou consommés : couvrir chez un
//! opérateur dont l'offre reste à prendre ferait de la couverture son « premier pari » et gâcherait l'offre.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::pick::{Candidate, Target, pick};
use super::{OPERATORS, OfferKind, PICKS_PER_BOOKMAKER, operator_name};
use crate::betting::settle::result_of;
use crate::domain::{Euros, Outcome};
use crate::error::{Error, Result};
use crate::ingest::aliases::Aliases;
use crate::store::bonus::{
    self, Eligibility, EventRef, Leg, MatchResult, NewOperation, Role, StoredOffer, StoredOperation,
};
use crate::store::journal::SettlementSource;

/// État de l'offre d'un opérateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferState {
    /// Aucune offre enregistrée : lire le règlement avant tout dépôt.
    ToCheck,
    /// Offre inaccessible : le compte sert seulement à couvrir.
    Ineligible,
    /// Offre à prendre.
    Available,
    /// Pari bonus enregistré : l'offre est prise.
    Consumed,
}

impl OfferState {
    /// Nom affiché.
    pub fn label(self) -> &'static str {
        match self {
            OfferState::ToCheck => "à vérifier",
            OfferState::Ineligible => "inéligible",
            OfferState::Available => "disponible",
            OfferState::Consumed => "consommée",
        }
    }
}

/// Où en est un opérateur.
#[derive(Debug, Clone, PartialEq)]
pub struct OperatorStatus {
    /// Clé de l'opérateur.
    pub key: String,
    /// Nom affiché.
    pub name: String,
    /// Vrai si l'archive a ses cotes (sinon : calcul à la main sur les cotes affichées).
    pub in_archive: bool,
    /// Offre en vigueur, si elle est enregistrée.
    pub offer: Option<StoredOffer>,
    /// État de l'offre.
    pub state: OfferState,
    /// Freebet reçu et pas encore joué (premier pari remboursé en freebet puis perdu).
    pub pending_freebet: Option<Euros>,
    /// Gain net en argent des opérations réglées dont le pari bonus est chez cet opérateur.
    pub realized: f64,
    /// Opérations en attente de règlement.
    pub open_operations: usize,
}

/// Une offre à jouer et les opérations proposées sur le dernier relevé.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Ce que l'on cherche à jouer.
    pub target: Target,
    /// Nom affiché de l'opérateur.
    pub name: String,
    /// Vrai si l'archive a les cotes de l'opérateur ; sinon `candidates` est vide.
    pub in_archive: bool,
    /// Offre en vigueur (son identifiant sert à enregistrer l'opération).
    pub offer_id: i64,
    /// Opérations proposées, de la plus rentable à la moins rentable.
    pub candidates: Vec<Candidate>,
}

/// Tout ce qu'affichent l'application, le site et `edge bonus status`.
#[derive(Debug, Clone, PartialEq)]
pub struct Board {
    /// Un état par opérateur : ceux de [`OPERATORS`], puis ceux saisis à la main.
    pub operators: Vec<OperatorStatus>,
    /// Comptes de couverture.
    pub cover_accounts: Vec<String>,
    /// Offres à jouer, avec leurs propositions.
    pub plans: Vec<Plan>,
    /// Toutes les opérations, de la plus ancienne à la plus récente.
    pub operations: Vec<StoredOperation>,
    /// Gain net en argent de toutes les opérations réglées.
    pub realized: f64,
    /// Somme des gains garantis des opérations encore ouvertes.
    pub open_guaranteed: f64,
}

/// Gain net d'une opération pour un résultat donné, un euro de freebet reçu comptant pour `freebet_value`.
///
/// Pari bonus gagnant : `mise·(cote − 1)`. Perdant : `0` s'il est remboursé en argent ou si c'était un freebet ;
/// `−mise + freebet_value·mise` s'il est remboursé en freebet. Couverture : `mise·(cote − 1)` ou `−mise`. Match annulé : 0.
pub fn gain(kind: OfferKind, legs: &[Leg], result: MatchResult, freebet_value: f64) -> f64 {
    let MatchResult::Played(outcome) = result else {
        return 0.0;
    };
    legs.iter()
        .map(|leg| {
            let stake = leg.stake.get();
            if leg.outcome == outcome {
                return stake * (leg.odds.get() - 1.0);
            }
            match (leg.role, kind) {
                (Role::Cover, _) => -stake,
                (Role::Bonus, OfferKind::CashRefund | OfferKind::Freebet) => 0.0,
                (Role::Bonus, OfferKind::FreebetRefund) => -stake * (1.0 - freebet_value),
            }
        })
        .sum()
}

/// Gain garanti : le plus petit gain sur les trois issues, le freebet éventuel valant `freebet_value` par euro.
pub fn guaranteed_gain(kind: OfferKind, legs: &[Leg], freebet_value: f64) -> f64 {
    Outcome::ALL
        .into_iter()
        .map(|o| gain(kind, legs, MatchResult::Played(o), freebet_value))
        .fold(f64::INFINITY, f64::min)
}

/// Gain net réalisé, en argent seulement, d'une opération réglée ; `None` tant qu'elle ne l'est pas.
/// Le freebet reçu n'y compte pas : il rapportera dans sa propre opération.
pub fn realized_gain(operation: &StoredOperation) -> Option<f64> {
    let settled = operation.settled.as_ref()?;
    let op = &operation.operation;
    Some(gain(op.kind, &op.legs, settled.result, 0.0))
}

/// Freebet gagné par une opération : premier pari remboursé en freebet, réglé et perdu.
fn freebet_earned(operation: &StoredOperation) -> Option<Euros> {
    let op = &operation.operation;
    let settled = operation.settled.as_ref()?;
    let bonus = operation.bonus_leg();
    match (op.kind, settled.result) {
        (OfferKind::FreebetRefund, MatchResult::Played(outcome)) if outcome != bonus.outcome => {
            Some(bonus.stake)
        }
        _ => None,
    }
}

/// Construit une opération à enregistrer ; le gain garanti est recalculé sur les paris eux-mêmes.
pub fn new_operation(
    offer_id: i64,
    kind: OfferKind,
    freebet_value: Option<f64>,
    event: EventRef,
    legs: Vec<Leg>,
) -> NewOperation {
    NewOperation {
        offer_id,
        kind,
        freebet_value,
        guaranteed: guaranteed_gain(kind, &legs, freebet_value.unwrap_or(0.0)),
        event,
        legs,
    }
}

/// L'état de chaque opérateur, déduit des offres en vigueur et des opérations.
pub fn statuses(offers: &[StoredOffer], operations: &[StoredOperation]) -> Vec<OperatorStatus> {
    // Les opérateurs connus d'abord, dans l'ordre de la liste ; puis ceux saisis à la main, par clé.
    let mut keys: Vec<String> = OPERATORS.iter().map(|o| o.key.to_string()).collect();
    for offer in offers {
        if !keys.contains(&offer.offer.bookmaker) {
            keys.push(offer.offer.bookmaker.clone());
        }
    }
    keys.into_iter()
        .map(|key| status_of(key, offers, operations))
        .collect()
}

fn status_of(
    key: String,
    offers: &[StoredOffer],
    operations: &[StoredOperation],
) -> OperatorStatus {
    let offer = offers.iter().find(|o| o.offer.bookmaker == key).cloned();
    // Les opérations dont le pari bonus est chez cet opérateur.
    let own: Vec<&StoredOperation> = operations
        .iter()
        .filter(|op| op.bonus_leg().bookmaker == key)
        .collect();
    let state = match offer.as_ref().map(|o| &o.offer.eligibility) {
        None => OfferState::ToCheck,
        Some(Eligibility::Ineligible) => OfferState::Ineligible,
        Some(Eligibility::Eligible { .. }) if own.is_empty() => OfferState::Available,
        Some(Eligibility::Eligible { .. }) => OfferState::Consumed,
    };
    let earned: f64 = own
        .iter()
        .filter_map(|op| freebet_earned(op))
        .map(Euros::get)
        .sum();
    let used: f64 = own
        .iter()
        .filter(|op| op.operation.kind == OfferKind::Freebet)
        .map(|op| op.bonus_leg().stake.get())
        .sum();
    let pending_freebet = Euros::new(earned - used).filter(|left| left.get() >= Euros::STEP);
    OperatorStatus {
        name: operator_name(&key).to_string(),
        in_archive: OPERATORS.iter().any(|o| o.key == key && o.in_archive),
        key,
        offer,
        state,
        pending_freebet,
        realized: own.iter().filter_map(|op| realized_gain(op)).sum(),
        open_operations: own.iter().filter(|op| op.settled.is_none()).count(),
    }
}

/// Les comptes de couverture : opérateurs inéligibles ou dont l'offre est consommée.
pub fn cover_accounts(statuses: &[OperatorStatus]) -> Vec<String> {
    statuses
        .iter()
        .filter(|s| matches!(s.state, OfferState::Ineligible | OfferState::Consumed))
        .map(|s| s.key.clone())
        .collect()
}

/// Ce qui reste à jouer : chaque offre disponible, et chaque freebet reçu non joué.
fn targets(statuses: &[OperatorStatus], freebet_value: f64) -> Vec<(Target, i64)> {
    let mut all = Vec::new();
    for status in statuses {
        let Some(stored) = &status.offer else {
            continue;
        };
        let Eligibility::Eligible {
            kind,
            amount,
            min_odds,
        } = &stored.offer.eligibility
        else {
            continue;
        };
        let target = |kind: OfferKind, amount: Euros| Target {
            bookmaker: status.key.clone(),
            kind,
            amount,
            min_odds: *min_odds,
            freebet_value,
        };
        if status.state == OfferState::Available {
            all.push((target(*kind, *amount), stored.id));
        }
        if let Some(freebet) = status.pending_freebet {
            all.push((target(OfferKind::Freebet, freebet), stored.id));
        }
    }
    all
}

/// Le tableau de bord : états, comptes de couverture, propositions sur le dernier relevé, opérations et bilan.
pub fn board(conn: &Connection, freebet_value: f64, now: DateTime<Utc>) -> Result<Board> {
    let offers = bonus::latest_offers(conn)?;
    let operations = bonus::operations(conn)?;
    let operators = statuses(&offers, &operations);
    let cover_accounts = cover_accounts(&operators);
    let mut plans = Vec::new();
    for (target, offer_id) in targets(&operators, freebet_value) {
        let in_archive = operators
            .iter()
            .any(|s| s.key == target.bookmaker && s.in_archive);
        let candidates = if in_archive {
            pick(conn, &target, &cover_accounts, now, PICKS_PER_BOOKMAKER)?
        } else {
            Vec::new()
        };
        plans.push(Plan {
            name: operator_name(&target.bookmaker).to_string(),
            target,
            in_archive,
            offer_id,
            candidates,
        });
    }
    let realized = operations.iter().filter_map(realized_gain).sum();
    let open_guaranteed = operations
        .iter()
        .filter(|op| op.settled.is_none())
        .map(|op| op.operation.guaranteed)
        .sum();
    Ok(Board {
        operators,
        cover_accounts,
        plans,
        operations,
        realized,
        open_guaranteed,
    })
}

/// Vérifie une opération contre l'état du journal, puis l'enregistre. Renvoie son identifiant.
///
/// Refusé, avec un message qui dit quoi corriger : offre inconnue, périmée, inéligible ou déjà consommée ; mise du bonus
/// au-dessus du montant de l'offre ou cote sous sa cote minimale ; couverture chez un opérateur qui n'est pas un compte de
/// couverture ; deux mises sur une même issue.
pub fn record_operation(
    conn: &mut Connection,
    new: &NewOperation,
    now: DateTime<Utc>,
) -> Result<i64> {
    let offers = bonus::latest_offers(conn)?;
    let operations = bonus::operations(conn)?;
    let operators = statuses(&offers, &operations);
    check_operation(new, &offers, &operators)?;
    bonus::insert_operation(conn, new, now)
}

fn check_operation(
    new: &NewOperation,
    offers: &[StoredOffer],
    operators: &[OperatorStatus],
) -> Result<()> {
    let refuse = |message: String| Err(Error::Input(message));
    let Some(offer) = offers.iter().find(|o| o.id == new.offer_id) else {
        return refuse(format!(
            "offre {} inconnue ou remplacée par une plus récente : relis `edge bonus status`",
            new.offer_id
        ));
    };
    let bookmaker = &offer.offer.bookmaker;
    let name = operator_name(bookmaker);
    let Eligibility::Eligible {
        kind: offer_kind,
        amount,
        min_odds,
    } = &offer.offer.eligibility
    else {
        return refuse(format!(
            "l'offre de {name} est enregistrée comme inéligible"
        ));
    };
    let status = operators
        .iter()
        .find(|s| &s.key == bookmaker)
        .expect("chaque offre en vigueur a un état (statuses parcourt toutes les offres)");

    let bonus_legs: Vec<&Leg> = new.legs.iter().filter(|l| l.role == Role::Bonus).collect();
    let [bonus_leg] = bonus_legs.as_slice() else {
        return refuse(format!(
            "une opération a exactement un pari bonus ({} reçu(s))",
            bonus_legs.len()
        ));
    };
    if &bonus_leg.bookmaker != bookmaker {
        return refuse(format!(
            "le pari bonus doit être placé chez {name}, l'opérateur de l'offre"
        ));
    }
    if bonus_leg.stake.get() > amount.get() + 1e-9 {
        return refuse(format!(
            "mise du bonus {:.2} € au-dessus du montant de l'offre ({:.2} €)",
            bonus_leg.stake.get(),
            amount.get()
        ));
    }
    if let Some(min) = min_odds
        && bonus_leg.odds.get() < min.get()
    {
        return refuse(format!(
            "cote {:.2} sous la cote minimale de l'offre ({:.2})",
            bonus_leg.odds.get(),
            min.get()
        ));
    }

    // Forme de l'opération : celle de l'offre tant qu'elle est disponible ; ensuite, seulement le freebet reçu.
    if new.kind == *offer_kind {
        if status.state != OfferState::Available {
            return refuse(format!("l'offre de {name} est déjà consommée"));
        }
    } else if new.kind == OfferKind::Freebet && *offer_kind == OfferKind::FreebetRefund {
        let pending = status.pending_freebet.map_or(0.0, Euros::get);
        if bonus_leg.stake.get() > pending + 1e-9 {
            return refuse(format!(
                "freebet de {:.2} € à jouer chez {name}, mise de {:.2} € demandée",
                pending,
                bonus_leg.stake.get()
            ));
        }
    } else {
        return refuse(format!(
            "une offre « {} » ne se joue pas en opération « {} »",
            offer_kind.label(),
            new.kind.label()
        ));
    }
    if (new.kind == OfferKind::FreebetRefund) != new.freebet_value.is_some() {
        return refuse(
            "la valeur d'un euro de freebet se donne pour un premier pari remboursé en freebet, et seulement pour lui".into(),
        );
    }

    let covers = cover_accounts(operators);
    for (index, leg) in new.legs.iter().enumerate() {
        if new.legs[..index].iter().any(|l| l.outcome == leg.outcome) {
            return refuse("deux mises sur une même issue dans une opération".into());
        }
        if leg.role == Role::Cover
            && (&leg.bookmaker == bookmaker || !covers.contains(&leg.bookmaker))
        {
            return refuse(format!(
                "{} n'est pas un compte de couverture : seuls les opérateurs inéligibles ou dont l'offre est consommée le sont",
                operator_name(&leg.bookmaker)
            ));
        }
    }
    Ok(())
}

/// Bilan d'un règlement automatique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettleReport {
    /// Opérations réglées pendant cette exécution.
    pub settled: usize,
    /// Opérations encore ouvertes : résultat absent de l'historique, ou nom d'équipe inconnu.
    pub still_open: usize,
}

/// Règle les opérations ouvertes dont le résultat est dans l'historique football-data importé.
pub fn settle_open_operations(
    conn: &Connection,
    aliases: &Aliases,
    now: DateTime<Utc>,
) -> Result<SettleReport> {
    let mut report = SettleReport {
        settled: 0,
        still_open: 0,
    };
    for operation in bonus::operations(conn)? {
        if operation.settled.is_some() {
            continue;
        }
        let event = &operation.operation.event;
        let found = if event.kickoff < now {
            result_of(
                conn,
                aliases,
                &event.sport_key,
                &event.home_team,
                &event.away_team,
                event.kickoff,
            )?
        } else {
            None
        };
        match found {
            Some(outcome) => {
                bonus::insert_settlement(
                    conn,
                    operation.id,
                    MatchResult::Played(outcome),
                    SettlementSource::FootballData,
                    now,
                )?;
                report.settled += 1;
            }
            None => report.still_open += 1,
        }
    }
    Ok(report)
}

/// Règle une opération à la main. Erreur si elle est inconnue ou déjà réglée.
pub fn settle_operation(
    conn: &Connection,
    id: i64,
    result: MatchResult,
    now: DateTime<Utc>,
) -> Result<()> {
    if !bonus::operations(conn)?.iter().any(|op| op.id == id) {
        return Err(Error::Input(format!("opération {id} inconnue")));
    }
    if !bonus::insert_settlement(conn, id, result, SettlementSource::Manual, now)? {
        return Err(Error::Input(format!(
            "l'opération {id} est déjà réglée : un règlement ne se réécrit pas"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;
    use crate::store::bonus::tests::{at, database, eligible, event, leg, offer};
    use crate::store::bonus::{insert_offer, insert_settlement};

    fn record(
        conn: &mut Connection,
        offer_id: i64,
        kind: OfferKind,
        legs: Vec<Leg>,
    ) -> Result<i64> {
        let value = (kind == OfferKind::FreebetRefund).then_some(0.53);
        let new = new_operation(offer_id, kind, value, event(), legs);
        record_operation(conn, &new, at("2026-10-21T12:00:00Z"))
    }

    fn state(conn: &Connection, key: &str) -> OperatorStatus {
        let offers = bonus::latest_offers(conn).unwrap();
        let operations = bonus::operations(conn).unwrap();
        statuses(&offers, &operations)
            .into_iter()
            .find(|s| s.key == key)
            .unwrap()
    }

    #[test]
    fn gains_follow_the_form_of_the_offer() {
        let legs = [
            leg(Role::Bonus, "betclic_fr", Outcome::Away, 3.0, 100.0),
            leg(Role::Cover, "winamax_fr", Outcome::Home, 2.0, 60.0),
            leg(Role::Cover, "winamax_fr", Outcome::Draw, 4.0, 30.0),
        ];
        let home = MatchResult::Played(Outcome::Home);
        // Le bonus perd, la couverture domicile gagne 60, celle du nul perd 30.
        assert_abs_diff_eq!(gain(OfferKind::CashRefund, &legs, home, 0.5), 30.0);
        assert_abs_diff_eq!(gain(OfferKind::Freebet, &legs, home, 0.5), 30.0);
        assert_abs_diff_eq!(
            gain(OfferKind::FreebetRefund, &legs, home, 0.5),
            30.0 - 50.0
        );
        assert_abs_diff_eq!(
            gain(OfferKind::FreebetRefund, &legs, home, 0.0),
            30.0 - 100.0
        );
        // Le bonus gagne 200, les deux couvertures perdent 90.
        assert_abs_diff_eq!(
            gain(
                OfferKind::CashRefund,
                &legs,
                MatchResult::Played(Outcome::Away),
                0.5
            ),
            110.0
        );
        assert_abs_diff_eq!(
            gain(OfferKind::CashRefund, &legs, MatchResult::Void, 0.5),
            0.0
        );
        assert_abs_diff_eq!(guaranteed_gain(OfferKind::CashRefund, &legs, 0.0), 30.0);
    }

    #[test]
    fn an_offer_goes_from_available_to_consumed_and_its_operator_becomes_a_cover_account() {
        let mut conn = database();
        let now = at("2026-10-21T10:00:00Z");
        assert_eq!(state(&conn, "betclic_fr").state, OfferState::ToCheck);
        insert_offer(&conn, &offer("winamax_fr", Eligibility::Ineligible), now).unwrap();
        let betclic = insert_offer(
            &conn,
            &offer("betclic_fr", eligible(OfferKind::CashRefund, 100.0)),
            now,
        )
        .unwrap();
        assert_eq!(state(&conn, "betclic_fr").state, OfferState::Available);
        let operators = statuses(&bonus::latest_offers(&conn).unwrap(), &[]);
        assert_eq!(cover_accounts(&operators), vec!["winamax_fr".to_string()]);

        record(
            &mut conn,
            betclic,
            OfferKind::CashRefund,
            vec![
                leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.2, 100.0),
                leg(Role::Cover, "winamax_fr", Outcome::Home, 1.85, 173.0),
                leg(Role::Cover, "winamax_fr", Outcome::Draw, 3.9, 82.1),
            ],
        )
        .unwrap();
        let after = state(&conn, "betclic_fr");
        assert_eq!(after.state, OfferState::Consumed);
        assert_eq!(after.open_operations, 1);
        let operators = statuses(
            &bonus::latest_offers(&conn).unwrap(),
            &bonus::operations(&conn).unwrap(),
        );
        assert_eq!(
            cover_accounts(&operators),
            vec!["betclic_fr".to_string(), "winamax_fr".to_string()]
        );

        // Une seconde opération sur la même offre est refusée.
        let again = record(
            &mut conn,
            betclic,
            OfferKind::CashRefund,
            vec![leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.2, 100.0)],
        );
        assert!(again.unwrap_err().to_string().contains("déjà consommée"));
    }

    #[test]
    fn covering_at_an_operator_whose_offer_is_still_open_is_refused() {
        let mut conn = database();
        let now = at("2026-10-21T10:00:00Z");
        let betclic = insert_offer(
            &conn,
            &offer("betclic_fr", eligible(OfferKind::CashRefund, 100.0)),
            now,
        )
        .unwrap();
        insert_offer(
            &conn,
            &offer("pmu_fr", eligible(OfferKind::CashRefund, 100.0)),
            now,
        )
        .unwrap();
        for cover_book in ["pmu_fr", "unibet_fr", "betclic_fr"] {
            let refused = record(
                &mut conn,
                betclic,
                OfferKind::CashRefund,
                vec![
                    leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.2, 100.0),
                    leg(Role::Cover, cover_book, Outcome::Home, 1.85, 173.0),
                ],
            );
            assert!(
                refused
                    .unwrap_err()
                    .to_string()
                    .contains("compte de couverture"),
                "{cover_book}"
            );
        }
        assert!(bonus::operations(&conn).unwrap().is_empty());
    }

    #[test]
    fn the_offer_amount_and_minimum_odds_are_enforced() {
        let mut conn = database();
        let now = at("2026-10-21T10:00:00Z");
        let mut limited = offer("betclic_fr", eligible(OfferKind::Freebet, 50.0));
        if let Eligibility::Eligible { min_odds, .. } = &mut limited.eligibility {
            *min_odds = crate::domain::Odds::new(2.0);
        }
        let id = insert_offer(&conn, &limited, now).unwrap();
        let too_big = record(
            &mut conn,
            id,
            OfferKind::Freebet,
            vec![leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.0, 60.0)],
        );
        assert!(
            too_big
                .unwrap_err()
                .to_string()
                .contains("au-dessus du montant")
        );
        let too_low = record(
            &mut conn,
            id,
            OfferKind::Freebet,
            vec![leg(Role::Bonus, "betclic_fr", Outcome::Home, 1.5, 50.0)],
        );
        assert!(too_low.unwrap_err().to_string().contains("cote minimale"));
        let wrong_book = record(
            &mut conn,
            id,
            OfferKind::Freebet,
            vec![leg(Role::Bonus, "pmu_fr", Outcome::Away, 4.0, 50.0)],
        );
        assert!(
            wrong_book
                .unwrap_err()
                .to_string()
                .contains("doit être placé chez Betclic")
        );
    }

    #[test]
    fn a_lost_freebet_refund_leaves_a_freebet_to_play_then_nothing() {
        let mut conn = database();
        let now = at("2026-10-21T10:00:00Z");
        insert_offer(&conn, &offer("winamax_fr", Eligibility::Ineligible), now).unwrap();
        let id = insert_offer(
            &conn,
            &offer("betclic_fr", eligible(OfferKind::FreebetRefund, 100.0)),
            now,
        )
        .unwrap();
        let first = record(
            &mut conn,
            id,
            OfferKind::FreebetRefund,
            vec![
                leg(Role::Bonus, "betclic_fr", Outcome::Draw, 3.1, 100.0),
                leg(Role::Cover, "winamax_fr", Outcome::Home, 2.1, 122.0),
                leg(Role::Cover, "winamax_fr", Outcome::Away, 3.4, 75.3),
            ],
        )
        .unwrap();
        // Avant le règlement, aucun freebet n'est encore dû ; un freebet joué trop tôt est refusé.
        assert_eq!(state(&conn, "betclic_fr").pending_freebet, None);
        let early = record(
            &mut conn,
            id,
            OfferKind::Freebet,
            vec![leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.0, 100.0)],
        );
        assert!(early.is_err());

        insert_settlement(
            &conn,
            first,
            MatchResult::Played(Outcome::Home),
            SettlementSource::Manual,
            now,
        )
        .unwrap();
        let after = state(&conn, "betclic_fr");
        assert_eq!(after.pending_freebet.map(Euros::get), Some(100.0));
        // Réalisé en argent : −100 (bonus perdu) + 122 × 1,1 (couverture gagnante) − 75,3 (couverture perdue).
        assert_abs_diff_eq!(after.realized, -100.0 + 134.2 - 75.3, epsilon = 1e-9);

        record(
            &mut conn,
            id,
            OfferKind::Freebet,
            vec![
                leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.0, 100.0),
                leg(Role::Cover, "winamax_fr", Outcome::Home, 1.9, 157.9),
                leg(Role::Cover, "winamax_fr", Outcome::Draw, 3.6, 83.3),
            ],
        )
        .unwrap();
        assert_eq!(state(&conn, "betclic_fr").pending_freebet, None);
    }

    #[test]
    fn a_manual_settlement_is_written_once() {
        let mut conn = database();
        let now = at("2026-10-21T10:00:00Z");
        let id = insert_offer(
            &conn,
            &offer("betsson_fr", eligible(OfferKind::Freebet, 10.0)),
            now,
        )
        .unwrap();
        let op = record(
            &mut conn,
            id,
            OfferKind::Freebet,
            vec![leg(Role::Bonus, "betsson_fr", Outcome::Home, 2.0, 10.0)],
        )
        .unwrap();
        settle_operation(&conn, op, MatchResult::Played(Outcome::Home), now).unwrap();
        assert!(settle_operation(&conn, op, MatchResult::Void, now).is_err());
        assert!(settle_operation(&conn, 99, MatchResult::Void, now).is_err());
        assert_abs_diff_eq!(state(&conn, "betsson_fr").realized, 10.0);
    }
}
