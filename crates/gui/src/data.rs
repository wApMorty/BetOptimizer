//! Les données affichées par l'application : une photographie du journal et de l'archive à un instant donné.
//!
//! Tout est lu par l'API de `edge-core` ; rien n'est calculé ici qui n'existe pas déjà pour la ligne de commande.
//! L'application ne contacte jamais l'API de cotes : elle ne dépense aucun crédit.

use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use edge_core::betting::clv::{self, ClvReport};
use edge_core::betting::ledger::{self, UpcomingEvent};
use edge_core::betting::live::{self, Opportunity};
use edge_core::betting::overview::{self, BankrollCurve, ReliabilityBin};
use edge_core::betting::recommend::RECOMMEND_MIN_EDGE;
use edge_core::bonus::tracking::{self, Board};
use edge_core::config::Config;
use edge_core::domain::Euros;
use edge_core::ingest::aliases::Aliases;
use edge_core::paths::Paths;
use edge_core::report::lotofoot_page::{self, LotofootPage};
use edge_core::store::journal::{self, StoredBet, StoredRecommendation};
use edge_core::store::open_migrated;
use edge_core::store::{archive, history, nba, tennis};

/// Nombre de tranches du diagramme de fiabilité : peu de recommandations réglées, donc peu de tranches.
const RELIABILITY_BINS: usize = 5;

/// Un rapport HTML présent dans le dossier des rapports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportFile {
    /// Nom du fichier.
    pub name: String,
    /// Chemin complet.
    pub path: PathBuf,
    /// Dernière modification.
    pub modified: Option<SystemTime>,
}

/// Fraîcheur d'un flux d'archive (football, tennis, NBA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    /// Nom affiché.
    pub label: &'static str,
    /// Dernier relevé réussi.
    pub last: Option<DateTime<Utc>>,
    /// Éléments archivés (matchs terminés ou importés), quand le flux en compte.
    pub archived: Option<u32>,
}

/// Crédits de l'API de cotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credits {
    /// Crédits utilisés dans la période de quota.
    pub used: u32,
    /// Plafond configuré.
    pub cap: u32,
}

/// Matchs examinés lors de la lecture du dernier relevé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanSummary {
    /// Matchs à venir dans un relevé assez récent.
    pub events: usize,
    /// Matchs ignorés faute d'un 1X2 complet de Pinnacle.
    pub without_reference: usize,
    /// Prix aberrants écartés.
    pub implausible_quotes: usize,
}

/// Tout ce que les onglets affichent.
pub struct Data {
    /// Instant de la lecture.
    pub now: DateTime<Utc>,
    /// Configuration (bookmakers, mises).
    pub config: Config,
    /// Bilan de la lecture du relevé.
    pub scan: ScanSummary,
    /// Prix actuels dont l'avantage atteint le seuil de notification, avec la mise suggérée.
    pub notified: Vec<Opportunity>,
    /// Prix actuels d'avantage positif sous le seuil (journal fantôme), sans mise.
    pub below: Vec<Opportunity>,
    /// Journal des recommandations, de la plus récente à la plus ancienne.
    pub recommendations: Vec<StoredRecommendation>,
    /// Paris réels, du plus récent au plus ancien.
    pub bets: Vec<StoredBet>,
    /// Matchs à venir, pour le formulaire des paris hors modèle.
    pub upcoming: Vec<UpcomingEvent>,
    /// Courbe de bankroll.
    pub curve: BankrollCurve,
    /// CLV réel par groupe et par tranche d'avantage.
    pub clv: ClvReport,
    /// Diagramme de fiabilité des recommandations réglées.
    pub reliability: Vec<ReliabilityBin>,
    /// Rapports HTML disponibles.
    pub reports: Vec<ReportFile>,
    /// Fraîcheur des relevés par sport.
    pub feeds: Vec<Feed>,
    /// Crédits de l'API, quand un relevé les a lus.
    pub credits: Option<Credits>,
    /// Bonus de bienvenue : offres, propositions du dernier relevé, opérations.
    pub bonus: Board,
    /// Loto Foot : dernière séance, bilans et vérification (le contenu de la page `/lotofoot` du site).
    pub lotofoot: LotofootPage,
}

/// Lit le journal et l'archive.
pub fn load(paths: &Paths, now: DateTime<Utc>) -> Result<Data> {
    let conn = open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init` dans un terminal")?;
    let config = Config::load(&paths.config_file)?;
    let aliases = Aliases::bundled()?;
    let initial = Euros::new(config.initial_bankroll).context("initial_bankroll invalide")?;

    let curve = overview::bankroll_curve(&conn, initial)?;
    let available = Euros::new(curve.available).context("bankroll disponible invalide")?;
    let min_stake = Euros::new(config.min_stake).context("min_stake invalide")?;

    let scan = live::scan(
        &conn,
        config.margin_method,
        0.0,
        &config.excluded_bookmakers,
        now,
    )?;
    let summary = ScanSummary {
        events: scan.events,
        without_reference: scan.without_reference,
        implausible_quotes: scan.implausible_quotes,
    };
    let (mut notified, below): (Vec<Opportunity>, Vec<Opportunity>) = scan
        .opportunities
        .into_iter()
        .partition(|o| o.selection.edge >= RECOMMEND_MIN_EDGE);
    live::size_stakes(&mut notified, available, config.kelly_fraction, min_stake)?;

    let mut recommendations = journal::recommendations(&conn)?;
    recommendations.reverse();
    let mut bets = journal::bets(&conn)?;
    bets.reverse();

    let feeds = vec![
        Feed {
            label: "Football",
            last: archive::last_odds_call(&conn, "soccer_")?,
            archived: Some(history::count_games(&conn)?),
        },
        Feed {
            label: "Tennis",
            last: archive::last_odds_call(&conn, "tennis_")?,
            archived: Some(tennis::count_matches(&conn)?),
        },
        Feed {
            label: "NBA",
            last: archive::last_odds_call(&conn, "basketball_")?,
            archived: Some(nba::count_games(&conn)?),
        },
    ];
    let credits = archive::last_known_usage(&conn)?.map(|used| Credits {
        used,
        cap: config.monthly_credit_cap,
    });

    Ok(Data {
        now,
        feeds,
        credits,
        scan: summary,
        notified,
        below,
        recommendations,
        bets,
        upcoming: ledger::upcoming_events(&conn, now)?,
        clv: clv::report(&conn, &aliases, config.margin_method)?,
        reliability: overview::reliability(&conn, &aliases, RELIABILITY_BINS)?,
        reports: report_files(paths),
        bonus: tracking::board(&conn, config.bonus_freebet_value, now)?,
        lotofoot: lotofoot_page::page(&conn, now, edge_core::format::long_date(now))?,
        curve,
        config,
    })
}

/// Les fichiers `.html` du dossier des rapports, du plus récent au plus ancien. Un dossier absent donne une liste vide.
fn report_files(paths: &Paths) -> Vec<ReportFile> {
    let Ok(entries) = std::fs::read_dir(&paths.reports_dir) else {
        return Vec::new();
    };
    let mut files: Vec<ReportFile> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "html"))
        .map(|entry| ReportFile {
            name: entry.file_name().to_string_lossy().into_owned(),
            modified: entry.metadata().and_then(|m| m.modified()).ok(),
            path: entry.path(),
        })
        .collect();
    files.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sample;

    #[test]
    fn the_snapshot_reads_the_current_prices_the_journal_and_the_upcoming_matches() {
        let sample = sample();

        let data = load(&sample.paths, sample.now).unwrap();

        // Betclic à 2,30 contre Pinnacle à 2,00 : un prix notifié, avec sa mise suggérée.
        assert_eq!(data.notified.len(), 1);
        assert!(data.notified[0].stake.is_some());
        assert!(data.below.is_empty());
        assert_eq!(data.scan.events, 1);
        assert_eq!(data.recommendations.len(), 1);
        assert_eq!(data.upcoming.len(), 1);
        assert!(data.bets.is_empty());
        assert_eq!(data.curve.available, 1000.0);
        assert!(data.clv.groups.iter().any(|g| g.n == 1));
        // La séance Loto Foot, lue comme la page `/lotofoot` du site.
        assert_eq!(data.lotofoot.tickets.len(), 2);
        assert_eq!(data.lotofoot.grids[0].matches.len(), 7);
        assert!(data.lotofoot.tickets.iter().all(|t| !t.played));
    }

    #[test]
    fn a_missing_reports_folder_gives_an_empty_list_and_reports_are_listed_newest_first() {
        let sample = sample();
        assert!(load(&sample.paths, sample.now).unwrap().reports.is_empty());

        std::fs::create_dir_all(&sample.paths.reports_dir).unwrap();
        std::fs::write(sample.paths.reports_dir.join("a.html"), "<p>a</p>").unwrap();
        std::fs::write(sample.paths.reports_dir.join("notes.txt"), "ignoré").unwrap();

        let reports = load(&sample.paths, sample.now).unwrap().reports;

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "a.html");
    }
}
