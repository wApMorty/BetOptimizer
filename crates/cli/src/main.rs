//! Point d'entrée du binaire `edge`.
//!
//! Ce fichier ne fait que lire la ligne de commande et aiguiller vers [`commands`] ; toute la logique
//! vit dans la bibliothèque `edge-core`.

mod bonus;
mod boost;
mod commands;
mod serve;

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
        /// Avantage minimal, en points de pourcentage (1 = 1 %, le seuil des recommandations).
        #[arg(long, value_name = "POURCENT", default_value_t = 1.0)]
        min_edge: f64,
        /// Bankroll, en euros (par défaut : bankroll courante, calculée depuis tes paris enregistrés).
        #[arg(long, value_name = "EUROS")]
        bankroll: Option<f64>,
        /// Fraction de Kelly (par défaut : celle de la configuration).
        #[arg(long, value_name = "FRACTION")]
        kelly: Option<f64>,
        /// Mise minimale, en euros (par défaut : celle de la configuration).
        #[arg(long, value_name = "EUROS")]
        min_stake: Option<f64>,
    },
    /// Relève la clôture de Pinnacle des matchs suivis qui commencent dans les 30 minutes (1 crédit par match).
    Closing {
        /// Liste les matchs concernés sans appeler l'API.
        #[arg(long)]
        dry_run: bool,
    },
    /// Envoie un message de test sur Discord (avec une image d'exemple) pour vérifier le webhook.
    NotifyTest,
    /// Écrit les recommandations du dernier relevé dans le journal immuable, archive le rapport du jour pour le site
    /// (et notifie sur Discord avec --notify).
    Recommend {
        /// Envoie le bilan sur Discord, image des recommandations jointe ; lien vers le site si `site_url` est configuré,
        /// sinon rapport HTML joint.
        #[arg(long)]
        notify: bool,
    },
    /// Publie le site de rapports (rapport du jour, rapports précédents, opportunités, paper trading), en lecture seule.
    Serve,
    /// NBA : archive des cotes et des résultats.
    Nba {
        #[command(subcommand)]
        command: NbaCommand,
    },
    /// Tennis : données de tennis-data, Elo par surface, comparaison au marché.
    Tennis {
        #[command(subcommand)]
        command: TennisCommand,
    },
    /// Mes paris réels : enregistrement, liste et règlement.
    Bet {
        #[command(subcommand)]
        command: BetCommand,
    },
    /// CLV réel des recommandations et des paris (cote obtenue contre clôture de Pinnacle).
    Clv,
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
    /// Bonus de bienvenue couverts : suivi des offres, opérations proposées, calcul des couvertures (docs/bonus-bienvenue.md).
    Bonus {
        #[command(subcommand)]
        command: BonusCommand,
    },
    /// Cotes boostées : étude historique sur les données de WePari.
    Boost {
        #[command(subcommand)]
        command: BoostCommand,
    },
    /// Données historiques football-data.co.uk (résultats, statistiques, cotes).
    Fd {
        #[command(subcommand)]
        command: FdCommand,
    },
}

#[derive(Subcommand)]
enum NbaCommand {
    /// Archive les cotes du soir (vainqueur, handicaps, totaux) et, tous les deux jours, les résultats.
    Snapshot {
        /// Affiche le coût prévu sans appeler l'API.
        #[arg(long)]
        dry_run: bool,
    },
    /// Règle le modèle sur la validation (15/11/2026 → 01/02/2027) et le gèle. Refusé avant le 01/02/2027.
    Tune,
    /// Génère le rapport HTML : le modèle contre Pinnacle (exploratoire tant que rien n'est gelé).
    Report {
        /// Fichier de sortie (par défaut : dossier des rapports de l'application).
        #[arg(long, value_name = "FICHIER")]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum TennisCommand {
    /// Importe les tableurs de tennis-data téléchargés à la main dans `tennis-data/atp` et `tennis-data/wta`.
    Import,
    /// Contrôle qualité des données de tennis (signale, ne corrige jamais).
    Check,
    /// Règle l'Elo sur la validation (2022-2023) et gèle la meilleure configuration. Le test n'est pas lu.
    Tune,
    /// Génère le rapport HTML : l'Elo contre le marché sur le test (à partir du 2024-01-01).
    Report {
        /// Fichier de sortie (par défaut : dossier des rapports de l'application).
        #[arg(long, value_name = "FICHIER")]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum BetCommand {
    /// Enregistre un pari placé : `--rec <n°>` suit une recommandation, `--event <id>` est un pari hors modèle.
    #[command(group(clap::ArgGroup::new("target").required(true).args(["rec", "event"])))]
    Log {
        /// Numéro de la recommandation suivie (cote et mise par défaut : les siennes).
        #[arg(long, value_name = "N", conflicts_with_all = ["event", "outcome"])]
        rec: Option<i64>,
        /// Identifiant du match, pour un pari hors modèle (affiché par `edge value`).
        #[arg(long, value_name = "ID", requires_all = ["bookmaker", "outcome", "odds", "stake"])]
        event: Option<String>,
        /// Bookmaker où le pari est placé (`betclic_fr`…).
        #[arg(long)]
        bookmaker: Option<String>,
        /// Issue jouée : H (domicile), D (nul) ou A (extérieur).
        #[arg(long, value_name = "H|D|A")]
        outcome: Option<String>,
        /// Cote réellement obtenue.
        #[arg(long)]
        odds: Option<f64>,
        /// Mise en euros.
        #[arg(long)]
        stake: Option<f64>,
    },
    /// Liste les paris enregistrés et la bankroll courante.
    List,
    /// Règle les paris ouverts depuis l'historique football-data, ou un pari à la main avec --id et --result.
    Settle {
        /// Numéro du pari à régler à la main.
        #[arg(long, value_name = "N", requires = "result")]
        id: Option<i64>,
        /// Résultat saisi à la main : won, lost ou void (match reporté ou annulé : remboursé).
        #[arg(long, value_name = "won|lost|void", requires = "id")]
        result: Option<String>,
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
    /// Abaque des seuils (0 à 8 %, pas de 0,1 point) sur l'entraînement et la validation : rapport HTML, sans rien geler.
    Curve {
        /// Fichier de sortie (par défaut : dossier des rapports de l'application).
        #[arg(long, value_name = "FICHIER")]
        out: Option<PathBuf>,
    },
    /// Gèle le seuil de plus forte croissance attendue de chaque bookmaker : nouvelle session de réglage, comptée.
    Freeze,
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
enum BonusCommand {
    /// État de chaque offre, comptes de couverture, opérations proposées sur le dernier relevé, bilan.
    Status,
    /// Les cinq meilleures opérations du dernier relevé pour un opérateur.
    Pick {
        /// Opérateur du bonus (`betclic_fr`…).
        #[arg(long)]
        bookmaker: String,
    },
    /// Enregistre ce que tu sais de l'offre d'un opérateur, après lecture de son règlement (la plus récente fait foi).
    Offer {
        /// Clé de l'opérateur (`betclic_fr`, `bet365_fr`…).
        #[arg(long)]
        bookmaker: String,
        /// Forme : cash_refund (remboursé en argent), freebet_refund (remboursé en freebet) ou freebet.
        #[arg(long, value_name = "FORME")]
        kind: Option<String>,
        /// Montant maximal de l'offre, en euros.
        #[arg(long, value_name = "EUROS")]
        amount: Option<f64>,
        /// Cote minimale imposée par l'offre.
        #[arg(long, value_name = "COTE")]
        min_odds: Option<f64>,
        /// Offre inaccessible (dépôt passé…) : le compte ne sert qu'à couvrir.
        #[arg(long)]
        ineligible: bool,
        /// Où l'éligibilité a été vérifiée (« règlement du 18/04/2025, clause C », « service client »…).
        #[arg(long)]
        source: String,
        /// Date de la vérification, AAAA-MM-JJ (par défaut : aujourd'hui).
        #[arg(long, value_name = "DATE")]
        checked_on: Option<chrono::NaiveDate>,
        /// Remarque libre.
        #[arg(long)]
        note: Option<String>,
    },
    /// Calcule les mises de couverture sur les cotes affichées au moment de parier (ne lit ni n'écrit la base).
    Cover {
        /// Forme : cash_refund, freebet_refund ou freebet.
        #[arg(long, value_name = "FORME")]
        kind: String,
        /// Montant du bonus, en euros.
        #[arg(long, value_name = "EUROS")]
        amount: f64,
        /// Cote du pari bonus.
        #[arg(long, value_name = "COTE")]
        odds: f64,
        /// Cote de couverture de chaque autre issue (deux fois en 1X2).
        #[arg(long = "cover", value_name = "COTE", required = true)]
        covers: Vec<f64>,
        /// Valeur d'un euro de freebet (par défaut : bonus_freebet_value de la configuration).
        #[arg(long, value_name = "VALEUR")]
        freebet_value: Option<f64>,
    },
    /// Enregistre une opération jouée : le pari bonus et ses couvertures, aux cotes et mises réellement obtenues.
    Log {
        /// Opérateur du bonus.
        #[arg(long)]
        bookmaker: String,
        /// Forme (par défaut : celle de l'offre, ou le freebet reçu à jouer).
        #[arg(long, value_name = "FORME")]
        kind: Option<String>,
        /// Identifiant du match (affiché par `edge bonus status`).
        #[arg(long, value_name = "ID")]
        event: String,
        /// Pari bonus : ISSUE:COTE:MISE, par exemple A:3.10:100.
        #[arg(long, value_name = "ISSUE:COTE:MISE")]
        bonus: String,
        /// Couverture : OPÉRATEUR:ISSUE:COTE:MISE, par exemple winamax_fr:H:2.10:122 (une par autre issue).
        #[arg(long = "cover", value_name = "OPÉRATEUR:ISSUE:COTE:MISE")]
        covers: Vec<String>,
    },
    /// Règle les opérations ouvertes depuis l'historique football-data, ou une opération à la main avec --id et --result.
    Settle {
        /// Numéro de l'opération à régler à la main.
        #[arg(long, value_name = "N", requires = "result")]
        id: Option<i64>,
        /// Résultat du match : H, D, A, ou void (reporté ou annulé).
        #[arg(long, value_name = "H|D|A|void", requires = "id")]
        result: Option<String>,
    },
}

#[derive(Subcommand)]
enum BoostCommand {
    /// Rentabilité historique des cotes boostées de WePari (pages téléchargées à la main dans le dossier `wepari`),
    /// tarifées contre Pinnacle sans marge.
    Study,
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
        Command::Nba { command } => match command {
            NbaCommand::Snapshot { dry_run } => commands::nba_snapshot(&paths, dry_run),
            NbaCommand::Tune => commands::nba_tune(&paths),
            NbaCommand::Report { out } => commands::nba_report(&paths, out.as_deref()),
        },
        Command::Tennis { command } => match command {
            TennisCommand::Import => commands::tennis_import(&paths),
            TennisCommand::Check => commands::tennis_check(&paths),
            TennisCommand::Tune => commands::tennis_tune(&paths),
            TennisCommand::Report { out } => commands::tennis_report(&paths, out.as_deref()),
        },
        Command::Bet { command } => match command {
            BetCommand::Log {
                rec,
                event,
                bookmaker,
                outcome,
                odds,
                stake,
            } => commands::bet_log(
                &paths,
                &commands::BetLogOptions {
                    rec,
                    event,
                    bookmaker,
                    outcome,
                    odds,
                    stake,
                },
            ),
            BetCommand::List => commands::bet_list(&paths),
            BetCommand::Settle { id, result } => {
                commands::bet_settle(&paths, id.zip(result.as_deref()))
            }
        },
        Command::Clv => commands::clv_report(&paths),
        Command::Closing { dry_run } => commands::closing(&paths, dry_run),
        Command::NotifyTest => commands::notify_test(&paths),
        Command::Recommend { notify } => commands::recommend(&paths, notify),
        Command::Serve => commands::serve(&paths),
        Command::Backtest { command } => match command {
            BacktestCommand::Tune => commands::backtest_tune(&paths),
            BacktestCommand::Run => commands::backtest_run(&paths),
            BacktestCommand::Report { out } => commands::backtest_report(&paths, out.as_deref()),
            BacktestCommand::Curve { out } => commands::backtest_curve(&paths, out.as_deref()),
            BacktestCommand::Freeze => commands::backtest_freeze(&paths),
        },
        Command::Model { command } => match command {
            ModelCommand::Tune => commands::model_tune(&paths),
            ModelCommand::Evaluate => commands::model_evaluate(&paths),
            ModelCommand::Report { out } => commands::model_report(&paths, out.as_deref()),
        },
        Command::Market { command } => match command {
            MarketCommand::Report { out } => commands::market_report(&paths, out.as_deref()),
        },
        Command::Bonus { command } => match command {
            BonusCommand::Status => bonus::status(&paths),
            BonusCommand::Pick { bookmaker } => bonus::pick(&paths, &bookmaker),
            BonusCommand::Offer {
                bookmaker,
                kind,
                amount,
                min_odds,
                ineligible,
                source,
                checked_on,
                note,
            } => bonus::offer(
                &paths,
                &bonus::OfferOptions {
                    bookmaker,
                    kind,
                    amount,
                    min_odds,
                    ineligible,
                    source,
                    checked_on,
                    note,
                },
            ),
            BonusCommand::Cover {
                kind,
                amount,
                odds,
                covers,
                freebet_value,
            } => bonus::cover_calc(
                &paths,
                &bonus::CoverOptions {
                    kind,
                    amount,
                    odds,
                    covers,
                    freebet_value,
                },
            ),
            BonusCommand::Log {
                bookmaker,
                kind,
                event,
                bonus: bonus_leg,
                covers,
            } => bonus::log(
                &paths,
                &bonus::LogOptions {
                    bookmaker,
                    kind,
                    event,
                    bonus: bonus_leg,
                    covers,
                },
            ),
            BonusCommand::Settle { id, result } => bonus::settle(&paths, id.zip(result.as_deref())),
        },
        Command::Boost { command } => match command {
            BoostCommand::Study => boost::study(&paths),
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
