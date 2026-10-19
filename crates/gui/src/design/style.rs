//! Applique la palette et l'échelle typographique à egui : fonds, contrôles, espacements, tailles de texte.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText,
    Shadow, Stroke, TextStyle, Theme, ThemePreference, Vec2, Visuals, style::ScrollStyle,
};

use super::tokens::{DARK, LIGHT, Palette, radius, type_scale};

/// Famille des textes en demi-gras (titres, chiffres, noms de match).
pub fn semibold_family() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Demi-gras pour un texte : la police par défaut d'egui n'a pas de graisse, `strong()` ne fait que changer la couleur.
pub trait Semibold {
    /// Ce texte en demi-gras.
    fn semibold(self) -> RichText;
}

impl Semibold for RichText {
    fn semibold(self) -> RichText {
        self.family(semibold_family())
    }
}

/// Installe la police **Inter** (SIL OFL 1.1, `assets/fonts/`) en tête des familles proportionnelle et demi-gras ; les polices
/// d'egui restent derrière pour les symboles qu'Inter n'a pas.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "inter-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
        proportional.insert(0, "inter".into());
    }
    let mut semibold = vec!["inter-semibold".to_string()];
    semibold.extend(fallbacks);
    fonts.families.insert(semibold_family(), semibold);
    ctx.set_fonts(fonts);
}

/// Nom de la police des grands chiffres et des titres d'écran.
pub fn display_style() -> TextStyle {
    TextStyle::Name("display".into())
}

/// Nom de la police des chiffres d'indicateur.
pub fn figure_style() -> TextStyle {
    TextStyle::Name("figure".into())
}

/// Choisit le thème et installe les styles des deux thèmes.
pub fn apply(ctx: &egui::Context, dark: bool) {
    ctx.set_theme(if dark {
        ThemePreference::Dark
    } else {
        ThemePreference::Light
    });
    for (theme, palette) in [(Theme::Dark, &DARK), (Theme::Light, &LIGHT)] {
        ctx.set_visuals_of(theme, visuals(palette));
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = Vec2::new(10.0, 10.0);
            style.spacing.button_padding = Vec2::new(14.0, 8.0);
            style.spacing.interact_size.y = 34.0;
            style.spacing.scroll = ScrollStyle::floating();
            style.spacing.window_margin = Margin::same(18);
            style.text_styles.insert(
                TextStyle::Heading,
                FontId::new(type_scale::TITLE, semibold_family()),
            );
            style.text_styles.insert(
                TextStyle::Body,
                FontId::new(type_scale::BODY, FontFamily::Proportional),
            );
            style.text_styles.insert(
                TextStyle::Button,
                FontId::new(type_scale::BODY, FontFamily::Proportional),
            );
            style.text_styles.insert(
                TextStyle::Small,
                FontId::new(type_scale::SMALL, FontFamily::Proportional),
            );
            style.text_styles.insert(
                TextStyle::Monospace,
                FontId::new(type_scale::BODY, FontFamily::Monospace),
            );
            style.text_styles.insert(
                display_style(),
                FontId::new(type_scale::DISPLAY, semibold_family()),
            );
            style.text_styles.insert(
                figure_style(),
                FontId::new(type_scale::FIGURE, semibold_family()),
            );
        });
    }
}

/// Les couleurs d'egui pour une palette.
fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let control = CornerRadius::same(radius::CONTROL);
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(radius::CARD);
    v.menu_corner_radius = control;
    v.extreme_bg_color = p.raised;
    v.faint_bg_color = p.surface;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_shadow = Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: p.shadow,
    };
    v.popup_shadow = v.window_shadow;

    v.widgets.noninteractive.bg_fill = p.surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.corner_radius = control;

    v.widgets.inactive.bg_fill = p.raised;
    v.widgets.inactive.weak_bg_fill = p.raised;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.corner_radius = control;

    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.corner_radius = control;

    v.widgets.active.bg_fill = p.accent_soft;
    v.widgets.active.weak_bg_fill = p.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.active.corner_radius = control;

    v.widgets.open = v.widgets.hovered;
    v
}
