//! Formulaires de saisie : les champs envoyés par le navigateur deviennent une [`Action`] validée, ou un message qui
//! dit quoi corriger. Chaque formulaire sait aussi se préremplir (recommandation suivie, opération proposée…).
//!
//! Les règles métier (cote minimale, mise positive, offre consommée…) restent dans le cœur (`betting::ledger`,
//! `bonus::tracking`) : ici on ne fait que lire des nombres écrits à la française.

use chrono::NaiveDate;
use serde::Serialize;

use super::actions::Action;
use super::form::{Form, parse_number, required};
use crate::betting::ledger::FollowedInput;
use crate::bonus::OfferKind;
use crate::bonus::cover::cover;
use crate::bonus::pick::{Candidate, Target};
use crate::bonus::tracking::{OperationRequest, guaranteed_gain};
use crate::domain::{Euros, Odds, Outcome};
use crate::format;
use crate::loto::Grid;
use crate::store::bonus::{Eligibility, Leg, NewOffer, Role};
use crate::store::journal::StoredRecommendation;

/// Code d'une issue dans un champ : `1`, `N` ou `2`.
pub fn outcome_code(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Home => "1",
        Outcome::Draw => "N",
        Outcome::Away => "2",
    }
}

/// L'issue d'un code de champ.
pub fn outcome_from(code: &str) -> Option<Outcome> {
    match code {
        "1" => Some(Outcome::Home),
        "N" => Some(Outcome::Draw),
        "2" => Some(Outcome::Away),
        _ => None,
    }
}

/// Formulaire « J'ai parié » d'une recommandation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FollowForm {
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
            odds: format::decimal(r.odds.get(), 2),
            stake: r
                .stake
                .map_or_else(String::new, |s| format::decimal(s.get(), 1)),
            bookmaker: r.bookmaker.clone(),
        }
    }

    /// Lit les champs envoyés.
    pub fn from_form(form: &Form) -> FollowForm {
        FollowForm {
            odds: form.get("cote").to_string(),
            stake: form.get("mise").to_string(),
            bookmaker: form.get("bookmaker").to_string(),
        }
    }

    /// Valide la saisie ; un champ vide reprend la valeur de la recommandation.
    pub fn to_action(&self, recommendation_id: i64) -> Result<Action, String> {
        Ok(Action::LogFollowed {
            recommendation_id,
            input: FollowedInput {
                odds: parse_number(&self.odds, "cote")?,
                stake: parse_number(&self.stake, "mise")?,
                bookmaker: Some(self.bookmaker.trim().to_string()).filter(|b| !b.is_empty()),
            },
        })
    }
}

/// Formulaire d'un pari hors modèle.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OffModelForm {
    /// Match choisi dans la liste des matchs à venir.
    pub event_id: String,
    /// Bookmaker.
    pub bookmaker: String,
    /// Issue jouée (`1`, `N`, `2`).
    pub outcome: String,
    /// Cote obtenue.
    pub odds: String,
    /// Mise.
    pub stake: String,
}

impl OffModelForm {
    /// Lit les champs envoyés.
    pub fn from_form(form: &Form) -> OffModelForm {
        OffModelForm {
            event_id: form.get("match").to_string(),
            bookmaker: form.get("bookmaker").to_string(),
            outcome: form.get("issue").to_string(),
            odds: form.get("cote").to_string(),
            stake: form.get("mise").to_string(),
        }
    }

    /// Valide la saisie.
    pub fn to_action(&self) -> Result<Action, String> {
        if self.event_id.is_empty() {
            return Err("choisis un match dans la liste".to_string());
        }
        let outcome =
            outcome_from(&self.outcome).ok_or("choisis une issue (domicile, nul ou extérieur)")?;
        if self.bookmaker.trim().is_empty() {
            return Err("le bookmaker est requis".to_string());
        }
        Ok(Action::LogOffModel {
            event_id: self.event_id.clone(),
            bookmaker: self.bookmaker.trim().to_string(),
            outcome,
            odds: required(&self.odds, "cote")?,
            stake: required(&self.stake, "mise")?,
        })
    }
}

/// Un pari d'une opération bonus, tel que saisi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BonusLegForm {
    /// Pari bonus (`true`) ou couverture.
    pub bonus: bool,
    /// Opérateur.
    pub bookmaker: String,
    /// Issue jouée (`1`, `N`, `2`).
    pub outcome: String,
    /// Libellé de l'issue (`domicile`).
    pub outcome_label: &'static str,
    /// Cote réellement obtenue.
    pub odds: String,
    /// Mise.
    pub stake: String,
}

impl BonusLegForm {
    fn outcome(&self) -> Result<Outcome, String> {
        outcome_from(&self.outcome).ok_or_else(|| format!("issue inconnue : « {} »", self.outcome))
    }

    fn what(&self) -> String {
        format!("{} ({})", self.outcome_label, self.bookmaker)
    }

    fn odds(&self) -> Result<Odds, String> {
        let what = self.what();
        Odds::new(required(&self.odds, &format!("cote {what}"))?)
            .ok_or_else(|| format!("cote {what} : elle doit être supérieure à 1"))
    }

    fn to_leg(&self) -> Result<Leg, String> {
        let what = self.what();
        Ok(Leg {
            role: if self.bonus { Role::Bonus } else { Role::Cover },
            bookmaker: self.bookmaker.clone(),
            outcome: self.outcome()?,
            odds: self.odds()?,
            stake: Euros::new(required(&self.stake, &format!("mise {what}"))?)
                .filter(|s| s.get() > 0.0)
                .ok_or_else(|| format!("mise {what} : elle doit être positive"))?,
        })
    }
}

/// Formulaire « J'ai joué cette opération » : prérempli avec la proposition, à corriger avec les cotes réellement
/// affichées. Les paris sont numérotés dans les champs (`cote0`, `mise0`, `bookmaker1`…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BonusForm {
    /// Opérateur du bonus.
    pub bookmaker: String,
    /// Forme de l'opération (`freebet_refund`…).
    pub kind: String,
    /// Match de l'archive.
    pub event_id: String,
    /// Le pari bonus puis les couvertures.
    pub legs: Vec<BonusLegForm>,
}

impl BonusForm {
    /// Préremplit le formulaire avec une opération proposée.
    pub fn from_candidate(target: &Target, candidate: &Candidate) -> BonusForm {
        BonusForm {
            bookmaker: target.bookmaker.clone(),
            kind: target.kind.as_str().to_string(),
            event_id: candidate.event.event_id.clone(),
            legs: candidate
                .legs
                .iter()
                .map(|leg| BonusLegForm {
                    bonus: leg.role == Role::Bonus,
                    bookmaker: leg.bookmaker.clone(),
                    outcome: outcome_code(leg.outcome).to_string(),
                    outcome_label: format::outcome_label(leg.outcome),
                    odds: format::decimal(leg.odds.get(), 2),
                    stake: format::decimal(leg.stake.get(), 2),
                })
                .collect(),
        }
    }

    /// Lit les champs envoyés : `paris` paris numérotés de 0 à `paris − 1` (trois au plus).
    pub fn from_form(form: &Form) -> BonusForm {
        let count = form.get("paris").parse::<usize>().unwrap_or(0).min(3);
        let legs = (0..count)
            .map(|i| {
                let outcome = form.get(&format!("issue{i}")).to_string();
                BonusLegForm {
                    bonus: i == 0,
                    bookmaker: form.get(&format!("bookmaker{i}")).to_string(),
                    outcome_label: outcome_from(&outcome).map_or("?", format::outcome_label),
                    outcome,
                    odds: form.get(&format!("cote{i}")).to_string(),
                    stake: form.get(&format!("mise{i}")).to_string(),
                }
            })
            .collect();
        BonusForm {
            bookmaker: form.get("operateur").to_string(),
            kind: form.get("forme").to_string(),
            event_id: form.get("match").to_string(),
            legs,
        }
    }

    fn offer_kind(&self) -> Result<OfferKind, String> {
        OfferKind::parse(&self.kind).map_err(|e| e.to_string())
    }

    fn legs(&self) -> Result<Vec<Leg>, String> {
        self.legs.iter().map(BonusLegForm::to_leg).collect()
    }

    /// Recalcule les mises de couverture sur les cotes saisies (la mise du bonus ne change pas).
    pub fn recompute(&mut self, freebet_value: f64) -> Result<(), String> {
        let bonus = self
            .legs
            .iter()
            .find(|l| l.bonus)
            .ok_or("le pari bonus manque")?
            .to_leg()?;
        // Des couvertures, seules les cotes comptent : leurs mises sont justement ce que l'on recalcule.
        let others = self
            .legs
            .iter()
            .filter(|l| !l.bonus)
            .map(BonusLegForm::odds)
            .collect::<Result<Vec<Odds>, String>>()?;
        let result = cover(
            self.offer_kind()?,
            bonus.stake,
            bonus.odds,
            freebet_value,
            &others,
        )
        .map_err(|e| e.to_string())?;
        let covers = self.legs.iter_mut().filter(|l| !l.bonus);
        for (leg, stake) in covers.zip(&result.stakes) {
            leg.stake = format::decimal(stake.get(), 2);
        }
        Ok(())
    }

    /// Gain garanti des valeurs saisies, ou `None` tant qu'une valeur est illisible.
    pub fn guaranteed(&self, freebet_value: f64) -> Option<f64> {
        let kind = self.offer_kind().ok()?;
        let value = if kind == OfferKind::FreebetRefund {
            freebet_value
        } else {
            0.0
        };
        self.legs()
            .ok()
            .map(|legs| guaranteed_gain(kind, &legs, value))
    }

    /// Valide la saisie. Les règles du journal (offre consommée, compte de couverture…) sont vérifiées par le cœur.
    pub fn to_action(&self, freebet_value: f64) -> Result<Action, String> {
        Ok(Action::LogBonus(OperationRequest {
            bookmaker: self.bookmaker.clone(),
            kind: Some(self.offer_kind()?),
            event_id: self.event_id.clone(),
            legs: self.legs()?,
            freebet_value,
        }))
    }
}

/// Formulaire d'enregistrement d'une offre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferForm {
    /// Opérateur.
    pub bookmaker: String,
    /// L'offre est-elle accessible ?
    pub eligible: bool,
    /// Forme de l'offre (`freebet_refund`…).
    pub kind: String,
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
            kind: OfferKind::FreebetRefund.as_str().to_string(),
            amount: "100".to_string(),
            min_odds: String::new(),
            source: String::new(),
            note: String::new(),
        }
    }

    /// Lit les champs envoyés.
    pub fn from_form(form: &Form) -> OfferForm {
        OfferForm {
            bookmaker: form.get("operateur").to_string(),
            eligible: form.get("eligible") != "non",
            kind: form.get("forme").to_string(),
            amount: form.get("montant").to_string(),
            min_odds: form.get("cote_min").to_string(),
            source: form.get("source").to_string(),
            note: form.get("remarque").to_string(),
        }
    }

    /// Valide la saisie ; `today` est la date de vérification enregistrée.
    pub fn to_action(&self, today: NaiveDate) -> Result<Action, String> {
        let bookmaker = self.bookmaker.trim();
        if !crate::bonus::is_operator_key(bookmaker) {
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
                kind: OfferKind::parse(&self.kind).map_err(|e| e.to_string())?,
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

/// Une grille de Grand Loto jouée hors des recommandations, telle que saisie sur la page `/loto`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct LotoGridForm {
    /// Les cinq numéros puis la chance, dans n'importe quelle écriture (`2-3-30-35-44+10`, `2 3 30 35 44 10`…).
    pub grid: String,
    /// Édition (AAAA-MM-JJ).
    pub date: String,
    /// Mise, en euros.
    pub stake: String,
}

impl LotoGridForm {
    /// Lit les champs envoyés.
    pub fn from_form(form: &Form) -> LotoGridForm {
        LotoGridForm {
            grid: form.get("grille").to_string(),
            date: form.get("edition").to_string(),
            stake: form.get("mise").to_string(),
        }
    }

    /// Valide la saisie : six nombres (cinq numéros de 1 à 49, distincts, puis la chance de 1 à 10), une édition, une mise.
    pub fn to_action(&self) -> Result<Action, String> {
        // Tout ce qui n'est pas un chiffre sépare deux nombres : « 2-3-30-35-44+10 » comme « 2 3 30 35 44 10 ».
        let numbers: Vec<u8> = self
            .grid
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(|part| part.parse().unwrap_or(0))
            .collect();
        let grid = match numbers.as_slice() {
            [a, b, c, d, e, chance] => Grid::new([*a, *b, *c, *d, *e], *chance),
            _ => None,
        }
        .ok_or("grille attendue : cinq numéros distincts de 1 à 49 puis la chance de 1 à 10 (2-3-30-35-44+10)")?;
        let date = NaiveDate::parse_from_str(self.date.trim(), "%Y-%m-%d")
            .map_err(|_| "choisis l'édition dans la liste".to_string())?;
        let stake =
            parse_number(&self.stake, "mise")?.unwrap_or(crate::loto::rules::GRAND_LOTO_PRICE);
        if stake <= 0.0 {
            return Err("la mise doit être strictement positive".to_string());
        }
        Ok(Action::LotoGrid { date, grid, stake })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_follow_form_without_changes_lets_the_recommendation_decide() {
        let form = FollowForm::from_form(&Form::parse("cote=&mise=&bookmaker=+"));
        assert_eq!(
            form.to_action(3).unwrap(),
            Action::LogFollowed {
                recommendation_id: 3,
                input: FollowedInput::default(),
            }
        );
        let typed = FollowForm::from_form(&Form::parse("cote=2%2C25&mise=40&bookmaker=winamax_fr"));
        let Action::LogFollowed { input, .. } = typed.to_action(3).unwrap() else {
            panic!("action inattendue");
        };
        assert_eq!(input.odds, Some(2.25));
        assert_eq!(input.stake, Some(40.0));
        assert_eq!(input.bookmaker.as_deref(), Some("winamax_fr"));
        assert!(
            FollowForm::from_form(&Form::parse("cote=x"))
                .to_action(3)
                .is_err()
        );
    }

    #[test]
    fn an_off_model_form_needs_a_match_an_outcome_a_bookmaker_odds_and_a_stake() {
        let complete = "match=m1&bookmaker=+winamax_fr+&issue=N&cote=3%2C4&mise=5";
        assert_eq!(
            OffModelForm::from_form(&Form::parse(complete)).to_action(),
            Ok(Action::LogOffModel {
                event_id: "m1".into(),
                bookmaker: "winamax_fr".into(),
                outcome: Outcome::Draw,
                odds: 3.4,
                stake: 5.0,
            })
        );
        let missing = |body: &str| {
            OffModelForm::from_form(&Form::parse(body))
                .to_action()
                .unwrap_err()
        };
        assert!(missing("bookmaker=w&issue=N&cote=3&mise=5").contains("match"));
        assert!(missing("match=m1&bookmaker=w&issue=X&cote=3&mise=5").contains("issue"));
        assert!(missing("match=m1&bookmaker=+&issue=N&cote=3&mise=5").contains("bookmaker"));
        assert!(missing("match=m1&bookmaker=w&issue=N&mise=5").contains("cote"));
        assert!(missing("match=m1&bookmaker=w&issue=N&cote=3&mise=x").contains("mise"));
    }

    #[test]
    fn a_bonus_form_recomputes_cover_stakes_from_the_typed_odds() {
        let body = "operateur=betclic_fr&forme=freebet&match=m1&paris=3\
            &bookmaker0=betclic_fr&issue0=2&cote0=4%2C2&mise0=100\
            &bookmaker1=winamax_fr&issue1=1&cote1=1%2C85&mise1=0\
            &bookmaker2=winamax_fr&issue2=N&cote2=3%2C90&mise2=0";
        let mut form = BonusForm::from_form(&Form::parse(body));
        assert!(form.legs[0].bonus && !form.legs[1].bonus);
        // Mises nulles : illisibles pour le gain garanti, mais le recalcul les remplace.
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
        let ineligible = OfferForm::from_form(&Form::parse(
            "operateur=pmu_fr&eligible=non&montant=0&source=service+client",
        ));
        assert!(matches!(
            ineligible.to_action(today),
            Ok(Action::RecordOffer(NewOffer {
                eligibility: Eligibility::Ineligible,
                ..
            }))
        ));
        form.bookmaker = "Betclic".into();
        assert!(form.to_action(today).unwrap_err().contains("clé"));
    }

    #[test]
    fn a_loto_grid_is_read_in_any_writing() {
        let action = |grid: &str, date: &str, stake: &str| {
            LotoGridForm {
                grid: grid.to_string(),
                date: date.to_string(),
                stake: stake.to_string(),
            }
            .to_action()
        };
        let expected = Action::LotoGrid {
            date: NaiveDate::from_ymd_opt(2026, 12, 25).unwrap(),
            grid: Grid::parse("2-3-30-35-44+10").unwrap(),
            stake: 5.0,
        };
        assert_eq!(
            action("44 35 30 3 2 + 10", "2026-12-25", ""),
            Ok(expected.clone())
        );
        assert_eq!(action("2-3-30-35-44+10", "2026-12-25", "5"), Ok(expected));
        for (grid, date, stake) in [
            ("2-3-30-35+10", "2026-12-25", "5"),
            ("2-2-30-35-44+10", "2026-12-25", "5"),
            ("2-3-30-35-50+10", "2026-12-25", "5"),
            ("2-3-30-35-44+11", "2026-12-25", "5"),
            ("2-3-30-35-44+10", "", "5"),
            ("2-3-30-35-44+10", "2026-12-25", "0"),
            ("2-3-30-35-44+10", "2026-12-25", "cinq"),
        ] {
            assert!(action(grid, date, stake).is_err(), "{grid} {date} {stake}");
        }
    }
}
