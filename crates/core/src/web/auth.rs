//! Connexion et protection des formulaires.
//!
//! Un seul secret, `serve_token` (fichier de configuration en 0600 ou variable `EDGE_SERVE_TOKEN`). La page
//! `/connexion` le demande une fois ; s'il est juste, le navigateur reçoit un cookie de session qui le contient, valable un
//! an. Le cookie est `HttpOnly` (illisible par un script) et `SameSite=Strict` : un autre site ne peut pas le faire
//! envoyer, donc pas faire écrire dans le journal à ta place. En plus, chaque `POST` doit venir du site lui-même
//! (en-tête `Origin`). Le site est en HTTP : le secret circule en clair, mais seulement sur le réseau de la maison.

use super::form::{Form, encode};
use super::view::Flash;

/// Nom du cookie de session.
const SESSION: &str = "edge_session";
/// Nom du cookie du message à afficher après une action.
const FLASH: &str = "edge_flash";
/// Durée de la session : un an, en secondes.
const SESSION_SECONDS: u32 = 365 * 24 * 3600;

/// Valeur du cookie `name` dans l'en-tête `Cookie` (`a=1; edge_session=…`).
pub fn cookie<'a>(header: Option<&'a str>, name: &str) -> Option<&'a str> {
    header?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Compare deux secrets en temps constant : la durée ne dépend pas de la position de la première différence, ce qui
/// empêcherait de deviner le secret caractère par caractère en chronométrant les réponses.
pub fn same_secret(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    // Une longueur différente suffit à refuser ; la longueur du secret n'est pas, elle, une information sensible.
    if a.len() != b.len() {
        return false;
    }
    // On accumule les différences de tous les octets (OU exclusif) au lieu de s'arrêter à la première.
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// La requête est-elle connectée ?
pub fn signed_in(cookies: Option<&str>, token: &str) -> bool {
    cookie(cookies, SESSION).is_some_and(|value| same_secret(value, token))
}

/// Le formulaire vient-il du site lui-même ? Le navigateur joint l'en-tête `Origin` (`http://hôte:port`) à tout envoi
/// de formulaire ; il doit désigner l'hôte que la requête vise. Sans `Origin`, on refuse.
pub fn same_origin(origin: Option<&str>, host: Option<&str>) -> bool {
    match (origin, host) {
        (Some(origin), Some(host)) => origin.strip_prefix("http://") == Some(host),
        _ => false,
    }
}

/// Le cookie qui ouvre la session.
pub fn session_cookie(token: &str) -> String {
    format!("{SESSION}={token}; Path=/; Max-Age={SESSION_SECONDS}; HttpOnly; SameSite=Strict")
}

/// Le cookie qui ferme la session.
pub fn end_session() -> String {
    format!("{SESSION}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict")
}

/// Le cookie qui transporte un message jusqu'à la page suivante (une minute au plus).
pub fn flash_cookie(flash: &Flash) -> String {
    let kind = if flash.kind == "t-neg" { "e" } else { "o" };
    format!(
        "{FLASH}={kind}.{}; Path=/; Max-Age=60; HttpOnly; SameSite=Strict",
        encode(&flash.text)
    )
}

/// Le cookie qui efface le message une fois affiché.
pub fn clear_flash() -> String {
    format!("{FLASH}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict")
}

/// Le message transporté par le cookie, s'il y en a un.
pub fn read_flash(cookies: Option<&str>) -> Option<Flash> {
    let value = cookie(cookies, FLASH)?;
    let (kind, encoded) = value.split_once('.')?;
    // Le texte est encodé comme un champ de formulaire : `Form` sait le relire.
    let text = Form::parse(&format!("m={encoded}")).get("m").to_string();
    if text.is_empty() {
        return None;
    }
    Some(if kind == "e" {
        Flash::error(text)
    } else {
        Flash::ok(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cookie_is_found_by_its_exact_name() {
        let header = Some("theme=sombre; edge_session=abc; edge_session_old=zzz");
        assert_eq!(cookie(header, "edge_session"), Some("abc"));
        assert_eq!(cookie(header, "edge"), None);
        assert_eq!(cookie(None, "edge_session"), None);
    }

    #[test]
    fn secrets_are_compared_whole() {
        assert!(same_secret("0123456789abcdef", "0123456789abcdef"));
        assert!(!same_secret("0123456789abcdeX", "0123456789abcdef"));
        assert!(!same_secret("0123456789abcde", "0123456789abcdef"));
        assert!(!same_secret("", "0123456789abcdef"));
        assert!(signed_in(Some("edge_session=s3cret"), "s3cret"));
        assert!(!signed_in(Some("edge_session=autre"), "s3cret"));
        assert!(!signed_in(None, "s3cret"));
    }

    #[test]
    fn a_form_must_come_from_the_site_itself() {
        assert!(same_origin(
            Some("http://cachyos.local:8737"),
            Some("cachyos.local:8737")
        ));
        assert!(!same_origin(
            Some("http://evil.example"),
            Some("cachyos.local:8737")
        ));
        assert!(!same_origin(
            Some("https://cachyos.local:8737"),
            Some("cachyos.local:8737")
        ));
        assert!(!same_origin(None, Some("cachyos.local:8737")));
        assert!(!same_origin(Some("null"), Some("cachyos.local:8737")));
    }

    #[test]
    fn the_session_cookie_is_http_only_and_strict() {
        let cookie = session_cookie("abc");
        assert!(cookie.starts_with("edge_session=abc;"));
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        assert!(end_session().contains("Max-Age=0"));
    }

    #[test]
    fn a_flash_message_survives_the_trip_through_its_cookie() {
        let sent = Flash::error("Pari n° 3 : déjà réglé ; rien à faire.");
        let set = flash_cookie(&sent);
        // Le navigateur renvoie seulement `nom=valeur`.
        let value = set.split(';').next().unwrap();
        assert!(!value.contains(' '));
        assert_eq!(read_flash(Some(value)), Some(sent));
        assert_eq!(read_flash(Some("edge_flash=")), None);
        assert_eq!(read_flash(None), None);
    }
}
