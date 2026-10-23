//! Application Web d'edge (`edge serve`) : routage et pages, sans réseau.
//!
//! Le serveur HTTP de la CLI traduit chaque requête en [`Request`], appelle [`handle`] et renvoie la [`Response`]
//! obtenue : tout ce qui s'affiche se teste ici, sans ouvrir de port. Les pages sont rendues côté serveur, sans script.

mod charts;
pub mod form;
mod pages;
mod view;

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::ingest::aliases::Aliases;
use crate::paths::Paths;
use crate::report::daily;
use crate::report::picks_image::{INTER_REGULAR, INTER_SEMIBOLD};
use form::Form;
use view::{Flash, Layout, render};

/// Type de contenu des pages.
pub const HTML: &str = "text/html; charset=utf-8";
/// Type de contenu des polices.
const TTF: &str = "font/ttf";
/// Type de contenu d'un message brut.
const TEXT: &str = "text/plain; charset=utf-8";
/// Taille maximale du corps d'une requête : un formulaire d'edge tient en quelques centaines d'octets.
pub const MAX_BODY: usize = 16 * 1024;

const MESSAGE_TEMPLATE: &str = include_str!("templates/message.html");

/// Ce dont l'application a besoin pour répondre. Construite une fois au démarrage de `edge serve`.
#[derive(Debug, Clone)]
pub struct App {
    /// Emplacements : base, rapports quotidiens, rapports d'analyse.
    pub paths: Paths,
    /// Configuration lue au démarrage (bankroll de départ, Kelly, bookmakers…).
    pub config: Config,
    /// Table d'alias (résultats des matchs).
    pub aliases: Aliases,
}

/// Méthode HTTP d'une requête.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Lire une page.
    Get,
    /// Comme `Get`, sans le corps de la réponse (le serveur HTTP s'en charge).
    Head,
    /// Envoyer un formulaire.
    Post,
    /// Toute autre méthode : refusée.
    Other,
}

/// Une requête, réduite à ce que l'application lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    /// Méthode.
    pub method: Method,
    /// Chemin et paramètres (`/performance?vue=clv`).
    pub target: &'a str,
    /// En-tête `Cookie`.
    pub cookies: Option<&'a str>,
    /// En-tête `Origin` (envoyé par le navigateur avec un formulaire).
    pub origin: Option<&'a str>,
    /// En-tête `Host`.
    pub host: Option<&'a str>,
    /// Corps (champs d'un formulaire).
    pub body: &'a [u8],
}

/// Une réponse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// Statut HTTP.
    pub status: u16,
    /// Type de contenu.
    pub content_type: &'static str,
    /// Corps.
    pub body: Vec<u8>,
    /// Adresse d'une redirection (`Location`).
    pub location: Option<String>,
    /// Cookies à déposer (`Set-Cookie`), un par en-tête.
    pub cookies: Vec<String>,
    /// Contenu immuable (police) : le navigateur peut le garder en cache. Les pages, elles, ne sont jamais gardées.
    pub cacheable: bool,
}

impl Response {
    fn html(status: u16, html: String) -> Response {
        Response {
            status,
            content_type: HTML,
            body: html.into_bytes(),
            location: None,
            cookies: Vec::new(),
            cacheable: false,
        }
    }

    /// Un message en texte brut (requête refusée avant d'atteindre l'application).
    pub fn text(status: u16, text: &str) -> Response {
        Response {
            content_type: TEXT,
            ..Response::html(status, text.to_string())
        }
    }

    fn font(bytes: &[u8]) -> Response {
        Response {
            status: 200,
            content_type: TTF,
            body: bytes.to_vec(),
            location: None,
            cookies: Vec::new(),
            cacheable: true,
        }
    }
}

/// Répond à une requête. Une erreur devient une page 500 qui la décrit (aucun secret n'y figure : les erreurs du cœur
/// ne portent ni clé API ni URL de webhook).
pub fn handle(app: &App, request: &Request<'_>, now: DateTime<Utc>) -> Response {
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target, ""));
    let result = match request.method {
        Method::Get | Method::Head => get(app, path, &Form::parse(query), now),
        Method::Post | Method::Other => Ok(message(
            405,
            "Méthode refusée",
            "Site en lecture seule : seules les requêtes GET et HEAD sont acceptées.",
        )),
    };
    result.unwrap_or_else(|error| {
        tracing::error!("{} : {error}", request.target);
        message(
            500,
            "Erreur",
            &format!("La page n'a pas pu être calculée : {error}"),
        )
    })
}

/// Les pages en lecture.
fn get(app: &App, path: &str, query: &Form, now: DateTime<Utc>) -> Result<Response> {
    let flash: Option<Flash> = None;
    let html = match path {
        "/" | "/index.html" => pages::today::page(app, now, flash)?,
        "/opportunites" => pages::opportunities::page(app, query, now, flash)?,
        "/paris" => pages::bets::page(app, now, flash)?,
        "/performance" => pages::performance::page(app, query, now, flash)?,
        "/paper" => pages::paper::page(app, now, flash)?,
        "/bonus" => pages::bonus::page(app, query, now, flash)?,
        "/lotofoot" => pages::lotofoot::page(app, now, flash)?,
        "/latence" => pages::latency::page(app, now, flash)?,
        "/rapports" | "/rapports/" => pages::reports::page(app, now, flash)?,
        "/police/inter-regular.ttf" => return Ok(Response::font(INTER_REGULAR)),
        "/police/inter-semibold.ttf" => return Ok(Response::font(INTER_SEMIBOLD)),
        _ => return archived(app, path),
    };
    Ok(Response::html(200, html))
}

/// `/rapports/<nom>` (rapport quotidien) ou `/rapports/analyse/<nom>` (rapport d'analyse). Le nom doit être
/// **exactement** celui d'un fichier listé : un chemin comme `../edge.db` ne correspond à aucun et donne une page
/// introuvable, aucun chemin n'est construit à partir de l'adresse.
fn archived(app: &App, path: &str) -> Result<Response> {
    let file: Option<PathBuf> = if let Some(name) = path.strip_prefix("/rapports/analyse/") {
        pages::reports::analysis_files(&app.paths.reports_dir)
            .into_iter()
            .find(|f| f.name == name)
            .map(|f| f.path)
    } else if let Some(name) = path.strip_prefix("/rapports/") {
        daily::list(&app.paths.daily_reports_dir)?
            .into_iter()
            .find(|report| report.name == name)
            .map(|report| app.paths.daily_reports_dir.join(report.name))
    } else {
        None
    };
    match file {
        Some(file) => {
            let body = std::fs::read(&file).map_err(Error::io(&file))?;
            Ok(Response {
                body,
                ..Response::html(200, String::new())
            })
        }
        None => Ok(not_found()),
    }
}

/// Page de message (introuvable, erreur).
#[derive(Debug, Clone, PartialEq, Serialize)]
struct MessagePage {
    #[serde(flatten)]
    layout: Layout,
    title: String,
    message: String,
}

fn not_found() -> Response {
    message(
        404,
        "Page introuvable",
        "Cette adresse ne correspond à aucune page du site.",
    )
}

/// Une page de message ; si même elle ne peut pas être rendue, un texte brut suffit.
fn message(status: u16, title: &str, text: &str) -> Response {
    let page = MessagePage {
        layout: Layout::new("", Utc::now(), None),
        title: title.to_string(),
        message: text.to_string(),
    };
    let html = render("message.html", MESSAGE_TEMPLATE, &page).unwrap_or_else(|_| text.to_string());
    Response::html(status, html)
}

#[cfg(test)]
mod testing;

#[cfg(test)]
mod tests {
    use super::*;
    use testing::{Sample, sample};

    fn get(sample: &Sample, target: &str) -> (u16, String) {
        let response = sample.get(target);
        let html = testing::unescape(&String::from_utf8(response.body).unwrap());
        (response.status, html)
    }

    #[test]
    fn only_the_listed_archive_files_can_be_read_and_unknown_paths_are_not_found() {
        let sample = sample();
        daily::save(
            &sample.app.paths.daily_reports_dir,
            "2026-10-20T12:10:00Z".parse().unwrap(),
            "<p>rapport</p>",
        )
        .unwrap();
        std::fs::create_dir_all(&sample.app.paths.reports_dir).unwrap();
        std::fs::write(
            sample.app.paths.reports_dir.join("market.html"),
            "<p>marché</p>",
        )
        .unwrap();
        std::fs::write(sample.app.paths.reports_dir.join("notes.txt"), "non").unwrap();

        assert_eq!(
            get(&sample, "/rapports/2026-10-20-121000Z.html"),
            (200, "<p>rapport</p>".to_string())
        );
        assert_eq!(
            get(&sample, "/rapports/analyse/market.html"),
            (200, "<p>marché</p>".to_string())
        );
        assert_eq!(get(&sample, "/rapports/2026-10-20-121000Z.html?x=1").0, 200);
        for path in [
            "/rapports/../edge.db",
            "/rapports/..%2Fedge.db",
            "/rapports/2026-10-21-121000Z.html",
            "/rapports/analyse/notes.txt",
            "/rapports/analyse/../edge.db",
            "/edge.db",
            "/inconnue",
        ] {
            let (status, body) = get(&sample, path);
            assert_eq!(status, 404, "{path}");
            assert!(body.contains("Page introuvable"));
        }
    }

    #[test]
    fn the_fonts_are_served_and_may_be_cached_but_pages_may_not() {
        let sample = sample();
        let font = sample.get("/police/inter-regular.ttf");
        assert_eq!((font.status, font.content_type), (200, "font/ttf"));
        assert!(font.cacheable);
        assert!(!sample.get("/").cacheable);
    }

    #[test]
    fn every_tab_answers_on_a_filled_journal_and_on_an_empty_database() {
        let filled = sample();
        let empty = testing::empty();
        for target in [
            "/",
            "/opportunites",
            "/opportunites?toutes",
            "/paris",
            "/performance",
            "/performance?vue=clv",
            "/performance?vue=calibration",
            "/paper",
            "/bonus",
            "/lotofoot",
            "/latence",
            "/rapports",
        ] {
            for sample in [&filled, &empty] {
                let (status, html) = get(sample, target);
                assert_eq!(status, 200, "{target} : {html}");
                assert!(html.contains("class=\"side\""), "{target} : barre latérale");
            }
        }
    }

    #[test]
    fn a_missing_database_gives_an_error_page_not_a_crash() {
        let mut sample = testing::empty();
        sample.app.paths.db_file = sample.app.paths.db_file.with_file_name("absente.db");

        let (status, html) = get(&sample, "/paper");

        assert_eq!(status, 500);
        assert!(html.contains("absente.db"), "{html}");
    }
}
