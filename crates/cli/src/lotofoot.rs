//! `edge lotofoot …` : Loto Foot (phase 14). Relevé horaire des grilles (FDJ, repli resultats-fdj.fr), séance du samedi,
//! tickets joués, bilan.
//!
//! La logique vit dans `edge_core::lotofoot` ; ce fichier lit la configuration, branche les vrais clients et affiche.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use edge_core::config::{Config, WEBHOOK_ENV};
use edge_core::domain::Euros;
use edge_core::format::{chance, decimal, kickoff_label};
use edge_core::ingest::aliases::Aliases;
use edge_core::lotofoot::alert;
use edge_core::lotofoot::bulletin;
use edge_core::lotofoot::fetch::{self, FetchReport, SiteClient};
use edge_core::lotofoot::live::{self, Notified};
use edge_core::lotofoot::outlets::Outlet;
use edge_core::lotofoot::played::{self, Declared, Manual};
use edge_core::lotofoot::session::{self, Request, SessionReport};
use edge_core::lotofoot::settle::{self, Ledger};
use edge_core::lotofoot::verify;
use edge_core::lotofoot::{Game, GridId, Ticket};
use edge_core::notify::discord;
use edge_core::paths::Paths;
use edge_core::report::lotofoot_page;
use edge_core::store;

/// `edge lotofoot fetch` : relève les grilles ouvertes et les résultats (sans crédit) sur la FDJ, et sur resultats-fdj.fr
/// si la FDJ échoue ; `result_pages` pages de résultats au plus (une suffit au quotidien). `--notify` signale sur Discord
/// chaque grille vue pour la première fois, avec son heure de clôture, et les alertes de source.
pub fn fetch(paths: &Paths, notify: bool, result_pages: u32) -> Result<()> {
    let mut conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    let report = run_fetch(&mut conn, result_pages)?;
    print_fetch(&report);
    if notify {
        let now = Utc::now();
        let listed = alert::pending(&conn, now)?;
        if !listed.is_empty() {
            let config = Config::load(&paths.config_file)?;
            let webhook = config.discord_webhook(std::env::var(WEBHOOK_ENV).ok().as_deref())?;
            let outlets = config
                .lotofoot_outlets
                .iter()
                .map(Outlet::from_config)
                .collect::<edge_core::error::Result<Vec<_>>>()?;
            let text =
                lotofoot_page::listing_text(&listed, &outlets, now, config.site_url.as_deref());
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
        send_source_alerts(paths, &conn, &report, now)?;
    }
    if !report.failures.is_empty() {
        // Code de sortie non nul : systemd marque l'unité en échec ; le passage suivant relira les pages.
        bail!("{} page(s) non lue(s)", report.failures.len());
    }
    Ok(())
}

/// Envoie sur Discord les alertes de source dues (page périmée, FDJ en échec avec repli, aucune source, grille annulée) et
/// les inscrit au journal des alertes (`alert_log`) : `edge_core::lotofoot::alert::source_alerts` décide de ce qui est dû
/// et de la cadence de chaque alerte. Une page périmée, un repli ou une grille annulée ne sont pas un échec du relevé
/// (les pages ont été lues) : le code de sortie reste nul, ce sont ces alertes et la pastille de l'accueil qui préviennent.
fn send_source_alerts(
    paths: &Paths,
    conn: &store::Connection,
    report: &FetchReport,
    now: DateTime<Utc>,
) -> Result<()> {
    let due = alert::source_alerts(conn, report, now)?;
    if due.is_empty() {
        return Ok(());
    }
    let config = Config::load(&paths.config_file)?;
    let webhook = config.discord_webhook(std::env::var(WEBHOOK_ENV).ok().as_deref())?;
    for alert in &due {
        // Comme pour les nouvelles grilles : rien n'est noté si l'envoi échoue, l'alerte repartira au relevé suivant.
        discord::send(&webhook, &alert.text, &[], &mut discord::http_post)
            .context("alerte Discord de source non envoyée")?;
        alert::record_source_alert(conn, alert, now)?;
        println!(
            "Alerte de source signalée sur Discord ({}).",
            alert.subjects.join(", ")
        );
    }
    Ok(())
}

fn run_fetch(conn: &mut store::Connection, result_pages: u32) -> Result<FetchReport> {
    let client = SiteClient::new();
    let aliases = Aliases::bundled()?;
    Ok(fetch::run(
        conn,
        &aliases,
        &mut Utc::now,
        &mut |url, params| client.get(url, params),
        &mut std::thread::sleep,
        result_pages,
    )?)
}

fn print_fetch(report: &FetchReport) {
    for shown in &report.stale {
        println!("  SOURCE FIGÉE : {}", shown.reason());
    }
    for source in &report.sources {
        println!("Source : {source}.");
    }
    if report.grids.is_empty() && report.stale.is_empty() && report.failures.is_empty() {
        println!("Aucune grille LF7 ou LF8 ouverte (aucune n'est affichée par la source lue).");
    } else if !report.grids.is_empty() {
        println!("Grilles en cours : {}.", report.grids.join(" ; "));
    }
    println!(
        "Répartitions de rencontres archivées : {} ; grilles nouvellement résultées : {}.",
        report.splits, report.new_results
    );
    for failure in &report.failures {
        println!("  échec : {} : {}", failure.page, failure.reason);
    }
    for warning in &report.warnings {
        println!("  attention : {warning}");
    }
}

/// `edge lotofoot recommend` : séance (relevé frais, cotes, tickets), écrite au journal ; `--notify` l'envoie sur Discord.
/// Même chemin que le bouton « Optimiser maintenant » de l'application Web, avec le budget mensuel en vigueur.
pub fn recommend(paths: &Paths, notify: bool, dry_run: bool) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let mut conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    let aliases = Aliases::bundled()?;
    // Le timer ne remplace jamais une recommandation : il n'analyse que les grilles nouvelles. Budget : celui saisi dans
    // l'application, sinon celui de la configuration.
    let budget = session::monthly_budget(&conn, &config, Utc::now())?;
    let request = Request::from_budget(budget, false, dry_run)?;
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
            "  {} (clôture {}) : mises estimées {} €, meilleur ticket à {} (scénario défavorable)",
            e.grid,
            kickoff_label(e.closes_at),
            decimal(e.stakes, 0),
            decimal(e.best_pessimistic, 2)
        );
    }
    for s in &report.skipped {
        println!("  écartée : {} — {}", s.grid, s.reason);
    }
    if !report.months.is_empty() {
        println!(
            "  budget : {} ; {}",
            lotofoot_page::months_text(&report.months),
            lotofoot_page::threshold_text(report)
        );
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
    if let Some(oddspapi) = lotofoot_page::oddspapi_text(&report.oddspapi) {
        println!("  {oddspapi}");
    }
    if report.chosen.is_empty() {
        println!("\nAucun nouveau ticket : rien de plus à jouer.");
        return;
    }
    println!("\n{} ticket(s) à 1 € :", report.chosen.len());
    for (i, c) in report.chosen.iter().enumerate() {
        println!(
            "  {:>2}. {}  {}  gagne {}, espérance {} (défavorable {})",
            i + 1,
            c.grid,
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
        let outlook =
            report
                .evaluated
                .iter()
                .find(|e| e.grid == l.grid)
                .map_or(String::new(), |e| {
                    format!(
                        ", {}",
                        lotofoot_page::ranks_text(&lotofoot_page::line_ranks(&e.model, &l.line))
                    )
                });
        println!(
            "  {}  {}  {} €{outlook}  (tickets {})",
            l.grid,
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
    /// Grille d'un ticket saisi à la main (`lf7:133`, année en cours, ou `lf7:2026:133`).
    pub grid: Option<String>,
    /// Ticket saisi à la main (`1N22121`).
    pub ticket: Option<String>,
    /// Mise par ticket.
    pub stake: f64,
}

/// Grille d'une option : `lf7:133` (année en cours, heure de Paris) ou `lf7:2026:133`.
fn parse_grid(text: &str, now: DateTime<Utc>) -> Option<GridId> {
    let parts: Vec<&str> = text.split(':').collect();
    match parts[..] {
        [game, number] => Some(GridId::closing(
            Game::from_code(game)?,
            number.parse().ok()?,
            now,
        )),
        [game, year, number] => Some(GridId {
            game: Game::from_code(game)?,
            year: year.parse().ok()?,
            number: number.parse().ok()?,
        }),
        _ => None,
    }
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
        let grid =
            parse_grid(grid, now).context("grille attendue au format lf7:133 ou lf7:2026:133")?;
        let ticket = Ticket::parse(code).context("ticket attendu au format 1N22121")?;
        match played::declare_manual(&conn, grid, &ticket, now, stake)? {
            Manual::AlreadyDeclared(id) => {
                println!("  {grid} {ticket} : ticket recommandé n°{id} déjà déclaré joué, ignoré")
            }
            Manual::Recorded(reco_id) => {
                println!(
                    "  {grid} {ticket} enregistré{}",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_option_defaults_to_the_current_year() {
        let now = DateTime::parse_from_rfc3339("2027-02-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let grid = |year, number| GridId {
            game: Game::Lf7,
            year,
            number,
        };
        assert_eq!(parse_grid("lf7:12", now), Some(grid(2027, 12)));
        assert_eq!(parse_grid("lf7:2026:133", now), Some(grid(2026, 133)));
        assert_eq!(parse_grid("lf7", now), None);
        assert_eq!(parse_grid("lf9:12", now), None);
    }
}
