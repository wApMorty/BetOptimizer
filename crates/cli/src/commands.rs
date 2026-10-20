//! Les sous-commandes de `edge`. Chacune lit la configuration, appelle `edge-core` et affiche le résultat.

use std::io::IsTerminal;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use edge_core::backtest::candidates::load_inputs;
use edge_core::backtest::{curve, study};
use edge_core::betting::kelly::{self, to_units};
use edge_core::betting::{clv, ledger, live, recommend, settle};
use edge_core::config::{API_KEY_ENV, Config, WEBHOOK_ENV};
use edge_core::domain::{Euros, Outcome};
use edge_core::format;
use edge_core::ingest::aliases::Aliases;
use edge_core::ingest::client::OddsApiClient;
use edge_core::ingest::football_data::fetch::{FileClient, fetch_all};
use edge_core::ingest::football_data::import::{import_files, load_files};
use edge_core::ingest::football_data::quality::{self, SeasonStatus};
use edge_core::ingest::football_data::{FIRST_SEASON, Season, wanted_files};
use edge_core::ingest::snapshot::{self, SnapshotReport};
use edge_core::ingest::tennis::{import as tennis_import_files, quality as tennis_quality};
use edge_core::ingest::{closing, nba};
use edge_core::init::initialize;
use edge_core::models::data::load_leagues;
use edge_core::models::tuning;
use edge_core::models::{elo_tuning, nba_tuning};
use edge_core::notify::{self, discord};
use edge_core::paths::Paths;
use edge_core::report::backtest_report;
use edge_core::report::market_report;
use edge_core::report::models_report;
use edge_core::report::nba_report;
use edge_core::report::recommend_report;
use edge_core::report::render::{
    render_backtest_report, render_market_report, render_models_report, render_nba_report,
    render_recommend_report, render_tennis_report, render_thresholds_report,
};
use edge_core::report::tennis_report;
use edge_core::report::thresholds_report;
use edge_core::store::archive::last_known_usage;
use edge_core::store::journal::{self, Settlement, SettlementSource};
use edge_core::store::nba as nba_store;
use edge_core::store::tennis as tennis_store;
use edge_core::store::{self, Connection};

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

/// `edge model report` : rapport HTML des modèles contre le marché sur le test.
pub fn model_report(paths: &Paths, out: Option<&std::path::Path>) -> Result<()> {
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let leagues = load_leagues(&conn)?;
    let last = leagues
        .iter()
        .flat_map(|league| league.games.iter().map(|game| game.date))
        .max()
        .context("aucun match en base : lance `edge fd fetch` puis `edge fd import`")?;
    let today = Utc::now().date_naive();

    println!("Calcul en cours (bootstrap par blocs, quelques secondes)…");
    let report = models_report::build(
        &conn,
        &leagues,
        config.margin_method,
        last,
        &today.format("%Y-%m-%d").to_string(),
    )?;
    let html = render_models_report(&report)?;

    let target = match out {
        Some(path) => path.to_path_buf(),
        None => paths.reports_dir.join("models.html"),
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    }
    std::fs::write(&target, html).with_context(|| format!("écriture de {}", target.display()))?;
    println!("Rapport écrit : {}", target.display());
    Ok(())
}

/// `edge backtest tune` : règle le seuil d'avantage de chaque source sur l'entraînement et la validation, puis le gèle.
pub fn backtest_tune(paths: &Paths) -> Result<()> {
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let inputs = load_inputs(&conn)?;
    let predictions = study::load_model_predictions(&conn)?;
    println!(
        "{} matchs chargés ; {} prédictions du modèle. Seuils essayés : {:?}. Critère : CLV moyen le plus élevé, {} paris au moins.",
        inputs.len(),
        predictions.len(),
        study::THRESHOLDS,
        study::MIN_SELECTION_BETS
    );

    let rows = study::tune(
        &conn,
        &inputs,
        &predictions,
        config.margin_method,
        Utc::now(),
    )?;

    for row in &rows {
        match row.chosen {
            Some(threshold) => println!("{} : seuil gelé à {:.0} %", row.source, threshold * 100.0),
            None => println!(
                "{} : aucun seuil éligible (moins de {} paris)",
                row.source,
                study::MIN_SELECTION_BETS
            ),
        }
        for result in &row.results {
            println!(
                "    seuil {:>4.0} % : {:>5} paris, CLV {}, rendement {}",
                result.threshold * 100.0,
                result.bets,
                percent_or_dash(result.clv),
                percent_or_dash(result.roi)
            );
        }
    }
    let sessions = edge_core::store::backtests::selection_session_count(&conn)?;
    println!("Sessions de réglage des seuils enregistrées : {sessions}.");
    Ok(())
}

/// Réglages de `edge value` ; une valeur absente prend celle de la configuration.
pub struct ValueOptions {
    /// Avantage minimal, en points de pourcentage (3 = 3 %).
    pub min_edge_pct: f64,
    /// Bankroll, en euros ; par défaut la bankroll courante (départ + gains − mises ouvertes de tes paris enregistrés).
    pub bankroll: Option<f64>,
    /// Fraction de Kelly (1 = Kelly complet).
    pub kelly_fraction: Option<f64>,
    /// Mise minimale, en euros.
    pub min_stake: Option<f64>,
}

/// Mises résolues : options de la ligne de commande, sinon configuration et journal.
struct Staking {
    bankroll: Euros,
    kelly_fraction: f64,
    min_stake: Euros,
}

fn staking(
    conn: &Connection,
    config: &Config,
    bankroll: Option<f64>,
    kelly_fraction: Option<f64>,
    min_stake: Option<f64>,
) -> Result<Staking> {
    let kelly_fraction = kelly_fraction.unwrap_or(config.kelly_fraction);
    if !(kelly_fraction > 0.0 && kelly_fraction <= 1.0) {
        bail!("--kelly doit être dans ]0 ; 1]");
    }
    let bankroll = match bankroll {
        Some(amount) => Euros::new(amount).filter(|b| b.get() > 0.0),
        None => {
            let initial =
                Euros::new(config.initial_bankroll).context("initial_bankroll invalide")?;
            Some(journal::current_bankroll(conn, initial)?)
        }
    };
    let min_stake = Euros::new(min_stake.unwrap_or(config.min_stake));
    let (Some(bankroll), Some(min_stake)) = (bankroll, min_stake) else {
        bail!("--bankroll doit être strictement positif et --min-stake positif ou nul");
    };
    Ok(Staking {
        bankroll,
        kelly_fraction,
        min_stake,
    })
}

/// Une sélection à afficher, qu'elle vienne d'un relevé (`edge value`) ou du journal (`edge recommend`).
struct Pick<'a> {
    kickoff: DateTime<Utc>,
    home: &'a str,
    away: &'a str,
    sport_key: &'a str,
    bookmaker: &'a str,
    outcome: Outcome,
    odds: f64,
    probability: f64,
    edge: f64,
    minimal_odds: f64,
    stake: Option<Euros>,
    event_id: &'a str,
}

fn print_pick(pick: &Pick<'_>) {
    println!(
        "\n{}  {} – {}  ({})",
        format::kickoff_label(pick.kickoff),
        pick.home,
        pick.away,
        pick.sport_key
    );
    let stake = pick.stake.map_or_else(
        || "sous la mise minimale".to_string(),
        |s| format!("mise {} €", format::decimal(s.get(), 1)),
    );
    println!(
        "    {} · {} à {} · cote juste {} · avantage {} % · cote minimale {} · {}",
        pick.bookmaker,
        format::outcome_label(pick.outcome),
        format::decimal(pick.odds, 2),
        format::decimal(1.0 / pick.probability, 2),
        format::signed(pick.edge * 100.0, 1),
        format::decimal(pick.minimal_odds, 2),
        stake
    );
    println!("    match {}", pick.event_id);
}

/// `edge value` : les prix des derniers relevés qui dépassent la cote juste de Pinnacle. Lecture seule.
pub fn value(paths: &Paths, options: &ValueOptions) -> Result<()> {
    // NaN échoue à la comparaison : `is_nan` l'attrape explicitement.
    if options.min_edge_pct.is_nan() || options.min_edge_pct < 0.0 {
        bail!("--min-edge doit être positif ou nul");
    }
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let staking = staking(
        &conn,
        &config,
        options.bankroll,
        options.kelly_fraction,
        options.min_stake,
    )?;
    let now = Utc::now();
    let min_edge = options.min_edge_pct / 100.0;
    let mut scan = live::scan(
        &conn,
        config.margin_method,
        min_edge,
        &config.excluded_bookmakers,
        now,
    )?;
    live::size_stakes(
        &mut scan.opportunities,
        staking.bankroll,
        staking.kelly_fraction,
        staking.min_stake,
    )?;

    if scan.events == 0 {
        println!(
            "Aucun match à venir dans un relevé de moins de {} h : lance `edge snapshot`.",
            live::MAX_SNAPSHOT_AGE_HOURS
        );
        return Ok(());
    }
    println!(
        "{} match(s) à venir ; {} sans cotes complètes de Pinnacle (ignorés) ; {} prix aberrant(s) écarté(s).",
        scan.events, scan.without_reference, scan.implausible_quotes
    );
    println!(
        "Seuil d'avantage {} % · Kelly ×{} · bankroll {} €",
        format::decimal(options.min_edge_pct, 1),
        format::decimal(staking.kelly_fraction, 2),
        format::decimal(staking.bankroll.get(), 0)
    );
    if scan.opportunities.is_empty() {
        println!("\nAucun prix n'atteint le seuil.");
    }
    for found in &scan.opportunities {
        print_pick(&Pick {
            kickoff: found.kickoff,
            home: &found.home_team,
            away: &found.away_team,
            sport_key: &found.sport_key,
            bookmaker: &found.bookmaker,
            outcome: found.selection.outcome,
            odds: found.selection.odds.get(),
            probability: found.selection.probability.get(),
            edge: found.selection.edge,
            minimal_odds: found.minimal_odds,
            stake: found.stake,
            event_id: &found.event_id,
        });
        println!("    prix mis à jour {}", format::age(found.priced_at, now));
    }
    println!(
        "\nÀ lire : la probabilité est celle de Pinnacle sans marge, pas celle d'un modèle. Aucune règle n'est validée hors\n\
         échantillon (voir `edge backtest report`) : vérifie chaque prix à la main avant tout pari ; si la cote affichée\n\
         par l'opérateur est sous la cote minimale, ne parie pas."
    );
    Ok(())
}

/// `edge closing` : relève la clôture de Pinnacle des matchs suivis qui commencent dans les 30 minutes.
///
/// Lancée toutes les dix minutes par un timer : sans match concerné, elle n'appelle pas l'API, ne demande même pas la
/// clé, et ne dit rien quand la sortie va dans un journal (elle parle seulement dans un terminal).
pub fn closing(paths: &Paths, dry_run: bool) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let mut conn = open_database(paths)?;
    let targets = closing::pending(&conn, Utc::now())?;
    let talkative = dry_run || std::io::stdout().is_terminal();

    if targets.is_empty() {
        if talkative {
            println!(
                "Aucun match suivi (recommandation ou pari) ne commence dans les {} prochaines minutes sans clôture.",
                closing::WINDOW_MINUTES
            );
        }
        return Ok(());
    }
    if dry_run {
        println!(
            "Aucun appel n'est effectué. Matchs dont la clôture serait relevée (1 crédit chacun) :"
        );
        for target in &targets {
            println!(
                "  {} · {}",
                format::kickoff_label(target.kickoff),
                target.event_id
            );
        }
        return Ok(());
    }

    let client = OddsApiClient::new(api_key_from(&config)?);
    let report = closing::run(&mut conn, &config, &mut Utc::now, &mut |path, params| {
        client.get(path, params)
    })?;
    for done in &report.captured {
        println!(
            "Clôture relevée : {} ({}){}",
            done.target.event_id,
            format::kickoff_label(done.target.kickoff),
            if done.has_pinnacle {
                ""
            } else {
                " — Pinnacle sans cote : inutilisable"
            }
        );
    }
    for failure in &report.failures {
        eprintln!(
            "Clôture manquée : {} — {}",
            failure.target.event_id, failure.reason
        );
    }
    if !report.failures.is_empty() {
        bail!("{} clôture(s) manquée(s)", report.failures.len());
    }
    Ok(())
}

/// Ce que `edge bet log` reçoit de la ligne de commande.
pub struct BetLogOptions {
    /// Recommandation suivie.
    pub rec: Option<i64>,
    /// Match d'un pari hors modèle.
    pub event: Option<String>,
    /// Bookmaker (hors modèle).
    pub bookmaker: Option<String>,
    /// Issue `H`, `D` ou `A` (hors modèle).
    pub outcome: Option<String>,
    /// Cote obtenue (par défaut : celle de la recommandation).
    pub odds: Option<f64>,
    /// Mise (par défaut : la mise recommandée).
    pub stake: Option<f64>,
}

/// `edge bet log` : enregistre un pari réellement placé, suivant une recommandation ou hors modèle.
pub fn bet_log(paths: &Paths, options: &BetLogOptions) -> Result<()> {
    let conn = open_database(paths)?;
    let new = match (options.rec, &options.event) {
        (Some(id), _) => ledger::followed_bet(
            &conn,
            id,
            &ledger::FollowedInput {
                odds: options.odds,
                stake: options.stake,
                bookmaker: options.bookmaker.clone(),
            },
        )?,
        (None, Some(event_id)) => {
            let code = options
                .outcome
                .as_deref()
                .context("--outcome est requis (H, D ou A)")?;
            let outcome = Outcome::from_code(code).context("--outcome doit valoir H, D ou A")?;
            ledger::off_model_bet(
                &conn,
                event_id,
                options
                    .bookmaker
                    .as_deref()
                    .context("--bookmaker est requis")?,
                outcome,
                options.odds.context("--odds est requis")?,
                options.stake.context("--stake est requis")?,
            )?
        }
        (None, None) => bail!("précise --rec <n°> ou --event <identifiant>"),
    };
    let off_model = new.recommendation_id.is_none();
    let id = ledger::record(&conn, &new, Utc::now())?;
    println!(
        "Pari n° {id} enregistré{} : {} – {}, {} {} à {}, mise {} €.",
        if off_model { " (hors modèle)" } else { "" },
        new.home_team,
        new.away_team,
        new.bookmaker,
        format::outcome_label(new.outcome),
        format::decimal(new.odds.get(), 2),
        format::decimal(new.stake.get(), 1)
    );
    Ok(())
}

/// `edge bet list` : les paris enregistrés avec leur état.
pub fn bet_list(paths: &Paths) -> Result<()> {
    let conn = open_database(paths)?;
    let bets = journal::bets(&conn)?;
    if bets.is_empty() {
        println!(
            "Aucun pari enregistré. `edge bet log --rec <n°>` enregistre un pari suivant une recommandation."
        );
        return Ok(());
    }
    for stored in &bets {
        let b = &stored.bet;
        let state = match &stored.result {
            None => "ouvert".to_string(),
            Some(r) => match r.settlement {
                Settlement::Won => format!("gagné {} €", format::signed(r.profit, 2)),
                Settlement::Lost => format!("perdu {} €", format::signed(r.profit, 2)),
                Settlement::Void => "remboursé".to_string(),
            },
        };
        println!(
            "n° {:<4} {}  {} – {}  {} {} à {} · mise {} €  [{}{}]",
            stored.id,
            format::kickoff_label(b.kickoff),
            b.home_team,
            b.away_team,
            b.bookmaker,
            format::outcome_label(b.outcome),
            format::decimal(b.odds.get(), 2),
            format::decimal(b.stake.get(), 1),
            state,
            b.recommendation_id
                .map_or(", hors modèle".to_string(), |id| format!(
                    ", recommandation n° {id}"
                ))
        );
    }
    let initial = Euros::new(Config::load(&paths.config_file)?.initial_bankroll)
        .context("initial_bankroll invalide")?;
    println!(
        "\nBankroll courante : {} €",
        format::decimal(journal::current_bankroll(&conn, initial)?.get(), 2)
    );
    Ok(())
}

/// `edge bet settle` : règle les paris ouverts depuis l'historique football-data, ou un pari à la main.
pub fn bet_settle(paths: &Paths, manual: Option<(i64, &str)>) -> Result<()> {
    let conn = open_database(paths)?;
    let now = Utc::now();
    if let Some((id, result)) = manual {
        let settlement = match result {
            "won" => Settlement::Won,
            "lost" => Settlement::Lost,
            "void" => Settlement::Void,
            other => bail!("--result doit valoir won, lost ou void (reçu : {other})"),
        };
        let stored = journal::bets(&conn)?
            .into_iter()
            .find(|b| b.id == id)
            .with_context(|| format!("pari n° {id} introuvable"))?;
        if !journal::insert_result(&conn, &stored, settlement, SettlementSource::Manual, now)? {
            bail!("le pari n° {id} est déjà réglé : un règlement ne se modifie pas");
        }
        println!("Pari n° {id} réglé à la main : {result}.");
        return Ok(());
    }
    let report = settle::settle_open_bets(&conn, &Aliases::bundled()?, now)?;
    println!(
        "{} pari(s) réglé(s) depuis l'historique ; {} encore ouvert(s).",
        report.settled, report.still_open
    );
    if report.still_open > 0 {
        println!(
            "Pour les paris ouverts : `edge fd fetch` puis `edge fd import` ramènent les résultats récents (football-data les publie avec quelques jours de retard) ; sinon `edge bet settle --id <n°> --result won|lost|void`."
        );
    }
    Ok(())
}

/// `edge clv` : CLV réel des recommandations et des paris, par groupe et par tranche d'avantage.
pub fn clv_report(paths: &Paths) -> Result<()> {
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let report = clv::report(&conn, &Aliases::bundled()?, config.margin_method)?;
    println!(
        "CLV = cote obtenue × probabilité de clôture de Pinnacle sans marge − 1 (intervalle à 95 %).\n"
    );
    for group in &report.groups {
        println!(
            "{} : {} (clôture connue : {}, réglés : {})",
            group.label, group.n, group.with_closing, group.settled
        );
        if group.n == 0 {
            continue;
        }
        println!(
            "    CLV {}   rendement à mise fixe {}",
            ci_text(group.clv),
            ci_text(group.flat_roi)
        );
        if let Some((staked, profit)) = group.money {
            println!(
                "    argent réel réglé : misé {} €, gain net {} €",
                format::decimal(staked, 2),
                format::signed(profit, 2)
            );
        }
        if group.with_closing < 30 {
            println!("    (moins de 30 clôtures : indicatif seulement)");
        }
    }
    println!("\nPar tranche d'avantage annoncé (notifiées et journal fantôme) :");
    for bucket in &report.buckets {
        println!(
            "  {:<10} {:>4} · clôture connue {:>4} · avantage moyen {} % · CLV moyen {}",
            bucket.label,
            bucket.n,
            bucket.with_closing,
            format::signed(bucket.mean_edge * 100.0, 1),
            bucket.mean_clv.map_or("—".to_string(), |v| format!(
                "{} %",
                format::signed(v * 100.0, 2)
            ))
        );
    }
    for (title, splits) in [
        ("Par opérateur", &report.by_bookmaker),
        ("Par issue", &report.by_outcome),
    ] {
        println!("\n{title} (notifiées et journal fantôme) :");
        for split in splits {
            println!(
                "  {:<11} {:>4} · clôture connue {:>4} · avantage moyen {} % · CLV {}",
                split.key,
                split.n,
                split.with_closing,
                format::signed(split.mean_edge * 100.0, 1),
                ci_text(split.clv)
            );
        }
    }
    Ok(())
}

fn ci_text(ci: Option<edge_core::eval::bootstrap::MeanCi>) -> String {
    ci.map_or("—".to_string(), |c| {
        format!(
            "{} % [{} ; {}]",
            format::signed(c.mean * 100.0, 2),
            format::signed(c.low * 100.0, 1),
            format::signed(c.high * 100.0, 1)
        )
    })
}

/// `edge tennis import` : lit les tableurs de tennis-data téléchargés à la main et les importe (idempotent).
pub fn tennis_import(paths: &Paths) -> Result<()> {
    let (files, ignored) = tennis_import_files::find_files(&paths.tennis_data_dir)?;
    if files.is_empty() {
        bail!(
            "aucun tableur dans {} : télécharge les fichiers de http://tennis-data.co.uk/data.php dans les sous-dossiers atp/ et wta/ (un fichier par année, 2013 à aujourd'hui)",
            paths.tennis_data_dir.display()
        );
    }
    for path in &ignored {
        println!("Ignoré (le nom n'est pas une année) : {}", path.display());
    }
    let mut conn = open_database(paths)?;
    let report = tennis_import_files::import_files(&mut conn, &files)?;
    for file in &report.files {
        println!(
            "{} {} : {} match(s), {} ligne(s) rejetée(s), {} paire(s) de cotes écartée(s)",
            file.tour.as_str().to_uppercase(),
            file.season,
            file.matches,
            file.rejected.len(),
            file.invalid_odds
        );
        for rejected in file.rejected.iter().take(5) {
            println!("    ligne {} : {}", rejected.row, rejected.reason);
        }
        if file.rejected.len() > 5 {
            println!("    … et {} autre(s)", file.rejected.len() - 5);
        }
    }
    println!(
        "{} match(s) écrits ou mis à jour, {} ligne(s) rejetée(s). Contrôle : `edge tennis check`.",
        report.total_matches(),
        report.total_rejected()
    );
    Ok(())
}

/// `edge tennis check` : contrôle qualité des données de tennis (signale, ne corrige jamais).
pub fn tennis_check(paths: &Paths) -> Result<()> {
    let conn = open_database(paths)?;
    let year =
        u16::try_from(chrono::Datelike::year(&Utc::now())).unwrap_or(tennis_quality::FIRST_YEAR);
    let report = tennis_quality::run(&conn, year)?;
    if report.seasons.is_empty() {
        bail!("aucun match de tennis en base : lance d'abord `edge tennis import`");
    }
    println!("Circuit  Année  Matchs  Terminés  Abandons  Forfaits  Avec Pinnacle  Hors année");
    for s in &report.seasons {
        println!(
            "{:<7}  {:>5}  {:>6}  {:>8}  {:>8}  {:>8}  {:>9} ({:>3} %)  {:>10}",
            s.tour.as_str().to_uppercase(),
            s.season,
            s.matches,
            s.completed,
            s.retired,
            s.walkovers,
            s.with_pinnacle,
            100 * s.with_pinnacle / s.matches.max(1),
            s.out_of_season
        );
    }
    if !report.missing.is_empty() {
        let list: Vec<String> = report
            .missing
            .iter()
            .map(|(tour, year)| format!("{} {year}", tour.as_str().to_uppercase()))
            .collect();
        println!("\nAnnées sans aucun match : {}", list.join(", "));
    }
    println!(
        "\nCotes aberrantes (marge hors de [1,00 ; 1,30]) : {}",
        report.odds_anomalies.len()
    );
    for a in report.odds_anomalies.iter().take(10) {
        println!(
            "    {} {} – {} ({}) : {}",
            a.date,
            a.winner,
            a.loser,
            a.bookmaker,
            format::decimal(a.implied_sum, 3)
        );
    }
    println!(
        "Dates approximatives (un joueur deux fois le même jour dans le même tournoi, ex. demi-finale et finale) : {}",
        report.repeated_dates
    );
    println!(
        "Tours avancés datés entièrement avant un tour précédent : {}",
        report.round_order_violations
    );
    println!(
        "Noms à la même date dans deux tournois (homonymes probables) : {}",
        report.homonyms.len()
    );
    for h in report.homonyms.iter().take(15) {
        println!(
            "    {} {} : {} jour(s)",
            h.tour.as_str().to_uppercase(),
            h.name,
            h.days
        );
    }
    Ok(())
}

/// `edge tennis tune` : règle l'Elo sur la validation (2022-2023) et gèle la meilleure configuration.
pub fn tennis_tune(paths: &Paths) -> Result<()> {
    let conn = open_database(paths)?;
    let matches = tennis_store::load_matches(&conn)?;
    if matches.is_empty() {
        bail!("aucun match de tennis en base : lance d'abord `edge tennis import`");
    }
    let sessions = tennis_store::tuning_sessions(&conn)?;
    if sessions > 0 {
        println!(
            "Attention : {sessions} session(s) de réglage existent déjà (journalisées, jamais effacées). Chaque session supplémentaire est une occasion de s'ajuster au hasard ; le rapport les compte."
        );
    }
    let result = elo_tuning::tune(&matches)
        .context("aucun match de validation (2022-2023) : importe les années 2022 et 2023")?;
    let mut ranked = result.rows.clone();
    ranked.sort_by(|a, b| a.log_loss.total_cmp(&b.log_loss));
    println!(
        "{} configurations essayées. Les cinq meilleures (log loss de validation) :",
        result.rows.len()
    );
    for row in ranked.iter().take(5) {
        println!(
            "  K = {}/(n+5)^{}, poids de surface {} : {} sur {} matchs",
            format::decimal(row.config.k_base, 0),
            format::decimal(row.config.k_shape, 1),
            format::decimal(row.config.surface_weight, 2),
            format::decimal(row.log_loss, 4),
            row.matches
        );
    }
    tennis_store::insert_tuning(
        &conn,
        &tennis_store::FrozenElo {
            created_at: Utc::now(),
            k_base: result.best.config.k_base,
            k_shape: result.best.config.k_shape,
            surface_weight: result.best.config.surface_weight,
            validation_log_loss: result.best.log_loss,
            validation_matches: result.best.matches,
            configs_tried: result.rows.len(),
        },
    )?;
    println!(
        "Configuration gelée. Le test (à partir du 2024-01-01) n'a pas été lu. Rapport : `edge tennis report`."
    );
    Ok(())
}

/// `edge tennis report` : rapport HTML, l'Elo contre le marché sur le test.
pub fn tennis_report(paths: &Paths, out: Option<&std::path::Path>) -> Result<()> {
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let now = Utc::now();
    let year = u16::try_from(chrono::Datelike::year(&now)).unwrap_or(tennis_quality::FIRST_YEAR);
    let report = tennis_report::build(
        &conn,
        config.margin_method,
        &now.format("%Y-%m-%d").to_string(),
        year,
    )?;
    let html = render_tennis_report(&report)?;
    let target = match out {
        Some(path) => path.to_path_buf(),
        None => paths.reports_dir.join("tennis.html"),
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    }
    std::fs::write(&target, html).with_context(|| format!("écriture de {}", target.display()))?;
    println!("Rapport écrit : {}", target.display());
    Ok(())
}

/// `edge nba snapshot` : cotes du soir et résultats de la NBA (lancé par le timer de 22 h UTC).
pub fn nba_snapshot(paths: &Paths, dry_run: bool) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    if !config.nba_archive {
        bail!("l'archive NBA est désactivée (nba_archive = false dans la configuration)");
    }
    let mut conn = open_database(paths)?;
    if dry_run {
        println!("Aucun appel n'est effectué (--dry-run).");
        println!(
            "Cotes : {} marché(s) ({}) × {} bookmaker(s) = {} crédit(s) par relevé, si la NBA est en saison.",
            config.nba_markets.len(),
            config.nba_markets.join(", "),
            config.bookmakers.len(),
            nba::odds_cost(&config)
        );
        match nba::next_scores_call(&conn, Utc::now())? {
            None => println!(
                "Résultats : dus au prochain relevé ({} crédits).",
                nba::SCORES_COST
            ),
            Some(due) => println!(
                "Résultats : prochain appel à partir de {} ({} crédits).",
                format::kickoff_label(due),
                nba::SCORES_COST
            ),
        }
        return Ok(());
    }
    let client = OddsApiClient::new(api_key_from(&config)?);
    let report = nba::run(&mut conn, &config, &mut Utc::now, &mut |path, params| {
        client.get(path, params)
    })?;
    if !report.in_season {
        println!("La NBA n'est pas en saison : aucun crédit dépensé.");
        return Ok(());
    }
    match (&report.odds, &report.odds_failure) {
        (Some(odds), _) => println!(
            "Cotes : {} match(s), {} cote(s) archivée(s) ({} déjà commencés ignorés)",
            odds.stored.events, odds.stored.quotes, odds.already_started
        ),
        (None, Some(reason)) => eprintln!("Cotes non archivées : {reason}"),
        (None, None) => {}
    }
    let scores = &report.scores;
    if scores.fetched {
        println!(
            "Résultats : {} match(s) terminé(s) écrit(s), {} à venir ou en cours{}",
            scores.stored,
            scores.not_finished,
            if scores.malformed > 0 {
                format!(", {} sans score lisible (non écrits)", scores.malformed)
            } else {
                String::new()
            }
        );
    }
    if let Some(reason) = &scores.skipped {
        println!("Résultats non demandés : {reason}");
    }
    println!(
        "Crédits utilisés : {} → {} (plafond {})",
        report.used_before, report.used_after, config.monthly_credit_cap
    );
    if report.odds_failure.is_some() {
        bail!("les cotes de la NBA n'ont pas été archivées");
    }
    Ok(())
}

/// `edge nba tune` : règle le modèle sur la validation (15/11/2026 → 01/02/2027) et le gèle ; refusé avant le 01/02/2027.
pub fn nba_tune(paths: &Paths) -> Result<()> {
    let conn = open_database(paths)?;
    let today = Utc::now().date_naive();
    if today < nba_tuning::TEST_START {
        bail!(
            "la fenêtre de validation se termine le {} : régler avant ce jour figerait un modèle sur une validation incomplète. D'ici là, `edge nba report` donne un diagnostic exploratoire",
            nba_tuning::TEST_START.format("%d/%m/%Y")
        );
    }
    let games = nba_store::load_games(&conn)?;
    let sessions = nba_store::tuning_sessions(&conn)?;
    if sessions > 0 {
        println!(
            "Attention : {sessions} session(s) de réglage existent déjà (journalisées, jamais effacées). Chaque session supplémentaire est une occasion de s'ajuster au hasard ; le rapport les compte."
        );
    }
    let tuned = nba_tuning::tune(&games).context(
        "aucun match de validation exploitable : l'archive est-elle complète depuis novembre ?",
    )?;
    println!(
        "{} configurations essayées. Retenu : K {}, avantage du terrain {} Elo, dos à dos {} Elo, conservation {}, pas du total {}.",
        tuned.configs_tried,
        format::decimal(tuned.elo.k, 0),
        format::decimal(tuned.elo.home_advantage, 0),
        format::decimal(tuned.elo.back_to_back, 0),
        format::decimal(tuned.elo.carry_over, 2),
        format::decimal(tuned.totals_rate, 2)
    );
    println!(
        "Validation : {} matchs, log loss {}, {} points de marge par point d'Elo (σ marge {}, σ total {}).",
        tuned.validation_games,
        format::decimal(tuned.validation_log_loss, 4),
        format::decimal(tuned.margin_per_elo, 4),
        format::decimal(tuned.margin_sigma, 1),
        format::decimal(tuned.total_sigma, 1)
    );
    nba_store::insert_tuning(
        &conn,
        &nba_store::FrozenNba {
            created_at: Utc::now(),
            k: tuned.elo.k,
            home_advantage: tuned.elo.home_advantage,
            back_to_back: tuned.elo.back_to_back,
            carry_over: tuned.elo.carry_over,
            totals_rate: tuned.totals_rate,
            margin_per_elo: tuned.margin_per_elo,
            margin_sigma: tuned.margin_sigma,
            total_sigma: tuned.total_sigma,
            validation_log_loss: tuned.validation_log_loss,
            validation_games: tuned.validation_games,
            configs_tried: tuned.configs_tried,
        },
    )?;
    println!(
        "Configuration gelée. Le test (à partir du 01/02/2027) n'a pas été lu. Rapport : `edge nba report`."
    );
    Ok(())
}

/// `edge nba report` : rapport HTML (exploratoire tant que rien n'est gelé, puis sur le test).
pub fn nba_report(paths: &Paths, out: Option<&std::path::Path>) -> Result<()> {
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let now = Utc::now();
    let report = nba_report::build(
        &conn,
        config.margin_method,
        &now.format("%Y-%m-%d").to_string(),
    )?;
    if report.exploratory {
        println!(
            "Mode exploratoire : aucune configuration gelée, les chiffres décrivent sans rien valider."
        );
    }
    let html = render_nba_report(&report)?;
    let target = match out {
        Some(path) => path.to_path_buf(),
        None => paths.reports_dir.join("nba.html"),
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    }
    std::fs::write(&target, html).with_context(|| format!("écriture de {}", target.display()))?;
    println!("Rapport écrit : {}", target.display());
    Ok(())
}

/// `edge notify-test` : envoie un message de test (avec une pièce jointe) pour vérifier le webhook Discord.
pub fn notify_test(paths: &Paths) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let webhook = config.discord_webhook(std::env::var(WEBHOOK_ENV).ok().as_deref())?;
    let page = "<!doctype html><meta charset=\"utf-8\"><title>Test edge</title><p>Le webhook fonctionne.</p>";
    discord::send(
        &webhook,
        "**edge** · test de notification : le webhook fonctionne. Aucune recommandation.",
        Some(discord::Attachment {
            filename: "test-edge.html",
            content_type: "text/html; charset=utf-8",
            bytes: page.as_bytes(),
        }),
        &mut discord::http_post,
    )?;
    println!("Message de test envoyé sur Discord.");
    Ok(())
}

/// `edge recommend` : écrit les recommandations du dernier relevé dans le journal immuable et notifie sur Discord.
pub fn recommend(paths: &Paths, notify: bool) -> Result<()> {
    let mut conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    // Échouer avant d'écrire si la notification demandée est impossible : mieux vaut le savoir tout de suite.
    let webhook = if notify {
        Some(config.discord_webhook(std::env::var(WEBHOOK_ENV).ok().as_deref())?)
    } else {
        None
    };
    let staking = staking(&conn, &config, None, None, None)?;
    let now = Utc::now();
    let settings = recommend::Settings {
        method: config.margin_method,
        min_edge: recommend::RECOMMEND_MIN_EDGE,
        // Déplacement (pas de `clone`) : `config` ne sert plus après, `Settings` peut prendre possession de la liste.
        excluded_bookmakers: config.excluded_bookmakers,
        kelly_fraction: staking.kelly_fraction,
        min_stake: staking.min_stake,
        bankroll: staking.bankroll,
        code_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let report = recommend::run(&mut conn, &settings, now)?;

    println!(
        "{} match(s) à venir ; {} sans cotes complètes de Pinnacle (ignorés) ; {} prix aberrant(s) écarté(s).",
        report.events, report.without_reference, report.implausible_quotes
    );
    println!(
        "{} recommandation(s) nouvelle(s), {} ligne(s) de journal fantôme, {} déjà connue(s).",
        report.recommended().count(),
        report.shadow().count(),
        report.already_known
    );
    if !report.priced.is_empty() {
        println!("Prix les plus proches de la valeur (information, pas des recommandations) :");
        for found in report.priced.iter().take(notify::message::CLOSEST_PRICES) {
            let s = &found.selection;
            println!(
                "  {} · {} – {} · {} · {} à {} · cote juste {} · avantage {} % · Kelly {}",
                format::kickoff_label(found.kickoff),
                found.home_team,
                found.away_team,
                found.bookmaker,
                format::outcome_label(s.outcome),
                format::decimal(s.odds.get(), 2),
                format::decimal(1.0 / s.probability.get(), 2),
                format::signed(s.edge * 100.0, 1),
                format::units(to_units(
                    kelly::kelly_fraction(s.probability, s.odds) * staking.kelly_fraction
                )),
            );
        }
    }
    for found in report.recommended() {
        let r = &found.record;
        print_pick(&Pick {
            kickoff: r.kickoff,
            home: &r.home_team,
            away: &r.away_team,
            sport_key: &r.sport_key,
            bookmaker: &r.bookmaker,
            outcome: r.outcome,
            odds: r.odds.get(),
            probability: r.probability.get(),
            edge: r.edge,
            minimal_odds: r.minimal_odds,
            stake: r.stake,
            event_id: &r.event_id,
        });
        let stake = r.stake.map_or_else(
            || "sous le minimum".to_string(),
            |s| format::units(to_units(s.get() / r.bankroll.get())),
        );
        println!("    recommandation n° {} · mise {stake}", found.id);
    }

    // Le message part chaque jour, même sans recommandation : il montre ce que l'algorithme a regardé (et alerte si Pinnacle manque).
    let Some(webhook) = webhook else {
        return Ok(());
    };
    let recommended: Vec<_> = report.recommended().collect();
    let text = notify::message::daily(&notify::message::Daily {
        now,
        events: report.events,
        without_reference: report.without_reference,
        min_edge: recommend::RECOMMEND_MIN_EDGE,
        kelly_fraction: staking.kelly_fraction,
        bankroll: staking.bankroll.get(),
        recommended: &recommended,
        priced: &report.priced,
    });
    let data = recommend_report::build(
        &report,
        recommend::RECOMMEND_MIN_EDGE,
        staking.kelly_fraction,
        staking.bankroll.get(),
        now,
    );
    let html = render_recommend_report(&data)?;
    let filename = format!("recommandations-{}.html", now.format("%Y-%m-%d"));
    discord::send(
        &webhook,
        &text,
        Some(discord::Attachment {
            filename: &filename,
            content_type: "text/html; charset=utf-8",
            bytes: html.as_bytes(),
        }),
        &mut discord::http_post,
    )?;
    println!("Notification envoyée sur Discord.");
    Ok(())
}

fn percent_or_dash(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |v| format!("{:+.1} %", v * 100.0))
}

/// `edge backtest run` : évalue les seuils gelés sur le test et journalise les exécutions.
pub fn backtest_run(paths: &Paths) -> Result<()> {
    let mut conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let inputs = load_inputs(&conn)?;
    let predictions = study::load_model_predictions(&conn)?;

    let studies = study::study_all(&conn, &inputs, &predictions, config.margin_method)?;
    let stored = study::persist_test_runs(
        &mut conn,
        &inputs,
        &predictions,
        config.margin_method,
        Utc::now(),
    )?;

    for study in &studies {
        let test = study.periods.last();
        let (bets, clv) = test.map_or((0, None), |p| (p.bets, p.clv));
        println!(
            "{} (seuil {:.0} %) : {} paris au test, CLV {}",
            study.label,
            study.threshold * 100.0,
            bets,
            clv.map_or("—".to_string(), |c| format!(
                "{:+.2} % [{:+.2} ; {:+.2}]",
                c.mean * 100.0,
                c.low * 100.0,
                c.high * 100.0
            ))
        );
        for row in &study.staking {
            println!(
                "    Kelly ×{}: bankroll finale {:.0} €, drawdown {:.0} %",
                row.kelly_fraction,
                row.summary.final_bankroll,
                row.summary.max_drawdown * 100.0
            );
        }
    }
    println!("{stored} exécution(s) nouvellement journalisée(s).");
    Ok(())
}

/// `edge backtest report` : rapport HTML des règles de sélection (seuils gelés, CLV, Kelly, ruine).
pub fn backtest_report(paths: &Paths, out: Option<&std::path::Path>) -> Result<()> {
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let today = Utc::now().date_naive();

    println!("Calcul en cours (simulations Monte Carlo, quelques secondes)…");
    let report = backtest_report::build(
        &conn,
        config.margin_method,
        &today.format("%Y-%m-%d").to_string(),
    )?;
    let html = render_backtest_report(&report)?;

    let target = match out {
        Some(path) => path.to_path_buf(),
        None => paths.reports_dir.join("backtest.html"),
    };
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    }
    std::fs::write(&target, html).with_context(|| format!("écriture de {}", target.display()))?;
    println!("Rapport écrit : {}", target.display());
    Ok(())
}

/// `edge backtest curve` : l'abaque des seuils, en lecture seule (rien n'est gelé).
pub fn backtest_curve(paths: &Paths, out: Option<&std::path::Path>) -> Result<()> {
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let today = Utc::now().date_naive();

    println!("Abaque sur l'entraînement et la validation (le test n'est pas lu)…");
    let report = thresholds_report::build(
        &conn,
        &Aliases::bundled()?,
        config.margin_method,
        &today.format("%Y-%m-%d").to_string(),
    )?;
    for source in &report.sources {
        let zero = &source.curve.points[0];
        match &source.best {
            Some(best) => println!(
                "{:<10} {:>5} paris à 0 % · meilleur seuil {} % : {} paris, croissance ×1 {} (bankroll × {})",
                source.curve.label,
                zero.bets,
                format::decimal(best.point.threshold * 100.0, 1),
                best.point.bets,
                format::decimal(best.point.growth[0], 3),
                format::decimal(best.multiplier, 2)
            ),
            None => println!(
                "{:<10} {:>5} paris à 0 % · aucun seuil ne garde {} paris",
                source.curve.label, zero.bets, report.min_selection_bets
            ),
        }
    }
    let html = render_thresholds_report(&report)?;
    let target = out.map_or_else(
        || paths.reports_dir.join("thresholds.html"),
        std::path::Path::to_path_buf,
    );
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    }
    std::fs::write(&target, html).with_context(|| format!("écriture de {}", target.display()))?;
    println!("Rapport écrit : {}", target.display());
    println!(
        "Rien n'est gelé. Pour geler ces seuils (nouvelle session, comptée) : edge backtest freeze"
    );
    Ok(())
}

/// `edge backtest freeze` : gèle le seuil de plus forte croissance attendue de chaque bookmaker.
pub fn backtest_freeze(paths: &Paths) -> Result<()> {
    let conn = open_database(paths)?;
    let config = Config::load(&paths.config_file)?;
    let inputs = load_inputs(&conn)?;
    let curves = curve::curves(&inputs, config.margin_method)?;
    let frozen = curve::freeze(&conn, &curves, Utc::now())?;
    for record in &frozen {
        println!(
            "{:<10} seuil gelé à {} % ({} paris au réglage, croissance ×1 {})",
            record.source,
            format::decimal(record.threshold * 100.0, 1),
            record.selection_bets,
            record
                .selection_growth
                .map_or("—".to_string(), |g| format::decimal(g, 3))
        );
    }
    let sessions = edge_core::store::backtests::selection_session_count(&conn)?;
    println!(
        "{} seuil(s) gelé(s), {} configurations essayées ; {sessions} session(s) de réglage au total.",
        frozen.len(),
        curve::configs_tried()
    );
    println!("Évaluation unique sur le test : edge backtest run, puis edge backtest report");
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
    if config.tennis_archive {
        println!(
            "  + les tournois de tennis en cours, découverts au moment du relevé (gratuit) : {} crédit(s) chacun, {} au plus",
            plan.cost_per_call, config.tennis_max_tournaments
        );
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
    store::open_migrated(&paths.db_file).context("base inaccessible : lance d'abord `edge init`")
}
