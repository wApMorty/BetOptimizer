//! Rendu HTML des rapports et des pages du site avec `minijinja` (gabarits embarqués dans le binaire).
//!
//! Le fichier produit est autonome : styles et graphiques SVG sont dans le HTML, aucune ressource externe,
//! et la mise en page s'adapte à un écran de téléphone.

use minijinja::{Environment, Value};

use serde::Serialize;

use super::backtest_report::BacktestReport;
use super::bonus_page::BonusPage;
use super::market_report::MarketReport;
use super::models_report::ModelsReport;
use super::nba_report::NbaReport;
use super::recommend_report::RecommendReport;
use super::site::{MessagePage, OpportunitiesPage, PaperPage, ReportsPage};
use super::tennis_report::TennisReport;
use super::thresholds_report::ThresholdsReport;
use crate::error::{Error, Result};

const MARKET_TEMPLATE: &str = include_str!("templates/market.html");
const MODELS_TEMPLATE: &str = include_str!("templates/models.html");
const BACKTEST_TEMPLATE: &str = include_str!("templates/backtest.html");
const RECOMMEND_TEMPLATE: &str = include_str!("templates/recommend.html");
const TENNIS_TEMPLATE: &str = include_str!("templates/tennis.html");
const NBA_TEMPLATE: &str = include_str!("templates/nba.html");
const THRESHOLDS_TEMPLATE: &str = include_str!("templates/thresholds.html");
/// Base commune (styles, barre de navigation) dont héritent le rapport quotidien et les pages du site.
const SITE_BASE_TEMPLATE: &str = include_str!("templates/site_base.html");
const SITE_REPORTS_TEMPLATE: &str = include_str!("templates/site_reports.html");
const SITE_OPPORTUNITIES_TEMPLATE: &str = include_str!("templates/site_opportunities.html");
const SITE_PAPER_TEMPLATE: &str = include_str!("templates/site_paper.html");
const SITE_MESSAGE_TEMPLATE: &str = include_str!("templates/site_message.html");
const SITE_BONUS_TEMPLATE: &str = include_str!("templates/site_bonus.html");

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

/// Fraction au format français en pourcentage **signé** : `0,009` → `+0,9 %`.
///
/// Un résultat qui s'arrondit à zéro n'a pas de signe (`0,0 %`), comme dans [`percent`].
fn signed_percent(value: f64, digits: Option<usize>) -> String {
    let text = percent(value, digits);
    let is_zero = !text.chars().any(|c| ('1'..='9').contains(&c));
    if text.starts_with('-') || is_zero {
        text
    } else {
        format!("+{text}")
    }
}

/// Génère le HTML du rapport « marché ».
pub fn render_market_report(report: &MarketReport) -> Result<String> {
    render_template("market.html", MARKET_TEMPLATE, report)
}

/// Génère le HTML du rapport « modèles ».
pub fn render_models_report(report: &ModelsReport) -> Result<String> {
    render_template("models.html", MODELS_TEMPLATE, report)
}

/// Génère le HTML du rapport des recommandations (la pièce jointe de la notification).
pub fn render_recommend_report(report: &RecommendReport) -> Result<String> {
    render_template("recommend.html", RECOMMEND_TEMPLATE, report)
}

/// Génère le HTML du rapport « NBA ».
pub fn render_nba_report(report: &NbaReport) -> Result<String> {
    render_template("nba.html", NBA_TEMPLATE, report)
}

/// Génère le HTML du rapport « tennis ».
pub fn render_tennis_report(report: &TennisReport) -> Result<String> {
    render_template("tennis.html", TENNIS_TEMPLATE, report)
}

/// Génère le HTML du rapport « backtest ».
pub fn render_backtest_report(report: &BacktestReport) -> Result<String> {
    render_template("backtest.html", BACKTEST_TEMPLATE, report)
}

/// Génère le HTML du rapport « abaque des seuils ».
pub fn render_thresholds_report(report: &ThresholdsReport) -> Result<String> {
    render_template("thresholds.html", THRESHOLDS_TEMPLATE, report)
}

/// Génère la page « rapports précédents » du site.
pub fn render_site_reports(page: &ReportsPage) -> Result<String> {
    render_template("site_reports.html", SITE_REPORTS_TEMPLATE, page)
}

/// Génère la page « opportunités actives » du site.
pub fn render_site_opportunities(page: &OpportunitiesPage) -> Result<String> {
    render_template("site_opportunities.html", SITE_OPPORTUNITIES_TEMPLATE, page)
}

/// Génère la page « paper trading » du site.
pub fn render_site_paper(page: &PaperPage) -> Result<String> {
    render_template("site_paper.html", SITE_PAPER_TEMPLATE, page)
}

/// Génère la page « bonus » du site.
pub fn render_site_bonus(page: &BonusPage) -> Result<String> {
    render_template("site_bonus.html", SITE_BONUS_TEMPLATE, page)
}

/// Génère une page de message du site (aucun rapport, page introuvable, erreur).
pub fn render_site_message(page: &MessagePage) -> Result<String> {
    render_template("site_message.html", SITE_MESSAGE_TEMPLATE, page)
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
    // La base est chargée à chaque rendu : un gabarit qui l'étend (`{% extends "site_base.html" %}`) la trouve.
    env.add_template("site_base.html", SITE_BASE_TEMPLATE)
        .map_err(|e| Error::Data(format!("gabarit du rapport : {e}")))?;
    env.add_template(name, source)
        .map_err(|e| Error::Data(format!("gabarit du rapport : {e}")))?;
    env.add_filter("num", number);
    env.add_filter("pct", percent);
    env.add_filter("spct", signed_percent);
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

    #[test]
    fn signed_percentages_carry_a_plus_sign_only_when_positive() {
        assert_eq!(signed_percent(0.009, Some(1)), "+0,9 %");
        assert_eq!(signed_percent(-0.0402, Some(2)), "-4,02 %");
        assert_eq!(signed_percent(0.0, Some(1)), "0,0 %");
        assert_eq!(signed_percent(-0.0001, Some(1)), "0,0 %");
    }
}
