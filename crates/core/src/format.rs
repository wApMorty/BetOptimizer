//! Mise en forme à la française, pour la CLI et les notifications : virgule décimale, heure de Paris.

use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use chrono_tz::Europe::Paris;

use crate::domain::Outcome;

/// Nombre avec virgule décimale et `digits` décimales : `2,30`.
pub fn decimal(value: f64, digits: usize) -> String {
    format!("{value:.digits$}").replace('.', ",")
}

/// Comme [`decimal`], avec le signe toujours écrit : `+11,9`, `-0,4`.
pub fn signed(value: f64, digits: usize) -> String {
    format!("{value:+.digits$}").replace('.', ",")
}

/// Coup d'envoi en heure de Paris : `sam. 24/10 15:00`.
pub fn kickoff_label(at: DateTime<Utc>) -> String {
    let local = at.with_timezone(&Paris);
    let day = match local.weekday() {
        Weekday::Mon => "lun.",
        Weekday::Tue => "mar.",
        Weekday::Wed => "mer.",
        Weekday::Thu => "jeu.",
        Weekday::Fri => "ven.",
        Weekday::Sat => "sam.",
        Weekday::Sun => "dim.",
    };
    format!(
        "{day} {:02}/{:02} {:02}:{:02}",
        local.day(),
        local.month(),
        local.hour(),
        local.minute()
    )
}

/// Âge d'une date : `il y a 12 min` ou `il y a 3 h 20`.
pub fn age(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let minutes = (now - at).num_minutes().max(0);
    if minutes < 60 {
        format!("il y a {minutes} min")
    } else {
        format!("il y a {} h {:02}", minutes / 60, minutes % 60)
    }
}

/// Nom français d'une issue : `domicile`, `nul`, `extérieur`.
pub fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Home => "domicile",
        Outcome::Draw => "nul",
        Outcome::Away => "extérieur",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    #[test]
    fn numbers_use_the_decimal_comma_and_an_explicit_sign() {
        assert_eq!(decimal(2.3, 2), "2,30");
        assert_eq!(decimal(1000.0, 0), "1000");
        assert_eq!(signed(11.89, 1), "+11,9");
        assert_eq!(signed(-0.44, 1), "-0,4");
    }

    #[test]
    fn kickoff_is_shown_in_paris_time_with_the_french_weekday() {
        // Octobre 2026 : l'heure d'été est encore en vigueur (UTC+2) jusqu'au 25 octobre.
        assert_eq!(
            kickoff_label(at("2026-10-24T13:00:00Z")),
            "sam. 24/10 15:00"
        );
        // Après le passage à l'heure d'hiver : UTC+1.
        assert_eq!(
            kickoff_label(at("2026-10-26T20:00:00Z")),
            "lun. 26/10 21:00"
        );
    }

    #[test]
    fn ages_are_written_in_minutes_then_hours() {
        let now = at("2026-10-18T15:00:00Z");
        assert_eq!(age(at("2026-10-18T14:48:00Z"), now), "il y a 12 min");
        assert_eq!(age(at("2026-10-18T11:05:00Z"), now), "il y a 3 h 55");
        // Une date dans le futur ne donne pas un âge négatif.
        assert_eq!(age(at("2026-10-18T16:00:00Z"), now), "il y a 0 min");
    }

    #[test]
    fn outcomes_have_french_names() {
        assert_eq!(outcome_label(Outcome::Draw), "nul");
    }
}
