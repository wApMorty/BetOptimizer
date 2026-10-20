//! Image PNG des recommandations du jour, jointe au message Discord : lisible d'un coup d'œil sur un téléphone.
//!
//! On écrit un SVG (des cartes, aux couleurs du site), puis `resvg` le convertit en PNG avec la police Inter embarquée : le
//! rendu ne dépend d'aucune police du système et reste identique sous Linux et Windows.

use std::fmt::Write;
use std::sync::Arc;

use resvg::{tiny_skia, usvg};

use super::recommend_report::Row;
use crate::error::{Error, Result};

/// Inter (SIL OFL 1.1, `assets/fonts/` à la racine du dépôt), lue à la compilation et rangée dans le binaire.
const INTER_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../../assets/fonts/Inter-SemiBold.ttf");

/// Largeur de l'image, en points ; le PNG est rendu au double pour rester net sur un écran dense.
const WIDTH: u32 = 720;
const SCALE: f32 = 2.0;
/// Hauteurs : en-tête, une carte (marge comprise), pied.
const HEADER: u32 = 92;
const CARD: u32 = 138;
const FOOTER: u32 = 70;
/// Au-delà, l'image renvoie au rapport complet plutôt que de s'allonger sans fin.
pub const MAX_PICKS: usize = 10;

/// Couleurs du thème sombre du site (`site_base.html`).
const BG: &str = "#161615";
const CARD_BG: &str = "#201f1e";
const LINE: &str = "#34332f";
const FG: &str = "#ececea";
const MUTED: &str = "#a3a39d";
const ACCENT: &str = "#7fb2ee";
const WARN: &str = "#f2c46b";

/// Ce que l'image affiche.
#[derive(Debug, Clone, Copy)]
pub struct Picks<'a> {
    /// Titre (`Recommandations du mardi 20 octobre`).
    pub title: &'a str,
    /// Ligne sous le titre (fraction de Kelly, unité).
    pub subtitle: &'a str,
    /// Recommandations, déjà mises en forme pour le rapport.
    pub rows: &'a [Row],
    /// Où trouver la suite si elles ne tiennent pas toutes (`sur le site`).
    pub more_at: &'a str,
}

/// Échappe un texte pour le placer dans du XML : un nom d'équipe contenant `&` ne doit pas casser le SVG.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Le SVG de l'image.
pub fn picks_svg(picks: &Picks<'_>) -> String {
    let shown = &picks.rows[..picks.rows.len().min(MAX_PICKS)];
    let height = HEADER + CARD * shown.len() as u32 + FOOTER;
    let mut svg = String::new();
    // `write!` sur une `String` ne peut pas échouer : son résultat est ignoré sans risque (`let _`).
    let _ = write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" font-family="Inter">"#
    );
    let _ = write!(
        svg,
        r#"<rect width="100%" height="100%" fill="{BG}"/><text x="24" y="44" font-size="26" font-weight="600" fill="{FG}">{}</text><text x="24" y="72" font-size="15" fill="{MUTED}">{}</text>"#,
        escape(picks.title),
        escape(picks.subtitle)
    );
    for (index, row) in shown.iter().enumerate() {
        card(&mut svg, HEADER + CARD * index as u32, row);
    }
    let footer = HEADER + CARD * shown.len() as u32;
    let hidden = picks.rows.len() - shown.len();
    let mut first =
        "Expérimental : probabilité de Pinnacle sans marge, aucune règle validée hors échantillon."
            .to_string();
    if hidden > 0 {
        first = format!("… et {hidden} autre(s) {}.", picks.more_at);
    }
    let _ = write!(
        svg,
        r#"<text x="24" y="{}" font-size="14" fill="{WARN}">{}</text><text x="24" y="{}" font-size="14" fill="{WARN}">Vérifie la cote sur le site de l'opérateur : sous la cote minimale, ne parie pas.</text></svg>"#,
        footer + 28,
        escape(&first),
        footer + 50
    );
    svg
}

/// Une carte : match, coup d'envoi et compétition, opérateur et cote, cote minimale et mise.
fn card(svg: &mut String, top: u32, row: &Row) {
    let _ = write!(
        svg,
        r#"<rect x="16" y="{top}" width="{}" height="{}" rx="12" fill="{CARD_BG}" stroke="{LINE}"/>"#,
        WIDTH - 32,
        CARD - 14
    );
    let _ = write!(
        svg,
        r#"<text x="32" y="{}" font-size="19" font-weight="600" fill="{FG}">{}</text>"#,
        top + 32,
        escape(&row.fixture)
    );
    // Le coup d'envoi sur la deuxième ligne, pas aligné à droite : un long nom de match ne peut pas le chevaucher.
    let _ = write!(
        svg,
        r#"<text x="32" y="{}" font-size="14" fill="{MUTED}">{} · {} · recommandation n° {}</text>"#,
        top + 55,
        escape(&row.kickoff),
        escape(&row.league),
        row.id
    );
    // `tspan` : un mot en demi-gras ou en couleur au milieu d'une ligne.
    let _ = write!(
        svg,
        r#"<text x="32" y="{}" font-size="17" fill="{FG}"><tspan font-weight="600">{}</tspan> · {} à <tspan font-weight="600">{}</tspan> · avantage <tspan font-weight="600">{}</tspan></text>"#,
        top + 84,
        escape(&row.bookmaker),
        escape(&row.outcome),
        escape(&row.odds),
        escape(&row.edge)
    );
    let _ = write!(
        svg,
        r#"<text x="32" y="{}" font-size="17" fill="{FG}">cote minimale <tspan font-weight="600">{}</tspan> · mise <tspan font-weight="600" fill="{ACCENT}">{}</tspan></text>"#,
        top + 110,
        escape(&row.minimal_odds),
        escape(&row.stake)
    );
}

/// Convertit un SVG en PNG, avec la police Inter embarquée comme seule police connue.
pub fn render_png(svg: &str) -> Result<Vec<u8>> {
    let mut fonts = usvg::fontdb::Database::new();
    // `to_vec` copie les octets (environ 400 Ko chacun) : la base de polices veut posséder ses données.
    fonts.load_font_data(INTER_REGULAR.to_vec());
    fonts.load_font_data(INTER_SEMIBOLD.to_vec());
    let options = usvg::Options {
        font_family: "Inter".to_string(),
        fontdb: Arc::new(fonts),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|e| Error::Data(format!("image des recommandations : SVG illisible ({e})")))?;
    let size =
        tree.size().to_int_size().scale_by(SCALE).ok_or_else(|| {
            Error::Data("image des recommandations : taille invalide".to_string())
        })?;
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| Error::Data("image des recommandations : taille invalide".to_string()))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|e| Error::Data(format!("image des recommandations : encodage PNG ({e})")))
}

/// L'image PNG des recommandations.
pub fn picks_png(picks: &Picks<'_>) -> Result<Vec<u8>> {
    render_png(&picks_svg(picks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, fixture: &str) -> Row {
        Row {
            id,
            kickoff: "sam. 24/10 16:00".into(),
            league: "Premiership écossaise".into(),
            fixture: fixture.into(),
            bookmaker: "betclic_fr".into(),
            outcome: "domicile".into(),
            odds: "2,30".into(),
            fair_odds: "2,06".into(),
            edge: "+11,9 %".into(),
            minimal_odds: "2,08".into(),
            stake: "9,1 u (91,4 €)".into(),
        }
    }

    fn picks(rows: &[Row]) -> Picks<'_> {
        Picks {
            title: "Recommandations du mardi 20 octobre",
            subtitle: "Kelly ×0,50 · 1 u = 1 % de la bankroll",
            rows,
            more_at: "sur le site",
        }
    }

    #[test]
    fn team_names_are_escaped_and_long_lists_point_to_the_full_report() {
        let rows: Vec<Row> = (0..12)
            .map(|i| row(i, "Brighton & Hove – <Wolves>"))
            .collect();

        let svg = picks_svg(&picks(&rows));

        assert!(svg.contains("Brighton &amp; Hove – &lt;Wolves&gt;"));
        assert_eq!(svg.matches("<rect x=\"16\"").count(), MAX_PICKS);
        assert!(svg.contains("… et 2 autre(s) sur le site."));
    }

    #[test]
    #[ignore = "aperçu manuel : EDGE_PICKS_PREVIEW=fichier.png cargo test -- --ignored preview"]
    fn preview() {
        let rows = [
            row(12, "Falkirk – Kilmarnock"),
            row(13, "Borussia Mönchengladbach – Bayer Leverkusen"),
        ];
        let path = std::env::var("EDGE_PICKS_PREVIEW").unwrap();
        std::fs::write(path, picks_png(&picks(&rows)).unwrap()).unwrap();
    }

    #[test]
    fn the_png_has_the_expected_size_and_signature() {
        let rows = [row(1, "Falkirk – Kilmarnock")];

        let png = picks_png(&picks(&rows)).unwrap();

        // Signature PNG, puis largeur et hauteur dans l'en-tête IHDR (octets 16 à 23, gros-boutiste).
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!(width, WIDTH * 2);
        assert_eq!(height, (HEADER + CARD + FOOTER) * 2);
    }

    #[test]
    fn the_embedded_font_is_found_so_text_is_actually_drawn() {
        // Même image avec et sans texte : si la police manquait, le texte serait ignoré et les deux PNG identiques.
        let with_text = render_png(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40"><text x="4" y="30" font-size="24" font-weight="600">Falkirk</text></svg>"#,
        )
        .unwrap();
        let without =
            render_png(r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40"></svg>"#)
                .unwrap();

        assert_ne!(with_text, without);
    }
}
