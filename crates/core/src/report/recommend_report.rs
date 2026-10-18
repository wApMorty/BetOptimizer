//! Rapport des recommandations d'une exécution : celui qui est joint à la notification.
//!
//! Les champs sont déjà mis en forme (virgule décimale, heure de Paris) : le gabarit n'a plus qu'à les afficher.

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;

use crate::betting::recommend::Report;
use crate::format::{decimal, kickoff_label, outcome_label, signed};
use crate::ingest::football_data::LEAGUES;
use crate::store::journal::StoredRecommendation;

/// Une recommandation, prête à afficher.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// Numéro dans le journal (sert à `edge bet log --rec`).
    pub id: i64,
    /// Coup d'envoi, heure de Paris.
    pub kickoff: String,
    /// Compétition.
    pub league: String,
    /// `Domicile – Extérieur`.
    pub fixture: String,
    /// Bookmaker.
    pub bookmaker: String,
    /// Issue recommandée.
    pub outcome: String,
    /// Cote proposée.
    pub odds: String,
    /// Cote juste selon Pinnacle sans marge.
    pub fair_odds: String,
    /// Avantage en pourcentage signé.
    pub edge: String,
    /// Cote minimale.
    pub minimal_odds: String,
    /// Mise suggérée.
    pub stake: String,
}

/// Tout ce que le rapport affiche.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecommendReport {
    /// Date et heure de génération, heure de Paris.
    pub generated_at: String,
    /// Recommandations notifiées.
    pub rows: Vec<Row>,
    /// Lignes ajoutées au journal fantôme pendant cette exécution.
    pub shadow_count: usize,
    /// Matchs à venir examinés.
    pub events: usize,
    /// Matchs sans référence Pinnacle exploitable.
    pub without_reference: usize,
    /// Seuil d'avantage, en pourcentage.
    pub min_edge: String,
    /// Fraction de Kelly.
    pub kelly_fraction: String,
    /// Bankroll utilisée pour les mises.
    pub bankroll: String,
}

fn row(found: &StoredRecommendation) -> Row {
    let r = &found.record;
    Row {
        id: found.id,
        kickoff: kickoff_label(r.kickoff),
        league: LEAGUES
            .iter()
            .find(|league| league.sport_key == r.sport_key)
            .map_or_else(|| r.sport_key.clone(), |league| league.name.to_string()),
        fixture: format!("{} – {}", r.home_team, r.away_team),
        bookmaker: r.bookmaker.clone(),
        outcome: outcome_label(r.outcome).to_string(),
        odds: decimal(r.odds.get(), 2),
        fair_odds: decimal(1.0 / r.probability.get(), 2),
        edge: format!("{} %", signed(r.edge * 100.0, 1)),
        minimal_odds: decimal(r.minimal_odds, 2),
        stake: r.stake.map_or_else(
            || "sous le minimum".to_string(),
            |s| format!("{} €", decimal(s.get(), 1)),
        ),
    }
}

/// Construit le rapport d'une exécution de `edge recommend`.
pub fn build(
    report: &Report,
    min_edge: f64,
    kelly_fraction: f64,
    bankroll: f64,
    now: DateTime<Utc>,
) -> RecommendReport {
    RecommendReport {
        generated_at: now
            .with_timezone(&Paris)
            .format("%d/%m/%Y %H:%M")
            .to_string(),
        rows: report.recommended().map(row).collect(),
        shadow_count: report.shadow().count(),
        events: report.events,
        without_reference: report.without_reference,
        min_edge: decimal(min_edge * 100.0, 1),
        kelly_fraction: decimal(kelly_fraction, 2),
        bankroll: decimal(bankroll, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::betting::recommend::{RECOMMEND_MIN_EDGE, Settings, run};
    use crate::betting::testing::{NOW, PINNACLE, at, database, event, store};
    use crate::domain::Euros;
    use crate::market::margin::MarginMethod;
    use crate::report::render::render_recommend_report;

    #[test]
    fn the_report_shows_the_pick_the_minimal_odds_the_stake_and_the_warning() {
        let mut conn = database();
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 2.3, 3.2, 3.4)])],
        );
        let settings = Settings {
            method: MarginMethod::Multiplicative,
            min_edge: RECOMMEND_MIN_EDGE,
            kelly_fraction: 1.0,
            min_stake: Euros::new(1.0).unwrap(),
            bankroll: Euros::new(1000.0).unwrap(),
            code_version: "test".into(),
        };
        let report = run(&mut conn, &settings, at(NOW)).unwrap();

        let data = build(&report, RECOMMEND_MIN_EDGE, 1.0, 1000.0, at(NOW));
        let html = render_recommend_report(&data).unwrap();

        assert_eq!(data.rows.len(), 1);
        assert_eq!(data.rows[0].league, "Premiership écossaise");
        assert!(html.contains("Falkirk – Kilmarnock"));
        assert!(html.contains("betclic_fr"));
        assert!(html.contains("+11,9 %"));
        assert!(html.contains("91,4 €"));
        assert!(html.contains("Expérimental"));
        // Le gabarit échappe `/` en `&#x2f;` (affiché normalement par le navigateur) : on ne teste que l'heure.
        assert!(html.contains("17:00"), "heure de Paris (UTC+2)");
        assert_eq!(data.generated_at, "18/10/2026 17:00");
    }

    #[test]
    fn without_any_recommendation_the_report_says_so() {
        let data = RecommendReport {
            generated_at: "18/10/2026 17:00".into(),
            rows: Vec::new(),
            shadow_count: 2,
            events: 60,
            without_reference: 10,
            min_edge: "3,0".into(),
            kelly_fraction: "1,00".into(),
            bankroll: "1000".into(),
        };

        let html = render_recommend_report(&data).unwrap();

        assert!(html.contains("Aucune nouvelle recommandation"));
        assert!(html.contains("2 ligne(s) ajoutée(s) au journal fantôme"));
    }
}
