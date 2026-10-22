//! OddsPapi : historique gratuit des cotes, une entrée par changement de prix (phase 13, `docs/sources.md` §34).
//!
//! Sert au test prospectif de la latence des opérateurs français (`eval::latency`). Trois fichiers : le client HTTP
//! (ici), la lecture des réponses ([`format`]) et le téléchargement hebdomadaire ([`fetch`]).

pub mod fetch;
pub mod format;

use std::time::Duration;

use super::client::{HttpResponse, redact};
use crate::config::ApiKey;
use crate::domain::Quota;
use crate::error::{Error, Result};

/// Racine de l'API (version 4, celle de l'offre gratuite).
pub const BASE_URL: &str = "https://api.oddspapi.io/v4/";

/// Bookmakers demandés à l'historique : la référence et les deux opérateurs français couverts (Betclic est absent
/// d'OddsPapi). Trois au plus par appel (documentation de `historical-odds`).
pub const BOOKMAKERS: &str = "pinnacle,winamax.fr,unibet.fr";

/// Les six championnats du projet : identifiant OddsPapi (`GET /v4/tournaments?sportId=10`, lu le 2026-10-21) et code
/// football-data, qui désigne la section de la table d'alias.
pub const LEAGUES: [(u32, &str); 6] = [
    (17, "E0"),
    (8, "SP1"),
    (35, "D1"),
    (23, "I1"),
    (34, "F1"),
    (36, "SC0"),
];

/// Délai minimal entre deux appels à l'historique (« endpoint cooldown : 5000 ms »), avec une marge.
pub const HISTORY_COOLDOWN: Duration = Duration::from_millis(5300);

/// Délai minimal entre deux appels aux autres points d'accès (« cooldown : 1000 ms »), avec une marge.
pub const OTHER_COOLDOWN: Duration = Duration::from_millis(1100);

/// Client de l'API. Même précautions que celui de The Odds API : la clé est dans l'URL, donc toute erreur est
/// expurgée, et aucune redirection n'est suivie (elle pourrait emporter la clé vers un autre site).
pub struct OddsPapiClient {
    agent: ureq::Agent,
    key: ApiKey,
}

impl OddsPapiClient {
    /// Crée un client avec la clé donnée.
    pub fn new(key: ApiKey) -> OddsPapiClient {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .https_only(true)
            // L'historique d'un match peut peser quelques mégaoctets (Pinnacle change de prix des centaines de fois).
            .timeout_global(Some(Duration::from_secs(120)))
            .build();
        OddsPapiClient {
            agent: ureq::Agent::new_with_config(config),
            key,
        }
    }

    /// Envoie un `GET` sur `path` (`fixtures`, `historical-odds`, `account`) avec les paramètres `params`, sans clé.
    ///
    /// Le champ `quota` de la réponse reste vide : OddsPapi ne renvoie pas les en-têtes de The Odds API ; sa consommation
    /// se lit sur `account` ([`format::parse_usage`]).
    pub fn get(&self, path: &str, params: &str) -> Result<HttpResponse> {
        let url = request_url(path, params, &self.key);
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| Error::Http(redact(&e.to_string(), &self.key)))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            // Plafond de lecture relevé à 64 Mo : la limite par défaut de `ureq` (10 Mo) couperait un gros historique.
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| Error::Http(redact(&e.to_string(), &self.key)))?;
        Ok(HttpResponse {
            status,
            quota: Quota::default(),
            body,
        })
    }
}

/// URL complète d'un appel : la clé est ajoutée en dernier paramètre (`apiKey`, accepté par la version 4).
fn request_url(path: &str, params: &str, key: &ApiKey) -> String {
    let separator = if params.is_empty() { "" } else { "&" };
    format!(
        "{BASE_URL}{path}?{params}{separator}apiKey={}",
        key.expose()
    )
}

/// Code football-data (section de la table d'alias) d'un championnat OddsPapi ; `None` hors des six suivis.
pub fn league_div(tournament_id: u32) -> Option<&'static str> {
    LEAGUES
        .iter()
        .find(|(id, _)| *id == tournament_id)
        .map(|(_, div)| *div)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_the_last_parameter() {
        let url = request_url("fixtures", "tournamentId=17", &ApiKey::new("K"));
        assert_eq!(
            url,
            "https://api.oddspapi.io/v4/fixtures?tournamentId=17&apiKey=K"
        );
        let url = request_url("account", "", &ApiKey::new("K"));
        assert_eq!(url, "https://api.oddspapi.io/v4/account?apiKey=K");
    }

    #[test]
    fn the_six_leagues_map_to_their_alias_sections() {
        assert_eq!(league_div(17), Some("E0"));
        assert_eq!(league_div(36), Some("SC0"));
        assert_eq!(league_div(7), None);
    }
}
