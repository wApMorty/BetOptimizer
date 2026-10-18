//! Point d'entrée du binaire `edge`.
//!
//! Ce fichier ne fait que lire la ligne de commande, appeler `edge-core` et afficher le résultat :
//! toute la logique vit dans la bibliothèque.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use edge_core::config::{API_KEY_ENV, Config};
use edge_core::init::initialize;
use edge_core::paths::Paths;
use tracing_subscriber::EnvFilter;

/// Moteur de recommandations de paris sportifs. Il ne parie jamais lui-même.
#[derive(Parser)]
#[command(name = "edge", version)]
struct Cli {
    /// Dossier unique pour la configuration et la base (remplace les emplacements standard).
    #[arg(long, global = true, value_name = "DOSSIER")]
    home: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Crée la configuration par défaut et la base de données, puis applique les migrations.
    Init,
}

/// `anyhow::Result` accepte n'importe quelle erreur : le `?` convertit celles de `edge-core`
/// et `main` les affiche, avec leur chaîne de causes, avant de quitter en code d'erreur.
fn main() -> Result<()> {
    init_logging();
    let cli = Cli::parse();
    let paths = match &cli.home {
        Some(dir) => Paths::under(dir),
        None => Paths::discover()?,
    };
    // `match` exhaustif : l'ajout d'une sous-commande sans la traiter ici ne compilerait plus.
    match cli.command {
        Command::Init => run_init(&paths),
    }
}

/// Les journaux vont sur la sortie d'erreur (journald les recueille sous systemd).
/// Niveau `info` par défaut, réglable par la variable `RUST_LOG`.
fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn run_init(paths: &Paths) -> Result<()> {
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
    // `.ok()` transforme `Result<String, VarError>` en `Option<String>` : variable absente = `None`.
    let from_env = std::env::var(API_KEY_ENV).ok();
    // On n'affiche jamais la clé, seulement sa présence.
    if config.api_key(from_env.as_deref()).is_ok() {
        println!("Clé API : trouvée");
    } else {
        println!(
            "Clé API : absente (nécessaire dès la phase 1 : définis {API_KEY_ENV} ou odds_api_key)"
        );
    }
    Ok(())
}
