//! Tests d'intégration : on lance le vrai binaire `edge` et on observe ce qu'il fait.
//!
//! `CARGO_BIN_EXE_edge` est fourni par Cargo : le chemin du binaire compilé pour ces tests.

use std::process::{Command, Output};

const SECRET: &str = "SECRET-DE-TEST-123";

/// Lance `edge --home <dossier> <args>` sans clé API dans l'environnement, sauf si précisée.
fn run_edge(home: &std::path::Path, args: &[&str], api_key: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_edge"));
    command.arg("--home").arg(home).args(args);
    command.env_remove("EDGE_ODDS_API_KEY");
    if let Some(key) = api_key {
        command.env("EDGE_ODDS_API_KEY", key);
    }
    command.output().expect("le binaire edge doit se lancer")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn init_creates_config_and_database() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["init"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.contains("(créée)"));
    assert!(stdout.contains("version du schéma : 1"));
    assert!(stdout.contains("Clé API : absente"));
    assert!(home.path().join("edge.toml").exists());
    assert!(home.path().join("edge.db").exists());
}

#[test]
fn init_twice_succeeds_and_keeps_the_config() {
    let home = tempfile::tempdir().unwrap();
    run_edge(home.path(), &["init"], None);

    let output = run_edge(home.path(), &["init"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("(déjà présente)"));
}

#[test]
fn key_from_environment_is_reported_but_never_printed() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["init"], Some(SECRET));

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("Clé API : trouvée"));
    assert!(!text(&output.stdout).contains(SECRET));
    assert!(!text(&output.stderr).contains(SECRET));
}

#[test]
fn unknown_command_fails() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["nimporte-quoi"], None);

    assert!(!output.status.success());
}

#[cfg(unix)]
#[test]
fn config_with_a_key_and_open_permissions_makes_init_fail_clearly() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("edge.toml");
    fs::write(&config, format!(r#"odds_api_key = "{SECRET}""#)).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();

    let output = run_edge(home.path(), &["init"], None);

    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("chmod 600"));
    assert!(!stderr.contains(SECRET));
}
