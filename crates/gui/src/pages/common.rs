//! Aides communes aux écrans : noms de ligues, dates relatives, montants, en-têtes.

use chrono::{DateTime, Utc};
use edge_core::format;
use edge_core::ingest::football_data::LEAGUES;
use eframe::egui::{RichText, Ui};

use crate::design::icons::Icon;
use crate::design::motion::{Motion, reveal};
use crate::design::widgets::{display, pal};

/// Nom lisible d'une compétition à partir de sa clé The Odds API.
pub fn league_name(sport_key: &str) -> String {
    if let Some(league) = LEAGUES.iter().find(|l| l.sport_key == sport_key) {
        return league.name.to_string();
    }
    if sport_key.starts_with("tennis_") {
        return "Tennis".to_string();
    }
    if sport_key.starts_with("basketball_") {
        return "NBA".to_string();
    }
    sport_key.to_string()
}

/// Icône du sport d'une compétition.
pub fn sport_icon(sport_key: &str) -> Icon {
    if sport_key.starts_with("tennis_") {
        Icon::Tennis
    } else if sport_key.starts_with("basketball_") {
        Icon::Basketball
    } else {
        Icon::Football
    }
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

/// Montant en euros avec des espaces entre les milliers : `1 158 €`, `91,4 €` (une décimale seulement si `digits` > 0).
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

/// En-tête d'écran : un titre en grand et une phrase en dessous, apparaissant d'abord.
pub fn page_header(ui: &mut Ui, motion: &Motion, title: &str, subtitle: &str) {
    let p = pal(ui);
    reveal(ui, motion, 0, |ui| {
        ui.label(display(title).color(p.text));
        if !subtitle.is_empty() {
            ui.label(RichText::new(subtitle).color(p.muted));
        }
    });
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
    }

    #[test]
    fn competitions_get_readable_names_and_sport_icons() {
        assert_eq!(league_name("soccer_epl"), "Premier League");
        assert_eq!(league_name("tennis_wta_ningbo_open"), "Tennis");
        assert_eq!(league_name("basketball_nba"), "NBA");
        assert_eq!(sport_icon("tennis_atp_x"), Icon::Tennis);
        assert_eq!(sport_icon("soccer_spl"), Icon::Football);
    }
}
