//! `edge serve` : publie l'application Web sur le réseau local.
//!
//! `tiny_http` accepte les connexions sur ses propres threads et nous les remet une à une ; cette boucle les traite dans
//! l'ordre. Tout le contenu vient de `edge_core::web` : ici, on ne fait que traduire HTTP ↔ `web::Request`/`Response`.

use std::io::Read;

use chrono::Utc;
use edge_core::web::{self, App, MAX_BODY, Method};
use tiny_http::{Header, Request, Response, Server};

/// En-têtes ajoutés à toute réponse. La politique de sécurité n'autorise que les styles intégrés, les images `data:` et
/// les polices du site : une page ne peut charger aucune ressource externe ni exécuter de script, ses formulaires ne
/// peuvent viser que le site lui-même, et aucun autre site ne peut l'afficher dans un cadre.
const SECURITY_HEADERS: [(&str, &str); 3] = [
    (
        "Content-Security-Policy",
        "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src 'self'; form-action 'self'; \
         frame-ancestors 'none'; base-uri 'none'",
    ),
    ("X-Content-Type-Options", "nosniff"),
    // `same-origin` et non `no-referrer` : avec `no-referrer`, Chromium envoie `Origin: null` avec les formulaires, et le
    // contrôle d'origine du cœur les refuserait tous. Aucune adresse du site n'est transmise à un site extérieur.
    ("Referrer-Policy", "same-origin"),
];

/// Traite les requêtes jusqu'à l'arrêt du processus (systemd l'arrête par un signal).
pub fn run(server: &Server, app: &App) {
    for request in server.incoming_requests() {
        handle(request, app);
    }
}

/// Le nom de méthode de `tiny_http` traduit pour le cœur.
fn method(request: &Request) -> Method {
    match request.method() {
        tiny_http::Method::Get => Method::Get,
        tiny_http::Method::Head => Method::Head,
        tiny_http::Method::Post => Method::Post,
        _ => Method::Other,
    }
}

/// Valeur d'un en-tête de la requête.
fn header_value<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

/// Répond à une requête (`tiny_http` n'envoie pas le corps d'une réponse à `HEAD`).
fn handle(mut request: Request, app: &App) {
    // Le corps d'abord : le lire demande un emprunt exclusif (`&mut`) de la requête, qu'aucun en-tête emprunté ne doit
    // gêner. Seul un `POST` en a un : sans `Content-Length`, `tiny_http` lirait jusqu'à la fermeture de la connexion,
    // que le client n'envoie qu'après la réponse. `take` borne la lecture : au-delà de `MAX_BODY`, le formulaire est
    // refusé sans être lu en entier.
    let mut body = Vec::new();
    let read = if method(&request) == Method::Post {
        request
            .as_reader()
            .take(MAX_BODY as u64 + 1)
            .read_to_end(&mut body)
            .map(|_| ())
    } else {
        Ok(())
    };
    let page = if read.is_err() || body.len() > MAX_BODY {
        web::Response::text(413, "Formulaire trop long ou illisible.")
    } else {
        let incoming = web::Request {
            method: method(&request),
            target: request.url(),
            cookies: header_value(&request, "Cookie"),
            origin: header_value(&request, "Origin"),
            host: header_value(&request, "Host"),
            body: &body,
        };
        web::handle(app, &incoming, Utc::now())
    };
    tracing::info!(
        "{} {} → {} ({})",
        request.method(),
        // La partie `?…` peut porter les champs du calculateur : rien de secret, mais inutile dans le journal.
        request.url().split('?').next().unwrap_or_default(),
        page.status,
        request
            .remote_addr()
            .map_or("adresse inconnue".to_string(), ToString::to_string)
    );
    let mut response = Response::from_data(page.body)
        .with_status_code(page.status)
        .with_header(header("Content-Type", page.content_type));
    for (name, value) in SECURITY_HEADERS {
        response = response.with_header(header(name, value));
    }
    // Une police ne change jamais (elle est dans le binaire) ; une page change à chaque relevé : jamais de copie périmée.
    let cache = if page.cacheable {
        "max-age=31536000, immutable"
    } else {
        "no-store"
    };
    response = response.with_header(header("Cache-Control", cache));
    if let Some(location) = &page.location {
        response = response.with_header(header("Location", location));
    }
    for cookie in &page.cookies {
        response = response.with_header(header("Set-Cookie", cookie));
    }
    if page.status == 405 {
        response = response.with_header(header("Allow", "GET, HEAD, POST"));
    }
    // `respond` consomme la requête : elle ne peut recevoir qu'une réponse, le compilateur l'interdit ensuite.
    if let Err(error) = request.respond(response) {
        // Le client a fermé la connexion avant la fin : rien à faire de plus que le noter.
        tracing::warn!("réponse non envoyée : {error}");
    }
}

/// Construit un en-tête. Les noms et valeurs passés ici sont des constantes ou des valeurs produites par le cœur (adresse
/// de redirection, cookie), en ASCII sans retour à la ligne : la construction ne peut pas échouer.
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("en-tête : nom et valeur ASCII sans retour à la ligne")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;

    use edge_core::config::{Config, ServeToken};
    use edge_core::ingest::aliases::Aliases;
    use edge_core::paths::Paths;
    use edge_core::store;

    /// Envoie une requête brute et renvoie la réponse complète (le serveur ferme après `Connection: close`).
    fn ask(addr: std::net::SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        // En octets : une police n'est pas du texte UTF-8 (`read_to_string` échouerait).
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).unwrap();
        String::from_utf8_lossy(&answer).into_owned()
    }

    #[test]
    fn the_server_answers_with_security_headers_and_refuses_an_oversized_form() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        store::open_migrated(&paths.db_file).unwrap();
        let app = App {
            paths,
            config: Config::default(),
            aliases: Aliases::bundled().unwrap(),
            token: ServeToken::new("jeton-de-test-0123456789abcdef").unwrap(),
        };
        // Port 0 : le système choisit un port libre, que l'on relit ensuite.
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();

        // `thread::scope` : le thread du serveur peut emprunter `server` et `app`, car le scope garantit qu'il se termine
        // avant que ces variables ne soient détruites (sans `scope`, il faudrait les partager par `Arc`).
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..4 {
                    handle(server.recv().unwrap(), &app);
                }
            });

            // Sans session : renvoi vers la connexion, avec les en-têtes de sécurité.
            let home = ask(
                addr,
                "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            );
            assert!(home.starts_with("HTTP/1.1 303"), "{home}");
            assert!(home.contains("Location: /connexion"));
            assert!(home.contains("Content-Security-Policy: default-src 'none'"));
            assert!(home.contains("form-action 'self'"));
            assert!(home.contains("Cache-Control: no-store"));
            assert!(home.contains("Referrer-Policy: same-origin"));

            let font = ask(
                addr,
                "GET /police/inter-semibold.ttf HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            );
            assert!(font.contains("Content-Type: font/ttf"), "{}", &font[..200]);
            assert!(font.contains("Cache-Control: max-age=31536000, immutable"));

            let missing = ask(
                addr,
                "GET /rapports/../edge.db HTTP/1.1\r\nHost: x\r\nCookie: edge_session=jeton-de-test-0123456789abcdef\r\nConnection: close\r\n\r\n",
            );
            assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

            let big = "x".repeat(MAX_BODY + 10);
            let oversized = ask(
                addr,
                &format!(
                    "POST /paris/regler HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{big}",
                    big.len()
                ),
            );
            assert!(oversized.starts_with("HTTP/1.1 413"), "{oversized}");
        });
    }
}
