//! `edge aliases …` : alias d'équipes déduits par la rencontre (phase 23). Liste des propositions et des alias acceptés,
//! report des alias acceptés dans `config/aliases.toml`.
//!
//! Les propositions se valident sur la page Loto Foot de l'application ; la logique vit dans `edge_core::lotofoot::
//! alias_deduction`, `edge_core::store::alias_proposals` et `edge_core::ingest::aliases`.

use std::path::Path;

use anyhow::{Context, Result};
use edge_core::format::kickoff_label;
use edge_core::ingest::aliases::{self, Aliases};
use edge_core::paths::Paths;
use edge_core::store::{self, alias_proposals};

/// `edge aliases list` : propositions en attente, puis alias acceptés (et s'ils sont déjà dans la table versionnée).
pub fn list(paths: &Paths) -> Result<()> {
    // Base migrée plutôt que lecture seule : juste après une réinstallation, la table des propositions (migration 26)
    // n'existerait pas encore, et la commande échouerait sur une erreur SQL au lieu de répondre.
    let conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    let bundled = Aliases::bundled()?;
    let merged = bundled.with_learned(&conn)?;
    let pending = alias_proposals::pending(&conn, &merged)?;
    if pending.is_empty() {
        println!("Aucune proposition en attente.");
    } else {
        println!("Propositions en attente (à valider sur la page Loto Foot de l'application) :");
        for s in &pending {
            let p = &s.proposal;
            println!(
                "  n°{} [{}] « {} » → « {} » : {} {} – {}, {} ({})",
                s.id,
                p.section,
                p.name,
                p.canonical,
                p.seen.source.label(),
                p.seen.home,
                p.seen.away,
                kickoff_label(p.seen.kickoff),
                p.context
            );
        }
    }
    let accepted = alias_proposals::accepted(&conn)?;
    if !accepted.is_empty() {
        println!("Alias acceptés :");
        for (section, name, canonical) in &accepted {
            // Déjà dans le binaire : exporté, commité et réinstallé.
            let state = if bundled.resolve(section, name).is_some() {
                "dans la table versionnée"
            } else {
                "à exporter"
            };
            println!("  [{section}] « {name} » → « {canonical} » ({state})");
        }
    }
    Ok(())
}

/// `edge aliases export --to config/aliases.toml` : reporte les alias acceptés dans le fichier, commentaires gardés.
pub fn export(paths: &Paths, to: &Path, dry_run: bool) -> Result<()> {
    // Base migrée plutôt que lecture seule : juste après une réinstallation, la table des propositions (migration 26)
    // n'existerait pas encore, et la commande échouerait sur une erreur SQL au lieu de répondre.
    let conn = store::open_migrated(&paths.db_file)
        .context("base inaccessible : lance d'abord `edge init`")?;
    let accepted = alias_proposals::accepted(&conn)?;
    let text = std::fs::read_to_string(to)
        .with_context(|| format!("lecture impossible : {}", to.display()))?;
    let (out, changed) = aliases::export_learned(&text, &accepted)?;
    if changed.is_empty() {
        println!(
            "Rien à reporter : {} contient déjà les {} alias acceptés.",
            to.display(),
            accepted.len()
        );
        return Ok(());
    }
    for line in &changed {
        println!("  {line}");
    }
    if dry_run {
        println!(
            "{} ligne(s) à changer (à blanc : rien n'est écrit).",
            changed.len()
        );
        return Ok(());
    }
    std::fs::write(to, out).with_context(|| format!("écriture impossible : {}", to.display()))?;
    println!(
        "{} ligne(s) changée(s) dans {}. Relis le diff, commite, puis réinstalle edge pour embarquer la table.",
        changed.len(),
        to.display()
    );
    Ok(())
}
