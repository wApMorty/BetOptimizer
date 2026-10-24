//! Onglet « Loto Foot » (`/lotofoot`) : budget du mois et bouton de séance, tickets à jouer, tickets en cours, dernière
//! séance (ce qu'elle a évalué ou écarté), bilans, vérification.
//!
//! Tout vient de [`crate::report::lotofoot_page::page`] ; ce module ajoute la présentation en bulletin : une ligne par
//! rencontre, une colonne par grille à remplir (simple ou multiple, `docs/sources.md` §39), les cases à cocher dans
//! chaque colonne ; puis le détail des tickets de la grille (espérance, règlement). Le bouton « Optimiser maintenant »
//! enregistre le budget mensuel saisi s'il a changé, puis lance la séance comme `edge lotofoot recommend`
//! ([`crate::lotofoot::live`]), au seuil que l'abaque tire de ce budget (§49, §51). Un ticket joué dont la grille a tous
//! ses résultats n'est plus affiché : il reste dans le bilan.
//!
//! Pendant les matchs, l'affichage relève les scores chez ESPN ([`crate::lotofoot::scores`]) : score à côté de chaque
//! rencontre, case juste ou fausse, couleur de chaque grille et de chaque ticket, et la page se recharge toutes les
//! minutes tant qu'un match se joue.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::Outcome;
use crate::error::Result;
use crate::lotofoot::abaque;
use crate::lotofoot::bulletin::session_lines;
use crate::lotofoot::espn::EspnClient;
use crate::lotofoot::live::{self, Live, Notified};
use crate::lotofoot::scores::{self, MatchNow, Standing};
use crate::lotofoot::session::{self, Request};
use crate::lotofoot::{Ticket, sign_char};
use crate::report::lotofoot_page::{
    self, GridTotal, Group, LineView, LotofootPage, RankView, TicketView,
};
use crate::store;
use crate::store::lotofoot as lotofoot_store;
use crate::web::App;
use crate::web::form::Form;
use crate::web::pages::entry::Reply;
use crate::web::view::{Flash, Layout, render};

const TEMPLATE: &str = include_str!("../templates/lotofoot.html");

/// Une grille à remplir, en tête et en pied de sa colonne.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Column {
    price: String,
    /// Numéros des tickets contenus (« n° 12, 13 »).
    tickets: String,
    played: bool,
    /// Couleur du suivi en direct (`t-wait`, `t-pos`, `t-warn`, `t-neg`), celle du meilleur ticket de la grille.
    tone: &'static str,
    /// État en clair du meilleur ticket (« une erreur (en cours) »).
    state: String,
    /// Tickets encore en lice pour un rang payé (« 3 / 4 en vie »), vide pour un ticket seul.
    alive: String,
}

/// Un rang en pied de bulletin (« Sans faute ») : pour chaque grille à remplir, sa chance et son gain moyen.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct RankRow {
    label: &'static str,
    /// `None` si les entrées du modèle manquent pour cette grille.
    cells: Vec<Option<RankView>>,
}

/// Une rencontre et les cases à cocher dans chaque grille à remplir (une case par signe : deux pour un double).
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Row {
    fixture: String,
    cells: Vec<Vec<String>>,
    /// Score (« 2 – 1 »), vide avant le coup d'envoi ou sans relevé.
    score: String,
    /// État du match (« 57' », « terminé », « résultat officiel »).
    state: String,
    /// Signe du moment (`1`, `N`, `2`), vide s'il n'y en a pas : les cases cochées se colorent juste ou faux.
    result: String,
    /// Le signe ne bougera plus.
    settled: bool,
    /// Le match se joue.
    live: bool,
}

/// Un ticket de la grille, pour le détail.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Detail {
    id: i64,
    ticket: String,
    played: bool,
    status: String,
    /// Tonalité du règlement (`t-pos` pour un rang payé, `t-neg` pour perdu).
    tone: &'static str,
    ev: String,
    ev_low: String,
    /// Chance d'être payé, tous rangs confondus.
    chance: String,
    /// Chance et gain moyen de chaque rang.
    ranks: Vec<RankView>,
    /// Couleur du suivi en direct.
    live_tone: &'static str,
    /// État en clair du suivi en direct.
    live_state: String,
}

/// Une grille du Loto Foot présentée en bulletin.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Bulletin {
    grid: String,
    /// `lf7-133`, pour le bouton « J'ai joué cette grille ».
    key: String,
    closes: String,
    /// Séance qui a choisi les tickets.
    session: String,
    /// Tickets pas encore déclarés joués.
    to_declare: usize,
    columns: Vec<Column>,
    rows: Vec<Row>,
    ranks: Vec<RankRow>,
    /// Toutes les colonnes ensemble.
    total: GridTotal,
    details: Vec<Detail>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Page {
    #[serde(flatten)]
    layout: Layout,
    #[serde(flatten)]
    lotofoot: LotofootPage,
    /// Grilles à jouer, en bulletins.
    play_bulletins: Vec<Bulletin>,
    /// Grilles en cours (tickets joués, résultats attendus), en bulletins.
    live_bulletins: Vec<Bulletin>,
    /// Bornes du budget mensuel (abaque).
    budget_min: u32,
    /// Voir `budget_min`.
    budget_max: u32,
    /// Un match des grilles se joue : la page se recharge toutes les minutes.
    auto_refresh: bool,
    /// Ce qui empêche de suivre certaines rencontres (noms inconnus, ESPN injoignable).
    live_issues: Vec<String>,
}

/// Suivi en direct : état des rencontres de chaque grille de la semaine, et ce qu'il faut signaler.
struct Follow {
    /// Grille (« LF8 n°146 ») → état de ses rencontres, dans l'ordre.
    grids: Vec<(String, Vec<MatchNow>)>,
    /// Rencontres commencées et pas finies.
    watching: usize,
    /// Problèmes à afficher.
    issues: Vec<String>,
}

/// Relève les scores (si l'application y est autorisée) puis lit l'état des rencontres des grilles suivies (à jouer et
/// en cours).
fn follow(app: &App, now: DateTime<Utc>) -> Result<Follow> {
    // Lecture et écriture : les scores relevés sont gardés en base (dernier état seulement, migration 17).
    let conn = store::open_migrated(&app.paths.db_file)?;
    let followed = lotofoot_page::followed(&conn, now)?;
    let mut recos = followed.to_play;
    recos.extend(followed.in_play);
    let (tracked, mut issues) = scores::tracked(&conn, &app.aliases, &recos)?;
    let mut watching = 0;
    if app.live_scores {
        let client = EspnClient::new();
        let report = scores::refresh(&conn, &app.aliases, &tracked, now, &mut |path, params| {
            client.get(path, params)
        })?;
        watching = report.watching;
    }
    issues.extend(scores::stored_issues(&conn, &tracked)?);
    let mut grids: Vec<(String, Vec<MatchNow>)> = Vec::new();
    for reco in &recos {
        let name = format!("{} n°{}", reco.game, reco.number);
        if !grids.iter().any(|(g, _)| *g == name) {
            grids.push((name, scores::grid_now(&conn, reco.game, reco.number)?));
        }
    }
    Ok(Follow {
        grids,
        watching,
        issues,
    })
}

/// La page.
pub fn page(app: &App, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    // Le suivi en direct ne doit jamais empêcher la page de s'afficher : une erreur devient un message.
    let follow = follow(app, now).unwrap_or_else(|error| Follow {
        grids: Vec::new(),
        watching: 0,
        issues: vec![format!("scores en direct indisponibles : {error}")],
    });
    let conn = store::open_read_only(&app.paths.db_file)?;
    let budget = session::monthly_budget(&conn, &app.config, now)?;
    let lotofoot = lotofoot_page::page(&conn, now, budget)?;
    let play_bulletins = bulletins(&lotofoot.to_play, &follow);
    let live_bulletins = bulletins(&lotofoot.in_play, &follow);
    render(
        "lotofoot.html",
        TEMPLATE,
        &Page {
            layout: Layout::new("lotofoot", now, flash),
            lotofoot,
            play_bulletins,
            live_bulletins,
            budget_min: abaque::MIN_BUDGET,
            budget_max: abaque::MAX_BUDGET,
            auto_refresh: follow.watching > 0,
            live_issues: follow.issues,
        },
    )
}

/// Les grilles d'un groupe en bulletins.
fn bulletins(group: &Group, follow: &Follow) -> Vec<Bulletin> {
    group
        .grids
        .iter()
        .map(|grid| {
            let lines: Vec<&LineView> =
                group.lines.iter().filter(|l| l.grid == grid.grid).collect();
            let matches: &[MatchNow] = follow
                .grids
                .iter()
                .find(|(g, _)| *g == grid.grid)
                .map_or(&[], |(_, m)| m.as_slice());
            let tickets: Vec<&TicketView> = group
                .tickets
                .iter()
                .filter(|t| t.grid == grid.grid)
                .collect();
            Bulletin {
                grid: grid.grid.clone(),
                key: grid.key.clone(),
                closes: grid.closes.clone(),
                session: grid.session.clone(),
                to_declare: tickets.iter().filter(|t| !t.played).count(),
                columns: lines
                    .iter()
                    .map(|l| column(l, &group.tickets, matches))
                    .collect(),
                rows: grid
                    .matches
                    .iter()
                    .enumerate()
                    .map(|(index, fixture)| row(fixture, index, &lines, matches.get(index)))
                    .collect(),
                ranks: rank_rows(&lines),
                total: grid.total.clone(),
                details: tickets.iter().map(|t| detail(t, matches)).collect(),
            }
        })
        .collect()
}

/// Bouton « Optimiser maintenant » : budget mensuel saisi (champ `budget`, euros entiers de l'abaque), enregistré s'il a
/// changé ; message Discord si la case `discord` est cochée. Le bouton « Enregistrer le budget » (`action=budget`)
/// enregistre sans lancer de séance. Le résultat revient en message sur l'onglet.
pub fn optimize(app: &App, form: &Form, now: DateTime<Utc>) -> Reply {
    let flash = match budget(form.get("budget")) {
        Err(message) => Flash::error(message),
        Ok(monthly) => match save_budget(app, monthly, now) {
            Err(error) => Flash::error(format!("budget non enregistré : {error}")),
            Ok(saved) if form.get("action") == "budget" => Flash::ok(if saved {
                format!("Budget enregistré : {monthly} € par mois.")
            } else {
                format!("Budget inchangé : {monthly} € par mois.")
            }),
            Ok(_) => match run_now(app, monthly, form.has("discord")) {
                Ok(message) => Flash::ok(message),
                Err(error) => Flash::error(format!("séance non faite : {error}")),
            },
        },
    };
    Reply::Done {
        to: "/lotofoot",
        flash,
    }
}

/// Budget saisi : un nombre entier d'euros dans l'abaque.
fn budget(text: &str) -> std::result::Result<u32, String> {
    match text.trim().parse::<u32>() {
        Ok(euros) if abaque::threshold(euros).is_ok() => Ok(euros),
        _ => Err(format!(
            "budget attendu : un nombre entier d'euros par mois, de {} à {}",
            abaque::MIN_BUDGET,
            abaque::MAX_BUDGET
        )),
    }
}

/// Enregistre le budget s'il diffère de celui en vigueur ; `true` s'il a été écrit.
fn save_budget(app: &App, monthly: u32, now: DateTime<Utc>) -> Result<bool> {
    let conn = store::open_migrated(&app.paths.db_file)?;
    if session::monthly_budget(&conn, &app.config, now)? == monthly
        && lotofoot_store::budget_at(&conn, now)?.is_some()
    {
        return Ok(false);
    }
    lotofoot_store::record_budget(&conn, now, monthly)?;
    Ok(true)
}

fn run_now(app: &App, monthly_budget: u32, notify: bool) -> Result<String> {
    let mut conn = store::open_migrated(&app.paths.db_file)?;
    // Le bouton recalcule aussi les grilles déjà recommandées (pas encore jouées) : la répartition a pu bouger.
    let request = Request::from_budget(monthly_budget, true, false)?;
    let live = live::recommend(&mut conn, &app.config, &app.aliases, request, notify)?;
    Ok(summary(&live))
}

/// Message de fin de séance : tickets et grilles à remplir, grilles écartées, relevé et Discord.
fn summary(live: &Live) -> String {
    let report = &live.report;
    let mut parts = Vec::new();
    match report.session_id {
        Some(id) => {
            let keyed: Vec<_> = report.chosen.iter().map(|c| (c.grid, &c.ticket)).collect();
            parts.push(format!(
                "Séance n°{id} : {} ticket(s), soit {} €, en {} grille(s) à remplir.",
                report.chosen.len(),
                report.chosen.len(),
                session_lines(&keyed).len()
            ));
        }
        None => parts.push("Aucun nouveau ticket : rien n'est écrit au journal.".to_string()),
    }
    if !report.skipped.is_empty() {
        let skipped: Vec<String> = report
            .skipped
            .iter()
            .map(|s| format!("{} ({})", s.grid, s.reason))
            .collect();
        parts.push(format!("Écartée(s) : {}.", skipped.join(" ; ")));
    }
    if !report.months.is_empty() {
        parts.push(format!(
            "Budget : {} ; {}.",
            lotofoot_page::months_text(&report.months),
            lotofoot_page::threshold_text(report)
        ));
    }
    if !live.fetched.failures.is_empty() {
        parts.push(format!(
            "Relevé de prono-match incomplet : {} page(s) non lue(s).",
            live.fetched.failures.len()
        ));
    }
    match &live.notified {
        Notified::NotAsked => {}
        Notified::Sent => parts.push("Envoyée sur Discord.".to_string()),
        Notified::Failed(error) => parts.push(format!("Message Discord non envoyé : {error}.")),
    }
    parts.join(" ")
}

/// Lignes « Sans faute » et « Une erreur » du pied de bulletin, une case par grille à remplir.
fn rank_rows(lines: &[&LineView]) -> Vec<RankRow> {
    // Toutes les grilles d'une même formule ont le même nombre de rangs : la plus longue liste donne les libellés.
    let Some(longest) = lines.iter().map(|l| &l.ranks).max_by_key(|r| r.len()) else {
        return Vec::new();
    };
    longest
        .iter()
        .enumerate()
        .map(|(k, rank)| RankRow {
            label: rank.label,
            cells: lines.iter().map(|l| l.ranks.get(k).cloned()).collect(),
        })
        .collect()
}

/// Une rencontre du bulletin : cases cochées de chaque grille à remplir, score et état du moment.
fn row(fixture: &str, index: usize, lines: &[&LineView], now: Option<&MatchNow>) -> Row {
    Row {
        fixture: fixture.to_string(),
        cells: lines
            .iter()
            .map(|l| l.signs.get(index).cloned().unwrap_or_default())
            .collect(),
        score: now.map_or(String::new(), |m| m.score.clone()),
        state: now.map_or(String::new(), |m| m.label.clone()),
        result: now
            .and_then(|m| m.outcome)
            .map_or(String::new(), |o| sign_char(o).to_string()),
        settled: now.is_some_and(|m| m.settled),
        live: now.is_some_and(|m| m.live),
    }
}

fn column(line: &LineView, tickets: &[TicketView], matches: &[MatchNow]) -> Column {
    let ids: Vec<String> = line.tickets.iter().map(i64::to_string).collect();
    // Les cases de la grille, rencontre par rencontre : son meilleur ticket fait sa couleur.
    let covers: Vec<Vec<Outcome>> = line
        .signs
        .iter()
        // Les cases d'une rencontre (`["1", "N"]`) se lisent comme un petit ticket `1N`.
        .map(|signs| Ticket::parse(&signs.concat()).map_or(Vec::new(), |t| t.signs().to_vec()))
        .collect();
    let best = Standing::of(&covers, matches);
    let standings: Vec<Standing> = tickets
        .iter()
        .filter(|t| line.tickets.contains(&t.id))
        .filter_map(|t| Ticket::parse(&t.ticket))
        .map(|t| Standing::of(&Standing::covers(&t), matches))
        .collect();
    let alive = standings.iter().filter(|s| s.alive()).count();
    Column {
        price: line.price.clone(),
        tickets: format!("n° {}", ids.join(", ")),
        played: line.played,
        tone: best.tone(),
        state: best.label(),
        alive: if standings.len() > 1 && best.known > 0 {
            format!("{alive} / {} en vie", standings.len())
        } else {
            String::new()
        },
    }
}

fn detail(ticket: &TicketView, matches: &[MatchNow]) -> Detail {
    let live = Ticket::parse(&ticket.ticket).map(|t| Standing::of(&Standing::covers(&t), matches));
    let tone = if ticket.status.starts_with("rang") {
        "t-pos"
    } else if ticket.status.starts_with("perdu") {
        "t-neg"
    } else {
        ""
    };
    Detail {
        id: ticket.id,
        ticket: ticket.ticket.clone(),
        played: ticket.played,
        status: ticket.status.clone(),
        tone,
        ev: ticket.ev.clone(),
        ev_low: ticket.ev_low.clone(),
        chance: ticket.chance.clone(),
        ranks: ticket.ranks.clone(),
        live_tone: live.map_or("t-wait", |s| s.tone()),
        live_state: live.map_or(String::new(), |s| s.label()),
    }
}

#[cfg(test)]
mod tests {
    use crate::web::testing::{empty, sample};

    #[test]
    fn the_session_is_shown_as_a_bulletin_with_one_column_per_grid_to_fill() {
        let html = sample().html("/lotofoot");
        assert!(html.contains("LF7 n°133"), "{html}");
        // Deux tickets, `1N21N21` et `2N21N21`, qui ne diffèrent que sur la première rencontre : une seule grille à
        // remplir, un double 12 puis six simples, soit huit cases cochées pour sept rencontres.
        assert_eq!(html.matches("class=\"sign s-").count(), 8, "{html}");
        assert!(html.contains("2 €"), "{html}");
        // Le détail garde chaque ticket.
        assert!(
            html.contains("1N21N21") && html.contains("2N21N21"),
            "{html}"
        );
        assert!(html.contains("J'ai joué cette grille"));
        assert!(html.contains("name=\"grille\" value=\"lf7-133\""), "{html}");
        // La séance est présentée à part des tickets.
        assert!(html.contains("Dernière séance"), "{html}");
        let html = empty().html("/lotofoot");
        assert!(html.contains("Rien à jouer"), "{html}");
        // Le bouton est là même sans séance, avec le budget mensuel de la configuration et son seuil.
        assert!(html.contains("action=\"/lotofoot/optimiser\""), "{html}");
        assert!(html.contains("value=\"60\""), "{html}");
        assert!(html.contains("τ = 1,8"), "{html}");
    }

    #[test]
    fn a_match_in_progress_colours_the_signs_and_the_grid() {
        use crate::lotofoot::Game;
        use crate::lotofoot::espn::Phase;
        use crate::store::lotofoot_live::{LiveScore, upsert_score};

        let sample = sample();
        let conn = crate::store::open_migrated(&sample.app.paths.db_file).unwrap();
        // Première rencontre en cours, 1 – 0 : le double 12 de la grille a un signe juste (1) et un faux (2).
        upsert_score(
            &conn,
            Game::Lf7,
            133,
            1,
            &LiveScore {
                league: "fra.2".to_string(),
                event_id: "401".to_string(),
                home_goals: 1,
                away_goals: 0,
                phase: Phase::Playing,
                label: "57'".to_string(),
                fetched_at: sample.now,
            },
        )
        .unwrap();
        let html = sample.html("/lotofoot");
        assert!(html.contains("1 – 0"), "{html}");
        assert!(
            html.contains("class=\"sign s-1 hit provisional\""),
            "{html}"
        );
        assert!(
            html.contains("class=\"sign s-2 miss provisional\""),
            "{html}"
        );
        // Le meilleur ticket est sans faute ; l'autre a une erreur provisoire mais reste en lice pour le 2e rang.
        assert!(
            html.contains("<span class=\"chip t-pos\">sans faute</span>"),
            "{html}"
        );
        assert!(html.contains("2 / 2 en vie"), "{html}");
        assert!(html.contains("une erreur (en cours)"), "{html}");
        // Les tests n'appellent pas ESPN : rien ne se joue selon l'application, la page ne se recharge pas seule.
        assert!(!html.contains("http-equiv=\"refresh\""));
    }

    #[test]
    fn the_budget_must_be_whole_euros_within_the_abaque() {
        assert_eq!(super::budget("60"), Ok(60));
        assert_eq!(super::budget(" 400 "), Ok(400));
        for wrong in ["0", "19", "401", "60,5", "", "soixante"] {
            assert!(super::budget(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn a_played_grid_with_all_its_results_is_no_longer_shown() {
        use crate::lotofoot::Game;
        use crate::lotofoot::pronomatch::GridResult;
        use crate::store::lotofoot::{Page as FetchPage, record_fetch, record_results};

        let sample = sample();
        let mut conn = crate::store::open_migrated(&sample.app.paths.db_file).unwrap();
        // Les deux tickets du LF7 n°133 sont joués : la grille passe dans « En cours ».
        let stake = crate::domain::Euros::new(1.0).unwrap();
        crate::lotofoot::played::declare_grid(&conn, Game::Lf7, 133, sample.now, stake).unwrap();
        let html = sample.html("/lotofoot");
        assert!(html.contains("En cours"), "{html}");
        assert!(html.contains("LF7 n°133"), "{html}");
        // Tous les résultats connus : la grille n'est plus suivie (elle reste au bilan).
        let fetch_id = record_fetch(&conn, sample.now, FetchPage::Results, 200, None).unwrap();
        let result = GridResult {
            game: Some(Game::Lf7),
            number: 133,
            date: None,
            matches: (1..=7)
                .map(|i| {
                    (
                        format!("H{i}"),
                        format!("A{i}"),
                        crate::domain::Outcome::Home,
                    )
                })
                .collect(),
            ranks: vec![],
        };
        record_results(&mut conn, fetch_id, &[result]).unwrap();
        let html = sample.html("/lotofoot");
        assert!(!html.contains("LF7 n°133"), "{html}");
    }
}
