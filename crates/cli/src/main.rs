//! Point d'entrée du binaire `edge`.
//!
//! Ce fichier ne fait que lire la ligne de commande et aiguiller vers [`commands`] ; toute la logique
//! vit dans la bibliothèque `edge-core`.

mod commands;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
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
    /// Affiche les crédits consommés (appel gratuit à l'API).
    Quota,
    /// Archive les cotes actuelles de toutes les ligues configurées (consomme des crédits).
    Snapshot {
        /// Affiche le coût prévu sans appeler l'API.
        #[arg(long)]
        dry_run: bool,
    },
    /// Données historiques football-data.co.uk (résultats, statistiques, cotes).
    Fd {
        #[command(subcommand)]
        command: FdCommand,
    },
}

#[derive(Subcommand)]
enum FdCommand {
    /// Télécharge les fichiers CSV manquants dans le cache (pause entre deux requêtes).
    Fetch {
        /// Retélécharge aussi les fichiers déjà en cache.
        #[arg(long)]
        force: bool,
    },
    /// Importe en base les fichiers du cache. Sûr à relancer : rien n'est dupliqué.
    Import,
    /// Contrôle qualité des données importées (signale, ne corrige jamais).
    Check,
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
        Command::Init => commands::init(&paths),
        Command::Quota => commands::quota(&paths),
        Command::Snapshot { dry_run } => commands::snapshot(&paths, dry_run),
        Command::Fd { command } => match command {
            FdCommand::Fetch { force } => commands::fd_fetch(&paths, force),
            FdCommand::Import => commands::fd_import(&paths),
            FdCommand::Check => commands::fd_check(&paths),
        },
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
