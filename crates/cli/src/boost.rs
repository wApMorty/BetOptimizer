//! `edge boost study` : étude historique des cotes boostées publiées par WePari, affichée en tableaux texte.
//!
//! Le calcul vit dans `edge_core::boost::study` ; ce fichier ne fait que charger les pages, regrouper et afficher.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use edge_core::boost::Operator;
use edge_core::boost::pricing::Rho;
use edge_core::boost::study::{self, StudyRow};
use edge_core::boost::wepari;
use edge_core::config::Config;
use edge_core::domain::Moment;
use edge_core::eval::bootstrap::MeanCi;
use edge_core::format;
use edge_core::ingest::aliases::Aliases;
use edge_core::paths::Paths;
use edge_core::store::{self, Connection};

/// Effectif minimal d'un groupe pour être affiché : en dessous, l'intervalle de confiance ne dit rien.
const MIN_GROUP: usize = 30;

/// Variantes de la grille comparées (ρ ajusté, ou fixé à deux valeurs) : trois configurations essayées.
const RHO_VARIANTS: [Rho; 3] = [Rho::Fitted, Rho::Fixed(0.0), Rho::Fixed(-0.1)];

/// Période de choix de la grille : les deux saisons qui précèdent la première donnée de WePari (août 2021). L'étude
/// des boosts porte ensuite sur des matchs que ce choix n'a pas vus.
const SELECTION_FROM: (i32, u32, u32) = (2019, 8, 1);
const SELECTION_TO: (i32, u32, u32) = (2021, 7, 31);

/// Seuils d'avantage d'avant-match testés comme règles de sélection.
const THRESHOLDS: [f64; 4] = [0.0, 0.025, 0.05, 0.10];

/// `edge boost study`.
pub fn study(paths: &Paths) -> Result<()> {
    let files = html_files(paths)?;
    let mut pages = Vec::new();
    for file in &files {
        let html = std::fs::read_to_string(file)
            .with_context(|| format!("lecture de {}", file.display()))?;
        let page =
            wepari::parse(&html).with_context(|| format!("lecture de {}", file.display()))?;
        println!(
            "{} : {} boost(s), {} ligne(s) illisible(s)",
            file.display(),
            page.boosts.len(),
            page.malformed
        );
        pages.push(page);
    }
    let config = Config::load(&paths.config_file)?;
    let conn = store::open_read_only(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init` puis `edge fd import`")?;
    println!("Choix de la grille sur 2019/20–2020/21 (quelques secondes)…\n");
    let rho = choose_grid(&conn, &config)?;
    println!("Rapprochement et tarification…\n");
    let result = study::run(
        &conn,
        &Aliases::bundled()?,
        &pages,
        config.margin_method,
        rho,
    )?;

    print_funnels(&result.funnels);
    print_quality(&result.rows);
    let operators: Vec<Operator> = result.funnels.iter().map(|(o, _)| *o).collect();
    for operator in operators {
        let rows: Vec<&StudyRow> = result
            .rows
            .iter()
            .filter(|r| r.operator == operator)
            .collect();
        println!("\n══ {} ══", operator.name());
        print_groups("Ensemble", &[("tous".to_string(), rows.clone())]);
        print_groups("Par famille de pari", &by(&rows, |r| r.category.clone()));
        print_groups(
            "Par cote boostée",
            &by(&rows, |r| odds_band(r.boosted.get())),
        );
        print_groups("Par mise maximale", &by(&rows, stake_label));
        print_strategies(&rows);
        print_unparsed(operator, &result.unparsed);
    }
    println!(
        "\nEspérance : gain moyen d'un euro misé à la cote boostée contre Pinnacle sans marge à la clôture (moyenne du \
         marché à défaut). ROI : gain réalisé. Marge d'origine : espérance de la cote avant boost. z : victoires \
         observées moins attendues, en écarts-types (au-delà de ±2, la tarification de la famille est suspecte)."
    );
    Ok(())
}

/// Compare les variantes de la grille sur la période de choix et garde celle de plus faible log loss moyenne sur les
/// paris de référence ; affiche ensuite le contrôle de la grille retenue sur la période de l'étude.
fn choose_grid(conn: &Connection, config: &Config) -> Result<Rho> {
    let date = |(y, m, d): (i32, u32, u32)| {
        NaiveDate::from_ymd_opt(y, m, d).context("date constante invalide")
    };
    let (from, to) = (date(SELECTION_FROM)?, date(SELECTION_TO)?);
    let mut best: Option<(Rho, f64)> = None;
    println!(
        "Grille calée sur la clôture de Pinnacle : log loss moyenne des paris de référence, 2019/20–2020/21"
    );
    for rho in RHO_VARIANTS {
        let check = study::check_grid(conn, config.margin_method, rho, from, to)?;
        let loss = check.mean_log_loss();
        println!(
            "  {:<12} {} match(s), {} écarté(s), log loss {}",
            rho_label(rho),
            check.games,
            check.rejected,
            format::decimal(loss, 5)
        );
        if best.is_none_or(|(_, b)| loss < b) {
            best = Some((rho, loss));
        }
    }
    let (rho, _) = best.context("aucune variante de grille évaluée")?;
    println!(
        "  retenue : {} ({} configurations essayées)",
        rho_label(rho),
        RHO_VARIANTS.len()
    );

    let check = study::check_grid(
        conn,
        config.margin_method,
        rho,
        date((2021, 8, 1))?,
        date((2026, 7, 31))?,
    )?;
    println!(
        "\nContrôle de la grille retenue sur 2021/22–2025/26 ({} matchs, {} écartés)",
        check.games, check.rejected
    );
    println!(
        "  {:<58} {:>8} {:>8} {:>6}",
        "pari de référence", "annoncé", "observé", "z"
    );
    for c in &check.checks {
        println!(
            "  {:<58} {:>8} {:>8} {:>6}",
            c.wording,
            percent1(c.predicted),
            percent1(c.observed),
            format::signed(c.z, 1)
        );
    }
    println!();
    Ok(rho)
}

fn rho_label(rho: Rho) -> String {
    match rho {
        Rho::Fitted => "ρ ajusté".to_string(),
        Rho::Fixed(value) => format!("ρ = {}", format::decimal(value, 2)),
    }
}

fn percent1(share: f64) -> String {
    format!("{} %", format::decimal(100.0 * share, 1))
}

/// Pages HTML du dossier WePari, triées par nom.
fn html_files(paths: &Paths) -> Result<Vec<std::path::PathBuf>> {
    let dir = &paths.wepari_dir;
    let entries = std::fs::read_dir(dir).with_context(|| {
        format!(
            "dossier {} absent : télécharge https://wepari.fr/indexh.html et https://wepari.fr/indexpsel.html dedans",
            dir.display()
        )
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("lecture de {}", dir.display()))?
            .path();
        if path.extension().is_some_and(|e| e == "html") {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        bail!("aucune page .html dans {}", dir.display());
    }
    Ok(files)
}

fn print_funnels(funnels: &[(Operator, study::Funnel)]) {
    println!(
        "Entonnoir : lus → match de nos ligues → intitulé sur le score final → tarifés (clôture / avant-match)"
    );
    for (operator, f) in funnels {
        println!(
            "  {:<14} {:>6} lus ({} illisibles) → {:>5} → {:>5} → {:>5} / {:>5}",
            operator.name(),
            f.read,
            f.malformed,
            f.matched,
            f.parsed,
            f.priced_close,
            f.priced_pre
        );
    }
}

/// Contrôle de qualité : notre issue (score en base + traduction de l'intitulé) contre celle publiée par WePari.
fn print_quality(rows: &[StudyRow]) {
    let checked: Vec<bool> = rows.iter().filter_map(|r| r.agrees_with_wepari).collect();
    let agree = checked.iter().filter(|a| **a).count();
    println!(
        "Contrôle : issue identique à celle de WePari pour {agree} boost(s) sur {} ({} %)",
        checked.len(),
        format::decimal(100.0 * agree as f64 / checked.len().max(1) as f64, 1)
    );
}

/// Regroupe les lignes selon une clé.
fn by<'a>(
    rows: &[&'a StudyRow],
    key: impl Fn(&StudyRow) -> String,
) -> Vec<(String, Vec<&'a StudyRow>)> {
    let mut groups: BTreeMap<String, Vec<&StudyRow>> = BTreeMap::new();
    for row in rows {
        // `*row` : `rows` contient des `&StudyRow`, l'itération donne des `&&StudyRow` ; on garde la référence simple.
        groups.entry(key(row)).or_default().push(*row);
    }
    let mut groups: Vec<(String, Vec<&StudyRow>)> = groups.into_iter().collect();
    groups.sort_by_key(|(_, rows)| std::cmp::Reverse(rows.len()));
    groups
}

fn stake_label(row: &StudyRow) -> String {
    match row.max_stake {
        Some(stake) => format!("mise max {} €", format::decimal(stake.get(), 0)),
        None => "mise max inconnue".to_string(),
    }
}

fn odds_band(odds: f64) -> String {
    let bands = [1.5, 2.0, 2.5, 3.0, 4.0, 6.0];
    match bands.iter().position(|b| odds < *b) {
        Some(0) => "< 1,50".to_string(),
        Some(i) => format!(
            "{} à {}",
            format::decimal(bands[i - 1], 2),
            format::decimal(bands[i], 2)
        ),
        None => "≥ 6,00".to_string(),
    }
}

fn print_groups(title: &str, groups: &[(String, Vec<&StudyRow>)]) {
    println!("\n{title} (clôture)");
    println!(
        "  {:<40} {:>6} {:>22} {:>7} {:>22} {:>8} {:>6}",
        "", "n", "espérance [IC 95 %]", "> 0", "ROI [IC 95 %]", "marge", "z"
    );
    for (label, rows) in groups {
        let Some(s) = study::summarize(rows, Moment::Close) else {
            continue;
        };
        if s.count < MIN_GROUP {
            continue;
        }
        println!(
            "  {:<40} {:>6} {:>22} {:>7} {:>22} {:>8} {:>6}",
            truncate(label, 40),
            s.count,
            interval(s.edge),
            percent(s.positive_share),
            interval(s.roi),
            signed_percent(s.base_edge),
            format::signed(s.calibration_z(rows, Moment::Close), 1)
        );
    }
}

/// Règles de sélection **sans fuite du futur** : on ne garde que les boosts dont l'avantage contre l'avant-match dépasse
/// un seuil, puis on mesure leur espérance contre la clôture et leur gain réel.
fn print_strategies(rows: &[&StudyRow]) {
    println!("\nSélection sur l'avantage d'avant-match (mesure à la clôture)");
    println!(
        "  {:<12} {:>6} {:>9} {:>22} {:>22} {:>16}",
        "seuil", "n", "par mois", "espérance [IC 95 %]", "ROI [IC 95 %]", "€/mois mise max"
    );
    let both: Vec<&StudyRow> = rows
        .iter()
        .filter(|r| r.fair_pre.is_some() && r.fair_close.is_some())
        .copied()
        .collect();
    let months = span_in_months(&both);
    let mut chosen_at_zero = Vec::new();
    for threshold in THRESHOLDS {
        let chosen: Vec<&StudyRow> = both
            .iter()
            .filter(|r| r.edge(Moment::Pre).is_some_and(|e| e > threshold))
            .copied()
            .collect();
        let Some(s) = study::summarize(&chosen, Moment::Close) else {
            continue;
        };
        if threshold == 0.0 {
            // `clone` d'un `Vec<&StudyRow>` : on copie les références, pas les lignes.
            chosen_at_zero = chosen.clone();
        }
        println!(
            "  {:<12} {:>6} {:>9} {:>22} {:>22} {:>16}",
            format!("> {}", signed_percent(threshold)),
            s.count,
            format::decimal(s.count as f64 / months, 1),
            interval(s.edge),
            interval(s.roi),
            expected_money(&chosen, months)
        );
    }
    print_groups(
        "Boosts d'avantage d'avant-match positif, par famille",
        &by(&chosen_at_zero, |r| r.category.clone()),
    );
    let by_stake = by(&chosen_at_zero, stake_label);
    print_groups(
        "Boosts d'avantage d'avant-match positif, par mise maximale",
        &by_stake,
    );
    for (label, rows) in &by_stake {
        println!(
            "  {label} : {} par mois, {} par mois à la mise maximale",
            format::decimal(rows.len() as f64 / months, 1),
            expected_money(rows, months)
        );
    }
}

/// Tournures non reconnues les plus fréquentes : ce que la traduction des intitulés laisse de côté.
fn print_unparsed(operator: Operator, unparsed: &[(Operator, String)]) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, wording) in unparsed.iter().filter(|(o, _)| *o == operator) {
        *counts.entry(wording.as_str()).or_default() += 1;
    }
    let mut counts: Vec<(&str, usize)> = counts.into_iter().collect();
    counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!(
        "\nIntitulés non évaluables les plus fréquents (sur {})",
        counts.iter().map(|(_, n)| n).sum::<usize>()
    );
    for (wording, n) in counts.iter().take(12) {
        println!("  {n:>5}  {}", truncate(wording, 100));
    }
}

/// Gain attendu par mois en misant le maximum autorisé sur chaque boost retenu (mise maximale publiée seulement).
fn expected_money(rows: &[&StudyRow], months: f64) -> String {
    let with_stake: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|r| Some((r.max_stake?.get(), r.edge(Moment::Close)?)))
        .collect();
    if with_stake.is_empty() {
        return "—".to_string();
    }
    let total: f64 = with_stake.iter().map(|(stake, edge)| stake * edge).sum();
    // Arrondi à l'euro ; `+ 0.0` change un −0 (somme négative infime arrondie) en +0, sinon affiché « -0 € ».
    let per_month = (total / months).round() + 0.0;
    format!("{} €", format::signed(per_month, 0))
}

/// Durée couverte par les lignes, en mois (au moins un).
fn span_in_months(rows: &[&StudyRow]) -> f64 {
    let first = rows.iter().map(|r| r.date).min();
    let last = rows.iter().map(|r| r.date).max();
    match first.zip(last) {
        Some((first, last)) => ((last - first).num_days() as f64 / 30.44).max(1.0),
        None => 1.0,
    }
}

fn interval(ci: MeanCi) -> String {
    format!(
        "{} [{} ; {}]",
        signed_percent(ci.mean),
        format::signed(100.0 * ci.low, 1),
        format::signed(100.0 * ci.high, 1)
    )
}

fn percent(share: f64) -> String {
    format!("{} %", format::decimal(100.0 * share, 0))
}

fn signed_percent(value: f64) -> String {
    format!("{} %", format::signed(100.0 * value, 1))
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let kept: String = text.chars().take(width - 1).collect();
        format!("{kept}…")
    }
}
