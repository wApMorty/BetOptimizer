//! Valeur marginale exacte de chacun des `3^n` tickets d'une grille, partage entre nos propres tickets compris (§69).
//!
//! Pour un ensemble de tickets distincts `T`, avec `c(o)` nos tickets à une erreur de l'issue `o` :
//! `V(T) = Σ_{t ∈ T} P(t) · M_0 · E[1 / (1 + W_0(t))] + Σ_o P(o) · M_1 · F(c(o), μ_1(o))`,
//! `F(c, μ) = c · E[1 / (c + W)]`, `W ~ Poisson(μ)` (Clair & Letscher 2007). Deux tickets distincts ne sont jamais parfaits
//! ensemble : au sans-faute, le partage entre nous n'existe pas. `F` étant concave en `c`, ajouter le meilleur ticket un par
//! un s'écarte de 0,07 % au plus de la borne de relaxation linéaire sur 24 grilles (`scripts/lotofoot/net_bound.py`).

use crate::domain::Outcome;

use super::Ticket;
use super::model::{Competitors, GridModel, expected_share, poisson_binomial};

/// Intervalles de la méthode de Simpson pour `F` (nombre pair) : 5·10⁻⁶ d'écart relatif au plus à la quadrature de l'étude.
const SIMPSON_INTERVALS: usize = 256;

/// Tables d'une grille dans un scénario, et tickets déjà choisis.
#[derive(Debug, Clone, PartialEq)]
pub struct ExactValues {
    matches: usize,
    /// `P(o) · M_0 · E[1 / (1 + W_0(o))]` pour chaque issue : ce que rapporte le sans-faute du ticket de même code.
    perfect: Vec<f64>,
    /// `P(o) · M_1 · (F(c + 1, μ) − F(c, μ))`, issue par issue et `c` de 0 à `2n − 1`, à plat : `2n` cases par issue.
    one_error: Vec<f64>,
    /// Les `2n` tickets à une erreur de chaque ticket, à plat. Être à une erreur est symétrique : ce sont aussi les issues
    /// où ce ticket fait une erreur.
    neighbours: Vec<usize>,
    /// `c(o)` : nos tickets à une erreur de chaque issue.
    near: Vec<usize>,
    /// Tickets déjà choisis.
    taken: Vec<bool>,
}

impl ExactValues {
    /// Calcule les tables sur les `3^n` issues (6 561 au LF8).
    pub fn new(grid: &GridModel, competitors: &Competitors) -> ExactValues {
        let n = grid.len();
        let width = 2 * n;
        let total = 3usize.pow(n as u32);
        let (first, second) = (grid.mass(0).unwrap_or(0.0), grid.mass(1).unwrap_or(0.0));
        let mut perfect = Vec::with_capacity(total);
        let mut one_error = Vec::with_capacity(total * width);
        let mut neighbours = Vec::with_capacity(total * width);
        for code in 0..total {
            let signs = digits(code, n);
            let (mut prob, mut shares) = (1.0, Vec::with_capacity(n));
            for (i, &sign) in signs.iter().enumerate() {
                prob *= grid.market(i).map_or(0.0, |p| p[sign]);
                shares.push(grid.public(i).map_or(0.0, |q| q[sign]));
            }
            let public = poisson_binomial(&shares);
            let expected = |errors: usize| {
                let lambda = grid.stakes() * public[n - errors];
                competitors.expected(lambda, grid.stakes(), n)
            };
            perfect.push(prob * first * expected_share(expected(0)));
            let mut previous = 0.0;
            for value in shared_parts(width, expected(1)) {
                one_error.push(prob * second * (value - previous));
                previous = value;
            }
            let powers = (0..n).map(|i| 3usize.pow(i as u32));
            for (i, power) in powers.enumerate() {
                for shift in [1, 2] {
                    let other = (signs[i] + shift) % 3;
                    neighbours.push(code + other * power - signs[i] * power);
                }
            }
        }
        ExactValues {
            matches: n,
            perfect,
            one_error,
            neighbours,
            near: vec![0; total],
            taken: vec![false; total],
        }
    }

    /// Ce qu'ajoute le ticket de code `code` à ceux déjà choisis.
    fn marginal(&self, code: usize) -> f64 {
        let width = 2 * self.matches;
        let near = &self.neighbours[code * width..(code + 1) * width];
        // Chaque ticket a `2n` voisins : un ticket non choisi laisse donc `c(o) ≤ 2n − 1` sur ses issues à une erreur.
        let sum: f64 = near
            .iter()
            .map(|&o| self.one_error[o * width + self.near[o]])
            .sum();
        self.perfect[code] + sum
    }

    /// Meilleur ticket restant et sa valeur marginale (le plus petit code à égalité : résultat reproductible). `None` si
    /// tous les tickets sont pris.
    pub fn best(&self) -> Option<(Ticket, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for code in (0..self.taken.len()).filter(|&code| !self.taken[code]) {
            let value = self.marginal(code);
            if best.is_none_or(|(_, top)| value > top) {
                best = Some((code, value));
            }
        }
        best.map(|(code, value)| (ticket_of(code, self.matches), value))
    }

    /// Ajoute un ticket aux tickets choisis. Un ticket déjà choisi ou de mauvaise longueur est sans effet.
    pub fn take(&mut self, ticket: &Ticket) {
        if ticket.len() != self.matches {
            return;
        }
        let code = code_of(ticket);
        if self.taken[code] {
            return;
        }
        self.taken[code] = true;
        let width = 2 * self.matches;
        for k in code * width..(code + 1) * width {
            self.near[self.neighbours[k]] += 1;
        }
    }
}

/// `F(c, μ) = c · E[1 / (c + W)]`, `W ~ Poisson(μ)`, pour `c = 1…count`.
///
/// `E[1 / (c + W)] = ∫₀¹ x^{c−1} e^{−μ(1−x)} dx` (on développe `e^{μx}` et on intègre terme à terme) ; méthode de Simpson sur
/// `[a, 1]`, `a = max(0, 1 − 40/μ)` : l'exponentielle est piquée en `x = 1`, et ce qui est laissé de côté pèse moins de `e^{−40}`.
fn shared_parts(count: usize, mu: f64) -> Vec<f64> {
    let start = (1.0 - 40.0 / mu).max(0.0);
    let step = (1.0 - start) / SIMPSON_INTERVALS as f64;
    let mut integrals = vec![0.0; count];
    for node in 0..=SIMPSON_INTERVALS {
        let x = start + step * node as f64;
        let weight = match node {
            0 => 1.0,
            _ if node == SIMPSON_INTERVALS => 1.0,
            _ if node % 2 == 1 => 4.0,
            _ => 2.0,
        };
        // `x^{c−1}` se multiplie par `x` à chaque `c` : pas de puissance à recalculer.
        let mut term = weight * (-mu * (1.0 - x)).exp();
        for integral in &mut integrals {
            *integral += term;
            term *= x;
        }
    }
    integrals
        .iter()
        .enumerate()
        .map(|(i, integral)| (i + 1) as f64 * integral * step / 3.0)
        .collect()
}

/// Chiffres en base 3 d'un code, rencontre par rencontre : le signe de chacune.
fn digits(code: usize, n: usize) -> Vec<usize> {
    let mut rest = code;
    (0..n)
        .map(|_| {
            let sign = rest % 3;
            rest /= 3;
            sign
        })
        .collect()
}

fn ticket_of(code: usize, n: usize) -> Ticket {
    Ticket::new(digits(code, n).iter().map(|&k| Outcome::ALL[k]).collect())
}

fn code_of(ticket: &Ticket) -> usize {
    ticket
        .signs()
        .iter()
        .rev()
        .fold(0, |code, sign| code * 3 + sign.index())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Euros, Probs1x2};
    use crate::lotofoot::Game;
    use crate::lotofoot::model::FROZEN;
    use approx::assert_relative_eq;

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

    /// `c · E[1 / (c + W)]` par la somme de la loi de Poisson.
    fn by_sum(c: f64, mu: f64) -> f64 {
        let (mut p, mut total) = ((-mu).exp(), 0.0);
        for w in 0..4_000 {
            total += p * c / (c + f64::from(w));
            p *= mu / f64::from(w + 1);
        }
        total
    }

    #[test]
    fn the_share_matches_its_closed_form_and_the_poisson_sum() {
        for mu in [1e-6, 0.01, 0.3, 1.0, 5.0, 30.0, 300.0] {
            let parts = shared_parts(16, mu);
            assert_relative_eq!(parts[0], expected_share(mu), max_relative = 1e-5);
            for c in [2, 5, 16] {
                assert_relative_eq!(parts[c - 1], by_sum(c as f64, mu), max_relative = 1e-5);
            }
        }
    }

    #[test]
    fn the_share_is_concave_in_the_number_of_our_tickets() {
        for mu in [0.3, 5.0, 300.0, 40_000.0] {
            let mut values = vec![0.0];
            values.extend(shared_parts(16, mu));
            assert!(values.windows(3).all(|w| w[2] - w[1] <= w[1] - w[0] + 1e-9));
        }
    }

    #[test]
    fn a_first_ticket_is_worth_its_expected_return() {
        let grid = grid();
        let values = ExactValues::new(&grid, &FROZEN);
        let (ticket, best) = values.best().unwrap();
        assert_relative_eq!(
            best,
            grid.value(&ticket, &FROZEN).expected_return,
            max_relative = 1e-5
        );
        let other = Ticket::parse("1N22121").unwrap();
        assert_relative_eq!(
            values.marginal(code_of(&other)),
            grid.value(&other, &FROZEN).expected_return,
            max_relative = 1e-5
        );
    }

    /// Valeur de l'ensemble par la formule directe, sur toutes les issues.
    fn set_value(values: &ExactValues, tickets: &[usize]) -> f64 {
        let width = 2 * values.matches;
        let mut total: f64 = tickets.iter().map(|&t| values.perfect[t]).sum();
        for o in 0..values.taken.len() {
            let c = values.neighbours[o * width..(o + 1) * width]
                .iter()
                .filter(|t| tickets.contains(t))
                .count();
            total += values.one_error[o * width..o * width + c]
                .iter()
                .sum::<f64>();
        }
        total
    }

    #[test]
    fn the_marginals_add_up_to_the_value_of_the_set() {
        let mut values = ExactValues::new(&grid(), &FROZEN);
        let (mut codes, mut sum) = (Vec::new(), 0.0);
        for _ in 0..24 {
            let (ticket, marginal) = values.best().unwrap();
            sum += marginal;
            codes.push(code_of(&ticket));
            values.take(&ticket);
        }
        assert_relative_eq!(sum, set_value(&values, &codes), max_relative = 1e-9);
    }

    #[test]
    fn a_ticket_is_never_taken_twice_and_codes_round_trip() {
        let mut values = ExactValues::new(&grid(), &FROZEN);
        let mut seen = Vec::new();
        for _ in 0..40 {
            let (ticket, _) = values.best().unwrap();
            assert!(!seen.contains(&ticket));
            assert_eq!(ticket_of(code_of(&ticket), 7), ticket);
            values.take(&ticket);
            seen.push(ticket);
        }
    }

    #[test]
    fn neighbours_are_one_error_away_and_symmetric() {
        let values = ExactValues::new(&grid(), &FROZEN);
        let width = 14;
        for code in [0, 17, 500, 2186] {
            let signs = digits(code, 7);
            for &other in &values.neighbours[code * width..(code + 1) * width] {
                let differ = digits(other, 7)
                    .iter()
                    .zip(&signs)
                    .filter(|(a, b)| a != b)
                    .count();
                assert_eq!(differ, 1);
                assert!(values.neighbours[other * width..(other + 1) * width].contains(&code));
            }
        }
    }
}
