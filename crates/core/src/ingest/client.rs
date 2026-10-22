//! Client HTTP bloquant pour The Odds API (`ureq`, sans `async`).
//!
//! Seul module à connaître la clé en clair au moment de l'appel. Toute erreur renvoyée est expurgée :
//! `ureq` peut recopier l'URL (donc la clé) dans ses messages.

use std::time::Duration;

use ureq::http::HeaderMap;

use super::odds_api::request_url;
use crate::config::ApiKey;
use crate::domain::Quota;
use crate::error::{Error, Result};

/// Réponse HTTP réduite à ce dont le programme a besoin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Code de statut HTTP (200, 401, 422…).
    pub status: u16,
    /// Compteurs de crédits lus dans les en-têtes.
    pub quota: Quota,
    /// Corps de la réponse, tel que reçu.
    pub body: String,
}

/// Client de l'API. Il garde la clé et réutilise la même connexion d'un appel à l'autre.
pub struct OddsApiClient {
    agent: ureq::Agent,
    key: ApiKey,
}

impl OddsApiClient {
    /// Crée un client avec la clé donnée.
    pub fn new(key: ApiKey) -> OddsApiClient {
        let config = ureq::Agent::config_builder()
            // Un statut 4xx/5xx n'est pas une erreur réseau : on veut lire le corps JSON qui explique.
            .http_status_as_error(false)
            // Une redirection pourrait renvoyer la clé (dans l'URL) vers un autre site : on les refuse.
            .max_redirects(0)
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        OddsApiClient {
            agent: ureq::Agent::new_with_config(config),
            key,
        }
    }

    /// Envoie un `GET` sur `path` avec les paramètres `params` (sans clé : elle est ajoutée ici).
    pub fn get(&self, path: &str, params: &str) -> Result<HttpResponse> {
        let url = request_url(path, params, &self.key);
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| self.http_error(&e))?;
        let status = response.status().as_u16();
        let quota = quota_from_headers(response.headers());
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| self.http_error(&e))?;
        Ok(HttpResponse {
            status,
            quota,
            body,
        })
    }

    /// Convertit une erreur `ureq` en [`Error::Http`], clé masquée.
    fn http_error(&self, error: &ureq::Error) -> Error {
        Error::Http(redact(&error.to_string(), &self.key))
    }
}

/// Remplace toute occurrence de la clé par `***`.
pub(crate) fn redact(text: &str, key: &ApiKey) -> String {
    // Garde-fou : remplacer une chaîne vide insérerait `***` entre chaque caractère.
    if key.expose().is_empty() {
        return text.to_string();
    }
    text.replace(key.expose(), "***")
}

fn quota_from_headers(headers: &HeaderMap) -> Quota {
    // `to_str()` échoue si la valeur n'est pas de l'ASCII visible : `.ok()` la traite alors comme absente.
    let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    Quota::from_header_values(
        text("x-requests-last"),
        text("x-requests-used"),
        text("x-requests-remaining"),
    )
}

#[cfg(test)]
mod tests {
    use ureq::http::HeaderValue;

    use super::*;

    #[test]
    fn quota_is_read_from_the_three_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-requests-last", HeaderValue::from_static("1"));
        headers.insert("x-requests-used", HeaderValue::from_static("8"));
        headers.insert("x-requests-remaining", HeaderValue::from_static("492"));

        let quota = quota_from_headers(&headers);

        assert_eq!(quota.cost, Some(1));
        assert_eq!(quota.used, Some(8));
        assert_eq!(quota.remaining, Some(492));
    }

    #[test]
    fn missing_headers_give_an_empty_quota() {
        // Cas réel d'une réponse 401 : aucun en-tête de quota.
        assert_eq!(quota_from_headers(&HeaderMap::new()), Quota::default());
    }

    #[test]
    fn the_key_is_masked_in_error_messages() {
        let client = OddsApiClient::new(ApiKey::new("CLE-SECRETE"));
        // `BadUri` recopie l'URI fautive, clé comprise : c'est le cas réel que l'on redoute.
        let error = ureq::Error::BadUri("https://x/?apiKey=CLE-SECRETE&a=b".to_string());

        let message = client.http_error(&error).to_string();

        assert!(!message.contains("CLE-SECRETE"));
        assert!(message.contains("apiKey=***"));
    }

    #[test]
    fn redacting_with_an_empty_key_leaves_the_text_alone() {
        assert_eq!(redact("abc", &ApiKey::new("")), "abc");
    }
}
