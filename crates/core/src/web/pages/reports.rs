//! Onglet « Rapports » (`/rapports`) : les rapports quotidiens archivés, les rapports d'analyse écrits par la ligne de
//! commande (`edge backtest report`…), et ce que l'archive contient.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;

use super::{Feed, feeds};
use crate::error::Result;
use crate::format::{age, long_date};
use crate::report::daily;
use crate::store;
use crate::web::App;
use crate::web::view::{Flash, Layout, render};

const TEMPLATE: &str = include_str!("../templates/reports.html");

/// Un rapport d'analyse du dossier des rapports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisFile {
    /// Nom du fichier.
    pub name: String,
    /// Chemin complet.
    pub path: PathBuf,
    /// Dernière modification.
    pub modified: Option<SystemTime>,
}

/// Les fichiers `.html` du dossier des rapports d'analyse, du plus récent au plus ancien. Un dossier absent donne une
/// liste vide. Seuls ces fichiers-là sont servis par `/rapports/analyse/<nom>`.
pub fn analysis_files(dir: &Path) -> Vec<AnalysisFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<AnalysisFile> = entries
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "html"))
        .map(|entry| AnalysisFile {
            name: entry.file_name().to_string_lossy().into_owned(),
            modified: entry.metadata().and_then(|m| m.modified()).ok(),
            path: entry.path(),
        })
        .collect();
    files.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    files
}

/// Un lien vers un rapport.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Link {
    href: String,
    label: String,
    note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ReportsPage {
    #[serde(flatten)]
    layout: Layout,
    daily: Vec<Link>,
    analysis: Vec<Link>,
    feeds: Vec<Feed>,
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

/// La page.
pub fn page(app: &App, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let daily = daily::list(&app.paths.daily_reports_dir)?
        .into_iter()
        .map(|report| Link {
            href: format!("/rapports/{}", report.name),
            label: run_label(report.at),
            note: String::new(),
        })
        .collect();
    let analysis = analysis_files(&app.paths.reports_dir)
        .into_iter()
        .map(|file| Link {
            href: format!("/rapports/analyse/{}", file.name),
            note: file
                .modified
                .map_or("date inconnue".to_string(), |t| age(t.into(), now)),
            label: file.name,
        })
        .collect();
    render(
        "reports.html",
        TEMPLATE,
        &ReportsPage {
            layout: Layout::new("reports", now, flash),
            daily,
            analysis,
            feeds: feeds(&conn, now)?,
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::report::daily;
    use crate::web::testing::empty;

    #[test]
    fn the_daily_reports_are_listed_newest_first_in_paris_time() {
        let sample = empty();
        let dir = &sample.app.paths.daily_reports_dir;
        assert!(sample.html("/rapports").contains("Aucun rapport archivé"));
        daily::save(dir, "2026-10-19T12:10:00Z".parse().unwrap(), "<p>hier</p>").unwrap();
        daily::save(
            dir,
            "2026-10-20T12:10:00Z".parse().unwrap(),
            "<p>aujourd'hui</p>",
        )
        .unwrap();

        let html = sample.html("/rapports");

        let first = html.find("/rapports/2026-10-20-121000Z.html").unwrap();
        let second = html.find("/rapports/2026-10-19-121000Z.html").unwrap();
        assert!(first < second);
        assert!(
            html.contains("lundi 19 octobre 2026 à 14:10"),
            "heure de Paris"
        );
        assert!(html.contains("Aucun rapport d'analyse"));
    }
}
