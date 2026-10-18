//! L'application : la barre d'onglets, le bandeau de messages, et l'exécution des actions demandées par les onglets.

use std::time::{Duration, Instant};

use chrono::Utc;
use edge_core::format;
use edge_core::paths::Paths;
use eframe::egui::{self, CentralPanel, Panel, RichText, Ui};

use crate::actions::{self, Action};
use crate::data::{self, Data};
use crate::tabs::{self, Tab, TabStates};
use crate::theme;

/// Les données sont relues d'elles-mêmes toutes les cinq minutes (les timers écrivent dans la même base).
const REFRESH_EVERY: Duration = Duration::from_secs(300);

/// Message affiché sous les onglets.
enum Notice {
    Success(String),
    Failure(String),
}

/// L'état de l'application.
pub struct App {
    paths: Paths,
    data: Option<Data>,
    notice: Option<Notice>,
    tab: Tab,
    states: TabStates,
    loaded_at: Instant,
}

impl App {
    /// Crée l'application et lit les données une première fois.
    pub fn new(paths: Paths) -> App {
        let mut app = App {
            paths,
            data: None,
            notice: None,
            tab: Tab::Recommendations,
            states: TabStates::default(),
            loaded_at: Instant::now(),
        };
        app.reload();
        app
    }

    /// Relit le journal et l'archive. En cas d'échec, les anciennes données restent affichées avec le message d'erreur.
    fn reload(&mut self) {
        match data::load(&self.paths, Utc::now()) {
            Ok(fresh) => self.data = Some(fresh),
            Err(error) => self.notice = Some(Notice::Failure(format!("{error:#}"))),
        }
        self.loaded_at = Instant::now();
    }

    /// Exécute une action de l'utilisateur, puis relit les données.
    fn run(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Refresh => self.reload(),
            Action::OpenReport(path) => {
                // Les espaces sont les seuls caractères courants à échapper dans un chemin de fichier.
                let url = format!("file://{}", path.display()).replace(' ', "%20");
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            other => match actions::apply(&self.paths, &other, Utc::now()) {
                Ok(message) => {
                    self.notice = Some(Notice::Success(message));
                    self.reload();
                }
                Err(error) => self.notice = Some(Notice::Failure(format!("{error:#}"))),
            },
        }
    }

    /// Dessine toute la fenêtre et renvoie les actions demandées.
    fn draw(&mut self, ui: &mut Ui) -> Vec<Action> {
        let mut requested = Vec::new();
        Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    ui.selectable_value(&mut self.tab, tab, tab.title());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Rafraîchir").clicked() {
                        requested.push(Action::Refresh);
                    }
                    if let Some(data) = &self.data {
                        ui.label(
                            RichText::new(format!(
                                "données lues {}",
                                format::age(data.now, Utc::now())
                            ))
                            .weak(),
                        );
                    }
                });
            });
        });
        if let Some(notice) = &self.notice {
            let mut dismissed = false;
            Panel::bottom("notice").show(ui, |ui| {
                ui.horizontal(|ui| {
                    match notice {
                        Notice::Success(text) => {
                            ui.label(RichText::new(text).color(theme::positive(ui)))
                        }
                        Notice::Failure(text) => {
                            ui.label(RichText::new(text).color(theme::negative(ui)))
                        }
                    };
                    dismissed = ui.button("Fermer").clicked();
                });
            });
            if dismissed {
                self.notice = None;
            }
        }
        CentralPanel::default().show(ui, |ui| match &self.data {
            Some(data) => requested.extend(tabs::show(self.tab, ui, data, &mut self.states)),
            None => {
                ui.heading("Données indisponibles");
                ui.label("La base n'a pas pu être lue (voir le message en bas). Lance `edge init` dans un terminal si c'est le premier démarrage.");
            }
        });
        requested
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if self.loaded_at.elapsed() >= REFRESH_EVERY {
            self.reload();
        }
        // Sans événement, egui ne redessine pas : on demande un réveil pour relire les données à temps.
        ui.ctx().request_repaint_after(REFRESH_EVERY);
        let requested = self.draw(ui);
        let ctx = ui.ctx().clone();
        for action in requested {
            self.run(&ctx, action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forms::FollowForm;
    use crate::testing::sample;
    use eframe::egui::{Context, RawInput};

    /// Dessine deux images de l'application sans fenêtre (la seconde profite des tailles mesurées par la première).
    fn frame(app: &mut App) -> Vec<Action> {
        let ctx = Context::default();
        let mut requested = Vec::new();
        for _ in 0..2 {
            let output = ctx.run_ui(RawInput::default(), |ui| {
                requested.extend(app.draw(ui));
            });
            // Pas de fenêtre pour recevoir les textures : on les abandonne explicitement.
            output.drop_without_applying_deltas();
        }
        requested
    }

    #[test]
    fn every_tab_draws_without_panicking_on_a_filled_journal() {
        let sample = sample();
        let mut app = App::new(sample.paths.clone());

        for tab in Tab::ALL {
            app.tab = tab;
            assert!(frame(&mut app).is_empty(), "{tab:?}");
        }
    }

    #[test]
    fn every_tab_draws_on_an_empty_database() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(Paths::under(dir.path()));

        for tab in Tab::ALL {
            app.tab = tab;
            assert!(frame(&mut app).is_empty(), "{tab:?}");
        }
        assert!(
            app.data.is_some(),
            "une base vierge est créée et migrée à la lecture"
        );
    }

    #[test]
    fn the_follow_form_opens_prefilled_and_draws() {
        let sample = sample();
        let mut app = App::new(sample.paths.clone());
        let found = app.data.as_ref().unwrap().recommendations[0].clone();
        app.states.recommendations.follow = Some(FollowForm::for_recommendation(&found));
        app.tab = Tab::Recommendations;

        frame(&mut app);

        let form = app.states.recommendations.follow.as_ref().unwrap();
        assert_eq!(form.odds, "2,30");
        assert_eq!(form.bookmaker, "betclic_fr");
        assert!(!form.stake.is_empty());
    }

    #[test]
    fn running_an_action_writes_the_journal_reloads_and_reports_the_outcome() {
        let sample = sample();
        let mut app = App::new(sample.paths.clone());
        let ctx = Context::default();
        let action = FollowForm::for_recommendation(&app.data.as_ref().unwrap().recommendations[0])
            .to_action()
            .unwrap();

        app.run(&ctx, action.clone());

        assert!(matches!(app.notice, Some(Notice::Success(_))));
        assert_eq!(app.data.as_ref().unwrap().bets.len(), 1);
        assert!(app.data.as_ref().unwrap().curve.open_stakes > 0.0);

        // Le même pari une seconde fois est refusé, et le message le dit.
        app.run(&ctx, action);
        assert!(
            matches!(&app.notice, Some(Notice::Failure(text)) if text.contains("déjà enregistré"))
        );
        assert_eq!(app.data.as_ref().unwrap().bets.len(), 1);
    }
}
