//! CLV réel : les recommandations et les paris du journal, confrontés à la clôture de Pinnacle.
//!
//! `CLV = cote obtenue × probabilité de clôture sans marge − 1` : l'espérance du pari si la clôture dit vrai. C'est la
//! mesure principale du projet (invariant 5 de CLAUDE.md), avec un intervalle de confiance par blocs de semaines.
//! Les paris hors modèle (`off_model`) sont rapportés à part des recommandations : on compare *mes* choix à ceux du modèle.

use chrono::{DateTime, Datelike, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::settle::result_of;
use crate::backtest::engine::Candidate;
use crate::backtest::metrics::{clv_ci, flat_roi_ci};
use crate::domain::{Odds, Odds1x2, Outcome, Probability, Probs1x2};
use crate::error::Result;
use crate::eval::bootstrap::MeanCi;
use crate::ingest::aliases::Aliases;
use crate::ingest::closing::WINDOW_MINUTES;
use crate::market::margin::{MarginMethod, remove_margin};
use crate::store::journal::{self, Settlement, Tier, timestamp};

/// Probabilités de clôture de Pinnacle sans marge, ou `None` si aucun relevé de Pinnacle n'a été pris dans les
/// 30 minutes avant le coup d'envoi (la clôture manque : le CLV de ce pari n'existe pas).
pub fn closing_fair(
    conn: &Connection,
    event_id: &str,
    kickoff: DateTime<Utc>,
    method: MarginMethod,
) -> Result<Option<Probs1x2>> {
    let since = timestamp(kickoff - Duration::minutes(WINDOW_MINUTES));
    // Le dernier relevé de Pinnacle dans la fenêtre : celui qui est le plus proche du coup d'envoi.
    let snapshot: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT e.api_call_id, e.home_team_raw, e.away_team_raw
             FROM event_seen e JOIN api_call a ON a.id = e.api_call_id
             WHERE e.event_id = ?1 AND a.called_at >= ?2
               AND EXISTS (SELECT 1 FROM quote q WHERE q.api_call_id = e.api_call_id AND q.event_id = e.event_id
                           AND q.bookmaker_key = 'pinnacle' AND q.market_key = 'h2h')
             ORDER BY e.api_call_id DESC LIMIT 1",
            params![event_id, since],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((call_id, home, away)) = snapshot else {
        return Ok(None);
    };
    let mut statement = conn.prepare(
        "SELECT outcome_raw, price FROM quote
         WHERE api_call_id = ?1 AND event_id = ?2 AND bookmaker_key = 'pinnacle' AND market_key = 'h2h'",
    )?;
    let rows = statement.query_map(params![call_id, event_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    let mut prices: [Option<Odds>; 3] = [None; 3];
    for row in rows {
        let (name, price) = row?;
        // Appariement par nom exact, comme pour le relevé quotidien (les issues arrivent dans l'ordre alphabétique).
        let outcome = if name == home {
            Outcome::Home
        } else if name == away {
            Outcome::Away
        } else if name == "Draw" {
            Outcome::Draw
        } else {
            continue;
        };
        prices[outcome.index()] = Odds::new(price);
    }
    let [Some(home), Some(draw), Some(away)] = prices else {
        return Ok(None);
    };
    Ok(remove_margin(&Odds1x2 { home, draw, away }, method))
}

/// Une ligne du tableau : un groupe de recommandations ou de paris.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Nom du groupe.
    pub label: &'static str,
    /// Éléments dans le groupe.
    pub n: usize,
    /// Éléments dont la clôture de Pinnacle est connue (donc un CLV).
    pub with_closing: usize,
    /// Éléments réglés (gagnés ou perdus ; les remboursés sont exclus du rendement).
    pub settled: usize,
    /// CLV moyen et son intervalle à 95 %.
    pub clv: Option<MeanCi>,
    /// Rendement moyen d'un euro misé à plat et son intervalle à 95 %.
    pub flat_roi: Option<MeanCi>,
    /// Pour les paris réels : `(mises réglées, gain net)` en euros.
    pub money: Option<(f64, f64)>,
}

/// Une tranche d'avantage annoncé (toutes recommandations confondues : notifiées et fantômes).
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket {
    /// Tranche, en texte.
    pub label: &'static str,
    /// Éléments dans la tranche.
    pub n: usize,
    /// Éléments avec clôture connue.
    pub with_closing: usize,
    /// Avantage annoncé moyen.
    pub mean_edge: f64,
    /// CLV moyen mesuré (sur ceux qui ont une clôture).
    pub mean_clv: Option<f64>,
}

/// CLV d'un sous-ensemble des recommandations (notifiées et fantômes) : un opérateur, ou une issue.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Split {
    /// Opérateur (`winamax_fr`) ou issue (`domicile`, `nul`, `extérieur`).
    pub key: String,
    /// Recommandations.
    pub n: usize,
    /// Recommandations dont la clôture de Pinnacle est connue.
    pub with_closing: usize,
    /// Avantage annoncé moyen.
    pub mean_edge: f64,
    /// CLV moyen et son intervalle à 95 %.
    pub clv: Option<MeanCi>,
}

/// Le rapport complet.
#[derive(Debug, Clone, PartialEq)]
pub struct ClvReport {
    /// Recommandations notifiées, journal fantôme, paris suivant une recommandation, paris hors modèle.
    pub groups: Vec<Group>,
    /// CLV par tranche d'avantage annoncé.
    pub buckets: Vec<Bucket>,
    /// CLV par opérateur français : chez lequel ouvrir un compte ?
    pub by_bookmaker: Vec<Split>,
    /// CLV par issue : le nul est-il trop payé, ou est-ce un artefact du retrait de marge ?
    pub by_outcome: Vec<Split>,
}

/// Tranches d'avantage : bornes basses (incluses) et nom.
const BUCKETS: [(f64, f64, &str); 5] = [
    (0.0, 0.01, "0 % à 1 %"),
    (0.01, 0.02, "1 % à 2 %"),
    (0.02, 0.03, "2 % à 3 %"),
    (0.03, 0.05, "3 % à 5 %"),
    (0.05, f64::INFINITY, "≥ 5 %"),
];

/// Un élément à analyser, tel que le journal le décrit.
struct Item {
    candidate: Candidate,
    /// Opérateur qui proposait le prix.
    bookmaker: String,
    /// `Some(gagné)` une fois réglé (gagné ou perdu) ; `None` si ouvert ou remboursé.
    won: Option<bool>,
    edge: Option<f64>,
    money: Option<(f64, f64)>,
}

/// Construit le rapport depuis le journal et l'archive.
pub fn report(conn: &Connection, aliases: &Aliases, method: MarginMethod) -> Result<ClvReport> {
    let mut notified = Vec::new();
    let mut shadow = Vec::new();
    let mut followed = Vec::new();
    let mut off_model = Vec::new();

    for stored in journal::recommendations(conn)? {
        let r = &stored.record;
        let close = closing_fair(conn, &r.event_id, r.kickoff, method)?;
        let won = result_of(
            conn,
            aliases,
            &r.sport_key,
            &r.home_team,
            &r.away_team,
            r.kickoff,
        )?
        .map(|actual| actual == r.outcome);
        let item = Item {
            candidate: candidate(
                stored.id,
                r.kickoff,
                r.outcome,
                r.odds,
                r.probability,
                close,
                won,
            ),
            bookmaker: r.bookmaker.clone(),
            won,
            edge: Some(r.edge),
            money: None,
        };
        match r.tier {
            Tier::Recommended => notified.push(item),
            Tier::Shadow => shadow.push(item),
        }
    }
    for stored in journal::bets(conn)? {
        let b = &stored.bet;
        let close = closing_fair(conn, &b.event_id, b.kickoff, method)?;
        let (won, money) = match &stored.result {
            Some(result) => (
                match result.settlement {
                    Settlement::Won => Some(true),
                    Settlement::Lost => Some(false),
                    Settlement::Void => None,
                },
                // Un pari remboursé a été misé puis rendu : il n'entre pas dans le total des mises réglées.
                (result.settlement != Settlement::Void).then_some((b.stake.get(), result.profit)),
            ),
            None => (None, None),
        };
        // Sans probabilité du modèle pour un pari libre, la probabilité de clôture (ou 0,5) sert de valeur neutre :
        // le champ n'intervient ni dans le CLV ni dans le rendement.
        let neutral = close.map_or(0.5, |p| p.get(b.outcome).get());
        let item = Item {
            candidate: candidate(
                stored.id,
                b.kickoff,
                b.outcome,
                b.odds,
                Probability::new(neutral).unwrap_or_else(unit),
                close,
                won,
            ),
            bookmaker: b.bookmaker.clone(),
            won,
            edge: None,
            money,
        };
        if b.recommendation_id.is_some() {
            followed.push(item);
        } else {
            off_model.push(item);
        }
    }

    let mut buckets = Vec::new();
    for (low, high, label) in BUCKETS {
        let in_bucket: Vec<&Item> = notified
            .iter()
            .chain(shadow.iter())
            .filter(|item| item.edge.is_some_and(|edge| edge >= low && edge < high))
            .collect();
        let clvs: Vec<f64> = in_bucket
            .iter()
            .filter_map(|item| clv_of(&item.candidate))
            .collect();
        let edges: Vec<f64> = in_bucket.iter().filter_map(|item| item.edge).collect();
        buckets.push(Bucket {
            label,
            n: in_bucket.len(),
            with_closing: clvs.len(),
            mean_edge: mean(&edges).unwrap_or(0.0),
            mean_clv: mean(&clvs),
        });
    }

    // Les deux paliers ensemble : le journal fantôme existe précisément pour mesurer le CLV sous le seuil.
    let recommendations: Vec<&Item> = notified.iter().chain(shadow.iter()).collect();
    let by_bookmaker = splits(&recommendations, |item| item.bookmaker.clone());
    let by_outcome = splits(&recommendations, |item| {
        outcome_label(item.candidate.outcome).to_string()
    });

    Ok(ClvReport {
        by_bookmaker,
        by_outcome,
        groups: vec![
            group("Recommandations notifiées", &notified),
            group("Journal fantôme (sous le seuil)", &shadow),
            group("Mes paris suivant une recommandation", &followed),
            group("Mes paris hors modèle", &off_model),
        ],
        buckets,
    })
}

fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Home => "domicile",
        Outcome::Draw => "nul",
        Outcome::Away => "extérieur",
    }
}

/// Regroupe les recommandations par clé (opérateur ou issue), dans l'ordre alphabétique des clés.
///
/// `key` est une fonction passée en paramètre (une fermeture) : le même regroupement sert aux deux découpages.
fn splits(items: &[&Item], key: impl Fn(&Item) -> String) -> Vec<Split> {
    let mut groups: std::collections::BTreeMap<String, Vec<&Item>> =
        std::collections::BTreeMap::new();
    for item in items {
        groups.entry(key(item)).or_default().push(item);
    }
    groups
        .into_iter()
        .map(|(key, members)| {
            // Clonés : `clv_ci` attend une tranche contiguë de candidats, que les éléments regroupés ne forment pas.
            let candidates: Vec<Candidate> = members.iter().map(|i| i.candidate.clone()).collect();
            let edges: Vec<f64> = members.iter().filter_map(|i| i.edge).collect();
            Split {
                key,
                n: members.len(),
                with_closing: candidates.iter().filter(|c| c.close_fair.is_some()).count(),
                mean_edge: mean(&edges).unwrap_or(0.0),
                clv: clv_ci(&candidates),
            }
        })
        .collect()
}

fn unit() -> Probability {
    // 0,5 est une probabilité valide : `new` ne peut pas échouer ici.
    Probability::new(0.5).expect("0,5 est une probabilité")
}

pub(crate) fn candidate(
    id: i64,
    kickoff: DateTime<Utc>,
    outcome: Outcome,
    odds: Odds,
    probability: Probability,
    close: Option<Probs1x2>,
    won: Option<bool>,
) -> Candidate {
    let date = kickoff.date_naive();
    Candidate {
        game_id: id,
        date,
        // Bloc de bootstrap : la semaine (les paris d'une même semaine ne sont pas indépendants).
        block: u32::try_from(date.num_days_from_ce() / 7).unwrap_or(0),
        outcome,
        odds,
        probability,
        close_fair: close.map(|c| c.get(outcome)),
        won: won.unwrap_or(false),
    }
}

fn clv_of(candidate: &Candidate) -> Option<f64> {
    candidate
        .close_fair
        .map(|close| candidate.odds.get() * close.get() - 1.0)
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn group(label: &'static str, items: &[Item]) -> Group {
    let all: Vec<Candidate> = items.iter().map(|i| i.candidate.clone()).collect();
    let settled: Vec<Candidate> = items
        .iter()
        .filter(|i| i.won.is_some())
        .map(|i| i.candidate.clone())
        .collect();
    let money: Vec<(f64, f64)> = items.iter().filter_map(|i| i.money).collect();
    Group {
        label,
        n: items.len(),
        with_closing: all.iter().filter(|c| c.close_fair.is_some()).count(),
        settled: settled.len(),
        clv: clv_ci(&all),
        flat_roi: flat_roi_ci(&settled),
        money: (!money.is_empty()).then(|| {
            (
                money.iter().map(|m| m.0).sum(),
                money.iter().map(|m| m.1).sum(),
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::testing::{PINNACLE, at, database, event_named, store};
    use crate::domain::Euros;
    use crate::store::journal::{
        NewBet, NewRecommendation, SettlementSource, bets, insert_bet, insert_recommendation,
        insert_result,
    };

    fn aliases() -> Aliases {
        Aliases::parse("[SC0]\n\"Falkirk\" = []\n\"Kilmarnock\" = []\n").unwrap()
    }

    fn recommendation(tier: Tier, event_id: &str, odds: f64, edge: f64) -> NewRecommendation {
        NewRecommendation {
            rule: "price_vs_pinnacle".into(),
            tier,
            event_id: event_id.into(),
            sport_key: "soccer_spl".into(),
            home_team: "Falkirk".into(),
            away_team: "Kilmarnock".into(),
            kickoff: at("2026-10-24T14:00:00Z"),
            bookmaker: "betclic_fr".into(),
            outcome: Outcome::Home,
            odds: Odds::new(odds).unwrap(),
            probability: Probability::new(0.4865).unwrap(),
            edge,
            min_edge: 0.0,
            minimal_odds: 2.0,
            stake: None,
            kelly_fraction: 1.0,
            bankroll: Euros::new(1000.0).unwrap(),
            margin_method: "multiplicative".into(),
            api_call_id: 1,
            code_version: "test".into(),
        }
    }

    /// Une base avec un relevé du matin (id 1) et la clôture de Pinnacle 20 minutes avant le coup d'envoi.
    fn with_closing() -> Connection {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-24T08:00:00Z",
            &[event_named("m1", "2026-10-24T14:00:00Z", &[PINNACLE])],
        );
        store(
            &mut conn,
            "2026-10-24T13:40:00Z",
            &[event_named("m1", "2026-10-24T14:00:00Z", &[PINNACLE])],
        );
        conn
    }

    #[test]
    fn the_closing_probabilities_come_from_the_snapshot_taken_in_the_last_30_minutes() {
        let conn = with_closing();

        let close = closing_fair(
            &conn,
            "m1",
            at("2026-10-24T14:00:00Z"),
            MarginMethod::Multiplicative,
        )
        .unwrap()
        .unwrap();

        // Pinnacle 2,00 / 3,60 / 4,00 : p_domicile = 0,5 / 1,02778.
        assert!((close.get(Outcome::Home).get() - 0.486_486).abs() < 1e-5);
    }

    #[test]
    fn the_morning_snapshot_alone_is_not_a_closing() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-24T08:00:00Z",
            &[event_named("m1", "2026-10-24T14:00:00Z", &[PINNACLE])],
        );

        let close = closing_fair(
            &conn,
            "m1",
            at("2026-10-24T14:00:00Z"),
            MarginMethod::Multiplicative,
        )
        .unwrap();

        assert!(close.is_none());
    }

    #[test]
    fn clv_is_the_obtained_odds_times_the_closing_probability_minus_one() {
        let conn = with_closing();
        insert_recommendation(
            &conn,
            &recommendation(Tier::Recommended, "m1", 2.3, 0.1189),
            at("2026-10-24T08:05:00Z"),
        )
        .unwrap();

        let report = report(&conn, &aliases(), MarginMethod::Multiplicative).unwrap();

        let notified = &report.groups[0];
        assert_eq!(
            (notified.n, notified.with_closing, notified.settled),
            (1, 1, 0)
        );
        // 2,30 × 0,486486 − 1 = +11,89 %.
        assert!((notified.clv.unwrap().mean - 0.118_92).abs() < 1e-4);
        assert!(notified.flat_roi.is_none(), "pas encore réglé");
    }

    #[test]
    fn the_buckets_group_notified_and_shadow_rows_by_announced_edge() {
        let mut conn = with_closing();
        store(
            &mut conn,
            "2026-10-24T13:41:00Z",
            &[event_named("m2", "2026-10-24T14:00:00Z", &[PINNACLE])],
        );
        insert_recommendation(
            &conn,
            &recommendation(Tier::Shadow, "m1", 2.05, 0.005),
            at("2026-10-24T08:05:00Z"),
        )
        .unwrap();
        insert_recommendation(
            &conn,
            &recommendation(Tier::Recommended, "m2", 2.3, 0.1189),
            at("2026-10-24T08:05:00Z"),
        )
        .unwrap();

        let report = report(&conn, &aliases(), MarginMethod::Multiplicative).unwrap();

        let by_label = |label: &str| {
            report
                .buckets
                .iter()
                .find(|b| b.label == label)
                .unwrap()
                .clone()
        };
        assert_eq!(by_label("0 % à 1 %").n, 1);
        assert_eq!(by_label("≥ 5 %").n, 1);
        assert_eq!(by_label("1 % à 2 %").n, 0);
        assert!((by_label("≥ 5 %").mean_clv.unwrap() - 0.118_92).abs() < 1e-4);
        // Les deux paliers comptent par opérateur et par issue (ici : Betclic, domicile).
        assert_eq!(report.by_bookmaker.len(), 1);
        assert_eq!(report.by_bookmaker[0].key, "betclic_fr");
        assert_eq!(report.by_bookmaker[0].n, 2);
        assert_eq!(report.by_outcome[0].key, "domicile");
        assert_eq!(report.by_outcome[0].with_closing, 2);
        // Le tableau par groupe sépare bien les deux paliers.
        assert_eq!(report.groups[0].n, 1);
        assert_eq!(report.groups[1].n, 1);
    }

    #[test]
    fn bets_are_split_between_followed_recommendations_and_off_model_and_count_real_money() {
        let conn = with_closing();
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'Falkirk'), (2, 'Kilmarnock');
             INSERT INTO game (div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
             VALUES ('SC0', '2026/27', '2026-10-24', 1, 2, 2, 1, 'H');",
        )
        .unwrap();
        let rec = insert_recommendation(
            &conn,
            &recommendation(Tier::Recommended, "m1", 2.3, 0.1189),
            at("2026-10-24T08:05:00Z"),
        )
        .unwrap()
        .unwrap();
        let bet = |recommendation_id, outcome| NewBet {
            recommendation_id,
            event_id: "m1".into(),
            sport_key: "soccer_spl".into(),
            home_team: "Falkirk".into(),
            away_team: "Kilmarnock".into(),
            kickoff: at("2026-10-24T14:00:00Z"),
            bookmaker: "betclic_fr".into(),
            outcome,
            odds: Odds::new(2.2).unwrap(),
            stake: Euros::new(50.0).unwrap(),
        };
        insert_bet(
            &conn,
            &bet(Some(rec), Outcome::Home),
            at("2026-10-24T09:00:00Z"),
        )
        .unwrap();
        insert_bet(&conn, &bet(None, Outcome::Draw), at("2026-10-24T09:00:00Z")).unwrap();
        for stored in bets(&conn).unwrap() {
            let settlement = if stored.bet.outcome == Outcome::Home {
                Settlement::Won
            } else {
                Settlement::Lost
            };
            insert_result(
                &conn,
                &stored,
                settlement,
                SettlementSource::FootballData,
                at("2026-10-26T09:00:00Z"),
            )
            .unwrap();
        }

        let report = report(&conn, &aliases(), MarginMethod::Multiplicative).unwrap();

        let followed = &report.groups[2];
        let off_model = &report.groups[3];
        assert_eq!((followed.n, off_model.n), (1, 1));
        let (staked, profit) = followed.money.unwrap();
        assert_eq!(staked, 50.0);
        assert!((profit - 60.0).abs() < 1e-9);
        assert_eq!(off_model.money, Some((50.0, -50.0)));
        // La recommandation elle-même est réglée via l'historique : gagnée, rendement +130 %.
        assert_eq!(report.groups[0].settled, 1);
        assert!((report.groups[0].flat_roi.unwrap().mean - 1.3).abs() < 1e-9);
        // Le CLV du pari suivi : 2,20 × 0,486486 − 1 = +7,03 %.
        assert!((followed.clv.unwrap().mean - 0.070_27).abs() < 1e-4);
    }

    #[test]
    fn an_empty_journal_gives_empty_groups() {
        let conn = database();

        let report = report(&conn, &aliases(), MarginMethod::Multiplicative).unwrap();

        assert!(report.groups.iter().all(|g| g.n == 0 && g.clv.is_none()));
        assert!(
            report
                .buckets
                .iter()
                .all(|b| b.n == 0 && b.mean_clv.is_none())
        );
    }
}
