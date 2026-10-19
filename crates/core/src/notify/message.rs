//! Texte d'une notification : court, lisible sur un écran de téléphone, limité à la taille d'un message Discord.

use chrono::{DateTime, Utc};

use crate::betting::kelly::{kelly_fraction, to_units};
use crate::betting::live::Opportunity;
use crate::format::{decimal, kickoff_label, long_date, outcome_label, signed, units};
use crate::store::journal::StoredRecommendation;

/// Longueur maximale d'un message Discord (caractères).
pub const MAX_CHARS: usize = 2000;

/// Marge gardée pour la ligne finale « et N autres ».
const RESERVED: usize = 100;

/// Nombre de prix « les plus proches de la valeur » montrés dans le message.
pub const CLOSEST_PRICES: usize = 5;

/// Ce que le message du jour sait dire d'une exécution de `edge recommend`.
pub struct Daily<'a> {
    /// Instant de l'exécution.
    pub now: DateTime<Utc>,
    /// Matchs à venir dans un relevé récent.
    pub events: usize,
    /// Matchs sans 1X2 complet de Pinnacle.
    pub without_reference: usize,
    /// Avantage minimal d'une recommandation.
    pub min_edge: f64,
    /// Fraction de Kelly choisie (1 = Kelly complet).
    pub kelly_fraction: f64,
    /// Bankroll courante, en euros : l'unité « u » en vaut 1 %.
    pub bankroll: f64,
    /// Nouvelles recommandations de l'exécution.
    pub recommended: &'a [&'a StoredRecommendation],
    /// Meilleur prix de chaque match, du plus proche de la valeur au plus éloigné (voir `recommend::Report::priced`).
    pub priced: &'a [Opportunity],
}

/// Message du jour : toujours envoyé, même sans recommandation, pour montrer ce que l'algorithme a regardé.
///
/// Il dit d'abord si l'exécution a eu de quoi travailler (Pinnacle présent ?), puis les recommandations s'il y en a, puis les
/// prix les plus proches de la valeur, **donnés pour information** : un avantage négatif veut dire que le prix est moins bon que le marché.
pub fn daily(daily: &Daily) -> String {
    let compared = daily.events.saturating_sub(daily.without_reference);
    let mut head = format!(
        "**edge** · bilan du {} · {compared} match(s) comparé(s) à Pinnacle",
        long_date(daily.now)
    );
    if daily.events == 0 {
        head.push_str("\n⚠ Aucun match à venir dans un relevé de moins de 30 h : le relevé de 14 h a-t-il tourné ?");
    } else if daily.without_reference * 2 >= daily.events {
        head.push_str(&format!(
            "\n⚠ Pinnacle manque sur {} match(s) sur {} : relevé inutilisable (panne de l'API ?), ces matchs n'ont pas été évalués.",
            daily.without_reference, daily.events
        ));
    }

    // Les sections du bas sont bâties d'abord : elles ont une taille bornée, les recommandations se partagent le reste.
    let mut tail = String::new();
    let shown: Vec<&Opportunity> = daily
        .priced
        .iter()
        .filter(|o| {
            !daily
                .recommended
                .iter()
                .any(|r| r.record.event_id == o.event_id)
        })
        .take(CLOSEST_PRICES)
        .collect();
    if !shown.is_empty() {
        tail.push_str(
            "\n\n**Prix les plus proches de la valeur** (pour information, ce ne sont pas des recommandations) :",
        );
        for (index, found) in shown.iter().enumerate() {
            tail.push_str(&closest_entry(
                index + 1,
                found,
                daily.min_edge,
                daily.kelly_fraction,
            ));
        }
        tail.push_str(
            "\n\n_Avantage = cote × probabilité de Pinnacle sans marge − 1. Négatif : le prix est moins bon que le marché. Liste complète dans le rapport joint._",
        );
    }
    // Sans cette ligne, « 9,1 u » ne veut rien dire : l'unité est dite à chaque message.
    tail.push_str(&format!(
        "\n_1 u = 1 % de la bankroll ({} €) · Kelly ×{}._",
        decimal(daily.bankroll, 0),
        decimal(daily.kelly_fraction, 2)
    ));

    let mut text = head;
    if daily.recommended.is_empty() {
        text.push_str(&format!(
            "\n\nAucune recommandation : aucun prix ne dépasse de {} % la cote juste de Pinnacle.",
            decimal(daily.min_edge * 100.0, 1)
        ));
    } else {
        text.push_str(&format!(
            "\n\n{} recommandation(s) **expérimentale(s)** : aucune règle n'est validée hors échantillon. Vérifie la cote sur le site avant tout pari.",
            daily.recommended.len()
        ));
        // `chars().count()` : la limite de Discord se compte en caractères, pas en octets (les accents en font deux).
        let room = MAX_CHARS - RESERVED - tail.chars().count();
        for (index, found) in daily.recommended.iter().enumerate() {
            let entry = entry(found);
            if text.chars().count() + entry.chars().count() > room {
                text.push_str(&format!(
                    "\n… et {} autre(s) : voir le rapport joint.",
                    daily.recommended.len() - index
                ));
                break;
            }
            text.push_str(&entry);
        }
    }
    text.push_str(&tail);
    text
}

/// Mise Kelly d'un pari seul, en unités, pour un prix donné : nulle sans avantage.
fn kelly_units(found: &Opportunity, fraction: f64) -> f64 {
    let s = &found.selection;
    to_units(kelly_fraction(s.probability, s.odds) * fraction)
}

fn closest_entry(rank: usize, found: &Opportunity, min_edge: f64, fraction: f64) -> String {
    let s = &found.selection;
    let above = if s.edge >= min_edge {
        " ✔ au-dessus du seuil"
    } else {
        ""
    };
    // Sous le seuil, la mise Kelly n'est qu'indicative : « ne pas miser » le dit, pour qu'elle ne passe pas pour un conseil.
    let kelly = if s.edge <= 0.0 {
        "Kelly 0 u (rien à miser)".to_string()
    } else if s.edge >= min_edge {
        format!("Kelly {}", units(kelly_units(found, fraction)))
    } else {
        format!(
            "Kelly {} (sous le seuil : ne pas miser)",
            units(kelly_units(found, fraction))
        )
    };
    format!(
        "\n{rank}. **{}** · {} – {}\n   {} · {} à {} · cote juste {} · cote minimale {} · avantage **{} %** · {kelly}{above}",
        kickoff_label(found.kickoff),
        found.home_team,
        found.away_team,
        found.bookmaker,
        outcome_label(s.outcome),
        decimal(s.odds.get(), 2),
        decimal(1.0 / s.probability.get(), 2),
        decimal(found.minimal_odds, 2),
        signed(s.edge * 100.0, 1),
    )
}

fn entry(found: &StoredRecommendation) -> String {
    let r = &found.record;
    // La mise est celle du Kelly simultané du lot, arrondie à 0,1 € : l'unité s'en déduit avec la bankroll du jour de la recommandation.
    let stake = r.stake.map_or_else(
        || "mise sous le minimum".to_string(),
        |s| {
            format!(
                "mise **{}** ({} €)",
                units(to_units(s.get() / r.bankroll.get())),
                decimal(s.get(), 1)
            )
        },
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

    const NOW: &str = "2026-10-19T12:10:00Z";

    fn priced(event: &str, bookmaker: &str, odds: f64, probability: f64) -> Opportunity {
        Opportunity {
            event_id: event.into(),
            api_call_id: 1,
            sport_key: "soccer_epl".into(),
            home_team: "Arsenal".into(),
            away_team: "Chelsea".into(),
            kickoff: "2026-10-24T13:00:00Z".parse().unwrap(),
            bookmaker: bookmaker.into(),
            selection: crate::betting::value::Selection {
                outcome: Outcome::Home,
                odds: Odds::new(odds).unwrap(),
                probability: Probability::new(probability).unwrap(),
                edge: probability * odds - 1.0,
            },
            minimal_odds: 1.03 / probability,
            priced_at: "2026-10-19T12:00:00Z".parse().unwrap(),
            stake: None,
        }
    }

    fn day<'a>(
        recommended: &'a [&'a StoredRecommendation],
        priced: &'a [Opportunity],
        events: usize,
        without_reference: usize,
    ) -> Daily<'a> {
        Daily {
            now: NOW.parse().unwrap(),
            events,
            without_reference,
            min_edge: 0.03,
            kelly_fraction: 1.0,
            bankroll: 1000.0,
            recommended,
            priced,
        }
    }

    #[test]
    fn with_no_recommendation_the_message_still_says_what_was_looked_at() {
        // Prix de Betclic à 2,10 pour une probabilité de 0,47 : cote juste 2,13, avantage −1,3 %.
        let close = [priced("m1", "betclic_fr", 2.10, 0.47)];

        let text = daily(&day(&[], &close, 107, 1));

        assert!(text.contains("bilan du lundi 19 octobre"), "{text}");
        assert!(text.contains("106 match(s) comparé(s) à Pinnacle"));
        assert!(text.contains("Aucune recommandation : aucun prix ne dépasse de 3,0 %"));
        assert!(text.contains("pas des recommandations"));
        assert!(text.contains("betclic_fr · domicile à 2,10 · cote juste 2,13 · cote minimale 2,19 · avantage **-1,3 %**"), "{text}");
        assert!(!text.contains('⚠'));
        assert!(text.contains("Kelly 0 u (rien à miser)"), "{text}");
    }

    #[test]
    fn the_kelly_stake_is_given_in_units_and_a_stake_below_the_threshold_says_not_to_bet() {
        // +1,3 % à 3,10 : Kelly = 0,013 / 2,1 = 0,62 % de la bankroll = 0,6 u, mais sous le seuil de 3 %.
        // +11,9 % à 2,30 : Kelly = 0,119 / 1,3 = 9,1 % de la bankroll = 9,1 u.
        let close = [
            priced("small", "winamax_fr", 3.10, 1.013 / 3.10),
            priced("big", "betclic_fr", 2.30, 0.4865),
        ];

        let text = daily(&day(&[], &close, 20, 0));

        assert!(
            text.contains("Kelly 0,6 u (sous le seuil : ne pas miser)"),
            "{text}"
        );
        assert!(text.contains("Kelly 9,1 u ✔ au-dessus du seuil"), "{text}");
    }

    #[test]
    fn a_missing_pinnacle_is_flagged_loudly() {
        let text = daily(&day(&[], &[], 95, 95));

        assert!(
            text.contains("⚠ Pinnacle manque sur 95 match(s) sur 95"),
            "{text}"
        );
        assert!(text.contains("0 match(s) comparé(s)"));
    }

    #[test]
    fn an_empty_archive_asks_whether_the_snapshot_ran() {
        let text = daily(&day(&[], &[], 0, 0));

        assert!(text.contains("le relevé de 14 h a-t-il tourné ?"), "{text}");
    }

    #[test]
    fn only_the_five_closest_prices_are_listed_and_a_recommended_match_is_not_repeated() {
        let many: Vec<Opportunity> = (0..9)
            .map(|i| {
                priced(
                    &format!("m{i}"),
                    "winamax_fr",
                    2.0 + 0.1 * f64::from(i),
                    0.5,
                )
            })
            .collect();
        let one = stored(1, Some(10.0));
        let mut already = one.clone();
        already.record.event_id = "m8".into();

        let text = daily(&day(&[&already], &many, 9, 0));

        assert_eq!(text.matches("winamax_fr ·").count(), 5, "{text}");
        assert!(text.contains("1 recommandation(s) **expérimentale(s)**"));
        // m8 est recommandé : il n'apparaît pas deux fois (cote 2,8).
        assert!(!text.contains("domicile à 2,80"), "{text}");
        assert!(text.contains("✔ au-dessus du seuil"));
    }

    #[test]
    fn a_message_carries_the_pick_the_minimal_odds_the_stake_and_the_warning() {
        let one = stored(1, Some(91.4));

        let text = daily(&day(&[&one], &[], 10, 0));

        assert!(text.contains("expérimentale"));
        assert!(text.contains("sam. 24/10 15:00"), "heure de Paris : {text}");
        assert!(text.contains("Arsenal – Chelsea"));
        assert!(text.contains("betclic_fr · domicile à 2,30"));
        assert!(text.contains("cote minimale 2,12"));
        assert!(text.contains("avantage +11,9 %"));
        assert!(text.contains("mise **9,1 u** (91,4 €)"), "{text}");
        assert!(
            text.contains("1 u = 1 % de la bankroll (1000 €) · Kelly ×1,00"),
            "{text}"
        );
    }

    #[test]
    fn a_stake_under_the_minimum_is_said_so() {
        let one = stored(1, None);

        assert!(daily(&day(&[&one], &[], 10, 0)).contains("mise sous le minimum"));
    }

    #[test]
    fn a_long_list_is_cut_below_the_discord_limit_and_says_how_many_were_left_out() {
        let many: Vec<StoredRecommendation> = (0..40).map(|i| stored(i, Some(10.0))).collect();
        let refs: Vec<&StoredRecommendation> = many.iter().collect();
        let close: Vec<Opportunity> = (0..9)
            .map(|i| priced(&format!("x{i}"), "unibet_fr", 2.2, 0.45))
            .collect();

        let text = daily(&day(&refs, &close, 60, 0));

        assert!(
            text.chars().count() <= MAX_CHARS,
            "{}",
            text.chars().count()
        );
        assert!(text.contains("autre(s) : voir le rapport joint"));
        assert!(
            text.contains("Prix les plus proches de la valeur"),
            "les prix du bas restent"
        );
    }
}
