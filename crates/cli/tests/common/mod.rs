//! Aide partagée par les tests d'intégration : lancer le vrai binaire `edge`.
//!
//! `CARGO_BIN_EXE_edge` est fourni par Cargo : le chemin du binaire compilé pour ces tests.

use std::path::Path;
use std::process::{Command, Output};

/// Lance `edge --home <dossier> <args>` sans clé API ni webhook dans l'environnement, sauf clé précisée.
pub fn run_edge(home: &Path, args: &[&str], api_key: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_edge"));
    command.arg("--home").arg(home).args(args);
    command.env_remove("EDGE_ODDS_API_KEY");
    command.env_remove("EDGE_DISCORD_WEBHOOK");
    if let Some(key) = api_key {
        command.env("EDGE_ODDS_API_KEY", key);
    }
    command.output().expect("le binaire edge doit se lancer")
}

/// Texte d'une sortie standard ou d'erreur.
pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
