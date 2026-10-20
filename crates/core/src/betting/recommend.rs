//! Recommandations quotidiennes : le dernier relevé de l'archive devient des lignes du journal immuable.
//!
//! Deux paliers, pour un même mécanisme (le prix d'un bookmaker contre Pinnacle sans marge, voir [`super::live`]) :
//!
//! - **`recommended`** : l'avantage atteint le seuil ([`RECOMMEND_MIN_EDGE`]) ; une mise est suggérée et la
//!   recommandation est notifiée ;
//! - **`shadow`** : l'avantage est positif mais sous le seuil ; journalisé en silence, sans mise, pour mesurer le CLV
//!   réel par tranche d'avantage sans attendre des années de recommandations rares.
//!
//! Relancer la commande ne duplique rien : un match a au plus une ligne par palier, la première écrite reste.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::live::{self, Opportunity};
use super::value::minimal_odds;
use crate::domain::Euros;
use crate::error::Result;
use crate::market::margin::MarginMethod;
use crate::store::journal::{
    self, NewRecommendation, StoredRecommendation, Tier, recommendation_exists,
};

/// Nom de la règle inscrit dans le journal.
pub const RULE: &str = "price_vs_pinnacle";

/// Avantage minimal d'une recommandation notifiée : 3 %, seuil gelé de la règle `soft:B365` du backtest
/// (`edge backtest report`), appliqué ici **par analogie** aux opérateurs français, qui n'ont pas d'historique.
/// Aucune règle n'est validée hors échantillon : chaque recommandation est expérimentale.
pub const RECOMMEND_MIN_EDGE: f64 = 0.03;

/// Réglages d'une exécution.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Méthode de retrait de la marge de Pinnacle.
    pub method: MarginMethod,
    /// Avantage minimal d'une recommandation notifiée.
    pub min_edge: f64,
    /// Bookmakers jamais recommandés (`excluded_bookmakers` de la configuration).
    pub excluded_bookmakers: Vec<String>,
    /// Fraction de Kelly.
    pub kelly_fraction: f64,
    /// Mise minimale.
    pub min_stake: Euros,
    /// Bankroll courante.
    pub bankroll: Euros,
    /// Version du programme, stockée avec chaque recommandation.
    pub code_version: String,
}

/// Bilan d'une exécution.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Matchs à venir dans un relevé assez récent.
    pub events: usize,
    /// Matchs ignorés faute d'un 1X2 complet et plausible de Pinnacle.
    pub without_reference: usize,
    /// Prix aberrants écartés.
    pub implausible_quotes: usize,
    /// Lignes écrites pendant cette exécution (les deux paliers).
    pub created: Vec<StoredRecommendation>,
    /// Candidats déjà présents dans le journal : rien n'a été réécrit.
    pub already_known: usize,
    /// Pour chaque match comparé à Pinnacle, le meilleur prix d'un opérateur, **même moins bon que le marché**, du plus proche
    /// de la valeur au plus éloigné. Ce n'est pas un journal : cela sert à montrer ce que l'algorithme a regardé.
    pub priced: Vec<Opportunity>,
}

impl Report {
    /// Les nouvelles recommandations à notifier.
    pub fn recommended(&self) -> impl Iterator<Item = &StoredRecommendation> {
        self.created
            .iter()
            .filter(|r| r.record.tier == Tier::Recommended)
    }

    /// Les nouvelles lignes du journal fantôme.
    pub fn shadow(&self) -> impl Iterator<Item = &StoredRecommendation> {
        self.created
            .iter()
            .filter(|r| r.record.tier == Tier::Shadow)
    }
}

/// Lit le dernier relevé et écrit les nouvelles recommandations dans le journal, en une seule transaction.
pub fn run(conn: &mut Connection, settings: &Settings, now: DateTime<Utc>) -> Result<Report> {
    // Seuil à −1 (= aucun) : le meilleur prix de chaque match, même négatif, pour pouvoir le montrer ; les deux paliers du
    // journal n'en retiennent ensuite que les avantages positifs.
    let scan = live::scan(
        conn,
        settings.method,
        -1.0,
        &settings.excluded_bookmakers,
        now,
    )?;
    let mut priced = scan.opportunities.clone();
    for opportunity in &mut priced {
        // Le scan a été fait sans seuil : la cote minimale qu'il a calculée n'a pas de sens, on la recalcule pour le seuil du jour.
        opportunity.minimal_odds =
            minimal_odds(opportunity.selection.probability, settings.min_edge);
    }
    priced.sort_by(|a, b| b.selection.edge.total_cmp(&a.selection.edge));

    let mut fresh_recommended: Vec<Opportunity> = Vec::new();
    let mut fresh_shadow: Vec<Opportunity> = Vec::new();
    let mut already_known = 0;
    for opportunity in scan.opportunities {
        if opportunity.selection.edge < 0.0 {
            continue;
        }
        let tier = if opportunity.selection.edge >= settings.min_edge {
            Tier::Recommended
        } else {
            Tier::Shadow
        };
        if recommendation_exists(conn, RULE, tier, &opportunity.event_id)? {
            already_known += 1;
            continue;
        }
        match tier {
            Tier::Recommended => fresh_recommended.push(opportunity),
            Tier::Shadow => fresh_shadow.push(opportunity),
        }
    }

    // Les nouvelles recommandations se partagent la même bankroll : Kelly simultané sur le lot.
    live::size_stakes(
        &mut fresh_recommended,
        settings.bankroll,
        settings.kelly_fraction,
        settings.min_stake,
    )?;

    let tx = conn.transaction()?;
    let mut created = Vec::new();
    let batches = [
        (Tier::Recommended, settings.min_edge, fresh_recommended),
        (Tier::Shadow, 0.0, fresh_shadow),
    ];
    for (tier, min_edge, opportunities) in batches {
        for opportunity in opportunities {
            let record = NewRecommendation {
                rule: RULE.to_string(),
                tier,
                event_id: opportunity.event_id,
                sport_key: opportunity.sport_key,
                home_team: opportunity.home_team,
                away_team: opportunity.away_team,
                kickoff: opportunity.kickoff,
                bookmaker: opportunity.bookmaker,
                outcome: opportunity.selection.outcome,
                odds: opportunity.selection.odds,
                probability: opportunity.selection.probability,
                edge: opportunity.selection.edge,
                min_edge,
                minimal_odds: minimal_odds(opportunity.selection.probability, min_edge),
                // Le journal fantôme ne mise rien : il observe.
                stake: if tier == Tier::Recommended {
                    opportunity.stake
                } else {
                    None
                },
                kelly_fraction: settings.kelly_fraction,
                bankroll: settings.bankroll,
                margin_method: settings.method.name().to_string(),
                api_call_id: opportunity.api_call_id,
                code_version: settings.code_version.clone(),
            };
            if let Some(id) = journal::insert_recommendation(&tx, &record, now)? {
                created.push(StoredRecommendation {
                    id,
                    created_at: now,
                    record,
                });
            } else {
                already_known += 1;
            }
        }
    }
    tx.commit()?;

    Ok(Report {
        events: scan.events,
        without_reference: scan.without_reference,
        implausible_quotes: scan.implausible_quotes,
        created,
        already_known,
        priced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::testing::{NOW, PINNACLE, at, database, event, event_named, store};
    use crate::domain::Outcome;

    fn settings() -> Settings {
        Settings {
            method: MarginMethod::Multiplicative,
            min_edge: RECOMMEND_MIN_EDGE,
            excluded_bookmakers: Vec::new(),
            kelly_fraction: 1.0,
            min_stake: Euros::new(1.0).unwrap(),
            bankroll: Euros::new(1000.0).unwrap(),
            code_version: "test".into(),
        }
    }

    #[test]
    fn a_large_edge_is_recommended_with_a_stake_and_a_small_one_goes_to_the_shadow_journal() {
        let mut conn = database();
        // Match 1 : +11,9 % chez Betclic (recommandé). Match 2 : environ +1,2 % chez PMU (fantôme).
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[
                event_named(
                    "big",
                    "2026-10-24T14:00:00Z",
                    &[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)],
                ),
                event_named(
                    "small",
                    "2026-10-25T14:00:00Z",
                    &[PINNACLE, ("pmu_fr", 2.08, 3.5, 3.8)],
                ),
            ],
        );

        let report = run(&mut conn, &settings(), at(NOW)).unwrap();

        assert_eq!(report.events, 2);
        let recommended: Vec<_> = report.recommended().collect();
        let shadow: Vec<_> = report.shadow().collect();
        assert_eq!(recommended.len(), 1);
        assert_eq!(shadow.len(), 1);

        let big = &recommended[0].record;
        assert_eq!(big.event_id, "big");
        assert_eq!(big.outcome, Outcome::Home);
        assert!(big.stake.is_some_and(|s| s.get() > 0.0));
        assert_eq!(big.min_edge, RECOMMEND_MIN_EDGE);
        // Cote minimale = 1,03 / p.
        assert!((big.minimal_odds - 1.03 / big.probability.get()).abs() < 1e-9);

        let small = &shadow[0].record;
        assert_eq!(small.event_id, "small");
        assert!(small.edge > 0.0 && small.edge < RECOMMEND_MIN_EDGE);
        assert_eq!(small.stake, None, "le journal fantôme ne mise rien");
        assert_eq!(small.min_edge, 0.0);
    }

    #[test]
    fn running_twice_writes_nothing_the_second_time() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );

        let first = run(&mut conn, &settings(), at(NOW)).unwrap();
        let second = run(&mut conn, &settings(), at("2026-10-18T16:00:00Z")).unwrap();

        assert_eq!(first.created.len(), 1);
        assert!(second.created.is_empty());
        assert_eq!(second.already_known, 1);
        assert_eq!(journal::recommendations(&conn).unwrap().len(), 1);
    }

    #[test]
    fn a_better_price_the_next_day_does_not_rewrite_the_first_recommendation() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );
        run(&mut conn, &settings(), at(NOW)).unwrap();
        store(
            &mut conn,
            "2026-10-19T14:00:00Z",
            &[event(&[PINNACLE, ("winamax_fr", 2.5, 3.2, 3.4)])],
        );

        let next = run(&mut conn, &settings(), at("2026-10-19T15:00:00Z")).unwrap();

        assert!(next.created.is_empty());
        let all = journal::recommendations(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].record.bookmaker, "betclic_fr");
    }

    #[test]
    fn the_stakes_of_a_batch_share_the_bankroll() {
        let mut conn = database();
        let books = [PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)];
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[
                event_named("a", "2026-10-24T14:00:00Z", &books),
                event_named("b", "2026-10-24T16:00:00Z", &books),
            ],
        );

        let report = run(&mut conn, &settings(), at(NOW)).unwrap();

        let stakes: Vec<f64> = report
            .recommended()
            .map(|r| r.record.stake.unwrap().get())
            .collect();
        assert_eq!(stakes.len(), 2);
        // Kelly d'un pari seul = 91,4 € ; deux paris simultanés se réduisent l'un l'autre mais restent sous la bankroll.
        assert!(stakes.iter().all(|s| *s < 91.4), "{stakes:?}");
        assert!(stakes.iter().sum::<f64>() < 1000.0);
    }

    #[test]
    fn a_stake_below_the_minimum_is_not_suggested_but_the_recommendation_is_kept() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );
        let mut tiny = settings();
        tiny.bankroll = Euros::new(5.0).unwrap();
        tiny.min_stake = Euros::new(1.0).unwrap();

        let report = run(&mut conn, &tiny, at(NOW)).unwrap();

        let recommended: Vec<_> = report.recommended().collect();
        assert_eq!(recommended.len(), 1);
        // 9,1 % de 5 € = 0,4 € < 1 €.
        assert_eq!(recommended[0].record.stake, None);
    }

    #[test]
    fn every_priced_match_is_reported_even_below_the_market_best_first_with_the_minimal_odds_of_the_day()
     {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[
                event_named(
                    "down",
                    "2026-10-24T14:00:00Z",
                    &[PINNACLE, ("betclic_fr", 1.95, 3.5, 3.9)],
                ),
                event_named(
                    "up",
                    "2026-10-25T14:00:00Z",
                    &[PINNACLE, ("pmu_fr", 2.08, 3.5, 3.8)],
                ),
            ],
        );

        let report = run(&mut conn, &settings(), at(NOW)).unwrap();

        let ids: Vec<&str> = report.priced.iter().map(|o| o.event_id.as_str()).collect();
        assert_eq!(
            ids,
            ["up", "down"],
            "du plus proche de la valeur au plus éloigné"
        );
        assert!(report.priced[1].selection.edge < 0.0);
        for o in &report.priced {
            assert!(
                (o.minimal_odds - (1.0 + RECOMMEND_MIN_EDGE) / o.selection.probability.get()).abs()
                    < 1e-9
            );
        }
        // Le journal, lui, n'a retenu que l'avantage positif.
        assert_eq!(report.created.len(), 1);
        assert_eq!(report.created[0].record.event_id, "up");
    }

    #[test]
    fn a_match_without_value_writes_nothing() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 1.95, 3.5, 3.9)])],
        );

        let report = run(&mut conn, &settings(), at(NOW)).unwrap();

        assert!(report.created.is_empty());
        assert_eq!(report.events, 1);
        assert!(journal::recommendations(&conn).unwrap().is_empty());
    }
}
