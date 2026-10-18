//! Texte d'une notification : court, lisible sur un écran de téléphone, limité à la taille d'un message Discord.

use crate::format::{decimal, kickoff_label, outcome_label, signed};
use crate::store::journal::StoredRecommendation;

/// Longueur maximale d'un message Discord (caractères).
pub const MAX_CHARS: usize = 2000;

/// Marge gardée pour la ligne finale « et N autres ».
const RESERVED: usize = 100;

/// Message des nouvelles recommandations. Vide de recommandation, il n'y a pas de message à envoyer : on renvoie `None`.
pub fn recommendations(recommended: &[&StoredRecommendation]) -> Option<String> {
    if recommended.is_empty() {
        return None;
    }
    let mut text = format!(
        "**edge** · {} recommandation(s) **expérimentale(s)** : aucune règle n'est validée hors échantillon. Vérifie la cote sur le site avant tout pari.",
        recommended.len()
    );
    for (index, found) in recommended.iter().enumerate() {
        let entry = entry(found);
        // `chars().count()` : la limite de Discord se compte en caractères, pas en octets (les accents en font deux).
        if text.chars().count() + entry.chars().count() > MAX_CHARS - RESERVED {
            text.push_str(&format!(
                "\n… et {} autre(s) : voir le rapport joint.",
                recommended.len() - index
            ));
            break;
        }
        text.push_str(&entry);
    }
    Some(text)
}

fn entry(found: &StoredRecommendation) -> String {
    let r = &found.record;
    let stake = r.stake.map_or_else(
        || "mise sous le minimum".to_string(),
        |s| format!("mise {} €", decimal(s.get(), 1)),
    );
    format!(
        "\n\n**{}** · {} – {}\n{} · {} à {} · cote minimale {} · avantage {} % · {}",
        kickoff_label(r.kickoff),
        r.home_team,
        r.away_team,
        r.bookmaker,
        outcome_label(r.outcome),
        decimal(r.odds.get(), 2),
        decimal(r.minimal_odds, 2),
        signed(r.edge * 100.0, 1),
        stake
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Euros, Odds, Outcome, Probability};
    use crate::store::journal::{NewRecommendation, Tier};

    fn stored(id: i64, stake: Option<f64>) -> StoredRecommendation {
        StoredRecommendation {
            id,
            created_at: "2026-10-18T14:05:00Z".parse().unwrap(),
            record: NewRecommendation {
                rule: "price_vs_pinnacle".into(),
                tier: Tier::Recommended,
                event_id: format!("match-{id}"),
                sport_key: "soccer_epl".into(),
                home_team: "Arsenal".into(),
                away_team: "Chelsea".into(),
                kickoff: "2026-10-24T13:00:00Z".parse().unwrap(),
                bookmaker: "betclic_fr".into(),
                outcome: Outcome::Home,
                odds: Odds::new(2.3).unwrap(),
                probability: Probability::new(0.4865).unwrap(),
                edge: 0.1189,
                min_edge: 0.03,
                minimal_odds: 2.117,
                stake: stake.and_then(Euros::new),
                kelly_fraction: 1.0,
                bankroll: Euros::new(1000.0).unwrap(),
                margin_method: "power".into(),
                api_call_id: 1,
                code_version: "test".into(),
            },
        }
    }

    #[test]
    fn nothing_to_recommend_means_no_message() {
        assert_eq!(recommendations(&[]), None);
    }

    #[test]
    fn a_message_carries_the_pick_the_minimal_odds_the_stake_and_the_warning() {
        let one = stored(1, Some(91.4));

        let text = recommendations(&[&one]).unwrap();

        assert!(text.contains("expérimentale"));
        assert!(text.contains("sam. 24/10 15:00"), "heure de Paris : {text}");
        assert!(text.contains("Arsenal – Chelsea"));
        assert!(text.contains("betclic_fr · domicile à 2,30"));
        assert!(text.contains("cote minimale 2,12"));
        assert!(text.contains("avantage +11,9 %"));
        assert!(text.contains("mise 91,4 €"));
    }

    #[test]
    fn a_stake_under_the_minimum_is_said_so() {
        let one = stored(1, None);

        assert!(
            recommendations(&[&one])
                .unwrap()
                .contains("mise sous le minimum")
        );
    }

    #[test]
    fn a_long_list_is_cut_below_the_discord_limit_and_says_how_many_were_left_out() {
        let many: Vec<StoredRecommendation> = (0..40).map(|i| stored(i, Some(10.0))).collect();
        let refs: Vec<&StoredRecommendation> = many.iter().collect();

        let text = recommendations(&refs).unwrap();

        assert!(
            text.chars().count() <= MAX_CHARS,
            "{}",
            text.chars().count()
        );
        assert!(text.contains("autre(s) : voir le rapport joint"));
    }
}
