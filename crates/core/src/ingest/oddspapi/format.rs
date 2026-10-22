//! Lecture des réponses d'OddsPapi : liste des matchs, historique des cotes, consommation du compte.
//!
//! Formats lus dans la documentation de la version 4 (oddspapi.io/us/docs, 2026-10-21) et vérifiés sur des réponses
//! réelles. Seuls les champs utiles sont déclarés : `serde` ignore le reste, notamment la clé et l'adresse électronique
//! que renvoie `account`, qui ne sont donc jamais copiées dans une structure du programme.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::domain::Odds;
use crate::error::{Error, Result};

/// Marchés conservés : 1X2 (101), plus/moins 2,5 buts (1010), les deux équipes marquent (104).
/// Le test de latence n'utilise que le 1X2 ; les deux autres sont archivés pour une étude future, sans coût de plus.
pub const MARKETS_KEPT: [u32; 3] = [101, 1010, 104];

/// Statut OddsPapi d'un match terminé (`statusId` 2 ; 0 : à venir, 1 : en cours, 3 : annulé).
pub const STATUS_FINISHED: u8 = 2;

/// Un match tel que le liste `GET /v4/fixtures`.
#[derive(Debug, Clone, PartialEq)]
pub struct Fixture {
    /// Identifiant OddsPapi (`id1000001761301153`).
    pub fixture_id: String,
    /// Championnat (17 : Premier League…).
    pub tournament_id: u32,
    /// Coup d'envoi prévu, UTC.
    pub kickoff: DateTime<Utc>,
    /// Équipe à domicile, nom brut d'OddsPapi.
    pub home: String,
    /// Équipe à l'extérieur, nom brut d'OddsPapi.
    pub away: String,
    /// Statut (voir [`STATUS_FINISHED`]).
    pub status_id: u8,
}

/// Un changement de prix d'une issue chez un bookmaker.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceChange {
    /// Bookmaker (`pinnacle`, `winamax.fr`…).
    pub bookmaker: String,
    /// Marché (voir [`MARKETS_KEPT`]).
    pub market_id: u32,
    /// Issue : pour le 1X2, 101 domicile, 102 nul, 103 extérieur.
    pub outcome_id: u32,
    /// Instant où OddsPapi a vu le changement, UTC.
    pub changed_at: DateTime<Utc>,
    /// Cote ; `None` pour une suspension publiée sans prix.
    pub price: Option<Odds>,
    /// `false` : l'issue est suspendue (indisponible) à partir de cet instant.
    pub active: bool,
}

/// Historique d'avant-match d'un match.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    /// Changements de prix antérieurs au coup d'envoi, triés par instant.
    pub prices: Vec<PriceChange>,
    /// Entrées écartées parce qu'actives sans cote valide (prix absent ou ≤ 1) : signalées, jamais corrigées.
    pub discarded: usize,
}

/// Consommation de l'abonnement actif.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Requêtes décomptées depuis le début du mois.
    pub used: u32,
    /// Allocation mensuelle de l'offre (250 en gratuit).
    pub limit: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFixture {
    fixture_id: String,
    tournament_id: u32,
    start_time: DateTime<Utc>,
    participant1_name: String,
    participant2_name: String,
    status_id: u8,
}

/// Lit la réponse de `GET /v4/fixtures` (un tableau de matchs).
pub fn parse_fixtures(body: &str) -> Result<Vec<Fixture>> {
    let raw: Vec<RawFixture> = serde_json::from_str(body)?;
    Ok(raw
        .into_iter()
        .map(|f| Fixture {
            fixture_id: f.fixture_id,
            tournament_id: f.tournament_id,
            kickoff: f.start_time,
            home: f.participant1_name,
            away: f.participant2_name,
            status_id: f.status_id,
        })
        .collect())
}

// Structure imbriquée de l'historique : bookmaker → marchés → issues → joueurs → entrées. Le « joueur » `0` porte les
// marchés d'équipe ; les autres clés servent aux paris sur un joueur, que l'on ignore. `BTreeMap` (et non `HashMap`) :
// l'ordre de parcours est fixé, donc le résultat est identique d'une exécution à l'autre (invariant 9).
#[derive(Deserialize)]
struct RawHistory {
    #[serde(default)]
    bookmakers: BTreeMap<String, RawBook>,
}

#[derive(Deserialize)]
struct RawBook {
    #[serde(default)]
    markets: BTreeMap<String, RawMarket>,
}

#[derive(Deserialize)]
struct RawMarket {
    #[serde(default)]
    outcomes: BTreeMap<String, RawOutcome>,
}

#[derive(Deserialize)]
struct RawOutcome {
    #[serde(default)]
    players: BTreeMap<String, Vec<RawEntry>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEntry {
    created_at: DateTime<Utc>,
    price: Option<f64>,
    active: Option<bool>,
}

/// Lit la réponse de `GET /v4/historical-odds` et ne garde que l'avant-match (`changed_at < kickoff`) des marchés suivis.
///
/// Une entrée sans indicateur `active` est tenue pour active : c'est le sens du prix publié.
pub fn parse_history(body: &str, kickoff: DateTime<Utc>) -> Result<History> {
    let raw: RawHistory = serde_json::from_str(body)?;
    let mut history = History::default();
    for (bookmaker, book) in raw.bookmakers {
        for (market_key, market) in book.markets {
            let Some(market_id) = kept_market(&market_key) else {
                continue;
            };
            for (outcome_key, outcome) in market.outcomes {
                let outcome_id = outcome_key.parse::<u32>().map_err(|_| {
                    Error::Data(format!("issue OddsPapi illisible : « {outcome_key} »"))
                })?;
                // `remove` plutôt que `get` : on prend possession des entrées du joueur 0 sans les copier.
                let mut players = outcome.players;
                for entry in players.remove("0").unwrap_or_default() {
                    if entry.created_at >= kickoff {
                        continue;
                    }
                    let active = entry.active.unwrap_or(true);
                    let price = entry.price.and_then(Odds::new);
                    if active && price.is_none() {
                        history.discarded += 1;
                        continue;
                    }
                    history.prices.push(PriceChange {
                        bookmaker: bookmaker.clone(),
                        market_id,
                        outcome_id,
                        changed_at: entry.created_at,
                        price,
                        active,
                    });
                }
            }
        }
    }
    history.prices.sort_by(|a, b| {
        (&a.bookmaker, a.market_id, a.outcome_id, a.changed_at).cmp(&(
            &b.bookmaker,
            b.market_id,
            b.outcome_id,
            b.changed_at,
        ))
    });
    Ok(history)
}

fn kept_market(key: &str) -> Option<u32> {
    key.parse::<u32>()
        .ok()
        .filter(|id| MARKETS_KEPT.contains(id))
}

#[derive(Deserialize)]
struct RawAccount {
    subscriptions: Vec<RawSubscription>,
}

#[derive(Deserialize)]
struct RawSubscription {
    is_active: bool,
    request_count: u32,
    request_limit: u32,
}

/// Lit la consommation de l'abonnement actif dans la réponse de `GET /v4/account`.
pub fn parse_usage(body: &str) -> Result<Usage> {
    let raw: RawAccount = serde_json::from_str(body)?;
    raw.subscriptions
        .into_iter()
        .find(|s| s.is_active)
        .map(|s| Usage {
            used: s.request_count,
            limit: s.request_limit,
        })
        .ok_or_else(|| Error::Data("OddsPapi : aucun abonnement actif sur ce compte".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().to_utc()
    }

    #[test]
    fn fixtures_are_read_with_their_raw_names() {
        let body = r#"[{"fixtureId":"id1","participant1Id":35,"participant2Id":34,"sportId":10,"tournamentId":17,
            "statusId":2,"startTime":"2026-04-13T19:00:00.000Z","participant1Name":"Liverpool FC",
            "participant2Name":"Manchester United","externalProviders":{"betradarId":61301153}}]"#;
        let fixtures = parse_fixtures(body).unwrap();
        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].home, "Liverpool FC");
        assert_eq!(fixtures[0].kickoff, at("2026-04-13T19:00:00Z"));
        assert_eq!(fixtures[0].status_id, STATUS_FINISHED);
    }

    // Exemple de la documentation, complété d'une entrée en direct, d'un prix invalide et d'un marché non suivi.
    const HISTORY: &str = r#"{"fixtureId":"id1","bookmakers":{"pinnacle":{"markets":{
        "101":{"outcomes":{"101":{"players":{"0":[
            {"createdAt":"2025-04-16T21:12:10.506331+00:00","price":9.11,"limit":1191.25,"active":false,"exchangeMeta":null},
            {"createdAt":"2025-04-16T20:50:58.321847+00:00","price":9.11,"limit":1191.25,"active":true,"exchangeMeta":null},
            {"createdAt":"2025-04-16T20:40:00+00:00","price":1.0,"active":true}]}}}},
        "10168":{"outcomes":{"10168":{"players":{"0":[
            {"createdAt":"2025-04-16T20:16:45.057806+00:00","price":1.729,"active":true}]}}}}}}}}"#;

    #[test]
    fn history_keeps_pre_match_entries_of_followed_markets_only() {
        let history = parse_history(HISTORY, at("2025-04-16T21:00:00Z")).unwrap();
        // L'entrée de 21:12 est après le coup d'envoi ; le marché 10168 n'est pas suivi ; le prix 1,0 est écarté.
        assert_eq!(history.prices.len(), 1);
        assert_eq!(history.discarded, 1);
        let change = &history.prices[0];
        assert_eq!((change.market_id, change.outcome_id), (101, 101));
        assert_eq!(change.price.map(Odds::get), Some(9.11));
        assert!(change.active);
    }

    #[test]
    fn an_empty_history_is_not_an_error() {
        let history = parse_history(r#"{"fixtureId":"id1"}"#, at("2025-04-16T21:00:00Z")).unwrap();
        assert_eq!(history, History::default());
    }

    #[test]
    fn usage_comes_from_the_active_subscription_and_secrets_are_ignored() {
        let body = r#"{"api_key":"SECRET","email":"x@y","subscriptions":[
            {"is_active":false,"request_count":250,"request_limit":250},
            {"is_active":true,"request_count":13,"request_limit":250,"bookmakers":{}}]}"#;
        assert_eq!(
            parse_usage(body).unwrap(),
            Usage {
                used: 13,
                limit: 250
            }
        );
    }
}
