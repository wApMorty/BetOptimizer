//! Journal des alertes d'exploitation (table `alert_log`, migration 23) : source figée, unité systemd en échec.
//!
//! Un timer passe toutes les heures, voire toutes les dix minutes : sans mémoire, une panne qui dure enverrait la même
//! alerte à chaque passage. Avant d'envoyer, l'appelant demande à [`due`] si l'alerte est due ; après un envoi réussi, il
//! l'inscrit avec [`record`]. Un envoi raté n'est pas inscrit : l'alerte repartira au passage suivant.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::journal::{parse_timestamp, timestamp};
use crate::error::Result;

/// Nature d'une alerte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    /// Une source affiche des données périmées (prono-match figé, `docs/sources.md` §73).
    StaleSource,
    /// Une unité systemd d'edge a échoué.
    UnitFailed,
    /// La FDJ n'a pas servi une page du Loto Foot, resultats-fdj.fr l'a lue à la place (phase 22, `docs/sources.md` §74).
    SourceFallback,
    /// Ni la FDJ ni resultats-fdj.fr n'ont pu servir une page du Loto Foot.
    SourceDown,
    /// Une grille du Loto Foot entièrement annulée porte des tickets joués : mises remboursées au guichet, à saisir à la main.
    CanceledGrid,
}

impl AlertKind {
    /// Valeur écrite en base.
    fn code(self) -> &'static str {
        match self {
            AlertKind::StaleSource => "stale_source",
            AlertKind::UnitFailed => "unit_failed",
            AlertKind::SourceFallback => "source_fallback",
            AlertKind::SourceDown => "source_down",
            AlertKind::CanceledGrid => "canceled_grid",
        }
    }
}

/// Inscrit une alerte envoyée. `subject` : ce qu'elle désigne (`lf8-2026-154`, `edge-snapshot.service`).
pub fn record(
    conn: &Connection,
    kind: AlertKind,
    subject: &str,
    detail: &str,
    sent_at: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO alert_log (sent_at, kind, subject, detail) VALUES (?1, ?2, ?3, ?4)",
        params![timestamp(sent_at), kind.code(), subject, detail],
    )?;
    Ok(())
}

/// Dernier envoi de cette alerte, s'il y en a eu un.
pub fn last_sent(
    conn: &Connection,
    kind: AlertKind,
    subject: &str,
) -> Result<Option<DateTime<Utc>>> {
    // `optional()` : une requête sans ligne devient `None` au lieu d'une erreur.
    let text: Option<String> = conn
        .query_row(
            "SELECT sent_at FROM alert_log WHERE kind = ?1 AND subject = ?2 ORDER BY sent_at DESC LIMIT 1",
            params![kind.code(), subject],
            |row| row.get(0),
        )
        .optional()?;
    // `transpose` : d'un `Option<Result<…>>` (lire la date, si elle existe) on fait un `Result<Option<…>>`, que `?` sait
    // propager.
    text.map(|t| parse_timestamp(&t)).transpose()
}

/// L'alerte est-elle due à `now` ? Oui si elle n'est jamais partie ; sinon, seulement si `repeat_after` est donné et
/// écoulé depuis le dernier envoi (`None` : une alerte qui ne part qu'une fois par sujet).
pub fn due(
    conn: &Connection,
    kind: AlertKind,
    subject: &str,
    now: DateTime<Utc>,
    repeat_after: Option<Duration>,
) -> Result<bool> {
    Ok(match (last_sent(conn, kind, subject)?, repeat_after) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(last), Some(delay)) => now - last >= delay,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    #[test]
    fn an_alert_without_repeat_is_due_once_per_subject() {
        let conn = database();
        let noon = at("2026-11-12T12:00:00Z");
        let kind = AlertKind::StaleSource;

        assert!(due(&conn, kind, "lf8-2026-154", noon, None).unwrap());
        record(&conn, kind, "lf8-2026-154", "source figée", noon).unwrap();

        let a_month_later = at("2026-12-12T12:00:00Z");
        assert!(!due(&conn, kind, "lf8-2026-154", a_month_later, None).unwrap());
        // Un autre sujet, ou une autre nature d'alerte, reste dû.
        assert!(due(&conn, kind, "lf7-2026-139", noon, None).unwrap());
        assert!(due(&conn, AlertKind::UnitFailed, "lf8-2026-154", noon, None).unwrap());
    }

    #[test]
    fn a_repeating_alert_waits_for_its_delay() {
        let conn = database();
        let kind = AlertKind::UnitFailed;
        let unit = "edge-closing.service";
        let six_hours = Some(Duration::hours(6));
        record(&conn, kind, unit, "échec", at("2026-11-12T12:00:00Z")).unwrap();

        assert!(!due(&conn, kind, unit, at("2026-11-12T12:10:00Z"), six_hours).unwrap());
        assert!(!due(&conn, kind, unit, at("2026-11-12T17:59:00Z"), six_hours).unwrap());
        assert!(due(&conn, kind, unit, at("2026-11-12T18:00:00Z"), six_hours).unwrap());

        // Le délai court depuis le DERNIER envoi, pas depuis le premier.
        record(&conn, kind, unit, "échec", at("2026-11-12T18:00:00Z")).unwrap();
        assert!(!due(&conn, kind, unit, at("2026-11-12T20:00:00Z"), six_hours).unwrap());
        assert_eq!(
            last_sent(&conn, kind, unit).unwrap(),
            Some(at("2026-11-12T18:00:00Z"))
        );
    }

    #[test]
    fn the_log_can_be_neither_rewritten_nor_emptied() {
        let conn = database();
        record(
            &conn,
            AlertKind::UnitFailed,
            "edge-snapshot.service",
            "échec",
            at("2026-11-12T12:00:00Z"),
        )
        .unwrap();

        assert!(
            conn.execute("UPDATE alert_log SET detail = ''", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM alert_log", []).is_err());
        // Une nature inconnue ou un sujet vide sont refusés par la base elle-même.
        assert!(
            conn.execute(
                "INSERT INTO alert_log (sent_at, kind, subject, detail) VALUES ('x', 'autre', 's', 'd')",
                []
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO alert_log (sent_at, kind, subject, detail) VALUES ('x', 'unit_failed', '', 'd')",
                []
            )
            .is_err()
        );
    }
}
