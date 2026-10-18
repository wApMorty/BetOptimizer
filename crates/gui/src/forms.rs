//! Formulaires de saisie : le texte tapé par l'utilisateur devient une action validée, ou un message qui dit quoi corriger.
//!
//! Fonctions pures, testées sans fenêtre. Les règles métier (cote minimale, mise positive…) restent dans `edge-core`
//! (`betting::ledger`) : ici on ne fait que lire des nombres écrits à la française.

use edge_core::betting::ledger::FollowedInput;
use edge_core::domain::Outcome;
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
}
