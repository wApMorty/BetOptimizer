//! Page `/latence` de l'application Web : état du test prospectif de la latence française (phase 13). Lecture seule.
//!
//! Le calcul vient de [`crate::eval::latency::evaluate`], comme `edge latency report` : ce module ne fait que le mettre
//! en forme pour le gabarit `latency.html` de l'application Web.

use serde::Serialize;

use crate::eval::bootstrap::MeanCi;
use crate::eval::latency::{
    Assessment, LatencyReport, MIN_BETS, Verdict, deadline, league_label, test_start,
};
use crate::format::{kickoff_label, outcome_label, signed};

/// Paris affichés, du plus récent au plus ancien.
const SHOWN_BETS: usize = 30;

/// Un pari de la règle.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BetView {
    /// Coup d'envoi, heure de Paris.
    pub kickoff: String,
    /// `Domicile – Extérieur`.
    pub fixture: String,
    /// Championnat.
    pub league: String,
    /// Opérateur.
    pub bookmaker: String,
    /// Horizon de décision (`60 min`).
    pub horizon: String,
    /// Issue.
    pub outcome: &'static str,
    /// Cote.
    pub odds: String,
    /// Avantage annoncé.
    pub edge: String,
    /// CLV.
    pub clv: String,
}

/// Résumé d'une lecture (principale ou secondaire).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SummaryView {
    /// Filtre de suivi appliqué.
    pub min_changes: usize,
    /// Matchs du test.
    pub matches: usize,
    /// Couples match × opérateur suivis.
    pub tracked: usize,
    /// Nombre de paris.
    pub bets: usize,
    /// CLV moyenne.
    pub clv: String,
    /// Intervalle à 95 %.
    pub ci: String,
}

/// La page `/latence`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LatencyPage {
    /// Début du test et échéance.
    pub start: String,
    /// Échéance.
    pub deadline: String,
    /// Paris nécessaires au verdict.
    pub min_bets: usize,
    /// Verdict en clair.
    pub verdict: String,
    /// Classe CSS du verdict (`warn` en attente, `bad` en échec ou trop rare, vide en succès).
    pub verdict_class: &'static str,
    /// Lecture principale.
    pub primary: SummaryView,
    /// Lecture secondaire.
    pub secondary: SummaryView,
    /// Paris de la lecture principale.
    pub bets: Vec<BetView>,
    /// Cotes comparées à notre archive.
    pub compared: usize,
    /// Dont identiques.
    pub identical: usize,
    /// Part identique, ou un tiret.
    pub identical_share: String,
    /// Noms absents de la table d'alias.
    pub unknown_names: Vec<String>,
}

fn percent(value: f64) -> String {
    format!("{} %", signed(value * 100.0, 1))
}

fn ci_text(ci: Option<MeanCi>) -> (String, String) {
    ci.map_or(("—".into(), "—".into()), |c| {
        (
            percent(c.mean),
            format!(
                "[{} ; {}] %",
                signed(c.low * 100.0, 1),
                signed(c.high * 100.0, 1)
            ),
        )
    })
}

fn summary(a: &Assessment) -> SummaryView {
    let (clv, ci) = ci_text(a.clv);
    SummaryView {
        min_changes: a.min_changes,
        matches: a.matches,
        tracked: a.tracked,
        bets: a.bets.len(),
        clv,
        ci,
    }
}

/// Verdict en clair et classe CSS, communs au site et à la CLI.
pub fn verdict_text(a: &Assessment) -> (String, &'static str) {
    match a.verdict {
        Verdict::Pending => (
            format!(
                "En attente : {} pari(s) sur {MIN_BETS} nécessaires (échéance le {}).",
                a.bets.len(),
                deadline().format("%d/%m/%Y")
            ),
            "warn",
        ),
        Verdict::Success => (
            format!(
                "SUCCÈS sur les {MIN_BETS} premiers paris : la latence française est mesurable et positive."
            ),
            "",
        ),
        Verdict::Failure => (
            format!("ÉCHEC sur les {MIN_BETS} premiers paris : pas d'avantage démontré."),
            "bad",
        ),
        Verdict::TooRare => (
            format!(
                "Signal trop rare : moins de {MIN_BETS} paris au {}, inutile en pratique.",
                deadline().format("%d/%m/%Y")
            ),
            "bad",
        ),
    }
}

/// Met en forme le rapport pour le gabarit.
pub fn view(report: &LatencyReport) -> LatencyPage {
    let (verdict, verdict_class) = verdict_text(&report.primary);
    let bets = report
        .primary
        .bets
        .iter()
        .rev()
        .take(SHOWN_BETS)
        .map(|b| BetView {
            kickoff: kickoff_label(b.kickoff),
            fixture: format!("{} – {}", b.home, b.away),
            league: league_label(b.tournament_id),
            bookmaker: b.bookmaker.clone(),
            horizon: format!("{} min", b.horizon_minutes),
            outcome: outcome_label(b.outcome),
            odds: format!("{:.2}", b.odds.get()).replace('.', ","),
            edge: percent(b.edge),
            clv: percent(b.clv),
        })
        .collect();
    let q = &report.quality;
    LatencyPage {
        start: test_start().format("%d/%m/%Y").to_string(),
        deadline: deadline().format("%d/%m/%Y").to_string(),
        min_bets: MIN_BETS,
        verdict,
        verdict_class,
        primary: summary(&report.primary),
        secondary: summary(&report.secondary),
        bets,
        compared: q.compared,
        identical: q.identical,
        identical_share: if q.compared == 0 {
            "—".into()
        } else {
            // Conversion de comptes en flottants pour un pourcentage affiché : aucune perte qui compte.
            format!("{:.0} %", 100.0 * q.identical as f64 / q.compared as f64)
        },
        unknown_names: q.unknown_names.iter().cloned().collect(),
    }
}
