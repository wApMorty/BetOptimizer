//! Rendu HTML du rapport « marché » avec `minijinja` (gabarit embarqué dans le binaire).
//!
//! Le fichier produit est autonome : styles et graphiques SVG sont dans le HTML, aucune ressource externe,
//! et la mise en page s'adapte à un écran de téléphone.

use minijinja::{Environment, Value};

use serde::Serialize;

use super::market_report::MarketReport;
use super::models_report::ModelsReport;
use crate::error::{Error, Result};

const MARKET_TEMPLATE: &str = include_str!("templates/market.html");
const MODELS_TEMPLATE: &str = include_str!("templates/models.html");

/// Nombre au format français (virgule décimale), `digits` décimales.
fn number(value: f64, digits: Option<usize>) -> String {
    let text = format!("{:.*}", digits.unwrap_or(3), value);
    // Une valeur négative arrondie à zéro s'écrirait « -0,0 » : on retire le signe.
    let text = match text.strip_prefix('-') {
        Some(unsigned) if unsigned.chars().all(|c| c == '0' || c == '.') => unsigned.to_string(),
        _ => text,
    };
    text.replace('.', ",")
}

/// Fraction au format français en pourcentage : `0,047` → `4,7 %`.
fn percent(value: f64, digits: Option<usize>) -> String {
    format!("{} %", number(value * 100.0, Some(digits.unwrap_or(1))))
}

/// Génère le HTML du rapport « marché ».
pub fn render_market_report(report: &MarketReport) -> Result<String> {
    render_template("market.html", MARKET_TEMPLATE, report)
}

/// Génère le HTML du rapport « modèles ».
pub fn render_models_report(report: &ModelsReport) -> Result<String> {
    render_template("models.html", MODELS_TEMPLATE, report)
}

/// Rend un gabarit avec les filtres `num` et `pct`.
///
/// Le nom se termine par `.html` : l'échappement automatique est actif (les graphiques SVG sont
/// explicitement marqués `|safe` dans les gabarits).
fn render_template<T: Serialize>(
    name: &'static str,
    source: &'static str,
    data: &T,
) -> Result<String> {
    let mut env = Environment::new();
    env.add_template(name, source)
        .map_err(|e| Error::Data(format!("gabarit du rapport : {e}")))?;
    env.add_filter("num", number);
    env.add_filter("pct", percent);
    let template = env
        .get_template(name)
        .map_err(|e| Error::Data(format!("gabarit du rapport : {e}")))?;
    template
        .render(Value::from_serialize(data))
        .map_err(|e| Error::Data(format!("rendu du rapport : {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_percentages_use_the_french_decimal_comma() {
        assert_eq!(number(1.23456, Some(2)), "1,23");
        assert_eq!(number(0.5, None), "0,500");
        assert_eq!(percent(0.0468, None), "4,7 %");
        assert_eq!(percent(0.141_5, Some(2)), "14,15 %");
        assert_eq!(percent(-0.04, Some(0)), "-4 %");
        // Zéro négatif arrondi : pas de signe.
        assert_eq!(percent(-0.0001, None), "0,0 %");
        assert_eq!(number(-0.000_001, Some(5)), "0,00000");
        assert_eq!(number(-0.0123, Some(2)), "-0,01");
    }
}
