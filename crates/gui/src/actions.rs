//! Les actions que l'utilisateur déclenche. Les onglets les *décrivent*, l'application les *exécute* sur le journal :
//! l'affichage ne touche jamais la base directement.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use edge_core::betting::ledger::{self, FollowedInput};
use edge_core::betting::settle;
use edge_core::bonus::operator_name;
use edge_core::bonus::tracking::{self, OperationRequest};
use edge_core::domain::Outcome;
use edge_core::format;
use edge_core::ingest::aliases::Aliases;
use edge_core::paths::Paths;
use edge_core::store::bonus::{self, MatchResult, NewOffer};
use edge_core::store::journal::{self, Settlement, SettlementSource};
use edge_core::store::open_migrated;

/// Une demande de l'utilisateur.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Relire le journal et l'archive.
    Refresh,
    /// Enregistrer un pari qui suit une recommandation.
    LogFollowed {
        /// Recommandation suivie.
        recommendation_id: i64,
        /// Valeurs saisies (l'absence reprend la recommandation).
        input: FollowedInput,
    },
    /// Enregistrer un pari hors modèle.
    LogOffModel {
        /// Match de l'archive.
        event_id: String,
        /// Bookmaker.
        bookmaker: String,
        /// Issue jouée.
        outcome: Outcome,
        /// Cote obtenue.
        odds: f64,
        /// Mise.
        stake: f64,
    },
    /// Régler les paris ouverts depuis l'historique football-data.
    SettleAuto,
    /// Régler un pari à la main.
    SettleManual {
        /// Numéro du pari.
        bet_id: i64,
        /// Résultat saisi.
        settlement: Settlement,
    },
    /// Ouvrir un rapport HTML dans le navigateur (traité par l'application, sans la base).
    OpenReport(PathBuf),
    /// Enregistrer une opération bonus jouée.
    LogBonus(OperationRequest),
    /// Enregistrer l'offre d'un opérateur.
    RecordOffer(NewOffer),
    /// Régler les opérations bonus ouvertes depuis l'historique football-data.
    SettleBonusAuto,
    /// Régler une opération bonus à la main.
    SettleBonusManual {
        /// Numéro de l'opération.
        operation_id: i64,
        /// Résultat du match.
        result: MatchResult,
    },
}

/// Exécute une action qui touche le journal et renvoie le message de succès à afficher.
pub fn apply(paths: &Paths, action: &Action, now: DateTime<Utc>) -> Result<String> {
    // `mut` : enregistrer une opération bonus ouvre une transaction, qui demande un accès exclusif à la connexion.
    let mut conn = open_migrated(&paths.db_file).context("base inaccessible")?;
    match action {
        Action::Refresh | Action::OpenReport(_) => Ok(String::new()),
        Action::LogFollowed {
            recommendation_id,
            input,
        } => {
            let bet = ledger::followed_bet(&conn, *recommendation_id, input)?;
            let id = ledger::record(&conn, &bet, now)?;
            Ok(format!(
                "Pari n° {id} enregistré : {} – {}, {} à {}, mise {} €.",
                bet.home_team,
                bet.away_team,
                format::outcome_label(bet.outcome),
                format::decimal(bet.odds.get(), 2),
                format::decimal(bet.stake.get(), 1)
            ))
        }
        Action::LogOffModel {
            event_id,
            bookmaker,
            outcome,
            odds,
            stake,
        } => {
            let bet = ledger::off_model_bet(&conn, event_id, bookmaker, *outcome, *odds, *stake)?;
            let id = ledger::record(&conn, &bet, now)?;
            Ok(format!(
                "Pari n° {id} enregistré (hors modèle) : {} – {}, mise {} €.",
                bet.home_team,
                bet.away_team,
                format::decimal(bet.stake.get(), 1)
            ))
        }
        Action::SettleAuto => {
            let report = settle::settle_open_bets(&conn, &Aliases::bundled()?, now)?;
            Ok(format!(
                "{} pari(s) réglé(s) depuis l'historique ; {} encore ouvert(s).",
                report.settled, report.still_open
            ))
        }
        Action::SettleManual { bet_id, settlement } => {
            let Some(stored) = journal::bets(&conn)?.into_iter().find(|b| b.id == *bet_id) else {
                bail!("pari n° {bet_id} introuvable");
            };
            if !journal::insert_result(&conn, &stored, *settlement, SettlementSource::Manual, now)?
            {
                bail!("le pari n° {bet_id} est déjà réglé : un règlement ne se modifie pas");
            }
            Ok(format!("Pari n° {bet_id} réglé à la main."))
        }
        Action::LogBonus(request) => {
            let new = tracking::prepare_operation(&conn, request)?;
            let id = tracking::record_operation(&mut conn, &new, now)?;
            Ok(format!(
                "Opération n° {id} enregistrée chez {} : gain garanti {} €.",
                operator_name(&request.bookmaker),
                format::signed(new.guaranteed, 2)
            ))
        }
        Action::RecordOffer(offer) => {
            let id = bonus::insert_offer(&conn, offer, now)?;
            Ok(format!(
                "Offre n° {id} enregistrée pour {}.",
                operator_name(&offer.bookmaker)
            ))
        }
        Action::SettleBonusAuto => {
            let report = tracking::settle_open_operations(&conn, &Aliases::bundled()?, now)?;
            Ok(format!(
                "{} opération(s) réglée(s) depuis l'historique ; {} encore ouverte(s).",
                report.settled, report.still_open
            ))
        }
        Action::SettleBonusManual {
            operation_id,
            result,
        } => {
            tracking::settle_operation(&conn, *operation_id, *result, now)?;
            Ok(format!("Opération n° {operation_id} réglée à la main."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sample;
    use edge_core::store::journal::bets;

    #[test]
    fn following_a_recommendation_writes_the_bet_and_a_second_attempt_is_refused() {
        let sample = sample();
        let action = Action::LogFollowed {
            recommendation_id: 1,
            input: FollowedInput::default(),
        };

        let message = apply(&sample.paths, &action, sample.now).unwrap();
        let again = apply(&sample.paths, &action, sample.now).unwrap_err();

        assert!(message.contains("Pari n° 1 enregistré"), "{message}");
        assert!(again.to_string().contains("déjà enregistré"));
        let conn = open_migrated(&sample.paths.db_file).unwrap();
        assert_eq!(bets(&conn).unwrap().len(), 1);
    }

    #[test]
    fn odds_below_the_minimum_are_refused_with_the_core_message() {
        let sample = sample();
        let action = Action::LogFollowed {
            recommendation_id: 1,
            input: FollowedInput {
                odds: Some(1.5),
                ..FollowedInput::default()
            },
        };

        let error = apply(&sample.paths, &action, sample.now).unwrap_err();

        assert!(error.to_string().contains("cote minimale"));
    }

    #[test]
    fn an_off_model_bet_then_a_manual_settlement_then_no_second_settlement() {
        let sample = sample();
        let off_model = Action::LogOffModel {
            event_id: "match-gui".into(),
            bookmaker: "winamax_fr".into(),
            outcome: Outcome::Draw,
            odds: 3.4,
            stake: 5.0,
        };
        assert!(
            apply(&sample.paths, &off_model, sample.now)
                .unwrap()
                .contains("hors modèle")
        );

        let settle = Action::SettleManual {
            bet_id: 1,
            settlement: Settlement::Void,
        };
        assert!(apply(&sample.paths, &settle, sample.now).is_ok());
        assert!(
            apply(&sample.paths, &settle, sample.now)
                .unwrap_err()
                .to_string()
                .contains("déjà réglé")
        );
        let missing = Action::SettleManual {
            bet_id: 99,
            settlement: Settlement::Won,
        };
        assert!(apply(&sample.paths, &missing, sample.now).is_err());
    }

    #[test]
    fn the_automatic_settlement_reports_open_bets_when_no_result_is_known() {
        let sample = sample();
        let follow = Action::LogFollowed {
            recommendation_id: 1,
            input: FollowedInput::default(),
        };
        apply(&sample.paths, &follow, sample.now).unwrap();

        let message = apply(&sample.paths, &Action::SettleAuto, sample.now).unwrap();

        assert!(message.contains("0 pari(s) réglé(s)"));
        assert!(message.contains("1 encore ouvert(s)"));
    }

    #[test]
    fn an_offer_then_a_covered_operation_then_a_settlement_are_written() {
        use edge_core::bonus::OfferKind;
        use edge_core::domain::{Euros, Odds};
        use edge_core::store::bonus::{Eligibility, Leg, Role};

        let sample = sample();
        let offer = |bookmaker: &str, eligibility| {
            Action::RecordOffer(NewOffer {
                bookmaker: bookmaker.into(),
                eligibility,
                checked_on: sample.now.date_naive(),
                source: "règlement".into(),
                note: None,
            })
        };
        apply(
            &sample.paths,
            &offer("pinnacle", Eligibility::Ineligible),
            sample.now,
        )
        .unwrap();
        let eligible = Eligibility::Eligible {
            kind: OfferKind::CashRefund,
            amount: Euros::new(100.0).unwrap(),
            min_odds: None,
        };
        assert!(
            apply(&sample.paths, &offer("betclic_fr", eligible), sample.now)
                .unwrap()
                .contains("Betclic")
        );

        let leg = |role, bookmaker: &str, outcome, odds, stake| Leg {
            role,
            bookmaker: bookmaker.into(),
            outcome,
            odds: Odds::new(odds).unwrap(),
            stake: Euros::new(stake).unwrap(),
        };
        let request = OperationRequest {
            bookmaker: "betclic_fr".into(),
            kind: None,
            event_id: "match-gui".into(),
            legs: vec![
                leg(Role::Bonus, "betclic_fr", Outcome::Away, 3.4, 100.0),
                leg(Role::Cover, "pinnacle", Outcome::Home, 2.0, 120.0),
                leg(Role::Cover, "pinnacle", Outcome::Draw, 3.6, 66.7),
            ],
            freebet_value: 0.53,
        };
        let message = apply(
            &sample.paths,
            &Action::LogBonus(request.clone()),
            sample.now,
        )
        .unwrap();
        assert!(message.contains("Opération n° 1"), "{message}");
        // L'offre est consommée : une seconde opération est refusée par le cœur.
        assert!(apply(&sample.paths, &Action::LogBonus(request), sample.now).is_err());

        let settle = Action::SettleBonusManual {
            operation_id: 1,
            result: MatchResult::Played(Outcome::Home),
        };
        assert!(apply(&sample.paths, &settle, sample.now).is_ok());
        assert!(apply(&sample.paths, &settle, sample.now).is_err());
        assert!(
            apply(&sample.paths, &Action::SettleBonusAuto, sample.now)
                .unwrap()
                .contains("0 opération(s)")
        );
    }
}
