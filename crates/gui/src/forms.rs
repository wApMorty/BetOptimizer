//! Formulaires de saisie : le texte tapé par l'utilisateur devient une action validée, ou un message qui dit quoi corriger.
//!
//! Fonctions pures, testées sans fenêtre. Les règles métier (cote minimale, mise positive…) restent dans `edge-core`
//! (`betting::ledger`) : ici on ne fait que lire des nombres écrits à la française.

use chrono::NaiveDate;
use edge_core::betting::ledger::FollowedInput;
use edge_core::bonus::OfferKind;
use edge_core::bonus::cover::{Cover, cover};
use edge_core::bonus::pick::{Candidate, Target};
use edge_core::bonus::tracking::{OperationRequest, guaranteed_gain};
use edge_core::domain::{Euros, Odds, Outcome};
use edge_core::format;
use edge_core::store::bonus::{Eligibility, Leg, NewOffer, Role};
use edge_core::store::journal::StoredRecommendation;

use crate::actions::Action;

/// Lit un nombre tapé à la main : virgule ou point décimal, texte vide = absent.
pub fn parse_number(text: &str, field: &str) -> Result<Option<f64>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(Some)
        .ok_or_else(|| format!("{field} : « {trimmed} » n'est pas un nombre"))
}

/// Un nombre obligatoire.
fn required(text: &str, field: &str) -> Result<f64, String> {
    parse_number(text, field)?.ok_or_else(|| format!("{field} est requis"))
}

/// Formulaire « J'ai parié » d'une recommandation, prérempli avec ses valeurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowForm {
    /// Recommandation suivie.
    pub recommendation_id: i64,
    /// Cote réellement obtenue.
    pub odds: String,
    /// Mise réellement placée.
    pub stake: String,
    /// Bookmaker chez qui le pari est placé.
    pub bookmaker: String,
}

impl FollowForm {
    /// Préremplit le formulaire avec la cote, la mise suggérée et le bookmaker de la recommandation.
    pub fn for_recommendation(found: &StoredRecommendation) -> FollowForm {
        let r = &found.record;
        FollowForm {
            recommendation_id: found.id,
            odds: edge_core::format::decimal(r.odds.get(), 2),
            stake: r
                .stake
                .map_or_else(String::new, |s| edge_core::format::decimal(s.get(), 1)),
            bookmaker: r.bookmaker.clone(),
        }
    }

    /// Valide la saisie.
    pub fn to_action(&self) -> Result<Action, String> {
        Ok(Action::LogFollowed {
            recommendation_id: self.recommendation_id,
            input: FollowedInput {
                odds: parse_number(&self.odds, "cote")?,
                stake: parse_number(&self.stake, "mise")?,
                bookmaker: Some(self.bookmaker.trim().to_string()).filter(|b| !b.is_empty()),
            },
        })
    }
}

/// Formulaire d'un pari hors modèle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OffModelForm {
    /// Match choisi dans la liste des matchs à venir.
    pub event_id: Option<String>,
    /// Bookmaker.
    pub bookmaker: String,
    /// Issue jouée.
    pub outcome: Option<Outcome>,
    /// Cote obtenue.
    pub odds: String,
    /// Mise.
    pub stake: String,
}

impl OffModelForm {
    /// Valide la saisie.
    pub fn to_action(&self) -> Result<Action, String> {
        let event_id = self
            .event_id
            .clone()
            .ok_or("choisis un match dans la liste")?;
        let outcome = self
            .outcome
            .ok_or("choisis une issue (domicile, nul ou extérieur)")?;
        if self.bookmaker.trim().is_empty() {
            return Err("le bookmaker est requis".to_string());
        }
        Ok(Action::LogOffModel {
            event_id,
            bookmaker: self.bookmaker.trim().to_string(),
            outcome,
            odds: required(&self.odds, "cote")?,
            stake: required(&self.stake, "mise")?,
        })
    }
}

/// Un pari d'une opération bonus, tel que saisi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BonusLegForm {
    /// Pari bonus ou couverture.
    pub role: Role,
    /// Opérateur.
    pub bookmaker: String,
    /// Issue jouée.
    pub outcome: Outcome,
    /// Cote réellement obtenue.
    pub odds: String,
    /// Mise.
    pub stake: String,
}

impl BonusLegForm {
    fn to_leg(&self) -> Result<Leg, String> {
        let what = format!(
            "{} ({})",
            format::outcome_label(self.outcome),
            self.bookmaker
        );
        Ok(Leg {
            role: self.role,
            bookmaker: self.bookmaker.clone(),
            outcome: self.outcome,
            odds: Odds::new(required(&self.odds, &format!("cote {what}"))?)
                .ok_or_else(|| format!("cote {what} : elle doit être supérieure à 1"))?,
            stake: Euros::new(required(&self.stake, &format!("mise {what}"))?)
                .filter(|s| s.get() > 0.0)
                .ok_or_else(|| format!("mise {what} : elle doit être positive"))?,
        })
    }
}

/// Formulaire « J'ai joué cette opération » : préremplie avec la proposition, à corriger avec les cotes réellement affichées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BonusForm {
    /// Opérateur du bonus.
    pub bookmaker: String,
    /// Forme de l'opération.
    pub kind: OfferKind,
    /// Match de l'archive.
    pub event_id: String,
    /// `Domicile – Extérieur`, pour le titre.
    pub event_label: String,
    /// Le pari bonus puis les couvertures.
    pub legs: Vec<BonusLegForm>,
}

impl BonusForm {
    /// Préremplit le formulaire avec une opération proposée.
    pub fn from_candidate(target: &Target, candidate: &Candidate) -> BonusForm {
        BonusForm {
            bookmaker: target.bookmaker.clone(),
            kind: target.kind,
            event_id: candidate.event.event_id.clone(),
            event_label: format!(
                "{} – {}",
                candidate.event.home_team, candidate.event.away_team
            ),
            legs: candidate
                .legs
                .iter()
                .map(|leg| BonusLegForm {
                    role: leg.role,
                    bookmaker: leg.bookmaker.clone(),
                    outcome: leg.outcome,
                    odds: format::decimal(leg.odds.get(), 2),
                    stake: format::decimal(leg.stake.get(), 2),
                })
                .collect(),
        }
    }

    fn legs(&self) -> Result<Vec<Leg>, String> {
        self.legs.iter().map(BonusLegForm::to_leg).collect()
    }

    /// Recalcule les mises de couverture sur les cotes saisies (la mise du bonus ne change pas).
    pub fn recompute(&mut self, freebet_value: f64) -> Result<(), String> {
        let Some(bonus) = self.legs.iter().find(|l| l.role == Role::Bonus) else {
            return Err("le pari bonus manque".to_string());
        };
        let bonus = bonus.to_leg()?;
        // Des couvertures, seules les cotes comptent : leurs mises sont justement ce que l'on recalcule.
        let mut others = Vec::new();
        for leg in self.legs.iter().filter(|l| l.role == Role::Cover) {
            let what = format!(
                "cote {} ({})",
                format::outcome_label(leg.outcome),
                leg.bookmaker
            );
            others.push(
                Odds::new(required(&leg.odds, &what)?)
                    .ok_or_else(|| format!("{what} : elle doit être supérieure à 1"))?,
            );
        }
        let result = cover(self.kind, bonus.stake, bonus.odds, freebet_value, &others)
            .map_err(|e| e.to_string())?;
        let covers = self.legs.iter_mut().filter(|l| l.role == Role::Cover);
        for (leg, stake) in covers.zip(&result.stakes) {
            leg.stake = format::decimal(stake.get(), 2);
        }
        Ok(())
    }

    /// Gain garanti des valeurs saisies, ou `None` tant qu'une valeur est illisible.
    pub fn guaranteed(&self, freebet_value: f64) -> Option<f64> {
        let value = if self.kind == OfferKind::FreebetRefund {
            freebet_value
        } else {
            0.0
        };
        self.legs()
            .ok()
            .map(|legs| guaranteed_gain(self.kind, &legs, value))
    }

    /// Valide la saisie. Les règles du journal (offre consommée, compte de couverture…) sont vérifiées par `edge-core`.
    pub fn to_action(&self, freebet_value: f64) -> Result<Action, String> {
        Ok(Action::LogBonus(OperationRequest {
            bookmaker: self.bookmaker.clone(),
            kind: Some(self.kind),
            event_id: self.event_id.clone(),
            legs: self.legs()?,
            freebet_value,
        }))
    }
}

/// Formulaire d'enregistrement d'une offre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferForm {
    /// Opérateur.
    pub bookmaker: String,
    /// L'offre est-elle accessible ?
    pub eligible: bool,
    /// Forme de l'offre.
    pub kind: OfferKind,
    /// Montant maximal.
    pub amount: String,
    /// Cote minimale (vide : aucune).
    pub min_odds: String,
    /// Où l'éligibilité a été vérifiée.
    pub source: String,
    /// Remarque libre.
    pub note: String,
}

impl OfferForm {
    /// Formulaire vierge pour un opérateur : remboursement en freebet de 100 €, la forme la plus courante.
    pub fn for_operator(bookmaker: &str) -> OfferForm {
        OfferForm {
            bookmaker: bookmaker.to_string(),
            eligible: true,
            kind: OfferKind::FreebetRefund,
            amount: "100".to_string(),
            min_odds: String::new(),
            source: String::new(),
            note: String::new(),
        }
    }

    /// Valide la saisie ; `today` est la date de vérification enregistrée.
    pub fn to_action(&self, today: NaiveDate) -> Result<Action, String> {
        let bookmaker = self.bookmaker.trim();
        if !edge_core::bonus::is_operator_key(bookmaker) {
            return Err(
                "opérateur : une clé en minuscules, chiffres et _ (betclic_fr…)".to_string(),
            );
        }
        if self.source.trim().is_empty() {
            return Err(
                "source : où as-tu vérifié l'éligibilité (règlement, service client…) ?"
                    .to_string(),
            );
        }
        let eligibility = if self.eligible {
            Eligibility::Eligible {
                kind: self.kind,
                amount: Euros::new(required(&self.amount, "montant")?)
                    .filter(|a| a.get() > 0.0)
                    .ok_or("montant : il doit être positif")?,
                min_odds: match parse_number(&self.min_odds, "cote minimale")? {
                    Some(value) => Some(
                        Odds::new(value).ok_or("cote minimale : elle doit être supérieure à 1")?,
                    ),
                    None => None,
                },
            }
        } else {
            Eligibility::Ineligible
        };
        Ok(Action::RecordOffer(NewOffer {
            bookmaker: bookmaker.to_string(),
            eligibility,
            checked_on: today,
            source: self.source.trim().to_string(),
            note: Some(self.note.trim().to_string()).filter(|n| !n.is_empty()),
        }))
    }
}

/// Calculateur de couverture de l'écran Bonus : rien n'est enregistré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalcForm {
    /// Forme, indice dans [`OfferKind::ALL`].
    pub kind: usize,
    /// Montant du bonus.
    pub amount: String,
    /// Cote du pari bonus.
    pub odds: String,
    /// Cotes de couverture des autres issues (la seconde est vide pour un marché à deux issues).
    pub covers: [String; 2],
}

impl Default for CalcForm {
    fn default() -> CalcForm {
        CalcForm {
            kind: 1,
            amount: "100".to_string(),
            odds: String::new(),
            covers: [String::new(), String::new()],
        }
    }
}

impl CalcForm {
    /// La forme choisie.
    pub fn kind(&self) -> OfferKind {
        OfferKind::ALL[self.kind.min(OfferKind::ALL.len() - 1)]
    }

    /// Calcule les mises, ou dit quoi corriger.
    pub fn compute(&self, freebet_value: f64) -> Result<Cover, String> {
        let amount = Euros::new(required(&self.amount, "montant")?)
            .ok_or("montant : il doit être positif")?;
        let odds = Odds::new(required(&self.odds, "cote du bonus")?)
            .ok_or("cote du bonus : elle doit être supérieure à 1")?;
        let mut others = Vec::new();
        for (index, text) in self.covers.iter().enumerate() {
            if let Some(value) = parse_number(text, &format!("couverture {}", index + 1))? {
                others.push(
                    Odds::new(value).ok_or("une cote de couverture doit être supérieure à 1")?,
                );
            }
        }
        cover(self.kind(), amount, odds, freebet_value, &others).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_accept_a_decimal_comma_and_an_empty_text_means_absent() {
        assert_eq!(parse_number("2,30", "cote"), Ok(Some(2.3)));
        assert_eq!(parse_number(" 2.3 ", "cote"), Ok(Some(2.3)));
        assert_eq!(parse_number("   ", "cote"), Ok(None));
        assert!(parse_number("abc", "cote").unwrap_err().contains("cote"));
        assert!(parse_number("inf", "mise").is_err());
        assert!(parse_number("NaN", "mise").is_err());
    }

    #[test]
    fn a_follow_form_without_changes_lets_the_recommendation_decide() {
        let form = FollowForm {
            recommendation_id: 3,
            odds: String::new(),
            stake: String::new(),
            bookmaker: "  ".into(),
        };

        let action = form.to_action().unwrap();

        assert_eq!(
            action,
            Action::LogFollowed {
                recommendation_id: 3,
                input: FollowedInput::default(),
            }
        );
    }

    #[test]
    fn a_follow_form_reads_the_typed_values() {
        let form = FollowForm {
            recommendation_id: 3,
            odds: "2,25".into(),
            stake: "40".into(),
            bookmaker: "winamax_fr".into(),
        };

        let Action::LogFollowed { input, .. } = form.to_action().unwrap() else {
            panic!("action inattendue");
        };

        assert_eq!(input.odds, Some(2.25));
        assert_eq!(input.stake, Some(40.0));
        assert_eq!(input.bookmaker.as_deref(), Some("winamax_fr"));
        assert!(
            FollowForm {
                odds: "x".into(),
                ..form
            }
            .to_action()
            .is_err()
        );
    }

    #[test]
    fn an_off_model_form_needs_a_match_an_outcome_a_bookmaker_odds_and_a_stake() {
        let complete = OffModelForm {
            event_id: Some("m1".into()),
            bookmaker: " winamax_fr ".into(),
            outcome: Some(Outcome::Draw),
            odds: "3,4".into(),
            stake: "5".into(),
        };

        assert_eq!(
            complete.to_action(),
            Ok(Action::LogOffModel {
                event_id: "m1".into(),
                bookmaker: "winamax_fr".into(),
                outcome: Outcome::Draw,
                odds: 3.4,
                stake: 5.0,
            })
        );
        let missing = |form: OffModelForm| form.to_action().unwrap_err();
        assert!(
            missing(OffModelForm {
                event_id: None,
                ..complete.clone()
            })
            .contains("match")
        );
        assert!(
            missing(OffModelForm {
                outcome: None,
                ..complete.clone()
            })
            .contains("issue")
        );
        assert!(
            missing(OffModelForm {
                bookmaker: " ".into(),
                ..complete.clone()
            })
            .contains("bookmaker")
        );
        assert!(
            missing(OffModelForm {
                odds: String::new(),
                ..complete.clone()
            })
            .contains("cote")
        );
        assert!(
            missing(OffModelForm {
                stake: "x".into(),
                ..complete
            })
            .contains("mise")
        );
    }

    fn bonus_form() -> BonusForm {
        let leg = |role, bookmaker: &str, outcome, odds: &str, stake: &str| BonusLegForm {
            role,
            bookmaker: bookmaker.into(),
            outcome,
            odds: odds.into(),
            stake: stake.into(),
        };
        BonusForm {
            bookmaker: "betclic_fr".into(),
            kind: OfferKind::Freebet,
            event_id: "m1".into(),
            event_label: "A – B".into(),
            legs: vec![
                leg(Role::Bonus, "betclic_fr", Outcome::Away, "4,2", "100"),
                leg(Role::Cover, "winamax_fr", Outcome::Home, "1,85", "0"),
                leg(Role::Cover, "winamax_fr", Outcome::Draw, "3,90", "0"),
            ],
        }
    }

    #[test]
    fn a_bonus_form_recomputes_cover_stakes_from_the_typed_odds() {
        let mut form = bonus_form();
        // Mises nulles : illisibles pour le calcul du gain garanti, mais le recalcul les remplace.
        assert!(form.guaranteed(0.53).is_none());
        form.recompute(0.53).unwrap();
        assert_eq!(form.legs[1].stake, "173,00");
        assert_eq!(form.legs[2].stake, "82,10");
        let guaranteed = form.guaranteed(0.53).unwrap();
        assert!((guaranteed - 64.9).abs() < 0.3, "{guaranteed}");
        let Action::LogBonus(request) = form.to_action(0.53).unwrap() else {
            panic!("action inattendue");
        };
        assert_eq!(request.kind, Some(OfferKind::Freebet));
        assert_eq!(request.legs.len(), 3);
        form.legs[1].odds = "1".into();
        assert!(form.recompute(0.53).unwrap_err().contains("supérieure à 1"));
    }

    #[test]
    fn an_offer_form_needs_a_key_a_source_and_for_an_eligible_offer_an_amount() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        let mut form = OfferForm::for_operator("betclic_fr");
        assert!(form.to_action(today).unwrap_err().contains("source"));
        form.source = "règlement".into();
        form.min_odds = "1,5".into();
        let Action::RecordOffer(offer) = form.to_action(today).unwrap() else {
            panic!("action inattendue");
        };
        assert_eq!(offer.checked_on, today);
        assert!(matches!(
            offer.eligibility,
            Eligibility::Eligible {
                min_odds: Some(_),
                ..
            }
        ));
        form.amount = "0".into();
        assert!(form.to_action(today).unwrap_err().contains("montant"));
        // Inéligible : le montant n'est pas lu.
        form.eligible = false;
        assert!(matches!(
            form.to_action(today),
            Ok(Action::RecordOffer(NewOffer {
                eligibility: Eligibility::Ineligible,
                ..
            }))
        ));
        form.bookmaker = "Betclic".into();
        assert!(form.to_action(today).unwrap_err().contains("clé"));
    }

    #[test]
    fn the_calculator_accepts_one_or_two_cover_odds() {
        let mut form = CalcForm {
            kind: 2,
            amount: "50".into(),
            odds: "2".into(),
            covers: ["2".into(), String::new()],
        };
        assert_eq!(form.kind(), OfferKind::Freebet);
        assert!((form.compute(0.53).unwrap().guaranteed - 25.0).abs() < 1e-9);
        form.covers = [String::new(), String::new()];
        assert!(form.compute(0.53).is_err());
        form.odds = "x".into();
        assert!(form.compute(0.53).unwrap_err().contains("cote du bonus"));
    }
}
