//! Les écritures demandées par les formulaires : chacune passe par les fonctions du cœur qu'utilise la ligne de commande,
//! avec leurs règles (cote minimale, règlement unique, offre consommée…). Rien n'est écrit ailleurs.

use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};

use crate::betting::ledger::{self, FollowedInput};
use crate::betting::settle;
use crate::bonus::operator_name;
use crate::bonus::tracking::{self, OperationRequest};
use crate::domain::{Euros, Outcome};
use crate::error::{Error, Result};
use crate::format;
use crate::ingest::aliases::Aliases;
use crate::loto::{self, Grid};
use crate::lotofoot::{TICKET_PRICE, played};
use crate::store::bonus::{self, MatchResult, NewOffer};
use crate::store::journal::{self, Settlement, SettlementSource};
use crate::store::open_migrated;

/// Une écriture.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Enregistrer un pari qui suit une recommandation.
    LogFollowed {
        /// Recommandation suivie.
        recommendation_id: i64,
        /// Valeurs saisies (l'absence reprend la recommandation).
        input: FollowedInput,
    },
    /// Enregistrer un pari hors modèle (`off_model`, CLAUDE.md, invariant 7).
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
    /// Déclarer jouée toute la dernière séance Loto Foot (comme `edge lotofoot played --last-session`).
    LotofootPlayedSession,
    /// Déclarer jouées les grilles de la dernière séance du Grand Loto (comme `edge loto played --last-session`).
    LotoPlayedSession,
    /// Déclarer une grille de Grand Loto saisie à la main (comme `edge loto played --grid … --date …`).
    LotoGrid {
        /// Édition.
        date: NaiveDate,
        /// La grille.
        grid: Grid,
        /// Mise, en euros.
        stake: f64,
    },
}

/// Exécute une écriture et renvoie le message à afficher.
pub fn apply(
    db_file: &Path,
    aliases: &Aliases,
    action: &Action,
    now: DateTime<Utc>,
) -> Result<String> {
    // `mut` : enregistrer une opération bonus ouvre une transaction, qui demande un accès exclusif à la connexion.
    let mut conn = open_migrated(db_file)?;
    match action {
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
            let report = settle::settle_open_bets(&conn, aliases, now)?;
            Ok(format!(
                "{} pari(s) réglé(s) depuis l'historique ; {} encore ouvert(s).",
                report.settled, report.still_open
            ))
        }
        Action::SettleManual { bet_id, settlement } => {
            let Some(stored) = journal::bets(&conn)?.into_iter().find(|b| b.id == *bet_id) else {
                return Err(Error::Input(format!("pari n° {bet_id} introuvable")));
            };
            if !journal::insert_result(&conn, &stored, *settlement, SettlementSource::Manual, now)?
            {
                return Err(Error::Input(format!(
                    "le pari n° {bet_id} est déjà réglé : un règlement ne se modifie pas"
                )));
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
            let report = tracking::settle_open_operations(&conn, aliases, now)?;
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
        Action::LotofootPlayedSession => {
            let stake = Euros::new(TICKET_PRICE)
                .ok_or_else(|| Error::Data("prix du ticket invalide".to_string()))?;
            let declared = played::declare_last_session(&conn, now, stake)?;
            let mut message = format!(
                "{} ticket(s) Loto Foot déclaré(s) joué(s).",
                declared.written.len()
            );
            if !declared.already.is_empty() {
                message.push_str(&format!(
                    " {} déjà déclaré(s) : ignoré(s).",
                    declared.already.len()
                ));
            }
            Ok(message)
        }
        Action::LotoPlayedSession => {
            let written = loto::played::declare_last_session(&conn, now)?;
            Ok(format!(
                "{} grille(s) du Grand Loto déclarée(s) jouée(s).",
                written.len()
            ))
        }
        Action::LotoGrid { date, grid, stake } => {
            let (id, off_model) = loto::played::declare_manual(&conn, *date, *grid, *stake, now)?;
            let kind = if off_model {
                "hors modèle"
            } else {
                "rattachée à sa recommandation"
            };
            Ok(format!(
                "Grille {grid} du {} déclarée jouée ({kind}), n° {id}.",
                date.format("%d/%m/%Y")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::journal::bets;
    use crate::web::testing::{Sample, empty, sample};

    fn run(sample: &Sample, action: &Action) -> Result<String> {
        apply(
            &sample.app.paths.db_file,
            &sample.app.aliases,
            action,
            sample.now,
        )
    }

    #[test]
    fn following_a_recommendation_writes_the_bet_and_a_second_attempt_is_refused() {
        let sample = sample();
        let action = Action::LogFollowed {
            recommendation_id: 1,
            input: FollowedInput::default(),
        };

        let message = run(&sample, &action).unwrap();
        let again = run(&sample, &action).unwrap_err();

        assert!(message.contains("Pari n° 1 enregistré"), "{message}");
        assert!(again.to_string().contains("déjà enregistré"));
        let conn = open_migrated(&sample.app.paths.db_file).unwrap();
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

        let error = run(&sample, &action).unwrap_err();

        assert!(error.to_string().contains("cote minimale"));
    }

    #[test]
    fn an_off_model_bet_is_marked_then_settled_once() {
        let sample = sample();
        let off_model = Action::LogOffModel {
            event_id: "match-web".into(),
            bookmaker: "winamax_fr".into(),
            outcome: Outcome::Draw,
            odds: 3.4,
            stake: 5.0,
        };
        assert!(run(&sample, &off_model).unwrap().contains("hors modèle"));
        // Invariant 7 : un pari sans recommandation est rangé à part.
        let conn = open_migrated(&sample.app.paths.db_file).unwrap();
        assert_eq!(bets(&conn).unwrap()[0].bet.recommendation_id, None);

        let settle = Action::SettleManual {
            bet_id: 1,
            settlement: Settlement::Void,
        };
        assert!(run(&sample, &settle).is_ok());
        assert!(
            run(&sample, &settle)
                .unwrap_err()
                .to_string()
                .contains("déjà réglé")
        );
        let missing = Action::SettleManual {
            bet_id: 99,
            settlement: Settlement::Won,
        };
        assert!(run(&sample, &missing).is_err());
    }

    #[test]
    fn the_automatic_settlement_reports_open_bets_when_no_result_is_known() {
        let sample = sample();
        let follow = Action::LogFollowed {
            recommendation_id: 1,
            input: FollowedInput::default(),
        };
        run(&sample, &follow).unwrap();

        let message = run(&sample, &Action::SettleAuto).unwrap();

        assert!(message.contains("0 pari(s) réglé(s)"));
        assert!(message.contains("1 encore ouvert(s)"));
    }

    #[test]
    fn an_offer_then_a_covered_operation_then_a_settlement_are_written() {
        use crate::bonus::OfferKind;
        use crate::domain::Odds;
        use crate::store::bonus::{Eligibility, Leg, Role};

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
        run(&sample, &offer("pinnacle", Eligibility::Ineligible)).unwrap();
        let eligible = Eligibility::Eligible {
            kind: OfferKind::CashRefund,
            amount: Euros::new(100.0).unwrap(),
            min_odds: None,
        };
        assert!(
            run(&sample, &offer("betclic_fr", eligible))
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
            event_id: "match-web".into(),
            legs: vec![
                leg(Role::Bonus, "betclic_fr", Outcome::Away, 3.4, 100.0),
                leg(Role::Cover, "pinnacle", Outcome::Home, 2.0, 120.0),
                leg(Role::Cover, "pinnacle", Outcome::Draw, 3.6, 66.7),
            ],
            freebet_value: 0.53,
        };
        let message = run(&sample, &Action::LogBonus(request.clone())).unwrap();
        assert!(message.contains("Opération n° 1"), "{message}");
        // L'offre est consommée : une seconde opération est refusée par le cœur.
        assert!(run(&sample, &Action::LogBonus(request)).is_err());

        let settle = Action::SettleBonusManual {
            operation_id: 1,
            result: MatchResult::Played(Outcome::Home),
        };
        assert!(run(&sample, &settle).is_ok());
        assert!(run(&sample, &settle).is_err());
        assert!(
            run(&sample, &Action::SettleBonusAuto)
                .unwrap()
                .contains("0 opération(s)")
        );
    }

    #[test]
    fn declaring_the_lotofoot_session_twice_writes_each_ticket_once() {
        use crate::store::lotofoot::plays;

        let sample = sample();
        let first = run(&sample, &Action::LotofootPlayedSession).unwrap();
        let again = run(&sample, &Action::LotofootPlayedSession).unwrap();

        assert!(first.starts_with("2 ticket(s)"), "{first}");
        assert!(again.contains("2 déjà déclaré(s)"), "{again}");
        let conn = open_migrated(&sample.app.paths.db_file).unwrap();
        assert_eq!(plays(&conn).unwrap().len(), 2);
        let error = run(&empty(), &Action::LotofootPlayedSession).unwrap_err();
        assert!(error.to_string().contains("aucune séance"), "{error}");
    }
}
