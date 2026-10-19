//! Jetons de design : palettes, espacements, rayons et échelle typographique.
//!
//! Les neutres sont **teintés** de bleu (un gris pur paraît mort), le texte secondaire est atténué plutôt que rapetissé, et une
//! seule couleur d'accent sert à guider le regard ; le vert, le rouge et l'ambre ne servent qu'à dire gain, perte et prudence.
//! Contrastes visés : 4,5:1 pour le texte courant, 3:1 pour les grands titres.

use eframe::egui::Color32;

/// Une palette complète (sombre ou claire).
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Palette sombre ?
    pub dark: bool,
    /// Fond de l'application.
    pub bg: Color32,
    /// Fond d'une carte.
    pub surface: Color32,
    /// Fond d'un élément posé sur une carte (champ, pastille neutre).
    pub raised: Color32,
    /// Fond au survol.
    pub hover: Color32,
    /// Filet très discret.
    pub border: Color32,
    /// Texte principal.
    pub text: Color32,
    /// Texte secondaire.
    pub muted: Color32,
    /// Texte tertiaire (légendes).
    pub faint: Color32,
    /// Accent (actions, sélection).
    pub accent: Color32,
    /// Accent translucide (fonds de pastille).
    pub accent_soft: Color32,
    /// Texte posé sur l'accent.
    pub on_accent: Color32,
    /// Gain, CLV positif.
    pub positive: Color32,
    /// Fond translucide du gain.
    pub positive_soft: Color32,
    /// Perte, CLV négatif.
    pub negative: Color32,
    /// Fond translucide de la perte.
    pub negative_soft: Color32,
    /// Prudence.
    pub warning: Color32,
    /// Fond translucide de la prudence.
    pub warning_soft: Color32,
    /// Couleur des ombres.
    pub shadow: Color32,
}

/// Palette sombre « ardoise ».
pub const DARK: Palette = Palette {
    dark: true,
    bg: Color32::from_rgb(13, 16, 23),
    surface: Color32::from_rgb(20, 25, 35),
    raised: Color32::from_rgb(28, 35, 50),
    hover: Color32::from_rgb(35, 44, 62),
    border: Color32::from_rgb(43, 53, 72),
    text: Color32::from_rgb(232, 236, 244),
    muted: Color32::from_rgb(154, 164, 184),
    faint: Color32::from_rgb(132, 143, 169),
    accent: Color32::from_rgb(124, 141, 255),
    accent_soft: Color32::from_rgba_unmultiplied_const(124, 141, 255, 38),
    on_accent: Color32::from_rgb(11, 14, 24),
    positive: Color32::from_rgb(75, 214, 160),
    positive_soft: Color32::from_rgba_unmultiplied_const(75, 214, 160, 34),
    negative: Color32::from_rgb(255, 124, 138),
    negative_soft: Color32::from_rgba_unmultiplied_const(255, 124, 138, 34),
    warning: Color32::from_rgb(242, 198, 96),
    warning_soft: Color32::from_rgba_unmultiplied_const(242, 198, 96, 34),
    shadow: Color32::from_rgba_unmultiplied_const(0, 0, 0, 120),
};

/// Palette claire « papier ».
pub const LIGHT: Palette = Palette {
    dark: false,
    bg: Color32::from_rgb(244, 245, 249),
    surface: Color32::from_rgb(255, 255, 255),
    raised: Color32::from_rgb(238, 240, 246),
    hover: Color32::from_rgb(230, 233, 242),
    border: Color32::from_rgb(225, 228, 236),
    text: Color32::from_rgb(21, 26, 38),
    muted: Color32::from_rgb(86, 96, 121),
    faint: Color32::from_rgb(98, 108, 134),
    accent: Color32::from_rgb(70, 86, 214),
    accent_soft: Color32::from_rgba_unmultiplied_const(70, 86, 214, 28),
    on_accent: Color32::from_rgb(255, 255, 255),
    positive: Color32::from_rgb(19, 122, 75),
    positive_soft: Color32::from_rgba_unmultiplied_const(19, 122, 75, 26),
    negative: Color32::from_rgb(192, 51, 74),
    negative_soft: Color32::from_rgba_unmultiplied_const(192, 51, 74, 26),
    warning: Color32::from_rgb(138, 90, 0),
    warning_soft: Color32::from_rgba_unmultiplied_const(138, 90, 0, 28),
    shadow: Color32::from_rgba_unmultiplied_const(30, 40, 80, 40),
};

/// Palette correspondant au mode du thème courant.
pub fn palette(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

/// Échelle d'espacement (points) : 4, 8, 12, 16, 24, 32.
pub mod space {
    /// 4 pt.
    pub const XS: f32 = 4.0;
    /// 8 pt.
    pub const SM: f32 = 8.0;
    /// 12 pt.
    pub const MD: f32 = 12.0;
    /// 16 pt.
    pub const LG: f32 = 16.0;
    /// 24 pt.
    pub const XL: f32 = 24.0;
    /// 32 pt.
    pub const XXL: f32 = 32.0;
}

/// Rayons de coin.
pub mod radius {
    /// Cartes.
    pub const CARD: u8 = 14;
    /// Boutons, champs.
    pub const CONTROL: u8 = 9;
    /// Pastilles.
    pub const PILL: u8 = 255;
}

/// Échelle typographique (points).
pub mod type_scale {
    /// Grand titre d'écran (verdict).
    pub const DISPLAY: f32 = 32.0;
    /// Chiffre d'un indicateur.
    pub const FIGURE: f32 = 28.0;
    /// Titre de section.
    pub const TITLE: f32 = 20.0;
    /// Texte courant.
    pub const BODY: f32 = 14.5;
    /// Légende.
    pub const SMALL: f32 = 12.5;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Luminance relative (WCAG 2.x) d'une couleur sRGB.
    /// L = 0,2126 R + 0,7152 G + 0,0722 B, où chaque canal c vaut c/12,92 si c ≤ 0,03928, sinon ((c + 0,055)/1,055)^2,4.
    fn luminance(color: Color32) -> f64 {
        let channel = |value: u8| {
            let c = f64::from(value) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    /// Rapport de contraste WCAG : (L clair + 0,05) / (L sombre + 0,05).
    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn every_text_colour_reaches_the_aa_contrast_on_every_surface_it_sits_on() {
        for p in [&DARK, &LIGHT] {
            for surface in [p.bg, p.surface, p.raised] {
                for (name, text) in [
                    ("text", p.text),
                    ("muted", p.muted),
                    ("faint", p.faint),
                    ("accent", p.accent),
                    ("positive", p.positive),
                    ("negative", p.negative),
                    ("warning", p.warning),
                ] {
                    let ratio = contrast(text, surface);
                    assert!(
                        ratio >= 4.5,
                        "{name} sur {surface:?} (thème sombre : {}) : {ratio:.2}",
                        p.dark
                    );
                }
            }
            let on_accent = contrast(p.on_accent, p.accent);
            assert!(
                on_accent >= 4.5,
                "texte du bouton principal : {on_accent:.2}"
            );
        }
    }
}
