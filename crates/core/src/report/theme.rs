//! Palette commune au site, au rapport quotidien et aux images jointes à Discord : celle de l'ancienne application egui.
//!
//! Les neutres sont teintés de bleu, une seule couleur d'accent guide le regard ; le vert, le rouge et l'ambre ne disent que
//! gain, perte et prudence. Le site en tire des variables CSS ([`css_variables`]), les images SVG les couleurs elles-mêmes.

use std::fmt::Write;

/// Une couleur sRGB opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Forme CSS opaque : `#7c8dff`.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Forme CSS translucide : `rgba(124, 141, 255, 0.15)` (fonds de pastille).
    pub fn alpha(self, opacity: f64) -> String {
        format!("rgba({}, {}, {}, {opacity})", self.0, self.1, self.2)
    }
}

/// Une palette complète (sombre ou claire).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Fond de la page.
    pub bg: Rgb,
    /// Fond d'une carte.
    pub surface: Rgb,
    /// Fond d'un élément posé sur une carte (champ, pastille neutre).
    pub raised: Rgb,
    /// Fond au survol.
    pub hover: Rgb,
    /// Filet discret.
    pub border: Rgb,
    /// Texte principal.
    pub text: Rgb,
    /// Texte secondaire.
    pub muted: Rgb,
    /// Texte tertiaire (légendes).
    pub faint: Rgb,
    /// Accent (actions, sélection).
    pub accent: Rgb,
    /// Texte posé sur l'accent.
    pub on_accent: Rgb,
    /// Gain, CLV positif.
    pub positive: Rgb,
    /// Perte, CLV négatif.
    pub negative: Rgb,
    /// Prudence.
    pub warning: Rgb,
    /// Opacité des fonds translucides (pastilles, encadrés).
    pub soft: f64,
}

/// Palette sombre « ardoise ».
pub const DARK: Palette = Palette {
    bg: Rgb(13, 16, 23),
    surface: Rgb(20, 25, 35),
    raised: Rgb(28, 35, 50),
    hover: Rgb(35, 44, 62),
    border: Rgb(43, 53, 72),
    text: Rgb(232, 236, 244),
    muted: Rgb(154, 164, 184),
    faint: Rgb(132, 143, 169),
    accent: Rgb(124, 141, 255),
    on_accent: Rgb(11, 14, 24),
    positive: Rgb(75, 214, 160),
    negative: Rgb(255, 124, 138),
    warning: Rgb(242, 198, 96),
    soft: 0.14,
};

/// Palette claire « papier ».
pub const LIGHT: Palette = Palette {
    bg: Rgb(244, 245, 249),
    surface: Rgb(255, 255, 255),
    raised: Rgb(238, 240, 246),
    hover: Rgb(230, 233, 242),
    border: Rgb(225, 228, 236),
    text: Rgb(21, 26, 38),
    muted: Rgb(86, 96, 121),
    faint: Rgb(98, 108, 134),
    accent: Rgb(70, 86, 214),
    on_accent: Rgb(255, 255, 255),
    positive: Rgb(19, 122, 75),
    negative: Rgb(192, 51, 74),
    warning: Rgb(138, 90, 0),
    soft: 0.11,
};

/// Les variables CSS d'une palette, une déclaration par ligne (`--bg: #0d1017;`).
fn declarations(p: &Palette) -> String {
    let mut css = String::new();
    for (name, color) in [
        ("bg", p.bg),
        ("surface", p.surface),
        ("raised", p.raised),
        ("hover", p.hover),
        ("border", p.border),
        ("text", p.text),
        ("muted", p.muted),
        ("faint", p.faint),
        ("accent", p.accent),
        ("on-accent", p.on_accent),
        ("positive", p.positive),
        ("negative", p.negative),
        ("warning", p.warning),
    ] {
        // `write!` sur une `String` ne peut pas échouer : son résultat est ignoré sans risque (`let _`).
        let _ = write!(css, "--{name}: {}; ", color.hex());
    }
    for (name, color) in [
        ("accent", p.accent),
        ("positive", p.positive),
        ("negative", p.negative),
        ("warning", p.warning),
    ] {
        let _ = write!(css, "--{name}-soft: {}; ", color.alpha(p.soft));
    }
    css
}

/// Les variables CSS du site : palette claire par défaut, sombre si le système la demande.
pub fn css_variables() -> String {
    format!(
        ":root {{ color-scheme: light dark; {} }}\n@media (prefers-color-scheme: dark) {{ :root {{ {} }} }}",
        declarations(&LIGHT),
        declarations(&DARK)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Luminance relative (WCAG 2.x) d'une couleur sRGB.
    /// L = 0,2126 R + 0,7152 G + 0,0722 B, où chaque canal c vaut c/12,92 si c ≤ 0,03928, sinon ((c + 0,055)/1,055)^2,4.
    fn luminance(color: Rgb) -> f64 {
        let channel = |value: u8| {
            let c = f64::from(value) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
    }

    /// Rapport de contraste WCAG : (L clair + 0,05) / (L sombre + 0,05).
    fn contrast(a: Rgb, b: Rgb) -> f64 {
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
                    assert!(ratio >= 4.5, "{name} sur {surface:?} : {ratio:.2}");
                }
            }
            assert!(contrast(p.on_accent, p.accent) >= 4.5);
        }
    }

    #[test]
    fn the_css_declares_both_palettes_with_their_soft_backgrounds() {
        let css = css_variables();
        assert!(css.contains("--bg: #f4f5f9;"), "{css}");
        assert!(css.contains("prefers-color-scheme: dark"));
        assert!(css.contains("--bg: #0d1017;"));
        assert!(css.contains("--accent-soft: rgba(124, 141, 255, 0.14);"));
        assert_eq!(Rgb(124, 141, 255).hex(), "#7c8dff");
    }
}
