//! Aides d'affichage communes aux pages : cadre (barre latérale, message), montants, dates relatives, rendu des gabarits.

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;

use crate::error::Result;
use crate::format;
use crate::report::render::render_page;

/// Message affiché en tête de page après une action (« Pari n° 3 enregistré »), ou une erreur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Flash {
    /// Classe CSS de l'encadré : `t-pos` (réussite) ou `t-neg` (échec).
    pub kind: &'static str,
    /// Texte.
    pub text: String,
}

impl Flash {
    /// Une réussite.
    pub fn ok(text: impl Into<String>) -> Flash {
        Flash {
            kind: "t-pos",
            text: text.into(),
        }
    }

    /// Un échec.
    pub fn error(text: impl Into<String>) -> Flash {
        Flash {
            kind: "t-neg",
            text: text.into(),
        }
    }
}

/// Le cadre commun : barre latérale, onglet actif, heure de calcul, message éventuel.
///
/// Chaque page l'inclut avec `#[serde(flatten)]` : ses champs arrivent au gabarit au même niveau que ceux de la page,
/// là où `site_base.html` les attend (`nav`, `active`…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Layout {
    /// Barre latérale affichée (toujours, sauf sur la page de connexion).
    pub nav: bool,
    /// Onglet actif (`today`, `bets`…).
    pub active: &'static str,
    /// Instant de calcul, heure de Paris.
    pub generated_at: String,
    /// Message à afficher en tête.
    pub flash: Option<Flash>,
}

impl Layout {
    /// Le cadre d'un onglet.
    pub fn new(active: &'static str, now: DateTime<Utc>, flash: Option<Flash>) -> Layout {
        Layout {
            nav: true,
            active,
            generated_at: generated_at(now),
            flash,
        }
    }
}

/// `23/10/2026 14:10`, heure de Paris.
pub fn generated_at(now: DateTime<Utc>) -> String {
    now.with_timezone(&Paris)
        .format("%d/%m/%Y %H:%M")
        .to_string()
}

/// Rend un gabarit de page (qui étend `site_base.html`).
pub fn render<T: Serialize>(name: &'static str, source: &'static str, page: &T) -> Result<String> {
    render_page(name, source, page)
}

/// `Domicile – Extérieur`.
pub fn fixture(home: &str, away: &str) -> String {
    format!("{home} – {away}")
}

/// `sam. 24/10 15:00 · dans 3 j`.
pub fn when(kickoff: DateTime<Utc>, now: DateTime<Utc>) -> String {
    format!(
        "{} · {}",
        format::kickoff_label(kickoff),
        format::until(kickoff, now)
    )
}

/// Pourcentage signé : `+4,1 %`.
pub fn signed_pct(fraction: f64, digits: usize) -> String {
    format!("{} %", format::signed(fraction * 100.0, digits))
}

/// Montant en euros avec des espaces entre les milliers : `1 158 €`, `91,4 €` (décimales seulement si `digits` > 0).
pub fn euros(amount: f64, digits: usize) -> String {
    let text = format::decimal(amount.abs(), digits);
    let (integer, decimals) = match text.split_once(',') {
        Some((i, d)) => (i.to_string(), Some(d.to_string())),
        None => (text, None),
    };
    let mut grouped = String::new();
    for (index, digit) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    let sign = if amount < 0.0 { "-" } else { "" };
    match decimals {
        Some(d) => format!("{sign}{grouped},{d} €"),
        None => format!("{sign}{grouped} €"),
    }
}

/// Montant signé en euros : `+112,00 €`.
pub fn signed_euros(amount: f64) -> String {
    format!("{} €", format::signed(amount, 2))
}

/// Classe de couleur de texte d'un signe : `pos`, `neg`, ou rien pour zéro.
pub fn sign_color(value: f64) -> &'static str {
    if value > 0.0 {
        "pos"
    } else if value < 0.0 {
        "neg"
    } else {
        ""
    }
}

/// Largeur d'une jauge en pour cent, bornée à [0 ; 100].
pub fn gauge_width(fraction: f64) -> String {
    format::decimal((fraction * 100.0).clamp(0.0, 100.0), 1).replace(',', ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_has_spaces_between_thousands_and_a_decimal_comma() {
        assert_eq!(euros(1158.0, 0), "1 158 €");
        assert_eq!(euros(91.4, 1), "91,4 €");
        assert_eq!(euros(1_234_567.0, 0), "1 234 567 €");
        assert_eq!(euros(-70.0, 0), "-70 €");
        assert_eq!(euros(999.0, 0), "999 €");
        assert_eq!(euros(0.0, 2), "0,00 €");
    }

    #[test]
    fn percentages_and_amounts_carry_their_sign() {
        assert_eq!(signed_pct(0.0412, 1), "+4,1 %");
        assert_eq!(signed_pct(-0.0123, 2), "-1,23 %");
        assert_eq!(signed_euros(112.0), "+112,00 €");
        assert_eq!(sign_color(0.0), "");
    }

    #[test]
    fn a_gauge_width_is_a_css_percentage_between_zero_and_one_hundred() {
        assert_eq!(gauge_width(0.4167), "41.7");
        assert_eq!(gauge_width(3.0), "100.0");
        assert_eq!(gauge_width(-0.2), "0.0");
    }
}
