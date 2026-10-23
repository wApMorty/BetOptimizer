//! Onglet « Bonus » (`/bonus`) : offres de bienvenue à jouer, opérations couvertes proposées sur le dernier relevé,
//! calculateur, suivi des offres et des opérations.
//!
//! Tout vient de [`crate::bonus::tracking::board`], le même tableau que `edge bonus status`. Le calculateur est un
//! formulaire `GET` : ses champs reviennent dans l'adresse, la page les relit et affiche le résultat, rien n'est écrit.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::bonus::cover::{Cover, cover};
use crate::bonus::pick::Candidate;
use crate::bonus::tracking::{self, OfferState, OperatorStatus, Plan};
use crate::bonus::{OfferKind, operator_name};
use crate::domain::{Euros, Odds};
use crate::error::Result;
use crate::format;
use crate::report::recommend_report::league_name;
use crate::store::bonus::{Eligibility, Leg, MatchResult, Role, StoredOperation};
use crate::store::{self};
use crate::web::App;
use crate::web::form::{Form, encode, parse_number, required};
use crate::web::view::{Flash, Layout, euros, fixture, render, sign_color, signed_euros, when};

const TEMPLATE: &str = include_str!("../templates/bonus.html");

/// Opérations proposées par offre (la CLI `edge bonus pick` en montre davantage).
const SHOWN_PER_PLAN: usize = 3;

/// Libellés courts des formes d'offre, dans l'ordre de [`OfferKind::ALL`].
pub const KIND_LABELS: [&str; 3] = ["Remb. argent", "Remb. freebet", "Freebet"];

/// Un pari d'une opération, en pastille (`Betclic · domicile 2,63 · 100,00 €`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LegChip {
    /// Texte.
    pub text: String,
    /// Le pari bonus (mis en avant) ?
    pub bonus: bool,
}

/// Les pastilles des paris d'une opération.
pub fn leg_chips(legs: &[Leg]) -> Vec<LegChip> {
    legs.iter()
        .map(|leg| LegChip {
            text: format!(
                "{} · {} {} · {}",
                operator_name(&leg.bookmaker),
                format::outcome_label(leg.outcome),
                format::decimal(leg.odds.get(), 2),
                euros(leg.stake.get(), 2)
            ),
            bonus: leg.role == Role::Bonus,
        })
        .collect()
}

/// Une opération proposée.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct CandidateView {
    fixture: String,
    meta: String,
    legs: Vec<LegChip>,
    guaranteed: String,
    color: &'static str,
    cash_out: String,
    /// Adresse du formulaire « J'ai joué », prérempli avec cette opération.
    play: String,
}

/// Une offre à jouer.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct PlanView {
    name: String,
    kind: &'static str,
    amount: String,
    min_odds: Option<String>,
    in_archive: bool,
    candidates: Vec<CandidateView>,
}

/// Un opérateur et l'état de son offre.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct OperatorView {
    key: String,
    name: String,
    state: &'static str,
    tone: &'static str,
    offer: Option<String>,
    checked: Option<String>,
    source: Option<String>,
    freebet: Option<String>,
    realized: Option<String>,
    realized_tone: &'static str,
    has_offer: bool,
}

/// Une opération enregistrée.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct OperationView {
    id: i64,
    fixture: String,
    meta: String,
    legs: Vec<LegChip>,
    guaranteed: String,
    /// Réglée : gain réalisé, sa couleur, et le résultat.
    gain: Option<String>,
    color: &'static str,
    result: String,
}

/// Le calculateur : champs saisis, et résultat ou message d'erreur.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Calculator {
    kinds: Vec<(&'static str, &'static str)>,
    kind: &'static str,
    amount: String,
    odds: String,
    cover1: String,
    cover2: String,
    result: Option<String>,
    detail: Option<String>,
    color: &'static str,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BonusPage {
    #[serde(flatten)]
    layout: Layout,
    realized: String,
    realized_color: &'static str,
    open_guaranteed: String,
    to_play: usize,
    covers: String,
    plans: Vec<PlanView>,
    calc: Calculator,
    operators: Vec<OperatorView>,
    operations: Vec<OperationView>,
}

/// La page ; les champs du calculateur arrivent dans l'adresse.
pub fn page(app: &App, query: &Form, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let freebet_value = app.config.bonus_freebet_value;
    let board = tracking::board(&conn, freebet_value, now)?;
    let covers: Vec<&str> = board
        .cover_accounts
        .iter()
        .map(|k| operator_name(k))
        .collect();
    render(
        "bonus.html",
        TEMPLATE,
        &BonusPage {
            layout: Layout::new("bonus", now, flash),
            realized: signed_euros(board.realized),
            realized_color: sign_color(board.realized),
            open_guaranteed: signed_euros(board.open_guaranteed),
            to_play: board.plans.len(),
            covers: if covers.is_empty() {
                "aucun compte de couverture".to_string()
            } else {
                format!("couverture : {}", covers.join(", "))
            },
            plans: board
                .plans
                .iter()
                .map(|plan| plan_view(plan, now))
                .collect(),
            calc: calculator(query, freebet_value),
            operators: board.operators.iter().map(operator_view).collect(),
            // Les plus récentes d'abord.
            operations: board
                .operations
                .iter()
                .rev()
                .map(|op| operation_view(op, now))
                .collect(),
        },
    )
}

fn plan_view(plan: &Plan, now: DateTime<Utc>) -> PlanView {
    PlanView {
        name: plan.name.clone(),
        kind: plan.target.kind.label(),
        amount: euros(plan.target.amount.get(), 2),
        min_odds: plan
            .target
            .min_odds
            .map(|min| format!("cote ≥ {}", format::decimal(min.get(), 2))),
        in_archive: plan.in_archive,
        candidates: plan
            .candidates
            .iter()
            .take(SHOWN_PER_PLAN)
            .map(|c| candidate_view(plan, c, now))
            .collect(),
    }
}

fn candidate_view(plan: &Plan, candidate: &Candidate, now: DateTime<Utc>) -> CandidateView {
    let event = &candidate.event;
    CandidateView {
        fixture: fixture(&event.home_team, &event.away_team),
        meta: format!(
            "{} · {} · cotes {}",
            league_name(&event.sport_key),
            when(event.kickoff, now),
            format::age(candidate.priced_at, now)
        ),
        legs: leg_chips(&candidate.legs),
        guaranteed: signed_euros(candidate.cover.guaranteed),
        color: sign_color(candidate.cover.guaranteed),
        cash_out: euros(candidate.cover.cash_out, 0),
        play: format!(
            "/bonus/operation?operateur={}&forme={}&match={}",
            encode(&plan.target.bookmaker),
            plan.target.kind.as_str(),
            encode(&event.event_id)
        ),
    }
}

fn operator_view(status: &OperatorStatus) -> OperatorView {
    let tone = match status.state {
        OfferState::Available => "t-pos",
        OfferState::Consumed => "t-acc",
        OfferState::Ineligible => "t-warn",
        OfferState::ToCheck => "",
    };
    let stored = status.offer.as_ref();
    OperatorView {
        key: status.key.clone(),
        name: status.name.clone(),
        state: status.state.label(),
        tone,
        offer: stored.and_then(|s| match &s.offer.eligibility {
            Eligibility::Eligible { kind, amount, .. } => {
                Some(format!("{} · {}", kind.label(), euros(amount.get(), 0)))
            }
            Eligibility::Ineligible => None,
        }),
        checked: stored.map(|s| format!("vérifié le {}", s.offer.checked_on.format("%d/%m/%Y"))),
        source: stored.map(|s| s.offer.source.clone()),
        freebet: status
            .pending_freebet
            .map(|f| format!("freebet {} à jouer", euros(f.get(), 2))),
        realized: (status.realized != 0.0)
            .then(|| format!("réalisé {}", signed_euros(status.realized))),
        realized_tone: if status.realized > 0.0 {
            "t-pos"
        } else {
            "t-neg"
        },
        has_offer: stored.is_some(),
    }
}

fn operation_view(stored: &StoredOperation, now: DateTime<Utc>) -> OperationView {
    let op = &stored.operation;
    let gain = tracking::realized_gain(stored);
    let result = match &stored.settled {
        Some(settled) => {
            let result = match settled.result {
                MatchResult::Played(outcome) => format::outcome_label(outcome),
                MatchResult::Void => "annulé",
            };
            format!(
                "réglée : {result} · garanti {}",
                signed_euros(op.guaranteed)
            )
        }
        None => String::new(),
    };
    OperationView {
        id: stored.id,
        fixture: fixture(&op.event.home_team, &op.event.away_team),
        meta: format!(
            "n° {} · {} chez {} · {}",
            stored.id,
            op.kind.label(),
            operator_name(&stored.bonus_leg().bookmaker),
            when(op.event.kickoff, now)
        ),
        legs: leg_chips(&op.legs),
        guaranteed: format!("garanti {}", signed_euros(op.guaranteed)),
        color: gain.map_or("", sign_color),
        gain: gain.filter(|_| stored.settled.is_some()).map(signed_euros),
        result,
    }
}

/// La forme d'offre nommée dans l'adresse (`freebet_refund`…), le remboursement en freebet par défaut.
pub fn kind_from(text: &str) -> OfferKind {
    OfferKind::parse(text).unwrap_or(OfferKind::FreebetRefund)
}

/// Calcule les mises de couverture d'un bonus : montant, cote du pari bonus, une ou deux cotes de couverture.
fn compute(
    kind: OfferKind,
    query: &Form,
    freebet_value: f64,
) -> std::result::Result<Cover, String> {
    let amount = Euros::new(required(query.get("montant"), "montant")?)
        .ok_or("montant : il doit être positif")?;
    let odds = Odds::new(required(query.get("cote"), "cote du bonus")?)
        .ok_or("cote du bonus : elle doit être supérieure à 1")?;
    let mut others = Vec::new();
    for (index, field) in ["c1", "c2"].iter().enumerate() {
        if let Some(value) = parse_number(query.get(field), &format!("couverture {}", index + 1))? {
            others.push(Odds::new(value).ok_or("une cote de couverture doit être supérieure à 1")?);
        }
    }
    cover(kind, amount, odds, freebet_value, &others).map_err(|e| e.to_string())
}

fn calculator(query: &Form, freebet_value: f64) -> Calculator {
    let kind = kind_from(query.get("forme"));
    let mut calc = Calculator {
        kinds: OfferKind::ALL
            .iter()
            .zip(KIND_LABELS)
            .map(|(k, label)| (k.as_str(), label))
            .collect(),
        kind: kind.as_str(),
        amount: if query.has("montant") {
            query.get("montant").to_string()
        } else {
            "100".to_string()
        },
        odds: query.get("cote").to_string(),
        cover1: query.get("c1").to_string(),
        cover2: query.get("c2").to_string(),
        result: None,
        detail: None,
        color: "",
        error: None,
    };
    if calc.odds.trim().is_empty() {
        return calc;
    }
    match compute(kind, query, freebet_value) {
        Ok(result) => {
            let stakes: Vec<String> = result.stakes.iter().map(|s| euros(s.get(), 2)).collect();
            calc.result = Some(format!("Mises de couverture : {}", stakes.join(" · ")));
            let mut detail = format!(
                "Gain garanti {} · argent sorti {}",
                signed_euros(result.guaranteed),
                euros(result.cash_out, 2)
            );
            if kind == OfferKind::FreebetRefund {
                detail.push_str(&format!(
                    " · freebet compté {} par euro",
                    format::decimal(freebet_value, 2)
                ));
            }
            calc.detail = Some(detail);
            calc.color = sign_color(result.guaranteed);
        }
        Err(message) => calc.error = Some(message),
    }
    calc
}

#[cfg(test)]
mod tests {
    use crate::web::testing::empty;

    #[test]
    fn the_calculator_reads_its_fields_from_the_address() {
        let sample = empty();
        let page = |target: &str| sample.html(target);

        let html = page("/bonus?forme=freebet&montant=50&cote=2&c1=2&c2=");
        assert!(html.contains("Gain garanti +25,00 €"), "{html}");
        assert!(html.contains("value=\"freebet\" checked"));

        assert!(page("/bonus?forme=freebet&montant=50&cote=x").contains("cote du bonus"));
        assert!(page("/bonus?forme=freebet&montant=50&cote=2").contains("couverture"));
        // Rien de saisi : la consigne, pas d'erreur.
        assert!(page("/bonus").contains("Tape la cote du pari bonus"));
    }

    #[test]
    fn without_any_offer_the_page_says_what_to_do() {
        let html = empty().html("/bonus");
        assert!(html.contains("Aucune offre à jouer"), "{html}");
        assert!(html.contains("href=\"/bonus/offre?operateur=betclic_fr\""));
    }
}
