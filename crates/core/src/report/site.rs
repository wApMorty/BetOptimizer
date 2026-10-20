//! Pages du site de rapports (`edge serve`) : routage et contenu, sans réseau.
//!
//! Le serveur HTTP de la CLI passe le chemin demandé à [`respond`] et renvoie la [`Page`] obtenue : tout ce qui s'affiche se
//! teste ici, sans ouvrir de port. Lecture seule : la base est ouverte sans droit d'écriture, l'archive n'est que lue.

use std::path::PathBuf;

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;

use super::daily::{self, DailyReport};
use super::recommend_report::{Row, league_name, row};
use super::render::{
    render_site_message, render_site_opportunities, render_site_paper, render_site_reports,
};
use super::svg::cumulative_units_chart;
use crate::betting::paper::{self, PaperBet, PaperTrading, Status};
use crate::error::{Error, Result};
use crate::eval::bootstrap::MeanCi;
use crate::format::{age, decimal, kickoff_label, long_date, outcome_label, signed, units, until};
use crate::ingest::aliases::Aliases;
use crate::market::margin::MarginMethod;
use crate::store::{self, journal};

/// Type de contenu de toutes les pages.
pub const HTML: &str = "text/html; charset=utf-8";

/// Sous ce nombre de clôtures, la CLV est signalée comme indicative (même seuil que `edge clv`).
const FEW_CLOSINGS: usize = 30;

/// Ce dont le site a besoin pour répondre.
#[derive(Debug, Clone)]
pub struct Site {
    /// Base SQLite, ouverte en lecture seule à chaque requête.
    pub db_file: PathBuf,
    /// Archive des rapports quotidiens.
    pub daily_dir: PathBuf,
    /// Table d'alias (résultats des matchs).
    pub aliases: Aliases,
    /// Méthode de retrait de la marge (clôture de Pinnacle).
    pub method: MarginMethod,
}

/// Une réponse : statut HTTP et corps HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Statut HTTP (200, 404 ou 500).
    pub status: u16,
    /// Corps.
    pub body: Vec<u8>,
}

/// Lien vers un rapport archivé.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportLink {
    /// Nom du fichier.
    pub name: String,
    /// Date et heure de l'exécution, heure de Paris.
    pub label: String,
}

/// Page « rapports précédents ».
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportsPage {
    /// Barre de navigation (toujours affichée sur le site).
    pub nav: bool,
    /// Lien actif de la barre de navigation.
    pub active: &'static str,
    /// Rapports, du plus récent au plus ancien.
    pub reports: Vec<ReportLink>,
}

/// Une opportunité active.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpportunityRow {
    /// La recommandation, mise en forme comme dans le rapport quotidien.
    pub rec: Row,
    /// Délai avant le coup d'envoi (`dans 3 h 20`).
    pub until: String,
    /// Âge de la recommandation (`il y a 2 h 05`).
    pub seen: String,
}

/// Page « opportunités actives ».
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpportunitiesPage {
    /// Barre de navigation.
    pub nav: bool,
    /// Lien actif.
    pub active: &'static str,
    /// Instant de calcul, heure de Paris.
    pub generated_at: String,
    /// Opportunités, du coup d'envoi le plus proche au plus lointain.
    pub rows: Vec<OpportunityRow>,
}

/// Une ligne du tableau du paper trading.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PaperRow {
    /// Numéro de la recommandation.
    pub id: i64,
    /// Coup d'envoi, heure de Paris.
    pub kickoff: String,
    /// `Domicile – Extérieur`.
    pub fixture: String,
    /// Compétition.
    pub league: String,
    /// Opérateur.
    pub bookmaker: String,
    /// Issue jouée.
    pub outcome: String,
    /// Cote.
    pub odds: String,
    /// Avantage annoncé.
    pub edge: String,
    /// Mise, en unités.
    pub stake: String,
    /// CLV, ou `—` sans clôture.
    pub clv: String,
    /// Résultat (`gagné +2,6 u`, `à venir`…).
    pub result: String,
    /// Classe CSS du résultat (`won`, `lost`, `open`).
    pub css: &'static str,
}

/// Page « paper trading ».
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PaperPage {
    /// Barre de navigation.
    pub nav: bool,
    /// Lien actif.
    pub active: &'static str,
    /// Instant de calcul, heure de Paris.
    pub generated_at: String,
    /// Paris suivis.
    pub n: usize,
    /// Paris réglés.
    pub settled: usize,
    /// Paris dont le match a commencé, sans résultat connu.
    pub pending: usize,
    /// Paris à venir.
    pub upcoming: usize,
    /// Gain net réglé, en unités.
    pub profit: String,
    /// Mises réglées, en unités.
    pub staked: String,
    /// Rendement des mises.
    pub roi: String,
    /// CLV moyen.
    pub clv: String,
    /// Intervalle à 95 % de la CLV.
    pub clv_ci: String,
    /// Paris dont la clôture est connue.
    pub with_closing: usize,
    /// Rendement à mise fixe.
    pub flat_roi: String,
    /// Intervalle à 95 % du rendement à mise fixe.
    pub flat_roi_ci: String,
    /// Baisse maximale du gain cumulé, en unités.
    pub drawdown: String,
    /// Moins de 30 clôtures : CLV indicative.
    pub few_closings: bool,
    /// Graphique SVG du gain cumulé (absent tant que rien n'est réglé).
    pub chart: Option<String>,
    /// Paris, du plus récent au plus ancien.
    pub rows: Vec<PaperRow>,
}

/// Page de message (aucun rapport, page introuvable, erreur).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MessagePage {
    /// Barre de navigation.
    pub nav: bool,
    /// Lien actif.
    pub active: &'static str,
    /// Titre.
    pub title: String,
    /// Texte.
    pub message: String,
}

/// Répond à une requête `GET` sur `path`. Une erreur devient une page 500 qui la décrit (aucun secret n'y figure :
/// les erreurs du cœur ne portent ni clé API ni URL de webhook).
pub fn respond(site: &Site, path: &str, now: DateTime<Utc>) -> Page {
    // La partie `?…` est ignorée : aucune page n'a de paramètre.
    let path = path.split_once('?').map_or(path, |(before, _)| before);
    let result = match path {
        "/" | "/index.html" => today(site),
        "/rapports" | "/rapports/" => reports(site),
        "/opportunites" => opportunities(site, now),
        "/paper" => paper_page(site, now),
        _ => match path.strip_prefix("/rapports/") {
            Some(name) => archived(site, name),
            None => not_found(),
        },
    };
    result.unwrap_or_else(|error| {
        tracing::error!("page {path} : {error}");
        let page = MessagePage {
            nav: true,
            active: "",
            title: "Erreur".into(),
            message: format!("La page n'a pas pu être calculée : {error}"),
        };
        Page {
            status: 500,
            // Si même la page d'erreur échoue, un texte brut suffit.
            body: render_site_message(&page)
                .unwrap_or_else(|_| page.message.clone())
                .into_bytes(),
        }
    })
}

fn ok(html: String) -> Result<Page> {
    Ok(Page {
        status: 200,
        body: html.into_bytes(),
    })
}

fn not_found() -> Result<Page> {
    let html = render_site_message(&MessagePage {
        nav: true,
        active: "",
        title: "Page introuvable".into(),
        message: "Cette adresse ne correspond à aucune page du site.".into(),
    })?;
    Ok(Page {
        status: 404,
        body: html.into_bytes(),
    })
}

/// Lit un rapport archivé tel qu'il a été écrit.
fn read_report(site: &Site, report: &DailyReport) -> Result<Page> {
    let path = site.daily_dir.join(&report.name);
    let body = std::fs::read(&path).map_err(Error::io(&path))?;
    Ok(Page { status: 200, body })
}

/// `/` : le dernier rapport archivé.
fn today(site: &Site) -> Result<Page> {
    match daily::list(&site.daily_dir)?.first() {
        Some(latest) => read_report(site, latest),
        None => ok(render_site_message(&MessagePage {
            nav: true,
            active: "today",
            title: "Rapport du jour".into(),
            message: "Aucun rapport archivé : le premier sera écrit par la prochaine exécution de « edge recommend »."
                .into(),
        })?),
    }
}

/// `/rapports/<nom>` : un rapport archivé. Le nom doit être **exactement** celui d'un fichier listé par l'archive : un
/// chemin comme `../edge.db` ne correspond à aucun et donne une page introuvable.
fn archived(site: &Site, name: &str) -> Result<Page> {
    match daily::list(&site.daily_dir)?
        .iter()
        .find(|report| report.name == name)
    {
        Some(report) => read_report(site, report),
        None => not_found(),
    }
}

/// `/rapports` : la liste des rapports archivés.
fn reports(site: &Site) -> Result<Page> {
    let reports = daily::list(&site.daily_dir)?
        .into_iter()
        .map(|report| ReportLink {
            label: run_label(report.at),
            name: report.name,
        })
        .collect();
    ok(render_site_reports(&ReportsPage {
        nav: true,
        active: "reports",
        reports,
    })?)
}

/// Date et heure d'une exécution, heure de Paris : `mardi 20 octobre 2026 à 14:10`.
fn run_label(at: DateTime<Utc>) -> String {
    let local = at.with_timezone(&Paris);
    format!(
        "{} {} à {:02}:{:02}",
        long_date(at),
        local.format("%Y"),
        local.hour(),
        local.minute()
    )
}

fn generated_at(now: DateTime<Utc>) -> String {
    now.with_timezone(&Paris)
        .format("%d/%m/%Y %H:%M")
        .to_string()
}

/// `/opportunites` : les recommandations qui seraient jouées et dont le match n'a pas commencé.
fn opportunities(site: &Site, now: DateTime<Utc>) -> Result<Page> {
    let conn = store::open_read_only(&site.db_file)?;
    let mut active: Vec<_> = journal::recommendations(&conn)?
        .into_iter()
        .filter(|stored| paper::is_played(stored) && stored.record.kickoff > now)
        .collect();
    active.sort_by_key(|stored| stored.record.kickoff);
    let rows = active
        .iter()
        .map(|stored| OpportunityRow {
            rec: row(stored),
            until: until(stored.record.kickoff, now),
            seen: age(stored.created_at, now),
        })
        .collect();
    ok(render_site_opportunities(&OpportunitiesPage {
        nav: true,
        active: "opportunities",
        generated_at: generated_at(now),
        rows,
    })?)
}

/// `/paper` : le bilan du paper trading.
fn paper_page(site: &Site, now: DateTime<Utc>) -> Result<Page> {
    let conn = store::open_read_only(&site.db_file)?;
    let paper = paper::paper_trading(&conn, &site.aliases, site.method, now)?;
    ok(render_site_paper(&paper_view(&paper, now)?)?)
}

/// Pourcentage signé : `0,0123` → `+1,2 %`.
fn signed_percent(value: f64) -> String {
    format!("{} %", signed(value * 100.0, 1))
}

/// Moyenne et intervalle d'un `MeanCi` : (`+1,2 %`, `[−0,8 ; +3,1] %`), ou des tirets.
fn ci_parts(ci: Option<MeanCi>) -> (String, String) {
    ci.map_or(("—".into(), "—".into()), |c| {
        (
            signed_percent(c.mean),
            format!(
                "[{} ; {}] %",
                signed(c.low * 100.0, 1),
                signed(c.high * 100.0, 1)
            ),
        )
    })
}

fn paper_view(paper: &PaperTrading, now: DateTime<Utc>) -> Result<PaperPage> {
    let (clv, clv_ci) = ci_parts(paper.clv);
    let (flat_roi, flat_roi_ci) = ci_parts(paper.flat_roi);
    let chart = if paper.curve.is_empty() {
        None
    } else {
        Some(cumulative_units_chart(
            &paper.curve,
            "Gain cumulé du paper trading",
        )?)
    };
    Ok(PaperPage {
        nav: true,
        active: "paper",
        generated_at: generated_at(now),
        n: paper.bets.len(),
        settled: paper.count(Status::Won) + paper.count(Status::Lost),
        pending: paper.count(Status::Pending),
        upcoming: paper.count(Status::Upcoming),
        profit: format!("{} u", signed(paper.profit_units, 1)),
        staked: units(paper.staked_units),
        roi: paper.roi.map_or("—".into(), signed_percent),
        clv,
        clv_ci,
        with_closing: paper.with_closing,
        flat_roi,
        flat_roi_ci,
        drawdown: units(paper.max_drawdown_units),
        few_closings: paper.with_closing < FEW_CLOSINGS,
        chart,
        // Du plus récent au plus ancien : la dernière recommandation en haut.
        rows: paper.bets.iter().rev().map(paper_row).collect(),
    })
}

fn paper_row(bet: &PaperBet) -> PaperRow {
    let stored = &bet.recommendation;
    let r = &stored.record;
    let (result, css) = match (bet.status, bet.profit_units) {
        (Status::Won, Some(profit)) => (format!("gagné {} u", signed(profit, 1)), "won"),
        (Status::Lost, Some(profit)) => (format!("perdu {} u", signed(profit, 1)), "lost"),
        (Status::Pending, _) => ("résultat attendu".to_string(), "open"),
        // `Upcoming`, et par prudence un gagné ou perdu sans gain calculé (impossible : `paper_trading` le calcule toujours).
        _ => ("à venir".to_string(), "open"),
    };
    PaperRow {
        id: stored.id,
        kickoff: kickoff_label(r.kickoff),
        fixture: format!("{} – {}", r.home_team, r.away_team),
        league: league_name(&r.sport_key),
        bookmaker: r.bookmaker.clone(),
        outcome: outcome_label(r.outcome).to_string(),
        odds: decimal(r.odds.get(), 2),
        edge: signed_percent(r.edge),
        stake: units(bet.stake_units),
        clv: bet.clv.map_or("—".into(), signed_percent),
        result,
        css,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Euros, Odds, Outcome, Probability};
    use crate::store::journal::{NewRecommendation, Tier, insert_recommendation};

    const NOW: &str = "2026-10-20T13:00:00Z";

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    /// Un site sur une base migrée vide, dans un dossier temporaire (gardé vivant par l'appelant).
    fn site() -> (tempfile::TempDir, Site) {
        let dir = tempfile::tempdir().unwrap();
        let db_file = dir.path().join("edge.db");
        store::open_migrated(&db_file).unwrap();
        let site = Site {
            db_file,
            daily_dir: dir.path().join("daily"),
            aliases: Aliases::parse("[SC0]\n\"Falkirk\" = []\n\"Kilmarnock\" = []\n").unwrap(),
            method: MarginMethod::Multiplicative,
        };
        (dir, site)
    }

    fn get(site: &Site, path: &str) -> (u16, String) {
        let page = respond(site, path, at(NOW));
        (page.status, String::from_utf8(page.body).unwrap())
    }

    fn recommendation(event_id: &str, kickoff: &str, stake: Option<f64>) -> NewRecommendation {
        NewRecommendation {
            rule: "price_vs_pinnacle".into(),
            tier: Tier::Recommended,
            event_id: event_id.into(),
            sport_key: "soccer_spl".into(),
            home_team: "Falkirk".into(),
            away_team: "Kilmarnock".into(),
            kickoff: at(kickoff),
            bookmaker: "betclic_fr".into(),
            outcome: Outcome::Home,
            odds: Odds::new(2.3).unwrap(),
            probability: Probability::new(0.4865).unwrap(),
            edge: 0.1189,
            min_edge: 0.01,
            minimal_odds: 2.08,
            stake: stake.map(|s| Euros::new(s).unwrap()),
            kelly_fraction: 0.5,
            bankroll: Euros::new(1000.0).unwrap(),
            margin_method: "power".into(),
            api_call_id: 1,
            code_version: "test".into(),
        }
    }

    /// Insère des recommandations ; la clé étrangère vers `api_call` est satisfaite par un appel factice.
    fn insert(site: &Site, recommendations: &[NewRecommendation]) {
        let conn = store::open(&site.db_file).unwrap();
        conn.execute_batch(
            "INSERT INTO api_call (id, called_at, endpoint, params, http_status)
             VALUES (1, '2026-10-20T12:00:00Z', 'odds', '', 200)",
        )
        .unwrap();
        for new in recommendations {
            insert_recommendation(&conn, new, at("2026-10-20T12:10:00Z"))
                .unwrap()
                .unwrap();
        }
    }

    #[test]
    fn the_home_page_is_the_latest_archived_report() {
        let (_dir, site) = site();
        assert!(get(&site, "/").1.contains("Aucun rapport archivé"));

        daily::save(&site.daily_dir, at("2026-10-19T12:10:00Z"), "<p>hier</p>").unwrap();
        daily::save(
            &site.daily_dir,
            at("2026-10-20T12:10:00Z"),
            "<p>aujourd'hui</p>",
        )
        .unwrap();

        assert_eq!(get(&site, "/"), (200, "<p>aujourd'hui</p>".to_string()));
        let (status, list) = get(&site, "/rapports");
        assert_eq!(status, 200);
        assert!(list.contains("/rapports/2026-10-19-121000Z.html"));
        assert!(
            list.contains("lundi 19 octobre 2026 à 14:10"),
            "heure de Paris"
        );
        assert_eq!(
            get(&site, "/rapports/2026-10-19-121000Z.html").1,
            "<p>hier</p>"
        );
    }

    #[test]
    fn only_the_listed_archive_files_can_be_read_and_unknown_paths_are_not_found() {
        let (_dir, site) = site();
        daily::save(&site.daily_dir, at("2026-10-20T12:10:00Z"), "x").unwrap();

        for path in [
            "/rapports/../edge.db",
            "/rapports/..%2Fedge.db",
            "/rapports/2026-10-21-121000Z.html",
            "/edge.db",
            "/inconnue",
        ] {
            let (status, body) = get(&site, path);
            assert_eq!(status, 404, "{path}");
            assert!(body.contains("Page introuvable"));
        }
        assert_eq!(get(&site, "/rapports/2026-10-20-121000Z.html?x=1").0, 200);
    }

    #[test]
    fn opportunities_are_the_staked_recommendations_whose_match_has_not_started() {
        let (_dir, site) = site();
        insert(
            &site,
            &[
                recommendation("passé", "2026-10-19T14:00:00Z", Some(20.0)),
                recommendation("sans-mise", "2026-10-24T14:00:00Z", None),
                recommendation("à-venir", "2026-10-20T18:00:00Z", Some(20.0)),
            ],
        );

        let (status, html) = get(&site, "/opportunites");

        assert_eq!(status, 200);
        assert_eq!(html.matches("class=\"card\"").count(), 1, "{html}");
        assert!(html.contains("recommandation n° 3"));
        assert!(html.contains("dans 5 h 00"));
        assert!(html.contains("2,0 u (20,0 €)"));
    }

    #[test]
    fn the_paper_trading_page_lists_staked_recommendations_only() {
        let (_dir, site) = site();
        insert(
            &site,
            &[
                recommendation("joué", "2026-10-24T14:00:00Z", Some(20.0)),
                recommendation("sans-mise", "2026-10-25T14:00:00Z", None),
            ],
        );

        let (status, html) = get(&site, "/paper");

        assert_eq!(status, 200);
        assert!(html.contains("n° 1"));
        assert!(!html.contains("n° 2"), "pas de mise, pas de ligne");
        assert!(html.contains("à venir"));
        assert!(html.contains("Moins de 30 clôtures"));
        assert!(!html.contains("<svg"), "rien de réglé, pas de courbe");
    }

    #[test]
    fn a_missing_database_gives_an_error_page_not_a_crash() {
        let (dir, mut site) = site();
        site.db_file = dir.path().join("absente.db");

        let (status, html) = get(&site, "/paper");

        assert_eq!(status, 500);
        assert!(html.contains("absente.db"));
    }
}
