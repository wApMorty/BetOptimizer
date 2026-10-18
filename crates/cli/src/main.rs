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
    /// Prix des derniers relevés qui dépassent la cote juste de Pinnacle (lecture seule, ne parie jamais).
    Value {
        /// Avantage minimal, en points de pourcentage (3 = 3 %).
        #[arg(long, value_name = "POURCENT", default_value_t = 3.0)]
        min_edge: f64,
        /// Bankroll courante, en euros.
        #[arg(long, value_name = "EUROS", default_value_t = 1000.0)]
        bankroll: f64,
        /// Fraction de Kelly (1 = Kelly complet, 0,5 = demi-Kelly).
        #[arg(long, value_name = "FRACTION", default_value_t = 1.0)]
        kelly: f64,
        /// Mise minimale, en euros : un pari plus petit n'est pas suggéré.
        #[arg(long, value_name = "EUROS", default_value_t = 1.0)]
        min_stake: f64,
    },
    /// Backtest des règles de sélection de paris (seuils gelés, Kelly, CLV).
    Backtest {
        #[command(subcommand)]
        command: BacktestCommand,
    },
    /// Modèles de prévision des scores (Poisson, Dixon-Coles).
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// Analyses du marché des paris (retrait de marge, calibration).
    Market {
        #[command(subcommand)]
        command: MarketCommand,
    },
    /// Données historiques football-data.co.uk (résultats, statistiques, cotes).
    Fd {
        #[command(subcommand)]
        command: FdCommand,
    },
}

#[derive(Subcommand)]
enum BacktestCommand {
    /// Règle le seuil d'avantage de chaque source sur l'entraînement et la validation, puis le gèle.
    Tune,
    /// Évalue les seuils gelés sur le test et journalise les exécutions.
    Run,
    /// Génère le rapport HTML : où est la valeur, et est-elle réelle ?
    Report {
        /// Fichier de sortie (par défaut : dossier des rapports de l'application).
        #[arg(long, value_name = "FICHIER")]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ModelCommand {
    /// Régle les hyperparamètres sur la validation 2024/25 et fige la meilleure configuration de chaque modèle.
    Tune,
    /// Évalue les configurations figées sur le test (2025/26 puis 2026/27 en avant).
    Evaluate,
    /// Génère le rapport HTML : modèles contre marché sur le test, calibration, forces d'équipe.
    Report {
        /// Fichier de sortie (par défaut : dossier des rapports de l'application).
        #[arg(long, value_name = "FICHIER")]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum MarketCommand {
    /// Génère le rapport HTML sur la qualité du marché.
    Report {
        /// Fichier de sortie (par défaut : dossier des rapports de l'application).
        #[arg(long, value_name = "FICHIER")]
        out: Option<PathBuf>,
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
        Command::Value {
            min_edge,
            bankroll,
            kelly,
            min_stake,
        } => commands::value(
            &paths,
            &commands::ValueOptions {
                min_edge_pct: min_edge,
                bankroll,
                kelly_fraction: kelly,
                min_stake,
            },
        ),
        Command::Backtest { command } => match command {
            BacktestCommand::Tune => commands::backtest_tune(&paths),
            BacktestCommand::Run => commands::backtest_run(&paths),
            BacktestCommand::Report { out } => commands::backtest_report(&paths, out.as_deref()),
        },
        Command::Model { command } => match command {
            ModelCommand::Tune => commands::model_tune(&paths),
            ModelCommand::Evaluate => commands::model_evaluate(&paths),
            ModelCommand::Report { out } => commands::model_report(&paths, out.as_deref()),
        },
        Command::Market { command } => match command {
            MarketCommand::Report { out } => commands::market_report(&paths, out.as_deref()),
        },
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
