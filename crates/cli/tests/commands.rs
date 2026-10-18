//! Tests d'intégration de `edge quota` et `edge snapshot`, sans réseau : on n'y fournit jamais de clé,
//! sauf pour `--dry-run` qui n'en a pas besoin.

mod common;

use common::{run_edge, text};

#[test]
fn dry_run_shows_the_cost_and_needs_no_key() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["snapshot", "--dry-run"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.contains("Aucun appel n'est effectué"));
    assert!(stdout.contains("6 ligue(s) × 1 crédit(s) = 6 crédit(s)"));
    assert!(stdout.contains("soccer_spl"));
    assert!(stdout.contains("Quota encore inconnu"));
}

#[test]
fn snapshot_without_a_key_fails_before_touching_the_database() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["snapshot"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("EDGE_ODDS_API_KEY"));
    assert!(!home.path().join("edge.db").exists());
}

#[test]
fn quota_without_a_key_fails_clearly() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["quota"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("clé API introuvable"));
}
