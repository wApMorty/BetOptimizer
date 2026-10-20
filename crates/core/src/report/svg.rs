//! Graphiques SVG (crate `plotters`, sans bibliothèque système : le texte est écrit en balises SVG).

use std::fmt::Display;

use plotters::prelude::*;

use crate::backtest::curve::CurvePoint;
use crate::error::{Error, Result};
use crate::market::calibration::Bin;

/// Classes de moins de 30 prévisions : leur point est tracé sans moustache (intervalle trop large pour guider).
const MIN_N_FOR_WHISKER: usize = 30;

fn draw_error(error: impl Display) -> Error {
    Error::Data(format!("dessin du graphique : {error}"))
}

/// Diagramme de fiabilité : fréquence observée en fonction de la probabilité prévue.
///
/// Un marché calibré suit la diagonale. Les moustaches sont les intervalles de Wilson à 95 %.
pub fn reliability_chart(bins: &[Bin], title: &str) -> Result<String> {
    let mut svg = String::new();
    {
        let root = SVGBackend::with_string(&mut svg, (360, 360)).into_drawing_area();
        root.fill(&WHITE).map_err(draw_error)?;
        let mut chart = ChartBuilder::on(&root)
            .caption(title, ("sans-serif", 15))
            .margin(8)
            .x_label_area_size(34)
            .y_label_area_size(42)
            .build_cartesian_2d(0f64..1f64, 0f64..1f64)
            .map_err(draw_error)?;
        chart
            .configure_mesh()
            .x_desc("probabilité prévue")
            .y_desc("fréquence observée")
            .light_line_style(RGBColor(235, 235, 235))
            .draw()
            .map_err(draw_error)?;

        let grey = RGBColor(130, 130, 130);
        let blue = RGBColor(31, 94, 170);
        chart
            .draw_series(LineSeries::new([(0.0, 0.0), (1.0, 1.0)], &grey))
            .map_err(draw_error)?;
        for bin in bins.iter().filter(|bin| bin.n >= MIN_N_FOR_WHISKER) {
            chart
                .draw_series(LineSeries::new(
                    [
                        (bin.mean_predicted, bin.wilson_low),
                        (bin.mean_predicted, bin.wilson_high),
                    ],
                    &blue,
                ))
                .map_err(draw_error)?;
        }
        chart
            .draw_series(PointSeries::of_element(
                bins.iter().map(|bin| (bin.mean_predicted, bin.observed)),
                3,
                &blue,
                &|coord, size, style| Circle::new(coord, size, style.filled()),
            ))
            .map_err(draw_error)?;
        root.present().map_err(draw_error)?;
    }
    Ok(svg)
}

/// Bornes d'un axe vertical qui contiennent toutes les valeurs **et zéro**, avec 8 % de marge.
fn vertical_range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let (low, high) = values
        .filter(|v| v.is_finite())
        .fold((0.0_f64, 0.0_f64), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let pad = ((high - low) * 0.08).max(1e-4);
    (low - pad, high + pad)
}

/// Abaque : croissance logarithmique totale attendue en fonction du seuil, une courbe par fraction de Kelly
/// (`fractions[j]` pour `point.growth[j]`), et un trait vertical au seuil retenu.
pub fn threshold_growth_chart(
    points: &[CurvePoint],
    fractions: &[f64],
    best: Option<f64>,
    title: &str,
) -> Result<String> {
    let x_max = points.last().map_or(8.0, |p| p.threshold * 100.0);
    let (y_low, y_high) = vertical_range(points.iter().flat_map(|p| p.growth.iter().copied()));
    let mut svg = String::new();
    {
        let root = SVGBackend::with_string(&mut svg, (420, 300)).into_drawing_area();
        root.fill(&WHITE).map_err(draw_error)?;
        let mut chart = ChartBuilder::on(&root)
            .caption(title, ("sans-serif", 15))
            .margin(8)
            .x_label_area_size(34)
            .y_label_area_size(48)
            .build_cartesian_2d(0f64..x_max, y_low..y_high)
            .map_err(draw_error)?;
        chart
            .configure_mesh()
            .x_desc("avantage minimal (%)")
            .y_desc("croissance log. totale")
            .light_line_style(RGBColor(235, 235, 235))
            .draw()
            .map_err(draw_error)?;
        let grey = RGBColor(130, 130, 130);
        chart
            .draw_series(LineSeries::new([(0.0, 0.0), (x_max, 0.0)], &grey))
            .map_err(draw_error)?;
        if let Some(best) = best {
            let x = best * 100.0;
            chart
                .draw_series(LineSeries::new([(x, y_low), (x, y_high)], &grey))
                .map_err(draw_error)?;
        }
        let colors = [
            RGBColor(31, 94, 170),
            RGBColor(214, 120, 30),
            RGBColor(26, 127, 75),
        ];
        for (j, fraction) in fractions.iter().enumerate() {
            let color = colors[j % colors.len()];
            chart
                .draw_series(LineSeries::new(
                    points
                        .iter()
                        .filter_map(|p| p.growth.get(j).map(|g| (p.threshold * 100.0, *g))),
                    color.stroke_width(2),
                ))
                .map_err(draw_error)?
                .label(format!("Kelly ×{}", fraction.to_string().replace('.', ",")))
                .legend(move |(x, y)| {
                    PathElement::new([(x, y), (x + 16, y)], color.stroke_width(2))
                });
        }
        chart
            .configure_series_labels()
            .background_style(WHITE.mix(0.85))
            .border_style(grey)
            .label_font(("sans-serif", 12))
            .draw()
            .map_err(draw_error)?;
        root.present().map_err(draw_error)?;
    }
    Ok(svg)
}

/// CLV moyen par pari en fonction du seuil, avec la bande de son intervalle à 95 %.
pub fn threshold_clv_chart(points: &[CurvePoint], title: &str) -> Result<String> {
    let x_max = points.last().map_or(8.0, |p| p.threshold * 100.0);
    let with_clv: Vec<(f64, f64, f64, f64)> = points
        .iter()
        .filter_map(|p| p.clv.map(|c| (p.threshold * 100.0, c.mean, c.low, c.high)))
        .collect();
    let (y_low, y_high) = vertical_range(
        with_clv
            .iter()
            .flat_map(|(_, _, low, high)| [*low * 100.0, *high * 100.0]),
    );
    let mut svg = String::new();
    {
        let root = SVGBackend::with_string(&mut svg, (420, 300)).into_drawing_area();
        root.fill(&WHITE).map_err(draw_error)?;
        let mut chart = ChartBuilder::on(&root)
            .caption(title, ("sans-serif", 15))
            .margin(8)
            .x_label_area_size(34)
            .y_label_area_size(48)
            .build_cartesian_2d(0f64..x_max, y_low..y_high)
            .map_err(draw_error)?;
        chart
            .configure_mesh()
            .x_desc("avantage minimal (%)")
            .y_desc("CLV moyen (%)")
            .light_line_style(RGBColor(235, 235, 235))
            .draw()
            .map_err(draw_error)?;
        let grey = RGBColor(130, 130, 130);
        let blue = RGBColor(31, 94, 170);
        chart
            .draw_series(LineSeries::new([(0.0, 0.0), (x_max, 0.0)], &grey))
            .map_err(draw_error)?;
        // Bande : le bord haut de gauche à droite, puis le bord bas de droite à gauche, fermés en un polygone.
        let band: Vec<(f64, f64)> = with_clv
            .iter()
            .map(|(x, _, _, high)| (*x, high * 100.0))
            .chain(
                with_clv
                    .iter()
                    .rev()
                    .map(|(x, _, low, _)| (*x, low * 100.0)),
            )
            .collect();
        if !band.is_empty() {
            chart
                .draw_series(std::iter::once(Polygon::new(band, blue.mix(0.18).filled())))
                .map_err(draw_error)?;
        }
        chart
            .draw_series(LineSeries::new(
                with_clv.iter().map(|(x, mean, _, _)| (*x, mean * 100.0)),
                blue.stroke_width(2),
            ))
            .map_err(draw_error)?;
        root.present().map_err(draw_error)?;
    }
    Ok(svg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::calibration::reliability;

    #[test]
    fn chart_is_a_self_contained_svg_with_title_and_axis_labels() {
        let pairs: Vec<(f64, bool)> = (0..200).map(|i| (0.3, i % 10 < 3)).collect();
        let bins = reliability(&pairs, 0.05);

        let svg = reliability_chart(&bins, "Premier League").unwrap();

        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("Premier League"));
        assert!(svg.contains("probabilité prévue"));
        assert!(svg.contains("<circle"), "un point par classe");
        assert!(!svg.contains("href"), "aucune ressource externe");
    }

    #[test]
    fn an_empty_chart_still_renders() {
        assert!(reliability_chart(&[], "vide").unwrap().contains("<svg"));
        assert!(
            threshold_growth_chart(&[], &[1.0], None, "vide")
                .unwrap()
                .contains("<svg")
        );
        assert!(threshold_clv_chart(&[], "vide").unwrap().contains("<svg"));
    }

    #[test]
    fn the_threshold_charts_draw_one_line_per_kelly_fraction_and_a_clv_band() {
        use crate::eval::bootstrap::MeanCi;
        let points: Vec<CurvePoint> = (0..=10)
            .map(|k| CurvePoint {
                threshold: f64::from(k) * 0.001,
                bets: 100 - k as usize,
                clv: Some(MeanCi {
                    mean: 0.01,
                    low: 0.0,
                    high: 0.02,
                }),
                flat_roi: None,
                growth: vec![0.3, 0.2, 0.1],
                growth_ci: None,
            })
            .collect();

        let growth =
            threshold_growth_chart(&points, &[1.0, 0.5, 0.25], Some(0.004), "B365").unwrap();
        let clv = threshold_clv_chart(&points, "B365").unwrap();

        assert!(growth.contains("Kelly ×0,5"), "légende au format français");
        assert!(growth.contains("avantage minimal"));
        assert!(clv.contains("<polygon"), "bande de l'intervalle");
        assert!(!growth.contains("href") && !clv.contains("href"));
    }
}
