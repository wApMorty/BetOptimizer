//! Relevé des grilles du Loto Foot (`edge lotofoot fetch`, toutes les heures) : grilles LF7 et LF8 ouvertes avec leur
//! répartition FDJ, résultats et rapports des grilles closes. Deux sources, dans cet ordre (`docs/sources.md` §74) :
//!
//! 1. **la FDJ** (site des points de vente, [`super::fdj`]), source principale : la page des grilles ouvertes, puis la
//!    première page des résultats ;
//! 2. **resultats-fdj.fr** ([`super::resultats_fdj`]), repli lu **seulement si la FDJ échoue** pour une page : son accueil
//!    (grilles ouvertes), puis sa liste des résultats et la page de détail des seules grilles encore incomplètes en base.
//!
//! Une page FDJ échoue si le réseau tombe, si le statut HTTP n'est pas 200, si l'état de l'application manque (page de
//! défi anti-robot au lieu de la page) ou si elle est illisible. **Aucun contournement** : un refus de la FDJ (HTTP 403,
//! page de défi) fait basculer sur le repli, sans autre tentative ; le client ne suit même pas les redirections. Les
//! requêtes sont espacées de [`PAGE_COOLDOWN`], y compris d'une source à l'autre.
//!
//! **Page périmée** (`docs/sources.md` §73) : une page qui affiche encore comme « ouverte » une grille close depuis plus de
//! [`STALE_AFTER_HOURS`] heures n'est plus à jour. Du 5 au 12 novembre 2026, prono-match est resté figé sur le LF8 n°154 et
//! le relevé a enregistré la même page toutes les heures, sans rien signaler. Une telle lecture est journalisée avec une
//! erreur, remontée dans [`FetchReport::stale`], et la grille close n'est ni composée ni répartie. Le garde-fou vaut pour
//! les deux sources. Une page périmée est lue, elle n'est pas un échec : le repli n'est pas appelé.
//!
//! L'appel réseau, l'horloge et les pauses sont reçus en paramètres : en production le vrai client, `Utc::now` et
//! `thread::sleep`, dans les tests des pages synthétiques. Une page illisible n'arrête pas le passage : la lecture est
//! journalisée avec son erreur, et les autres pages sont relevées.

use std::time::Duration;

use chrono::{DateTime, Datelike, Utc};
use rusqlite::Connection;

use super::grid_page::OpenGrid;
use super::resultats_fdj::ResultLink;
use super::{GridId, fdj, resultats_fdj};
use crate::error::{Error, Result};
use crate::format::kickoff_label;
use crate::ingest::aliases::Aliases;
use crate::ingest::client::HttpResponse;
use crate::ingest::oddspapi::fetch::Pause;
use crate::ingest::snapshot::{Clock, Fetch};
use crate::store::lotofoot::{self as store, Page, Source};

/// Délai entre deux requêtes : les sites ne sont pas des API, on les ménage.
pub const PAGE_COOLDOWN: Duration = Duration::from_secs(5);

/// Heures après la clôture au-delà desquelles une grille encore affichée « ouverte » signale une page périmée.
/// Sur les quatorze grilles relevées avant la panne de prono-match, le site avait changé de grille dès le relevé suivant la
/// clôture (moins d'une heure) : trois heures laissent de la marge sans retarder l'alerte.
pub const STALE_AFTER_HOURS: i64 = 3;

/// `User-Agent` honnête : il nomme l'outil et son usage, sans donnée personnelle ni imitation d'un navigateur.
const USER_AGENT: &str = "edge/0.1 (usage personnel, une page par heure)";

/// Une grille close que la page d'une source affiche encore comme ouverte.
///
/// `Copy` : trois petites valeurs, que l'on passe sans se soucier de qui les possède.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleGrid {
    /// La grille.
    pub grid: GridId,
    /// Sa fin de validation, en UTC.
    pub closes_at: DateTime<Utc>,
    /// La source dont la page est périmée.
    pub source: Source,
}

impl StaleGrid {
    /// Raison écrite au journal des lectures (`lotofoot_fetch.error`) et montrée dans l'application.
    pub fn reason(&self) -> String {
        format!(
            "page périmée ({}) : {} encore affichée comme grille en cours, close depuis {}",
            self.source.label(),
            self.grid,
            kickoff_label(self.closes_at)
        )
    }
}

/// La grille affichée est-elle close depuis assez longtemps, à `at`, pour que la page soit dite périmée ?
fn stale_grid(source: Source, shown: &OpenGrid, at: DateTime<Utc>) -> Option<StaleGrid> {
    (at - shown.closes_at >= chrono::Duration::hours(STALE_AFTER_HOURS)).then(|| StaleGrid {
        grid: GridId::closing(shown.game, shown.number, shown.closes_at),
        closes_at: shown.closes_at,
        source,
    })
}

/// Client HTTP bloquant des sites du Loto Foot (aucune clé) : il sert la FDJ comme resultats-fdj.fr.
pub struct SiteClient {
    agent: ureq::Agent,
}

impl Default for SiteClient {
    fn default() -> Self {
        SiteClient::new()
    }
}

impl SiteClient {
    /// Crée le client. Le `User-Agent` nomme l'outil (`USER_AGENT`).
    pub fn new() -> SiteClient {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // Aucune redirection suivie : un défi anti-robot passe parfois par une redirection, et nous ne le suivons pas.
            // Le statut 3xx est journalisé comme une réponse inattendue.
            .max_redirects(0)
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(USER_AGENT)
            .build();
        SiteClient {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// `GET` d'une adresse complète ; le second argument (paramètres) est ignoré : c'est la forme de [`Fetch`].
    pub fn get(&self, url: &str, _params: &str) -> Result<HttpResponse> {
        let mut response = self
            .agent
            .get(url)
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

/// Une page qu'une des deux sources n'a pas pu servir, avec la raison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageIssue {
    /// La page.
    pub page: Page,
    /// La raison : celle de la FDJ si le repli a servi ; celle de la FDJ puis du repli si aucune n'a servi.
    pub reason: String,
}

/// Bilan d'un passage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchReport {
    /// Grilles ouvertes vues, avec leur état (une par formule en temps normal).
    pub grids: Vec<String>,
    /// Répartitions de rencontres écrites.
    pub splits: usize,
    /// Grilles dont les résultats ont été écrits pour la première fois.
    pub new_results: usize,
    /// Source qui a servi chaque page lue : « grilles ouvertes : FDJ », « résultats (page 1) : resultats-fdj.fr ».
    pub sources: Vec<String>,
    /// Pages que la FDJ n'a pas servies et que resultats-fdj.fr a lues à la place.
    pub fallbacks: Vec<PageIssue>,
    /// Pages que ni la FDJ ni resultats-fdj.fr n'ont pu servir.
    pub failures: Vec<PageIssue>,
    /// Pages lues mais périmées : une grille qu'elles affichent « ouverte » est close depuis longtemps.
    pub stale: Vec<StaleGrid>,
    /// Grilles entièrement annulées qui portent des tickets joués : mises remboursées au guichet, à saisir à la main.
    pub canceled: Vec<GridId>,
    /// Lues mais à signaler : composition d'une grille changée, résultats refusés (`docs/sources.md` §52), grille annulée.
    pub warnings: Vec<String>,
}

/// Lance un passage : grilles ouvertes, puis `result_pages` pages de résultats (une suffit au quotidien : la première page
/// de la FDJ porte toutes les grilles des deux dernières semaines, `docs/sources.md` §74).
pub fn run(
    conn: &mut Connection,
    aliases: &Aliases,
    now: Clock,
    fetch: Fetch,
    pause: Pause,
    result_pages: u32,
) -> Result<FetchReport> {
    let mut io = Io {
        now,
        fetch,
        pause,
        asked: false,
    };
    let mut report = FetchReport::default();
    read_open_grids(conn, aliases, &mut io, &mut report)?;
    for page_number in 1..=result_pages.max(1) {
        if !read_results(conn, aliases, &mut io, page_number, &mut report)? {
            break;
        }
    }
    Ok(report)
}

/// Pourquoi une page n'a pas été lue : le code HTTP (0 si le réseau a échoué avant toute réponse) et la raison.
struct Failure {
    status: u16,
    reason: String,
}

impl Failure {
    /// Le réseau a échoué (hôte introuvable, délai dépassé, TLS…).
    fn network(error: &Error) -> Failure {
        Failure {
            status: 0,
            reason: error.to_string(),
        }
    }

    /// Réponse HTTP autre que 200. Un refus (403, 429) est dit comme tel : c'est la signature d'un anti-robots.
    fn http(status: u16) -> Failure {
        let hint = if matches!(status, 403 | 429) {
            " (accès refusé, anti-robots ?)"
        } else {
            ""
        };
        Failure {
            status,
            reason: format!("réponse HTTP {status} inattendue{hint}"),
        }
    }

    /// La page est arrivée (HTTP 200) mais n'est pas lisible : état de l'application absent, format changé…
    fn unreadable(error: Error) -> Failure {
        Failure {
            status: 200,
            reason: error.to_string(),
        }
    }
}

/// Accès au réseau, à l'horloge et aux pauses d'un passage.
///
/// Toutes les requêtes passent par [`Io::get`] : c'est là que se garde l'écart de [`PAGE_COOLDOWN`] entre deux requêtes,
/// quelle que soit la source. Les trois champs sont des références vers des fonctions que l'appelant possède (`&mut dyn
/// FnMut`) : la durée de vie `'a` dit seulement que `Io` ne leur survit pas.
struct Io<'a> {
    now: Clock<'a>,
    fetch: Fetch<'a>,
    pause: Pause<'a>,
    /// Une requête a déjà été faite : la suivante attend d'abord.
    asked: bool,
}

impl Io<'_> {
    /// `GET` de `url` : l'instant de la requête et le corps de la page, ou la raison de l'échec.
    fn get(&mut self, url: &str) -> (DateTime<Utc>, std::result::Result<String, Failure>) {
        if self.asked {
            (self.pause)(PAGE_COOLDOWN);
        }
        self.asked = true;
        let at = (self.now)();
        let body = match (self.fetch)(url, "") {
            Err(error) => Err(Failure::network(&error)),
            Ok(response) if response.status != 200 => Err(Failure::http(response.status)),
            Ok(response) => Ok(response.body),
        };
        (at, body)
    }
}

/// Adresse d'une page de la FDJ.
fn fdj_url(path: &str) -> String {
    format!("{}{path}", fdj::BASE_URL)
}

/// Adresse d'une page de resultats-fdj.fr.
fn fallback_url(path: &str) -> String {
    format!("{}{path}", resultats_fdj::BASE_URL)
}

/// Journalise la lecture manquée d'une page.
fn journal_failure(
    conn: &Connection,
    at: DateTime<Utc>,
    source: Source,
    page: Page,
    failure: &Failure,
) -> Result<()> {
    store::record_fetch(
        conn,
        at,
        source,
        page,
        failure.status,
        Some(&failure.reason),
    )?;
    Ok(())
}

/// Le bilan retient que la FDJ n'a pas servi `page` et que le repli l'a lue.
fn note_fallback(page: Page, fdj_failure: &Failure, report: &mut FetchReport) {
    report.warnings.push(format!(
        "FDJ en échec ({page}), repli {} utilisé : {}",
        Source::ResultatsFdj.label(),
        fdj_failure.reason
    ));
    report.fallbacks.push(PageIssue {
        page,
        reason: fdj_failure.reason.clone(),
    });
}

/// Le bilan retient que ni la FDJ ni le repli n'ont servi `page`.
fn note_failure(
    page: Page,
    fdj_failure: &Failure,
    fallback_failure: &Failure,
    report: &mut FetchReport,
) {
    report.failures.push(PageIssue {
        page,
        reason: format!(
            "{} : {} ; {} : {}",
            Source::Fdj.label(),
            fdj_failure.reason,
            Source::ResultatsFdj.label(),
            fallback_failure.reason
        ),
    });
}

// ---------------------------------------------------------------------------------------------------------------------
// Grilles ouvertes
// ---------------------------------------------------------------------------------------------------------------------

/// Grilles ouvertes : la FDJ, puis resultats-fdj.fr si la FDJ échoue.
fn read_open_grids(
    conn: &mut Connection,
    aliases: &Aliases,
    io: &mut Io,
    report: &mut FetchReport,
) -> Result<()> {
    let (at, body) = io.get(&fdj_url(fdj::OPEN_PATH));
    let grids = body.and_then(|html| fdj::parse_open_page(&html, at).map_err(Failure::unreadable));
    let fdj_failure = match grids {
        Ok(grids) => return record_open(conn, aliases, at, Source::Fdj, grids, report),
        Err(failure) => failure,
    };
    journal_failure(conn, at, Source::Fdj, Page::Open, &fdj_failure)?;

    let (at, body) = io.get(&fallback_url(resultats_fdj::HOME_PATH));
    let grids = body
        .and_then(|html| resultats_fdj::parse_open_page(&html, at).map_err(Failure::unreadable));
    match grids {
        Ok(grids) => {
            note_fallback(Page::Open, &fdj_failure, report);
            record_open(conn, aliases, at, Source::ResultatsFdj, grids, report)
        }
        Err(fallback_failure) => {
            journal_failure(
                conn,
                at,
                Source::ResultatsFdj,
                Page::Open,
                &fallback_failure,
            )?;
            note_failure(Page::Open, &fdj_failure, &fallback_failure, report);
            Ok(())
        }
    }
}

/// Écrit les grilles lues sur la page des grilles ouvertes de `source`, et en rend compte.
///
/// Une grille dont la clôture est passée n'est ni composée ni répartie : sa répartition n'est plus une observation d'avant
/// clôture. Si elle est close depuis [`STALE_AFTER_HOURS`] heures ou plus, la page est périmée : la lecture est journalisée
/// avec cette erreur et la grille remontée dans [`FetchReport::stale`].
fn record_open(
    conn: &mut Connection,
    aliases: &Aliases,
    at: DateTime<Utc>,
    source: Source,
    grids: Vec<OpenGrid>,
    report: &mut FetchReport,
) -> Result<()> {
    // `partition` sépare la liste en deux, d'un seul parcours : les grilles closes d'un côté, les ouvertes de l'autre.
    let (closed, open): (Vec<OpenGrid>, Vec<OpenGrid>) =
        grids.into_iter().partition(|grid| grid.closes_at <= at);
    let mut stale = Vec::new();
    for grid in &closed {
        match stale_grid(source, grid, at) {
            Some(shown) => stale.push(shown),
            None => report.grids.push(format!(
                "{} n°{} (close, grille suivante pas encore affichée)",
                grid.game, grid.number
            )),
        }
    }
    let reason = (!stale.is_empty()).then(|| {
        stale
            .iter()
            .map(StaleGrid::reason)
            .collect::<Vec<_>>()
            .join(" ; ")
    });
    let fetch_id = store::record_fetch(conn, at, source, Page::Open, 200, reason.as_deref())?;
    report.stale.extend(stale);
    report
        .sources
        .push(format!("{} : {}", Page::Open, source.label()));
    // `&open` : on emprunte la liste, les grilles restent à nous (le bilan relit chacune après l'écriture).
    for grid in &open {
        record_one_open_grid(conn, aliases, fetch_id, grid, report)?;
    }
    Ok(())
}

/// Écrit une grille ouverte : sa composition, sa répartition. Les rapports passés ne viennent pas de la page des grilles
/// ouvertes : ils viennent des résultats.
fn record_one_open_grid(
    conn: &mut Connection,
    aliases: &Aliases,
    fetch_id: i64,
    grid: &OpenGrid,
    report: &mut FetchReport,
) -> Result<()> {
    let (game, number) = (grid.game, grid.number);
    let record = store::record_open_grid(conn, aliases, fetch_id, grid)?;
    report.splits += record.splits;
    let written = record.refused.is_none();
    // `extend` prend le refus (un `Option` se parcourt comme une liste de zéro ou un élément) : il n'est plus lu ensuite,
    // inutile de le copier.
    report.warnings.extend(record.refused);
    if written {
        if record.recomposed {
            report.warnings.push(format!(
                "{game} n°{number} : les rencontres ont changé, nouvelle composition enregistrée ; \
                 répartitions et tickets antérieurs écartés"
            ));
        }
        let state = if grid.split.is_some() {
            "répartition publiée"
        } else if grid.accepting {
            "ouverte, sans répartition"
        } else {
            "pas encore ouverte"
        };
        report.grids.push(format!("{game} n°{number} ({state})"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Résultats
// ---------------------------------------------------------------------------------------------------------------------

/// Page `page_number` des résultats : la FDJ, puis resultats-fdj.fr si la FDJ échoue. Rend `true` s'il reste des pages
/// après celle-ci.
fn read_results(
    conn: &mut Connection,
    aliases: &Aliases,
    io: &mut Io,
    page_number: u32,
    report: &mut FetchReport,
) -> Result<bool> {
    let (at, body) = io.get(&fdj_url(&fdj::results_path(page_number)));
    let results = body.and_then(|html| fdj::parse_results_page(&html).map_err(Failure::unreadable));
    let fdj_failure = match results {
        Ok(page) => {
            let fetch_id = store::record_fetch(conn, at, Source::Fdj, Page::Results, 200, None)?;
            report.sources.push(format!(
                "{} (page {page_number}) : {}",
                Page::Results,
                Source::Fdj.label()
            ));
            let record = store::record_results(conn, aliases, fetch_id, &page.results)?;
            report.new_results += record.new;
            report.warnings.extend(record.refused);
            report.warnings.extend(page.warnings);
            note_canceled(conn, &page.canceled_draws, report)?;
            return Ok(page.offset + page.limit < page.total);
        }
        Err(failure) => failure,
    };
    journal_failure(conn, at, Source::Fdj, Page::Results, &fdj_failure)?;
    read_results_fallback(conn, aliases, io, page_number, &fdj_failure, report)
}

/// Grilles entièrement annulées (`docs/sources.md` §74) : toujours un avertissement ; et, si des tickets y sont joués, une
/// entrée de [`FetchReport::canceled`] pour l'alerte (mises remboursées au guichet, à saisir à la main).
fn note_canceled(conn: &Connection, canceled: &[GridId], report: &mut FetchReport) -> Result<()> {
    for id in canceled {
        report.warnings.push(format!(
            "{id} : grille entièrement annulée, mises remboursées au guichet"
        ));
        if store::played_count(conn, *id)? > 0 {
            report.canceled.push(*id);
        }
    }
    Ok(())
}

/// Repli pour la page `page_number` des résultats : la liste de resultats-fdj.fr, puis la page de détail des grilles
/// LF7 et LF8 dont les résultats ou les rapports manquent encore en base. Rend `true` si la liste n'était pas vide.
fn read_results_fallback(
    conn: &mut Connection,
    aliases: &Aliases,
    io: &mut Io,
    page_number: u32,
    fdj_failure: &Failure,
    report: &mut FetchReport,
) -> Result<bool> {
    let (at, body) = io.get(&fallback_url(&resultats_fdj::results_path(page_number)));
    let links =
        body.and_then(|html| resultats_fdj::parse_results_list(&html).map_err(Failure::unreadable));
    let links = match links {
        Ok(links) => links,
        Err(fallback_failure) => {
            journal_failure(
                conn,
                at,
                Source::ResultatsFdj,
                Page::Results,
                &fallback_failure,
            )?;
            note_failure(Page::Results, fdj_failure, &fallback_failure, report);
            return Ok(false);
        }
    };
    store::record_fetch(conn, at, Source::ResultatsFdj, Page::Results, 200, None)?;
    note_fallback(Page::Results, fdj_failure, report);
    report.sources.push(format!(
        "{} (page {page_number}) : {}",
        Page::Results,
        Source::ResultatsFdj.label()
    ));
    for link in &links {
        read_detail(conn, aliases, io, link, report)?;
    }
    Ok(!links.is_empty())
}

/// Les résultats et les deux rapports de la grille du lien sont-ils déjà en base ? Sans jour de clôture lisible dans la
/// liste, on ne sait pas de quelle année est la grille : on ne devine pas, la page de détail sera lue.
fn is_complete(conn: &Connection, link: &ResultLink) -> Result<bool> {
    let Some(closed) = link.closed else {
        return Ok(false);
    };
    let id = GridId {
        game: link.game,
        year: closed.year(),
        number: link.number,
    };
    let paid_ranks = link.game.rank_shares().len();
    Ok(store::results(conn, id)?.is_some() && store::ranks(conn, id)?.len() >= paid_ranks)
}

/// Lit la page de détail d'une grille de la liste, si elle manque encore en base, et en écrit les résultats et rapports.
/// Un détail illisible n'est pas un échec du passage : un avertissement, la page sera relue au passage suivant.
fn read_detail(
    conn: &mut Connection,
    aliases: &Aliases,
    io: &mut Io,
    link: &ResultLink,
    report: &mut FetchReport,
) -> Result<()> {
    if is_complete(conn, link)? {
        return Ok(());
    }
    let label = format!("{} n°{}", link.game, link.number);
    let (at, body) = io.get(&fallback_url(link.path()));
    let result = body
        .and_then(|html| resultats_fdj::parse_result_detail(&html).map_err(Failure::unreadable));
    match result {
        Err(failure) => {
            journal_failure(conn, at, Source::ResultatsFdj, Page::Detail, &failure)?;
            report.warnings.push(format!(
                "détail {label} non lu ({}) : {}",
                Source::ResultatsFdj.label(),
                failure.reason
            ));
        }
        Ok(result) => {
            let fetch_id =
                store::record_fetch(conn, at, Source::ResultatsFdj, Page::Detail, 200, None)?;
            report
                .sources
                .push(format!("détail {label} : {}", Source::ResultatsFdj.label()));
            // Pas de résultat complet encore (rencontres en cours) : rien à écrire, la grille sera relue.
            if let Some(result) = result {
                let record =
                    store::record_results(conn, aliases, fetch_id, std::slice::from_ref(&result))?;
                report.new_results += record.new;
                report.warnings.extend(record.refused);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::{Value, json};

    use super::*;
    use crate::domain::{Euros, Outcome};
    use crate::lotofoot::Game;
    use crate::lotofoot::grid_page::{GridResult, MatchResult, RankLine};
    use crate::store::migrate::migrate;

    // Pages SYNTHÉTIQUES, écrites à la main sur le modèle de la structure des deux sites : aucune copie de leurs pages.

    const FDJ_OPEN: &str =
        "https://www.pointdevente.parionssport.fdj.fr/grilles/ouvertes/loto-foot";
    const FDJ_RESULTS: &str =
        "https://www.pointdevente.parionssport.fdj.fr/grilles/resultats/loto-foot";
    const FDJ_RESULTS_2: &str =
        "https://www.pointdevente.parionssport.fdj.fr/grilles/resultats/loto-foot?page=2";
    const RFDJ_HOME: &str = "https://resultats-fdj.fr/loto-foot/";
    const RFDJ_LIST: &str = "https://resultats-fdj.fr/loto-foot/resultats";
    const RFDJ_DETAIL_140: &str = "https://resultats-fdj.fr/loto-foot/grille/4957-loto-foot-7-n140";
    const RFDJ_DETAIL_141: &str = "https://resultats-fdj.fr/loto-foot/grille/4958-loto-foot-7-n141";
    const RFDJ_DETAIL_139: &str = "https://resultats-fdj.fr/loto-foot/grille/4956-loto-foot-7-n139";

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// Réponse d'un faux site : une page, ou un réseau en panne.
    enum Reply {
        Page(u16, String),
        Down,
    }

    /// Faux réseau : les réponses par adresse, et la liste des adresses demandées, dans l'ordre.
    struct Fake {
        replies: HashMap<String, Reply>,
        calls: Vec<String>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake {
                replies: HashMap::new(),
                calls: Vec::new(),
            }
        }

        fn page(mut self, url: &str, body: String) -> Fake {
            self.replies.insert(url.to_string(), Reply::Page(200, body));
            self
        }

        fn status(mut self, url: &str, status: u16) -> Fake {
            self.replies
                .insert(url.to_string(), Reply::Page(status, String::new()));
            self
        }

        fn down(mut self, url: &str) -> Fake {
            self.replies.insert(url.to_string(), Reply::Down);
            self
        }

        fn get(&mut self, url: &str) -> Result<HttpResponse> {
            self.calls.push(url.to_string());
            match self.replies.get(url) {
                Some(Reply::Page(status, body)) => Ok(HttpResponse {
                    status: *status,
                    quota: Default::default(),
                    body: body.clone(),
                }),
                Some(Reply::Down) | None => Err(Error::Http("hôte injoignable".to_string())),
            }
        }
    }

    /// Un passage à `when`, avec `pages` pages de résultats au plus ; rend le bilan et le nombre de pauses.
    fn pass(
        conn: &mut Connection,
        aliases: &Aliases,
        fake: &mut Fake,
        when: &str,
        pages: u32,
    ) -> (FetchReport, usize) {
        let mut pauses = 0;
        let report = run(
            conn,
            aliases,
            &mut || at(when),
            &mut |url, _| fake.get(url),
            &mut |_| pauses += 1,
            pages,
        )
        .unwrap();
        (report, pauses)
    }

    // --- FDJ : état Angular embarqué dans la page --------------------------------------------------------------------

    /// Échappement d'Angular, inverse de `fdj::decode_entities` : `&` d'abord.
    fn encode(json: &str) -> String {
        json.replace('&', "&a;")
            .replace('"', "&q;")
            .replace('\'', "&s;")
            .replace('<', "&l;")
            .replace('>', "&g;")
    }

    fn fdj_page(grids: Vec<Value>, total: u32) -> String {
        let state = json!({
            "gridsResolver": {
                "grids": grids,
                "pagination": {"limit": 20, "offset": 0, "total": total, "page": 1},
            },
        });
        format!(
            "<html><body><script id=\"serverApp-state\" type=\"application/json\">{}</script></body></html>",
            encode(&state.to_string())
        )
    }

    /// Une rencontre ouverte, répartition 50 / 30 / 20.
    fn open_event(i: usize) -> Value {
        json!({
            "opponents": [format!("Dom{i}"), format!("Ext{i}")],
            "result": null,
            "outcomes": [
                {"label": "1", "percentagePlayed": 50},
                {"label": "N", "percentagePlayed": 30},
                {"label": "2", "percentagePlayed": 20},
            ],
        })
    }

    /// Une rencontre jouée : `result` au libellé de la FDJ ; plus de répartition.
    fn played_event(i: usize, result: &str) -> Value {
        json!({
            "opponents": [format!("Dom{i}"), format!("Ext{i}")],
            "result": result,
            "outcomes": [{"label": "", "percentagePlayed": 0}],
        })
    }

    /// Une grille FDJ : formule (7 ou 8), numéro, clôture UTC, rencontres, rapports (gains en centimes).
    fn fdj_grid(
        formula: usize,
        number: u32,
        closes: &str,
        events: Vec<Value>,
        ranks: Value,
    ) -> Value {
        json!({
            "gridNumber": number,
            // Dernier chiffre de l'année de clôture × 1 000 + numéro : 2026 → 6 000.
            "externalDrawNumber": 6000 + number,
            "startValidationDate": "2026-11-01T23:10:00.000Z",
            "endValidationDate": closes,
            "drawCancelled": false,
            "jackpot": {"amount": 0, "type": 0, "gmg": ""},
            "product": {"deeplink": format!("lotofoot{formula}"), "numberEvents": events.len()},
            "events": events,
            "winranks": ranks,
        })
    }

    fn open_grid_json(formula: usize, number: u32, closes: &str) -> Value {
        fdj_grid(
            formula,
            number,
            closes,
            (1..=formula).map(open_event).collect(),
            json!([]),
        )
    }

    /// Une grille résultée : tous les signes « 1 » ; deux rangs payés.
    fn result_grid_json(formula: usize, number: u32, closes: &str) -> Value {
        let ranks = json!([
            {"rankNumberMarket": formula, "rankNumberGrid": 135, "rankAmount": 57_800},
            {"rankNumberMarket": formula - 1, "rankNumberGrid": 2393, "rankAmount": 3_980},
        ]);
        fdj_grid(
            formula,
            number,
            closes,
            (1..=formula)
                .map(|i| played_event(i, "firstOpponentWinner"))
                .collect(),
            ranks,
        )
    }

    const LF7_OPEN: &str = "2026-11-17T17:40:00.000Z";
    const LF8_OPEN: &str = "2026-11-15T13:55:00.000Z";

    /// Page des grilles ouvertes de la FDJ : le LF7 n°143 et le LF8 n°158.
    fn fdj_open() -> String {
        fdj_page(
            vec![
                open_grid_json(7, 143, LF7_OPEN),
                open_grid_json(8, 158, LF8_OPEN),
            ],
            2,
        )
    }

    /// Page des résultats de la FDJ : le LF7 n°139 et le LF8 n°155, résultés.
    fn fdj_results() -> String {
        fdj_page(
            vec![
                result_grid_json(7, 139, "2026-11-08T13:55:00.000Z"),
                result_grid_json(8, 155, "2026-11-07T13:55:00.000Z"),
            ],
            99,
        )
    }

    // --- resultats-fdj.fr --------------------------------------------------------------------------------------------

    fn rfdj_open_row(pos: usize, home: &str, away: &str) -> String {
        let cell = |label: &str, pct: &str| {
            format!(
                r#"<div class="lf-cellbox"><span data-l="{label}" class="lf-cell">{label}</span><span class="lf-pct">{pct}</span></div>"#
            )
        };
        format!(
            r#"<div class="lf-row" data-pos="{pos}" data-res="" data-prov=""><span class="lf-num">{pos}</span><span class="lf-team lf-home">{home}</span><div class="lf-cells">{}{}{}</div><span class="lf-team">{away}</span></div>"#,
            cell("1", "50 %"),
            cell("N", "30 %"),
            cell("2", "20 %"),
        )
    }

    /// Accueil de resultats-fdj.fr avec une grille ouverte (`formula` rencontres, `deadline` à l'heure de Paris).
    fn rfdj_home(formula: usize, number: u32, deadline: &str) -> String {
        let rows: String = (1..=formula)
            .map(|i| rfdj_open_row(i, &format!("Dom{i}"), &format!("Ext{i}")))
            .collect();
        format!(
            r#"<html><body><section class="lf-cards lf-cards-open"><article class="panel lf-grid"><header><h2><a href="/loto-foot/grille/855-loto-foot-{formula}-n{number}">Loto Foot {formula} n°{number}</a></h2></header>
<div class="lf-deadline">Fin de validation : <strong>{deadline}</strong></div><div class="lf-rows">{rows}</div></article></section></body></html>"#
        )
    }

    fn rfdj_list_row(formula: usize, number: u32, internal: u32, date: &str) -> String {
        format!(
            r#"<tr><td class="lr-grid" data-l="Grille"><span class="lr-prod">LF {formula}</span> <strong>n°{number}</strong></td><td class="lr-date" data-l="Date">{date}</td><td class="lr-go"><a href="/loto-foot/grille/{internal}-loto-foot-{formula}-n{number}">Détails</a></td></tr>"#
        )
    }

    fn rfdj_list(rows: &[String]) -> String {
        format!(
            r#"<html><body><table class="lf-list"><tbody>{}</tbody></table></body></html>"#,
            rows.concat()
        )
    }

    /// Détail d'une grille LF7 : signes par rencontre (`1`, `N`, `2`, `A`), clôture de Paris et deux rangs.
    fn rfdj_detail(number: u32, closes: &str, results: [&str; 7]) -> String {
        let rows: String = results
            .iter()
            .enumerate()
            .map(|(i, res)| {
                format!(
                    r#"<div class="lf-row" data-pos="{p}" data-res="{res}" data-prov=""><span class="lf-num">{p}</span><span class="lf-team lf-home">Dom{p}</span><span class="lf-team">Ext{p}</span></div>"#,
                    p = i + 1
                )
            })
            .collect();
        format!(
            r#"<html><body><article class="panel lf-grid"><header><h2>Loto Foot 7 n°{number}</h2><span>Fin de validation : {closes}</span></header>
<div class="lf-rows">{rows}</div><div class="draw-ranks-box"><table><tbody>
<tr><td><strong>7 sur 7</strong></td><td class="num">135</td><td class="lf-gain">578 €</td></tr>
<tr><td><strong>6 sur 7</strong></td><td class="num">2 393</td><td class="lf-gain">39,80 €</td></tr>
</tbody></table></div></article></body></html>"#
        )
    }

    // --- Base de départ ----------------------------------------------------------------------------------------------

    /// Écrit en base les résultats (tous « 1 ») et les deux rapports du LF7 n°`number`, clos le 2026-11-08 (heure de Paris).
    fn seed_results(conn: &mut Connection, number: u32) {
        let fetch_id = store::record_fetch(
            conn,
            at("2026-11-09T10:00:00Z"),
            Source::Fdj,
            Page::Results,
            200,
            None,
        )
        .unwrap();
        let rank = |good, winners, gain| RankLine {
            good,
            winners,
            unit_gain: Euros::new(gain),
        };
        let result = GridResult {
            game: Game::Lf7,
            number,
            date: chrono::NaiveDate::from_ymd_opt(2026, 11, 8),
            closes_at: None,
            matches: (1..=7)
                .map(|i| {
                    (
                        format!("Dom{i}"),
                        format!("Ext{i}"),
                        MatchResult::Sign(Outcome::Home),
                    )
                })
                .collect(),
            ranks: vec![rank(7, 135, 578.0), rank(6, 2393, 39.8)],
        };
        store::record_results(conn, &Aliases::default(), fetch_id, &[result]).unwrap();
    }

    // --- Les tests ---------------------------------------------------------------------------------------------------

    /// Passage nominal : la FDJ sert les deux pages, resultats-fdj.fr n'est pas appelé. Deux requêtes, une pause.
    #[test]
    fn a_nominal_pass_reads_two_pages_from_the_fdj_and_never_calls_the_fallback() {
        let mut conn = db();
        let mut fake = Fake::new()
            .page(FDJ_OPEN, fdj_open())
            .page(FDJ_RESULTS, fdj_results());

        let (report, pauses) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T09:00:00Z",
            1,
        );

        assert_eq!(fake.calls, [FDJ_OPEN, FDJ_RESULTS]);
        assert_eq!(pauses, 1);
        assert_eq!(
            report.grids,
            [
                "LF7 n°143 (répartition publiée)",
                "LF8 n°158 (répartition publiée)"
            ]
        );
        assert_eq!(report.splits, 15);
        assert_eq!(
            report.sources,
            ["grilles ouvertes : FDJ", "résultats (page 1) : FDJ"]
        );
        // Rattrapage : les deux grilles de la page des résultats, absentes de la base, sont écrites avec leurs rapports.
        assert_eq!(report.new_results, 2);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM lotofoot_result"), 15);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM lotofoot_rank"), 4);
        assert!(report.fallbacks.is_empty() && report.failures.is_empty());
        assert!(report.stale.is_empty() && report.warnings.is_empty());
        // Le journal des lectures dit quelle source a servi chaque page.
        let rows: Vec<(String, String, Option<String>)> = conn
            .prepare("SELECT source, page, error FROM lotofoot_fetch ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                ("fdj".to_string(), "open".to_string(), None),
                ("fdj".to_string(), "results".to_string(), None)
            ]
        );
        // Un second passage identique n'écrit plus de résultat ni de rapport : la première observation fait foi.
        let (again, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T10:00:00Z",
            1,
        );
        assert_eq!(again.new_results, 0);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM lotofoot_rank"), 4);
    }

    /// Un refus 403 de la FDJ : une seule tentative par page, aucune insistance, bascule sur le repli. Le détail n'est lu
    /// que pour la grille qui manque en base (LF7 n°140, une rencontre annulée) ; le LF7 n°139, complet, est sauté.
    #[test]
    fn a_403_from_the_fdj_falls_back_to_resultats_fdj_and_reads_only_incomplete_grids() {
        let mut conn = db();
        seed_results(&mut conn, 139);
        let list = rfdj_list(&[
            rfdj_list_row(7, 140, 4957, "Mar. 10 nov. 2026"),
            rfdj_list_row(7, 139, 4956, "Dim. 8 nov. 2026"),
        ]);
        let canceled = ["1", "1", "1", "1", "A", "1", "1"];
        let mut fake = Fake::new()
            .status(FDJ_OPEN, 403)
            .status(FDJ_RESULTS, 403)
            .page(
                RFDJ_HOME,
                rfdj_home(8, 158, "dimanche 15 novembre 2026 à 14h55"),
            )
            .page(RFDJ_LIST, list)
            .page(
                RFDJ_DETAIL_140,
                rfdj_detail(140, "mardi 10 novembre 2026 à 18h40", canceled),
            );

        let (report, pauses) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T09:00:00Z",
            1,
        );

        // Chaque page FDJ n'est demandée qu'une fois ; le détail du n°139 (déjà complet) n'est pas demandé.
        assert_eq!(
            fake.calls,
            [FDJ_OPEN, RFDJ_HOME, FDJ_RESULTS, RFDJ_LIST, RFDJ_DETAIL_140]
        );
        assert_eq!(pauses, 4);
        assert_eq!(report.grids, ["LF8 n°158 (répartition publiée)"]);
        assert_eq!(report.new_results, 1);
        assert!(report.failures.is_empty());
        assert_eq!(report.fallbacks.len(), 2);
        assert_eq!(report.fallbacks[0].page, Page::Open);
        assert!(report.fallbacks[0].reason.contains("HTTP 403"));
        assert!(report.fallbacks[0].reason.contains("anti-robots"));
        assert_eq!(report.fallbacks[1].page, Page::Results);
        assert!(
            report.warnings.iter().any(|w| w
                .starts_with("FDJ en échec (grilles ouvertes), repli resultats-fdj.fr utilisé")),
            "{:?}",
            report.warnings
        );
        assert_eq!(
            report.sources,
            [
                "grilles ouvertes : resultats-fdj.fr",
                "résultats (page 1) : resultats-fdj.fr",
                "détail LF7 n°140 : resultats-fdj.fr"
            ]
        );
        // La rencontre annulée du n°140 est écrite `C`.
        let code: String = conn
            .query_row(
                "SELECT outcome FROM lotofoot_result WHERE game = 'lf7' AND year = 2026 AND number = 140 AND position = 5",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(code, "C");
        // Journal : la FDJ en 403 (avec sa raison), puis le repli lu.
        let rows: Vec<(String, String, i64, bool)> = conn
            .prepare("SELECT source, page, http_status, error IS NOT NULL FROM lotofoot_fetch WHERE id > 1 ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let expected: Vec<(&str, &str, i64, bool)> = vec![
            ("fdj", "open", 403, true),
            ("resultats_fdj", "open", 200, false),
            ("fdj", "results", 403, true),
            ("resultats_fdj", "results", 200, false),
            ("resultats_fdj", "detail", 200, false),
        ];
        let rows: Vec<(&str, &str, i64, bool)> = rows
            .iter()
            .map(|(s, p, c, e)| (s.as_str(), p.as_str(), *c, *e))
            .collect();
        assert_eq!(rows, expected);
    }

    /// Une page de défi anti-robot (HTTP 200, mais sans état de l'application) est un échec de lecture, comme un 403 : le
    /// repli prend le relais pour cette page seulement ; la page des résultats de la FDJ est quand même tentée.
    #[test]
    fn a_challenge_page_without_state_makes_only_that_page_fall_back() {
        let mut conn = db();
        let mut fake = Fake::new()
            .page(
                FDJ_OPEN,
                "<html><body>Vérification en cours…</body></html>".to_string(),
            )
            .page(
                RFDJ_HOME,
                rfdj_home(8, 158, "dimanche 15 novembre 2026 à 14h55"),
            )
            .page(FDJ_RESULTS, fdj_results());

        let (report, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T09:00:00Z",
            1,
        );

        assert_eq!(fake.calls, [FDJ_OPEN, RFDJ_HOME, FDJ_RESULTS]);
        assert_eq!(report.fallbacks.len(), 1);
        assert!(
            report.fallbacks[0].reason.contains("serverApp-state"),
            "{:?}",
            report.fallbacks
        );
        assert_eq!(
            report.sources,
            [
                "grilles ouvertes : resultats-fdj.fr",
                "résultats (page 1) : FDJ"
            ]
        );
        assert_eq!(report.new_results, 2);
    }

    /// Ni la FDJ ni le repli : chaque page est journalisée en erreur pour les deux sources, le bilan porte les deux raisons.
    #[test]
    fn two_sources_down_are_journaled_and_reported_for_each_page() {
        let mut conn = db();
        let mut fake = Fake::new()
            .down(FDJ_OPEN)
            .status(RFDJ_HOME, 503)
            .status(FDJ_RESULTS, 503)
            .status(RFDJ_LIST, 500);

        let (report, pauses) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T09:00:00Z",
            1,
        );

        assert_eq!(fake.calls, [FDJ_OPEN, RFDJ_HOME, FDJ_RESULTS, RFDJ_LIST]);
        assert_eq!(pauses, 3);
        assert!(
            report.grids.is_empty() && report.sources.is_empty() && report.fallbacks.is_empty()
        );
        assert_eq!(report.failures.len(), 2);
        assert_eq!(report.failures[0].page, Page::Open);
        assert_eq!(
            report.failures[0].reason,
            "FDJ : échec de la requête HTTP : hôte injoignable ; resultats-fdj.fr : réponse HTTP 503 inattendue"
        );
        assert_eq!(report.failures[1].page, Page::Results);
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM lotofoot_fetch WHERE error IS NOT NULL"
            ),
            4
        );
        // Le réseau tombé est journalisé avec le code 0.
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM lotofoot_fetch WHERE source = 'fdj' AND page = 'open' AND http_status = 0"
            ),
            1
        );
    }

    /// Une grille close depuis moins de trois heures n'est pas une page périmée ; à trois heures pile, oui. Dans les deux cas
    /// ni sa composition ni sa répartition d'après clôture ne sont écrites ; la FDJ n'est pas jugée en échec (pas de repli).
    #[test]
    fn a_grid_still_listed_after_its_closing_is_stale_from_three_hours_and_never_archived() {
        let mut conn = db();
        let mut fake = Fake::new()
            .page(FDJ_OPEN, fdj_open())
            .page(FDJ_RESULTS, fdj_page(vec![], 0));
        // Avant la clôture du LF8 (13 h 55 UTC) : relevé normal, huit répartitions.
        let (before, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T10:00:00Z",
            1,
        );
        assert_eq!(before.splits, 15);
        assert!(before.stale.is_empty());

        // Une heure après : la FDJ n'a pas encore changé de grille, ce n'est pas une panne.
        let (soon, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T14:55:00Z",
            1,
        );
        assert!(soon.stale.is_empty());
        assert_eq!(
            soon.grids,
            [
                "LF8 n°158 (close, grille suivante pas encore affichée)",
                "LF7 n°143 (répartition publiée)"
            ]
        );
        assert_eq!(soon.splits, 7, "seul le LF7, encore ouvert, est réparti");

        // Trois heures pile : périmée. La page n'est pas un échec : pas de repli, pas de répartition de plus.
        let splits_before = count(&conn, "SELECT COUNT(*) FROM lotofoot_split");
        let (stale, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T16:55:00Z",
            1,
        );
        assert_eq!(stale.stale.len(), 1);
        assert_eq!(stale.stale[0].grid.code(), "lf8-2026-158");
        assert_eq!(stale.stale[0].source, Source::Fdj);
        assert!(stale.failures.is_empty() && stale.fallbacks.is_empty());
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM lotofoot_split"),
            splits_before + 7
        );
        assert!(!fake.calls.contains(&RFDJ_HOME.to_string()));
        let error: String = conn
            .query_row(
                "SELECT error FROM lotofoot_fetch WHERE page = 'open' ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            error,
            "page périmée (FDJ) : LF8 n°158 encore affichée comme grille en cours, close depuis dim. 15/11 14:55"
        );

        // Sur une base neuve, une page périmée n'écrit pas la grille close qu'elle affiche à tort.
        let mut fresh = db();
        let only_closed = fdj_page(vec![open_grid_json(8, 158, LF8_OPEN)], 1);
        let mut fake = Fake::new()
            .page(FDJ_OPEN, only_closed)
            .page(FDJ_RESULTS, fdj_page(vec![], 0));
        let (report, _) = pass(
            &mut fresh,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T20:00:00Z",
            1,
        );
        assert_eq!(report.stale.len(), 1);
        assert_eq!(count(&fresh, "SELECT COUNT(*) FROM lotofoot_grid"), 0);
    }

    /// Le garde-fou vaut aussi pour le repli : resultats-fdj.fr figé sur une grille close est signalé, avec son nom.
    #[test]
    fn the_fallback_page_is_checked_for_staleness_too() {
        let mut conn = db();
        let mut fake = Fake::new()
            .status(FDJ_OPEN, 403)
            .page(
                RFDJ_HOME,
                rfdj_home(8, 158, "dimanche 15 novembre 2026 à 14h55"),
            )
            .page(FDJ_RESULTS, fdj_page(vec![], 0));

        let (report, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T18:00:00Z",
            1,
        );

        assert_eq!(report.stale.len(), 1);
        assert_eq!(report.stale[0].source, Source::ResultatsFdj);
        assert!(
            report.stale[0]
                .reason()
                .starts_with("page périmée (resultats-fdj.fr) : LF8 n°158")
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM lotofoot_grid"), 0);
    }

    /// Une grille entièrement annulée : toujours un avertissement ; et, si un ticket y est joué, une alerte due (une seule
    /// fois : le journal des alertes, testé dans `alert`, fait le reste).
    #[test]
    fn a_fully_canceled_grid_warns_and_signals_only_when_a_ticket_is_played() {
        let mut conn = db();
        // Le LF7 n°141 est d'abord vu ouvert : la grille existe en base, un ticket peut y être joué.
        let open = fdj_page(vec![open_grid_json(7, 141, "2026-11-10T17:40:00.000Z")], 1);
        let mut fake = Fake::new()
            .page(FDJ_OPEN, open)
            .page(FDJ_RESULTS, fdj_page(vec![], 0));
        pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-09T10:00:00Z",
            1,
        );

        let mut canceled = result_grid_json(7, 141, "2026-11-10T17:40:00.000Z");
        canceled["drawCancelled"] = json!(true);
        let results = fdj_page(vec![canceled], 1);
        let mut fake = Fake::new()
            .page(FDJ_OPEN, fdj_page(vec![], 0))
            .page(FDJ_RESULTS, results);

        let (none_played, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-11T10:00:00Z",
            1,
        );
        assert!(none_played.canceled.is_empty());
        assert!(
            none_played.warnings.contains(
                &"LF7 n°141 : grille entièrement annulée, mises remboursées au guichet".to_string()
            ),
            "{:?}",
            none_played.warnings
        );
        // Une grille annulée n'a ni signes ni rapports : rien n'est écrit comme résultat.
        assert_eq!(none_played.new_results, 0);

        let ticket = crate::lotofoot::Ticket::parse("1111111").unwrap();
        store::record_play(
            &conn,
            &store::NewPlay {
                played_at: at("2026-11-09T11:00:00Z"),
                grid: GridId::closing(Game::Lf7, 141, at("2026-11-10T17:40:00Z")),
                ticket: &ticket,
                stake: Euros::new(1.0).unwrap(),
                reco_id: None,
            },
        )
        .unwrap();
        let (played, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-11T11:00:00Z",
            1,
        );
        assert_eq!(
            played.canceled,
            [GridId::closing(Game::Lf7, 141, at("2026-11-10T17:40:00Z"))]
        );
    }

    /// LF7 n°139 est en base sous les noms de prono-match (`Atlético Madrid`), la FDJ écrit `Atl. Madrid` : avec la table
    /// d'alias, les résultats sont acceptés ; sans elle, ils sont refusés avec un message qui nomme la rencontre.
    #[test]
    fn results_under_a_fdj_abbreviation_are_accepted_thanks_to_the_alias_table() {
        let seed = |conn: &mut Connection| {
            let mut grid = open_grid_json(7, 139, "2026-11-08T13:55:00.000Z");
            grid["events"][0]["opponents"] = json!(["Atlético Madrid", "Ext1"]);
            let fake_open = fdj_page(vec![grid], 1);
            let mut fake = Fake::new()
                .page(FDJ_OPEN, fake_open)
                .page(FDJ_RESULTS, fdj_page(vec![], 0));
            pass(
                conn,
                &Aliases::default(),
                &mut fake,
                "2026-11-07T10:00:00Z",
                1,
            );
        };
        let mut results = result_grid_json(7, 139, "2026-11-08T13:55:00.000Z");
        results["events"][0]["opponents"] = json!(["Atl. Madrid", "Ext1"]);
        let page = fdj_page(vec![results], 1);

        // Avec la table embarquée (section LOTOFOOT) : acceptée.
        let mut conn = db();
        seed(&mut conn);
        let mut fake = Fake::new()
            .page(FDJ_OPEN, fdj_page(vec![], 0))
            .page(FDJ_RESULTS, page.clone());
        let (accepted, _) = pass(
            &mut conn,
            &Aliases::bundled().unwrap(),
            &mut fake,
            "2026-11-09T10:00:00Z",
            1,
        );
        assert_eq!(accepted.new_results, 1);
        assert!(accepted.warnings.is_empty(), "{:?}", accepted.warnings);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM lotofoot_result"), 7);

        // Sans table : refusée, et le message dit quelle rencontre diffère.
        let mut conn = db();
        seed(&mut conn);
        let mut fake = Fake::new()
            .page(FDJ_OPEN, fdj_page(vec![], 0))
            .page(FDJ_RESULTS, page);
        let (refused, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-09T10:00:00Z",
            1,
        );
        assert_eq!(refused.new_results, 0);
        assert_eq!(refused.warnings.len(), 1);
        assert!(
            refused.warnings[0].contains(
                "rencontre 1 : « Atl. Madrid – Ext1 » lu, « Atlético Madrid – Ext1 » en base"
            ),
            "{:?}",
            refused.warnings
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM lotofoot_result"), 0);
    }

    /// `--result-pages` : la page 2 n'est lue que sur demande, et la pagination arrête la lecture quand la liste est finie.
    #[test]
    fn extra_result_pages_are_read_only_on_request_and_stop_at_the_end_of_the_list() {
        let mut conn = db();
        let mut fake = Fake::new()
            .page(FDJ_OPEN, fdj_page(vec![], 0))
            .page(FDJ_RESULTS, fdj_results())
            .page(FDJ_RESULTS_2, fdj_page(vec![], 99));

        let (default, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T09:00:00Z",
            1,
        );
        assert_eq!(fake.calls, [FDJ_OPEN, FDJ_RESULTS]);
        assert_eq!(default.sources.len(), 2);

        fake.calls.clear();
        let (more, pauses) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T10:00:00Z",
            2,
        );
        // `total` 99 et `limit` 20 : la page 2 existe, on la lit ; la limite demandée (2) arrête la lecture là.
        assert_eq!(fake.calls, [FDJ_OPEN, FDJ_RESULTS, FDJ_RESULTS_2]);
        assert_eq!(pauses, 2);
        assert!(
            more.sources
                .iter()
                .any(|s| s.starts_with("résultats (page 2)"))
        );

        // Sur une liste d'une seule page (`total` ≤ `limit`), on ne lit pas plus loin, même si l'on en demande trois.
        let mut conn = db();
        let mut fake = Fake::new()
            .page(FDJ_OPEN, fdj_page(vec![], 0))
            .page(FDJ_RESULTS, fdj_page(vec![], 12));
        pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T09:00:00Z",
            3,
        );
        assert_eq!(fake.calls, [FDJ_OPEN, FDJ_RESULTS]);
    }

    /// Un détail illisible du repli n'est pas un échec du passage : un avertissement, journalisé, relu au passage suivant.
    #[test]
    fn an_unreadable_detail_is_a_warning_and_the_other_details_are_still_read() {
        let mut conn = db();
        let list = rfdj_list(&[
            rfdj_list_row(7, 141, 4958, "Mar. 10 nov. 2026"),
            rfdj_list_row(7, 140, 4957, "Mar. 10 nov. 2026"),
        ]);
        let mut fake = Fake::new()
            .status(FDJ_OPEN, 500)
            .page(
                RFDJ_HOME,
                rfdj_home(8, 158, "dimanche 15 novembre 2026 à 14h55"),
            )
            .status(FDJ_RESULTS, 500)
            .page(RFDJ_LIST, list)
            .page(RFDJ_DETAIL_141, "<html>page cassée</html>".to_string())
            .page(
                RFDJ_DETAIL_140,
                rfdj_detail(140, "mardi 10 novembre 2026 à 18h40", ["1"; 7]),
            );

        let (report, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T09:00:00Z",
            1,
        );

        assert_eq!(
            fake.calls[3..],
            [RFDJ_LIST, RFDJ_DETAIL_141, RFDJ_DETAIL_140]
        );
        assert!(report.failures.is_empty());
        assert_eq!(report.new_results, 1);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.starts_with("détail LF7 n°141 non lu (resultats-fdj.fr)")),
            "{:?}",
            report.warnings
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM lotofoot_fetch WHERE page = 'detail' AND error IS NOT NULL"
            ),
            1
        );
        // La liste sans jour lisible ne se devine pas : le détail est lu. Un détail déjà complet n'est plus relu.
        let list = rfdj_list(&[rfdj_list_row(7, 139, 4956, "date illisible")]);
        let mut fake = Fake::new()
            .status(FDJ_OPEN, 500)
            .page(
                RFDJ_HOME,
                rfdj_home(8, 158, "dimanche 15 novembre 2026 à 14h55"),
            )
            .status(FDJ_RESULTS, 500)
            .page(RFDJ_LIST, list)
            .page(
                RFDJ_DETAIL_139,
                rfdj_detail(139, "dimanche 8 novembre 2026 à 14h55", ["1"; 7]),
            );
        let (undated, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T10:00:00Z",
            1,
        );
        assert_eq!(fake.calls[3..], [RFDJ_LIST, RFDJ_DETAIL_139]);
        assert_eq!(undated.new_results, 1);
    }

    /// Même grille, rencontre remplacée : la nouvelle composition est enregistrée et signalée.
    #[test]
    fn a_grid_whose_matches_change_is_reported() {
        let mut conn = db();
        let first = fdj_page(vec![open_grid_json(7, 143, LF7_OPEN)], 1);
        let mut fake = Fake::new()
            .page(FDJ_OPEN, first)
            .page(FDJ_RESULTS, fdj_page(vec![], 0));
        let (calm, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T09:00:00Z",
            1,
        );
        assert!(calm.warnings.is_empty());

        let mut changed = open_grid_json(7, 143, LF7_OPEN);
        changed["events"][0]["opponents"] = json!(["Autre", "Équipe"]);
        let mut fake = Fake::new()
            .page(FDJ_OPEN, fdj_page(vec![changed], 1))
            .page(FDJ_RESULTS, fdj_page(vec![], 0));
        let (report, _) = pass(
            &mut conn,
            &Aliases::default(),
            &mut fake,
            "2026-11-15T10:00:00Z",
            1,
        );

        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].starts_with("LF7 n°143 : les rencontres ont changé"));
    }
}
