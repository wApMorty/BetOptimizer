//! Application Web d'edge (`edge serve`) : routage et pages, sans réseau.
//!
//! Le serveur HTTP de la CLI traduit chaque requête en [`Request`], appelle [`handle`] et renvoie la [`Response`]
//! obtenue : tout ce qui s'affiche se teste ici, sans ouvrir de port. Les pages sont rendues côté serveur, sans script.
//! Tout est derrière la connexion ([`auth`]), sauf la page de connexion elle-même et les polices.

pub mod auth;
mod charts;
pub mod form;
mod pages;
mod view;

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::{Config, ServeToken};
use crate::error::{Error, Result};
use crate::ingest::aliases::Aliases;
use crate::paths::Paths;
use crate::report::daily;
use crate::report::picks_image::{INTER_REGULAR, INTER_SEMIBOLD};
use form::Form;
use view::{Layout, render};

/// Type de contenu des pages.
pub const HTML: &str = "text/html; charset=utf-8";
/// Type de contenu des polices.
const TTF: &str = "font/ttf";
/// Type de contenu d'un message brut.
const TEXT: &str = "text/plain; charset=utf-8";
/// Taille maximale du corps d'une requête : un formulaire d'edge tient en quelques centaines d'octets.
pub const MAX_BODY: usize = 16 * 1024;

const MESSAGE_TEMPLATE: &str = include_str!("templates/message.html");
const LOGIN_TEMPLATE: &str = include_str!("templates/login.html");

/// Ce dont l'application a besoin pour répondre. Construite une fois au démarrage de `edge serve`.
#[derive(Debug, Clone)]
pub struct App {
    /// Emplacements : base, rapports quotidiens, rapports d'analyse.
    pub paths: Paths,
    /// Configuration lue au démarrage (bankroll de départ, Kelly, bookmakers…).
    pub config: Config,
    /// Table d'alias (résultats des matchs).
    pub aliases: Aliases,
    /// Secret de connexion.
    pub token: ServeToken,
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

    /// Redirection après un formulaire (`303 See Other`) : le navigateur recharge `location` en `GET`, si bien
    /// qu'actualiser la page ne renvoie pas le formulaire.
    fn redirect(location: &str, cookies: Vec<String>) -> Response {
        Response {
            location: Some(location.to_string()),
            cookies,
            ..Response::html(303, String::new())
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
    let signed_in = auth::signed_in(request.cookies, app.token.expose());
    let result = match request.method {
        Method::Get | Method::Head => match path {
            "/police/inter-regular.ttf" => Ok(Response::font(INTER_REGULAR)),
            "/police/inter-semibold.ttf" => Ok(Response::font(INTER_SEMIBOLD)),
            "/connexion" if signed_in => Ok(Response::redirect("/", Vec::new())),
            "/connexion" => login(200, None),
            // Toute autre adresse, même inconnue, renvoie à la connexion : rien n'est révélé avant.
            _ if !signed_in => Ok(Response::redirect("/connexion", Vec::new())),
            _ => get(app, path, &Form::parse(query), request.cookies, now),
        },
        // Un formulaire envoyé depuis un autre site est refusé avant même de regarder la session.
        Method::Post if !auth::same_origin(request.origin, request.host) => Ok(message(
            403,
            "Formulaire refusé",
            "Ce formulaire ne vient pas de l'application elle-même.",
        )),
        Method::Post => {
            // Un champ de formulaire n'est que du texte : des octets invalides deviennent `�`, et la validation du
            // champ concerné les refusera.
            let form = Form::parse(&String::from_utf8_lossy(request.body));
            post(app, path, &form, signed_in, now)
        }
        Method::Other => Ok(message(
            405,
            "Méthode refusée",
            "Seules les requêtes GET, HEAD et POST sont acceptées.",
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

/// Les pages en lecture. Un message laissé par l'action précédente est affiché une fois, puis effacé.
fn get(
    app: &App,
    path: &str,
    query: &Form,
    cookies: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Response> {
    let flash = auth::read_flash(cookies);
    let clear = flash.is_some();
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
        _ => return archived(app, path),
    };
    let mut response = Response::html(200, html);
    if clear {
        response.cookies.push(auth::clear_flash());
    }
    Ok(response)
}

/// Les formulaires. Seule la connexion est acceptée sans session.
fn post(
    app: &App,
    path: &str,
    form: &Form,
    signed_in: bool,
    _now: DateTime<Utc>,
) -> Result<Response> {
    match path {
        "/connexion" => {
            if auth::same_secret(form.get("secret"), app.token.expose()) {
                tracing::info!("connexion réussie");
                let cookie = auth::session_cookie(app.token.expose());
                Ok(Response::redirect("/", vec![cookie]))
            } else {
                tracing::warn!("connexion refusée : secret erroné");
                login(401, Some("Secret erroné."))
            }
        }
        "/deconnexion" => Ok(Response::redirect("/connexion", vec![auth::end_session()])),
        _ if !signed_in => Ok(message(
            403,
            "Connexion requise",
            "La session a expiré : reconnecte-toi, puis recommence.",
        )),
        _ => Ok(not_found()),
    }
}

/// Page de connexion.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct LoginPage {
    #[serde(flatten)]
    layout: Layout,
    error: Option<&'static str>,
}

fn login(status: u16, error: Option<&'static str>) -> Result<Response> {
    let page = LoginPage {
        layout: Layout {
            nav: false,
            ..Layout::new("", Utc::now(), None)
        },
        error,
    };
    Ok(Response::html(
        status,
        render("login.html", LOGIN_TEMPLATE, &page)?,
    ))
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
    use view::Flash;

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
    fn without_a_session_every_page_leads_to_the_login_page_but_the_fonts_do_not() {
        let sample = testing::empty();
        for target in ["/", "/paris", "/rapports/analyse/x.html", "/inconnue"] {
            let response = sample.send(&testing::request(Method::Get, target, b""));
            assert_eq!(response.status, 303, "{target}");
            assert_eq!(response.location.as_deref(), Some("/connexion"));
        }
        let login = sample.send(&testing::request(Method::Get, "/connexion", b""));
        assert_eq!(login.status, 200);
        assert!(
            !String::from_utf8(login.body)
                .unwrap()
                .contains("class=\"side\"")
        );
        let font = sample.send(&testing::request(
            Method::Get,
            "/police/inter-regular.ttf",
            b"",
        ));
        assert_eq!(font.status, 200);
        // Déjà connecté : la page de connexion renvoie à l'accueil.
        assert_eq!(sample.get("/connexion").status, 303);
    }

    #[test]
    fn the_right_secret_opens_a_session_and_a_wrong_one_does_not() {
        let sample = testing::empty();
        let wrong = sample.send(&testing::request(Method::Post, "/connexion", b"secret=non"));
        assert_eq!(wrong.status, 401);
        assert!(wrong.cookies.is_empty());

        let body = format!("secret={}", testing::TOKEN);
        let right = sample.send(&testing::request(
            Method::Post,
            "/connexion",
            body.as_bytes(),
        ));
        assert_eq!(right.status, 303);
        assert_eq!(right.location.as_deref(), Some("/"));
        assert!(right.cookies[0].starts_with(testing::SESSION));

        let out = sample.post("/deconnexion", "");
        assert_eq!(out.location.as_deref(), Some("/connexion"));
        assert!(out.cookies[0].contains("Max-Age=0"));
    }

    #[test]
    fn a_form_from_another_site_or_without_a_session_is_refused() {
        let sample = testing::empty();
        let foreign = Request {
            cookies: Some(testing::SESSION),
            origin: Some("http://evil.example"),
            ..testing::request(Method::Post, "/paris/regler", b"")
        };
        assert_eq!(sample.send(&foreign).status, 403);
        let no_origin = Request {
            origin: None,
            ..foreign
        };
        assert_eq!(sample.send(&no_origin).status, 403);
        let anonymous = testing::request(Method::Post, "/paris/regler", b"");
        assert_eq!(sample.send(&anonymous).status, 403);
        let other = testing::request(Method::Other, "/", b"");
        assert_eq!(sample.send(&other).status, 405);
    }

    #[test]
    fn a_flash_message_is_shown_once_then_cleared() {
        let sample = testing::empty();
        let cookie = auth::flash_cookie(&Flash::ok("Pari n° 3 enregistré."));
        let value = cookie.split(';').next().unwrap();
        let cookies = format!("{}; {value}", testing::SESSION);
        let request = Request {
            cookies: Some(&cookies),
            ..testing::request(Method::Get, "/paris", b"")
        };

        let response = sample.send(&request);

        let html = testing::unescape(&String::from_utf8(response.body).unwrap());
        assert!(html.contains("Pari n° 3 enregistré."), "{html}");
        assert!(response.cookies[0].starts_with("edge_flash=;"));
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
