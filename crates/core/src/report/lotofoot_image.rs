//! Image PNG de la séance Loto Foot, jointe au message Discord : un bulletin par grille, à recopier au point de vente.
//!
//! Chaque bulletin reprend la disposition du bulletin papier : une ligne par rencontre, et pour chaque grille à remplir
//! (simple ou multiple, regroupée par [`crate::lotofoot::bulletin::session_lines`]) les trois cases 1, N, 2. Les cases à
//! cocher sont noircies ; le prix de la grille est écrit dessous. Même principe que [`super::picks_image`] : un SVG écrit
//! ici, converti en PNG par `resvg` avec la police Inter embarquée.

use std::fmt::Write;

use chrono::{DateTime, Utc};

use super::picks_image::render_png;
use super::theme::LIGHT;
use crate::domain::Outcome;
use crate::error::Result;
use crate::format::kickoff_label;
use crate::lotofoot::Game;
use crate::lotofoot::bulletin::{Line, session_lines};
use crate::lotofoot::session::SessionReport;

/// Fond « papier » du bulletin et encre des cases cochées.
const PAPER: &str = "#fbf8f0";
const RULE: &str = "#d8d1bf";
/// Marge extérieure et largeur de la colonne des rencontres.
const MARGIN: u32 = 24;
const NAMES: u32 = 330;
/// Une case, l'écart entre deux cases, l'écart entre deux grilles.
const BOX: u32 = 24;
const GAP: u32 = 5;
const COLUMN_GAP: u32 = 22;
/// Hauteurs : en-tête de l'image, bandeau d'un bulletin, en-tête des colonnes, une rencontre, pied d'un bulletin.
const HEADER: u32 = 88;
const BAND: u32 = 46;
const COLUMN_HEAD: u32 = 30;
const ROW: u32 = 32;
const FOOT: u32 = 40;
/// Écart entre deux bulletins, pied de l'image.
const BETWEEN: u32 = 22;
const FOOTER: u32 = 54;
/// Largeur minimale de l'image.
const MIN_WIDTH: u32 = 720;

/// Un bulletin : une grille du Loto Foot et les grilles à y remplir.
#[derive(Debug, Clone, PartialEq)]
pub struct Bulletin {
    /// Formule et numéro.
    pub grid: (Game, u32),
    /// Fin de validation.
    pub closes_at: DateTime<Utc>,
    /// Rencontres, dans l'ordre du bulletin.
    pub matches: Vec<(String, String)>,
    /// Grilles à remplir, simples ou multiples.
    pub lines: Vec<Line>,
}

/// Les bulletins d'une séance, dans l'ordre des grilles de la séance. Vide s'il n'y a rien à jouer.
pub fn bulletins(report: &SessionReport) -> Vec<Bulletin> {
    let tickets: Vec<_> = report.chosen.iter().map(|c| (c.grid, &c.ticket)).collect();
    let lines = session_lines(&tickets);
    report
        .evaluated
        .iter()
        .filter_map(|grid| {
            let key = (grid.game, grid.number);
            let own: Vec<Line> = lines
                .iter()
                .filter(|l| l.grid == key)
                .map(|l| l.line.clone())
                .collect();
            (!own.is_empty()).then(|| Bulletin {
                grid: key,
                closes_at: grid.closes_at,
                matches: grid.matches.clone(),
                lines: own,
            })
        })
        .collect()
}

/// Échappe un texte pour le placer dans du SVG.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Largeur d'une colonne de grille : trois cases et leurs écarts.
fn column_width() -> u32 {
    3 * BOX + 2 * GAP
}

/// Largeur de l'image : la colonne des noms et les grilles du bulletin qui en a le plus.
fn width(bulletins: &[Bulletin]) -> u32 {
    let columns = bulletins.iter().map(|b| b.lines.len()).max().unwrap_or(1) as u32;
    (2 * MARGIN + 16 + NAMES + columns * (column_width() + COLUMN_GAP)).max(MIN_WIDTH)
}

/// Hauteur d'un bulletin.
fn bulletin_height(bulletin: &Bulletin) -> u32 {
    BAND + COLUMN_HEAD + ROW * bulletin.matches.len() as u32 + FOOT
}

/// Le SVG de la séance : titre, sous-titre, un bulletin par grille.
pub fn session_svg(title: &str, subtitle: &str, bulletins: &[Bulletin]) -> String {
    let p = &LIGHT;
    let width = width(bulletins);
    let height = HEADER
        + bulletins.iter().map(bulletin_height).sum::<u32>()
        + BETWEEN * bulletins.len().saturating_sub(1) as u32
        + FOOTER;
    let mut svg = String::new();
    // `write!` sur une `String` ne peut pas échouer : son résultat est ignoré sans risque (`let _`).
    let _ = write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" font-family="Inter"><rect width="100%" height="100%" fill="{}"/><text x="{MARGIN}" y="44" font-size="26" font-weight="600" fill="{}">{}</text><text x="{MARGIN}" y="72" font-size="15" fill="{}">{}</text>"#,
        p.bg.hex(),
        p.text.hex(),
        escape(title),
        p.muted.hex(),
        escape(subtitle)
    );
    let mut top = HEADER;
    for bulletin in bulletins {
        draw_bulletin(&mut svg, top, width, bulletin);
        top += bulletin_height(bulletin) + BETWEEN;
    }
    let _ = write!(
        svg,
        r#"<text x="{MARGIN}" y="{}" font-size="14" fill="{}">Coche les cases foncées, une grille par colonne. Puis, dans l'application : « J'ai joué la séance ».</text></svg>"#,
        height - 22,
        p.muted.hex()
    );
    svg
}

/// Un bulletin : bandeau, en-têtes des grilles, une ligne par rencontre, prix des grilles.
fn draw_bulletin(svg: &mut String, top: u32, width: u32, bulletin: &Bulletin) {
    let p = &LIGHT;
    let (left, right) = (MARGIN, width - MARGIN);
    let height = bulletin_height(bulletin);
    let ink = p.text.hex();
    // Le ticket : papier, coins arrondis, bandeau coloré en haut.
    let _ = write!(
        svg,
        r#"<rect x="{left}" y="{top}" width="{}" height="{height}" rx="14" fill="{PAPER}" stroke="{RULE}"/><path d="M{left} {} a14 14 0 0 1 14 -14 h{} a14 14 0 0 1 14 14 v{} h-{} z" fill="{}"/>"#,
        right - left,
        top + 14,
        right - left - 28,
        BAND - 14,
        right - left,
        p.accent.hex()
    );
    let (game, number) = bulletin.grid;
    let formula = match game {
        Game::Lf7 => "LOTO FOOT 7",
        Game::Lf8 => "LOTO FOOT 8",
    };
    let _ = write!(
        svg,
        r#"<text x="{}" y="{}" font-size="19" font-weight="600" fill="{}" letter-spacing="1.5">{formula}</text><text x="{}" y="{}" font-size="15" fill="{}" text-anchor="end">grille n°{number} · clôture {}</text>"#,
        left + 18,
        top + 30,
        p.on_accent.hex(),
        right - 18,
        top + 30,
        p.on_accent.hex(),
        escape(&kickoff_label(bulletin.closes_at))
    );
    // Encoches de part et d'autre, sous le bandeau, et pointillés de découpe : l'allure d'un ticket.
    let notch_y = top + BAND;
    let _ = write!(
        svg,
        r#"<circle cx="{left}" cy="{notch_y}" r="9" fill="{bg}"/><circle cx="{right}" cy="{notch_y}" r="9" fill="{bg}"/><line x1="{}" x2="{}" y1="{y}" y2="{y}" stroke="{RULE}" stroke-dasharray="5 5"/>"#,
        left + 14,
        right - 14,
        bg = p.bg.hex(),
        y = top + height - FOOT + 4
    );

    let first_column = left + 16 + NAMES;
    let column_left = |index: usize| first_column + index as u32 * (column_width() + COLUMN_GAP);
    let head_y = top + BAND + 22;
    for (index, _) in bulletin.lines.iter().enumerate() {
        let x = column_left(index);
        // Les grilles sont nommées A, B, C… dans l'ordre du regroupement.
        let label = char::from(b'A' + (index % 26) as u8);
        let _ = write!(
            svg,
            r#"<text x="{}" y="{head_y}" font-size="13" font-weight="600" fill="{}" text-anchor="middle">Grille {label}</text>"#,
            x + column_width() / 2,
            p.muted.hex()
        );
    }
    for (row, (home, away)) in bulletin.matches.iter().enumerate() {
        let y = top + BAND + COLUMN_HEAD + ROW * row as u32;
        if row % 2 == 1 {
            let _ = write!(
                svg,
                r#"<rect x="{}" y="{}" width="{}" height="{ROW}" fill="{RULE}" fill-opacity="0.25"/>"#,
                left + 1,
                y,
                right - left - 2
            );
        }
        let _ = write!(
            svg,
            r#"<text x="{}" y="{}" font-size="15" fill="{ink}"><tspan font-weight="600">{}.</tspan> {} – {}</text>"#,
            left + 18,
            y + 21,
            row + 1,
            escape(home),
            escape(away)
        );
        for (index, line) in bulletin.lines.iter().enumerate() {
            let chosen = line.signs().get(row).map_or(&[][..], Vec::as_slice);
            for (k, (sign, label)) in [
                (Outcome::Home, "1"),
                (Outcome::Draw, "N"),
                (Outcome::Away, "2"),
            ]
            .into_iter()
            .enumerate()
            {
                let x = column_left(index) + k as u32 * (BOX + GAP);
                let checked = chosen.contains(&sign);
                let (fill, text) = if checked {
                    (ink.clone(), PAPER.to_string())
                } else {
                    ("none".to_string(), p.faint.hex())
                };
                let _ = write!(
                    svg,
                    r#"<rect x="{x}" y="{}" width="{BOX}" height="{BOX}" rx="4" fill="{fill}" stroke="{}"/><text x="{}" y="{}" font-size="13" font-weight="600" fill="{text}" text-anchor="middle">{label}</text>"#,
                    y + 4,
                    if checked {
                        ink.clone()
                    } else {
                        RULE.to_string()
                    },
                    x + BOX / 2,
                    y + 21
                );
            }
        }
    }
    let foot_y = top + height - FOOT + 27;
    let _ = write!(
        svg,
        r#"<text x="{}" y="{foot_y}" font-size="14" fill="{}">{} grille(s) · {} €</text>"#,
        left + 18,
        p.muted.hex(),
        bulletin.lines.len(),
        bulletin.lines.iter().map(Line::combinations).sum::<usize>()
    );
    for (index, line) in bulletin.lines.iter().enumerate() {
        let _ = write!(
            svg,
            r#"<text x="{}" y="{foot_y}" font-size="14" font-weight="600" fill="{ink}" text-anchor="middle">{} €</text>"#,
            column_left(index) + column_width() / 2,
            line.combinations()
        );
    }
}

/// L'image PNG de la séance ; `None` s'il n'y a rien à jouer.
pub fn session_png(report: &SessionReport, title: &str) -> Result<Option<Vec<u8>>> {
    let bulletins = bulletins(report);
    if bulletins.is_empty() {
        return Ok(None);
    }
    let tickets = report.chosen.len();
    let grids: usize = bulletins.iter().map(|b| b.lines.len()).sum();
    let subtitle = format!(
        "{tickets} ticket(s) à 1 € ({tickets} €) en {grids} grille(s) à remplir · à valider avant la clôture"
    );
    render_png(&session_svg(title, &subtitle, &bulletins)).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lotofoot::Ticket;

    fn bulletin(lines: &[&str]) -> Bulletin {
        Bulletin {
            grid: (Game::Lf7, 133),
            closes_at: "2026-10-24T12:00:00Z".parse().unwrap(),
            matches: (1..=7)
                .map(|i| (format!("Équipe {i} & Cie"), format!("Visiteur {i}")))
                .collect(),
            lines: lines
                .iter()
                .map(|code| Line::single(&Ticket::parse(code).unwrap()))
                .collect(),
        }
    }

    #[test]
    fn each_line_checks_exactly_one_box_per_match_for_a_simple_grid() {
        let svg = session_svg(
            "Loto Foot",
            "2 tickets",
            &[bulletin(&["1N21N21", "2N21N21"])],
        );
        // Deux grilles simples de sept rencontres : quatorze cases noircies, sur 2 × 7 × 3 = 42.
        assert_eq!(svg.matches("width=\"24\" height=\"24\"").count(), 42);
        assert_eq!(
            svg.matches(&format!("fill=\"{}\" stroke", LIGHT.text.hex()))
                .count(),
            14
        );
        assert!(svg.contains("Grille A") && svg.contains("Grille B"));
        assert!(svg.contains("Équipe 1 &amp; Cie"));
        assert!(svg.contains("LOTO FOOT 7"));
        assert!(svg.contains("2 grille(s) · 2 €"));
    }

    #[test]
    fn the_image_widens_with_the_number_of_grids_and_renders_to_png() {
        let narrow = width(&[bulletin(&["1N21N21"])]);
        let wide = width(&[bulletin(&[
            "1111111", "2222222", "NNNNNNN", "1N21N21", "2N21N21", "N121212",
        ])]);
        assert_eq!(narrow, MIN_WIDTH);
        assert!(wide > narrow);
        let png = render_png(&session_svg(
            "Loto Foot",
            "1 ticket",
            &[bulletin(&["1N21N21"])],
        ))
        .unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
