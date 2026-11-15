//! Tests d'intégration de `edge aliases` (phase 23) : une proposition acceptée se liste, puis se reporte dans le fichier
//! d'alias, une seule fois.

mod common;

use chrono::{DateTime, Utc};
use common::{run_edge, text};
use edge_core::ingest::aliases::Aliases;
use edge_core::lotofoot::alias_deduction::{Proposal, Seen, SeenSource};
use edge_core::store::{self, alias_proposals};

#[test]
fn an_accepted_alias_is_listed_then_exported_once() {
    let home = tempfile::tempdir().unwrap();
    assert!(run_edge(home.path(), &["init"], None).status.success());
    let listed = run_edge(home.path(), &["aliases", "list"], None);
    assert!(text(&listed.stdout).contains("Aucune proposition en attente."));

    let now: DateTime<Utc> = "2026-11-15T10:00:00Z".parse().unwrap();
    let mut conn = store::open_migrated(&home.path().join("edge.db")).unwrap();
    for (name, canonical) in [
        ("Fulham Football Club", "Fulham"),
        ("Leeds Club Inconnu", "Leeds United"),
    ] {
        let proposal = Proposal {
            section: "LOTOFOOT".into(),
            name: name.into(),
            canonical: canonical.into(),
            context: "LF8 n°160, rencontre 1".into(),
            seen: Seen {
                source: SeenSource::OddsApi,
                id: "ev1".into(),
                home: canonical.into(),
                away: "Everton".into(),
                kickoff: now,
            },
        };
        alias_proposals::record(&conn, &proposal, now).unwrap();
    }
    alias_proposals::decide(&mut conn, &Aliases::bundled().unwrap(), 1, true, now).unwrap();
    drop(conn);

    let listed = text(&run_edge(home.path(), &["aliases", "list"], None).stdout);
    assert!(
        listed.contains("n°2 [LOTOFOOT] « Leeds Club Inconnu » → « Leeds United »"),
        "{listed}"
    );
    assert!(
        listed.contains("« Fulham Football Club » → « Fulham » (à exporter)"),
        "{listed}"
    );

    let file = home.path().join("aliases.toml");
    std::fs::write(&file, "# en-tête\n[LOTOFOOT]\n\"Fulham\" = []\n").unwrap();
    let to = file.to_str().unwrap();
    let dry = run_edge(
        home.path(),
        &["aliases", "export", "--to", to, "--dry-run"],
        None,
    );
    assert!(
        text(&dry.stdout).contains("à blanc"),
        "{}",
        text(&dry.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# en-tête\n[LOTOFOOT]\n\"Fulham\" = []\n"
    );
    let done = run_edge(home.path(), &["aliases", "export", "--to", to], None);
    assert!(done.status.success(), "{}", text(&done.stderr));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# en-tête\n[LOTOFOOT]\n\"Fulham\" = [\"Fulham Football Club\"]\n"
    );
    let again = run_edge(home.path(), &["aliases", "export", "--to", to], None);
    assert!(text(&again.stdout).contains("Rien à reporter"));
}
