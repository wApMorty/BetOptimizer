//! Les sous-commandes de `edge`. Chacune lit la configuration, appelle `edge-core` et affiche le résultat.

use anyhow::{Context, Result, bail};
use chrono::Utc;
use edge_core::config::{API_KEY_ENV, Config};
use edge_core::ingest::client::OddsApiClient;
use edge_core::ingest::snapshot::{self, SnapshotReport};
use edge_core::init::initialize;
use edge_core::paths::Paths;
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
