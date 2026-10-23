//! `edge lotofoot …` : Loto Foot (phase 14). Relevé horaire de prono-match, séance du samedi, tickets joués, bilan.
//!
//! La logique vit dans `edge_core::lotofoot` ; ce fichier lit la configuration, branche les vrais clients et affiche.

use anyhow::{Context, Result, bail};
use chrono::Utc;
use edge_core::config::{Config, WEBHOOK_ENV};
use edge_core::domain::Euros;
use edge_core::format::{chance, decimal, kickoff_label};
use edge_core::ingest::aliases::Aliases;
use edge_core::lotofoot::alert;
use edge_core::lotofoot::bulletin;
use edge_core::lotofoot::fetch::{self, FetchReport, PronoMatchClient};
use edge_core::lotofoot::live::{self, Notified};
use edge_core::lotofoot::played::{self, Declared, Manual};
use edge_core::lotofoot::session::{Request, SessionReport};
use edge_core::lotofoot::settle::{self, Ledger};
use edge_core::lotofoot::verify;
use edge_core::lotofoot::{Game, Ticket};
use edge_core::notify::discord;
use edge_core::paths::Paths;
use edge_core::report::lotofoot_page;
use edge_core::store;

/// `edge lotofoot fetch` : relève les pages du LF7, du LF8 et des résultats (sans crédit) ; `--notify` signale sur Discord
/// chaque grille vue pour la première fois, avec son heure de clôture.
pub fn fetch(paths: &Paths, notify: bool) -> Result<()> {
    let mut conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    let report = run_fetch(&mut conn)?;
    print_fetch(&report);
    if notify {
        let now = Utc::now();
        let listed = alert::pending(&conn, now)?;
        if !listed.is_empty() {
            let config = Config::load(&paths.config_file)?;
            let webhook = config.discord_webhook(std::env::var(WEBHOOK_ENV).ok().as_deref())?;
            let text = lotofoot_page::listing_text(&listed, config.site_url.as_deref());
            // Un échec fait échouer l'unité (visible dans `systemctl --user status`) ; rien n'est noté, donc l'alerte
            // repartira au relevé suivant.
            discord::send(&webhook, &text, &[], &mut discord::http_post)
                .context("alerte Discord des nouvelles grilles non envoyée")?;
            alert::record(&conn, &listed, now)?;
            println!(
                "{} nouvelle(s) grille(s) signalée(s) sur Discord.",
                listed.len()
            );
        }
    }
    if !report.failures.is_empty() {
        // Code de sortie non nul : systemd marque l'unité en échec ; le passage suivant relira les pages.
        bail!("{} page(s) non lue(s)", report.failures.len());
    }
    Ok(())
}

fn run_fetch(conn: &mut store::Connection) -> Result<FetchReport> {
    let client = PronoMatchClient::new();
    Ok(fetch::run(
        conn,
        &mut Utc::now,
        &mut |path, params| client.get(path, params),
        &mut std::thread::sleep,
    )?)
}

fn print_fetch(report: &FetchReport) {
    if report.grids.is_empty() {
        println!("Aucune grille LF7 ou LF8 lue sur prono-match (page modifiée ?).");
    } else {
        println!("Grilles en cours : {}.", report.grids.join(" ; "));
    }
    println!(
        "Répartitions de rencontres archivées : {} ; grilles nouvellement résultées : {}.",
        report.splits, report.new_results
    );
    for failure in &report.failures {
        println!("  échec : {failure}");
    }
}

/// `edge lotofoot recommend` : séance (relevé frais, cotes, tickets), écrite au journal ; `--notify` l'envoie sur Discord.
/// Même chemin que le bouton « Optimiser maintenant » de l'application Web, avec la mise plafond de la configuration.
pub fn recommend(paths: &Paths, notify: bool, dry_run: bool) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let mut conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    let aliases = Aliases::bundled()?;
    // Le timer ne remplace jamais une recommandation : il n'analyse que les grilles nouvelles.
    let request = Request {
        max_tickets: config.lotofoot_max_tickets,
        replace: false,
        dry_run,
    };
    let live = live::recommend(&mut conn, &config, &aliases, request, notify)?;
    print_fetch(&live.fetched);
    print_session(&live.report, dry_run);
    match live.notified {
        Notified::NotAsked => {}
        Notified::Sent => println!("Séance envoyée sur Discord."),
        // La séance est écrite ; l'échec du message fait quand même échouer la commande, pour que le timer le signale.
        Notified::Failed(error) => {
            bail!("séance écrite, mais message Discord non envoyé : {error}")
        }
    }
    Ok(())
}

fn print_session(report: &SessionReport, dry_run: bool) {
    let mode = if dry_run {
        " (à blanc : rien n'est écrit, aucune ligue n'est relevée)"
    } else {
        ""
    };
    println!("\nSéance Loto Foot{mode}");
    for e in &report.evaluated {
        println!(
            "  {} n°{} (clôture {}) : mises estimées {} €, meilleur ticket à {} (scénario défavorable)",
            e.game,
            e.number,
            kickoff_label(e.closes_at),
            decimal(e.stakes, 0),
            decimal(e.best_pessimistic, 2)
        );
    }
    for s in &report.skipped {
        println!("  écartée : {} — {}", s.grid, s.reason);
    }
    if !report.weeks.is_empty() {
        println!("  budget : {}", lotofoot_page::weeks_text(&report.weeks));
    }
    if !report.unknown_names.is_empty() {
        println!(
            "  noms inconnus (à ajouter dans config/aliases.toml) : {}",
            report.unknown_names.join(", ")
        );
    }
    if !report.leagues.fetched.is_empty() || !report.leagues.refused.is_empty() {
        println!(
            "  compétitions relevées : {} ({} crédit(s)) ; refusées : {}",
            report.leagues.fetched.join(", "),
            report.leagues.spent,
            if report.leagues.refused.is_empty() {
                "aucune".to_string()
            } else {
                report.leagues.refused.join(" ; ")
            }
        );
    }
    if report.chosen.is_empty() {
        println!("\nAucun nouveau ticket : rien de plus à jouer.");
        return;
    }
    println!("\n{} ticket(s) à 1 € :", report.chosen.len());
    for (i, c) in report.chosen.iter().enumerate() {
        println!(
            "  {:>2}. {} n°{}  {}  gagne {}, espérance {} (défavorable {})",
            i + 1,
            c.grid.0,
            c.grid.1,
            c.ticket,
            chance(c.base.paid_probability),
            decimal(c.base.expected_return, 2),
            decimal(c.pessimistic.expected_return, 2)
        );
    }
    print_lines(report);
    if let Some(id) = report.session_id {
        println!(
            "Séance n°{id} écrite au journal. Après validation : `edge lotofoot played --last-session`."
        );
    }
}

/// Les mêmes tickets regroupés en grilles multiples sans perte : moins de grilles à remplir, même mise, mêmes gains.
fn print_lines(report: &SessionReport) {
    let tickets: Vec<_> = report.chosen.iter().map(|c| (c.grid, &c.ticket)).collect();
    let lines = bulletin::session_lines(&tickets);
    println!(
        "\nÀ remplir : {} grille(s) pour ces {} tickets (mêmes tickets, même mise ; 2 grilles par formule sur un bulletin) :",
        lines.len(),
        report.chosen.len()
    );
    for l in &lines {
        let ranks: Vec<String> = l.tickets.iter().map(|i| (i + 1).to_string()).collect();
        let outlook = report
            .evaluated
            .iter()
            .find(|e| (e.game, e.number) == l.grid)
            .map_or(String::new(), |e| {
                format!(
                    ", {}",
                    lotofoot_page::ranks_text(&lotofoot_page::line_ranks(&e.model, &l.line))
                )
            });
        println!(
            "  {} n°{}  {}  {} €{outlook}  (tickets {})",
            l.grid.0,
            l.grid.1,
            l.line,
            l.line.combinations(),
            ranks.join(", ")
        );
    }
}

/// Options de `edge lotofoot played`.
pub struct PlayedOptions {
    /// Numéros des tickets recommandés joués.
    pub ids: Vec<i64>,
    /// Tous les tickets de la dernière séance.
    pub last_session: bool,
    /// Grille d'un ticket saisi à la main (`lf7:133`).
    pub grid: Option<String>,
    /// Ticket saisi à la main (`1N22121`).
    pub ticket: Option<String>,
    /// Mise par ticket.
    pub stake: f64,
}

/// `edge lotofoot played` : enregistre les tickets réellement joués (journal immuable ; hors modèle si aucune
/// recommandation ne correspond). La logique est dans `edge_core::lotofoot::played`, partagée avec l'application.
pub fn played(paths: &Paths, options: &PlayedOptions) -> Result<()> {
    let asked = options.last_session || !options.ids.is_empty() || options.grid.is_some();
    if !asked {
        bail!("rien à enregistrer : indique des numéros, --last-session, ou --grid et --ticket");
    }
    let conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    let stake = Euros::new(options.stake)
        .filter(|s| s.get() > 0.0)
        .context("mise invalide")?;
    let now = Utc::now();
    let mut written = 0;
    // Les numéros d'abord : un numéro inconnu est refusé avant que rien ne soit écrit.
    if !options.ids.is_empty() {
        written += print_declared(&played::declare(&conn, &options.ids, now, stake)?);
    }
    if options.last_session {
        written += print_declared(&played::declare_last_session(&conn, now, stake)?);
    }
    if let (Some(grid), Some(code)) = (&options.grid, &options.ticket) {
        let (game, number) = grid
            .split_once(':')
            .and_then(|(g, n)| Some((Game::from_code(g)?, n.parse::<u32>().ok()?)))
            .context("grille attendue au format lf7:133")?;
        let ticket = Ticket::parse(code).context("ticket attendu au format 1N22121")?;
        match played::declare_manual(&conn, game, number, &ticket, now, stake)? {
            Manual::AlreadyDeclared(id) => println!(
                "  {game} n°{number} {ticket} : ticket recommandé n°{id} déjà déclaré joué, ignoré"
            ),
            Manual::Recorded(reco_id) => {
                println!(
                    "  {game} n°{number} {ticket} enregistré{}",
                    if reco_id.is_none() {
                        " (hors modèle)"
                    } else {
                        ""
                    }
                );
                written += 1;
            }
        }
    }
    println!("{written} ticket(s) joué(s) enregistré(s).");
    Ok(())
}

fn print_declared(declared: &Declared) -> usize {
    for id in &declared.already {
        println!("  ticket n°{id} déjà déclaré joué : ignoré");
    }
    declared.written.len()
}

/// `edge lotofoot report` : bilans et vérification du modèle (lecture seule).
pub fn report(paths: &Paths) -> Result<()> {
    let conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    let recommended = settle::recommended(&conn)?;
    let (model, off) = settle::played_ledgers(&conn)?;
    println!("Loto Foot (docs/sources.md §37 et §38)\n");
    print_ledger("Tickets recommandés (suivi du modèle)", &recommended);
    print_ledger("Tickets joués du modèle", &model);
    print_ledger("Tickets joués hors modèle", &off);
    let v = verify::verify(&conn, Utc::now())?;
    println!("\nVérification : {}", lotofoot_page::verdict_text(&v));
    println!(
        "  fréquence : {} ticket(s) payé(s) pour {} attendu(s) (probabilité d'en avoir aussi peu : {})",
        v.frequency.observed,
        decimal(v.frequency.expected, 2),
        decimal(v.frequency.p_low, 3)
    );
    println!(
        "  gain : {} € réalisés pour {} € attendus, centile {} de la loi simulée",
        decimal(v.gain.realized, 2),
        decimal(v.gain.simulated_mean, 2),
        decimal(v.gain.percentile, 3)
    );
    for b in v.bands.iter().filter(|b| b.observations > 0) {
        println!(
            "  λ {} à {} : {} observation(s), publiés / prédits = {}{}",
            b.low,
            b.high,
            b.observations,
            b.ratio().map_or("—".into(), |r| decimal(r, 2)),
            if b.failing() { "  ← hors bornes" } else { "" }
        );
    }
    Ok(())
}

fn print_ledger(label: &str, l: &Ledger) {
    println!(
        "{label} : {} ticket(s), {} réglé(s), mises {} €, gains {} €, rendement {}, payés {}",
        l.tickets,
        l.settled,
        decimal(l.staked, 0),
        decimal(l.returned, 2),
        l.roi().map_or("—".into(), |r| format!(
            "{} %",
            edge_core::format::signed(r * 100.0, 1)
        )),
        l.paid
    );
}
