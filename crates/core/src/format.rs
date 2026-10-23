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

/// Mise en unités : `9,1 u`. Les unités sont celles de [`crate::betting::kelly::UNIT`] (1 u = 1 % de la bankroll).
pub fn units(value: f64) -> String {
    format!("{} u", decimal(value, 1))
}

/// Chance écrite « 1 sur N », pour les événements rares : `1 sur 4,3`, `1 sur 180`, `1 sur 2 400`. N est arrondi à deux
/// chiffres significatifs au-delà de 10 (la précision du modèle ne va pas plus loin). « — » pour une probabilité nulle.
pub fn chance(probability: f64) -> String {
    if probability.is_nan() || probability <= 0.0 {
        return "—".to_string();
    }
    let n = 1.0 / probability;
    if n < 10.0 {
        format!("1 sur {}", decimal(n, 1))
    } else {
        format!("1 sur {}", grouped(significant(n, 2)))
    }
}

/// Montant approché : `≈ 4,80 €`, `≈ 45 €`, `≈ 3 150 €` (trois chiffres significatifs au-delà de 100 €).
pub fn rough_euros(amount: f64) -> String {
    if amount < 10.0 {
        format!("≈ {} €", decimal(amount.max(0.0), 2))
    } else if amount < 100.0 {
        format!("≈ {} €", decimal(amount, 0))
    } else {
        format!("≈ {} €", grouped(significant(amount, 3)))
    }
}

/// `x ≥ 1` arrondi à `digits` chiffres significatifs : `significant(2 437, 2) = 2 400`.
fn significant(x: f64, digits: i32) -> u64 {
    let scale = 10f64.powi(x.log10().floor() as i32 + 1 - digits).max(1.0);
    // Valeur positive et finie, arrondie à l'unité : la conversion en entier est exacte.
    ((x / scale).round() * scale) as u64
}

/// Entier aux milliers séparés par une espace insécable (le nombre ne se coupe pas en fin de ligne) : `12 400`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(digit);
    }
    out
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

/// Date en toutes lettres, en heure de Paris : `lundi 19 octobre`.
pub fn long_date(at: DateTime<Utc>) -> String {
    let local = at.with_timezone(&Paris);
    let day = match local.weekday() {
        Weekday::Mon => "lundi",
        Weekday::Tue => "mardi",
        Weekday::Wed => "mercredi",
        Weekday::Thu => "jeudi",
        Weekday::Fri => "vendredi",
        Weekday::Sat => "samedi",
        Weekday::Sun => "dimanche",
    };
    let month = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ][local.month0() as usize];
    format!("{day} {} {month}", local.day())
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

/// Délai avant une date future : `dans 45 min`, `dans 3 h 20`, `dans 2 j`. Une date passée donne `maintenant`.
pub fn until(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let minutes = (at - now).num_minutes();
    if minutes <= 0 {
        "maintenant".to_string()
    } else if minutes < 60 {
        format!("dans {minutes} min")
    } else if minutes < 48 * 60 {
        format!("dans {} h {:02}", minutes / 60, minutes % 60)
    } else {
        format!("dans {} j", minutes / (24 * 60))
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

    #[test]
    fn rare_chances_read_as_one_in_n() {
        assert_eq!(chance(0.25), "1 sur 4,0");
        assert_eq!(chance(1.0 / 183.0), "1 sur 180");
        assert_eq!(chance(1.0 / 2437.0), "1 sur 2\u{a0}400");
        assert_eq!(chance(0.0), "—");
    }

    #[test]
    fn rough_amounts_keep_three_significant_digits() {
        assert_eq!(rough_euros(4.8), "≈ 4,80 €");
        assert_eq!(rough_euros(45.4), "≈ 45 €");
        assert_eq!(rough_euros(3_147.0), "≈ 3\u{a0}150 €");
        assert_eq!(rough_euros(152_480.0), "≈ 152\u{a0}000 €");
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    #[test]
    fn numbers_use_the_decimal_comma_and_an_explicit_sign() {
        assert_eq!(decimal(2.3, 2), "2,30");
        assert_eq!(decimal(1000.0, 0), "1000");
        assert_eq!(signed(11.89, 1), "+11,9");
        assert_eq!(signed(-0.44, 1), "-0,4");
        assert_eq!(units(9.14), "9,1 u");
        assert_eq!(units(0.0), "0,0 u");
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
    fn the_long_date_uses_paris_time_and_french_names() {
        assert_eq!(long_date(at("2026-10-19T12:00:00Z")), "lundi 19 octobre");
        // 23 h UTC le 31 décembre : déjà le 1er janvier à Paris.
        assert_eq!(long_date(at("2026-12-31T23:30:00Z")), "vendredi 1 janvier");
    }

    #[test]
    fn the_time_until_a_date_is_written_in_minutes_hours_then_days() {
        let now = at("2026-10-18T15:00:00Z");

        assert_eq!(until(at("2026-10-18T15:45:00Z"), now), "dans 45 min");
        assert_eq!(until(at("2026-10-18T18:20:00Z"), now), "dans 3 h 20");
        assert_eq!(until(at("2026-10-21T15:00:00Z"), now), "dans 3 j");
        assert_eq!(until(at("2026-10-18T14:00:00Z"), now), "maintenant");
    }

    #[test]
    fn outcomes_have_french_names() {
        assert_eq!(outcome_label(Outcome::Draw), "nul");
    }
}
