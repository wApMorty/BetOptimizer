//! Alerte « nouvelle grille » : prévenir Paul sur Discord de chaque grille LF7 ou LF8 qui sort, avec son heure de clôture,
//! sans aucun ticket (il est trop tôt : la répartition des autres joueurs n'est pas encore représentative, §47).
//!
//! Une grille est à signaler tant qu'elle n'est pas close et que son alerte n'est pas notée au journal
//! (`lotofoot_alert`). L'alerte n'est notée qu'après l'envoi réussi : un échec est retenté au relevé suivant, et peu importe
//! quel relevé (horaire, séance, bouton) a vu la grille en premier.
//!
//! Le module décide aussi des **alertes de source** du relevé (phase 22, `docs/sources.md` §74) : page périmée, FDJ en échec
//! avec repli, aucune source, grille annulée. Chacune est limitée par le journal `alert_log` ([`crate::store::alerts`]) :
//! le relevé passe toutes les heures, et sans cette mémoire une panne qui dure enverrait le même message à chaque passage.

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;

use super::GridId;
use super::fetch::FetchReport;
use crate::error::Result;
use crate::report::lotofoot_page;
use crate::store::alerts::{self, AlertKind};
use crate::store::lotofoot as store;

/// Une grille à signaler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Grille.
    pub grid: GridId,
    /// Fin de validation, en UTC.
    pub closes_at: DateTime<Utc>,
    /// Pactole annoncé, en euros entiers.
    pub jackpot: Option<u32>,
    /// La répartition FDJ est déjà publiée (sinon la grille est seulement annoncée, ou vient d'ouvrir).
    pub split_published: bool,
}

/// Grilles encore ouvertes à `now` dont l'alerte n'a pas été envoyée, dans l'ordre de clôture.
pub fn pending(conn: &Connection, now: DateTime<Utc>) -> Result<Vec<Listed>> {
    let mut listed = Vec::new();
    for row in store::grids_closing_after(conn, now)? {
        if store::alerted(conn, row.id)? {
            continue;
        }
        listed.push(Listed {
            grid: row.id,
            closes_at: row.closes_at,
            // Un pactole est un montant rond en euros : l'arrondi ne perd rien.
            jackpot: row.jackpot.map(|j| j.get().round() as u32),
            split_published: store::latest_split(conn, row.id, now)?.is_some(),
        });
    }
    Ok(listed)
}

/// Note les alertes comme envoyées (à appeler seulement après un envoi réussi).
pub fn record(conn: &Connection, listed: &[Listed], sent_at: DateTime<Utc>) -> Result<()> {
    for grid in listed {
        store::record_alert(conn, grid.grid, sent_at)?;
    }
    Ok(())
}

/// Heures entre deux alertes « FDJ en échec, repli utilisé » : le repli sert tant que la FDJ est en panne, une alerte par
/// jour suffit (consigne de la phase 22).
pub const FALLBACK_ALERT_EVERY_HOURS: i64 = 24;

/// Heures entre deux alertes « aucune source » pour une même page : la même cadence que les unités en échec
/// (`notify::message::UNIT_ALERT_EVERY_HOURS`).
pub const DOWN_ALERT_EVERY_HOURS: i64 = 6;

/// Sujet de l'alerte « FDJ en échec » : une seule alerte pour toutes les pages, au plus une fois par jour.
const FALLBACK_SUBJECT: &str = "fdj";

/// Une alerte de source à envoyer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAlert {
    /// Nature de l'alerte, clé du journal `alert_log`.
    pub kind: AlertKind,
    /// Ce que l'alerte désigne (grille ou page) ; chaque sujet est inscrit au journal après un envoi réussi.
    pub subjects: Vec<String>,
    /// Texte du message Discord.
    pub text: String,
}

/// Alertes de source dues à `now` d'après le bilan d'un relevé, une par nature au plus, dans cet ordre :
/// - page périmée : une fois par grille ;
/// - grille entièrement annulée portant des tickets joués : une fois par grille ;
/// - FDJ en échec, repli utilisé : au plus une fois par [`FALLBACK_ALERT_EVERY_HOURS`] heures ;
/// - ni la FDJ ni le repli : au plus une fois par [`DOWN_ALERT_EVERY_HOURS`] heures et par page.
///
/// Rien n'est inscrit ici : l'appelant envoie, puis inscrit avec [`record_source_alert`] (un envoi raté repart au relevé
/// suivant).
pub fn source_alerts(
    conn: &Connection,
    report: &FetchReport,
    now: DateTime<Utc>,
) -> Result<Vec<SourceAlert>> {
    let mut due = Vec::new();

    let mut stale = Vec::new();
    for shown in &report.stale {
        if alerts::due(conn, AlertKind::StaleSource, &shown.grid.code(), now, None)? {
            // `*shown` : `StaleGrid` est `Copy`, on en garde une copie plutôt qu'un emprunt du bilan.
            stale.push(*shown);
        }
    }
    if !stale.is_empty() {
        due.push(SourceAlert {
            kind: AlertKind::StaleSource,
            subjects: stale.iter().map(|s| s.grid.code()).collect(),
            text: lotofoot_page::stale_text(&stale),
        });
    }

    let mut canceled = Vec::new();
    for id in &report.canceled {
        if alerts::due(conn, AlertKind::CanceledGrid, &id.code(), now, None)? {
            canceled.push(*id);
        }
    }
    if !canceled.is_empty() {
        due.push(SourceAlert {
            kind: AlertKind::CanceledGrid,
            subjects: canceled.iter().map(|id| id.code()).collect(),
            text: lotofoot_page::canceled_text(&canceled),
        });
    }

    let daily = Some(Duration::hours(FALLBACK_ALERT_EVERY_HOURS));
    if !report.fallbacks.is_empty()
        && alerts::due(
            conn,
            AlertKind::SourceFallback,
            FALLBACK_SUBJECT,
            now,
            daily,
        )?
    {
        due.push(SourceAlert {
            kind: AlertKind::SourceFallback,
            subjects: vec![FALLBACK_SUBJECT.to_string()],
            text: lotofoot_page::fallback_text(&report.fallbacks),
        });
    }

    let every = Some(Duration::hours(DOWN_ALERT_EVERY_HOURS));
    let mut down = Vec::new();
    for issue in &report.failures {
        if alerts::due(conn, AlertKind::SourceDown, issue.page.code(), now, every)? {
            // `clone` : le bilan reste à l'appelant, qui l'affiche encore après l'envoi.
            down.push(issue.clone());
        }
    }
    if !down.is_empty() {
        due.push(SourceAlert {
            kind: AlertKind::SourceDown,
            subjects: down.iter().map(|i| i.page.code().to_string()).collect(),
            text: lotofoot_page::down_text(&down),
        });
    }
    Ok(due)
}

/// Inscrit au journal les sujets d'une alerte de source envoyée (à appeler seulement après un envoi réussi).
pub fn record_source_alert(
    conn: &Connection,
    alert: &SourceAlert,
    sent_at: DateTime<Utc>,
) -> Result<()> {
    for subject in &alert.subjects {
        alerts::record(conn, alert.kind, subject, &alert.text, sent_at)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Euros;
    use crate::ingest::aliases::Aliases;
    use crate::lotofoot::Game;
    use crate::lotofoot::grid_page::OpenGrid;
    use crate::store::lotofoot::{Page, Source, record_fetch, record_open_grid};
    use crate::store::migrate::migrate;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Une grille annoncée, sans répartition.
    fn announce(conn: &mut Connection, game: Game, number: u32, closes: &str) {
        let fetch = record_fetch(
            conn,
            at("2026-10-24T08:07:00Z"),
            Source::Fdj,
            Page::Open,
            200,
            None,
        )
        .unwrap();
        let page = OpenGrid {
            game,
            accepting: false,
            number,
            closes_at: at(closes),
            jackpot: Euros::new(150_000.0),
            matches: (1..=game.matches())
                .map(|i| (format!("H{i}"), format!("A{i}")))
                .collect(),
            split: None,
            split_at: None,
        };
        record_open_grid(conn, &Aliases::default(), fetch, &page).unwrap();
    }

    #[test]
    fn an_open_grid_is_signalled_once_and_a_closed_one_never() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        announce(&mut conn, Game::Lf8, 147, "2026-10-26T17:55:00Z");
        announce(&mut conn, Game::Lf7, 132, "2026-10-22T18:00:00Z");
        let now = at("2026-10-24T09:00:00Z");
        let first = pending(&conn, now).unwrap();
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(
            (first[0].grid.game, first[0].grid.number, first[0].jackpot),
            (Game::Lf8, 147, Some(150_000))
        );
        assert!(!first[0].split_published);
        // Rien n'est noté tant que l'envoi n'a pas réussi : la grille reste à signaler.
        assert_eq!(pending(&conn, now).unwrap(), first);
        record(&conn, &first, now).unwrap();
        assert!(pending(&conn, now).unwrap().is_empty());
        // Noter deux fois la même alerte ne fait rien.
        record(&conn, &first, now).unwrap();
    }

    use super::super::fetch::{PageIssue, StaleGrid};
    use crate::store::lotofoot::{Page as FetchPage, Source as FetchSource};

    fn lf8_154() -> GridId {
        GridId::closing(Game::Lf8, 154, at("2026-11-06T18:55:00Z"))
    }

    #[test]
    fn a_stale_page_and_a_canceled_grid_are_each_signalled_once_per_grid() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let now = at("2026-11-15T10:00:00Z");
        let report = FetchReport {
            stale: vec![StaleGrid {
                grid: lf8_154(),
                closes_at: at("2026-11-06T18:55:00Z"),
                source: FetchSource::Fdj,
            }],
            canceled: vec![lf8_154()],
            ..FetchReport::default()
        };

        let due = source_alerts(&conn, &report, now).unwrap();

        let kinds: Vec<AlertKind> = due.iter().map(|a| a.kind).collect();
        assert_eq!(kinds, [AlertKind::StaleSource, AlertKind::CanceledGrid]);
        assert_eq!(due[1].subjects, ["lf8-2026-154"]);
        // Rien n'est noté avant l'envoi : les mêmes alertes sont encore dues.
        assert_eq!(source_alerts(&conn, &report, now).unwrap(), due);
        for alert in &due {
            record_source_alert(&conn, alert, now).unwrap();
        }
        // Une fois envoyées, elles ne repartent plus, même un mois plus tard.
        let later = at("2026-12-15T10:00:00Z");
        assert!(source_alerts(&conn, &report, later).unwrap().is_empty());
    }

    #[test]
    fn the_fdj_fallback_alert_repeats_after_a_day_and_the_down_alert_after_six_hours_per_page() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let report = FetchReport {
            fallbacks: vec![PageIssue {
                page: FetchPage::Open,
                reason: "réponse HTTP 403 inattendue".to_string(),
            }],
            failures: vec![
                PageIssue {
                    page: FetchPage::Open,
                    reason: "FDJ : a ; resultats-fdj.fr : b".to_string(),
                },
                PageIssue {
                    page: FetchPage::Results,
                    reason: "FDJ : c ; resultats-fdj.fr : d".to_string(),
                },
            ],
            ..FetchReport::default()
        };
        let noon = at("2026-11-15T12:00:00Z");

        let first = source_alerts(&conn, &report, noon).unwrap();

        assert_eq!(
            first.iter().map(|a| a.kind).collect::<Vec<_>>(),
            [AlertKind::SourceFallback, AlertKind::SourceDown]
        );
        // Un sujet par page pour « aucune source » : chaque page a sa propre cadence.
        assert_eq!(first[1].subjects, ["open", "results"]);
        for alert in &first {
            record_source_alert(&conn, alert, noon).unwrap();
        }

        // Six heures moins une minute : rien. Six heures pile : « aucune source » repart, pas le repli (24 h).
        assert!(
            source_alerts(&conn, &report, at("2026-11-15T17:59:00Z"))
                .unwrap()
                .is_empty()
        );
        let six = source_alerts(&conn, &report, at("2026-11-15T18:00:00Z")).unwrap();
        assert_eq!(six.len(), 1);
        assert_eq!(six[0].kind, AlertKind::SourceDown);
        // Un jour plus tard, le repli repart aussi.
        let day = source_alerts(&conn, &report, at("2026-11-16T12:00:00Z")).unwrap();
        assert_eq!(day.len(), 2);

        // Seule la page des résultats tombe : l'alerte de la page des grilles ouvertes, déjà envoyée, ne repart pas.
        let mut partial = report.clone();
        partial.failures.remove(0);
        partial.fallbacks.clear();
        assert!(
            source_alerts(&conn, &partial, at("2026-11-15T12:30:00Z"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_clean_report_signals_nothing() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let due = source_alerts(&conn, &FetchReport::default(), at("2026-11-15T12:00:00Z"));
        assert!(due.unwrap().is_empty());
    }
}
