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

#[test]
fn fd_import_without_downloaded_files_points_to_fd_fetch() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["fd", "import"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("edge fd fetch"));
}

#[test]
fn market_report_on_an_empty_database_explains_what_to_do() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["market", "report"], None);

    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("edge fd fetch") && stderr.contains("edge fd import"));
}

#[test]
fn fd_check_on_an_empty_database_has_nothing_to_report() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["fd", "check"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("0 ligue-saisons"));
}

/// Parcours complet hors réseau : un CSV réel déposé dans le cache, importé, contrôlé, puis analysé.
#[test]
fn a_cached_csv_goes_through_import_check_and_report() {
    let home = tempfile::tempdir().unwrap();
    let cache = home.path().join("football-data");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("E0_2526.csv"),
        include_bytes!("../../core/tests/fixtures/fd/E0_2526_sample.csv"),
    )
    .unwrap();

    let import = run_edge(home.path(), &["fd", "import"], None);
    assert!(import.status.success(), "{}", text(&import.stderr));
    assert!(text(&import.stdout).contains("1 fichiers importés : 5 matchs (5 nouveaux)"));

    // Relancer ne duplique rien.
    let again = run_edge(home.path(), &["fd", "import"], None);
    assert!(text(&again.stdout).contains("5 matchs (0 nouveaux)"));

    let check = run_edge(home.path(), &["fd", "check"], None);
    assert!(check.status.success(), "{}", text(&check.stderr));
    assert!(text(&check.stdout).contains("Résultats publiés contredisant les buts : 0"));

    let report_path = home.path().join("rapport.html");
    let report = run_edge(
        home.path(),
        &["market", "report", "--out", report_path.to_str().unwrap()],
        None,
    );
    assert!(report.status.success(), "{}", text(&report.stderr));
    let html = std::fs::read_to_string(&report_path).unwrap();
    assert!(html.contains("Le marché comme pronostiqueur"));
    assert!(html.contains("<svg"));
}

#[test]
fn model_evaluate_without_a_frozen_tuning_points_to_model_tune() {
    let home = tempfile::tempdir().unwrap();
    let cache = home.path().join("football-data");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("E0_2526.csv"),
        include_bytes!("../../core/tests/fixtures/fd/E0_2526_sample.csv"),
    )
    .unwrap();
    assert!(
        run_edge(home.path(), &["fd", "import"], None)
            .status
            .success()
    );

    let output = run_edge(home.path(), &["model", "evaluate"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("edge model tune"));
}

#[test]
fn model_report_on_an_empty_database_explains_what_to_do() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["model", "report"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("edge fd fetch"));
}
