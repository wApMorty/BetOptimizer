//! Relevé horaire des pages de prono-match.fr (`edge lotofoot fetch`) : grilles LF7 et LF8 en cours, répartition FDJ,
//! rapports des dernières grilles et résultats officiels. Trois pages par passage, une toutes les 5 secondes.
//!
//! **Page périmée** (`docs/sources.md` §73) : une page qui affiche encore comme « grille en cours » une grille close
//! depuis plus de [`STALE_AFTER_HOURS`] heures n'est plus à jour. Du 5 au 12 novembre 2026, le site est resté figé sur le
//! LF8 n°154 et le relevé a enregistré la même page toutes les heures, sans rien signaler. Une telle lecture est
//! désormais journalisée avec une erreur, remontée dans [`FetchReport::stale`], et sa répartition n'est pas archivée.
//!
//! L'appel réseau, l'horloge et les pauses sont reçus en paramètres : en production le vrai client, `Utc::now` et
//! `thread::sleep`, dans les tests des pages synthétiques. Une page illisible n'arrête pas le passage : la lecture est
//! journalisée avec son erreur, et les autres pages sont relevées.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::grid_page::OpenGrid;
use super::pronomatch::{self, BASE_URL, RESULTS_PATH, grid_path};
use super::{Game, GridId};
use crate::error::{Error, Result};
use crate::format::kickoff_label;
use crate::ingest::client::HttpResponse;
use crate::ingest::oddspapi::fetch::Pause;
use crate::ingest::snapshot::{Clock, Fetch};
use crate::store::lotofoot::{self as store, Page};

/// Délai entre deux pages : le site n'est pas une API, on le ménage.
pub const PAGE_COOLDOWN: Duration = Duration::from_secs(5);

/// Heures après la clôture au-delà desquelles une grille encore affichée « en cours » signale une page périmée.
/// Sur les quatorze grilles relevées avant la panne, le site avait changé de grille dès le relevé suivant la clôture
/// (moins d'une heure) : trois heures laissent de la marge sans retarder l'alerte.
pub const STALE_AFTER_HOURS: i64 = 3;

/// Une grille close que le site affiche encore comme grille en cours.
///
/// `Copy` : deux petites valeurs, que l'on passe sans se soucier de qui les possède.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleGrid {
    /// La grille.
    pub grid: GridId,
    /// Sa fin de validation, en UTC.
    pub closes_at: DateTime<Utc>,
}

impl StaleGrid {
    /// Raison écrite au journal des lectures (`lotofoot_fetch.error`) et montrée dans l'application.
    pub fn reason(&self) -> String {
        format!(
            "page périmée : {} encore affichée comme grille en cours, close depuis {}",
            self.grid,
            kickoff_label(self.closes_at)
        )
    }
}

/// La grille affichée est-elle close depuis assez longtemps, à `at`, pour que la page soit dite périmée ?
fn stale_grid(game: Game, shown: &OpenGrid, at: DateTime<Utc>) -> Option<StaleGrid> {
    (at - shown.closes_at >= chrono::Duration::hours(STALE_AFTER_HOURS)).then(|| StaleGrid {
        grid: GridId::closing(game, shown.number, shown.closes_at),
        closes_at: shown.closes_at,
    })
}

/// Client HTTP bloquant du site (aucune clé).
pub struct PronoMatchClient {
    agent: ureq::Agent,
}

impl Default for PronoMatchClient {
    fn default() -> Self {
        PronoMatchClient::new()
    }
}

impl PronoMatchClient {
    /// Crée le client. Le `User-Agent` nomme l'outil, sans donnée personnelle.
    pub fn new() -> PronoMatchClient {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent("edge/0.1 (usage personnel, une page par heure)")
            .build();
        PronoMatchClient {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// `GET` d'un chemin du site (`loto-foot-7/`) ; le second argument (paramètres) est ignoré.
    pub fn get(&self, path: &str, _params: &str) -> Result<HttpResponse> {
        let url = format!("{BASE_URL}{path}");
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| Error::Http(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(HttpResponse {
            status,
            quota: Default::default(),
            body,
        })
    }
}

/// Bilan d'un passage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchReport {
    /// Grilles en cours vues (une par formule au plus).
    pub grids: Vec<String>,
    /// Répartitions de rencontres écrites.
    pub splits: usize,
    /// Grilles dont les résultats ont été écrits pour la première fois.
    pub new_results: usize,
    /// Pages non lues, avec la raison.
    pub failures: Vec<String>,
    /// Pages lues mais périmées : la grille qu'elles affichent « en cours » est close depuis longtemps.
    pub stale: Vec<StaleGrid>,
    /// Lues mais à signaler : composition d'une grille changée, résultats refusés (`docs/sources.md` §52).
    pub warnings: Vec<String>,
}

/// Lance un passage : page du LF7, page du LF8, page des résultats.
pub fn run(conn: &mut Connection, now: Clock, fetch: Fetch, pause: Pause) -> Result<FetchReport> {
    let mut report = FetchReport::default();
    for (i, game) in Game::ALL.into_iter().enumerate() {
        if i > 0 {
            pause(PAGE_COOLDOWN);
        }
        let at = now();
        match fetch(&grid_path(game), "") {
            Err(error) => failed(
                conn,
                at,
                Page::Grid(game),
                0,
                &error.to_string(),
                &mut report,
            )?,
            Ok(response) if response.status != 200 => failed(
                conn,
                at,
                Page::Grid(game),
                response.status,
                "réponse HTTP inattendue",
                &mut report,
            )?,
            Ok(response) => match pronomatch::parse_grid_page(&response.body, game, at) {
                Err(error) => failed(
                    conn,
                    at,
                    Page::Grid(game),
                    200,
                    &error.to_string(),
                    &mut report,
                )?,
                Ok(mut page) => {
                    // Une grille « en cours » dont la clôture est passée : le site n'a pas encore changé de grille, ou
                    // il est figé. `take_if` la retire de `page` et nous la remet : ni sa composition ni sa répartition
                    // (qui n'est plus une observation d'avant clôture) ne sont écrites ; les rapports passés le sont.
                    let closed = page.open.take_if(|grid| grid.closes_at <= at);
                    let stale = closed.as_ref().and_then(|grid| stale_grid(game, grid, at));
                    let reason = stale.map(|s| s.reason());
                    let fetch_id =
                        store::record_fetch(conn, at, Page::Grid(game), 200, reason.as_deref())?;
                    let record = store::record_grid_page(conn, fetch_id, game, &page)?;
                    match (stale, closed) {
                        (Some(stale), _) => report.stale.push(stale),
                        (None, Some(grid)) => report.grids.push(format!(
                            "{game} n°{} (close, grille suivante pas encore affichée)",
                            grid.number
                        )),
                        (None, None) => {}
                    }
                    report.splits += record.splits;
                    if record.undated > 0 {
                        report.warnings.push(format!(
                            "{game} : {} ligne(s) de rapports sans jour de clôture lisible, non écrites",
                            record.undated
                        ));
                    }
                    let written = record.refused.is_none();
                    // `extend` prend le refus (un `Option` se parcourt comme une liste de zéro ou un élément) : il n'est
                    // plus lu ensuite, inutile de le copier.
                    report.warnings.extend(record.refused);
                    if let Some(grid) = page.open.as_ref().filter(|_| written) {
                        if record.recomposed {
                            report.warnings.push(format!(
                                "{game} n°{} : les rencontres ont changé, nouvelle composition enregistrée ; \
                                 répartitions et tickets antérieurs écartés",
                                grid.number
                            ));
                        }
                        let state = if grid.split.is_some() {
                            "répartition publiée"
                        } else if grid.accepting {
                            "ouverte, sans répartition"
                        } else {
                            "pas encore ouverte"
                        };
                        report
                            .grids
                            .push(format!("{game} n°{} ({state})", grid.number));
                    }
                }
            },
        }
    }
    pause(PAGE_COOLDOWN);
    let at = now();
    match fetch(RESULTS_PATH, "") {
        Err(error) => failed(conn, at, Page::Results, 0, &error.to_string(), &mut report)?,
        Ok(response) if response.status != 200 => failed(
            conn,
            at,
            Page::Results,
            response.status,
            "réponse HTTP inattendue",
            &mut report,
        )?,
        Ok(response) => match pronomatch::parse_results_page(&response.body) {
            Err(error) => failed(
                conn,
                at,
                Page::Results,
                200,
                &error.to_string(),
                &mut report,
            )?,
            Ok(results) => {
                let fetch_id = store::record_fetch(conn, at, Page::Results, 200, None)?;
                let record = store::record_results(conn, fetch_id, &results)?;
                report.new_results += record.new;
                report.warnings.extend(record.refused);
            }
        },
    }
    Ok(report)
}

/// Journalise une page non lue et l'ajoute au bilan.
fn failed(
    conn: &Connection,
    at: DateTime<Utc>,
    page: Page,
    status: u16,
    reason: &str,
    report: &mut FetchReport,
) -> Result<()> {
    store::record_fetch(conn, at, page, status, Some(reason))?;
    report.failures.push(format!("{page:?} : {reason}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate::migrate;
    use chrono::{DateTime, Utc};

    fn page(n: usize, number: u32) -> String {
        let rows: String = (1..=n)
            .map(|i| {
                format!(
                    "<tr><td>{i}</td><td>A{i} - B{i}</td><td>50·30·20</td><td>—</td><td>1</td></tr>"
                )
            })
            .collect();
        format!(
            "<p><b>Loto Foot {n} n°{number}</b> · grille ouverte · clôture samedi 24 octobre à 16h55 · répartition des joueurs relevée samedi 24 octobre à 11h55.</p>\
             <table><thead><tr><th>#</th><th>Match</th><th>Joueurs</th></tr></thead><tbody>{rows}</tbody></table>\
             <table><tbody><tr><td>n°{}</td><td>vendredi 23 octobre</td><td><b>100 €</b><small>10 gagnants</small></td><td><b>5 €</b><small>2000 gagnants</small></td></tr></tbody></table>",
            number - 1
        )
    }

    #[test]
    fn a_pass_reads_three_pages_and_survives_a_broken_one() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut clock = || {
            DateTime::parse_from_rfc3339("2026-10-24T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        };
        let mut calls = Vec::new();
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            calls.push(path.to_string());
            let body = match path {
                "loto-foot-7/" => page(7, 133),
                "loto-foot-8/" => "<html>page en travaux</html>".to_string(),
                _ => "<html></html>".to_string(),
            };
            Ok(HttpResponse {
                status: 200,
                quota: Default::default(),
                body,
            })
        };
        let mut pauses = 0;
        let mut pause = |_: Duration| pauses += 1;
        let report = run(&mut conn, &mut clock, &mut fetch, &mut pause).unwrap();
        assert_eq!(
            calls,
            ["loto-foot-7/", "loto-foot-8/", "resultats-loto-foot/"]
        );
        assert_eq!(pauses, 2);
        assert_eq!(report.splits, 7);
        assert_eq!(report.grids, ["LF7 n°133 (répartition publiée)"]);
        // La page du LF8 n'a pas de grille : ce n'est pas une erreur, simplement rien à lire.
        assert!(report.failures.is_empty());
        let logged: i64 = conn
            .query_row("SELECT COUNT(*) FROM lotofoot_fetch", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logged, 3);
    }

    /// Un passage à `at` où la page du LF8 montre la grille n°`number` (clôture le 24 octobre à 14 h 55 UTC).
    fn pass_at(conn: &mut Connection, at: &str, number: u32) -> FetchReport {
        let at: DateTime<Utc> = at.parse().unwrap();
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            Ok(HttpResponse {
                status: 200,
                quota: Default::default(),
                body: if path == "loto-foot-8/" {
                    page(8, number)
                } else {
                    "<html></html>".to_string()
                },
            })
        };
        run(conn, &mut || at, &mut fetch, &mut |_| {}).unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_page_still_showing_a_grid_closed_for_hours_is_stale_and_its_split_is_not_archived() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        // Avant la clôture : relevé normal, huit répartitions écrites.
        let before = pass_at(&mut conn, "2026-10-24T10:00:00Z", 147);
        assert_eq!((before.splits, before.stale.len()), (8, 0));

        // Quatre heures après la clôture, la même page : elle est périmée.
        let after = pass_at(&mut conn, "2026-10-24T19:00:00Z", 147);

        assert_eq!(after.stale.len(), 1);
        assert_eq!(after.stale[0].grid.code(), "lf8-2026-147");
        assert_eq!(after.splits, 0);
        assert!(after.grids.is_empty() && after.failures.is_empty());
        // La répartition d'après clôture n'est pas archivée.
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM lotofoot_split"), 8);
        let error: String = conn
            .query_row(
                "SELECT error FROM lotofoot_fetch WHERE page = 'lf8' ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            error,
            "page périmée : LF8 n°147 encore affichée comme grille en cours, close depuis sam. 24/10 16:55"
        );

        // Sur une base neuve, une page périmée livre quand même les rapports des grilles passées (deux rangs du n°146),
        // mais pas la grille close qu'elle affiche à tort.
        let mut fresh = Connection::open_in_memory().unwrap();
        migrate(&mut fresh).unwrap();
        assert_eq!(
            pass_at(&mut fresh, "2026-10-24T19:00:00Z", 147).stale.len(),
            1
        );
        assert_eq!(count(&fresh, "SELECT COUNT(*) FROM lotofoot_rank"), 2);
        assert_eq!(count(&fresh, "SELECT COUNT(*) FROM lotofoot_grid"), 0);
    }

    #[test]
    fn a_grid_closed_for_less_than_three_hours_is_not_yet_a_stale_page() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();

        // Une heure après la clôture : le site n'a pas encore changé de grille, ce n'est pas une panne.
        let report = pass_at(&mut conn, "2026-10-24T16:00:00Z", 147);

        assert!(report.stale.is_empty());
        assert_eq!(
            report.grids,
            ["LF8 n°147 (close, grille suivante pas encore affichée)"]
        );
        assert_eq!(report.splits, 0);
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM lotofoot_fetch WHERE error IS NOT NULL"
            ),
            0
        );
        // Trois heures pile : périmée.
        let limit = pass_at(&mut conn, "2026-10-24T17:55:00Z", 147);
        assert_eq!(limit.stale.len(), 1);
    }

    #[test]
    fn a_grid_whose_matches_change_is_reported() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut clock = || {
            DateTime::parse_from_rfc3339("2026-10-24T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        };
        let mut pass = |conn: &mut Connection, lf7: String| {
            let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
                Ok(HttpResponse {
                    status: 200,
                    quota: Default::default(),
                    body: if path == "loto-foot-7/" {
                        lf7.clone()
                    } else {
                        "<html></html>".to_string()
                    },
                })
            };
            run(conn, &mut clock, &mut fetch, &mut |_| {}).unwrap()
        };
        assert!(pass(&mut conn, page(7, 133)).warnings.is_empty());
        // Même grille, rencontre 1 remplacée.
        let changed = page(7, 133).replace("A1 - B1", "C1 - D1");
        let report = pass(&mut conn, changed);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].starts_with("LF7 n°133 : les rencontres ont changé"));
    }

    #[test]
    fn an_http_error_is_journaled_not_raised() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut clock = || {
            DateTime::parse_from_rfc3339("2026-10-24T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        };
        let mut fetch = |_: &str, _: &str| -> Result<HttpResponse> {
            Ok(HttpResponse {
                status: 503,
                quota: Default::default(),
                body: String::new(),
            })
        };
        let report = run(&mut conn, &mut clock, &mut fetch, &mut |_| {}).unwrap();
        assert_eq!(report.failures.len(), 3);
        let errors: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM lotofoot_fetch WHERE error IS NOT NULL AND http_status = 503",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(errors, 3);
    }
}
