//! Icônes vectorielles dessinées au pinceau : aucune police d'icônes, aucun fichier image.
//!
//! Chaque icône vit dans un carré unité `[0, 1]²` que l'on place dans un rectangle : trait de 1,6 pt, extrémités arrondies.

use std::f32::consts::TAU;

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

/// Les icônes disponibles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Aujourd'hui (soleil).
    Today,
    /// Opportunités (éclair).
    Bolt,
    /// Paris (billet).
    Ticket,
    /// Performance (courbe).
    Chart,
    /// Rapports (page).
    Doc,
    /// Football.
    Football,
    /// Basket.
    Basketball,
    /// Tennis.
    Tennis,
    /// Thème sombre.
    Moon,
    /// Thème clair.
    Sun,
    /// Rafraîchir.
    Refresh,
    /// Information.
    Info,
    /// Fermer.
    Close,
    /// Validé.
    Check,
    /// Mouvement réduit (pause).
    Pause,
    /// Bonus (paquet cadeau).
    Gift,
}

/// Dessine une icône dans `rect`.
pub fn draw(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    let at = |x: f32, y: f32| {
        pos2(
            rect.min.x + x * rect.width(),
            rect.min.y + y * rect.height(),
        )
    };
    let line = |points: &[(f32, f32)], closed: bool| {
        let pts: Vec<Pos2> = points.iter().map(|(x, y)| at(*x, *y)).collect();
        painter.add(if closed {
            Shape::closed_line(pts, stroke)
        } else {
            Shape::line(pts, stroke)
        });
    };
    let radius = |r: f32| r * rect.width();
    match icon {
        // Une cible : le verdict du jour (le soleil est réservé au réglage du thème).
        Icon::Today => {
            painter.circle_stroke(at(0.5, 0.5), radius(0.4), stroke);
            painter.circle_stroke(at(0.5, 0.5), radius(0.22), stroke);
            painter.circle_filled(at(0.5, 0.5), radius(0.07), color);
        }
        Icon::Bolt => line(
            &[
                (0.58, 0.06),
                (0.28, 0.55),
                (0.48, 0.55),
                (0.42, 0.94),
                (0.72, 0.42),
                (0.52, 0.42),
            ],
            true,
        ),
        Icon::Ticket => {
            painter.rect_stroke(
                Rect::from_min_max(at(0.08, 0.26), at(0.92, 0.74)),
                3.0,
                stroke,
                eframe::egui::StrokeKind::Inside,
            );
            painter.line_segment([at(0.62, 0.34), at(0.62, 0.66)], Stroke::new(1.2, color));
        }
        Icon::Chart => {
            line(&[(0.12, 0.12), (0.12, 0.86), (0.9, 0.86)], false);
            line(
                &[(0.22, 0.7), (0.42, 0.48), (0.58, 0.58), (0.84, 0.22)],
                false,
            );
        }
        Icon::Doc => {
            painter.rect_stroke(
                Rect::from_min_max(at(0.2, 0.08), at(0.8, 0.92)),
                3.0,
                stroke,
                eframe::egui::StrokeKind::Inside,
            );
            for y in [0.34, 0.5, 0.66] {
                painter.line_segment([at(0.32, y), at(0.68, y)], Stroke::new(1.2, color));
            }
        }
        Icon::Football => {
            painter.circle_stroke(at(0.5, 0.5), radius(0.42), stroke);
            let pentagon: Vec<Pos2> = (0..5)
                .map(|i| {
                    let a = i as f32 / 5.0 * TAU - TAU / 4.0;
                    at(0.5 + 0.16 * a.cos(), 0.5 + 0.16 * a.sin())
                })
                .collect();
            painter.add(Shape::convex_polygon(pentagon, color, Stroke::NONE));
            for i in 0..5 {
                let a = i as f32 / 5.0 * TAU - TAU / 4.0;
                painter.line_segment(
                    [
                        at(0.5 + 0.2 * a.cos(), 0.5 + 0.2 * a.sin()),
                        at(0.5 + 0.42 * a.cos(), 0.5 + 0.42 * a.sin()),
                    ],
                    Stroke::new(1.2, color),
                );
            }
        }
        Icon::Basketball => {
            painter.circle_stroke(at(0.5, 0.5), radius(0.42), stroke);
            painter.line_segment([at(0.5, 0.08), at(0.5, 0.92)], Stroke::new(1.2, color));
            painter.line_segment([at(0.08, 0.5), at(0.92, 0.5)], Stroke::new(1.2, color));
            line(&[(0.2, 0.2), (0.3, 0.38), (0.3, 0.62), (0.2, 0.8)], false);
            line(&[(0.8, 0.2), (0.7, 0.38), (0.7, 0.62), (0.8, 0.8)], false);
        }
        Icon::Tennis => {
            painter.circle_stroke(at(0.5, 0.5), radius(0.42), stroke);
            line(
                &[(0.18, 0.22), (0.34, 0.4), (0.34, 0.6), (0.18, 0.78)],
                false,
            );
            line(
                &[(0.82, 0.22), (0.66, 0.4), (0.66, 0.6), (0.82, 0.78)],
                false,
            );
        }
        Icon::Moon => {
            // Croissant : l'arc extérieur d'un grand cercle, puis l'arc intérieur d'un cercle décalé qui le « mord ».
            let (outer_center, outer_r) = ((0.5_f32, 0.5_f32), 0.38_f32);
            let (inner_center, inner_r) = ((0.66_f32, 0.46_f32), 0.31_f32);
            let distance =
                |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
            let ring = |center: (f32, f32), r: f32| -> Vec<(f32, f32)> {
                (0..=60)
                    .map(|i| {
                        let a = i as f32 / 60.0 * TAU;
                        (center.0 + r * a.cos(), center.1 - r * a.sin())
                    })
                    .collect()
            };
            let mut outline: Vec<(f32, f32)> = ring(outer_center, outer_r)
                .into_iter()
                .filter(|p| distance(*p, inner_center) > inner_r)
                .collect();
            let mut inner: Vec<(f32, f32)> = ring(inner_center, inner_r)
                .into_iter()
                .filter(|p| distance(*p, outer_center) < outer_r)
                .collect();
            inner.reverse();
            outline.extend(inner);
            line(&outline, true);
        }
        Icon::Sun => {
            painter.circle_stroke(at(0.5, 0.5), radius(0.18), stroke);
            for step in 0..8 {
                let angle = step as f32 / 8.0 * TAU;
                let (c, s) = (angle.cos(), angle.sin());
                painter.line_segment(
                    [
                        at(0.5 + 0.3 * c, 0.5 + 0.3 * s),
                        at(0.5 + 0.42 * c, 0.5 + 0.42 * s),
                    ],
                    stroke,
                );
            }
        }
        Icon::Refresh => {
            let points: Vec<(f32, f32)> = (0..=18)
                .map(|i| {
                    let a = -0.6 + i as f32 / 18.0 * (TAU - 1.4);
                    (0.5 + 0.36 * a.cos(), 0.5 + 0.36 * a.sin())
                })
                .collect();
            line(&points, false);
            line(&[(0.88, 0.14), (0.88, 0.38), (0.64, 0.38)], false);
        }
        Icon::Info => {
            painter.circle_stroke(at(0.5, 0.5), radius(0.42), stroke);
            painter.circle_filled(at(0.5, 0.32), radius(0.05), color);
            painter.line_segment([at(0.5, 0.46), at(0.5, 0.72)], stroke);
        }
        Icon::Close => {
            painter.line_segment([at(0.22, 0.22), at(0.78, 0.78)], stroke);
            painter.line_segment([at(0.78, 0.22), at(0.22, 0.78)], stroke);
        }
        Icon::Check => line(&[(0.18, 0.54), (0.4, 0.74), (0.82, 0.28)], false),
        Icon::Pause => {
            painter.line_segment([at(0.34, 0.2), at(0.34, 0.8)], Stroke::new(2.4, color));
            painter.line_segment([at(0.66, 0.2), at(0.66, 0.8)], Stroke::new(2.4, color));
        }
        Icon::Gift => {
            // Couvercle, boîte, ruban vertical et deux boucles.
            for (min, max) in [((0.1, 0.3), (0.9, 0.44)), ((0.16, 0.44), (0.84, 0.88))] {
                painter.rect_stroke(
                    Rect::from_min_max(at(min.0, min.1), at(max.0, max.1)),
                    2.0,
                    stroke,
                    eframe::egui::StrokeKind::Inside,
                );
            }
            painter.line_segment([at(0.5, 0.3), at(0.5, 0.88)], stroke);
            line(&[(0.5, 0.3), (0.3, 0.12), (0.24, 0.24), (0.5, 0.3)], false);
            line(&[(0.5, 0.3), (0.7, 0.12), (0.76, 0.24), (0.5, 0.3)], false);
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{Context, RawInput, vec2};

    use super::*;

    #[test]
    fn every_icon_draws_without_panicking() {
        let ctx = Context::default();
        let output = ctx.run_ui(RawInput::default(), |ui| {
            let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(24.0, 24.0));
            for icon in [
                Icon::Today,
                Icon::Bolt,
                Icon::Ticket,
                Icon::Chart,
                Icon::Doc,
                Icon::Football,
                Icon::Basketball,
                Icon::Tennis,
                Icon::Moon,
                Icon::Sun,
                Icon::Refresh,
                Icon::Info,
                Icon::Close,
                Icon::Check,
                Icon::Pause,
                Icon::Gift,
            ] {
                draw(ui.painter(), rect, icon, Color32::WHITE);
            }
        });
        output.drop_without_applying_deltas();
    }
}
