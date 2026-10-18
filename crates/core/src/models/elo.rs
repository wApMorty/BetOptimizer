//! Elo de tennis : un classement général et un classement par surface, mélangés.
//!
//! Formule d'Elo (1978), échelle de 400 points : la probabilité que `a` batte `b` vaut
//!
//! ```text
//! P(a bat b) = 1 / (1 + 10^((r_b − r_a) / 400))
//! ```
//!
//! Après un match, le vainqueur gagne `K·(1 − P(il gagnait))` et le perdant perd la même quantité calculée avec son `K`.
//!
//! Sources vérifiées le 2026-10-18 (Tennis Abstract, « An Introduction to Tennis Elo », J. Sackmann, 2019) : formule
//! ci-dessus, départ à 1500, quatre classements par joueur (général, dur, terre, gazon), prévision par un **mélange
//! 50/50** du classement de surface et du classement général ; aucun réglage de `K` n'y a battu un `K` simple.
//!
//! Choix propres à ce projet, réglés sur la validation puis gelés :
//! - `K` décroît avec l'expérience : `K(n) = k_base / (n + 5)^k_shape` (un joueur neuf bouge vite, un joueur installé
//!   lentement) ; ni la source ci-dessus ni moi n'avons pu citer une source pour cette forme précise, c'est un choix ;
//! - le poids du classement de surface (`surface_weight`) n'est pas fixé à 0,5 mais réglé ;
//! - un classement de surface naît **égal au classement général** du joueur au moment où il apparaît sur cette surface
//!   (plutôt que 1500, qui sous-estimerait un bon joueur sur une surface nouvelle).
//!
//! Un joueur est identifié par son nom publié (« Federer R. ») : voir le contrôle des homonymes dans
//! `ingest::tennis::quality`.

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::ingest::tennis::{Status, Surface, Tour, round_rank};
use crate::store::tennis::StoredMatch;

/// Classement de départ d'un joueur inconnu.
pub const INITIAL_RATING: f64 = 1500.0;

/// Échelle d'Elo : un écart de 400 points donne 10 chances contre 1.
const SCALE: f64 = 400.0;

/// Décalage de l'expérience dans `K` : évite une division par zéro et borne le `K` du premier match.
const K_OFFSET: f64 = 5.0;

/// Hyperparamètres de l'Elo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EloConfig {
    /// Numérateur de `K`.
    pub k_base: f64,
    /// Exposant de la décroissance de `K` avec le nombre de matchs joués.
    pub k_shape: f64,
    /// Poids du classement de surface dans le mélange (0 = général seul, 1 = surface seule).
    pub surface_weight: f64,
}

impl EloConfig {
    /// Départ raisonnable avant réglage : mélange 50/50 de Sackmann.
    pub fn starting_point() -> EloConfig {
        EloConfig {
            k_base: 250.0,
            k_shape: 0.4,
            surface_weight: 0.5,
        }
    }

    /// `K(n) = k_base / (n + 5)^k_shape`, où `n` est le nombre de matchs déjà joués avec ce classement.
    pub fn k(&self, matches_played: u32) -> f64 {
        self.k_base / (f64::from(matches_played) + K_OFFSET).powf(self.k_shape)
    }
}

/// Un classement et le nombre de matchs qui l'ont construit.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rating {
    value: f64,
    matches: u32,
}

/// L'état de l'Elo d'un circuit : tous les joueurs vus jusqu'ici.
#[derive(Debug, Clone)]
pub struct Elo {
    config: EloConfig,
    general: HashMap<String, Rating>,
    by_surface: HashMap<(String, Surface), Rating>,
}

/// `1 / (1 + 10^(−écart / 400))` : probabilité de victoire du joueur dont le classement dépasse l'autre de `écart`.
fn expected_score(gap: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-gap / SCALE))
}

impl Elo {
    /// Un Elo vide.
    pub fn new(config: EloConfig) -> Elo {
        Elo {
            config,
            general: HashMap::new(),
            by_surface: HashMap::new(),
        }
    }

    /// Matchs déjà joués par le joueur (classement général).
    pub fn experience(&self, player: &str) -> u32 {
        self.general.get(player).map_or(0, |r| r.matches)
    }

    fn general_rating(&self, player: &str) -> Rating {
        self.general.get(player).copied().unwrap_or(Rating {
            value: INITIAL_RATING,
            matches: 0,
        })
    }

    /// Classement de surface, ou le classement général si le joueur n'a jamais joué sur cette surface.
    fn surface_rating(&self, player: &str, surface: Surface) -> Rating {
        self.by_surface
            .get(&(player.to_string(), surface))
            .copied()
            .unwrap_or(Rating {
                value: self.general_rating(player).value,
                matches: 0,
            })
    }

    /// Classement utilisé pour la prévision : `(1 − w)·général + w·surface`.
    pub fn rating(&self, player: &str, surface: Surface) -> f64 {
        let w = self.config.surface_weight;
        (1.0 - w) * self.general_rating(player).value
            + w * self.surface_rating(player, surface).value
    }

    /// Probabilité que `a` batte `b` sur cette surface.
    pub fn probability(&self, a: &str, b: &str, surface: Surface) -> f64 {
        expected_score(self.rating(a, surface) - self.rating(b, surface))
    }

    /// Enregistre un résultat : met à jour le classement général et celui de la surface, chacun avec sa propre prévision.
    pub fn update(&mut self, winner: &str, loser: &str, surface: Surface) {
        let (general_winner, general_loser) =
            (self.general_rating(winner), self.general_rating(loser));
        let (surface_winner, surface_loser) = (
            self.surface_rating(winner, surface),
            self.surface_rating(loser, surface),
        );
        let expected_general = expected_score(general_winner.value - general_loser.value);
        let expected_surface = expected_score(surface_winner.value - surface_loser.value);

        let moved = |rating: Rating, expected: f64, won: bool| {
            let k = self.config.k(rating.matches);
            let delta = if won {
                k * (1.0 - expected)
            } else {
                -k * (1.0 - expected)
            };
            Rating {
                value: rating.value + delta,
                matches: rating.matches + 1,
            }
        };
        let updates = [
            (winner, moved(general_winner, expected_general, true)),
            (loser, moved(general_loser, expected_general, false)),
        ];
        let surface_updates = [
            (winner, moved(surface_winner, expected_surface, true)),
            (loser, moved(surface_loser, expected_surface, false)),
        ];
        for (name, rating) in updates {
            self.general.insert(name.to_string(), rating);
        }
        for (name, rating) in surface_updates {
            self.by_surface.insert((name.to_string(), surface), rating);
        }
    }
}

/// La prévision d'un match, faite avant que son résultat ne soit connu.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// Identifiant du match en base.
    pub match_id: i64,
    /// Date du match.
    pub date: NaiveDate,
    /// Circuit.
    pub tour: Tour,
    /// Surface.
    pub surface: Surface,
    /// Nombre maximal de sets.
    pub best_of: u8,
    /// Terminé, ou abandon (un forfait n'est jamais prévu).
    pub status: Status,
    /// Probabilité que le **vainqueur réel** gagne, selon l'Elo : elle ne dépend que des classements d'avant le match.
    pub p_winner: f64,
    /// Expérience du moins expérimenté des deux joueurs (matchs déjà joués) : un Elo neuf n'a pas de sens.
    pub min_experience: u32,
}

/// Rejoue tous les matchs dans l'ordre chronologique et prévoit chacun avec les classements **d'avant sa vague**.
///
/// - une « vague » = une date et un tour de tournoi (voir [`round_rank`]) : toutes les prévisions d'une vague sont faites
///   avant d'appliquer ses résultats, donc un résultat ne peut pas influencer la prévision d'un match de la même vague
///   (aucune fuite du futur). À date égale, un tour précédent passe avant : la demi-finale que tennis-data date du
///   jour de la finale compte pour prévoir la finale. Les matchs reçus dans le désordre sont triés (tri stable) ;
/// - un forfait n'est ni prévu ni compté ; un abandon est prévu (la prévision précède l'abandon) mais ne modifie pas les
///   classements : il ne dit rien de la force relative ;
/// - un circuit = un Elo : ATP et WTA ne se rencontrent jamais.
pub fn walk_forward(matches: &[StoredMatch], config: EloConfig) -> Vec<Prediction> {
    let mut states: HashMap<Tour, Elo> = Tour::ALL
        .into_iter()
        .map(|t| (t, Elo::new(config)))
        .collect();
    let mut predictions = Vec::with_capacity(matches.len());
    let wave = |m: &StoredMatch| (m.date, round_rank(&m.round));
    let mut ordered: Vec<&StoredMatch> = matches.iter().collect();
    ordered.sort_by_key(|m| wave(m));
    let mut start = 0;
    while start < ordered.len() {
        let key = wave(ordered[start]);
        let end = start
            + ordered[start..]
                .iter()
                .take_while(|m| wave(m) == key)
                .count();
        let today = &ordered[start..end];

        for m in today.iter().filter(|m| m.status != Status::Walkover) {
            let Some(elo) = states.get(&m.tour) else {
                continue;
            };
            predictions.push(Prediction {
                match_id: m.id,
                date: m.date,
                tour: m.tour,
                surface: m.surface,
                best_of: m.best_of,
                status: m.status,
                p_winner: elo.probability(&m.winner, &m.loser, m.surface),
                min_experience: elo.experience(&m.winner).min(elo.experience(&m.loser)),
            });
        }
        for m in today.iter().filter(|m| m.status == Status::Completed) {
            if let Some(elo) = states.get_mut(&m.tour) {
                elo.update(&m.winner, &m.loser, m.surface);
            }
        }
        start = end;
    }
    predictions
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn config() -> EloConfig {
        EloConfig::starting_point()
    }

    fn game(
        id: i64,
        date: &str,
        winner: &str,
        loser: &str,
        surface: Surface,
        status: Status,
    ) -> StoredMatch {
        StoredMatch {
            id,
            tour: Tour::Atp,
            season: 2024,
            tournament: "Fake Open".into(),
            series: None,
            date: date.parse().unwrap(),
            surface,
            round: "1st Round".into(),
            best_of: 3,
            winner: winner.into(),
            loser: loser.into(),
            status,
            odds: HashMap::new(),
        }
    }

    #[test]
    fn equal_ratings_give_one_chance_in_two() {
        let elo = Elo::new(config());

        assert_abs_diff_eq!(
            elo.probability("A", "B", Surface::Hard),
            0.5,
            epsilon = 1e-12
        );
    }

    #[test]
    fn a_gap_of_256_points_gives_about_81_percent_as_in_the_published_example() {
        // Tennis Abstract : « un écart de 256 points donne au mieux classé environ 81,4 % ».
        assert_abs_diff_eq!(expected_score(256.0), 0.814, epsilon = 1e-3);
        assert_abs_diff_eq!(expected_score(400.0), 10.0 / 11.0, epsilon = 1e-12);
        assert_abs_diff_eq!(
            expected_score(-120.0) + expected_score(120.0),
            1.0,
            epsilon = 1e-12
        );
    }

    #[test]
    fn k_decreases_with_experience() {
        let c = config();

        assert!(c.k(0) > c.k(10));
        assert!(c.k(10) > c.k(500));
        assert_abs_diff_eq!(c.k(0), 250.0 / 5f64.powf(0.4), epsilon = 1e-12);
    }

    #[test]
    fn a_win_between_equals_moves_both_ratings_by_the_same_amount_in_opposite_directions() {
        let mut elo = Elo::new(config());

        elo.update("A", "B", Surface::Clay);

        let up = elo.general_rating("A").value - INITIAL_RATING;
        let down = INITIAL_RATING - elo.general_rating("B").value;
        assert!(up > 0.0);
        assert_abs_diff_eq!(up, down, epsilon = 1e-9);
        // Prévision à 50 %, donc le gain vaut K·0,5.
        assert_abs_diff_eq!(up, config().k(0) * 0.5, epsilon = 1e-9);
        assert_eq!(elo.experience("A"), 1);
    }

    #[test]
    fn an_upset_moves_the_winner_more_than_an_expected_win() {
        let mut elo = Elo::new(config());
        for _ in 0..30 {
            elo.update("Favori", "Outsider", Surface::Hard);
        }
        let mut expected = elo.clone();
        let mut upset = elo.clone();

        expected.update("Favori", "Outsider", Surface::Hard);
        upset.update("Outsider", "Favori", Surface::Hard);

        let gain_expected =
            expected.general_rating("Favori").value - elo.general_rating("Favori").value;
        let gain_upset =
            upset.general_rating("Outsider").value - elo.general_rating("Outsider").value;
        assert!(gain_expected > 0.0);
        assert!(
            gain_upset > 3.0 * gain_expected,
            "{gain_upset} contre {gain_expected}"
        );
    }

    #[test]
    fn a_new_surface_rating_starts_from_the_general_rating() {
        let mut elo = Elo::new(config());
        for _ in 0..20 {
            elo.update("Star", "Faire-valoir", Surface::Hard);
        }
        let general = elo.general_rating("Star").value;
        assert!(general > INITIAL_RATING + 50.0);

        // Jamais vu sur gazon : son classement de gazon est son classement général, pas 1500.
        assert_abs_diff_eq!(
            elo.surface_rating("Star", Surface::Grass).value,
            general,
            epsilon = 1e-12
        );
        assert_abs_diff_eq!(elo.rating("Star", Surface::Grass), general, epsilon = 1e-9);
    }

    #[test]
    fn the_surface_weight_selects_between_general_and_surface_ratings() {
        let mut general_only = Elo::new(EloConfig {
            surface_weight: 0.0,
            ..config()
        });
        let mut surface_only = Elo::new(EloConfig {
            surface_weight: 1.0,
            ..config()
        });
        for elo in [&mut general_only, &mut surface_only] {
            for _ in 0..15 {
                elo.update("Terrien", "Autre", Surface::Clay);
            }
            for _ in 0..15 {
                elo.update("Autre", "Terrien", Surface::Hard);
            }
        }

        // Sur terre, le « terrien » est bien meilleur que sur dur : seul le classement de surface le voit.
        let by_surface = surface_only.probability("Terrien", "Autre", Surface::Clay);
        let overall = general_only.probability("Terrien", "Autre", Surface::Clay);
        assert!(by_surface > overall + 0.05, "{by_surface} contre {overall}");
    }

    #[test]
    fn predictions_use_only_the_ratings_from_before_the_day() {
        let matches = vec![
            game(1, "2024-01-01", "A", "B", Surface::Hard, Status::Completed),
            game(2, "2024-01-01", "A", "C", Surface::Hard, Status::Completed),
            game(3, "2024-01-02", "A", "B", Surface::Hard, Status::Completed),
        ];

        let predictions = walk_forward(&matches, config());

        // Le 1er janvier, tout le monde est à 1500 : deux fois 50 %, quel que soit l'ordre dans la journée.
        assert_abs_diff_eq!(predictions[0].p_winner, 0.5, epsilon = 1e-12);
        assert_abs_diff_eq!(predictions[1].p_winner, 0.5, epsilon = 1e-12);
        // Le 2, A a gagné deux fois : il est favori.
        assert!(predictions[2].p_winner > 0.5);
        assert_eq!(predictions[2].min_experience, 1);
    }

    #[test]
    fn a_semifinal_dated_like_the_final_still_counts_for_the_final() {
        let semifinal = StoredMatch {
            round: "Semifinals".into(),
            ..game(1, "2024-01-07", "A", "B", Surface::Hard, Status::Completed)
        };
        let final_ = StoredMatch {
            round: "The Final".into(),
            ..game(2, "2024-01-07", "A", "C", Surface::Hard, Status::Completed)
        };

        // Reçus dans le désordre : la finale d'abord.
        let predictions = walk_forward(&[final_, semifinal], config());

        let ids: Vec<i64> = predictions.iter().map(|p| p.match_id).collect();
        assert_eq!(
            ids,
            vec![1, 2],
            "la demi-finale est rejouée avant la finale"
        );
        assert_abs_diff_eq!(predictions[0].p_winner, 0.5, epsilon = 1e-12);
        assert!(
            predictions[1].p_winner > 0.5,
            "A a gagné sa demi-finale : il est favori de la finale"
        );
    }

    #[test]
    fn rewriting_the_future_never_changes_an_earlier_prediction() {
        let past = vec![
            game(1, "2024-01-01", "A", "B", Surface::Hard, Status::Completed),
            game(2, "2024-01-03", "B", "C", Surface::Clay, Status::Completed),
            game(3, "2024-01-05", "A", "C", Surface::Hard, Status::Completed),
        ];
        let mut with_other_future = past.clone();
        // Après le 5 janvier, l'histoire change du tout au tout.
        with_other_future.push(game(
            4,
            "2024-02-01",
            "C",
            "A",
            Surface::Hard,
            Status::Completed,
        ));
        let mut with_another_future = past.clone();
        with_another_future.push(game(
            4,
            "2024-02-01",
            "B",
            "A",
            Surface::Grass,
            Status::Completed,
        ));

        let reference = walk_forward(&past, config());
        let first = walk_forward(&with_other_future, config());
        let second = walk_forward(&with_another_future, config());

        for (a, (b, c)) in reference.iter().zip(first.iter().zip(second.iter())) {
            assert_eq!(a.p_winner, b.p_winner, "match {}", a.match_id);
            assert_eq!(a.p_winner, c.p_winner, "match {}", a.match_id);
        }
        // Et le futur, lui, change bien (le test de fuite a des dents).
        assert_ne!(first[3].p_winner, second[3].p_winner);
    }

    #[test]
    fn walkovers_are_neither_predicted_nor_counted_and_retirements_are_predicted_but_do_not_move_ratings()
     {
        let matches = vec![
            game(1, "2024-01-01", "A", "B", Surface::Hard, Status::Walkover),
            game(2, "2024-01-02", "A", "B", Surface::Hard, Status::Retired),
            game(3, "2024-01-03", "A", "B", Surface::Hard, Status::Completed),
        ];

        let predictions = walk_forward(&matches, config());

        let ids: Vec<i64> = predictions.iter().map(|p| p.match_id).collect();
        assert_eq!(ids, vec![2, 3]);
        // Ni le forfait ni l'abandon n'ont bougé les classements : la prévision du 3 est encore 50 %.
        assert_abs_diff_eq!(predictions[1].p_winner, 0.5, epsilon = 1e-12);
        assert_eq!(predictions[1].min_experience, 0);
    }

    #[test]
    fn the_two_tours_have_separate_ratings() {
        let mut wta = game(2, "2024-01-02", "A", "B", Surface::Hard, Status::Completed);
        wta.tour = Tour::Wta;
        let matches = vec![
            game(1, "2024-01-01", "A", "B", Surface::Hard, Status::Completed),
            wta,
        ];

        let predictions = walk_forward(&matches, config());

        // « A » de la WTA n'est pas « A » de l'ATP : son premier match est prévu à 50 %.
        assert_abs_diff_eq!(predictions[1].p_winner, 0.5, epsilon = 1e-12);
    }
}
