//! Rapport des recommandations d'une exécution : celui qui est joint à la notification.
//!
//! Les champs sont déjà mis en forme (virgule décimale, heure de Paris) : le gabarit n'a plus qu'à les afficher.

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;

use crate::betting::kelly::{kelly_fraction as kelly_fraction_of, to_units};
use crate::betting::live::Opportunity;
use crate::betting::recommend::Report;
use crate::format::{decimal, kickoff_label, outcome_label, signed, units};
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

/// Le meilleur prix d'un match, même moins bon que le marché : une ligne du tableau « tous les matchs ».
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PriceRow {
    /// Coup d'envoi, heure de Paris.
    pub kickoff: String,
    /// Compétition.
    pub league: String,
    /// `Domicile – Extérieur`.
    pub fixture: String,
    /// Bookmaker du meilleur prix.
    pub bookmaker: String,
    /// Issue concernée.
    pub outcome: String,
    /// Cote proposée.
    pub odds: String,
    /// Cote juste selon Pinnacle sans marge.
    pub fair_odds: String,
    /// Avantage en pourcentage signé.
    pub edge: String,
    /// Mise Kelly d'un pari seul, en unités (0 sans avantage). Indicative sous le seuil.
    pub kelly: String,
    /// L'avantage atteint-il le seuil ?
    pub above_threshold: bool,
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
    /// Le meilleur prix de chaque match comparé à Pinnacle, du plus proche de la valeur au plus éloigné.
    pub prices: Vec<PriceRow>,
    /// Pinnacle manque sur la moitié des matchs ou plus : le relevé n'a pas pu être évalué.
    pub reference_missing: bool,
    /// Candidats déjà inscrits au journal par une exécution précédente (non répétés dans ce rapport).
    pub already_known: usize,
    /// Afficher la barre de navigation du site : vrai pour la copie archivée, faux pour la pièce jointe Discord.
    pub nav: bool,
}

fn price_row(found: &Opportunity, min_edge: f64, kelly: f64) -> PriceRow {
    let s = &found.selection;
    PriceRow {
        kickoff: kickoff_label(found.kickoff),
        league: league_name(&found.sport_key),
        fixture: format!("{} – {}", found.home_team, found.away_team),
        bookmaker: found.bookmaker.clone(),
        outcome: outcome_label(s.outcome).to_string(),
        odds: decimal(s.odds.get(), 2),
        fair_odds: decimal(1.0 / s.probability.get(), 2),
        edge: format!("{} %", signed(s.edge * 100.0, 1)),
        kelly: units(to_units(kelly_fraction_of(s.probability, s.odds) * kelly)),
        above_threshold: s.edge >= min_edge,
    }
}

/// Nom affiché d'une compétition : la ligue de football, « Tennis » ou « NBA », sinon la clé de The Odds API elle-même.
pub fn league_name(sport_key: &str) -> String {
    if let Some(league) = LEAGUES.iter().find(|l| l.sport_key == sport_key) {
        return league.name.to_string();
    }
    if sport_key.starts_with("tennis_") {
        return "Tennis".to_string();
    }
    if sport_key.starts_with("basketball_") {
        return "NBA".to_string();
    }
    sport_key.to_string()
}

/// Une recommandation du journal, mise en forme.
pub(crate) fn row(found: &StoredRecommendation) -> Row {
    let r = &found.record;
    Row {
        id: found.id,
        kickoff: kickoff_label(r.kickoff),
        league: league_name(&r.sport_key),
        fixture: format!("{} – {}", r.home_team, r.away_team),
        bookmaker: r.bookmaker.clone(),
        outcome: outcome_label(r.outcome).to_string(),
        odds: decimal(r.odds.get(), 2),
        fair_odds: decimal(1.0 / r.probability.get(), 2),
        edge: format!("{} %", signed(r.edge * 100.0, 1)),
        minimal_odds: decimal(r.minimal_odds, 2),
        stake: r.stake.map_or_else(
            || "sous le minimum".to_string(),
            |s| {
                format!(
                    "{} ({} €)",
                    units(to_units(s.get() / r.bankroll.get())),
                    decimal(s.get(), 1)
                )
            },
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
        prices: report
            .priced
            .iter()
            .map(|o| price_row(o, min_edge, kelly_fraction))
            .collect(),
        reference_missing: report.events == 0 || report.without_reference * 2 >= report.events,
        already_known: report.already_known,
        nav: false,
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
            excluded_bookmakers: Vec::new(),
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
        assert!(
            html.contains("9,1 u (91,4 €)"),
            "mise en unités puis en euros"
        );
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
            prices: Vec::new(),
            reference_missing: false,
            already_known: 0,
            nav: false,
        };

        let html = render_recommend_report(&data).unwrap();

        assert!(html.contains("Aucune nouvelle recommandation"));
        assert!(html.contains("2 ligne(s) ajoutée(s) au journal fantôme"));
        assert!(
            !html.contains("Tous les matchs examinés"),
            "pas de tableau sans prix"
        );
    }

    #[test]
    fn the_report_lists_every_match_even_below_the_market_and_flags_a_missing_pinnacle() {
        let mut conn = database();
        // Betclic à 1,95 (cote juste 2,00 contre Pinnacle) : avantage négatif, mais le match doit figurer au tableau.
        store(
            &mut conn,
            "2026-10-18T14:00:00Z",
            &[event(&[PINNACLE, ("betclic_fr", 1.95, 3.1, 3.3)])],
        );
        let settings = Settings {
            method: MarginMethod::Multiplicative,
            min_edge: RECOMMEND_MIN_EDGE,
            excluded_bookmakers: Vec::new(),
            kelly_fraction: 1.0,
            min_stake: Euros::new(1.0).unwrap(),
            bankroll: Euros::new(1000.0).unwrap(),
            code_version: "test".into(),
        };
        let report = run(&mut conn, &settings, at(NOW)).unwrap();
        assert!(report.created.is_empty(), "rien n'est journalisé sous 0 %");

        let data = build(&report, RECOMMEND_MIN_EDGE, 1.0, 1000.0, at(NOW));
        let html = render_recommend_report(&data).unwrap();

        assert_eq!(data.prices.len(), 1);
        assert!(!data.prices[0].above_threshold);
        assert_eq!(data.prices[0].kelly, "0,0 u", "pas d'avantage, pas de mise");
        assert!(
            data.prices[0].edge.starts_with('-'),
            "{}",
            data.prices[0].edge
        );
        assert!(html.contains("Tous les matchs examinés"));
        assert!(html.contains("Falkirk – Kilmarnock"));
        assert!(!html.contains("Pinnacle manque"));

        let mut broken = data.clone();
        broken.reference_missing = true;
        broken.without_reference = 60;
        assert!(
            render_recommend_report(&broken)
                .unwrap()
                .contains("Pinnacle manque sur 60 match(s)")
        );
    }
}
