//! `edge latency …` : test prospectif de la latence des opérateurs français contre Pinnacle (phase 13).
//!
//! Le téléchargement vit dans `edge_core::ingest::oddspapi::fetch`, la règle figée dans `edge_core::eval::latency` ; ce
//! fichier lit la configuration, branche le vrai client et affiche.

use anyhow::{Context, Result, bail};
use chrono::{Local, Utc};
use edge_core::config::{Config, ODDSPAPI_KEY_ENV};
use edge_core::eval::latency::{self, Assessment, LatencyReport, MIN_BETS};
use edge_core::format;
use edge_core::ingest::aliases::Aliases;
use edge_core::ingest::oddspapi::OddsPapiClient;
use edge_core::ingest::oddspapi::fetch::{self, FetchReport};
use edge_core::paths::Paths;
use edge_core::report::latency_page::verdict_text;
use edge_core::store;

/// Paris rappelés en fin de rapport, du plus récent au plus ancien.
const SHOWN_BETS: usize = 10;

/// `edge latency fetch` : archive l'historique OddsPapi des matchs terminés depuis le début du test.
pub fn fetch(paths: &Paths, dry_run: bool) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    if dry_run {
        println!(
            "Passage prévu : {} requêtes décomptées (une liste de matchs par championnat), plafond configuré {} ;",
            fetch::cost(),
            config.oddspapi_monthly_cap
        );
        println!(
            "l'historique de chaque nouveau match est gratuit (environ 5 s par match). Aucun appel n'a été fait."
        );
        return Ok(());
    }
    // `.ok()` : variable absente = `None` ; `as_deref` passe de `Option<String>` à `Option<&str>` sans copier.
    let from_env = std::env::var(ODDSPAPI_KEY_ENV).ok();
    let client = OddsPapiClient::new(config.oddspapi_key(from_env.as_deref())?);
    let mut conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;

    let report = fetch::run(
        &mut conn,
        config.oddspapi_monthly_cap,
        &mut Utc::now,
        &mut |path, params| client.get(path, params),
        &mut std::thread::sleep,
    )?;
    print_fetch(&report);
    if !report.failures.is_empty() {
        // Code de sortie non nul : systemd marque l'unité en échec ; ces matchs seront redemandés au passage suivant.
        bail!("{} match(s) non archivé(s)", report.failures.len());
    }
    Ok(())
}

fn print_fetch(report: &FetchReport) {
    println!(
        "OddsPapi : {} requêtes décomptées sur {} avant ce passage (+{}).",
        report.used_before,
        report.limit,
        fetch::cost()
    );
    println!(
        "Matchs terminés depuis le début du test : {} ; déjà archivés : {} ; historiques archivés : {} ({} changements de prix) ; sans historique : {}.",
        report.listed, report.already_known, report.downloaded, report.prices, report.missing
    );
    if report.discarded > 0 {
        println!(
            "Entrées écartées (actives sans cote valide) : {}.",
            report.discarded
        );
    }
    for failure in &report.failures {
        println!("  échec : {failure}");
    }
}

/// `edge latency report` : applique la règle figée et affiche le verdict (lecture seule).
pub fn report(paths: &Paths) -> Result<()> {
    let conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    // Date locale du portable (Europe/Paris), comme `edge bonus` : la CLI n'a pas besoin de `chrono-tz`.
    let today = Local::now().date_naive();
    let report = latency::evaluate(&conn, &Aliases::bundled()?, today)?;
    print_report(&report);
    Ok(())
}

fn print_report(report: &LatencyReport) {
    println!(
        "Test prospectif de la latence française (docs/sources.md §34) — depuis le {}, échéance le {}.\n",
        latency::test_start().format("%d/%m/%Y"),
        latency::deadline().format("%d/%m/%Y")
    );
    print_assessment("Règle principale", &report.primary);
    print_assessment("Lecture secondaire (sans verdict)", &report.secondary);
    let (verdict, _) = verdict_text(&report.primary);
    println!("\nVerdict : {verdict}");

    let bets = &report.primary.bets;
    if !bets.is_empty() {
        println!("\nDerniers paris de la règle :");
        for b in bets.iter().rev().take(SHOWN_BETS) {
            println!(
                "  {}  {} – {}  ({})  {} {} à {} min : cote {}, avantage {} %, CLV {} %",
                format::kickoff_label(b.kickoff),
                b.home,
                b.away,
                latency::league_label(b.tournament_id),
                b.bookmaker,
                format::outcome_label(b.outcome),
                b.horizon_minutes,
                format::decimal(b.odds.get(), 2),
                format::signed(b.edge * 100.0, 1),
                format::signed(b.clv * 100.0, 1),
            );
        }
    }

    let q = &report.quality;
    println!(
        "\nContrôle de qualité : {} cote(s) identique(s) sur {} comparée(s) à notre archive The Odds API.",
        q.identical, q.compared
    );
    if !q.unknown_names.is_empty() {
        println!(
            "Noms absents de config/aliases.toml (exclus du seul contrôle) : {}",
            q.unknown_names
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

fn print_assessment(label: &str, a: &Assessment) {
    let clv = a.clv.map_or_else(
        || "—".to_string(),
        |c| {
            format!(
                "{} % [{} ; {}]",
                format::signed(c.mean * 100.0, 2),
                format::signed(c.low * 100.0, 2),
                format::signed(c.high * 100.0, 2)
            )
        },
    );
    println!(
        "{label} (suivi ≥ {} changements en 24 h) : {} match(s), {} couple(s) suivis, {} pari(s) sur {MIN_BETS}, CLV {clv}",
        a.min_changes,
        a.matches,
        a.tracked,
        a.bets.len()
    );
}
