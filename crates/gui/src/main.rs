//! Point d'entrée de l'application `edge-gui`.
//!
//! L'application lit et écrit le journal (recommandations, paris, règlements) par l'API d'`edge-core`. Elle ne contacte
//! jamais l'API de cotes ni aucun bookmaker : ni crédit dépensé, ni pari placé par le programme.
//!
//! Options : `--home DOSSIER` (emplacements de test), `--demo` (données fictives, rien lu ni écrit), `--light` (thème clair),
//! `--no-motion` (aucune animation), `--screenshot DOSSIER` (photographie les écrans de la démo puis quitte).

mod actions;
mod app;
mod data;
mod demo;
mod design;
mod forms;
mod pages;
mod screenshot;

#[cfg(test)]
mod testing;

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use edge_core::paths::Paths;
use eframe::egui;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut home: Option<PathBuf> = None;
    let mut options = app::Options::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--home" => {
                home = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| anyhow!("--home attend un dossier"))?,
                );
            }
            "--screenshot" => {
                options.screenshot_dir = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| anyhow!("--screenshot attend un dossier"))?,
                );
                options.demo = true;
            }
            "--demo" => options.demo = true,
            "--light" => options.light = true,
            "--no-motion" => options.reduce_motion = true,
            other => {
                return Err(anyhow!(
                    "option inconnue : {other} (options : --home DOSSIER, --demo, --light, --no-motion, --screenshot DOSSIER)"
                ));
            }
        }
    }
    let paths = match home {
        Some(dir) => Paths::under(&dir),
        None => Paths::discover()?,
    };

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("edge")
            .with_app_id("edge")
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([980.0, 640.0]),
        renderer: eframe::Renderer::Wgpu,
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "edge",
        native,
        Box::new(move |creation| {
            crate::design::style::install_fonts(&creation.egui_ctx);
            crate::design::style::apply(&creation.egui_ctx, !options.light);
            Ok(Box::new(app::App::new(paths, &options)))
        }),
    )
    .map_err(|error| anyhow!("{error}"))
}
