//! Configuration de l'application et clé API.
//!
//! La configuration est un fichier TOML facultatif ; chaque valeur omise prend sa valeur par défaut.
//! La clé API se lit d'abord dans la variable d'environnement, puis dans le fichier, qui doit alors
//! être en permissions 0600 (jamais lisible par un autre utilisateur).

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::market::margin::MarginMethod;

/// Nom de la variable d'environnement qui contient la clé API.
pub const API_KEY_ENV: &str = "EDGE_ODDS_API_KEY";

/// Nom de la variable d'environnement qui contient la clé d'OddsPapi (historique des cotes, phase 13).
pub const ODDSPAPI_KEY_ENV: &str = "EDGE_ODDSPAPI_KEY";

/// Nom de la variable d'environnement qui contient l'URL du webhook Discord.
pub const WEBHOOK_ENV: &str = "EDGE_DISCORD_WEBHOOK";

/// Nom de la variable d'environnement qui contient le secret de connexion à l'application Web (`edge serve`).
pub const SERVE_TOKEN_ENV: &str = "EDGE_SERVE_TOKEN";

/// Longueur minimale du secret de connexion : 24 caractères tirés au hasard ne se devinent pas en essayant.
pub const MIN_SERVE_TOKEN: usize = 24;

/// Débuts d'URL acceptés pour un webhook Discord : l'URL contient un secret et le rapport est envoyé à cette adresse,
/// on refuse donc tout autre site (faute de frappe, ou fichier de configuration modifié).
const WEBHOOK_PREFIXES: [&str; 2] = [
    "https://discord.com/api/webhooks/",
    "https://discordapp.com/api/webhooks/",
];

/// Contenu de `config/edge.example.toml`, embarqué dans le binaire à la compilation (`include_str!`).
/// `edge init` l'écrit chez l'utilisateur ; un test garantit qu'il reste identique aux valeurs par défaut.
pub const EXAMPLE_TOML: &str = include_str!("../../../config/edge.example.toml");

/// Clé secrète de l'API.
///
/// C'est un *newtype* : une structure à un seul champ qui enveloppe un `String`. Pour le compilateur,
/// une `ApiKey` n'est pas un `String` : on ne peut pas la passer par erreur à une fonction qui attend
/// un nom de ligue, ni l'afficher sans le vouloir. Ici, `Debug` est écrit à la main pour masquer la
/// valeur : même un `{:?}` oublié dans un log ne divulgue rien.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct ApiKey(String);

impl ApiKey {
    /// Enveloppe une valeur déjà lue.
    pub fn new(value: impl Into<String>) -> ApiKey {
        ApiKey(value.into())
    }

    /// Donne accès à la valeur en clair. Le nom rend chaque usage visible à la relecture :
    /// à n'appeler que pour construire la requête HTTP.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

/// URL d'un webhook Discord. Elle porte le jeton : quiconque la connaît peut écrire dans le salon.
///
/// Même principe que [`ApiKey`] : newtype dont `Debug` masque la valeur.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct WebhookUrl(String);

impl WebhookUrl {
    /// Enveloppe une URL après avoir vérifié qu'elle pointe bien vers Discord.
    pub fn new(value: impl Into<String>) -> Result<WebhookUrl> {
        let value = value.into();
        if WEBHOOK_PREFIXES
            .iter()
            .any(|prefix| value.starts_with(prefix))
        {
            Ok(WebhookUrl(value))
        } else {
            Err(Error::InvalidConfig(format!(
                "l'URL du webhook doit commencer par {}",
                WEBHOOK_PREFIXES[0]
            )))
        }
    }

    /// Donne accès à l'URL en clair : à n'appeler que pour envoyer la requête.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WebhookUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WebhookUrl(***)")
    }
}

/// Secret de connexion à l'application Web. Même principe que [`ApiKey`] : newtype dont `Debug` masque la valeur.
///
/// Il finit dans un cookie : seuls les lettres, chiffres, `-` et `_` sont admis, ce qui évite tout caractère que l'en-tête
/// `Cookie` interdit (espace, `;`, `,`…).
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct ServeToken(String);

impl ServeToken {
    /// Vérifie et enveloppe un secret : [`MIN_SERVE_TOKEN`] caractères au moins, lettres, chiffres, `-` ou `_`.
    pub fn new(value: impl Into<String>) -> Result<ServeToken> {
        let value = value.into();
        let allowed = value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if value.chars().count() < MIN_SERVE_TOKEN || !allowed {
            return Err(Error::InvalidConfig(format!(
                "serve_token : {MIN_SERVE_TOKEN} caractères au moins, lettres, chiffres, - ou _ (par exemple la sortie de `openssl rand -hex 24`)"
            )));
        }
        Ok(ServeToken(value))
    }

    /// Donne accès au secret en clair : à n'appeler que pour le comparer à celui d'une requête.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ServeToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServeToken(***)")
    }
}

/// Configuration de l'application.
///
/// `#[serde(default)]` : un champ absent du fichier reçoit la valeur de [`Config::default`].
/// `deny_unknown_fields` : un champ inconnu (faute de frappe) est une erreur, jamais ignoré.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Plafond de crédits consommés par période de quota (garde de budget).
    pub monthly_credit_cap: u32,
    /// Ligues archivées, au format des clés de The Odds API (`soccer_epl`…).
    pub leagues: Vec<String>,
    /// Bookmakers interrogés (`pinnacle`, `winamax_fr`…).
    pub bookmakers: Vec<String>,
    /// Bookmakers archivés mais jamais recommandés ni montrés (choix personnel : opérateur écarté). Peut être vide.
    pub excluded_bookmakers: Vec<String>,
    /// Marchés interrogés (`h2h` = victoire / nul / défaite).
    pub markets: Vec<String>,
    /// Méthode de retrait de la marge des cotes. `power` : log loss le plus bas sur 19 138 matchs Pinnacle
    /// (rapport `edge market report`, 2026-10-18), écart avec les autres non significatif au niveau Pinnacle.
    pub margin_method: MarginMethod,
    /// Clé API écrite dans le fichier (alternative à la variable d'environnement).
    pub odds_api_key: Option<ApiKey>,
    /// Clé d'OddsPapi écrite dans le fichier (alternative à `EDGE_ODDSPAPI_KEY`). Secret : fichier en permissions 0600.
    pub oddspapi_api_key: Option<ApiKey>,
    /// Plafond de requêtes OddsPapi décomptées par mois (l'offre gratuite en donne 250 ; l'historique n'est pas décompté).
    pub oddspapi_monthly_cap: u32,
    /// Bankroll de départ, en euros. La bankroll courante en déduit les gains et les mises de tes paris enregistrés.
    pub initial_bankroll: f64,
    /// Archiver aussi les cotes des tournois de tennis en cours (clés découvertes par `/sports`, 1 crédit par tournoi).
    pub tennis_archive: bool,
    /// Nombre maximal de tournois de tennis archivés par relevé (garde de coût pendant les semaines chargées).
    pub tennis_max_tournaments: u32,
    /// Archiver les cotes et les résultats de la NBA (`edge nba snapshot`, relevé du soir).
    pub nba_archive: bool,
    /// Marchés archivés pour la NBA : `h2h` (vainqueur), `spreads` (handicaps), `totals` (plus/moins). 1 crédit chacun.
    pub nba_markets: Vec<String>,
    /// Crédits mis de côté pour la NBA : le tennis est refusé dès que `crédits utilisés + coût` dépasserait `plafond − réserve`.
    pub nba_reserve_credits: u32,
    /// Fraction de Kelly appliquée aux mises suggérées (0,5 = demi-Kelly, choix du 2026-10-20 ; 1 = Kelly complet).
    pub kelly_fraction: f64,
    /// Mise minimale, en euros : un pari plus petit n'est pas suggéré.
    pub min_stake: f64,
    /// URL du webhook Discord (alternative à la variable d'environnement). Secret : fichier en permissions 0600.
    pub discord_webhook_url: Option<WebhookUrl>,
    /// Adresse et port d'écoute de `edge serve`. `127.0.0.1` : ce poste seul ; `0.0.0.0` : tout le réseau local.
    /// Le type `SocketAddr` fait valider l'adresse par `serde` dès la lecture du fichier.
    pub serve_bind: SocketAddr,
    /// Secret de connexion à l'application Web (alternative à la variable d'environnement). Secret : fichier en 0600.
    pub serve_token: Option<ServeToken>,
    /// Adresse du site telle qu'un autre appareil la tape (`http://portable.local:8737/`) : rappelée dans le message
    /// Discord, qui ne joint alors plus le rapport HTML. Absente : le rapport reste joint.
    pub site_url: Option<String>,
    /// Valeur supposée d'un euro de freebet converti par couverture, pour le calcul d'un premier pari remboursé en freebet
    /// (`edge bonus`). Défaut : 0,53, bas de la fourchette mesurée sur l'archive (`docs/bonus-bienvenue.md` §4).
    pub bonus_freebet_value: f64,
    /// Loto Foot : nombre maximal de tickets d'un euro par séance (budget plafond, pas une obligation ; choix de Paul du
    /// 2026-10-22 : 16 €).
    pub lotofoot_max_tickets: u32,
    /// Loto Foot : espérance minimale d'un ticket dans le scénario défavorable pour qu'il soit joué (τ = 1,2).
    pub lotofoot_threshold: f64,
    /// Loto Foot : poids μ de la probabilité de gain dans la recherche des tickets (10).
    pub lotofoot_frequency_weight: f64,
    /// Loto Foot : heures minimales entre la séance et la clôture d'une grille (temps d'aller valider chez le buraliste).
    pub lotofoot_min_hours_before_close: u32,
    /// Loto Foot : âge maximal, en heures, d'une cote Pinnacle de l'archive pour qu'elle serve sans nouveau relevé.
    pub lotofoot_max_odds_age_hours: u32,
    /// Loto Foot : crédits The Odds API au plus par séance pour relever les ligues absentes de l'archive (1 crédit chacune).
    pub lotofoot_session_credits: u32,
    /// Loto Foot : compétitions où chercher les rencontres des grilles (`/events`, gratuit), en plus de `leagues`.
    pub lotofoot_leagues: Vec<String>,
    /// Grand Loto : éditions annoncées par la FDJ, saisies à la main (choix de Paul du 2026-10-23 : l'archive ne contient
    /// pas les tirages à venir, et les pages de fdj.fr ne sont pas lues). Vide par défaut.
    pub grand_loto_editions: Vec<GrandLotoEdition>,
    /// Grand Loto : budget d'une édition, en euros (choix de Paul du 2026-10-23 : 20 €, soit quatre grilles à 5 €).
    pub grand_loto_budget: f64,
    /// Grand Loto : espérance minimale d'une grille, en euros rendus par euro misé, au nombre de grilles pessimiste
    /// (choix de Paul du 2026-10-23 : 1,05).
    pub grand_loto_threshold: f64,
    /// Grand Loto : quantile de la prévision du nombre de grilles retenu comme pessimiste (0,8 : quatre chances sur cinq
    /// qu'il y ait moins de grilles).
    pub grand_loto_quantile: f64,
    /// Grand Loto : la séance a lieu au plus tant de jours avant l'édition (le timer quotidien la lance alors).
    pub grand_loto_days_before: u32,
}

/// Une édition du Grand Loto, telle que la FDJ l'annonce.
///
/// Dans `edge.toml` : `[[grand_loto_editions]]` puis `date = "2026-12-25"` et `jackpot = 20000000`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrandLotoEdition {
    /// Date du tirage (AAAA-MM-JJ, entre guillemets).
    pub date: chrono::NaiveDate,
    /// Jackpot garanti, en euros.
    pub jackpot: f64,
    /// Prix d'une grille, en euros (5 € depuis 2024 au moins).
    #[serde(default = "default_grand_loto_price")]
    pub price: f64,
    /// Nombre de codes gagnants (100 à chaque édition depuis 2017).
    #[serde(default = "default_grand_loto_codes")]
    pub codes: u32,
    /// Gain d'un code, en euros.
    #[serde(default = "default_grand_loto_code_gain")]
    pub code_gain: f64,
}

fn default_grand_loto_price() -> f64 {
    crate::loto::rules::GRAND_LOTO_PRICE
}

fn default_grand_loto_codes() -> u32 {
    100
}

fn default_grand_loto_code_gain() -> f64 {
    20_000.0
}

impl Default for Config {
    fn default() -> Config {
        // `to_string` crée un `String` possédé à partir du littéral `&'static str` : le champ
        // doit posséder sa donnée pour que la configuration vive indépendamment du code source.
        let owned = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        Config {
            monthly_credit_cap: 450,
            leagues: owned(&[
                "soccer_epl",
                "soccer_spain_la_liga",
                "soccer_germany_bundesliga",
                "soccer_italy_serie_a",
                "soccer_france_ligue_one",
                "soccer_spl",
            ]),
            bookmakers: owned(&[
                "pinnacle",
                "winamax_fr",
                "betclic_fr",
                "unibet_fr",
                "pmu_fr",
                "netbet_fr",
            ]),
            excluded_bookmakers: owned(&["netbet_fr"]),
            markets: owned(&["h2h"]),
            margin_method: MarginMethod::Power,
            odds_api_key: None,
            oddspapi_api_key: None,
            oddspapi_monthly_cap: 200,
            tennis_archive: true,
            tennis_max_tournaments: 6,
            nba_archive: true,
            nba_markets: owned(&["h2h", "spreads", "totals"]),
            nba_reserve_credits: 120,
            initial_bankroll: 1000.0,
            kelly_fraction: 0.5,
            min_stake: 1.0,
            discord_webhook_url: None,
            serve_bind: SocketAddr::from(([127, 0, 0, 1], 8737)),
            serve_token: None,
            site_url: None,
            bonus_freebet_value: crate::bonus::DEFAULT_FREEBET_VALUE,
            lotofoot_max_tickets: 16,
            lotofoot_threshold: 1.2,
            lotofoot_frequency_weight: 10.0,
            lotofoot_min_hours_before_close: 3,
            lotofoot_max_odds_age_hours: 30,
            lotofoot_session_credits: 8,
            // Ligues vues dans les grilles LF7/LF8 de 2025-2026 (`docs/sources.md` §38) ; clés vérifiées sur `/sports`
            // le 2026-10-22.
            lotofoot_leagues: owned(&[
                "soccer_france_ligue_two",
                "soccer_uefa_champs_league",
                "soccer_uefa_europa_league",
                "soccer_uefa_europa_conference_league",
                "soccer_efl_champ",
                "soccer_germany_bundesliga2",
                "soccer_italy_serie_b",
                "soccer_spain_segunda_division",
                "soccer_netherlands_eredivisie",
                "soccer_portugal_primeira_liga",
                "soccer_belgium_first_div",
                "soccer_turkey_super_league",
                "soccer_switzerland_superleague",
                "soccer_austria_bundesliga",
                "soccer_greece_super_league",
                "soccer_denmark_superliga",
                "soccer_norway_eliteserien",
                "soccer_sweden_allsvenskan",
                "soccer_poland_ekstraklasa",
                "soccer_league_of_ireland",
                "soccer_usa_mls",
                "soccer_brazil_campeonato",
                "soccer_argentina_primera_division",
                "soccer_uefa_nations_league",
                "soccer_fifa_world_cup_qualifiers_europe",
                "soccer_france_coupe_de_france",
                "soccer_germany_dfb_pokal",
                "soccer_england_efl_cup",
                "soccer_fa_cup",
                "soccer_spain_copa_del_rey",
                "soccer_italy_coppa_italia",
            ]),
            grand_loto_editions: Vec::new(),
            grand_loto_budget: 20.0,
            grand_loto_threshold: 1.05,
            grand_loto_quantile: 0.8,
            grand_loto_days_before: 3,
        }
    }
}

impl Config {
    /// Lit la configuration depuis un texte TOML.
    pub fn parse(text: &str) -> std::result::Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    /// Charge le fichier de configuration. Un fichier absent donne la configuration par défaut.
    ///
    /// Si le fichier contient une clé API, ses permissions doivent être 0600 (Unix).
    pub fn load(path: &Path) -> Result<Config> {
        // `match` avec garde (`if`) : on distingue « fichier absent » (cas normal à la première
        // utilisation) des autres erreurs d'entrée/sortie, qui sont de vrais problèmes.
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(Error::io(path)(e)),
        };
        let config = Config::parse(&text).map_err(|source| Error::ConfigParse {
            path: path.to_path_buf(),
            source,
        })?;
        config.validate()?;
        if config.odds_api_key.is_some()
            || config.oddspapi_api_key.is_some()
            || config.discord_webhook_url.is_some()
            || config.serve_token.is_some()
        {
            check_permissions(path)?;
        }
        Ok(config)
    }

    /// Vérifie les valeurs avant tout appel réseau : un identifiant malformé ou une liste vide
    /// serait envoyé à l'API, qui peut facturer la requête sans la rejeter.
    pub fn validate(&self) -> Result<()> {
        if self.monthly_credit_cap == 0 {
            return Err(Error::InvalidConfig(
                "monthly_credit_cap doit être supérieur à 0".to_string(),
            ));
        }
        if self.oddspapi_monthly_cap == 0 {
            return Err(Error::InvalidConfig(
                "oddspapi_monthly_cap doit être supérieur à 0".to_string(),
            ));
        }
        if !(self.initial_bankroll.is_finite() && self.initial_bankroll > 0.0) {
            return Err(Error::InvalidConfig(
                "initial_bankroll doit être un montant strictement positif".to_string(),
            ));
        }
        if !(self.kelly_fraction > 0.0 && self.kelly_fraction <= 1.0) {
            return Err(Error::InvalidConfig(
                "kelly_fraction doit être dans ]0 ; 1]".to_string(),
            ));
        }
        if !(self.bonus_freebet_value > 0.0 && self.bonus_freebet_value < 1.0) {
            return Err(Error::InvalidConfig(
                "bonus_freebet_value doit être dans ]0 ; 1[".to_string(),
            ));
        }
        if !(self.min_stake.is_finite() && self.min_stake >= 0.0) {
            return Err(Error::InvalidConfig(
                "min_stake doit être un montant positif ou nul".to_string(),
            ));
        }
        if self.lotofoot_max_tickets == 0 {
            return Err(Error::InvalidConfig(
                "lotofoot_max_tickets doit être supérieur à 0".to_string(),
            ));
        }
        if !(self.lotofoot_threshold.is_finite() && self.lotofoot_threshold >= 1.0) {
            return Err(Error::InvalidConfig(
                "lotofoot_threshold doit être au moins 1 (un ticket d'espérance inférieure à la mise perd en moyenne)"
                    .to_string(),
            ));
        }
        if !(self.lotofoot_frequency_weight.is_finite() && self.lotofoot_frequency_weight >= 0.0) {
            return Err(Error::InvalidConfig(
                "lotofoot_frequency_weight doit être positif ou nul".to_string(),
            ));
        }
        self.validate_grand_loto()?;
        let lists = [
            ("leagues", &self.leagues),
            ("bookmakers", &self.bookmakers),
            ("markets", &self.markets),
            ("nba_markets", &self.nba_markets),
            ("lotofoot_leagues", &self.lotofoot_leagues),
        ];
        for (field, names) in lists {
            if names.is_empty() {
                return Err(Error::InvalidConfig(format!(
                    "{field} ne doit pas être vide"
                )));
            }
            if let Some(bad) = names.iter().find(|name| !is_api_name(name)) {
                return Err(Error::InvalidConfig(format!(
                    "{field} : « {bad} » n'est pas un identifiant valide (minuscules, chiffres et _ uniquement)"
                )));
            }
        }
        if let Some(url) = &self.site_url
            && !is_site_url(url)
        {
            return Err(Error::InvalidConfig(format!(
                "site_url : « {url} » doit commencer par http:// ou https:// et ne contenir aucun espace"
            )));
        }
        if let Some(bad) = self
            .excluded_bookmakers
            .iter()
            .find(|name| !is_api_name(name))
        {
            return Err(Error::InvalidConfig(format!(
                "excluded_bookmakers : « {bad} » n'est pas un identifiant valide (minuscules, chiffres et _ uniquement)"
            )));
        }
        Ok(())
    }

    /// Vérifie les réglages du Grand Loto : budget d'au moins une grille, seuil d'au moins 1, quantile dans [0,5 ; 1[,
    /// éditions à jackpot et prix positifs, sans date en double.
    fn validate_grand_loto(&self) -> Result<()> {
        if !(self.grand_loto_threshold.is_finite() && self.grand_loto_threshold >= 1.0) {
            return Err(Error::InvalidConfig(
                "grand_loto_threshold doit être au moins 1 (une grille d'espérance inférieure à la mise perd en moyenne)"
                    .to_string(),
            ));
        }
        if !(0.5..1.0).contains(&self.grand_loto_quantile) {
            return Err(Error::InvalidConfig(
                "grand_loto_quantile doit être dans [0,5 ; 1[".to_string(),
            ));
        }
        if !(self.grand_loto_budget.is_finite() && self.grand_loto_budget > 0.0) {
            return Err(Error::InvalidConfig(
                "grand_loto_budget doit être un montant strictement positif".to_string(),
            ));
        }
        for (i, edition) in self.grand_loto_editions.iter().enumerate() {
            let valid = edition.jackpot.is_finite()
                && edition.jackpot > 0.0
                && edition.price.is_finite()
                && edition.price > 0.0
                && edition.code_gain.is_finite()
                && edition.code_gain >= 0.0;
            if !valid {
                return Err(Error::InvalidConfig(format!(
                    "grand_loto_editions : l'édition du {} doit avoir un jackpot et un prix positifs",
                    edition.date
                )));
            }
            if self.grand_loto_editions[..i]
                .iter()
                .any(|e| e.date == edition.date)
            {
                return Err(Error::InvalidConfig(format!(
                    "grand_loto_editions : l'édition du {} figure deux fois",
                    edition.date
                )));
            }
            if self.grand_loto_budget < edition.price {
                return Err(Error::InvalidConfig(format!(
                    "grand_loto_budget ({} €) ne paie pas une grille de l'édition du {} ({} €)",
                    self.grand_loto_budget, edition.date, edition.price
                )));
            }
        }
        Ok(())
    }

    /// Détermine l'URL du webhook Discord : la variable d'environnement prime, sinon celle du fichier.
    pub fn discord_webhook(&self, from_env: Option<&str>) -> Result<WebhookUrl> {
        match from_env.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => WebhookUrl::new(value),
            None => self
                .discord_webhook_url
                .clone()
                .filter(|url| !url.expose().is_empty())
                .ok_or(Error::MissingWebhook),
        }
    }

    /// Détermine le secret de connexion à l'application Web : la variable d'environnement (`from_env`) prime, sinon celui
    /// du fichier. Il est vérifié dans les deux cas.
    pub fn web_token(&self, from_env: Option<&str>) -> Result<ServeToken> {
        match from_env.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => ServeToken::new(value),
            None => match &self.serve_token {
                Some(token) if !token.expose().is_empty() => ServeToken::new(token.expose()),
                _ => Err(Error::MissingServeToken),
            },
        }
    }

    /// Détermine la clé API : la variable d'environnement (`from_env`) prime, sinon celle du fichier.
    ///
    /// L'appelant lit l'environnement et nous passe la valeur : la fonction reste pure, donc testable
    /// sans modifier l'environnement du processus.
    pub fn api_key(&self, from_env: Option<&str>) -> Result<ApiKey> {
        match from_env.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => Ok(ApiKey::new(value)),
            // `clone()` : `self` n'est qu'emprunté ici ; l'appelant reçoit sa propre copie de la clé
            // (une petite chaîne) et n'est plus lié à la durée de vie de la configuration.
            // Une clé vide dans le fichier (`odds_api_key = ""`) équivaut à une clé absente.
            None => self
                .odds_api_key
                .clone()
                .filter(|key| !key.expose().is_empty())
                .ok_or(Error::MissingApiKey),
        }
    }
}

impl Config {
    /// Détermine la clé d'OddsPapi : la variable d'environnement (`from_env`) prime, sinon celle du fichier.
    ///
    /// Même règle que [`Config::api_key`] ; seule l'erreur diffère, pour dire quelle clé manque.
    pub fn oddspapi_key(&self, from_env: Option<&str>) -> Result<ApiKey> {
        match from_env.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => Ok(ApiKey::new(value)),
            None => self
                .oddspapi_api_key
                .clone()
                .filter(|key| !key.expose().is_empty())
                .ok_or(Error::MissingOddsPapiKey),
        }
    }
}

/// Les clés de The Odds API (`soccer_epl`, `winamax_fr`, `h2h`) sont en minuscules, chiffres et `_`.
/// Cela garantit aussi qu'aucun caractère spécial ne peut se glisser dans l'URL.
fn is_api_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// L'URL du site part dans un message Discord : un lien web, sans espace ni caractère de contrôle.
fn is_site_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Refuse un fichier de secrets lisible ou modifiable par le groupe ou les autres.
#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)
        .map_err(Error::io(path))?
        .permissions()
        .mode()
        & 0o777;
    // 0o077 = bits « groupe » et « autres » ; s'il y en a un d'allumé, le fichier est trop ouvert.
    if mode & 0o077 != 0 {
        return Err(Error::InsecurePermissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

/// Hors Unix (Windows), les permissions fonctionnent autrement : aucun contrôle.
#[cfg(not(unix))]
fn check_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_file_matches_the_defaults() {
        // Garantit que l'exemple documenté et les valeurs par défaut du code ne divergent jamais.
        assert_eq!(Config::parse(EXAMPLE_TOML).unwrap(), Config::default());
    }

    #[test]
    fn oddspapi_key_prefers_the_environment_and_names_itself_when_missing() {
        let config = Config::parse(r#"oddspapi_api_key = "du-fichier""#).unwrap();
        assert_eq!(
            config.oddspapi_key(Some("de-l-env")).unwrap().expose(),
            "de-l-env"
        );
        assert_eq!(config.oddspapi_key(None).unwrap().expose(), "du-fichier");
        let err = Config::default().oddspapi_key(Some("  ")).unwrap_err();
        assert!(err.to_string().contains("EDGE_ODDSPAPI_KEY"));
    }

    #[test]
    fn the_web_token_prefers_the_environment_and_must_be_long_and_cookie_safe() {
        let long = "a".repeat(MIN_SERVE_TOKEN);
        let config = Config::parse(&format!("serve_token = \"{long}\"")).unwrap();
        assert_eq!(config.web_token(None).unwrap().expose(), long);
        let from_env = "b".repeat(30);
        assert_eq!(
            config.web_token(Some(&from_env)).unwrap().expose(),
            from_env
        );
        assert!(config.web_token(Some("court")).is_err());
        assert!(format!("{:?}", config.web_token(None).unwrap()).contains("***"));
        let spaced = Config::parse(&format!("serve_token = \"{long} ;x\"")).unwrap();
        assert!(spaced.web_token(None).is_err());
        let missing = Config::default().web_token(Some(" ")).unwrap_err();
        assert!(missing.to_string().contains("EDGE_SERVE_TOKEN"));
    }

    #[test]
    fn default_cap_is_450() {
        assert_eq!(Config::default().monthly_credit_cap, 450);
    }

    #[test]
    fn omitted_fields_keep_their_defaults() {
        let config = Config::parse("monthly_credit_cap = 300").unwrap();
        assert_eq!(config.monthly_credit_cap, 300);
        assert_eq!(config.leagues, Config::default().leagues);
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = Config::parse("monthly_credit_capp = 300").unwrap_err();
        assert!(err.to_string().contains("monthly_credit_capp"));
    }

    #[test]
    fn missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("absent.toml")).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn invalid_toml_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.toml");
        fs::write(&path, "ceci n'est pas du toml").unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(err.to_string().contains("edge.toml"));
    }

    #[test]
    fn environment_key_has_priority_over_file_key() {
        let config = Config::parse(r#"odds_api_key = "du-fichier""#).unwrap();
        let key = config.api_key(Some("de-l-environnement")).unwrap();
        assert_eq!(key.expose(), "de-l-environnement");
    }

    #[test]
    fn file_key_is_used_without_environment() {
        let config = Config::parse(r#"odds_api_key = "du-fichier""#).unwrap();
        assert_eq!(config.api_key(None).unwrap().expose(), "du-fichier");
    }

    #[test]
    fn empty_environment_value_counts_as_absent() {
        let config = Config::parse(r#"odds_api_key = "du-fichier""#).unwrap();
        assert_eq!(config.api_key(Some("  ")).unwrap().expose(), "du-fichier");
    }

    #[test]
    fn empty_key_in_the_file_counts_as_missing() {
        let config = Config::parse("odds_api_key = \"\"").unwrap();
        assert!(matches!(
            config.api_key(None).unwrap_err(),
            Error::MissingApiKey
        ));
    }

    #[test]
    fn missing_key_error_tells_what_to_do() {
        let err = Config::default().api_key(None).unwrap_err();
        let message = err.to_string();
        assert!(message.contains(API_KEY_ENV));
        assert!(message.contains("odds_api_key"));
    }

    #[test]
    fn default_config_is_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn a_grand_loto_edition_is_read_with_its_defaults() {
        let config =
            Config::parse("[[grand_loto_editions]]\ndate = \"2026-12-25\"\njackpot = 20000000\n")
                .unwrap();
        config.validate().unwrap();
        let edition = config.grand_loto_editions[0];
        assert_eq!(
            edition.date,
            chrono::NaiveDate::from_ymd_opt(2026, 12, 25).unwrap()
        );
        assert_eq!(
            (edition.price, edition.codes, edition.code_gain),
            (5.0, 100, 20_000.0)
        );
        assert_eq!(config.grand_loto_budget, 20.0);
        assert_eq!(config.grand_loto_threshold, 1.05);
    }

    #[test]
    fn invalid_grand_loto_settings_are_rejected() {
        for text in [
            "grand_loto_threshold = 0.9",
            "grand_loto_quantile = 1.0",
            "grand_loto_budget = 0.0",
            "grand_loto_budget = 4.0\n[[grand_loto_editions]]\ndate = \"2026-12-25\"\njackpot = 20000000",
            "[[grand_loto_editions]]\ndate = \"2026-12-25\"\njackpot = 0",
            "[[grand_loto_editions]]\ndate = \"2026-12-25\"\njackpot = 1\n[[grand_loto_editions]]\ndate = \"2026-12-25\"\njackpot = 2",
        ] {
            let config = Config::parse(text).unwrap();
            assert!(config.validate().is_err(), "{text}");
        }
        assert!(
            Config::parse("[[grand_loto_editions]]\ndate = \"2026-12-25\"\njackpot = 1\nprix = 5")
                .is_err()
        );
    }

    #[test]
    fn netbet_is_archived_but_excluded_by_default_and_the_exclusion_list_may_be_empty() {
        let config = Config::default();
        assert!(config.bookmakers.contains(&"netbet_fr".to_string()));
        assert_eq!(config.excluded_bookmakers, ["netbet_fr"]);
        Config::parse("excluded_bookmakers = []")
            .unwrap()
            .validate()
            .unwrap();
        assert!(
            Config::parse(r#"excluded_bookmakers = ["NetBet"]"#)
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[test]
    fn malformed_identifier_is_rejected_with_its_field() {
        let config = Config::parse(r#"bookmakers = ["pinnacle", "Winamax FR"]"#).unwrap();
        let message = config.validate().unwrap_err().to_string();
        assert!(message.contains("bookmakers"));
        assert!(message.contains("Winamax FR"));
    }

    #[test]
    fn empty_list_and_zero_cap_are_rejected() {
        assert!(Config::parse("leagues = []").unwrap().validate().is_err());
        assert!(
            Config::parse("monthly_credit_cap = 0")
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[test]
    fn load_rejects_an_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.toml");
        fs::write(&path, r#"markets = ["h2h;drop"]"#).unwrap();
        assert!(matches!(
            Config::load(&path).unwrap_err(),
            Error::InvalidConfig(_)
        ));
    }

    #[test]
    fn the_betting_defaults_are_the_documented_ones() {
        let config = Config::default();
        assert_eq!(config.initial_bankroll, 1000.0);
        assert_eq!(config.kelly_fraction, 0.5);
        assert_eq!(config.min_stake, 1.0);
        assert!(config.discord_webhook_url.is_none());
    }

    #[test]
    fn invalid_betting_values_are_rejected() {
        for text in [
            "initial_bankroll = 0.0",
            "initial_bankroll = -5.0",
            "kelly_fraction = 0.0",
            "kelly_fraction = 1.5",
            "min_stake = -1.0",
        ] {
            let config = Config::parse(text).unwrap();
            assert!(config.validate().is_err(), "{text}");
        }
    }

    #[test]
    fn the_site_listens_on_this_machine_only_by_default_and_its_url_is_checked() {
        let config = Config::default();
        assert_eq!(config.serve_bind.to_string(), "127.0.0.1:8737");
        assert_eq!(config.site_url, None);

        let lan = Config::parse(
            "serve_bind = \"0.0.0.0:8737\"\nsite_url = \"http://portable.local:8737/\"",
        )
        .unwrap();
        assert!(lan.validate().is_ok());
        assert!(
            Config::parse("serve_bind = \"portable:8737\"").is_err(),
            "une adresse IP est exigée"
        );
        let bad = Config::parse("site_url = \"portable.local:8737\"").unwrap();
        assert!(bad.validate().unwrap_err().to_string().contains("site_url"));
    }

    #[test]
    fn a_webhook_must_point_to_discord() {
        assert!(WebhookUrl::new("https://discord.com/api/webhooks/123/abc").is_ok());
        assert!(WebhookUrl::new("https://evil.example/api/webhooks/123/abc").is_err());
        assert!(WebhookUrl::new("http://discord.com/api/webhooks/123/abc").is_err());
    }

    #[test]
    fn the_environment_webhook_has_priority_and_the_file_is_the_fallback() {
        let config =
            Config::parse(r#"discord_webhook_url = "https://discord.com/api/webhooks/1/fichier""#)
                .unwrap();

        let from_env = config
            .discord_webhook(Some("https://discord.com/api/webhooks/2/env"))
            .unwrap();
        let from_file = config.discord_webhook(None).unwrap();

        assert!(from_env.expose().ends_with("/env"));
        assert!(from_file.expose().ends_with("/fichier"));
        assert!(matches!(
            Config::default().discord_webhook(None).unwrap_err(),
            Error::MissingWebhook
        ));
        assert!(!format!("{from_env:?}").contains("env"));
    }

    #[test]
    fn debug_output_never_shows_the_key() {
        let config = Config::parse(r#"odds_api_key = "SECRET-123""#).unwrap();
        assert!(!format!("{config:?}").contains("SECRET-123"));
        assert!(!format!("{:?}", config.api_key(None).unwrap()).contains("SECRET-123"));
    }

    #[cfg(unix)]
    mod permissions {
        use std::os::unix::fs::PermissionsExt;

        use super::*;

        fn write_with_mode(dir: &Path, content: &str, mode: u32) -> std::path::PathBuf {
            let path = dir.join("edge.toml");
            fs::write(&path, content).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            path
        }

        #[test]
        fn file_with_a_key_and_open_permissions_is_refused() {
            let dir = tempfile::tempdir().unwrap();
            let path = write_with_mode(dir.path(), r#"odds_api_key = "x""#, 0o644);
            let err = Config::load(&path).unwrap_err();
            assert!(matches!(
                err,
                Error::InsecurePermissions { mode: 0o644, .. }
            ));
            assert!(err.to_string().contains("chmod 600"));
        }

        #[test]
        fn file_with_a_key_and_mode_600_is_accepted() {
            let dir = tempfile::tempdir().unwrap();
            let path = write_with_mode(dir.path(), r#"odds_api_key = "x""#, 0o600);
            assert!(Config::load(&path).unwrap().odds_api_key.is_some());
        }

        #[test]
        fn file_without_a_key_may_stay_readable() {
            let dir = tempfile::tempdir().unwrap();
            let path = write_with_mode(dir.path(), "monthly_credit_cap = 300", 0o644);
            assert_eq!(Config::load(&path).unwrap().monthly_credit_cap, 300);
        }
    }
}
