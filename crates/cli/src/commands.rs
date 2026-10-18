//! Les sous-commandes de `edge`. Chacune lit la configuration, appelle `edge-core` et affiche le résultat.

use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use edge_core::config::{API_KEY_ENV, Config};
use edge_core::ingest::aliases::Aliases;
use edge_core::ingest::client::OddsApiClient;
use edge_core::ingest::football_data::fetch::{FileClient, fetch_all};
use edge_core::ingest::football_data::import::{import_files, load_files};
use edge_core::ingest::football_data::quality::{self, SeasonStatus};
use edge_core::ingest::football_data::{FIRST_SEASON, Season, wanted_files};
use edge_core::ingest::snapshot::{self, SnapshotReport};
use edge_core::init::initialize;
use edge_core::models::data::load_leagues;
use edge_core::models::tuning;
use edge_core::paths::Paths;
use edge_core::report::market_report;
use edge_core::report::render::render_market_report;
use edge_core::store::archive::last_known_usage;
use edge_core::store::{self, Connection, migrate};

/// `edge init` : crée la configuration par défaut et la base.
pub fn init(paths: &Paths) -> Result<()> {
    let report = initialize(paths)?;
    let state = if report.config_created {
        "créée"
    } else {
        "déjà présente"
    };
    println!("Configuration : {} ({state})", paths.config_file.display());
    println!(
        "Base de données : {} (version du schéma : {})",
        paths.db_file.display(),
        report.schema_version
    );

    // Charger la configuration la valide aussi (champ inconnu, permissions trop ouvertes…).
    let config = Config::load(&paths.config_file)?;
    // On n'affiche jamais la clé, seulement sa présence.
    if api_key_from(&config).is_ok() {
        println!("Clé API : trouvée");
    } else {
        println!(
            "Clé API : absente (nécessaire pour `edge quota` et `edge snapshot` : définis {API_KEY_ENV} ou odds_api_key)"
        );
    }
    Ok(())
}

/// `edge quota` : lit le quota réel par l'appel gratuit à `/sports`.
pub fn quota(paths: &Paths) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    // La clé est vérifiée avant d'ouvrir la base : un échec ne laisse aucune trace.
    let client = OddsApiClient::new(api_key_from(&config)?);
    let conn = open_database(paths)?;

    let quota = snapshot::read_quota(&conn, &mut Utc::now, &mut |path, params| {
        client.get(path, params)
    })?;

    let used = quota
        .used
        .context("l'API n'a pas renvoyé x-requests-used")?;
    match quota.remaining {
        Some(remaining) => println!(
            "Crédits utilisés : {used} sur {} (restants : {remaining})",
            used + remaining
        ),
        None => println!("Crédits utilisés : {used}"),
    }
    let cap = config.monthly_credit_cap;
    println!(
        "Plafond configuré : {cap} (marge avant le plafond : {})",
        cap.saturating_sub(used)
    );
    Ok(())
}

/// `edge snapshot` : archive les cotes de toutes les ligues configurées.
pub fn snapshot(paths: &Paths, dry_run: bool) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    if dry_run {
        return print_plan(paths, &config);
    }
    let client = OddsApiClient::new(api_key_from(&config)?);
    let mut conn = open_database(paths)?;

    let report = snapshot::run(&mut conn, &config, &mut Utc::now, &mut |path, params| {
        client.get(path, params)
    })?;

    print_report(&report, config.monthly_credit_cap);
    if !report.failures.is_empty() {
        // Code de sortie non nul : systemd marque l'unité en échec et `journalctl` montre pourquoi.
        bail!("{} ligue(s) non archivée(s)", report.failures.len());
    }
    Ok(())
}

/// `edge fd fetch` : télécharge les CSV football-data manquants (2016/17 à la saison en cours).
pub fn fd_fetch(paths: &Paths, force: bool) -> Result<()> {
    let current = Season::containing(Utc::now().date_naive());
    let wanted = wanted_files(FIRST_SEASON, current);
    let client = FileClient::new();
    println!(
        "{} fichiers voulus (2016/17 à {}). Une pause de 1,5 s sépare deux téléchargements.",
        wanted.len(),
        current.label()
    );
    let report = fetch_all(
        &paths.football_data_dir,
        &wanted,
        current,
        force,
        SystemTime::now(),
        &mut |url| client.get(url),
        &mut || std::thread::sleep(Duration::from_millis(1500)),
    )?;
    println!(
        "Téléchargés : {} ; déjà en cache : {} ; dossier : {}",
        report.downloaded,
        report.cached,
        paths.football_data_dir.display()
    );
    for url in &report.not_published {
        println!("Pas encore publié par le site : {url}");
    }
    Ok(())
}

/// `edge fd import` : importe en base les CSV du cache (idempotent).
pub fn fd_import(paths: &Paths) -> Result<()> {
    let current = Season::containing(Utc::now().date_naive());
    let wanted = wanted_files(FIRST_SEASON, current);
    let aliases = Aliases::bundled()?;
    let (files, missing) = load_files(&paths.football_data_dir, &wanted)?;
    if files.is_empty() {
        bail!(
            "aucun fichier dans {} : lance d'abord `edge fd fetch`",
            paths.football_data_dir.display()
        );
    }
    let mut conn = open_database(paths)?;

    let report = import_files(&mut conn, &aliases, &files)?;

    println!(
        "{} fichiers importés : {} matchs ({} nouveaux), {} lignes de cotes 1X2, {} lignes de cotes plus/moins.",
        report.files, report.games, report.new_games, report.odds_rows, report.totals_rows
    );
    println!(
        "Écartés : {} lignes incomplètes, {} doublons, {} triplets de cotes invalides.",
        report.incomplete_rows, report.duplicate_rows, report.dropped_odds
    );
    if !missing.is_empty() {
        println!(
            "{} fichier(s) absent(s) du cache (voir `edge fd fetch`) :",
            missing.len()
        );
        for path in missing.iter().take(5) {
            println!("  - {path}");
        }
    }
    Ok(())
}

/// `edge fd check` : contrôle qualité de l'historique importé.
pub fn fd_check(paths: &Paths) -> Result<()> {
    let conn = open_database(paths)?;
    let current = Season::containing(Utc::now().date_naive()).label();
    let report = quality::run(&conn, &current)?;

    let incomplete: Vec<_> = report
        .seasons
        .iter()
        .filter(|season| season.status == SeasonStatus::Incomplete)
        .collect();
    println!(
        "Saisons : {} ligue-saisons, {} incomplètes.",
        report.seasons.len(),
        incomplete.len()
    );
    for season in incomplete {
        println!(
            "  - {} {} : {} matchs sur {} attendus ({} équipes)",
            season.div, season.season, season.games, season.expected, season.teams
        );
    }
    println!(
        "Résultats publiés contredisant les buts : {}",
        report.result_mismatches
    );
    let (low, high) = quality::IMPLIED_SUM_RANGE;
    println!("Cotes dont la somme des probabilités implicites sort de [{low:.2} ; {high:.2}] :");
    let mut any = false;
    for check in report.odds.iter().filter(|check| check.out_of_range > 0) {
        any = true;
        println!(
            "  - {} ({}) : {} sur {}",
            check.bookmaker, check.moment, check.out_of_range, check.rows
        );
    }
    if !any {
        println!("  aucune");
    }
    for sample in report.odds_samples.iter().take(5) {
        println!(
            "    exemple : {} {} {}–{} {} {} somme {:.3}",
            sample.date,
            sample.div,
            sample.home,
            sample.away,
            sample.bookmaker,
            sample.moment,
            sample.implied_sum
        );
    }
    println!(
        "Matchs sans tirs : {} ; sans heure de coup d'envoi : {}",
        report.games_without_shots, report.games_without_kickoff
    );
    Ok(())
}

/// `edge market report` : rapport HTML sur la qualité du marché (marges, calibration, sources de cotes).
pub fn market_report(paths: &Paths, out: Option<&std::path::Path>) -> Result<()> {
    let conn = open_database(paths)?;
    let today = Utc::now().date_naive();
    let current = Season::containing(today).label();

    println!("Calcul en cours (bootstrap par blocs, quelques secondes)…");
    let report = market_report::build(&conn, &current, &today.format("%Y-%m-%d").to_string())?;
    let html = render_market_report(&report)?;

    let target = match out {
        Some(path) => path.to_path_buf(),
        None => paths.reports_dir.join("market.html"),
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    }
    std::fs::write(&target, html).with_context(|| format!("écriture de {}", target.display()))?;

    println!("Rapport écrit : {}", target.display());
    println!(
        "Méthode de retrait de marge au log loss le plus bas : {} (distinguable du bruit : {})",
        report.best_method,
        if report.best_method_is_distinguishable {
            "oui"
        } else {
            "non"
        }
    );
    Ok(())
}

/// `edge model tune` : régle les hyperparamètres sur la validation 2024/25 et fige la meilleure configuration.
pub fn model_tune(paths: &Paths) -> Result<()> {
    let mut conn = open_database(paths)?;
    let leagues = load_leagues(&conn)?;
    let grid = tuning::candidate_grid();
    let window = tuning::validation_window();
    println!(
        "{} configurations, walk-forward hebdomadaire sur {} → {} (reprenable : une configuration déjà calculée est relue).",
        grid.len(),
        window.first,
        window.last
    );

    let outcome = tuning::tune(
        &mut conn,
        &leagues,
        &grid,
        window,
        Utc::now(),
        &mut |index, total, result| {
            println!(
                "[{index}/{total}] {} : log loss {:.5} sur {} matchs",
                result.config.version(),
                result.log_loss,
                result.games
            );
        },
    )?;

    for (label, best) in [
        ("Poisson", &outcome.best_poisson),
        ("Dixon-Coles", &outcome.best_dixon_coles),
    ] {
        println!(
            "Meilleur {label} : ξ = {}, κ = {}, mélange tirs = {} → log loss {:.5} ({} matchs)",
            best.config.decay, best.config.ridge, best.config.blend, best.log_loss, best.games
        );
    }
    let sessions = edge_core::store::predictions::tuning_session_count(&conn)?;
    println!(
        "Sessions de réglage enregistrées : {sessions} (chaque réglage supplémentaire est une occasion de s'adapter au hasard)."
    );
    Ok(())
}

/// `edge model evaluate` : évalue les configurations figées sur le test (2025/26 puis 2026/27 en avant).
pub fn model_evaluate(paths: &Paths) -> Result<()> {
    let mut conn = open_database(paths)?;
    let leagues = load_leagues(&conn)?;
    let last = leagues
        .iter()
        .flat_map(|league| league.games.iter().map(|game| game.date))
        .max()
        .context("aucun match en base : lance `edge fd fetch` puis `edge fd import`")?;
    let window = tuning::test_window(last);
    println!("Test : {} → {}", window.first, window.last);
    for run in tuning::evaluate(&mut conn, &leagues, window, Utc::now())? {
        println!(
            "{} : {} matchs prédits (exécution {})",
            run.config.version(),
            run.games,
            run.run_id
        );
    }
    Ok(())
}

fn print_plan(paths: &Paths, config: &Config) -> Result<()> {
    let plan = snapshot::plan(config);
    println!("Aucun appel n'est effectué (--dry-run).");
    println!(
        "Relevé prévu : {} ligue(s) × {} crédit(s) = {} crédit(s)",
        plan.leagues.len(),
        plan.cost_per_call,
        plan.total_cost
    );
    for league in &plan.leagues {
        println!("  - {league}");
    }
    let conn = open_database(paths)?;
    let cap = config.monthly_credit_cap;
    match last_known_usage(&conn)? {
        Some(used) => println!(
            "Dernier quota connu : {used} utilisés ; après ce relevé : {} (plafond {cap})",
            used + plan.total_cost
        ),
        None => println!("Quota encore inconnu : lance `edge quota` (plafond {cap})."),
    }
    Ok(())
}

fn print_report(report: &SnapshotReport, cap: u32) {
    for league in &report.leagues {
        println!(
            "{} : {} matchs, {} cotes archivés ({} déjà commencés ignorés)",
            league.sport_key, league.stored.events, league.stored.quotes, league.already_started
        );
        if !league.missing_bookmakers.is_empty() {
            println!("    sans cote : {}", league.missing_bookmakers.join(", "));
        }
    }
    for failure in &report.failures {
        println!("{} : ÉCHEC — {}", failure.sport_key, failure.reason);
    }
    if report.quota_reset {
        println!("Le quota semble avoir été réinitialisé depuis le dernier relevé.");
    }
    println!(
        "Crédits utilisés : {} → {} (plafond {cap})",
        report.used_before, report.used_after
    );
}

/// La clé vient de l'environnement, sinon du fichier de configuration.
fn api_key_from(config: &Config) -> Result<edge_core::config::ApiKey> {
    // `.ok()` transforme `Result<String, VarError>` en `Option<String>` : variable absente = `None`.
    let from_env = std::env::var(API_KEY_ENV).ok();
    // `as_deref` : de `Option<String>` à `Option<&str>`, sans copier ni consommer la valeur.
    Ok(config.api_key(from_env.as_deref())?)
}

/// Ouvre la base et la met à jour : un relevé planifié ne doit pas échouer sur un schéma en retard.
fn open_database(paths: &Paths) -> Result<Connection> {
    let mut conn =
        store::open(&paths.db_file).context("base inaccessible : lance d'abord `edge init`")?;
    migrate::migrate(&mut conn)?;
    Ok(conn)
}
