//! Séries et diagnostics pour l'application : courbe de bankroll et diagramme de fiabilité des recommandations.
//!
//! Des fonctions pures sur le journal : rien n'est écrit, et rien ne dépend de l'interface qui les affiche.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::settle::result_of;
use crate::backtest::metrics::max_drawdown;
use crate::domain::Euros;
use crate::error::Result;
use crate::ingest::aliases::Aliases;
use crate::store::journal;

/// Un point de la courbe : la bankroll **réalisée** juste après un règlement.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvePoint {
    /// Instant du règlement (UTC).
    pub at: DateTime<Utc>,
    /// Bankroll initiale + gains nets réglés jusque-là.
    pub bankroll: f64,
}

/// La bankroll dans le temps, et ses indicateurs.
#[derive(Debug, Clone, PartialEq)]
pub struct BankrollCurve {
    /// Bankroll de départ.
    pub initial: f64,
    /// Un point par pari réglé (remboursés compris : ils ne changent pas la bankroll), par date de règlement.
    pub points: Vec<CurvePoint>,
    /// Gain net total des paris réglés.
    pub settled_profit: f64,
    /// Mises des paris encore ouverts : déjà sorties de la bankroll disponible.
    pub open_stakes: f64,
    /// Bankroll disponible : départ + gains réglés − mises ouvertes.
    pub available: f64,
    /// Plus forte baisse relative de la bankroll réalisée depuis un sommet.
    pub max_drawdown: f64,
}

/// Calcule la courbe de bankroll depuis les paris réels et leurs règlements.
pub fn bankroll_curve(conn: &Connection, initial: Euros) -> Result<BankrollCurve> {
    let mut settled: Vec<(DateTime<Utc>, f64)> = Vec::new();
    let mut open_stakes = 0.0;
    for stored in journal::bets(conn)? {
        match stored.result {
            Some(result) => settled.push((result.settled_at, result.profit)),
            None => open_stakes += stored.bet.stake.get(),
        }
    }
    // Tri stable par date de règlement : deux règlements simultanés gardent l'ordre d'enregistrement.
    settled.sort_by_key(|(at, _)| *at);

    let mut bankroll = initial.get();
    let mut points = Vec::with_capacity(settled.len());
    for (at, profit) in &settled {
        bankroll += profit;
        points.push(CurvePoint { at: *at, bankroll });
    }
    let settled_profit = bankroll - initial.get();
    Ok(BankrollCurve {
        initial: initial.get(),
        max_drawdown: max_drawdown(initial.get(), points.iter().map(|p| p.bankroll)),
        points,
        settled_profit,
        open_stakes,
        available: (bankroll - open_stakes).max(0.0),
    })
}

/// Une tranche du diagramme de fiabilité : la probabilité annoncée contre la fréquence observée.
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityBin {
    /// Borne basse (incluse) de la probabilité annoncée.
    pub low: f64,
    /// Borne haute (exclue, sauf pour la dernière tranche).
    pub high: f64,
    /// Recommandations réglées dans la tranche.
    pub n: usize,
    /// Probabilité annoncée moyenne.
    pub mean_probability: f64,
    /// Fréquence de victoire observée.
    pub frequency: f64,
}

/// Diagramme de fiabilité des recommandations réglées (les deux paliers) : la probabilité de Pinnacle sans marge
/// annoncée pour l'issue recommandée, contre la fréquence à laquelle elle s'est produite. Seules les tranches non vides sont rendues.
pub fn reliability(
    conn: &Connection,
    aliases: &Aliases,
    bins: usize,
) -> Result<Vec<ReliabilityBin>> {
    let bins = bins.max(1);
    // (somme des probabilités, victoires, effectif) par tranche.
    let mut totals = vec![(0.0_f64, 0usize, 0usize); bins];
    for stored in journal::recommendations(conn)? {
        let r = &stored.record;
        let Some(actual) = result_of(
            conn,
            aliases,
            &r.sport_key,
            &r.home_team,
            &r.away_team,
            r.kickoff,
        )?
        else {
            continue;
        };
        let p = r.probability.get();
        // La probabilité 1,0 va dans la dernière tranche plutôt que dans une tranche de plus.
        let index = ((p * bins as f64) as usize).min(bins - 1);
        let slot = &mut totals[index];
        slot.0 += p;
        slot.1 += usize::from(actual == r.outcome);
        slot.2 += 1;
    }
    Ok(totals
        .into_iter()
        .enumerate()
        .filter(|(_, (_, _, n))| *n > 0)
        .map(|(index, (sum, wins, n))| ReliabilityBin {
            low: index as f64 / bins as f64,
            high: (index + 1) as f64 / bins as f64,
            n,
            mean_probability: sum / n as f64,
            frequency: wins as f64 / n as f64,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::testing::{at, database};
    use crate::domain::{Odds, Outcome, Probability};
    use crate::store::journal::{
        NewBet, NewRecommendation, Settlement, SettlementSource, Tier, bets, insert_bet,
        insert_recommendation, insert_result,
    };

    fn bet(stake: f64) -> NewBet {
        NewBet {
            recommendation_id: None,
            event_id: "m1".into(),
            sport_key: "soccer_spl".into(),
            home_team: "Falkirk".into(),
            away_team: "Kilmarnock".into(),
            kickoff: at("2026-10-24T14:00:00Z"),
            bookmaker: "betclic_fr".into(),
            outcome: Outcome::Home,
            odds: Odds::new(2.0).unwrap(),
            stake: Euros::new(stake).unwrap(),
        }
    }

    #[test]
    fn without_any_bet_the_curve_is_flat_at_the_initial_bankroll() {
        let conn = database();

        let curve = bankroll_curve(&conn, Euros::new(1000.0).unwrap()).unwrap();

        assert!(curve.points.is_empty());
        assert_eq!(
            (curve.available, curve.settled_profit, curve.max_drawdown),
            (1000.0, 0.0, 0.0)
        );
    }

    #[test]
    fn the_curve_follows_settlements_in_date_order_and_counts_open_stakes() {
        let conn = database();
        for stake in [100.0, 100.0, 100.0, 50.0] {
            insert_bet(&conn, &bet(stake), at("2026-10-24T09:00:00Z")).unwrap();
        }
        let all = bets(&conn).unwrap();
        // Règlements saisis dans le désordre : le deuxième pari (perdu) est réglé AVANT le premier (gagné).
        insert_result(
            &conn,
            &all[1],
            Settlement::Lost,
            SettlementSource::Manual,
            at("2026-10-25T10:00:00Z"),
        )
        .unwrap();
        insert_result(
            &conn,
            &all[0],
            Settlement::Won,
            SettlementSource::Manual,
            at("2026-10-26T10:00:00Z"),
        )
        .unwrap();
        insert_result(
            &conn,
            &all[2],
            Settlement::Void,
            SettlementSource::Manual,
            at("2026-10-27T10:00:00Z"),
        )
        .unwrap();
        // all[3] reste ouvert (50 €).

        let curve = bankroll_curve(&conn, Euros::new(1000.0).unwrap()).unwrap();

        let values: Vec<f64> = curve.points.iter().map(|p| p.bankroll).collect();
        assert_eq!(values, vec![900.0, 1000.0, 1000.0]);
        assert_eq!(curve.settled_profit, 0.0);
        assert_eq!(curve.open_stakes, 50.0);
        assert_eq!(curve.available, 950.0);
        // Sommet initial 1 000 → 900 : baisse de 10 %.
        assert!((curve.max_drawdown - 0.10).abs() < 1e-12);
    }

    fn recommendation(event_id: &str, probability: f64, outcome: Outcome) -> NewRecommendation {
        NewRecommendation {
            rule: "price_vs_pinnacle".into(),
            tier: Tier::Recommended,
            event_id: event_id.into(),
            sport_key: "soccer_spl".into(),
            home_team: "Falkirk".into(),
            away_team: "Kilmarnock".into(),
            kickoff: at("2026-10-24T14:00:00Z"),
            bookmaker: "betclic_fr".into(),
            outcome,
            odds: Odds::new(2.3).unwrap(),
            probability: Probability::new(probability).unwrap(),
            edge: 0.05,
            min_edge: 0.03,
            minimal_odds: 2.1,
            stake: None,
            kelly_fraction: 1.0,
            bankroll: Euros::new(1000.0).unwrap(),
            margin_method: "power".into(),
            api_call_id: 1,
            code_version: "test".into(),
        }
    }

    #[test]
    fn the_reliability_diagram_bins_settled_recommendations_by_announced_probability() {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO api_call (id, called_at, endpoint, params, http_status) VALUES (1, '2026-10-24T10:00:00Z', 'odds', '', 200);
             INSERT INTO team (id, name) VALUES (1, 'Falkirk'), (2, 'Kilmarnock');
             INSERT INTO game (div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
             VALUES ('SC0', '2026/27', '2026-10-24', 1, 2, 2, 1, 'H');",
        )
        .unwrap();
        let aliases = Aliases::parse("[SC0]\n\"Falkirk\" = []\n\"Kilmarnock\" = []\n").unwrap();
        // Match gagné par l'équipe à domicile. Trois recommandations (une par match fictif) :
        //  - victoire à domicile annoncée à 0,45 → gagnée ;  - nul annoncé à 0,25 → perdu ;  - extérieur annoncé à 0,27 → perdu.
        for (event, p, outcome) in [
            ("a", 0.45, Outcome::Home),
            ("b", 0.25, Outcome::Draw),
            ("c", 0.27, Outcome::Away),
        ] {
            insert_recommendation(
                &conn,
                &recommendation(event, p, outcome),
                at("2026-10-24T10:05:00Z"),
            )
            .unwrap();
        }
        // Un pari encore sans résultat dans l'historique (autre date) n'entre pas dans le diagramme.
        let mut later = recommendation("d", 0.5, Outcome::Home);
        later.kickoff = at("2026-12-01T14:00:00Z");
        insert_recommendation(&conn, &later, at("2026-10-24T10:05:00Z")).unwrap();

        let bins = reliability(&conn, &aliases, 5).unwrap();

        // Tranches de 0,2 : [0,2 ; 0,4) contient 0,25 et 0,27 (2 perdus) ; [0,4 ; 0,6) contient 0,45 (1 gagné).
        assert_eq!(bins.len(), 2);
        assert_eq!((bins[0].n, bins[0].frequency), (2, 0.0));
        assert!((bins[0].mean_probability - 0.26).abs() < 1e-12);
        assert_eq!((bins[1].n, bins[1].frequency), (1, 1.0));
        assert!((bins[1].low - 0.4).abs() < 1e-12 && (bins[1].high - 0.6).abs() < 1e-12);
    }

    #[test]
    fn a_probability_of_one_falls_in_the_last_bin() {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO api_call (id, called_at, endpoint, params, http_status) VALUES (1, '2026-10-24T10:00:00Z', 'odds', '', 200);
             INSERT INTO team (id, name) VALUES (1, 'Falkirk'), (2, 'Kilmarnock');
             INSERT INTO game (div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
             VALUES ('SC0', '2026/27', '2026-10-24', 1, 2, 2, 1, 'H');",
        )
        .unwrap();
        let aliases = Aliases::parse("[SC0]\n\"Falkirk\" = []\n\"Kilmarnock\" = []\n").unwrap();
        insert_recommendation(
            &conn,
            &recommendation("a", 1.0, Outcome::Home),
            at("2026-10-24T10:05:00Z"),
        )
        .unwrap();

        let bins = reliability(&conn, &aliases, 10).unwrap();

        assert_eq!(bins.len(), 1);
        assert!((bins[0].high - 1.0).abs() < 1e-12);
    }
}
