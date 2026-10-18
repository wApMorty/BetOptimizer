//! Envoi d'un message Discord par webhook, avec un fichier joint (le rapport HTML).
//!
//! Source : documentation Discord, « Execute Webhook » (`POST /webhooks/{id}/{token}`) : `content` (2000 caractères
//! au plus), `allowed_mentions`, et pour les fichiers un `multipart/form-data` avec `payload_json` et `files[n]`.
//! Vérifiée le 2026-10-18 (voir `docs/sources.md`).
//!
//! L'URL du webhook porte le jeton : elle n'apparaît jamais dans un message d'erreur ni dans un journal.
//! La requête réseau est reçue en paramètre : en production c'est [`http_post`], dans les tests un faux.

use std::time::Duration;

use serde_json::json;

use super::message::MAX_CHARS;
use crate::config::WebhookUrl;
use crate::error::{Error, Result};

/// Fonction qui envoie un `POST` : reçoit l'URL, le type de contenu et le corps, renvoie le statut HTTP et le début de
/// la réponse.
pub type Post<'a> = &'a mut dyn FnMut(&str, &str, Vec<u8>) -> Result<(u16, String)>;

/// Fichier joint à un message.
#[derive(Debug, Clone, Copy)]
pub struct Attachment<'a> {
    /// Nom du fichier tel qu'il apparaîtra dans Discord.
    pub filename: &'a str,
    /// Type de contenu (`text/html; charset=utf-8`).
    pub content_type: &'a str,
    /// Contenu.
    pub bytes: &'a [u8],
}

/// Envoie `content` (tronqué à 2000 caractères) au webhook, avec un fichier joint éventuel.
pub fn send(
    webhook: &WebhookUrl,
    content: &str,
    attachment: Option<Attachment<'_>>,
    post: Post<'_>,
) -> Result<()> {
    // `allowed_mentions` vide : aucun @everyone ni @rôle ne peut partir, même si un nom d'équipe en contenait.
    let payload = json!({
        "content": truncate(content),
        "allowed_mentions": { "parse": [] },
    })
    .to_string();

    let (content_type, body) = match attachment {
        Some(file) => {
            let boundary = boundary_for(&payload, file.bytes);
            (
                format!("multipart/form-data; boundary={boundary}"),
                multipart_body(&boundary, &payload, file),
            )
        }
        None => ("application/json".to_string(), payload.into_bytes()),
    };

    let (status, answer) = post(webhook.expose(), &content_type, body)?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(Error::Http(format!(
            "Discord a refusé le message (HTTP {status}) : {}",
            answer.chars().take(200).collect::<String>()
        )))
    }
}

/// Limite le texte à la taille d'un message Discord, sans couper un caractère en deux.
fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(MAX_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// Choisit une frontière `multipart` absente du contenu envoyé (sinon le serveur couperait la pièce jointe en deux).
fn boundary_for(payload: &str, file: &[u8]) -> String {
    let mut counter = 0u32;
    loop {
        let candidate = format!("edge-boundary-{counter:04}-9f3c71a2");
        let marker = candidate.as_bytes();
        let clash = payload
            .as_bytes()
            .windows(marker.len())
            .any(|w| w == marker)
            || file.windows(marker.len()).any(|w| w == marker);
        if !clash {
            return candidate;
        }
        counter += 1;
    }
}

/// Corps `multipart/form-data` : `payload_json` puis `files[0]`.
fn multipart_body(boundary: &str, payload: &str, file: Attachment<'_>) -> Vec<u8> {
    // Le nom de fichier va dans un en-tête : on n'y laisse que des caractères sûrs (pas de guillemet ni de saut de ligne).
    let filename: String = file
        .filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();

    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{payload}\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"{filename}\"\r\nContent-Type: {}\r\n\r\n",
            file.content_type
        )
        .as_bytes(),
    );
    body.extend_from_slice(file.bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// `POST` réel avec `ureq`. L'URL (donc le jeton) est expurgée de toute erreur réseau.
pub fn http_post(url: &str, content_type: &str, body: Vec<u8>) -> Result<(u16, String)> {
    let config = ureq::Agent::config_builder()
        // Un statut 4xx/5xx n'est pas une erreur réseau : on veut lire la réponse qui explique.
        .http_status_as_error(false)
        // Une redirection enverrait le corps (et le jeton) ailleurs : on les refuse.
        .max_redirects(0)
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let redact = |text: String| redact(&text, url);
    let mut response = agent
        .post(url)
        .header("Content-Type", content_type)
        .send(&body[..])
        .map_err(|e| Error::Http(redact(e.to_string())))?;
    let status = response.status().as_u16();
    let answer = response
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::Http(redact(e.to_string())))?;
    Ok((status, answer))
}

/// Remplace l'URL du webhook, et son jeton seul, par `***`.
fn redact(text: &str, url: &str) -> String {
    let masked = text.replace(url, "***");
    match url.rsplit('/').next().filter(|token| !token.is_empty()) {
        Some(token) => masked.replace(token, "***"),
        None => masked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://discord.com/api/webhooks/123/jeton-secret";

    fn webhook() -> WebhookUrl {
        WebhookUrl::new(URL).unwrap()
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[test]
    fn a_plain_message_is_sent_as_json_with_mentions_disabled() {
        let mut seen = None;
        let mut post = |url: &str, content_type: &str, body: Vec<u8>| {
            seen = Some((url.to_string(), content_type.to_string(), body));
            Ok((204, String::new()))
        };

        send(&webhook(), "Bonjour @everyone", None, &mut post).unwrap();

        let (url, content_type, body) = seen.unwrap();
        assert_eq!(url, URL);
        assert_eq!(content_type, "application/json");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "Bonjour @everyone");
        assert_eq!(json["allowed_mentions"]["parse"], json!([]));
    }

    #[test]
    fn an_attachment_goes_in_a_multipart_body_with_the_payload_first() {
        let mut seen = None;
        let mut post = |_: &str, content_type: &str, body: Vec<u8>| {
            seen = Some((content_type.to_string(), body));
            Ok((200, String::new()))
        };
        let file = Attachment {
            filename: "rapport du 18/10.html",
            content_type: "text/html; charset=utf-8",
            bytes: "<p>é</p>".as_bytes(),
        };

        send(&webhook(), "Salut", Some(file), &mut post).unwrap();

        let (content_type, body) = seen.unwrap();
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap();
        let body = text(&body);
        let payload_at = body.find("name=\"payload_json\"").unwrap();
        let file_at = body.find("name=\"files[0]\"").unwrap();
        assert!(payload_at < file_at);
        assert!(body.starts_with(&format!("--{boundary}\r\n")));
        assert!(body.ends_with(&format!("\r\n--{boundary}--\r\n")));
        // Nom de fichier assaini : ni espace, ni barre, ni accent dans l'en-tête.
        assert!(
            body.contains("filename=\"rapport_du_18_10.html\""),
            "{body}"
        );
        assert!(body.contains("<p>é</p>"));
    }

    #[test]
    fn the_boundary_never_appears_inside_the_content() {
        let file = b"edge-boundary-0000-9f3c71a2 dans le fichier";

        let boundary = boundary_for("{}", file);

        assert_ne!(boundary, "edge-boundary-0000-9f3c71a2");
        assert!(
            !file
                .windows(boundary.len())
                .any(|w| w == boundary.as_bytes())
        );
    }

    #[test]
    fn a_long_message_is_cut_to_the_discord_limit_on_a_character_boundary() {
        let long = "é".repeat(MAX_CHARS + 50);

        let cut = truncate(&long);

        assert_eq!(cut.chars().count(), MAX_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("court"), "court");
    }

    #[test]
    fn a_refusal_is_reported_without_the_webhook_url() {
        let mut post = |_: &str, _: &str, _: Vec<u8>| Ok((429, "rate limited".to_string()));

        let error = send(&webhook(), "x", None, &mut post)
            .unwrap_err()
            .to_string();

        assert!(error.contains("429"));
        assert!(error.contains("rate limited"));
        assert!(!error.contains("jeton-secret"));
    }

    #[test]
    fn redaction_hides_both_the_whole_url_and_the_bare_token() {
        let message = redact(&format!("échec sur {URL} puis sur jeton-secret"), URL);

        assert!(!message.contains("jeton-secret"));
        assert!(!message.contains("discord.com"));
    }

    #[test]
    fn a_network_failure_is_passed_on_unchanged() {
        let mut post = |_: &str, _: &str, _: Vec<u8>| Err(Error::Http("réseau coupé".into()));

        let error = send(&webhook(), "x", None, &mut post).unwrap_err();

        assert!(error.to_string().contains("réseau coupé"));
    }
}
