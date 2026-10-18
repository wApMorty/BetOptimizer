//! Graphiques SVG (crate `plotters`, sans bibliothèque système : le texte est écrit en balises SVG).

use std::fmt::Display;

use plotters::prelude::*;

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
    }
}
