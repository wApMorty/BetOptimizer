//! Journal des bonus de bienvenue (migration 13) : offres, opérations, paris et règlements.
//!
//! Uniquement des `INSERT`, comme le journal des paris : une offre revue est une ligne de plus (la plus récente fait foi),
//! un règlement est une ligne de plus. Les règles métier (qui peut couvrir, quelle mise est permise) sont dans
//! `bonus::tracking` ; ce module ne fait que lire et écrire.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, params};

use super::journal::{SettlementSource, parse_timestamp, timestamp};
use crate::bonus::OfferKind;
use crate::domain::{Euros, Odds, Outcome};
use crate::error::{Error, Result};

/// Ce que l'on sait de l'offre d'un opérateur.
///
/// Un `enum` à données plutôt que des champs optionnels : une offre éligible a *toujours* une forme et un montant, une offre
/// inéligible n'en a *jamais*, et le compilateur interdit de construire l'entre-deux (la base le vérifie aussi).
#[derive(Debug, Clone, PartialEq)]
pub enum Eligibility {
    /// L'offre est accessible.
    Eligible {
        /// Forme de l'offre.
        kind: OfferKind,
        /// Montant maximal, en euros.
        amount: Euros,
        /// Cote minimale imposée par l'offre, s'il y en a une.
        min_odds: Option<Odds>,
    },
    /// L'offre n'est pas accessible (dépôt passé, par exemple) : le compte peut seulement servir à couvrir.
    Ineligible,
}

/// Une offre à enregistrer.
#[derive(Debug, Clone, PartialEq)]
pub struct NewOffer {
    /// Clé de l'opérateur (`betclic_fr`).
    pub bookmaker: String,
    /// Éligibilité, forme et montant.
    pub eligibility: Eligibility,
    /// Date de la vérification.
    pub checked_on: NaiveDate,
    /// Où l'éligibilité a été vérifiée (règlement, service client…).
    pub source: String,
    /// Remarque libre.
    pub note: Option<String>,
}

/// Une offre relue dans le journal.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredOffer {
    /// Identifiant.
    pub id: i64,
    /// Instant de l'enregistrement (UTC).
    pub created_at: DateTime<Utc>,
    /// Contenu.
    pub offer: NewOffer,
}

/// Rôle d'un pari dans une opération.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Le pari qui utilise le bonus (premier pari ou freebet).
    Bonus,
    /// Une couverture en argent.
    Cover,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::Bonus => "bonus",
            Role::Cover => "cover",
        }
    }

    fn parse(text: &str) -> Result<Role> {
        match text {
            "bonus" => Ok(Role::Bonus),
            "cover" => Ok(Role::Cover),
            other => Err(Error::Data(format!("rôle de pari inconnu : {other}"))),
        }
    }
}

/// Un pari d'une opération.
#[derive(Debug, Clone, PartialEq)]
pub struct Leg {
    /// Pari bonus ou couverture.
    pub role: Role,
    /// Opérateur chez qui le pari est placé.
    pub bookmaker: String,
    /// Issue jouée.
    pub outcome: Outcome,
    /// Cote réellement obtenue.
    pub odds: Odds,
    /// Mise (argent, ou freebet pour le pari bonus d'une opération de forme `Freebet`).
    pub stake: Euros,
}

/// Le match d'une opération, tel que The Odds API le nomme.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRef {
    /// Identifiant du match chez The Odds API.
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

/// Une opération à enregistrer : un pari bonus et ses couvertures, sur un même match.
#[derive(Debug, Clone, PartialEq)]
pub struct NewOperation {
    /// Offre utilisée.
    pub offer_id: i64,
    /// Forme de l'opération.
    pub kind: OfferKind,
    /// Valeur supposée d'un euro de freebet (`FreebetRefund` seulement).
    pub freebet_value: Option<f64>,
    /// Gain garanti calculé à l'enregistrement.
    pub guaranteed: f64,
    /// Match.
    pub event: EventRef,
    /// Les paris : un `Bonus`, et des `Cover`.
    pub legs: Vec<Leg>,
}

/// Résultat du match d'une opération.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResult {
    /// Le match a eu lieu : issue finale.
    Played(Outcome),
    /// Match reporté ou annulé : les paris sont remboursés.
    Void,
}

impl MatchResult {
    fn as_str(self) -> &'static str {
        match self {
            MatchResult::Played(outcome) => outcome.code(),
            MatchResult::Void => "void",
        }
    }

    fn parse(text: &str) -> Result<MatchResult> {
        if text == "void" {
            return Ok(MatchResult::Void);
        }
        Outcome::from_code(text)
            .map(MatchResult::Played)
            .ok_or_else(|| Error::Data(format!("résultat inconnu : {text}")))
    }
}

/// Règlement d'une opération.
#[derive(Debug, Clone, PartialEq)]
pub struct Settled {
    /// Instant du règlement (UTC).
    pub settled_at: DateTime<Utc>,
    /// Résultat du match.
    pub result: MatchResult,
    /// Origine du résultat.
    pub source: SettlementSource,
}

/// Une opération relue dans le journal.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredOperation {
    /// Identifiant.
    pub id: i64,
    /// Instant de l'enregistrement (UTC).
    pub created_at: DateTime<Utc>,
    /// Contenu.
    pub operation: NewOperation,
    /// Règlement, une fois connu.
    pub settled: Option<Settled>,
}

impl StoredOperation {
    /// Le pari bonus de l'opération (il y en a exactement un : index unique en base, vérifié à l'enregistrement).
    pub fn bonus_leg(&self) -> &Leg {
        self.operation
            .legs
            .iter()
            .find(|leg| leg.role == Role::Bonus)
            .expect("une opération a exactement un pari bonus (index bonus_bet_one_bonus)")
    }
}

/// Enregistre une offre et renvoie son identifiant.
pub fn insert_offer(conn: &Connection, new: &NewOffer, created_at: DateTime<Utc>) -> Result<i64> {
    let (status, kind, amount, min_odds) = match &new.eligibility {
        Eligibility::Eligible {
            kind,
            amount,
            min_odds,
        } => (
            "eligible",
            Some(kind.as_str()),
            Some(amount.get()),
            min_odds.map(Odds::get),
        ),
        Eligibility::Ineligible => ("ineligible", None, None, None),
    };
    conn.execute(
        "INSERT INTO bonus_offer (created_at, bookmaker_key, status, kind, amount, min_odds, checked_on, source, note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            timestamp(created_at),
            new.bookmaker,
            status,
            kind,
            amount,
            min_odds,
            new.checked_on.format("%Y-%m-%d").to_string(),
            new.source,
            new.note,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// L'offre en vigueur de chaque opérateur (la plus récente), par clé d'opérateur.
pub fn latest_offers(conn: &Connection) -> Result<Vec<StoredOffer>> {
    let mut statement = conn.prepare(
        "SELECT id, created_at, bookmaker_key, status, kind, amount, min_odds, checked_on, source, note
         FROM bonus_offer
         WHERE id IN (SELECT MAX(id) FROM bonus_offer GROUP BY bookmaker_key)
         ORDER BY bookmaker_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<f64>>(5)?,
            row.get::<_, Option<f64>>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, Option<String>>(9)?,
        ))
    })?;
    let mut all = Vec::new();
    for row in rows {
        let (id, created_at, bookmaker, status, kind, amount, min_odds, checked_on, source, note) =
            row?;
        let eligibility = match (status.as_str(), kind, amount) {
            ("eligible", Some(kind), Some(amount)) => Eligibility::Eligible {
                kind: OfferKind::parse(&kind)?,
                amount: Euros::new(amount)
                    .ok_or_else(|| Error::Data(format!("montant d'offre invalide : {amount}")))?,
                min_odds: min_odds.and_then(Odds::new),
            },
            ("ineligible", _, _) => Eligibility::Ineligible,
            _ => return Err(Error::Data(format!("offre {id} incohérente dans la base"))),
        };
        all.push(StoredOffer {
            id,
            created_at: parse_timestamp(&created_at)?,
            offer: NewOffer {
                bookmaker,
                eligibility,
                checked_on: NaiveDate::parse_from_str(&checked_on, "%Y-%m-%d").map_err(|e| {
                    Error::Data(format!(
                        "date de vérification illisible « {checked_on} » : {e}"
                    ))
                })?,
                source,
                note,
            },
        });
    }
    Ok(all)
}

/// Enregistre une opération et ses paris, en une transaction : tout ou rien. Renvoie l'identifiant de l'opération.
///
/// `&mut Connection` : ouvrir une transaction exige un accès exclusif à la connexion (comme pour les migrations).
pub fn insert_operation(
    conn: &mut Connection,
    new: &NewOperation,
    created_at: DateTime<Utc>,
) -> Result<i64> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO bonus_operation
             (created_at, offer_id, kind, freebet_value, guaranteed, event_id, sport_key, home_team_raw, away_team_raw,
              kickoff)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            timestamp(created_at),
            new.offer_id,
            new.kind.as_str(),
            new.freebet_value,
            new.guaranteed,
            new.event.event_id,
            new.event.sport_key,
            new.event.home_team,
            new.event.away_team,
            timestamp(new.event.kickoff),
        ],
    )?;
    let id = tx.last_insert_rowid();
    for leg in &new.legs {
        tx.execute(
            "INSERT INTO bonus_bet (operation_id, role, bookmaker_key, outcome, odds, stake)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                leg.role.as_str(),
                leg.bookmaker,
                leg.outcome.code(),
                leg.odds.get(),
                leg.stake.get(),
            ],
        )?;
    }
    // Sans `commit()`, `tx` est annulée à sa destruction : une erreur au milieu ne laisse aucune opération à moitié écrite.
    tx.commit()?;
    Ok(id)
}

/// Toutes les opérations, avec leurs paris et leur règlement, de la plus ancienne à la plus récente.
pub fn operations(conn: &Connection) -> Result<Vec<StoredOperation>> {
    let mut statement = conn.prepare(
        "SELECT o.id, o.created_at, o.offer_id, o.kind, o.freebet_value, o.guaranteed, o.event_id, o.sport_key,
                o.home_team_raw, o.away_team_raw, o.kickoff, s.settled_at, s.result, s.source
         FROM bonus_operation o LEFT JOIN bonus_settlement s ON s.operation_id = o.id
         ORDER BY o.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            (
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, f64>(5)?,
            ),
            (
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
            ),
            (
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
            ),
        ))
    })?;
    let mut all = Vec::new();
    for row in rows {
        let ((id, created_at, offer_id, kind, freebet_value, guaranteed), event, settlement) = row?;
        let (event_id, sport_key, home_team, away_team, kickoff) = event;
        let settled = match settlement {
            (Some(settled_at), Some(result), Some(source)) => Some(Settled {
                settled_at: parse_timestamp(&settled_at)?,
                result: MatchResult::parse(&result)?,
                source: SettlementSource::parse(&source)?,
            }),
            _ => None,
        };
        all.push(StoredOperation {
            id,
            created_at: parse_timestamp(&created_at)?,
            operation: NewOperation {
                offer_id,
                kind: OfferKind::parse(&kind)?,
                freebet_value,
                guaranteed,
                event: EventRef {
                    event_id,
                    sport_key,
                    home_team,
                    away_team,
                    kickoff: parse_timestamp(&kickoff)?,
                },
                legs: legs(conn, id)?,
            },
            settled,
        });
    }
    Ok(all)
}

/// Les paris d'une opération, dans l'ordre d'enregistrement.
fn legs(conn: &Connection, operation_id: i64) -> Result<Vec<Leg>> {
    let mut statement = conn.prepare_cached(
        "SELECT role, bookmaker_key, outcome, odds, stake FROM bonus_bet WHERE operation_id = ?1 ORDER BY id",
    )?;
    let rows = statement.query_map([operation_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, f64>(4)?,
        ))
    })?;
    let mut all = Vec::new();
    for row in rows {
        let (role, bookmaker, outcome, odds, stake) = row?;
        all.push(Leg {
            role: Role::parse(&role)?,
            bookmaker,
            outcome: Outcome::from_code(&outcome)
                .ok_or_else(|| Error::Data(format!("issue inconnue dans la base : {outcome}")))?,
            odds: Odds::new(odds).ok_or_else(|| Error::Data(format!("cote invalide : {odds}")))?,
            stake: Euros::new(stake)
                .ok_or_else(|| Error::Data(format!("mise invalide : {stake}")))?,
        });
    }
    Ok(all)
}

/// Règle une opération. Renvoie `false` si elle l'est déjà : un règlement ne se réécrit pas.
pub fn insert_settlement(
    conn: &Connection,
    operation_id: i64,
    result: MatchResult,
    source: SettlementSource,
    settled_at: DateTime<Utc>,
) -> Result<bool> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO bonus_settlement (operation_id, settled_at, result, source) VALUES (?1, ?2, ?3, ?4)",
        params![
            operation_id,
            timestamp(settled_at),
            result.as_str(),
            source.as_str()
        ],
    )?;
    Ok(changed == 1)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::store::migrate;

    pub(crate) fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    pub(crate) fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    pub(crate) fn offer(bookmaker: &str, eligibility: Eligibility) -> NewOffer {
        NewOffer {
            bookmaker: bookmaker.into(),
            eligibility,
            checked_on: NaiveDate::from_ymd_opt(2026, 10, 21).unwrap(),
            source: "règlement".into(),
            note: None,
        }
    }

    pub(crate) fn eligible(kind: OfferKind, amount: f64) -> Eligibility {
        Eligibility::Eligible {
            kind,
            amount: Euros::new(amount).unwrap(),
            min_odds: None,
        }
    }

    pub(crate) fn leg(role: Role, bookmaker: &str, outcome: Outcome, odds: f64, stake: f64) -> Leg {
        Leg {
            role,
            bookmaker: bookmaker.into(),
            outcome,
            odds: Odds::new(odds).unwrap(),
            stake: Euros::new(stake).unwrap(),
        }
    }

    pub(crate) fn event() -> EventRef {
        EventRef {
            event_id: "ev1".into(),
            sport_key: "soccer_epl".into(),
            home_team: "Arsenal".into(),
            away_team: "Chelsea".into(),
            kickoff: at("2026-10-25T14:00:00Z"),
        }
    }

    #[test]
    fn the_latest_offer_of_an_operator_is_the_one_in_force() {
        let conn = database();
        insert_offer(
            &conn,
            &offer("betclic_fr", eligible(OfferKind::FreebetRefund, 100.0)),
            at("2026-10-21T10:00:00Z"),
        )
        .unwrap();
        insert_offer(
            &conn,
            &offer("winamax_fr", Eligibility::Ineligible),
            at("2026-10-21T10:01:00Z"),
        )
        .unwrap();
        // Correction : Betclic rembourse en fait jusqu'à 50 €.
        insert_offer(
            &conn,
            &offer("betclic_fr", eligible(OfferKind::FreebetRefund, 50.0)),
            at("2026-10-21T10:02:00Z"),
        )
        .unwrap();
        let offers = latest_offers(&conn).unwrap();
        assert_eq!(offers.len(), 2);
        assert_eq!(offers[0].offer.bookmaker, "betclic_fr");
        assert_eq!(
            offers[0].offer.eligibility,
            eligible(OfferKind::FreebetRefund, 50.0)
        );
        assert_eq!(offers[1].offer.eligibility, Eligibility::Ineligible);
    }

    #[test]
    fn an_operation_is_written_and_read_back_with_its_legs_and_settlement() {
        let mut conn = database();
        let offer_id = insert_offer(
            &conn,
            &offer("betclic_fr", eligible(OfferKind::Freebet, 100.0)),
            at("2026-10-21T10:00:00Z"),
        )
        .unwrap();
        let new = NewOperation {
            offer_id,
            kind: OfferKind::Freebet,
            freebet_value: None,
            guaranteed: 54.0,
            event: event(),
            legs: vec![
                leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.2, 100.0),
                leg(Role::Cover, "winamax_fr", Outcome::Home, 1.85, 173.0),
                leg(Role::Cover, "winamax_fr", Outcome::Draw, 3.9, 82.1),
            ],
        };
        let id = insert_operation(&mut conn, &new, at("2026-10-21T11:00:00Z")).unwrap();
        assert!(
            insert_settlement(
                &conn,
                id,
                MatchResult::Played(Outcome::Home),
                SettlementSource::Manual,
                at("2026-10-26T10:00:00Z")
            )
            .unwrap()
        );
        assert!(
            !insert_settlement(
                &conn,
                id,
                MatchResult::Void,
                SettlementSource::Manual,
                at("2026-10-26T11:00:00Z")
            )
            .unwrap()
        );
        let stored = operations(&conn).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].operation, new);
        assert_eq!(stored[0].bonus_leg().bookmaker, "betclic_fr");
        assert_eq!(
            stored[0].settled.as_ref().unwrap().result,
            MatchResult::Played(Outcome::Home)
        );
    }

    #[test]
    fn the_bonus_journal_cannot_be_modified() {
        let mut conn = database();
        let offer_id = insert_offer(
            &conn,
            &offer("betclic_fr", eligible(OfferKind::Freebet, 100.0)),
            at("2026-10-21T10:00:00Z"),
        )
        .unwrap();
        let new = NewOperation {
            offer_id,
            kind: OfferKind::Freebet,
            freebet_value: None,
            guaranteed: 1.0,
            event: event(),
            legs: vec![leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.2, 100.0)],
        };
        let id = insert_operation(&mut conn, &new, at("2026-10-21T11:00:00Z")).unwrap();
        insert_settlement(
            &conn,
            id,
            MatchResult::Void,
            SettlementSource::Manual,
            at("2026-10-26T10:00:00Z"),
        )
        .unwrap();
        for sql in [
            "UPDATE bonus_offer SET amount = 1",
            "DELETE FROM bonus_offer",
            "UPDATE bonus_operation SET guaranteed = 0",
            "DELETE FROM bonus_operation",
            "UPDATE bonus_bet SET stake = 1",
            "DELETE FROM bonus_bet",
            "UPDATE bonus_settlement SET result = 'H'",
            "DELETE FROM bonus_settlement",
        ] {
            let error = conn.execute(sql, []).unwrap_err().to_string();
            assert!(error.contains("journal immuable"), "{sql} : {error}");
        }
    }

    #[test]
    fn the_database_refuses_two_bonus_legs_or_two_stakes_on_one_outcome() {
        let mut conn = database();
        let offer_id = insert_offer(
            &conn,
            &offer("betclic_fr", eligible(OfferKind::Freebet, 100.0)),
            at("2026-10-21T10:00:00Z"),
        )
        .unwrap();
        let mut new = NewOperation {
            offer_id,
            kind: OfferKind::Freebet,
            freebet_value: None,
            guaranteed: 1.0,
            event: event(),
            legs: vec![
                leg(Role::Bonus, "betclic_fr", Outcome::Away, 4.2, 100.0),
                leg(Role::Bonus, "betclic_fr", Outcome::Home, 1.9, 100.0),
            ],
        };
        assert!(insert_operation(&mut conn, &new, at("2026-10-21T11:00:00Z")).is_err());
        new.legs[1] = leg(Role::Cover, "winamax_fr", Outcome::Away, 4.0, 10.0);
        assert!(insert_operation(&mut conn, &new, at("2026-10-21T11:00:00Z")).is_err());
        // La transaction a tout annulé : aucune opération à moitié écrite.
        assert!(operations(&conn).unwrap().is_empty());
    }
}
