//! Page `/bonus` du site : où aller jouer un bonus, et avec quelles couvertures. Lecture seule, aucune saisie.
//!
//! Le contenu vient de [`crate::bonus::tracking::board`], comme l'écran Bonus de l'application et `edge bonus status` :
//! ce module ne fait que le mettre en forme pour le gabarit `site_bonus.html`.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::recommend_report::league_name;
use crate::bonus::operator_name;
use crate::bonus::pick::Candidate;
use crate::bonus::tracking::{Board, OfferState, OperatorStatus, Plan, realized_gain};
use crate::format::{age, decimal, kickoff_label, outcome_label, signed, until};
use crate::store::bonus::{Eligibility, Leg, MatchResult, Role, StoredOperation};

/// Opérations proposées par offre sur la page.
const SHOWN_PER_PLAN: usize = 3;

/// Un pari d'une opération.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LegView {
    /// `Bonus` ou `Couverture`.
    pub role: &'static str,
    /// Opérateur.
    pub bookmaker: String,
    /// Issue.
    pub outcome: &'static str,
    /// Cote.
    pub odds: String,
    /// Mise.
    pub stake: String,
}

/// Une opération proposée.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CandidateView {
    /// `Domicile – Extérieur`.
    pub fixture: String,
    /// Coup d'envoi, heure de Paris.
    pub kickoff: String,
    /// Délai avant le coup d'envoi.
    pub until: String,
    /// Compétition.
    pub league: String,
    /// Âge des cotes utilisées.
    pub age: String,
    /// Gain garanti.
    pub guaranteed: String,
    /// Argent sorti.
    pub cash_out: String,
    /// Le pari bonus puis les couvertures.
    pub legs: Vec<LegView>,
}

/// Une offre à jouer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanView {
    /// Opérateur.
    pub name: String,
    /// Forme de l'offre.
    pub kind: &'static str,
    /// Montant.
    pub amount: String,
    /// Cote minimale de l'offre, s'il y en a une.
    pub min_odds: Option<String>,
    /// Vrai si l'archive a les cotes de l'opérateur.
    pub in_archive: bool,
    /// Opérations proposées.
    pub candidates: Vec<CandidateView>,
}

/// L'état d'un opérateur.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperatorView {
    /// Nom.
    pub name: String,
    /// État de l'offre.
    pub state: &'static str,
    /// Classe CSS de l'état (`won` : disponible, `open` : autre).
    pub css: &'static str,
    /// Offre en vigueur (`remboursé en freebet jusqu'à 100,00 €`), vide si aucune.
    pub offer: String,
    /// Date et source de la vérification, vide si aucune.
    pub checked: String,
    /// Freebet à jouer et gain réalisé, vide si rien.
    pub extra: String,
}

/// Une opération enregistrée.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationView {
    /// Numéro.
    pub id: i64,
    /// `Domicile – Extérieur`.
    pub fixture: String,
    /// Coup d'envoi, heure de Paris.
    pub kickoff: String,
    /// Forme et opérateur.
    pub what: String,
    /// Gain garanti à l'enregistrement.
    pub guaranteed: String,
    /// `ouverte`, ou `réglée (domicile) : +52,66 €`.
    pub state: String,
    /// Classe CSS de l'état.
    pub css: &'static str,
}

/// La page `/bonus`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BonusPage {
    /// Barre de navigation.
    pub nav: bool,
    /// Lien actif.
    pub active: &'static str,
    /// Instant de calcul, heure de Paris.
    pub generated_at: String,
    /// Gain réalisé.
    pub realized: String,
    /// Gain garanti des opérations ouvertes.
    pub open_guaranteed: String,
    /// Comptes de couverture, ou une phrase qui dit qu'il n'y en a pas.
    pub cover_accounts: String,
    /// Offres à jouer.
    pub plans: Vec<PlanView>,
    /// Opérateurs.
    pub operators: Vec<OperatorView>,
    /// Opérations, de la plus récente à la plus ancienne.
    pub operations: Vec<OperationView>,
}

/// Met en forme le tableau de bord.
pub fn view(board: &Board, generated_at: String, now: DateTime<Utc>) -> BonusPage {
    let covers: Vec<&str> = board
        .cover_accounts
        .iter()
        .map(|k| operator_name(k))
        .collect();
    BonusPage {
        nav: true,
        active: "bonus",
        generated_at,
        realized: euros_signed(board.realized),
        open_guaranteed: euros_signed(board.open_guaranteed),
        cover_accounts: if covers.is_empty() {
            "aucun (enregistre au moins un opérateur inéligible, Winamax par exemple)".to_string()
        } else {
            covers.join(", ")
        },
        plans: board
            .plans
            .iter()
            .map(|plan| plan_view(plan, now))
            .collect(),
        operators: board.operators.iter().map(operator_view).collect(),
        operations: board.operations.iter().rev().map(operation_view).collect(),
    }
}

fn euros(amount: f64) -> String {
    format!("{} €", decimal(amount, 2))
}

fn euros_signed(amount: f64) -> String {
    format!("{} €", signed(amount, 2))
}

fn plan_view(plan: &Plan, now: DateTime<Utc>) -> PlanView {
    PlanView {
        name: plan.name.clone(),
        kind: plan.target.kind.label(),
        amount: euros(plan.target.amount.get()),
        min_odds: plan.target.min_odds.map(|m| decimal(m.get(), 2)),
        in_archive: plan.in_archive,
        candidates: plan
            .candidates
            .iter()
            .take(SHOWN_PER_PLAN)
            .map(|c| candidate_view(c, now))
            .collect(),
    }
}

fn candidate_view(candidate: &Candidate, now: DateTime<Utc>) -> CandidateView {
    let event = &candidate.event;
    CandidateView {
        fixture: format!("{} – {}", event.home_team, event.away_team),
        kickoff: kickoff_label(event.kickoff),
        until: until(event.kickoff, now),
        league: league_name(&event.sport_key),
        age: age(candidate.priced_at, now),
        guaranteed: euros_signed(candidate.cover.guaranteed),
        cash_out: euros(candidate.cover.cash_out),
        legs: candidate.legs.iter().map(leg_view).collect(),
    }
}

fn leg_view(leg: &Leg) -> LegView {
    LegView {
        role: match leg.role {
            Role::Bonus => "Bonus",
            Role::Cover => "Couverture",
        },
        bookmaker: operator_name(&leg.bookmaker).to_string(),
        outcome: outcome_label(leg.outcome),
        odds: decimal(leg.odds.get(), 2),
        stake: euros(leg.stake.get()),
    }
}

fn operator_view(status: &OperatorStatus) -> OperatorView {
    let (offer, checked) = match &status.offer {
        Some(stored) => (
            match &stored.offer.eligibility {
                Eligibility::Eligible {
                    kind,
                    amount,
                    min_odds,
                } => format!(
                    "{} jusqu'à {}{}",
                    kind.label(),
                    euros(amount.get()),
                    min_odds.map_or(String::new(), |m| format!(
                        ", cote ≥ {}",
                        decimal(m.get(), 2)
                    ))
                ),
                Eligibility::Ineligible => "compte de couverture".to_string(),
            },
            format!(
                "vérifié le {} ({})",
                stored.offer.checked_on.format("%d/%m/%Y"),
                stored.offer.source
            ),
        ),
        None => (String::new(), String::new()),
    };
    let mut extra = Vec::new();
    if let Some(freebet) = status.pending_freebet {
        extra.push(format!("freebet de {} à jouer", euros(freebet.get())));
    }
    if status.realized != 0.0 {
        extra.push(format!("réalisé {}", euros_signed(status.realized)));
    }
    OperatorView {
        name: status.name.clone(),
        state: status.state.label(),
        css: if status.state == OfferState::Available {
            "won"
        } else {
            "open"
        },
        offer,
        checked,
        extra: extra.join(" · "),
    }
}

fn operation_view(stored: &StoredOperation) -> OperationView {
    let op = &stored.operation;
    let (state, css) = match (&stored.settled, realized_gain(stored)) {
        (Some(settled), Some(gain)) => (
            format!(
                "réglée ({}) : {}",
                match settled.result {
                    MatchResult::Played(outcome) => outcome_label(outcome),
                    MatchResult::Void => "annulé",
                },
                euros_signed(gain)
            ),
            if gain >= 0.0 { "won" } else { "lost" },
        ),
        _ => ("ouverte".to_string(), "open"),
    };
    OperationView {
        id: stored.id,
        fixture: format!("{} – {}", op.event.home_team, op.event.away_team),
        kickoff: kickoff_label(op.event.kickoff),
        what: format!(
            "{} chez {}",
            op.kind.label(),
            operator_name(&stored.bonus_leg().bookmaker)
        ),
        guaranteed: euros_signed(op.guaranteed),
        state,
        css,
    }
}
