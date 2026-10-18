//! Point d'entrée de l'application `edge-gui`.
//!
//! L'application lit et écrit le journal (recommandations, paris, règlements) par l'API d'`edge-core`. Elle ne contacte
//! jamais l'API de cotes ni aucun bookmaker : ni crédit dépensé, ni pari placé par le programme.

mod actions;
mod app;
mod data;
mod forms;
mod tabs;
mod theme;

#[cfg(test)]
mod testing;

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use edge_core::paths::Paths;
use eframe::egui;

/// `edge-gui [--home DOSSIER]` : sans option, les emplacements standard de l'utilisateur (comme `edge`).
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut home: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--home" => {
                home = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| anyhow!("--home attend un dossier"))?,
                );
            }
            other => {
                return Err(anyhow!(
                    "option inconnue : {other} (seule --home DOSSIER existe)"
                ));
            }
        }
    }
    let paths = match home {
        Some(dir) => Paths::under(&dir),
        None => Paths::discover()?,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("edge")
            .with_app_id("edge")
            .with_inner_size([1180.0, 780.0]),
        renderer: eframe::Renderer::Wgpu,
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "edge",
        options,
        Box::new(|_creation| Ok(Box::new(app::App::new(paths)))),
    )
    .map_err(|error| anyhow!("{error}"))
}
