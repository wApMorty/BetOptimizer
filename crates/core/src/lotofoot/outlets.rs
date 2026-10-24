//! Points de vente où Paul valide ses grilles, et heure limite réelle de validation d'une grille.
//!
//! Une grille ne se joue qu'au guichet (règlement Loto Sports v6, art. 3.1.5). Son heure de clôture ne suffit donc pas :
//! la plupart des buralistes ferment le dimanche, jour où ferment environ 20 % des LF7, souvent vers 14 h. Pour chaque
//! point de vente de la configuration (`[[lotofoot_outlets]]`, horaires de la semaine à l'heure de Paris), on cherche le
//! dernier instant avant la clôture où il est ouvert (`docs/sources.md` §48).

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Europe::Paris;

use crate::config::LotofootOutlet;
use crate::error::{Error, Result};

/// Un point de vente et ses plages d'ouverture, du lundi (indice 0) au dimanche (indice 6), à l'heure de Paris.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outlet {
    /// Nom affiché.
    pub name: String,
    /// Plages de chaque jour : (ouverture, fermeture).
    pub week: [Vec<(NaiveTime, NaiveTime)>; 7],
}

/// Jours de la semaine dans la configuration, du lundi au dimanche.
pub const DAYS: [&str; 7] = ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"];

impl Outlet {
    /// Lit un point de vente de la configuration : chaque plage s'écrit `"07:00-20:00"`. Un jour absent ou vide est un
    /// jour de fermeture. Refus si une plage est illisible ou si elle finit avant de commencer.
    pub fn from_config(config: &LotofootOutlet) -> Result<Outlet> {
        let days = [
            &config.lun,
            &config.mar,
            &config.mer,
            &config.jeu,
            &config.ven,
            &config.sam,
            &config.dim,
        ];
        let mut week: [Vec<(NaiveTime, NaiveTime)>; 7] = Default::default();
        for (slot, (ranges, day)) in week.iter_mut().zip(days.iter().zip(DAYS)) {
            for range in ranges.iter() {
                let parsed = parse_range(range).ok_or_else(|| {
                    Error::InvalidConfig(format!(
                        "lotofoot_outlets, « {} », {day} : plage « {range} » illisible (attendu « 07:00-20:00 »)",
                        config.name
                    ))
                })?;
                slot.push(parsed);
            }
        }
        Ok(Outlet {
            name: config.name.clone(),
            week,
        })
    }

    /// Dernier instant où ce point de vente est ouvert, entre `now` et la clôture `closes_at` ; `None` s'il reste fermé
    /// d'ici là. On remonte les jours depuis celui de la clôture jusqu'à aujourd'hui (14 jours au plus).
    pub fn last_chance(
        &self,
        closes_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let close_day = closes_at.with_timezone(&Paris).date_naive();
        let today = now.with_timezone(&Paris).date_naive();
        let mut best: Option<DateTime<Utc>> = None;
        for back in 0..14 {
            let day = close_day - Duration::days(back);
            if day < today {
                break;
            }
            let index = day.weekday().num_days_from_monday() as usize;
            for &(open, shut) in &self.week[index] {
                let (Some(open), Some(shut)) = (paris(day, open), paris(day, shut)) else {
                    continue;
                };
                // Le guichet doit être ouvert, la grille pas encore close, et l'instant pas encore passé.
                let end = shut.min(closes_at);
                if end > open.max(now) && best.is_none_or(|b| end > b) {
                    best = Some(end);
                }
            }
            if best.is_some() {
                // Le jour le plus proche de la clôture qui a une plage ouverte l'emporte : inutile de remonter plus loin.
                break;
            }
        }
        best
    }
}

/// Lit `"07:00-20:00"` ; `None` si le texte est mal formé ou si la plage est vide.
fn parse_range(text: &str) -> Option<(NaiveTime, NaiveTime)> {
    let (open, shut) = text.split_once('-')?;
    let open = NaiveTime::parse_from_str(open.trim(), "%H:%M").ok()?;
    let shut = NaiveTime::parse_from_str(shut.trim(), "%H:%M").ok()?;
    (open < shut).then_some((open, shut))
}

/// Une heure de Paris un jour donné, en UTC. Lors du passage à l'heure d'hiver, une heure existe deux fois : on prend la
/// plus tôt ; une heure qui n'existe pas (passage à l'heure d'été) est ignorée.
fn paris(day: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    Paris
        .from_local_datetime(&day.and_time(time))
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Les horaires relevés le 2026-10-24 pour Le Longchamp (Cesson-Sévigné).
    fn longchamp() -> Outlet {
        let week = |ranges: &[&str]| ranges.iter().map(|r| r.to_string()).collect::<Vec<_>>();
        Outlet::from_config(&LotofootOutlet {
            name: "Le Longchamp".to_string(),
            lun: vec![],
            mar: week(&["07:00-20:00"]),
            mer: week(&["07:00-20:00"]),
            jeu: week(&["07:00-20:00"]),
            ven: week(&["07:00-20:00"]),
            sam: week(&["07:00-20:00"]),
            dim: week(&["08:30-13:00"]),
        })
        .unwrap()
    }

    #[test]
    fn a_sunday_grid_must_be_validated_before_the_shop_closes() {
        // LF7 n°133 : clôture dimanche 25 octobre à 17 h 10 (16 h 10 UTC, heure d'hiver). Le Longchamp ferme à 13 h.
        let limit = longchamp().last_chance(at("2026-10-25T16:10:00Z"), at("2026-10-24T09:00:00Z"));
        assert_eq!(limit, Some(at("2026-10-25T12:00:00Z")));
    }

    #[test]
    fn an_evening_grid_can_be_validated_until_it_closes() {
        // LF8 n°147 : clôture samedi 24 octobre à 17 h 55 (15 h 55 UTC) ; le tabac est ouvert jusqu'à 20 h.
        let limit = longchamp().last_chance(at("2026-10-24T15:55:00Z"), at("2026-10-24T09:00:00Z"));
        assert_eq!(limit, Some(at("2026-10-24T15:55:00Z")));
    }

    #[test]
    fn a_closed_day_sends_back_to_the_day_before() {
        // Clôture un lundi à 20 h 30 (18 h 30 UTC) : fermé le lundi, donc le dimanche à 13 h.
        let limit = longchamp().last_chance(at("2026-10-26T19:30:00Z"), at("2026-10-24T09:00:00Z"));
        assert_eq!(limit, Some(at("2026-10-25T12:00:00Z")));
        // Le dimanche déjà passé : plus aucune occasion avant la clôture.
        assert_eq!(
            longchamp().last_chance(at("2026-10-26T19:30:00Z"), at("2026-10-25T13:00:00Z")),
            None
        );
    }

    #[test]
    fn malformed_ranges_are_refused() {
        let mut config = LotofootOutlet {
            name: "X".to_string(),
            lun: vec!["7h-20h".to_string()],
            mar: vec![],
            mer: vec![],
            jeu: vec![],
            ven: vec![],
            sam: vec![],
            dim: vec![],
        };
        assert!(Outlet::from_config(&config).is_err());
        config.lun = vec!["20:00-07:00".to_string()];
        assert!(Outlet::from_config(&config).is_err());
    }
}
