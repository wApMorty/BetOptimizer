//! Tests d'intégration de `edge lotofoot played`, sans réseau : une grille et une séance écrites directement en base.

mod common;

use common::{run_edge, text};
use journal::{one_session, played_count};

#[test]
fn the_last_session_is_declared_once_and_a_second_declaration_is_ignored() {
    let home = tempfile::tempdir().unwrap();
    one_session(home.path());

    let first = run_edge(home.path(), &["lotofoot", "played", "--last-session"], None);
    let again = run_edge(home.path(), &["lotofoot", "played", "--last-session"], None);

    assert!(first.status.success(), "{}", text(&first.stderr));
    assert!(text(&first.stdout).contains("2 ticket(s) joué(s) enregistré(s)."));
    assert!(again.status.success(), "{}", text(&again.stderr));
    let stdout = text(&again.stdout);
    assert!(stdout.contains("déjà déclaré joué : ignoré"), "{stdout}");
    assert!(
        stdout.contains("0 ticket(s) joué(s) enregistré(s)."),
        "{stdout}"
    );
    assert_eq!(played_count(home.path()), 2);
}

#[test]
fn a_manual_ticket_is_linked_or_off_model_and_an_unknown_number_writes_nothing() {
    let home = tempfile::tempdir().unwrap();
    one_session(home.path());

    let unknown = run_edge(home.path(), &["lotofoot", "played", "1", "999"], None);
    assert!(!unknown.status.success());
    assert!(text(&unknown.stderr).contains("n°999 introuvable"));
    assert_eq!(played_count(home.path()), 0);

    let manual = |ticket: &str| {
        run_edge(
            home.path(),
            &[
                "lotofoot", "played", "--grid", "lf7:133", "--ticket", ticket,
            ],
            None,
        )
    };
    let off = manual("1111111");
    assert!(
        text(&off.stdout).contains("(hors modèle)"),
        "{}",
        text(&off.stdout)
    );
    let linked = manual("1N21N21");
    assert!(!text(&linked.stdout).contains("hors modèle"));
    let twice = manual("1N21N21");
    assert!(text(&twice.stdout).contains("déjà déclaré joué, ignoré"));
    assert_eq!(played_count(home.path()), 2);

    let nothing = run_edge(home.path(), &["lotofoot", "played"], None);
    assert!(!nothing.status.success());
    assert!(text(&nothing.stderr).contains("rien à enregistrer"));
}

/// Données de test écrites directement dans la base (`unwrap` toléré dans un module de test).
#[cfg(test)]
mod journal {
    use super::run_edge;
    use chrono::{Duration, Utc};
    use edge_core::lotofoot::pronomatch::{GridPage, OpenGrid};
    use edge_core::lotofoot::{Game, Ticket};
    use edge_core::store::lotofoot::{
        NewReco, NewSession, Page, plays, record_fetch, record_grid_page, record_session,
    };
    use edge_core::store::{migrate, open};

    /// Écrit dans la base du dossier une grille LF7 n°133 et une séance de deux tickets dessus.
    pub fn one_session(home: &std::path::Path) {
        assert!(run_edge(home, &["init"], None).status.success());
        let mut conn = open(&home.join("edge.db")).unwrap();
        migrate::migrate(&mut conn).unwrap();
        let now = Utc::now();
        let fetch_id = record_fetch(&conn, now, Page::Grid(Game::Lf7), 200, None).unwrap();
        let page = GridPage {
            open: Some(OpenGrid {
                game: Game::Lf7,
                accepting: true,
                number: 133,
                closes_at: now + Duration::days(1),
                jackpot: None,
                matches: (1..=7)
                    .map(|i| (format!("H{i}"), format!("A{i}")))
                    .collect(),
                split: Some(vec![[50, 30, 20]; 7]),
                split_at: None,
            }),
            history: vec![],
        };
        record_grid_page(&mut conn, fetch_id, Game::Lf7, &page).unwrap();
        let tickets = [
            Ticket::parse("1N21N21").unwrap(),
            Ticket::parse("2N21N21").unwrap(),
        ];
        let recos: Vec<NewReco<'_>> = tickets
            .iter()
            .enumerate()
            .map(|(i, ticket)| NewReco {
                game: Game::Lf7,
                number: 133,
                ticket,
                position: i + 1,
                ev_base: 1.8,
                ev_pessimistic: 1.4,
                paid_probability: 0.02,
                stakes_estimate: 200_000.0,
                split_fetch_id: fetch_id,
                inputs: "{}",
            })
            .collect();
        let session = NewSession {
            created_at: now,
            model_version: "test",
            params: "{}",
            notes: "{}",
        };
        record_session(&mut conn, &session, &recos).unwrap();
    }

    pub fn played_count(home: &std::path::Path) -> usize {
        plays(&open(&home.join("edge.db")).unwrap()).unwrap().len()
    }
}
