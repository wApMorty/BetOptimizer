//! `edge serve` : publie le site de rapports en HTTP, en lecture seule.
//!
//! `tiny_http` accepte les connexions sur ses propres threads et nous les remet une à une ; cette boucle les traite dans
//! l'ordre. Le contenu des pages vient de `edge_core::report::site` : ici, on ne fait que traduire HTTP ↔ `Page`.

use chrono::Utc;
use edge_core::report::site::{self, HTML, Page, Site};
use tiny_http::{Header, Method, Request, Response, Server};

/// En-têtes ajoutés à toute réponse. La politique de sécurité n'autorise que les styles intégrés et les images `data:` :
/// une page ne peut charger aucune ressource externe ni exécuter de script.
const SECURITY_HEADERS: [(&str, &str); 4] = [
    (
        "Content-Security-Policy",
        "default-src 'none'; style-src 'unsafe-inline'; img-src data:",
    ),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    // Les pages vivantes changent à chaque relevé : jamais de copie périmée dans un cache.
    ("Cache-Control", "no-store"),
];

/// Traite les requêtes jusqu'à l'arrêt du processus (systemd l'arrête par un signal).
pub fn run(server: &Server, site: &Site) {
    for request in server.incoming_requests() {
        handle(request, site);
    }
}

/// Répond à une requête : `GET` et `HEAD` seulement (`tiny_http` n'envoie pas le corps d'une réponse à `HEAD`).
fn handle(request: Request, site: &Site) {
    let page = match request.method() {
        Method::Get | Method::Head => site::respond(site, request.url(), Utc::now()),
        _ => Page {
            status: 405,
            body: "Site en lecture seule : seules les requêtes GET et HEAD sont acceptées."
                .as_bytes()
                .to_vec(),
        },
    };
    tracing::info!(
        "{} {} → {} ({})",
        request.method(),
        request.url(),
        page.status,
        request
            .remote_addr()
            .map_or("adresse inconnue".to_string(), ToString::to_string)
    );
    let mut response = Response::from_data(page.body)
        .with_status_code(page.status)
        .with_header(header("Content-Type", HTML));
    for (name, value) in SECURITY_HEADERS {
        response = response.with_header(header(name, value));
    }
    if page.status == 405 {
        response = response.with_header(header("Allow", "GET, HEAD"));
    }
    // `respond` consomme la requête : elle ne peut recevoir qu'une réponse, le compilateur l'interdit ensuite.
    if let Err(error) = request.respond(response) {
        // Le client a fermé la connexion avant la fin : rien à faire de plus que le noter.
        tracing::warn!("réponse non envoyée : {error}");
    }
}

/// Construit un en-tête. Les noms et valeurs passés ici sont des constantes ASCII sans retour à la ligne : la
/// construction ne peut pas échouer.
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("en-tête constant : nom et valeur ASCII sans retour à la ligne")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    use edge_core::ingest::aliases::Aliases;
    use edge_core::market::margin::MarginMethod;
    use edge_core::store;

    /// Envoie une requête brute et renvoie la réponse complète (le serveur ferme après `Connection: close`).
    fn ask(addr: std::net::SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    }

    #[test]
    fn the_server_answers_get_with_security_headers_and_refuses_writes() {
        let dir = tempfile::tempdir().unwrap();
        let db_file = dir.path().join("edge.db");
        store::open_migrated(&db_file).unwrap();
        let site = Site {
            db_file,
            daily_dir: dir.path().join("daily"),
            aliases: Aliases::bundled().unwrap(),
            method: MarginMethod::Power,
        };
        // Port 0 : le système choisit un port libre, que l'on relit ensuite.
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();

        // `thread::scope` : le thread du serveur peut emprunter `server` et `site`, car le scope garantit qu'il se termine
        // avant que ces variables ne soient détruites (sans `scope`, il faudrait les partager par `Arc`).
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..3 {
                    handle(server.recv().unwrap(), &site);
                }
            });

            let home = ask(
                addr,
                "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            );
            assert!(home.starts_with("HTTP/1.1 200"), "{home}");
            assert!(home.contains("Content-Security-Policy: default-src 'none'"));
            assert!(home.contains("Aucun rapport archivé"));

            let missing = ask(
                addr,
                "GET /rapports/../edge.db HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            );
            assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

            let post = ask(
                addr,
                "POST /paper HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            assert!(post.starts_with("HTTP/1.1 405"), "{post}");
            assert!(post.contains("Allow: GET, HEAD"));
        });
    }
}
