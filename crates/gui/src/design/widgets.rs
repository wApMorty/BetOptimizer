//! Composants : cartes, pastilles, boutons, indicateurs, jauges, courbes miniatures, contrôle segmenté, tiroir.
//!
//! Chaque composant lit la palette du thème courant et anime ses changements d'état (survol, appui) en 120 à 180 ms.

use std::hash::Hash;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, InnerResponse,
    Layout, Margin, Mesh, Pos2, Rect, Response, RichText, Sense, Shadow, Shape, Stroke, StrokeKind,
    Ui, UiBuilder, Vec2, pos2, vec2,
};

use super::icons::{self, Icon};
use super::motion::Motion;
use super::style::{display_style, figure_style};
use super::tokens::{Palette, palette, radius, space};

/// Palette du thème courant.
pub fn pal(ui: &Ui) -> &'static Palette {
    palette(ui.visuals().dark_mode)
}

/// Ton d'une pastille ou d'un indicateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Neutre.
    Neutral,
    /// Accent.
    Accent,
    /// Gain.
    Positive,
    /// Perte.
    Negative,
    /// Prudence.
    Warning,
}

impl Tone {
    /// `(texte, fond translucide)` du ton.
    pub fn colors(self, p: &Palette) -> (Color32, Color32) {
        match self {
            Tone::Neutral => (p.muted, p.raised),
            Tone::Accent => (p.accent, p.accent_soft),
            Tone::Positive => (p.positive, p.positive_soft),
            Tone::Negative => (p.negative, p.negative_soft),
            Tone::Warning => (p.warning, p.warning_soft),
        }
    }

    /// Ton d'une grandeur signée : gain, perte ou neutre.
    pub fn of_sign(value: f64) -> Tone {
        if value > 0.0 {
            Tone::Positive
        } else if value < 0.0 {
            Tone::Negative
        } else {
            Tone::Neutral
        }
    }
}

/// Mélange deux couleurs (`t` = 0 : `a`, 1 : `b`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

/// Titre d'écran en grand.
pub fn display(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).text_style(display_style())
}

/// Chiffre d'indicateur.
pub fn figure(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).text_style(figure_style())
}

/// Titre de section : texte courant renforcé, suivi éventuellement d'une légende.
pub fn section_title(ui: &mut Ui, title: &str, hint: Option<&str>) {
    let p = pal(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).heading().color(p.text));
        if let Some(hint) = hint {
            ui.label(RichText::new(hint).small().color(p.faint));
        }
    });
}

/// Carte : fond relevé, filet discret, ombre douce. Survolable, elle s'éclaire et se soulève légèrement.
pub fn card<R>(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    hoverable: bool,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let p = pal(ui);
    let id = ui.make_persistent_id(Id::new(id));
    let ctx = ui.ctx().clone();
    // Le survol se mesure sur la zone de l'image précédente : le fond doit être choisi avant de connaître la hauteur du contenu.
    let previous: Option<Rect> = ctx.data(|d| d.get_temp(id));
    let hovered = hoverable
        && previous
            .zip(ctx.pointer_hover_pos())
            .is_some_and(|(rect, pos)| rect.contains(pos));
    let t = ctx.animate_bool_with_time(id.with("hover"), hovered, 0.14);

    let shadow = Shadow {
        offset: [0, (3.0 + 3.0 * t) as i8],
        blur: (14.0 + 10.0 * t) as u8,
        spread: 0,
        color: p.shadow.gamma_multiply(0.5 + 0.5 * t),
    };
    let frame = Frame::new()
        .fill(mix(p.surface, p.raised, t * 0.6))
        .stroke(Stroke::new(1.0, mix(p.border, p.accent, t * 0.55)))
        .corner_radius(CornerRadius::same(radius::CARD))
        .inner_margin(Margin::same(16))
        .shadow(shadow);
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        add_contents(ui)
    });
    ctx.data_mut(|d| d.insert_temp(id, inner.response.rect));
    inner
}

/// Largeur en dessous de laquelle trois tuiles côte à côte sont trop étroites pour leur grand chiffre.
const TILES_STACK_BELOW: f32 = 620.0;

/// Nombre de colonnes d'une rangée de trois tuiles : trois, ou une seule (les tuiles s'empilent) si la fenêtre est étroite.
///
/// À utiliser avec `ui.columns(n, …)` et l'indice `i % n` pour la i-ième tuile.
pub fn tile_columns(ui: &Ui) -> usize {
    if ui.available_width() < TILES_STACK_BELOW {
        1
    } else {
        3
    }
}

/// Largeur en dessous de laquelle le bloc de gauche de [`split`] n'est plus lisible.
const SPLIT_MIN_LEFT: f32 = 240.0;

/// Un bloc à gauche, un bloc de largeur fixe aligné à droite ; si la place manque, le second passe dessous.
///
/// Remplace `with_layout(right_to_left)`, qui donne tout l'espace au bloc de droite et laisse celui de gauche déborder
/// dessus dès que la fenêtre est étroite (cas d'une fenêtre tuilée).
pub fn split(
    ui: &mut Ui,
    right_width: f32,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui),
) {
    let full = ui.available_rect_before_wrap();
    let gap = space::MD;
    if full.width() < right_width + gap + SPLIT_MIN_LEFT {
        ui.vertical(|ui| {
            left(ui);
            ui.add_space(space::SM);
            right(ui);
        });
        return;
    }
    let left_rect = Rect::from_min_max(full.min, pos2(full.max.x - right_width - gap, full.max.y));
    let right_rect = Rect::from_min_max(pos2(full.max.x - right_width, full.min.y), full.max);
    let mut left_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("split-left")
            .max_rect(left_rect)
            .layout(Layout::top_down(Align::LEFT)),
    );
    left(&mut left_ui);
    let mut right_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("split-right")
            .max_rect(right_rect)
            .layout(Layout::top_down(Align::RIGHT)),
    );
    right(&mut right_ui);
    ui.allocate_rect(
        left_ui.min_rect().union(right_ui.min_rect()),
        Sense::hover(),
    );
}

/// Pastille de texte.
pub fn chip(ui: &mut Ui, text: &str, tone: Tone) -> Response {
    let p = pal(ui);
    let (fg, bg) = tone.colors(p);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(12.5), fg);
    let size = galley.size() + vec2(18.0, 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::PILL), bg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    response
}

/// Petit point de couleur à `center`, qui pulse doucement s'il est « vivant » (donnée fraîche).
fn paint_status_dot(ui: &Ui, center: Pos2, color: Color32, pulse: bool) {
    if pulse {
        let time = ui.input(|i| i.time) as f32;
        let wave = (time * 1.6).rem_euclid(2.4) / 2.4;
        ui.painter().circle_filled(
            center,
            4.0 + 4.0 * wave,
            color.gamma_multiply((1.0 - wave) * 0.35),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    ui.painter().circle_filled(center, 4.0, color);
}

/// Pastille avec un point d'état devant le texte.
///
/// Dessinée d'un seul tenant (et non avec un cadre qui contient un `horizontal`) : un `horizontal_wrapped` ne passe à la ligne
/// que devant un composant simple, pas devant un conteneur déjà tracé qui déborderait.
pub fn status_chip(ui: &mut Ui, text: &str, tone: Tone, pulse: bool) -> Response {
    let p = pal(ui);
    let (fg, bg) = tone.colors(p);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(14.0), fg);
    let size = vec2(
        galley.size().x + 12.0 + 8.0 + 12.0 + 12.0,
        galley.size().y + 14.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::PILL), bg);
    paint_status_dot(ui, pos2(rect.left() + 18.0, rect.center().y), fg, pulse);
    let text_at = pos2(
        rect.left() + 12.0 + 12.0 + 8.0,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(text_at, galley, fg);
    response
}

/// Genre de bouton.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Action principale : fond d'accent.
    Primary,
    /// Action secondaire : fond relevé.
    Secondary,
    /// Action discrète : pas de fond au repos.
    Ghost,
}

/// Bouton à hauteur fixe, dont la couleur répond au survol et à l'appui.
pub fn button(ui: &mut Ui, text: &str, kind: ButtonKind) -> Response {
    let p = pal(ui);
    let text_color = match kind {
        ButtonKind::Primary => p.on_accent,
        ButtonKind::Secondary | ButtonKind::Ghost => p.text,
    };
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), FontId::proportional(14.0), text_color);
    let size = vec2(galley.size().x + 28.0, 34.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let id = response.id;
    let hover = ui
        .ctx()
        .animate_bool_with_time(id.with("h"), response.hovered(), 0.12);
    let press =
        ui.ctx()
            .animate_bool_with_time(id.with("p"), response.is_pointer_button_down_on(), 0.08);
    let (rest, lit) = match kind {
        ButtonKind::Primary => (p.accent, mix(p.accent, Color32::WHITE, 0.18)),
        ButtonKind::Secondary => (p.raised, p.hover),
        ButtonKind::Ghost => (Color32::TRANSPARENT, p.raised),
    };
    // L'appui rétrécit légèrement le bouton : un retour tactile, sans changer la mise en page.
    let shrunk = rect.shrink(1.5 * press);
    ui.painter().rect_filled(
        shrunk,
        CornerRadius::same(radius::CONTROL),
        mix(rest, lit, hover),
    );
    if kind == ButtonKind::Secondary {
        ui.painter().rect_stroke(
            shrunk,
            CornerRadius::same(radius::CONTROL),
            Stroke::new(1.0, mix(p.border, p.accent, hover * 0.5)),
            StrokeKind::Inside,
        );
    }
    ui.painter()
        .galley(shrunk.center() - galley.size() / 2.0, galley, text_color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// Bouton carré portant une icône.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    let p = pal(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let hover = ui
        .ctx()
        .animate_bool_with_time(response.id.with("h"), response.hovered(), 0.12);
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(radius::CONTROL),
        mix(Color32::TRANSPARENT, p.raised, hover),
    );
    icons::draw(
        ui.painter(),
        Rect::from_center_size(rect.center(), vec2(18.0, 18.0)),
        icon,
        mix(p.muted, p.text, hover),
    );
    response
        .on_hover_text(tooltip)
        .on_hover_cursor(CursorIcon::PointingHand)
}

/// Hauteur minimale du contenu d'une tuile d'indicateur.
const STAT_TILE_HEIGHT: f32 = 128.0;

/// Indicateur : intitulé, grand chiffre, légende. `figure` est déjà formaté (un chiffre qui défile est formaté par l'appelant).
pub fn stat_tile<R>(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    label: &str,
    figure_text: String,
    tone: Tone,
    footer: impl FnOnce(&mut Ui) -> R,
) -> R {
    let p = pal(ui);
    let (fg, _) = tone.colors(p);
    let color = if tone == Tone::Neutral { p.text } else { fg };
    card(ui, id, true, |ui| {
        // Même hauteur pour toutes les tuiles d'une rangée, qu'une légende passe sur une ou deux lignes.
        ui.set_min_height(STAT_TILE_HEIGHT);
        ui.label(RichText::new(label).small().color(p.muted));
        ui.add_space(space::XS);
        ui.label(figure(figure_text).color(color));
        ui.add_space(space::SM);
        footer(ui)
    })
    .inner
}

/// Jauge horizontale : `fraction` entre 0 et 1, remplissage animé par `progress`.
pub fn gauge(ui: &mut Ui, fraction: f32, progress: f32, tone: Tone) {
    let p = pal(ui);
    let (fg, _) = tone.colors(p);
    let color = if tone == Tone::Neutral { p.accent } else { fg };
    let width = ui.available_width().max(60.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::PILL), p.raised);
    let filled = Rect::from_min_size(
        rect.min,
        vec2(
            rect.width() * fraction.clamp(0.0, 1.0) * progress,
            rect.height(),
        ),
    );
    if filled.width() > 0.5 {
        ui.painter()
            .rect_filled(filled, CornerRadius::same(radius::PILL), color);
    }
}

/// Jauge d'avantage : 0 à `max`, avec un repère au seuil de notification.
pub fn edge_meter(ui: &mut Ui, edge: f64, threshold: f64, max: f64, progress: f32, width: f32) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::PILL), p.raised);
    let fraction = (edge / max).clamp(0.0, 1.0) as f32 * progress;
    let color = if edge >= threshold {
        p.positive
    } else {
        p.warning
    };
    ui.painter().rect_filled(
        Rect::from_min_size(rect.min, vec2(rect.width() * fraction, rect.height())),
        CornerRadius::same(radius::PILL),
        color,
    );
    let tick_x = rect.min.x + rect.width() * (threshold / max).clamp(0.0, 1.0) as f32;
    ui.painter().line_segment(
        [
            pos2(tick_x, rect.min.y - 2.0),
            pos2(tick_x, rect.max.y + 2.0),
        ],
        Stroke::new(1.5, p.faint),
    );
}

/// Courbe miniature avec un dégradé dessous. `reveal` (0 à 1) trace la courbe de gauche à droite.
pub fn sparkline(ui: &mut Ui, values: &[f64], size: Vec2, tone: Tone, reveal: f32) {
    let p = pal(ui);
    let (fg, _) = tone.colors(p);
    let color = if tone == Tone::Neutral { p.accent } else { fg };
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if values.len() < 2 {
        ui.painter().line_segment(
            [rect.left_center(), rect.right_center()],
            Stroke::new(1.5, p.border),
        );
        return;
    }
    let (min, max) = values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    let span = (max - min).max(1e-9);
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.min.x + rect.width() * i as f32 / (values.len() - 1) as f32;
            let y = rect.max.y - 3.0 - (rect.height() - 6.0) * ((v - min) / span) as f32;
            pos2(x, y)
        })
        .collect();
    let visible = revealed_points(&points, reveal);
    if visible.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    for pair in visible.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(a, color.gamma_multiply(0.30));
        mesh.colored_vertex(b, color.gamma_multiply(0.30));
        mesh.colored_vertex(pos2(b.x, rect.max.y), color.gamma_multiply(0.0));
        mesh.colored_vertex(pos2(a.x, rect.max.y), color.gamma_multiply(0.0));
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    ui.painter().add(Shape::mesh(mesh));
    ui.painter()
        .add(Shape::line(visible.clone(), Stroke::new(2.0, color)));
    if let Some(last) = visible.last() {
        ui.painter().circle_filled(*last, 3.0, color);
    }
}

/// Les points visibles d'une courbe tracée à `reveal` (0 à 1) de sa longueur, le dernier segment étant interpolé.
pub fn revealed_points(points: &[Pos2], reveal: f32) -> Vec<Pos2> {
    if points.len() < 2 || reveal >= 1.0 {
        return points.to_vec();
    }
    let segments = (points.len() - 1) as f32;
    let position = reveal.clamp(0.0, 1.0) * segments;
    let whole = position.floor() as usize;
    let fraction = position - whole as f32;
    let mut visible: Vec<Pos2> = points[..=whole.min(points.len() - 1)].to_vec();
    if whole + 1 < points.len() {
        let (a, b) = (points[whole], points[whole + 1]);
        visible.push(a + (b - a) * fraction);
    }
    visible
}

/// Contrôle segmenté : une pastille glisse vers l'option choisie. Renvoie `true` si le choix a changé.
pub fn segmented(
    ui: &mut Ui,
    id: impl Hash + std::fmt::Debug,
    labels: &[&str],
    selected: &mut usize,
) -> bool {
    let p = pal(ui);
    let id = ui.make_persistent_id(Id::new(id));
    let galleys: Vec<_> = labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap((*label).to_string(), FontId::proportional(13.5), p.text)
        })
        .collect();
    let segment = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 28.0;
    let size = vec2(segment * labels.len() as f32 + 8.0, 38.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CONTROL + 2), p.raised);
    let animated = ui.ctx().animate_value_with_time(id, *selected as f32, 0.2);
    let pill = Rect::from_min_size(
        rect.min + vec2(4.0 + segment * animated, 4.0),
        vec2(segment, rect.height() - 8.0),
    );
    ui.painter()
        .rect_filled(pill, CornerRadius::same(radius::CONTROL), p.surface);
    ui.painter().rect_stroke(
        pill,
        CornerRadius::same(radius::CONTROL),
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    let mut changed = false;
    for (index, (label, galley)) in labels.iter().zip(galleys).enumerate() {
        let cell = Rect::from_min_size(
            rect.min + vec2(4.0 + segment * index as f32, 4.0),
            vec2(segment, rect.height() - 8.0),
        );
        let response = ui.interact(cell, id.with(index), Sense::click());
        let active = index == *selected;
        let color = if active || response.hovered() {
            p.text
        } else {
            p.muted
        };
        let colored =
            ui.painter()
                .layout_no_wrap((*label).to_string(), FontId::proportional(13.5), color);
        ui.painter()
            .galley(cell.center() - galley.size() / 2.0, colored, color);
        if response.clicked() && !active {
            *selected = index;
            changed = true;
        }
        response.on_hover_cursor(CursorIcon::PointingHand);
    }
    changed
}

/// Champ de saisie : l'intitulé en petit au-dessus, le champ sur toute la largeur.
pub fn field(ui: &mut Ui, label: &str, value: &mut String, hint: &str) {
    let p = pal(ui);
    ui.label(RichText::new(label).small().color(p.muted));
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(10, 8)),
    );
    ui.add_space(space::XS);
}

/// Écran vide : une icône, une phrase qui explique, une piste pour la suite.
pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, hint: &str) {
    let p = pal(ui);
    ui.vertical_centered(|ui| {
        ui.add_space(space::XL);
        let (rect, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        icons::draw(ui.painter(), rect, icon, p.faint);
        ui.add_space(space::SM);
        ui.label(RichText::new(title).heading().color(p.text));
        ui.label(RichText::new(hint).color(p.muted));
        ui.add_space(space::XL);
    });
}

/// Un cadre de chiffre lisible (police à chasse fixe) : cotes, mises.
pub fn mono(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).monospace()
}

/// Défilement de chiffre : de 0 vers `target`, selon la progression de l'écran.
pub fn counted(target: f64, motion: &Motion, delay: f32) -> f64 {
    target * f64::from(motion.progress(delay, super::motion::COUNT_SECS))
}

/// Tiroir latéral : un panneau qui glisse depuis la droite sur un voile, fermé par la croix, le voile ou Échap.
///
/// Renvoie `false` quand il doit se fermer.
pub fn drawer(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    width: f32,
    add_contents: impl FnOnce(&mut Ui),
) -> bool {
    let p = palette(ctx.theme() == egui::Theme::Dark);
    let mut open = true;
    let screen = ctx.content_rect();
    let appear = ctx.animate_bool_with_time(Id::new(id).with("appear"), true, 0.22);
    let eased = super::motion::ease_out(appear);

    // Voile : assombrit le fond et ferme au clic.
    egui::Area::new(Id::new(id).with("scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_black_alpha((150.0 * eased) as u8));
            if response.clicked() {
                open = false;
            }
        });

    let offset = width * (1.0 - eased);
    egui::Area::new(Id::new(id).with("panel"))
        .order(egui::Order::Tooltip)
        .anchor(Align2::RIGHT_TOP, vec2(offset, 0.0))
        .show(ctx, |ui| {
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .inner_margin(Margin::same(20))
                .shadow(Shadow {
                    offset: [-6, 0],
                    blur: 36,
                    spread: 0,
                    color: p.shadow,
                })
                .show(ui, |ui| {
                    ui.set_width(width - 42.0);
                    ui.set_min_height(screen.height() - 42.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).heading().color(p.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, Icon::Close, "Fermer (Échap)").clicked() {
                                open = false;
                            }
                        });
                    });
                    ui.add_space(space::MD);
                    add_contents(ui);
                });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curve_is_revealed_from_left_to_right_with_an_interpolated_tip() {
        let points = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(20.0, 10.0)];

        assert_eq!(revealed_points(&points, 1.0).len(), 3);
        let half = revealed_points(&points, 0.5);
        assert_eq!(half.len(), 3);
        assert_eq!(
            half[2],
            pos2(10.0, 0.0),
            "la pointe est au premier tiers de la dernière portion"
        );
        let quarter = revealed_points(&points, 0.25);
        assert_eq!(quarter.last().copied(), Some(pos2(5.0, 0.0)));
        assert_eq!(
            revealed_points(&points, 0.0),
            vec![pos2(0.0, 0.0), pos2(0.0, 0.0)]
        );
    }

    #[test]
    fn the_sign_of_a_value_picks_the_tone() {
        assert_eq!(Tone::of_sign(0.3), Tone::Positive);
        assert_eq!(Tone::of_sign(-0.3), Tone::Negative);
        assert_eq!(Tone::of_sign(0.0), Tone::Neutral);
    }

    #[test]
    fn mixing_colours_goes_from_the_first_to_the_second() {
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
        assert_eq!(
            mix(Color32::BLACK, Color32::WHITE, 5.0),
            Color32::WHITE,
            "t est borné"
        );
    }
}
