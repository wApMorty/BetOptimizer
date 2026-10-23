//! Onglet « Latence » (`/latence`) : l'état du test prospectif de la latence française (phase 13).

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;

use crate::error::Result;
use crate::report::latency_page::{self, LatencyPage};
use crate::store;
use crate::web::App;
use crate::web::view::{Flash, Layout, render};

const TEMPLATE: &str = include_str!("../templates/latency.html");

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Page {
    #[serde(flatten)]
    layout: Layout,
    #[serde(flatten)]
    latency: LatencyPage,
}

/// La page.
pub fn page(app: &App, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let today = now.with_timezone(&Paris).date_naive();
    let report = crate::eval::latency::evaluate(&conn, &app.aliases, today)?;
    render(
        "latency.html",
        TEMPLATE,
        &Page {
            layout: Layout::new("latency", now, flash),
            latency: latency_page::view(&report),
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::web::testing::empty;

    #[test]
    fn the_latency_page_shows_the_frozen_rule_and_waits_without_data() {
        let html = empty().html("/latence");
        assert!(html.contains("Latence des opérateurs français"), "{html}");
        assert!(html.contains("class=\"on\"><svg"));
        assert!(html.contains("En attente : 0 pari(s) sur 32"));
        assert!(html.contains("Aucun pari pour l'instant"));
    }
}
