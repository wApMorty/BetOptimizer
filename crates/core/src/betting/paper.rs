//! Paper trading : les recommandations notifiées **et misées**, suivies comme si chacune avait été jouée.
//!
//! N'entrent que les paris que Paul jouerait : palier « notifiée », mise suggérée présente, avantage positif. Le journal
//! fantôme et les recommandations sous la mise minimale n'apparaissent pas et ne pèsent pas sur le rendement. Les mises
//! sont en unités (1 u = 1 % de la bankroll du jour de la recommandation) : les paris réels n'interviennent pas ici.

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::clv::{candidate, closing_fair};
use super::kelly::to_units;
use super::settle::result_of;
use crate::backtest::engine::Candidate;
use crate::backtest::metrics::{clv_ci, flat_roi_ci};
use crate::error::Result;
use crate::eval::bootstrap::MeanCi;
use crate::ingest::aliases::Aliases;
use crate::market::margin::MarginMethod;
use crate::store::journal::{self, StoredRecommendation, Tier};

/// Où en est un pari fictif.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Le match n'a pas commencé.
    Upcoming,
    /// Le match a commencé, son résultat n'est pas encore dans l'historique de football-data.
    Pending,
    /// Gagné.
    Won,
    /// Perdu.
    Lost,
}

/// Une recommandation suivie.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperBet {
    /// La recommandation telle que le journal l'a enregistrée.
    pub recommendation: StoredRecommendation,
    /// Mise, en unités.
    pub stake_units: f64,
    /// État.
    pub status: Status,
    /// Gain net en unités, une fois réglé.
    pub profit_units: Option<f64>,
    /// CLV contre la clôture de Pinnacle, si elle a été relevée.
    pub clv: Option<f64>,
}

/// Le bilan du paper trading.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperTrading {
    /// Les paris suivis, dans l'ordre des coups d'envoi.
    pub bets: Vec<PaperBet>,
    /// Somme des mises des paris réglés, en unités.
    pub staked_units: f64,
    /// Gain net des paris réglés, en unités.
    pub profit_units: f64,
    /// Rendement des mises Kelly : gain net / mises réglées. `None` tant que rien n'est réglé.
    pub roi: Option<f64>,
    /// Rendement d'une mise fixe par pari, avec son intervalle à 95 % (bootstrap par blocs de semaines).
    pub flat_roi: Option<MeanCi>,
    /// CLV moyen et son intervalle à 95 %, sur les paris dont la clôture est connue.
    pub clv: Option<MeanCi>,
    /// Paris dont la clôture de Pinnacle est connue.
    pub with_closing: usize,
    /// Gain cumulé (en unités) après chaque pari réglé, dans l'ordre des coups d'envoi.
    pub curve: Vec<f64>,
    /// Plus forte baisse du gain cumulé depuis un sommet, en unités (le départ, à 0, compte comme sommet).
    pub max_drawdown_units: f64,
}

impl PaperTrading {
    /// Nombre de paris dans un état donné.
    pub fn count(&self, status: Status) -> usize {
        self.bets.iter().filter(|bet| bet.status == status).count()
    }
}

/// La recommandation aurait-elle été jouée ? Notifiée, avec une mise et un avantage positif.
pub fn is_played(stored: &StoredRecommendation) -> bool {
    let r = &stored.record;
    r.tier == Tier::Recommended && r.stake.is_some() && r.edge > 0.0
}

/// Construit le bilan depuis le journal, l'archive des cotes (clôtures) et l'historique (résultats).
pub fn paper_trading(
    conn: &Connection,
    aliases: &Aliases,
    method: MarginMethod,
    now: DateTime<Utc>,
) -> Result<PaperTrading> {
    let mut bets = Vec::new();
    let mut candidates = Vec::new();
    // `into_iter` consomme le vecteur lu : chaque recommandation est déplacée dans son `PaperBet`, sans copie.
    for stored in journal::recommendations(conn)?
        .into_iter()
        .filter(is_played)
    {
        let r = &stored.record;
        let close = closing_fair(conn, &r.event_id, r.kickoff, method)?;
        let actual = result_of(
            conn,
            aliases,
            &r.sport_key,
            &r.home_team,
            &r.away_team,
            r.kickoff,
        )?;
        let status = match actual {
            Some(outcome) if outcome == r.outcome => Status::Won,
            Some(_) => Status::Lost,
            None if r.kickoff > now => Status::Upcoming,
            None => Status::Pending,
        };
        // `is_played` a vérifié la présence de la mise ; `map_or` évite pourtant de le redémontrer par un `expect`.
        let stake_units = r
            .stake
            .map_or(0.0, |s| to_units(s.get() / r.bankroll.get()));
        // Gain d'un pari à la cote décimale `c` : `mise × (c − 1)` s'il gagne, `−mise` s'il perd.
        let profit_units = match status {
            Status::Won => Some(stake_units * (r.odds.get() - 1.0)),
            Status::Lost => Some(-stake_units),
            Status::Upcoming | Status::Pending => None,
        };
        let won = matches!(status, Status::Won | Status::Lost).then_some(status == Status::Won);
        let entry = candidate(
            stored.id,
            r.kickoff,
            r.outcome,
            r.odds,
            r.probability,
            close,
            won,
        );
        // CLV = cote obtenue × probabilité de clôture sans marge − 1 (voir `clv`).
        let clv = entry.close_fair.map(|p| r.odds.get() * p.get() - 1.0);
        candidates.push((entry, won.is_some()));
        bets.push(PaperBet {
            recommendation: stored,
            stake_units,
            status,
            profit_units,
            clv,
        });
    }
    // Tri stable : deux matchs au même coup d'envoi gardent l'ordre du journal.
    bets.sort_by_key(|bet| bet.recommendation.record.kickoff);

    let profits: Vec<f64> = bets.iter().filter_map(|bet| bet.profit_units).collect();
    // `fold` partant de 0,0 plutôt que `sum` : la somme vide d'un `sum::<f64>()` vaut −0,0, affiché « -0,0 u ».
    let staked_units = bets
        .iter()
        .filter(|bet| bet.profit_units.is_some())
        .fold(0.0, |total, bet| total + bet.stake_units);
    let profit_units = profits.iter().fold(0.0, |total, profit| total + profit);
    let curve = cumulative(&profits);
    let all: Vec<Candidate> = candidates.iter().map(|(c, _)| c.clone()).collect();
    let settled: Vec<Candidate> = candidates
        .into_iter()
        .filter(|(_, settled)| *settled)
        .map(|(c, _)| c)
        .collect();
    Ok(PaperTrading {
        max_drawdown_units: max_drawdown_units(&curve),
        curve,
        staked_units,
        profit_units,
        roi: (staked_units > 0.0).then(|| profit_units / staked_units),
        flat_roi: flat_roi_ci(&settled),
        clv: clv_ci(&all),
        with_closing: all.iter().filter(|c| c.close_fair.is_some()).count(),
        bets,
    })
}

/// Sommes cumulées : `[1, -2, 3]` → `[1, -1, 2]`.
fn cumulative(values: &[f64]) -> Vec<f64> {
    // `scan` porte un accumulateur d'un élément au suivant et rend chaque état intermédiaire.
    values
        .iter()
        .scan(0.0, |total, value| {
            *total += value;
            Some(*total)
        })
        .collect()
}

/// Plus forte baisse depuis un sommet : `max_t (max_{s ≤ t} G_s − G_t)`, avec `G_0 = 0`. En unités, pas en pourcentage :
/// la courbe est un gain cumulé, pas une bankroll (chaque mise est un pourcentage d'une bankroll différente).
fn max_drawdown_units(curve: &[f64]) -> f64 {
    let mut peak: f64 = 0.0;
    let mut worst: f64 = 0.0;
    for value in curve {
        peak = peak.max(*value);
        worst = worst.max(peak - value);
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::testing::{PINNACLE, at, database, event_named, store};
    use crate::domain::{Euros, Odds, Outcome, Probability};
    use crate::store::journal::{NewRecommendation, insert_recommendation};
    use approx::assert_relative_eq;

    const NOW: &str = "2026-11-05T12:00:00Z";

    fn aliases() -> Aliases {
        Aliases::parse("[SC0]\n\"Falkirk\" = []\n\"Kilmarnock\" = []\n").unwrap()
    }

    /// Une recommandation sur Falkirk – Kilmarnock : 2 € misés sur 1000 € de bankroll, soit 0,2 u.
    fn recommendation(
        tier: Tier,
        event_id: &str,
        kickoff: &str,
        outcome: Outcome,
        stake: Option<f64>,
    ) -> NewRecommendation {
        NewRecommendation {
            rule: "price_vs_pinnacle".into(),
            tier,
            event_id: event_id.into(),
            sport_key: "soccer_spl".into(),
            home_team: "Falkirk".into(),
            away_team: "Kilmarnock".into(),
            kickoff: at(kickoff),
            bookmaker: "betclic_fr".into(),
            outcome,
            odds: Odds::new(2.3).unwrap(),
            probability: Probability::new(0.4865).unwrap(),
            edge: 0.1189,
            min_edge: 0.01,
            minimal_odds: 2.08,
            stake: stake.map(|s| Euros::new(s).unwrap()),
            kelly_fraction: 0.5,
            bankroll: Euros::new(1000.0).unwrap(),
            margin_method: "power".into(),
            api_call_id: 1,
            code_version: "test".into(),
        }
    }

    /// Deux matchs joués (24/10 : victoire à domicile ; 31/10 : victoire à l'extérieur) et un relevé du matin.
    fn with_results() -> Connection {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-24T08:00:00Z",
            &[event_named("m1", "2026-10-24T14:00:00Z", &[PINNACLE])],
        );
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'Falkirk'), (2, 'Kilmarnock');
             INSERT INTO game (div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result) VALUES
                 ('SC0', '2026/27', '2026-10-24', 1, 2, 2, 1, 'H'),
                 ('SC0', '2026/27', '2026-10-31', 1, 2, 0, 1, 'A');",
        )
        .unwrap();
        conn
    }

    fn insert(conn: &Connection, new: &NewRecommendation) {
        insert_recommendation(conn, new, at("2026-10-20T12:10:00Z"))
            .unwrap()
            .unwrap();
    }

    #[test]
    fn won_lost_pending_and_upcoming_recommendations_are_told_apart() {
        let conn = with_results();
        let home = Outcome::Home;
        insert(
            &conn,
            &recommendation(
                Tier::Recommended,
                "m1",
                "2026-10-24T14:00:00Z",
                home,
                Some(20.0),
            ),
        );
        insert(
            &conn,
            &recommendation(
                Tier::Recommended,
                "m2",
                "2026-10-31T15:00:00Z",
                home,
                Some(10.0),
            ),
        );
        insert(
            &conn,
            &recommendation(
                Tier::Recommended,
                "m3",
                "2026-11-03T15:00:00Z",
                home,
                Some(10.0),
            ),
        );
        insert(
            &conn,
            &recommendation(
                Tier::Recommended,
                "m4",
                "2026-11-08T15:00:00Z",
                home,
                Some(10.0),
            ),
        );

        let paper =
            paper_trading(&conn, &aliases(), MarginMethod::Multiplicative, at(NOW)).unwrap();

        let statuses: Vec<Status> = paper.bets.iter().map(|b| b.status).collect();
        assert_eq!(
            statuses,
            [Status::Won, Status::Lost, Status::Pending, Status::Upcoming]
        );
        // 20 € sur 1000 € = 2 u, gagné à 2,30 : +2,6 u. 10 € = 1 u, perdu : −1 u.
        assert_relative_eq!(paper.bets[0].stake_units, 2.0, epsilon = 1e-12);
        assert_relative_eq!(paper.bets[0].profit_units.unwrap(), 2.6, epsilon = 1e-12);
        assert_relative_eq!(paper.bets[1].profit_units.unwrap(), -1.0, epsilon = 1e-12);
        assert_eq!(paper.bets[2].profit_units, None);
        assert_relative_eq!(paper.staked_units, 3.0, epsilon = 1e-12);
        assert_relative_eq!(paper.profit_units, 1.6, epsilon = 1e-12);
        assert_relative_eq!(paper.roi.unwrap(), 1.6 / 3.0, epsilon = 1e-12);
        assert_eq!(paper.curve.len(), 2, "un point par pari réglé");
        assert_relative_eq!(paper.curve[1], 1.6, epsilon = 1e-12);
        assert_eq!(paper.count(Status::Upcoming), 1);
    }

    #[test]
    fn shadow_rows_and_recommendations_without_a_stake_are_neither_listed_nor_counted() {
        let conn = with_results();
        let home = Outcome::Home;
        insert(
            &conn,
            &recommendation(Tier::Recommended, "m1", "2026-10-24T14:00:00Z", home, None),
        );
        insert(
            &conn,
            &recommendation(
                Tier::Shadow,
                "m2",
                "2026-10-31T15:00:00Z",
                Outcome::Away,
                None,
            ),
        );

        let paper =
            paper_trading(&conn, &aliases(), MarginMethod::Multiplicative, at(NOW)).unwrap();

        assert!(paper.bets.is_empty());
        assert!(paper.profit_units.is_sign_positive(), "0,0 et non −0,0");
        assert_eq!(paper.roi, None);
        assert_eq!(paper.flat_roi, None);
        assert_eq!(paper.staked_units, 0.0);
    }

    #[test]
    fn the_clv_uses_the_pinnacle_closing_when_it_was_recorded() {
        let mut conn = with_results();
        store(
            &mut conn,
            "2026-10-24T13:40:00Z",
            &[event_named("m1", "2026-10-24T14:00:00Z", &[PINNACLE])],
        );
        insert(
            &conn,
            &recommendation(
                Tier::Recommended,
                "m1",
                "2026-10-24T14:00:00Z",
                Outcome::Home,
                Some(20.0),
            ),
        );

        let paper =
            paper_trading(&conn, &aliases(), MarginMethod::Multiplicative, at(NOW)).unwrap();

        // 2,30 × 0,486486 − 1 = +11,89 % (même calcul que `clv`).
        assert_relative_eq!(paper.bets[0].clv.unwrap(), 0.118_92, epsilon = 1e-4);
        assert_eq!(paper.with_closing, 1);
    }

    #[test]
    fn the_drawdown_is_measured_from_the_highest_point_reached_including_the_start() {
        assert_eq!(cumulative(&[1.0, -2.0, 3.0]), [1.0, -1.0, 2.0]);
        assert_relative_eq!(max_drawdown_units(&[1.0, -1.0, 2.0]), 2.0);
        assert_relative_eq!(max_drawdown_units(&[-0.5, -1.5, -1.0]), 1.5);
        assert_eq!(max_drawdown_units(&[]), 0.0);
    }
}
