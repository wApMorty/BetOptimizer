//! Choix des tickets d'une grille : le meilleur ticket restant, tant qu'il franchit le seuil (`docs/sources.md` §69 et §70).
//!
//! Règle, dans le scénario défavorable (« S3 ») : on prend le ticket de plus grande valeur marginale parmi les `3^n`
//! combinaisons, partage entre nos tickets compris (`lotofoot::exact`), tant qu'elle atteint le seuil du prix dynamique
//! (`lotofoot::price`) et qu'il reste du budget, 32 tickets au plus par grille. La séance passe les grilles dans l'ordre de
//! clôture (`lotofoot::session`). Aucun tirage au hasard : même grille, mêmes paramètres, mêmes tickets.

use super::exact::ExactValues;
use super::model::{Competitors, GridModel, TicketValue};
use super::{GridId, Ticket};

/// Nombre maximal de tickets retenus par grille, choix de Paul du 2026-10-31 (§70).
pub const MAX_TICKETS: usize = 32;

/// Un ticket retenu : sa valeur dans les deux scénarios et ce qui l'a fait retenir.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Grille du ticket : formule et numéro.
    pub grid: GridId,
    /// Le ticket.
    pub ticket: Ticket,
    /// Valeur dans le modèle de base.
    pub base: TicketValue,
    /// Valeur dans le scénario défavorable, le ticket seul.
    pub pessimistic: TicketValue,
    /// Valeur marginale défavorable au moment du choix, partage avec les tickets précédents de la grille déduit.
    pub marginal: f64,
    /// Seuil qu'il fallait atteindre.
    pub threshold: f64,
    /// Euros restants dans le mois juste avant ce ticket.
    pub budget_left: u32,
}

/// Tickets d'une grille, du meilleur au moins bon. `values` : tables de la grille dans le scénario défavorable ;
/// `threshold` : seuil selon les euros restants. Peut ne retenir aucun ticket.
pub fn pick(
    key: GridId,
    grid: &GridModel,
    competitors: &Competitors,
    values: &mut ExactValues,
    budget_left: u32,
    threshold: &dyn Fn(u32) -> f64,
) -> Vec<Candidate> {
    let worse = grid.pessimistic();
    let worse_competitors = competitors.pessimistic();
    let mut picked = Vec::new();
    let mut left = budget_left;
    while left > 0 && picked.len() < MAX_TICKETS {
        let Some((ticket, marginal)) = values.best() else {
            break;
        };
        let bar = threshold(left);
        if marginal < bar {
            break;
        }
        values.take(&ticket);
        picked.push(Candidate {
            grid: key,
            base: grid.value(&ticket, competitors),
            pessimistic: worse.value(&ticket, &worse_competitors),
            ticket,
            marginal,
            threshold: bar,
            budget_left: left,
        });
        left -= 1;
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Euros, Probs1x2};
    use crate::lotofoot::model::FROZEN;
    use crate::lotofoot::{Game, grid_of_2026};

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

    const REFERENCE_TICKETS: [&str; 16] = [
        "1222122", "1222112", "122212N", "12221N2", "1N22122", "122211N", "122N122", "1N22112",
        "12N2112", "12N2122", "12221NN", "122N112", "1N2212N", "1N221N2", "122N12N", "1212112",
    ];
    const REFERENCE_VALUES: [f64; 16] = [
        3.133151, 2.973599, 2.772238, 2.749688, 2.640132, 2.626644, 2.550676, 2.510467, 2.491117,
        2.439600, 2.433216, 2.429918, 2.335666, 2.314456, 2.256959, 2.247879,
    ];

    fn choose(budget: u32, threshold: &dyn Fn(u32) -> f64) -> Vec<Candidate> {
        let grid = grid();
        let mut values = ExactValues::new(&grid.pessimistic(), &FROZEN.pessimistic());
        let key = grid_of_2026(Game::Lf7, 1);
        pick(key, &grid, &FROZEN, &mut values, budget, threshold)
    }

    /// Mêmes tickets et mêmes valeurs que l'étude : `net_core.Table(…).greedy(k, copies=False)` sur cette grille.
    #[test]
    fn tickets_match_the_study_search() {
        let chosen = choose(16, &|_| 0.0);
        let tickets: Vec<String> = chosen.iter().map(|c| c.ticket.to_string()).collect();
        assert_eq!(tickets, REFERENCE_TICKETS);
        for (c, expected) in chosen.iter().zip(REFERENCE_VALUES) {
            approx::assert_relative_eq!(c.marginal, expected, max_relative = 1e-4);
        }
    }

    #[test]
    fn tickets_are_distinct_reproducible_and_ordered() {
        let first = choose(40, &|_| 0.0);
        assert_eq!(first.len(), MAX_TICKETS);
        let mut unique: Vec<_> = first.iter().map(|c| c.ticket.clone()).collect();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), MAX_TICKETS);
        assert!(
            first
                .windows(2)
                .all(|w| w[1].marginal <= w[0].marginal + 1e-9)
        );
        assert_eq!(choose(40, &|_| 0.0), first);
    }

    #[test]
    fn the_budget_and_the_threshold_stop_the_search() {
        assert_eq!(choose(3, &|_| 0.0).len(), 3);
        assert!(choose(0, &|_| 0.0).is_empty());
        // Un seuil inatteignable ne retient rien : la séance peut ne rien jouer.
        assert!(choose(16, &|_| 1e9).is_empty());
        let chosen = choose(16, &|_| 1.5);
        assert!(chosen.iter().all(|c| c.marginal >= 1.5));
    }

    /// Plus le budget baisse, plus le seuil monte : un seuil fonction de `b` arrête la grille avant son plafond.
    #[test]
    fn the_threshold_follows_the_budget_left() {
        let high = choose(16, &|_| 0.0)[3].marginal + 1e-6;
        let bar = move |left: u32| if left > 13 { 0.0 } else { high };
        let chosen = choose(16, &bar);
        assert_eq!(chosen.len(), 3);
        assert_eq!(
            chosen.iter().map(|c| c.budget_left).collect::<Vec<_>>(),
            [16, 15, 14]
        );
    }
}
