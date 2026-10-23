//! `edge loto …` : Grand Loto et alerte EuroMillions (phase 16). Relevé de l'historique FDJ, séance d'une édition,
//! grilles jouées, rapport d'étude.
//!
//! La logique vit dans `edge_core::loto` ; ce fichier lit la configuration, branche les vrais clients et affiche.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDate, Utc};
use edge_core::config::{Config, WEBHOOK_ENV};
use edge_core::loto::alert::{self, Notified};
use edge_core::loto::fdj::{self, FdjClient};
use edge_core::loto::played;
use edge_core::loto::popularity::Verdict;
use edge_core::loto::recommend::{self, Outcome, SessionReport};
use edge_core::loto::{Grid, rules};
use edge_core::notify::discord;
use edge_core::paths::Paths;
use edge_core::report::loto_report;
use edge_core::report::render::render_loto_report;
use edge_core::store;

fn open(paths: &Paths) -> Result<store::Connection> {
    store::open_migrated(&paths.db_file).context("base inaccessible : lance d'abord `edge init`")
}

/// `edge loto fetch` : relève l'historique FDJ, puis cherche l'alerte EuroMillions (`--notify` : l'envoie sur Discord).
pub fn fetch(paths: &Paths, notify: bool) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let mut conn = open(paths)?;
    let client = FdjClient::new();
    let report = fdj::run(
        &mut conn,
        &mut Utc::now,
        &mut |url| client.get(url),
        &mut std::thread::sleep,
    )?;
    for read in &report.read {
        println!("Lu : {read}");
    }
    println!("Tirages nouveaux : {}.", report.new_draws);
    if report.changed > 0 {
        println!(
            "Attention : {} tirage(s) déjà connu(s) publié(s) autrement (non réécrits ; la première publication fait foi).",
            report.changed
        );
    }
    for failure in &report.failures {
        println!("  échec : {failure}");
    }

    let (pending, notified) = alert::notify_pending(&conn, &config, Utc::now(), notify)?;
    if let Some(alert) = &pending {
        println!("\n{}", alert.message);
    }
    match notified {
        Notified::Nothing => println!("EuroMillions : aucune alerte due."),
        Notified::AlreadySent => println!("(alerte déjà envoyée)"),
        Notified::Shown => println!("(alerte affichée seulement : ajoute --notify pour l'envoyer)"),
        Notified::Sent => println!("Alerte envoyée sur Discord."),
        Notified::Failed(error) => bail!("alerte EuroMillions non envoyée : {error}"),
    }
    if !report.failures.is_empty() {
        // Code de sortie non nul : systemd marque l'unité en échec ; le passage suivant relira les archives.
        bail!("{} archive(s) non lue(s)", report.failures.len());
    }
    Ok(())
}

/// `edge loto recommend` : séance de l'édition due (ou de `--date`), écrite au journal ; `--notify` l'envoie sur Discord.
pub fn recommend(
    paths: &Paths,
    date: Option<NaiveDate>,
    notify: bool,
    dry_run: bool,
) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let mut conn = open(paths)?;
    match recommend::run(&mut conn, &config, Utc::now(), date, dry_run)? {
        Outcome::NotDue => {
            println!(
                "Aucune édition du Grand Loto dans les {} jours (grand_loto_editions dans edge.toml).",
                config.grand_loto_days_before
            );
        }
        Outcome::AlreadyDone(session) => {
            println!(
                "La séance de l'édition du {} existe déjà (n° {}, {}) : elle n'est jamais refaite.",
                session.draw_date.format("%d/%m/%Y"),
                session.id,
                session.created_at.format("%d/%m/%Y %H:%M UTC")
            );
        }
        Outcome::Done(report) => {
            print_session(&report, dry_run);
            if notify && !dry_run {
                let webhook = config.discord_webhook(std::env::var(WEBHOOK_ENV).ok().as_deref())?;
                let text = recommend::discord_text(&report);
                // La séance est écrite ; l'échec du message fait quand même échouer la commande, pour que le timer le
                // signale.
                discord::send(&webhook, &text, &[], &mut discord::http_post)
                    .context("séance écrite, mais message Discord non envoyé")?;
                println!("Séance envoyée sur Discord.");
            }
        }
    }
    Ok(())
}

fn print_session(report: &SessionReport, dry_run: bool) {
    let mode = if dry_run {
        " (à blanc : rien n'est écrit)"
    } else {
        ""
    };
    println!(
        "Grand Loto du {}{mode} : jackpot {:.0} M€, grille à {:.2} €.",
        report.date.format("%d/%m/%Y"),
        report.edition.jackpot / 1e6,
        report.edition.price
    );
    println!(
        "  Grilles prévues : {:.2} millions (pessimiste, {:.0}e centile : {:.2} millions), d'après {} édition(s) de la saison « {} ».",
        report.forecast.central / 1e6,
        report.forecast.quantile * 100.0,
        report.forecast.pessimistic / 1e6,
        report.forecast.editions.len(),
        report.forecast.season.label()
    );
    println!(
        "  Grille au hasard : {:.2} € rendus ({:.2} € au pessimiste).",
        report.uniform.0, report.uniform.1
    );
    let verdict = match report.walk.verdict {
        Verdict::Validated => "validées : grilles de numéros délaissés",
        Verdict::Rejected => "non validées : grilles au hasard",
    };
    println!(
        "  Préférences du public {verdict} (gain hors échantillon {:.1} [{:.1} ; {:.1}] par tirage).",
        report.walk.gain.mean, report.walk.gain.low, report.walk.gain.high
    );
    for c in &report.candidates {
        println!(
            "  {} {} : {:.2} € rendus ({:.2} € au pessimiste, popularité {:.2})",
            if c.kept { "à jouer" } else { "écartée" },
            c.grid,
            c.ev,
            c.ev_pessimistic,
            c.popularity
        );
    }
    if report.kept().next().is_none() {
        println!(
            "  Décision : ne pas jouer (seuil {:.2} € rendus par euro au pessimiste).",
            report.threshold
        );
    }
    if let Some(id) = report.session_id {
        println!("  Séance n° {id} écrite au journal.");
    }
}

/// Options de `edge loto played`.
pub struct PlayedOptions {
    /// Numéros des grilles recommandées.
    pub ids: Vec<i64>,
    /// Toutes les grilles de la dernière séance.
    pub last_session: bool,
    /// Grille saisie à la main.
    pub grid: Option<String>,
    /// Édition de la grille saisie à la main.
    pub date: Option<NaiveDate>,
    /// Mise de la grille saisie à la main.
    pub stake: f64,
}

/// `edge loto played` : déclare des grilles jouées.
pub fn played(paths: &Paths, options: &PlayedOptions) -> Result<()> {
    let conn = open(paths)?;
    let now = Utc::now();
    if let (Some(text), Some(date)) = (&options.grid, options.date) {
        let grid = Grid::parse(text).with_context(|| {
            format!("grille « {text} » illisible (format attendu : 2-3-30-35-44+10)")
        })?;
        let (id, off_model) = played::declare_manual(&conn, date, grid, options.stake, now)?;
        let kind = if off_model {
            "hors modèle"
        } else {
            "recommandée"
        };
        println!(
            "Grille {grid} du {} déclarée ({kind}), n° {id}.",
            date.format("%d/%m/%Y")
        );
        return Ok(());
    }
    let written = if options.last_session {
        played::declare_last_session(&conn, now)?
    } else if !options.ids.is_empty() {
        played::declare_recos(&conn, &options.ids, now)?
    } else {
        bail!("indique les numéros des grilles, --last-session, ou --grid et --date");
    };
    println!("{} grille(s) déclarée(s) jouée(s).", written.len());
    Ok(())
}

/// `edge loto study` : rapport HTML (Grand Loto passés, préférences, EuroMillions au plafond, journal).
pub fn study(paths: &Paths, output: Option<PathBuf>) -> Result<()> {
    let conn = open(paths)?;
    let report = loto_report::build(&conn, Local::now().date_naive())?;
    let html = render_loto_report(&report)?;
    let target = output.unwrap_or_else(|| paths.reports_dir.join("loto.html"));
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    }
    std::fs::write(&target, html).with_context(|| format!("écriture de {}", target.display()))?;
    println!("Rapport écrit : {}", target.display());
    println!("Grand Loto passés : {}.", report.editions.len());
    for e in &report.editions {
        println!(
            "  {} ({}) : {:.2} millions de grilles, espérance {:+.0} %",
            e.date,
            e.season,
            e.grids,
            e.expectation * 100.0
        );
    }
    if let Some(p) = &report.preferences {
        println!(
            "Préférences du public : gain hors échantillon {:.1} [{:.1} ; {:.1}] par tirage ; pente {} ; {}.",
            p.gain.mean,
            p.gain.low,
            p.gain.high,
            p.slope.map_or("incalculable".to_string(), |s| format!(
                "{:.2} [{:.2} ; {:.2}]",
                s.slope, s.low, s.high
            )),
            if p.validated {
                "validées"
            } else {
                "non validées"
            }
        );
    }
    println!(
        "EuroMillions : {} tirage(s) au plafond depuis 2020 ; {}.",
        report.caps.len(),
        if report.alert.is_some() {
            "alerte due"
        } else {
            "aucune alerte due"
        }
    );
    println!(
        "Prix d'une grille de Grand Loto : {:.2} €.",
        rules::GRAND_LOTO_PRICE
    );
    Ok(())
}
