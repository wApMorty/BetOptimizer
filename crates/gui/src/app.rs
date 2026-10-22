//! L'application : barre latérale, écrans, tiroir de saisie, notifications, et exécution des demandes des écrans.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::Utc;
use edge_core::domain::Outcome;
use edge_core::format;
use edge_core::paths::Paths;
use eframe::egui::{
    self, Align, Align2, CentralPanel, CornerRadius, Event, Frame, Id, Key, Layout, Margin, Panel,
    RichText, ScrollArea, Sense, Stroke, Ui, ViewportCommand, vec2,
};

use crate::actions::{self, Action};
use crate::data::{self, Data};
use crate::demo;
use crate::design::icons::{self, Icon};
use crate::design::motion::Motion;
use crate::design::style;
use crate::design::style::Semibold;
use crate::design::tokens::{radius, space};
use crate::design::widgets::{
    ButtonKind, Tone, button, chip, drawer, field, icon_button, mix, pal, segmented,
};
use crate::forms::{BonusForm, FollowForm, OffModelForm, OfferForm};
use crate::pages::{self, PageCtx, PageStates, Request, Tab};
use crate::screenshot::{self, SHOTS};

/// Les données sont relues d'elles-mêmes toutes les cinq minutes (les timers écrivent dans la même base).
const REFRESH_EVERY: Duration = Duration::from_secs(300);

/// Largeur de la barre latérale.
const NAV_WIDTH: f32 = 216.0;

/// Largeur de la barre latérale réduite aux icônes.
const NAV_COMPACT_WIDTH: f32 = 68.0;

/// Sous cette largeur de fenêtre, la barre latérale ne garde que les icônes (fenêtre tuilée, petit écran).
const COMPACT_BELOW: f32 = 1000.0;

/// Largeur maximale du contenu.
const CONTENT_WIDTH: f32 = 980.0;

/// Une notification.
struct Toast {
    text: String,
    failure: bool,
    born: f64,
    closing: Option<f64>,
}

/// Le tiroir de saisie ouvert, avec son formulaire et sa dernière erreur.
enum Drawer {
    Follow(FollowForm, Option<String>),
    OffModel(OffModelForm, Option<String>),
    Bonus(BonusForm, Option<String>),
    Offer(OfferForm, Option<String>),
}

/// Réglages de lancement.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Données fictives, rien lu ni écrit.
    pub demo: bool,
    /// Dossier où photographier les écrans (puis quitter).
    pub screenshot_dir: Option<PathBuf>,
    /// Thème clair au lieu du sombre.
    pub light: bool,
    /// Couper tous les mouvements.
    pub reduce_motion: bool,
}

/// L'état de l'application.
pub struct App {
    paths: Paths,
    data: Option<Data>,
    toasts: Vec<Toast>,
    tab: Tab,
    states: PageStates,
    drawer: Option<Drawer>,
    page_started: Option<f64>,
    reduce_motion: bool,
    dark: bool,
    demo: bool,
    loaded_at: Instant,
    screenshot: Option<screenshot::Plan>,
    styled: Option<bool>,
}

impl App {
    /// Crée l'application et lit les données une première fois.
    pub fn new(paths: Paths, options: &Options) -> App {
        let mut app = App {
            paths,
            data: None,
            toasts: Vec::new(),
            tab: Tab::Today,
            states: PageStates::default(),
            drawer: None,
            page_started: None,
            reduce_motion: options.reduce_motion,
            dark: !options.light,
            demo: options.demo,
            loaded_at: Instant::now(),
            screenshot: options.screenshot_dir.clone().map(|dir| screenshot::Plan {
                dir,
                index: 0,
                requested: false,
            }),
            styled: None,
        };
        if app.demo {
            app.data = Some(demo::data(Utc::now()));
        } else {
            app.reload();
        }
        app
    }

    /// Relit le journal et l'archive. En cas d'échec, les anciennes données restent affichées avec le message d'erreur.
    fn reload(&mut self) {
        if self.demo {
            return;
        }
        match data::load(&self.paths, Utc::now()) {
            Ok(fresh) => self.data = Some(fresh),
            Err(error) => self.toast(format!("{error:#}"), true, 0.0),
        }
        self.loaded_at = Instant::now();
    }

    fn toast(&mut self, text: String, failure: bool, now: f64) {
        self.toasts.push(Toast {
            text,
            failure,
            born: now,
            closing: None,
        });
    }

    fn go(&mut self, tab: Tab, now: f64) {
        if self.tab != tab {
            self.tab = tab;
            self.page_started = Some(now);
        }
    }

    /// Exécute une demande d'un écran.
    fn run(&mut self, ctx: &egui::Context, request: Request, now: f64) {
        match request {
            Request::Go(tab) => self.go(tab, now),
            Request::Follow(id) => {
                let found = self
                    .data
                    .as_ref()
                    .and_then(|d| d.recommendations.iter().find(|r| r.id == id));
                if let Some(found) = found {
                    self.drawer = Some(Drawer::Follow(FollowForm::for_recommendation(found), None));
                }
            }
            Request::NewOffModelBet => {
                self.drawer = Some(Drawer::OffModel(OffModelForm::default(), None))
            }
            Request::OpenBonus(form) => self.drawer = Some(Drawer::Bonus(form, None)),
            Request::NewOffer(bookmaker) => {
                self.drawer = Some(Drawer::Offer(OfferForm::for_operator(&bookmaker), None))
            }
            Request::Do(Action::Refresh) => self.reload(),
            Request::Do(Action::OpenReport(path)) => {
                // Les espaces sont les seuls caractères courants à échapper dans un chemin de fichier.
                let url = format!("file://{}", path.display()).replace(' ', "%20");
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            Request::Do(_) if self.demo => {
                self.toast("Mode démo : rien n'est enregistré.".to_string(), false, now);
            }
            Request::Do(action) => match actions::apply(&self.paths, &action, Utc::now()) {
                Ok(message) => {
                    self.toast(message, false, now);
                    self.reload();
                }
                Err(error) => self.toast(format!("{error:#}"), true, now),
            },
        }
    }

    /// Raccourcis clavier : 1 à 6 changent d'écran, R relit les données (sans champ de saisie actif).
    fn shortcuts(&self, ctx: &egui::Context) -> Vec<Request> {
        if ctx.egui_wants_keyboard_input() {
            return Vec::new();
        }
        let mut requests = Vec::new();
        ctx.input(|input| {
            let keys = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
            ];
            for (index, key) in keys.into_iter().enumerate() {
                if input.key_pressed(key) {
                    requests.push(Request::Go(Tab::ALL[index]));
                }
            }
            if input.key_pressed(Key::R) {
                requests.push(Request::Do(Action::Refresh));
            }
        });
        requests
    }

    /// Barre latérale : logo, écrans, rappel de prudence, réglages.
    fn sidebar(&mut self, ui: &mut Ui) -> Vec<Request> {
        let p = pal(ui);
        let mut requests = Vec::new();
        let compact = ui.ctx().content_rect().width() < COMPACT_BELOW;
        Panel::left("nav")
            .exact_size(if compact { NAV_COMPACT_WIDTH } else { NAV_WIDTH })
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.surface)
                    .inner_margin(Margin::symmetric(if compact { 10 } else { 14 }, 18))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| {
                if compact {
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 6.0, p.accent);
                } else {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(rect.center(), 5.0, p.accent);
                        ui.label(RichText::new("edge").size(22.0).semibold().color(p.text));
                    });
                }
                if self.demo {
                    ui.add_space(space::XS);
                    if compact {
                        chip(ui, "DÉMO", Tone::Warning).on_hover_text("Données fictives : rien n'est lu ni écrit.");
                    } else {
                        chip(ui, "DÉMO · données fictives", Tone::Warning);
                    }
                }
                ui.add_space(space::XL);

                // Pastille de sélection qui glisse d'un écran à l'autre.
                let row_height = 40.0;
                let pitch = row_height + 4.0;
                let origin = ui.cursor().min;
                let selected = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0) as f32;
                let animated = if self.reduce_motion {
                    selected
                } else {
                    ui.ctx().animate_value_with_time(Id::new("nav-pill"), selected, 0.22)
                };
                let width = ui.available_width();
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(origin + vec2(0.0, animated * pitch), vec2(width, row_height)),
                    CornerRadius::same(radius::CONTROL),
                    p.accent_soft,
                );
                for (index, tab) in Tab::ALL.into_iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, row_height), Sense::click());
                    ui.add_space(4.0 - ui.spacing().item_spacing.y);
                    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
                    if tab != self.tab && hover > 0.0 {
                        ui.painter().rect_filled(rect, CornerRadius::same(radius::CONTROL), mix(egui::Color32::TRANSPARENT, p.raised, hover));
                    }
                    let active = tab == self.tab;
                    let color = if active { p.accent } else { mix(p.muted, p.text, hover) };
                    if compact {
                        icons::draw(ui.painter(), egui::Rect::from_center_size(rect.center(), vec2(20.0, 20.0)), tab.icon(), color);
                    } else {
                        icons::draw(ui.painter(), egui::Rect::from_center_size(rect.left_center() + vec2(22.0, 0.0), vec2(20.0, 20.0)), tab.icon(), color);
                        ui.painter().text(
                            rect.left_center() + vec2(44.0, 0.0),
                            Align2::LEFT_CENTER,
                            tab.title(),
                            egui::FontId::proportional(14.5),
                            if active { p.text } else { color },
                        );
                        ui.painter().text(
                            rect.right_center() - vec2(12.0, 0.0),
                            Align2::RIGHT_CENTER,
                            (index + 1).to_string(),
                            egui::FontId::proportional(11.5),
                            p.faint,
                        );
                    }
                    if response.clicked() {
                        requests.push(Request::Go(tab));
                    }
                    let response = if compact { response.on_hover_text(format!("{} ({})", tab.title(), index + 1)) } else { response };
                    response.on_hover_cursor(egui::CursorIcon::PointingHand);
                }

                // Le bas se remplit de bas en haut : le premier élément ajouté est le plus bas.
                ui.with_layout(Layout::bottom_up(if compact { Align::Center } else { Align::LEFT }), |ui| {
                    let theme_tip = if self.dark { "Thème clair" } else { "Thème sombre" };
                    let theme_icon = if self.dark { Icon::Sun } else { Icon::Moon };
                    let motion_tip = if self.reduce_motion { "Activer les animations" } else { "Réduire les animations" };
                    // Colonne (de bas en haut) en mode compact, rangée sinon : on ajoute dans l'ordre inverse pour lire thème, animations, relecture.
                    let order = if compact { [2, 1, 0] } else { [0, 1, 2] };
                    let mut buttons = |ui: &mut Ui| {
                        for which in order {
                            match which {
                                0 => {
                                    if icon_button(ui, theme_icon, theme_tip).clicked() {
                                        self.dark = !self.dark;
                                    }
                                }
                                1 => {
                                    if icon_button(ui, Icon::Pause, motion_tip).clicked() {
                                        self.reduce_motion = !self.reduce_motion;
                                    }
                                }
                                _ => {
                                    if icon_button(ui, Icon::Refresh, "Relire les données (R)").clicked() {
                                        requests.push(Request::Do(Action::Refresh));
                                    }
                                }
                            }
                        }
                    };
                    if compact {
                        buttons(ui);
                    } else {
                        ui.horizontal(buttons);
                        ui.add_space(space::SM);
                        let (fg, bg) = Tone::Warning.colors(p);
                        Frame::new()
                            .fill(bg)
                            .corner_radius(CornerRadius::same(radius::CONTROL))
                            .inner_margin(Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.vertical(|ui| {
                                    ui.label(RichText::new("Aucune règle validée").small().semibold().color(fg));
                                    ui.label(RichText::new("recommandations expérimentales").small().color(fg));
                                });
                            })
                            .response
                            .on_hover_text("Les mesures disponibles ne montrent pas d'avantage hors échantillon. Voir l'écran Aujourd'hui.");
                        if let Some(data) = &self.data {
                            ui.add_space(space::SM);
                            ui.label(RichText::new(format!("données lues {}", format::age(data.now, Utc::now()))).small().color(p.faint));
                        }
                    }
                });
            });
        requests
    }

    /// Notifications empilées en haut à droite : elles glissent, puis disparaissent seules (sauf les erreurs).
    fn show_toasts(&mut self, ctx: &egui::Context, now: f64) {
        let dark = self.dark;
        let reduce = self.reduce_motion;
        for toast in &mut self.toasts {
            if toast.closing.is_none() && !toast.failure && now - toast.born > 4.5 {
                toast.closing = Some(now);
            }
        }
        let mut slot = 0.0_f32;
        for (index, toast) in self.toasts.iter_mut().enumerate() {
            let p = crate::design::tokens::palette(dark);
            let id = Id::new(("toast", index, toast.born.to_bits()));
            let visible = toast.closing.is_none();
            let t = if reduce {
                f32::from(u8::from(visible))
            } else {
                ctx.animate_bool_with_time(id, visible, 0.25)
            };
            let eased = crate::design::motion::ease_out(t);
            if eased <= 0.0 {
                continue;
            }
            let (fg, bg) = if toast.failure {
                Tone::Negative.colors(p)
            } else {
                Tone::Positive.colors(p)
            };
            let mut close = false;
            egui::Area::new(id)
                .order(egui::Order::Tooltip)
                .anchor(
                    Align2::RIGHT_TOP,
                    vec2(-20.0 + 60.0 * (1.0 - eased), 20.0 + slot),
                )
                .show(ctx, |ui| {
                    ui.set_opacity(eased);
                    Frame::new()
                        .fill(mix(p.surface, bg, 0.55))
                        .stroke(Stroke::new(1.0, fg.gamma_multiply(0.6)))
                        .corner_radius(CornerRadius::same(radius::CARD))
                        .inner_margin(Margin::symmetric(14, 12))
                        .shadow(egui::Shadow {
                            offset: [0, 8],
                            blur: 24,
                            spread: 0,
                            color: p.shadow,
                        })
                        .show(ui, |ui| {
                            ui.set_max_width(380.0);
                            ui.horizontal(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                                icons::draw(
                                    ui.painter(),
                                    rect,
                                    if toast.failure {
                                        Icon::Info
                                    } else {
                                        Icon::Check
                                    },
                                    fg,
                                );
                                ui.label(RichText::new(&toast.text).color(p.text));
                                if icon_button(ui, Icon::Close, "Fermer").clicked() {
                                    close = true;
                                }
                            });
                        });
                });
            if close {
                toast.closing = Some(now);
            }
            slot += 64.0 * eased;
        }
        // On oublie les notifications refermées depuis plus d'une demi-seconde.
        self.toasts
            .retain(|t| t.closing.is_none_or(|at| now - at < 0.6));
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    /// Le tiroir de saisie : formulaire « J'ai parié » ou pari hors modèle.
    fn show_drawer(&mut self, ctx: &egui::Context) -> Vec<Request> {
        let Some(mut current) = self.drawer.take() else {
            return Vec::new();
        };
        let mut requests = Vec::new();
        let p = crate::design::tokens::palette(self.dark);
        let bookmakers: Vec<String> = self
            .data
            .as_ref()
            .map(|d| d.config.bookmakers.clone())
            .unwrap_or_default();
        let upcoming: Vec<(String, String)> = self
            .data
            .as_ref()
            .map(|d| {
                d.upcoming
                    .iter()
                    .map(|e| {
                        (
                            e.event_id.clone(),
                            format!(
                                "{} · {} – {}",
                                format::kickoff_label(e.kickoff),
                                e.home_team,
                                e.away_team
                            ),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let freebet_value = self
            .data
            .as_ref()
            .map_or(edge_core::bonus::DEFAULT_FREEBET_VALUE, |d| {
                d.config.bonus_freebet_value
            });
        let cover_accounts: Vec<String> = self
            .data
            .as_ref()
            .map(|d| d.bonus.cover_accounts.clone())
            .unwrap_or_default();
        let title = match &current {
            Drawer::Follow(form, _) => {
                format!("J'ai parié · recommandation n° {}", form.recommendation_id)
            }
            Drawer::OffModel(..) => "Pari hors modèle".to_string(),
            Drawer::Bonus(form, _) => format!("J'ai joué · {}", form.event_label),
            Drawer::Offer(form, _) => format!(
                "Offre · {}",
                edge_core::bonus::operator_name(&form.bookmaker)
            ),
        };
        let mut submit = false;
        let mut keep_open = drawer(ctx, "drawer", &title, 420.0, |ui| match &mut current {
            Drawer::Follow(form, error) => {
                ui.label(RichText::new("Saisis ce que tu as réellement obtenu. Une cote sous la cote minimale est refusée : le pari ne suit plus la recommandation.").color(p.muted));
                ui.add_space(space::MD);
                field(ui, "Cote obtenue", &mut form.odds, "2,30");
                field(ui, "Mise (€)", &mut form.stake, "50");
                field(ui, "Bookmaker", &mut form.bookmaker, "betclic_fr");
                if let Some(message) = error {
                    ui.label(RichText::new(message.as_str()).color(p.negative));
                }
                ui.add_space(space::SM);
                submit = button(ui, "Enregistrer le pari", ButtonKind::Primary).clicked();
            }
            Drawer::OffModel(form, error) => {
                ui.label(RichText::new("Un pari qui ne suit aucune recommandation. Il est rapporté à part et exclu des métriques du modèle.").color(p.muted));
                ui.add_space(space::MD);
                ui.label(RichText::new("Match").small().color(p.muted));
                let selected = form
                    .event_id
                    .as_ref()
                    .and_then(|id| upcoming.iter().find(|(event, _)| event == id))
                    .map_or("Choisir un match…".to_string(), |(_, label)| {
                        label.clone()
                    });
                egui::ComboBox::from_id_salt("drawer-event")
                    .width(ui.available_width())
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (event_id, label) in &upcoming {
                            ui.selectable_value(&mut form.event_id, Some(event_id.clone()), label);
                        }
                    });
                ui.add_space(space::SM);
                ui.label(RichText::new("Issue").small().color(p.muted));
                let mut choice = match form.outcome {
                    Some(Outcome::Home) => 0,
                    Some(Outcome::Draw) => 1,
                    Some(Outcome::Away) => 2,
                    None => 0,
                };
                if segmented(
                    ui,
                    "drawer-outcome",
                    &["Domicile", "Nul", "Extérieur"],
                    &mut choice,
                ) || form.outcome.is_none()
                {
                    form.outcome = Some(Outcome::ALL[choice]);
                }
                ui.add_space(space::SM);
                ui.label(RichText::new("Bookmaker").small().color(p.muted));
                ui.horizontal(|ui| {
                    let room = (ui.available_width() - 110.0).max(120.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut form.bookmaker)
                            .hint_text("betclic_fr")
                            .desired_width(room)
                            .margin(Margin::symmetric(10, 8)),
                    );
                    egui::ComboBox::from_id_salt("drawer-bookmaker")
                        .selected_text("liste")
                        .show_ui(ui, |ui| {
                            for name in &bookmakers {
                                if ui.selectable_label(false, name).clicked() {
                                    form.bookmaker = name.clone();
                                }
                            }
                        });
                });
                ui.add_space(space::SM);
                field(ui, "Cote obtenue", &mut form.odds, "3,40");
                field(ui, "Mise (€)", &mut form.stake, "5");
                if let Some(message) = error {
                    ui.label(RichText::new(message.as_str()).color(p.negative));
                }
                ui.add_space(space::SM);
                submit = button(ui, "Enregistrer le pari", ButtonKind::Primary).clicked();
            }
            Drawer::Bonus(form, error) => {
                ui.label(RichText::new("Place d'abord le pari bonus, puis tape les cotes réellement affichées chez les opérateurs de couverture, recalcule les mises, place les couvertures aussitôt, et enregistre.").color(p.muted));
                ui.add_space(space::MD);
                for (index, leg) in form.legs.iter_mut().enumerate() {
                    let role = if leg.role == edge_core::store::bonus::Role::Bonus {
                        "Bonus"
                    } else {
                        "Couverture"
                    };
                    ui.label(
                        RichText::new(format!("{role} · {}", format::outcome_label(leg.outcome)))
                            .semibold(),
                    );
                    if leg.role == edge_core::store::bonus::Role::Cover {
                        egui::ComboBox::from_id_salt(("drawer-cover-book", index))
                            .width(ui.available_width())
                            .selected_text(
                                edge_core::bonus::operator_name(&leg.bookmaker).to_string(),
                            )
                            .show_ui(ui, |ui| {
                                for key in &cover_accounts {
                                    ui.selectable_value(
                                        &mut leg.bookmaker,
                                        key.clone(),
                                        edge_core::bonus::operator_name(key),
                                    );
                                }
                            });
                    } else {
                        ui.label(
                            RichText::new(edge_core::bonus::operator_name(&leg.bookmaker))
                                .color(p.muted),
                        );
                    }
                    ui.columns(2, |cols| {
                        field(&mut cols[0], "Cote", &mut leg.odds, "2,10");
                        field(&mut cols[1], "Mise (€)", &mut leg.stake, "100");
                    });
                }
                if button(
                    ui,
                    "Recalculer les mises de couverture",
                    ButtonKind::Secondary,
                )
                .clicked()
                {
                    *error = form.recompute(freebet_value).err();
                }
                ui.add_space(space::SM);
                match form.guaranteed(freebet_value) {
                    Some(gain) => ui.label(
                        RichText::new(format!("Gain garanti : {} €", format::signed(gain, 2)))
                            .semibold()
                            .color(Tone::of_sign(gain).colors(p).0),
                    ),
                    None => {
                        ui.label(RichText::new("Gain garanti : saisie incomplète").color(p.muted))
                    }
                };
                if let Some(message) = error {
                    ui.label(RichText::new(message.as_str()).color(p.negative));
                }
                ui.add_space(space::SM);
                submit = button(ui, "Enregistrer l'opération", ButtonKind::Primary).clicked();
            }
            Drawer::Offer(form, error) => {
                ui.label(RichText::new("Lis la clause d'éligibilité du règlement (ou demande au service client) avant tout dépôt. L'offre la plus récente fait foi.").color(p.muted));
                ui.add_space(space::MD);
                ui.label(RichText::new("Opérateur").small().color(p.muted));
                egui::ComboBox::from_id_salt("drawer-offer-book")
                    .width(ui.available_width())
                    .selected_text(edge_core::bonus::operator_name(&form.bookmaker).to_string())
                    .show_ui(ui, |ui| {
                        for operator in edge_core::bonus::OPERATORS {
                            ui.selectable_value(
                                &mut form.bookmaker,
                                operator.key.to_string(),
                                operator.name,
                            );
                        }
                    });
                ui.add_space(space::SM);
                let mut eligible = usize::from(!form.eligible);
                segmented(
                    ui,
                    "drawer-offer-eligible",
                    &["Éligible", "Inéligible"],
                    &mut eligible,
                );
                form.eligible = eligible == 0;
                ui.add_space(space::SM);
                if form.eligible {
                    let kinds = edge_core::bonus::OfferKind::ALL;
                    let mut kind = kinds.iter().position(|k| *k == form.kind).unwrap_or(0);
                    segmented(
                        ui,
                        "drawer-offer-kind",
                        &pages::bonus::KIND_LABELS,
                        &mut kind,
                    );
                    form.kind = kinds[kind];
                    ui.add_space(space::SM);
                    field(ui, "Montant maximal (€)", &mut form.amount, "100");
                    field(
                        ui,
                        "Cote minimale (vide : aucune)",
                        &mut form.min_odds,
                        "1,50",
                    );
                } else {
                    ui.label(RichText::new("Le compte ne servira qu'à couvrir.").color(p.muted));
                }
                field(
                    ui,
                    "Source",
                    &mut form.source,
                    "règlement du 18/04/2025, clause C",
                );
                field(ui, "Remarque", &mut form.note, "");
                if let Some(message) = error {
                    ui.label(RichText::new(message.as_str()).color(p.negative));
                }
                ui.add_space(space::SM);
                submit = button(ui, "Enregistrer l'offre", ButtonKind::Primary).clicked();
            }
        });

        if submit {
            let (action, error_slot) = match &mut current {
                Drawer::Follow(form, error) => (form.to_action(), error),
                Drawer::OffModel(form, error) => (form.to_action(), error),
                Drawer::Bonus(form, error) => (form.to_action(freebet_value), error),
                Drawer::Offer(form, error) => {
                    (form.to_action(chrono::Local::now().date_naive()), error)
                }
            };
            match action {
                Ok(action) => {
                    requests.push(Request::Do(action));
                    keep_open = false;
                }
                Err(message) => *error_slot = Some(message),
            }
        }
        if keep_open {
            self.drawer = Some(current);
        }
        requests
    }

    /// Prépare la vue suivante d'une série de captures, ou demande la capture quand l'écran est posé.
    fn drive_screenshots(&mut self, ctx: &egui::Context, now: f64) {
        let Some(plan) = self.screenshot.as_mut() else {
            return;
        };
        // Dernière capture faite : la fermeture de la fenêtre est demandée, on ne dessine plus rien de plus.
        if plan.index >= SHOTS.len() {
            return;
        }
        ctx.request_repaint();
        for event in ctx.input(|i| i.events.clone()) {
            if let Event::Screenshot { image, .. } = event {
                let shot = SHOTS[plan.index];
                let _ = std::fs::create_dir_all(&plan.dir);
                let path = plan.dir.join(format!("{}.png", shot.name));
                match screenshot::save(&image, &path) {
                    Ok(()) => println!("Capture : {}", path.display()),
                    Err(error) => eprintln!("Capture impossible : {error:#}"),
                }
                plan.index += 1;
                plan.requested = false;
                if plan.index >= SHOTS.len() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    return;
                }
                let next = SHOTS[plan.index];
                self.tab = next.tab;
                self.states.performance.segment = next.segment;
                self.page_started = Some(now);
                self.drawer = next
                    .drawer
                    .then(|| Drawer::OffModel(OffModelForm::default(), None));
                return;
            }
        }
        if plan.index == 0
            && !plan.requested
            && self.page_started.is_none_or(|start| now - start < 0.0)
        {
            return;
        }
        let shot = SHOTS[plan.index];
        let started = self.page_started.unwrap_or(now);
        if !plan.requested && now - started > 2.6 {
            if self.tab != shot.tab {
                self.tab = shot.tab;
                self.states.performance.segment = shot.segment;
                self.page_started = Some(now);
                self.drawer = shot
                    .drawer
                    .then(|| Drawer::OffModel(OffModelForm::default(), None));
                return;
            }
            plan.requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    /// Dessine toute la fenêtre.
    fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        if self.page_started.is_none() {
            self.page_started = Some(now);
        }
        if self.styled != Some(self.dark) {
            style::apply(&ctx, self.dark);
            self.styled = Some(self.dark);
            // Le style change à partir de l'image suivante : on la demande.
            ctx.request_repaint();
        }
        let p = pal(ui);
        let mut requests = self.shortcuts(&ctx);
        requests.extend(self.sidebar(ui));

        let started = self.page_started.unwrap_or(now);
        let page_ctx = PageCtx {
            motion: Motion {
                enabled: !self.reduce_motion,
                started,
                now,
            },
        };
        CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(32, 28)))
            .show(ui, |ui| match &self.data {
                Some(data) => {
                    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        // `set_max_width` agrandit aussi la zone : on ne borne donc que si la fenêtre est plus large que le contenu.
                        ui.set_max_width(ui.available_width().min(CONTENT_WIDTH));
                        requests.extend(pages::show(self.tab, ui, &page_ctx, data, &mut self.states));
                        ui.add_space(space::XXL);
                    });
                }
                None => {
                    ui.label(RichText::new("Données indisponibles").heading().color(p.text));
                    ui.label(RichText::new("La base n'a pas pu être lue (voir la notification). Lance `edge init` dans un terminal si c'est le premier démarrage.").color(p.muted));
                }
            });
        page_ctx.motion.keep_alive(&ctx);

        requests.extend(self.show_drawer(&ctx));
        for request in requests {
            self.run(&ctx, request, now);
        }
        self.show_toasts(&ctx, now);
        self.drive_screenshots(&ctx, now);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if !self.demo && self.loaded_at.elapsed() >= REFRESH_EVERY {
            self.reload();
        }
        // Sans événement, egui ne redessine pas : on demande un réveil pour relire les données à temps.
        ui.ctx().request_repaint_after(REFRESH_EVERY);
        self.draw(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sample;
    use eframe::egui::{Context, RawInput};

    /// Dessine deux images de l'application sans fenêtre (la seconde profite des tailles mesurées par la première).
    fn frames(app: &mut App, count: usize) {
        let ctx = Context::default();
        // En vrai, le style est installé à la création de la fenêtre (main.rs) ; ici, avant la première image.
        style::install_fonts(&ctx);
        style::apply(&ctx, app.dark);
        for _ in 0..count {
            let output = ctx.run_ui(RawInput::default(), |ui| app.draw(ui));
            // Pas de fenêtre pour recevoir les textures : on les abandonne explicitement.
            output.drop_without_applying_deltas();
        }
    }

    /// Dessine l'application dans une fenêtre de `width` × `height` points et rend la largeur réellement occupée.
    fn used_width_at(app: &mut App, width: f32, height: f32) -> f32 {
        let ctx = Context::default();
        style::install_fonts(&ctx);
        style::apply(&ctx, app.dark);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        for _ in 0..2 {
            let raw = RawInput {
                screen_rect: Some(screen),
                ..RawInput::default()
            };
            let output = ctx.run_ui(raw, |ui| app.draw(ui));
            output.drop_without_applying_deltas();
        }
        ctx.globally_used_rect().width()
    }

    fn demo_app() -> App {
        App::new(
            Paths::under(std::path::Path::new("/nonexistent")),
            &Options {
                demo: true,
                ..Options::default()
            },
        )
    }

    #[test]
    fn no_screen_overflows_the_window_from_a_tiled_width_to_a_wide_one() {
        for width in [640.0, 855.0, 1240.0, 1900.0] {
            for tab in Tab::ALL {
                let mut app = demo_app();
                app.tab = tab;
                let used = used_width_at(&mut app, width, 900.0);
                assert!(
                    used <= width + 1.0,
                    "{tab:?} occupe {used} points dans une fenêtre de {width}"
                );
            }
        }
    }

    #[test]
    fn every_screen_draws_on_the_demo_data_in_both_themes() {
        for light in [false, true] {
            let mut app = demo_app();
            app.dark = !light;
            for tab in Tab::ALL {
                app.tab = tab;
                for segment in 0..3 {
                    app.states.performance.segment = segment;
                    frames(&mut app, 2);
                }
            }
        }
    }

    #[test]
    fn every_screen_draws_on_a_filled_journal_and_on_an_empty_database() {
        let sample = sample();
        let mut filled = App::new(sample.paths.clone(), &Options::default());
        let dir = tempfile::tempdir().unwrap();
        let mut empty = App::new(Paths::under(dir.path()), &Options::default());

        for app in [&mut filled, &mut empty] {
            for tab in Tab::ALL {
                app.tab = tab;
                frames(app, 2);
            }
            assert!(app.data.is_some());
        }
    }

    #[test]
    fn the_drawers_open_prefilled_and_draw() {
        let sample = sample();
        let mut app = App::new(sample.paths.clone(), &Options::default());
        let ctx = Context::default();
        let id = app.data.as_ref().unwrap().recommendations[0].id;

        app.run(&ctx, Request::Follow(id), 0.0);
        frames(&mut app, 2);
        let Some(Drawer::Follow(form, _)) = &app.drawer else {
            panic!("le tiroir « J'ai parié » doit être ouvert");
        };
        assert_eq!(form.odds, "2,30");
        assert!(!form.stake.is_empty());

        app.run(&ctx, Request::NewOffModelBet, 0.0);
        frames(&mut app, 2);
        assert!(matches!(app.drawer, Some(Drawer::OffModel(..))));

        app.run(&ctx, Request::NewOffer("betclic_fr".into()), 0.0);
        frames(&mut app, 2);
        assert!(
            matches!(&app.drawer, Some(Drawer::Offer(form, _)) if form.bookmaker == "betclic_fr")
        );
    }

    #[test]
    fn the_bonus_drawer_opens_on_a_demo_candidate_and_draws() {
        let mut app = demo_app();
        let ctx = Context::default();
        let data = app.data.as_ref().unwrap();
        let plan = &data.bonus.plans[0];
        let form = BonusForm::from_candidate(&plan.target, &plan.candidates[0]);

        app.run(&ctx, Request::OpenBonus(form), 0.0);
        frames(&mut app, 2);

        let Some(Drawer::Bonus(form, _)) = &app.drawer else {
            panic!("le tiroir « J'ai joué » doit être ouvert");
        };
        assert_eq!(form.legs.len(), 3);
    }

    #[test]
    fn running_an_action_writes_the_journal_reloads_and_reports_the_outcome() {
        let sample = sample();
        let mut app = App::new(sample.paths.clone(), &Options::default());
        let ctx = Context::default();
        let action = FollowForm::for_recommendation(&app.data.as_ref().unwrap().recommendations[0])
            .to_action()
            .unwrap();

        app.run(&ctx, Request::Do(action.clone()), 1.0);

        assert_eq!(app.data.as_ref().unwrap().bets.len(), 1);
        assert!(app.data.as_ref().unwrap().curve.open_stakes > 0.0);
        assert!(
            app.toasts
                .iter()
                .any(|t| !t.failure && t.text.contains("enregistré"))
        );

        // Le même pari une seconde fois est refusé, et la notification d'erreur le dit.
        app.run(&ctx, Request::Do(action), 2.0);
        assert!(
            app.toasts
                .iter()
                .any(|t| t.failure && t.text.contains("déjà enregistré"))
        );
        assert_eq!(app.data.as_ref().unwrap().bets.len(), 1);
    }

    #[test]
    fn in_demo_mode_nothing_is_written_and_the_user_is_told() {
        let mut app = demo_app();
        let ctx = Context::default();

        app.run(&ctx, Request::Do(Action::SettleAuto), 1.0);

        assert!(app.toasts.iter().any(|t| t.text.contains("Mode démo")));
    }

    #[test]
    fn keys_one_to_six_are_requests_to_change_screen() {
        let app = demo_app();
        let ctx = Context::default();
        style::install_fonts(&ctx);
        style::apply(&ctx, app.dark);
        let mut raw = RawInput::default();
        raw.events.push(Event::Key {
            key: Key::Num3,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let mut requests = Vec::new();
        let output = ctx.run_ui(raw, |ui| requests = app.shortcuts(ui.ctx()));
        output.drop_without_applying_deltas();

        assert!(matches!(requests.as_slice(), [Request::Go(Tab::Bets)]));
    }
}
