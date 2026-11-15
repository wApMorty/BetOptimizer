//! Alias proposés par la déduction par la rencontre et décisions de Paul (tables `alias_proposal` et `alias_decision`,
//! migration 26, phase 23).
//!
//! Une proposition s'inscrit une fois ([`record`]) ; elle attend jusqu'à sa décision ([`decide`]), écrite une fois elle
//! aussi. Les alias acceptés ([`accepted`]) complètent la table versionnée sans recompiler edge
//! ([`crate::ingest::aliases::Aliases::with_learned`]), jusqu'à leur report dans `config/aliases.toml`.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::journal::{parse_timestamp, timestamp};
use crate::error::{Error, Result};
use crate::ingest::aliases::Aliases;
use crate::lotofoot::alias_deduction::{Proposal, Seen, SeenSource};

/// Une proposition inscrite, avec sa décision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// Numéro.
    pub id: i64,
    /// Date de l'inscription.
    pub proposed_at: DateTime<Utc>,
    /// La proposition.
    pub proposal: Proposal,
    /// Décision : `Some(true)` acceptée, `Some(false)` refusée, `None` en attente.
    pub accepted: Option<bool>,
}

/// Inscrit une proposition ; `false` si la même (section, nom, nom canonique) l'est déjà : la première preuve est gardée,
/// et une proposition refusée ne revient pas.
pub fn record(conn: &Connection, proposal: &Proposal, at: DateTime<Utc>) -> Result<bool> {
    let seen = &proposal.seen;
    let written = conn.execute(
        "INSERT OR IGNORE INTO alias_proposal
             (proposed_at, section, name, canonical, context, source, source_id, seen_home, seen_away, kickoff)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            timestamp(at),
            proposal.section,
            proposal.name,
            proposal.canonical,
            proposal.context,
            seen.source.code(),
            seen.id,
            seen.home,
            seen.away,
            timestamp(seen.kickoff),
        ],
    )?;
    Ok(written == 1)
}

const SELECT: &str =
    "SELECT p.id, p.proposed_at, p.section, p.name, p.canonical, p.context, p.source, p.source_id,
                             p.seen_home, p.seen_away, p.kickoff, d.accepted
                      FROM alias_proposal p LEFT JOIN alias_decision d ON d.proposal_id = p.id";

/// Une ligne de [`SELECT`], telle que lue. Les dates et la source restent du texte : leur lecture peut échouer avec une
/// erreur d'edge, que la fermeture de `rusqlite` ne sait pas porter ; [`stored`] les convertit ensuite.
struct RawRow {
    id: i64,
    proposed_at: String,
    section: String,
    name: String,
    canonical: String,
    context: String,
    source: String,
    source_id: String,
    seen_home: String,
    seen_away: String,
    kickoff: String,
    accepted: Option<bool>,
}

fn raw_row(row: &Row<'_>) -> rusqlite::Result<RawRow> {
    Ok(RawRow {
        id: row.get(0)?,
        proposed_at: row.get(1)?,
        section: row.get(2)?,
        name: row.get(3)?,
        canonical: row.get(4)?,
        context: row.get(5)?,
        source: row.get(6)?,
        source_id: row.get(7)?,
        seen_home: row.get(8)?,
        seen_away: row.get(9)?,
        kickoff: row.get(10)?,
        accepted: row.get(11)?,
    })
}

/// Convertit une ligne lue. `raw` est pris par valeur : ses textes passent dans la proposition sans copie.
fn stored(raw: RawRow) -> Result<Stored> {
    let source = SeenSource::from_code(&raw.source).ok_or_else(|| {
        Error::Data(format!(
            "source d'alias inconnue en base : « {} »",
            raw.source
        ))
    })?;
    Ok(Stored {
        id: raw.id,
        proposed_at: parse_timestamp(&raw.proposed_at)?,
        proposal: Proposal {
            section: raw.section,
            name: raw.name,
            canonical: raw.canonical,
            context: raw.context,
            seen: Seen {
                source,
                id: raw.source_id,
                home: raw.seen_home,
                away: raw.seen_away,
                kickoff: parse_timestamp(&raw.kickoff)?,
            },
        },
        accepted: raw.accepted,
    })
}

/// Toutes les propositions, dans l'ordre d'inscription.
pub fn all(conn: &Connection) -> Result<Vec<Stored>> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY p.id"))?;
    let raws = stmt
        .query_map([], raw_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raws.into_iter().map(stored).collect()
}

/// Une proposition par son numéro.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Stored>> {
    let raw = conn
        .query_row(&format!("{SELECT} WHERE p.id = ?1"), [id], raw_row)
        .optional()?;
    raw.map(stored).transpose()
}

/// Propositions en attente : sans décision, et dont le nom est encore inconnu de `aliases` (table versionnée complétée
/// des alias acceptés). Une proposition concurrente d'un alias accepté depuis disparaît ainsi d'elle-même.
pub fn pending(conn: &Connection, aliases: &Aliases) -> Result<Vec<Stored>> {
    Ok(all(conn)?
        .into_iter()
        .filter(|s| s.accepted.is_none())
        .filter(|s| {
            aliases
                .resolve(&s.proposal.section, &s.proposal.name)
                .is_none()
        })
        .collect())
}

/// Alias acceptés (section, nom, nom canonique), dans l'ordre des décisions.
pub fn accepted(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT p.section, p.name, p.canonical FROM alias_proposal p
         JOIN alias_decision d ON d.proposal_id = p.id WHERE d.accepted = 1 ORDER BY d.decided_at, p.id",
    )?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Accepte (`accept`) ou refuse une proposition en attente, et renvoie le message à afficher.
///
/// `bundled` est la table versionnée : les alias déjà acceptés y sont ajoutés ici, pour refuser une proposition dont le nom
/// est entre-temps devenu connu (une autre proposition acceptée, une nouvelle version d'edge).
pub fn decide(
    conn: &mut Connection,
    bundled: &Aliases,
    id: i64,
    accept: bool,
    at: DateTime<Utc>,
) -> Result<String> {
    // Transaction : la vérification et l'écriture forment un tout, qu'un second clic ne peut pas couper en deux.
    let tx = conn.transaction()?;
    let Some(found) = get(&tx, id)? else {
        return Err(Error::Input(format!(
            "proposition d'alias n°{id} introuvable"
        )));
    };
    let p = &found.proposal;
    if let Some(done) = found.accepted {
        let state = if done { "acceptée" } else { "refusée" };
        return Err(Error::Input(format!(
            "proposition n°{id} déjà {state} : « {} » → « {} »",
            p.name, p.canonical
        )));
    }
    if accept {
        let merged = bundled.with_learned(&tx)?;
        if let Some(team) = merged.resolve(&p.section, &p.name) {
            return Err(Error::Input(format!(
                "« {} » est déjà connu de la section {} (« {team} ») : proposition sans objet, refuse-la",
                p.name, p.section
            )));
        }
    }
    tx.execute(
        "INSERT INTO alias_decision (proposal_id, decided_at, accepted) VALUES (?1, ?2, ?3)",
        params![id, timestamp(at), accept],
    )?;
    tx.commit()?;
    Ok(if accept {
        format!(
            "Alias enregistré ({}) : « {} » → « {} ». Relance la séance pour en tenir compte.",
            p.section, p.name, p.canonical
        )
    } else {
        format!(
            "Proposition refusée ({}) : « {} » → « {} ».",
            p.section, p.name, p.canonical
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn proposal(name: &str, canonical: &str) -> Proposal {
        Proposal {
            section: "LOTOFOOT".to_string(),
            name: name.to_string(),
            canonical: canonical.to_string(),
            context: "LF8 n°151, rencontre 3 : Malaga – Fulham".to_string(),
            seen: Seen {
                source: SeenSource::OddsApi,
                id: "ev1".to_string(),
                home: canonical.to_string(),
                away: "Fulham".to_string(),
                kickoff: at("2026-11-01T15:00:00Z"),
            },
        }
    }

    const TABLE: &str = "[LOTOFOOT]\n\"Fulham\" = []\n\"Málaga\" = []\n";

    #[test]
    fn a_proposal_is_recorded_once_and_read_back_whole() {
        let conn = db();
        let p = proposal("Malaga", "Málaga");
        assert!(record(&conn, &p, at("2026-11-15T10:00:00Z")).unwrap());
        assert!(!record(&conn, &p, at("2026-11-15T11:00:00Z")).unwrap());
        let stored = all(&conn).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].proposal, p);
        assert_eq!(stored[0].proposed_at, at("2026-11-15T10:00:00Z"));
        assert_eq!(stored[0].accepted, None);
    }

    #[test]
    fn an_accepted_alias_is_learned_and_competing_proposals_vanish() {
        let mut conn = db();
        let bundled = Aliases::parse(TABLE).unwrap();
        let now = at("2026-11-15T10:00:00Z");
        record(&conn, &proposal("Malaga", "Málaga"), now).unwrap();
        record(&conn, &proposal("Malaga", "Malaga CF"), now).unwrap();
        assert_eq!(pending(&conn, &bundled).unwrap().len(), 2);

        let message = decide(&mut conn, &bundled, 1, true, now).unwrap();
        assert!(message.contains("« Malaga » → « Málaga »"), "{message}");
        let merged = bundled.with_learned(&conn).unwrap();
        assert_eq!(merged.resolve("LOTOFOOT", "Malaga"), Some("Málaga"));
        // L'autre proposition pour « Malaga » n'a plus d'objet : elle n'est plus en attente…
        assert!(pending(&conn, &merged).unwrap().is_empty());
        // … et l'accepter est refusé avec la raison.
        let err = decide(&mut conn, &bundled, 2, true, now).unwrap_err();
        assert!(err.to_string().contains("déjà connu"), "{err}");
        // Une décision est définitive.
        let err = decide(&mut conn, &bundled, 1, false, now).unwrap_err();
        assert!(err.to_string().contains("déjà acceptée"), "{err}");
        assert_eq!(
            accepted(&conn).unwrap(),
            [(
                "LOTOFOOT".to_string(),
                "Malaga".to_string(),
                "Málaga".to_string()
            )]
        );
    }

    #[test]
    fn a_refused_proposal_is_not_learned_nor_proposed_again() {
        let mut conn = db();
        let bundled = Aliases::parse(TABLE).unwrap();
        let now = at("2026-11-15T10:00:00Z");
        record(&conn, &proposal("Malaga", "Malaga CF"), now).unwrap();
        decide(&mut conn, &bundled, 1, false, now).unwrap();
        assert!(pending(&conn, &bundled).unwrap().is_empty());
        assert!(!record(&conn, &proposal("Malaga", "Malaga CF"), now).unwrap());
        assert_eq!(
            bundled
                .with_learned(&conn)
                .unwrap()
                .resolve("LOTOFOOT", "Malaga"),
            None
        );
        assert!(
            decide(&mut conn, &bundled, 9, true, now)
                .unwrap_err()
                .to_string()
                .contains("introuvable")
        );
    }

    #[test]
    fn proposals_and_decisions_cannot_be_changed() {
        let mut conn = db();
        let now = at("2026-11-15T10:00:00Z");
        record(&conn, &proposal("Malaga", "Málaga"), now).unwrap();
        decide(&mut conn, &Aliases::default(), 1, true, now).unwrap();
        assert!(
            conn.execute("UPDATE alias_proposal SET canonical = 'X'", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM alias_proposal", []).is_err());
        assert!(
            conn.execute("UPDATE alias_decision SET accepted = 0", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM alias_decision", []).is_err());
    }
}
