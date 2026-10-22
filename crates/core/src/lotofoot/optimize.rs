//! Choix des tickets : recherche gloutonne par grille, puis sélection de la semaine sous un seuil et un budget.
//!
//! Règle retenue le 2026-10-22 (`docs/sources.md` §37) :
//! 1. sur chaque grille, 16 tickets distincts choisis un à un, chacun maximisant `EV(t) + μ · P(t payé)` avec `μ = 10`
//!    (le terme en `μ` achète un peu de fréquence de gain au prix d'un peu d'espérance) ;
//! 2. chaque ticket est réévalué dans le scénario défavorable (« S3 ») ;
//! 3. sur toutes les grilles de la séance, on garde les tickets dont l'espérance défavorable atteint `τ = 1,2`, les
//!    meilleurs d'abord, dans la limite du budget (16 €).

use crate::domain::Outcome;

use super::Ticket;
use super::model::{Competitors, GridModel, TicketValue};

/// Poids de la probabilité de gain dans l'objectif de la recherche gloutonne.
pub const FREQUENCY_WEIGHT: f64 = 10.0;

/// Seuil d'espérance défavorable (S3) en dessous duquel un ticket n'est pas joué.
pub const THRESHOLD: f64 = 1.2;

/// Nombre maximal de tickets d'une séance (budget de 16 € à 1 € le ticket).
pub const MAX_TICKETS: usize = 16;

/// Passes au plus de la recherche locale (chaque passe essaie tous les changements d'un signe).
const LOCAL_PASSES: usize = 6;

/// Tickets d'une grille, du meilleur au moins bon selon la recherche gloutonne.
///
/// Chaque ticket part de trois points de départ (les favoris du marché ; le signe de plus grand rapport `p/q` ; le
/// dernier ticket choisi), s'améliore par changements d'un seul signe tant que l'objectif monte, et doit différer de
/// tous les tickets déjà choisis. Même grille, mêmes paramètres : mêmes tickets (aucun tirage au hasard).
pub fn best_tickets(
    grid: &GridModel,
    competitors: &Competitors,
    count: usize,
    weight: f64,
) -> Vec<Ticket> {
    let n = grid.len();
    let favourites: Vec<usize> = (0..n)
        .map(|i| argmax(grid.market(i).unwrap_or([0.0; 3])))
        .collect();
    let ratios: Vec<usize> = (0..n)
        .map(|i| {
            let p = grid.market(i).unwrap_or([0.0; 3]);
            let q = grid.public(i).unwrap_or([1.0; 3]);
            argmax([0, 1, 2].map(|k| p[k] / q[k].max(1e-12)))
        })
        .collect();
    let objective = |signs: &[usize]| {
        let value = grid.value(&to_ticket(signs), competitors);
        value.expected_return + weight * value.paid_probability
    };

    let mut picked: Vec<Vec<usize>> = Vec::with_capacity(count);
    while picked.len() < count {
        let mut starts = vec![favourites.clone(), ratios.clone()];
        if let Some(last) = picked.last() {
            starts.push(last.clone());
        }
        let mut best: Option<(f64, Vec<usize>)> = None;
        for start in starts {
            let Some(mut ticket) = admissible(start, &picked) else {
                continue;
            };
            let mut current = objective(&ticket);
            for _ in 0..LOCAL_PASSES {
                let mut improved = false;
                for i in 0..n {
                    for sign in 0..3 {
                        if ticket.get(i) == Some(&sign) {
                            continue;
                        }
                        let mut candidate = ticket.clone();
                        candidate[i] = sign;
                        if picked.contains(&candidate) {
                            continue;
                        }
                        let value = objective(&candidate);
                        if value > current + 1e-12 {
                            ticket = candidate;
                            current = value;
                            improved = true;
                        }
                    }
                }
                if !improved {
                    break;
                }
            }
            // Comparaison stricte : à égalité, le premier point de départ l'emporte (résultat reproductible).
            if best.as_ref().is_none_or(|(value, _)| current > *value) {
                best = Some((current, ticket));
            }
        }
        match best {
            Some((_, ticket)) => picked.push(ticket),
            // Aucun point de départ admissible : premier ticket libre dans l'ordre des codes (cas extrême, petite grille).
            None => match first_free(n, &picked) {
                Some(ticket) => picked.push(ticket),
                None => break,
            },
        }
    }
    picked.iter().map(|signs| to_ticket(signs)).collect()
}

/// Un ticket candidat d'une séance, évalué dans le modèle de base et dans le scénario défavorable.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate<K> {
    /// Grille du ticket (le type de la clé est libre : couple formule et numéro en production).
    pub grid: K,
    /// Le ticket.
    pub ticket: Ticket,
    /// Valeur dans le modèle de base.
    pub base: TicketValue,
    /// Valeur dans le scénario défavorable.
    pub pessimistic: TicketValue,
}

/// Candidats d'une grille : ses `MAX_TICKETS` meilleurs tickets pour le poids de fréquence `weight`, évalués dans les
/// deux scénarios.
pub fn candidates<K: Clone>(
    key: &K,
    grid: &GridModel,
    competitors: &Competitors,
    weight: f64,
) -> Vec<Candidate<K>> {
    let worse = grid.pessimistic();
    let worse_competitors = competitors.pessimistic();
    best_tickets(grid, competitors, MAX_TICKETS, weight)
        .into_iter()
        .map(|ticket| Candidate {
            grid: key.clone(),
            base: grid.value(&ticket, competitors),
            pessimistic: worse.value(&ticket, &worse_competitors),
            ticket,
        })
        .collect()
}

/// Sélection de la séance : espérance défavorable au moins égale à `threshold`, par espérance défavorable décroissante,
/// `max_tickets` au plus. Une séance peut ne retenir aucun ticket.
pub fn select<K>(
    mut candidates: Vec<Candidate<K>>,
    threshold: f64,
    max_tickets: usize,
) -> Vec<Candidate<K>> {
    candidates.retain(|c| c.pessimistic.expected_return >= threshold);
    // `total_cmp` ordonne aussi les flottants spéciaux : le tri ne peut pas paniquer. À égalité, le ticket le plus
    // petit dans l'ordre des codes passe d'abord (sélection reproductible).
    candidates.sort_by(|a, b| {
        b.pessimistic
            .expected_return
            .total_cmp(&a.pessimistic.expected_return)
            .then_with(|| a.ticket.cmp(&b.ticket))
    });
    candidates.truncate(max_tickets);
    candidates
}

/// Signe de plus grande valeur (le premier en cas d'égalité).
fn argmax(values: [f64; 3]) -> usize {
    let mut best = 0;
    for (k, value) in values.iter().enumerate() {
        if *value > values[best] {
            best = k;
        }
    }
    best
}

/// Rend un point de départ admissible (différent des tickets choisis) en changeant au besoin un signe ; `None` si
/// aucun changement d'un signe n'y suffit.
fn admissible(start: Vec<usize>, picked: &[Vec<usize>]) -> Option<Vec<usize>> {
    if !picked.contains(&start) {
        return Some(start);
    }
    for i in 0..start.len() {
        let mut candidate = start.clone();
        candidate[i] = (candidate[i] + 1) % 3;
        if !picked.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Premier ticket non encore choisi dans l'ordre des codes (base 3), ou `None` si tous le sont.
fn first_free(n: usize, picked: &[Vec<usize>]) -> Option<Vec<usize>> {
    let total = 3usize.checked_pow(u32::try_from(n).ok()?)?;
    (0..total)
        .map(|code| {
            let mut rest = code;
            (0..n)
                .map(|_| {
                    let sign = rest % 3;
                    rest /= 3;
                    sign
                })
                .collect::<Vec<usize>>()
        })
        .find(|ticket| !picked.contains(ticket))
}

fn to_ticket(signs: &[usize]) -> Ticket {
    Ticket::new(signs.iter().map(|&k| Outcome::ALL[k % 3]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Euros, Probs1x2};
    use crate::lotofoot::Game;
    use crate::lotofoot::model::FROZEN;

    fn grid() -> GridModel {
        let rows_p = [
            [0.47, 0.23, 0.30],
            [0.43, 0.25, 0.32],
            [0.14, 0.18, 0.68],
            [0.35, 0.27, 0.38],
            [0.50, 0.25, 0.25],
            [0.45, 0.24, 0.31],
            [0.45, 0.26, 0.29],
        ];
        let rows_q = [
            [0.31, 0.24, 0.45],
            [0.61, 0.19, 0.20],
            [0.14, 0.17, 0.69],
            [0.45, 0.26, 0.29],
            [0.35, 0.25, 0.40],
            [0.47, 0.25, 0.28],
            [0.61, 0.20, 0.19],
        ];
        let p: Vec<_> = rows_p
            .iter()
            .map(|r| Probs1x2::new(r[0], r[1], r[2]).unwrap())
            .collect();
        let q: Vec<_> = rows_q
            .iter()
            .map(|r| Probs1x2::new(r[0], r[1], r[2]).unwrap())
            .collect();
        let stakes = Euros::new(160_000.0).unwrap();
        GridModel::new(
            &p,
            &q,
            stakes,
            &GridModel::masses_for(Game::Lf7, stakes, None),
        )
        .unwrap()
    }

    #[test]
    fn tickets_are_distinct_and_reproducible() {
        let first = best_tickets(&grid(), &FROZEN, 16, FREQUENCY_WEIGHT);
        assert_eq!(first.len(), 16);
        let mut unique = first.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 16);
        assert_eq!(best_tickets(&grid(), &FROZEN, 16, FREQUENCY_WEIGHT), first);
    }

    /// Mêmes tickets que la recherche de l'étude (`scripts/lotofoot/budget16.py`, `greedy(…, 16, 1)`), dont dépendent
    /// les chiffres de la stratégie.
    #[test]
    fn tickets_match_the_study_search() {
        let expected = [
            "1222122", "1222112", "122212N", "12N2112", "12N2122", "1222121", "122211N", "N222122",
            "1222111", "1N22122", "1N2212N", "N222112", "1N22121", "1N22112", "1N2211N", "N22212N",
        ];
        let tickets: Vec<String> = best_tickets(&grid(), &FROZEN, 16, FREQUENCY_WEIGHT)
            .iter()
            .map(Ticket::to_string)
            .collect();
        assert_eq!(tickets, expected);
    }

    #[test]
    fn the_first_ticket_is_a_local_optimum_of_the_objective() {
        let grid = grid();
        let ticket = best_tickets(&grid, &FROZEN, 1, FREQUENCY_WEIGHT).remove(0);
        let objective = |t: &Ticket| {
            let v = grid.value(t, &FROZEN);
            v.expected_return + FREQUENCY_WEIGHT * v.paid_probability
        };
        let best = objective(&ticket);
        for i in 0..ticket.len() {
            for sign in Outcome::ALL {
                let mut signs = ticket.signs().to_vec();
                signs[i] = sign;
                assert!(objective(&Ticket::new(signs)) <= best + 1e-9);
            }
        }
    }

    #[test]
    fn selection_respects_the_threshold_the_budget_and_the_order() {
        let cands = candidates(&"grille", &grid(), &FROZEN, FREQUENCY_WEIGHT);
        let chosen = select(cands.clone(), 1.2, 5);
        assert!(chosen.len() <= 5);
        assert!(chosen.iter().all(|c| c.pessimistic.expected_return >= 1.2));
        assert!(
            chosen
                .windows(2)
                .all(|w| w[0].pessimistic.expected_return >= w[1].pessimistic.expected_return)
        );
        // Un seuil inatteignable ne retient rien : la séance peut ne rien jouer.
        assert!(select(cands, 1e9, 16).is_empty());
    }

    #[test]
    fn a_tiny_grid_runs_out_of_tickets_without_looping() {
        let p = vec![Probs1x2::new(0.5, 0.3, 0.2).unwrap(); 2];
        let stakes = Euros::new(100.0).unwrap();
        let masses = [Euros::new(70.0).unwrap()];
        let grid = GridModel::new(&p, &p, stakes, &masses).unwrap();
        // 3^2 = 9 tickets possibles : on en demande 12, on en obtient 9, tous distincts.
        let tickets = best_tickets(&grid, &FROZEN, 12, 0.0);
        assert_eq!(tickets.len(), 9);
    }
}
