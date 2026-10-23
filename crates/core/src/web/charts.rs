//! Graphiques SVG des pages : courbe, mini-courbe, barres, diagramme de fiabilité.
//!
//! Écrits à la main plutôt qu'avec `plotters` : quelques lignes et points suffisent, et les couleurs viennent des classes
//! CSS de la page (`line-acc`, `bar-pos`…), donc du thème clair ou sombre choisi par le système.

use std::fmt::Write;

use crate::format;

/// Dimensions d'un grand graphique (unités du `viewBox`, mis à l'échelle par la page).
const WIDTH: f64 = 760.0;
const HEIGHT: f64 = 300.0;
/// Marges : à gauche pour les graduations, en bas pour les étiquettes d'abscisse.
const LEFT: f64 = 56.0;
const RIGHT: f64 = 16.0;
const TOP: f64 = 14.0;
const BOTTOM: f64 = 34.0;

/// Échappe un texte pour le placer dans du SVG.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Un nombre pour un attribut SVG : point décimal, deux décimales.
fn n(value: f64) -> String {
    format!("{value:.2}")
}

/// Graduations « rondes » (1, 2 ou 5 × 10^k) couvrant [low ; high], environ `target` intervalles.
///
/// Pas brut r = (high − low) / target ; on prend m = 10^⌊log10 r⌋ et le plus petit pas parmi m, 2m, 5m, 10m qui dépasse
/// r : les graduations tombent alors sur des valeurs faciles à lire (méthode classique des « nice numbers », Heckbert 1990).
pub fn ticks(low: f64, high: f64, target: usize) -> Vec<f64> {
    if !(high - low).is_finite() || high <= low || target == 0 {
        return vec![low];
    }
    let raw = (high - low) / target as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|k| k * magnitude)
        .find(|step| *step >= raw)
        .unwrap_or(10.0 * magnitude);
    let mut value = (low / step).ceil() * step;
    let mut out = Vec::new();
    // La petite tolérance garde la dernière graduation malgré les erreurs d'arrondi des flottants.
    while value <= high + step * 1e-9 {
        // `+ 0.0` transforme un éventuel -0 en 0 : la graduation ne s'affichera pas « -0 ».
        out.push(value + 0.0);
        value += step;
    }
    out
}

/// Décimales utiles pour afficher des graduations espacées de `step`.
fn digits_for(step: f64) -> usize {
    if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else {
        2
    }
}

/// Domaine vertical : toutes les valeurs (et la ligne de base), élargi de 6 % pour ne pas coller aux bords.
fn domain(values: &[f64], baseline: Option<f64>) -> (f64, f64) {
    let all = values.iter().copied().chain(baseline);
    let (low, high) = all.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    if !low.is_finite() {
        return (0.0, 1.0);
    }
    if (high - low).abs() < 1e-9 {
        return (low - 1.0, high + 1.0);
    }
    let pad = (high - low) * 0.06;
    (low - pad, high + pad)
}

/// Courbe d'une série, un point par élément (le rang en abscisse).
///
/// `x_labels` : quelques rangs à étiqueter (`(rang, « 24/09 »)`) ; `baseline` : une ligne horizontale de référence
/// (bankroll de départ, zéro) ; `unit` : suffixe des graduations (` u`).
pub fn line(
    values: &[f64],
    x_labels: &[(usize, String)],
    baseline: Option<f64>,
    unit: &str,
) -> String {
    let (low, high) = domain(values, baseline);
    let plot_w = WIDTH - LEFT - RIGHT;
    let plot_h = HEIGHT - TOP - BOTTOM;
    let last = values.len().saturating_sub(1).max(1) as f64;
    let x = |rank: usize| LEFT + plot_w * rank as f64 / last;
    let y = |value: f64| TOP + plot_h * (high - value) / (high - low);

    let mut svg = String::new();
    // `write!` sur une `String` ne peut pas échouer : son résultat est ignoré sans risque (`let _`).
    let _ = write!(
        svg,
        r#"<svg viewBox="0 0 {} {}" role="img" xmlns="http://www.w3.org/2000/svg">"#,
        n(WIDTH),
        n(HEIGHT)
    );
    let marks = ticks(low, high, 4);
    let digits = marks
        .windows(2)
        .next()
        .map_or(0, |pair| digits_for(pair[1] - pair[0]));
    for mark in &marks {
        let _ = write!(
            svg,
            r#"<line class="grid" x1="{}" x2="{}" y1="{}" y2="{}"/><text x="{}" y="{}" text-anchor="end">{}{}</text>"#,
            n(LEFT),
            n(WIDTH - RIGHT),
            n(y(*mark)),
            n(y(*mark)),
            n(LEFT - 8.0),
            n(y(*mark) + 4.0),
            format::decimal(*mark, digits),
            escape(unit)
        );
    }
    if let Some(base) = baseline {
        let _ = write!(
            svg,
            r#"<line class="base" x1="{}" x2="{}" y1="{}" y2="{}"/>"#,
            n(LEFT),
            n(WIDTH - RIGHT),
            n(y(base)),
            n(y(base))
        );
    }
    for (rank, label) in x_labels {
        let _ = write!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle">{}</text>"#,
            n(x(*rank)),
            n(HEIGHT - 10.0),
            escape(label)
        );
    }
    let points: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(rank, value)| format!("{},{}", n(x(rank)), n(y(*value))))
        .collect();
    let _ = write!(
        svg,
        r#"<polyline class="line-acc" points="{}"/></svg>"#,
        points.join(" ")
    );
    svg
}

/// Mini-courbe d'une tuile : la série seule, sans axe, remplie jusqu'au bas. `tone` : `pos`, `neg` ou `acc`.
pub fn sparkline(values: &[f64], tone: &str) -> String {
    let (w, h) = (240.0, 40.0);
    let (low, high) = domain(values, None);
    let last = values.len().saturating_sub(1).max(1) as f64;
    let x = |rank: usize| w * rank as f64 / last;
    let y = |value: f64| 2.0 + (h - 4.0) * (high - value) / (high - low);
    let points: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(rank, value)| format!("{},{}", n(x(rank)), n(y(*value))))
        .collect();
    let area = format!(
        "0,{h} {} {},{h}",
        points.join(" "),
        n(x(values.len().saturating_sub(1)))
    );
    format!(
        r#"<svg viewBox="0 0 {w} {h}" preserveAspectRatio="none" xmlns="http://www.w3.org/2000/svg"><polygon class="area-{tone}" points="{area}"/><polyline class="line-{tone}" points="{}"/></svg>"#,
        points.join(" ")
    )
}

/// Barres verticales autour de zéro, une par catégorie ; une valeur absente laisse la place vide. Valeurs en fraction,
/// graduations en pour cent.
pub fn bars(items: &[(String, Option<f64>)]) -> String {
    let height = 240.0;
    let values: Vec<f64> = items.iter().filter_map(|(_, v)| *v).collect();
    let (low, high) = domain(&values, Some(0.0));
    let plot_w = WIDTH - LEFT - RIGHT;
    let plot_h = height - TOP - BOTTOM;
    let y = |value: f64| TOP + plot_h * (high - value) / (high - low);
    let slot = plot_w / items.len().max(1) as f64;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {} {}" role="img" xmlns="http://www.w3.org/2000/svg">"#,
        n(WIDTH),
        n(height)
    );
    let marks = ticks(low * 100.0, high * 100.0, 4);
    let digits = marks
        .windows(2)
        .next()
        .map_or(0, |pair| digits_for(pair[1] - pair[0]));
    for mark in &marks {
        let at = y(mark / 100.0);
        let _ = write!(
            svg,
            r#"<line class="grid" x1="{}" x2="{}" y1="{}" y2="{}"/><text x="{}" y="{}" text-anchor="end">{} %</text>"#,
            n(LEFT),
            n(WIDTH - RIGHT),
            n(at),
            n(at),
            n(LEFT - 8.0),
            n(at + 4.0),
            format::signed(*mark, digits)
        );
    }
    let zero = y(0.0);
    for (index, (label, value)) in items.iter().enumerate() {
        let center = LEFT + slot * (index as f64 + 0.5);
        if let Some(value) = value {
            let top = y(*value).min(zero);
            let class = if *value >= 0.0 { "bar-pos" } else { "bar-neg" };
            let _ = write!(
                svg,
                r#"<rect class="{class}" x="{}" y="{}" width="{}" height="{}" rx="3"/>"#,
                n(center - slot * 0.28),
                n(top),
                n(slot * 0.56),
                n((y(*value) - zero).abs())
            );
        }
        let _ = write!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle">{}</text>"#,
            n(center),
            n(height - 10.0),
            escape(label)
        );
    }
    let _ = write!(
        svg,
        r#"<line class="base" x1="{}" x2="{}" y1="{}" y2="{}"/></svg>"#,
        n(LEFT),
        n(WIDTH - RIGHT),
        n(zero),
        n(zero)
    );
    svg
}

/// Diagramme de fiabilité : probabilité annoncée (abscisse) contre fréquence observée (ordonnée), sur [0 ; 1]², avec la
/// diagonale de la fiabilité parfaite.
pub fn reliability(points: &[(f64, f64)]) -> String {
    let size = 320.0;
    let (left, bottom, top, right) = (48.0, 40.0, 12.0, 12.0);
    let side = size - left - right;
    let height = side + top + bottom;
    let x = |p: f64| left + side * p;
    let y = |p: f64| top + side * (1.0 - p);
    let mut svg = format!(
        r#"<svg viewBox="0 0 {} {}" role="img" xmlns="http://www.w3.org/2000/svg">"#,
        n(size),
        n(height)
    );
    for mark in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let label = format::decimal(mark * 100.0, 0);
        let _ = write!(
            svg,
            r#"<line class="grid" x1="{}" x2="{}" y1="{}" y2="{}"/><line class="grid" x1="{}" x2="{}" y1="{}" y2="{}"/><text x="{}" y="{}" text-anchor="end">{label} %</text><text x="{}" y="{}" text-anchor="middle">{label} %</text>"#,
            n(x(0.0)),
            n(x(1.0)),
            n(y(mark)),
            n(y(mark)),
            n(x(mark)),
            n(x(mark)),
            n(y(0.0)),
            n(y(1.0)),
            n(left - 6.0),
            n(y(mark) + 4.0),
            n(x(mark)),
            n(y(0.0) + 18.0)
        );
    }
    let _ = write!(
        svg,
        r#"<line class="base" x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
        n(x(0.0)),
        n(y(0.0)),
        n(x(1.0)),
        n(y(1.0))
    );
    for (announced, observed) in points {
        let _ = write!(
            svg,
            r#"<circle class="dot" cx="{}" cy="{}" r="6"/>"#,
            n(x(*announced)),
            n(y(*observed))
        );
    }
    let _ = write!(
        svg,
        r#"<text x="{}" y="{}" text-anchor="middle">probabilité annoncée</text></svg>"#,
        n(x(0.5)),
        n(height - 4.0)
    );
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_fall_on_round_values() {
        assert_eq!(ticks(980.0, 1215.0, 4), vec![1000.0, 1100.0, 1200.0]);
        assert_eq!(ticks(990.0, 1110.0, 4), vec![1000.0, 1050.0, 1100.0]);
        assert_eq!(ticks(-0.3, 1.1, 4), vec![0.0, 0.5, 1.0]);
        assert_eq!(ticks(-3.0, 2.5, 4), vec![-2.0, 0.0, 2.0]);
        assert_eq!(ticks(5.0, 5.0, 4), vec![5.0]);
    }

    #[test]
    fn a_line_has_one_point_per_value_and_its_baseline() {
        let svg = line(
            &[1000.0, 1050.0, 990.0],
            &[(0, "24/09".into())],
            Some(1000.0),
            "",
        );
        let polyline = svg.split("points=\"").nth(1).unwrap();
        let points = polyline.split('"').next().unwrap();
        assert_eq!(points.split(' ').count(), 3);
        assert!(svg.contains("class=\"base\""));
        assert!(svg.contains(">24/09<"));
        // Une seule valeur : pas de division par zéro, un point à gauche.
        assert!(line(&[5.0], &[], None, " u").contains("points=\"56.00,"));
    }

    #[test]
    fn bars_are_coloured_by_sign_and_a_missing_value_leaves_a_gap() {
        let svg = bars(&[
            ("0-2 %".into(), Some(0.012)),
            ("2-4 %".into(), None),
            ("> 4 %".into(), Some(-0.02)),
        ]);
        assert_eq!(svg.matches("<rect").count(), 2);
        assert!(svg.contains("bar-pos") && svg.contains("bar-neg"));
        assert!(svg.contains(">2-4 %<"));
    }

    #[test]
    fn team_names_are_escaped_and_the_diagram_has_one_dot_per_bin() {
        assert!(bars(&[("A & B".into(), Some(0.1))]).contains("A &amp; B"));
        let svg = reliability(&[(0.29, 0.26), (0.47, 0.5)]);
        assert_eq!(svg.matches("class=\"dot\"").count(), 2);
        assert!(sparkline(&[1.0, 2.0, 1.5], "pos").contains("area-pos"));
    }
}
