//! Regroupement sans perte des tickets d'une grille en grilles multiples, pour remplir moins de grilles au guichet.
//!
//! Une grille multiple est le produit cartésien de ses pronostics : `1N 2 1N2 …` contient les 2 × 1 × 3 = 6 tickets
//! simples obtenus en prenant un signe par ligne, coûte 6 € et paie exactement comme ces 6 tickets (règlement Loto
//! Sports v6, art. 5.2.5 et 10.5 ; rencontre annulée : art. 10.6). Regrouper des tickets **déjà choisis** ne change donc
//! ni la mise, ni l'espérance, ni le journal : on n'ajoute aucun ticket et on n'en retire aucun.

use std::fmt;

use crate::domain::Outcome;

use super::optimize::MAX_TICKETS;
use super::{Game, Ticket, sign_char};

/// Une grille à remplir : pour chaque rencontre, un, deux ou trois signes (simple, double, triple).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Line {
    /// Signes de chaque rencontre, dans l'ordre de la grille ; chaque liste est triée (1, N, 2) et sans doublon.
    signs: Vec<Vec<Outcome>>,
}

impl Line {
    /// La grille simple d'un ticket.
    pub fn single(ticket: &Ticket) -> Line {
        Line {
            signs: ticket.signs().iter().map(|&s| vec![s]).collect(),
        }
    }

    /// Signes de chaque rencontre.
    pub fn signs(&self) -> &[Vec<Outcome>] {
        &self.signs
    }

    /// Nombre de tickets simples contenus, donc prix en euros : produit du nombre de signes de chaque rencontre
    /// (règlement v6, art. 5.2.10 et 5.2.11).
    pub fn combinations(&self) -> usize {
        self.signs.iter().map(Vec::len).product()
    }

    /// Nombre de doubles.
    pub fn doubles(&self) -> usize {
        self.signs.iter().filter(|s| s.len() == 2).count()
    }

    /// Nombre de triples.
    pub fn triples(&self) -> usize {
        self.signs.iter().filter(|s| s.len() == 3).count()
    }

    /// Les tickets simples de la grille (produit cartésien), dans l'ordre des codes.
    pub fn tickets(&self) -> Vec<Ticket> {
        let mut partial: Vec<Vec<Outcome>> = vec![Vec::new()];
        for choices in &self.signs {
            // Chaque ticket partiel est prolongé par chacun des signes de la rencontre : `partial` grandit d'un facteur
            // `choices.len()` à chaque ligne.
            partial = partial
                .iter()
                .flat_map(|prefix| {
                    choices.iter().map(move |&sign| {
                        let mut next = prefix.clone();
                        next.push(sign);
                        next
                    })
                })
                .collect();
        }
        partial.into_iter().map(Ticket::new).collect()
    }

    /// Réunion de deux grilles qui ne diffèrent que sur une rencontre, où leurs signes sont disjoints ; `None` sinon.
    ///
    /// Si A et B sont égales partout sauf en i, A × … et B × … se réunissent en une seule grille dont la ligne i porte
    /// les signes des deux. Toute grille multiple se construit ainsi à partir de ses tickets (on la coupe en deux le
    /// long d'une rencontre à plusieurs signes, et on recommence) : la fermeture par réunion les trouve toutes.
    fn merge(&self, other: &Line) -> Option<Line> {
        if self.signs.len() != other.signs.len() {
            return None;
        }
        let mut differing = (0..self.signs.len()).filter(|&i| self.signs[i] != other.signs[i]);
        let i = differing.next()?;
        if differing.next().is_some() || self.signs[i].iter().any(|s| other.signs[i].contains(s)) {
            return None;
        }
        let mut signs = self.signs.clone();
        signs[i].extend_from_slice(&other.signs[i]);
        signs[i].sort();
        Some(Line { signs })
    }
}

/// Forme écrite : les signes de chaque rencontre, séparés par des espaces (`1N 2 2 1 N2 1N2 1`).
impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, choices) in self.signs.iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            for &sign in choices {
                write!(f, "{}", sign_char(sign))?;
            }
        }
        Ok(())
    }
}

/// Une grille à remplir dans une séance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLine {
    /// Grille du Loto Foot : formule et numéro.
    pub grid: (Game, u32),
    /// La grille à remplir, simple ou multiple.
    pub line: Line,
    /// Rangs (à partir de 0) des tickets qu'elle contient dans la liste de la séance.
    pub tickets: Vec<usize>,
}

/// Grilles à remplir pour toute une séance : les tickets de chaque grille du Loto Foot regroupés par [`group`]. Les
/// grilles du Loto Foot viennent dans l'ordre de leur premier ticket, chacune avec ses grilles à remplir.
///
/// `tickets` emprunte les tickets de la séance (`&Ticket`) : on ne fait que les lire, les recopier serait inutile.
pub fn session_lines(tickets: &[((Game, u32), &Ticket)]) -> Vec<SessionLine> {
    let mut keys: Vec<(Game, u32)> = Vec::new();
    for (key, _) in tickets {
        if !keys.contains(key) {
            keys.push(*key);
        }
    }
    let mut lines = Vec::new();
    for key in keys {
        // Rangs dans la séance des tickets de cette grille, et les tickets eux-mêmes, dans le même ordre.
        let ranks: Vec<usize> = (0..tickets.len())
            .filter(|&i| tickets[i].0 == key)
            .collect();
        let own: Vec<Ticket> = ranks.iter().map(|&i| tickets[i].1.clone()).collect();
        for line in group(key.0, &own) {
            let contained = line.tickets();
            let mut members: Vec<usize> = ranks
                .iter()
                .copied()
                .filter(|&i| contained.contains(tickets[i].1))
                .collect();
            members.sort_unstable();
            lines.push(SessionLine {
                grid: key,
                line,
                tickets: members,
            });
        }
    }
    lines
}

/// `true` si le règlement accepte une grille de cette formule avec `doubles` doubles et `triples` triples
/// (règlement v6, tableaux des art. 5.2.6 pour le LF7 et 5.2.7 pour le LF8 ; les autres grilles sont refusées par le
/// terminal).
pub fn allowed(game: Game, doubles: usize, triples: usize) -> bool {
    // Nombre maximal de doubles pour 0, 1, 2 et 3 triples.
    let max_doubles: [usize; 4] = match game {
        Game::Lf7 => [4, 3, 1, 0],
        Game::Lf8 => [5, 3, 2, 0],
    };
    max_doubles
        .get(triples)
        .is_some_and(|&most| doubles <= most)
}

/// Regroupe les tickets d'une grille en le moins possible de grilles, simples ou multiples, que le règlement accepte.
///
/// Chaque ticket se retrouve dans exactement une grille, et aucune grille ne contient de ticket absent de la liste :
/// la mise et les gains sont ceux des tickets. Ordre du résultat : celui du premier ticket de chaque grille dans la
/// liste (les meilleurs tickets d'abord). Même liste, même résultat.
///
/// Repli sans regroupement (une grille par ticket) si la liste dépasse [`MAX_TICKETS`], contient deux fois le même
/// ticket ou des tickets de longueur différente de la formule : ces cas n'arrivent pas en séance.
pub fn group(game: Game, tickets: &[Ticket]) -> Vec<Line> {
    let fallback = || tickets.iter().map(Line::single).collect();
    let distinct = tickets
        .iter()
        .enumerate()
        .all(|(i, t)| !tickets[..i].contains(t));
    if tickets.len() > MAX_TICKETS || !distinct || tickets.iter().any(|t| t.len() != game.matches())
    {
        return fallback();
    }
    let candidates = candidates(game, tickets);
    match exact_cover(tickets.len(), &candidates) {
        Some(chosen) => chosen
            .into_iter()
            .map(|k| candidates[k].0.clone())
            .collect(),
        None => fallback(),
    }
}

/// Toutes les grilles autorisées formées uniquement de tickets de la liste, chacune avec son masque : le bit j vaut 1
/// si la grille contient le ticket j. Triées des plus grandes aux plus petites, puis par ordre de leur premier ticket :
/// à nombre de grilles égal, la couverture retenue est donc toujours la même.
fn candidates(game: Game, tickets: &[Ticket]) -> Vec<(Line, u32)> {
    let mut lines: Vec<Line> = tickets.iter().map(Line::single).collect();
    // Fermeture par réunion : on réunit toute paire réunissable jusqu'à ce qu'aucune grille nouvelle n'apparaisse.
    // Une réunion de deux grilles faites de tickets de la liste n'en contient que de la liste : aucun contrôle n'est
    // nécessaire. Au plus 16 tickets, donc quelques centaines de grilles au pire.
    let mut start = 0;
    while start < lines.len() {
        let end = lines.len();
        for a in start..end {
            for b in 0..a {
                if let Some(merged) = lines[a].merge(&lines[b])
                    && !lines.contains(&merged)
                {
                    lines.push(merged);
                }
            }
        }
        start = end;
    }
    let mut masked: Vec<(Line, u32)> = lines
        .into_iter()
        .filter(|line| allowed(game, line.doubles(), line.triples()))
        .map(|line| {
            let mask = line.tickets().iter().fold(0u32, |mask, t| {
                // Chaque ticket de la grille est dans la liste (construction ci-dessus) : `position` le trouve.
                tickets
                    .iter()
                    .position(|x| x == t)
                    .map_or(mask, |j| mask | (1 << j))
            });
            (line, mask)
        })
        .collect();
    masked.sort_by_key(|(line, mask)| {
        (
            std::cmp::Reverse(line.combinations()),
            mask.trailing_zeros(),
        )
    });
    masked
}

/// Couverture exacte des `count` tickets par le moins de grilles possible : indices des grilles choisies, rangées par
/// leur premier ticket. `None` si aucune couverture n'existe (impossible : chaque ticket seul est une grille).
///
/// Programmation dynamique sur les sous-ensembles de tickets (masques de `count` bits, 2^16 = 65 536 au plus) :
/// `best[m]` est le nombre minimal de grilles qui couvrent exactement les tickets de `m`, avec la grille ajoutée en
/// dernier. Le ticket de plus petit indice de `m` doit être dans l'une d'elles ; on essaie chaque grille qui le contient
/// et reste dans `m`, ce qui ramène à `m` privé de cette grille, un masque plus petit déjà calculé.
fn exact_cover(count: usize, candidates: &[(Line, u32)]) -> Option<Vec<usize>> {
    let full: u32 = (1u32 << count) - 1;
    let mut best: Vec<Option<(usize, usize)>> = vec![None; 1 << count];
    best[0] = Some((0, usize::MAX));
    for m in 1..=full {
        let low = m & m.wrapping_neg();
        for (k, (_, mask)) in candidates.iter().enumerate() {
            if mask & low == 0 || mask & !m != 0 {
                continue;
            }
            let Some((lines, _)) = best[(m ^ mask) as usize] else {
                continue;
            };
            // Comparaison stricte : à égalité, la première grille de la liste triée l'emporte.
            if best[m as usize].is_none_or(|(current, _)| lines + 1 < current) {
                best[m as usize] = Some((lines + 1, k));
            }
        }
    }
    let mut chosen = Vec::new();
    let mut m = full;
    while m != 0 {
        let (_, k) = best[m as usize]?;
        chosen.push(k);
        m ^= candidates[k].1;
    }
    chosen.sort_by_key(|&k| candidates[k].1.trailing_zeros());
    Some(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tickets(codes: &[&str]) -> Vec<Ticket> {
        codes.iter().map(|c| Ticket::parse(c).unwrap()).collect()
    }

    /// Les tickets de toutes les grilles, triés : doit redonner exactement la liste de départ.
    fn expanded(lines: &[Line]) -> Vec<Ticket> {
        let mut all: Vec<Ticket> = lines.iter().flat_map(Line::tickets).collect();
        all.sort();
        all
    }

    #[test]
    fn a_line_holds_the_cartesian_product_of_its_signs() {
        let line = Line {
            signs: vec![
                vec![Outcome::Home, Outcome::Draw],
                vec![Outcome::Away],
                Outcome::ALL.to_vec(),
            ],
        };
        assert_eq!(line.combinations(), 6);
        assert_eq!((line.doubles(), line.triples()), (1, 1));
        assert_eq!(line.to_string(), "1N 2 1N2");
        let codes: Vec<String> = line.tickets().iter().map(Ticket::to_string).collect();
        assert_eq!(codes, ["121", "12N", "122", "N21", "N2N", "N22"]);
    }

    #[test]
    fn allowed_shapes_follow_the_rules_tables() {
        // LF7 (art. 5.2.6) : 4 doubles, ou 3 doubles et 1 triple, ou 1 double et 2 triples, ou 3 triples.
        assert!(allowed(Game::Lf7, 4, 0));
        assert!(!allowed(Game::Lf7, 5, 0));
        assert!(allowed(Game::Lf7, 3, 1));
        assert!(!allowed(Game::Lf7, 2, 2));
        assert!(allowed(Game::Lf7, 0, 3));
        assert!(!allowed(Game::Lf7, 0, 4));
        // LF8 (art. 5.2.7) : jusqu'à 5 doubles, et 2 doubles avec 2 triples.
        assert!(allowed(Game::Lf8, 5, 0));
        assert!(allowed(Game::Lf8, 2, 2));
        assert!(!allowed(Game::Lf8, 1, 3));
    }

    #[test]
    fn a_complete_product_becomes_one_multiple() {
        let line = Line {
            signs: vec![
                vec![Outcome::Home, Outcome::Draw],
                vec![Outcome::Away],
                vec![Outcome::Home],
                vec![Outcome::Draw, Outcome::Away],
                vec![Outcome::Home],
                Outcome::ALL.to_vec(),
                vec![Outcome::Away],
            ],
        };
        // Ordre quelconque à l'entrée : le regroupement ne dépend pas de l'ordre pour trouver la grille.
        let mut list = line.tickets();
        list.reverse();
        assert_eq!(group(Game::Lf7, &list), vec![line]);
    }

    #[test]
    fn an_incomplete_product_is_not_completed() {
        // Trois des quatre tickets de `1N 1N 1 1 1 1 1` : on ne peut pas ajouter le quatrième (il ne fait pas partie
        // du choix), donc deux grilles, l'une double, l'autre simple.
        let list = tickets(&["1111111", "1N11111", "N111111"]);
        let lines = group(Game::Lf7, &list);
        assert_eq!(lines.len(), 2);
        assert_eq!(expanded(&lines), {
            let mut l = list.clone();
            l.sort();
            l
        });
    }

    /// Les 16 tickets du test de référence de l'optimiseur : chaque ticket une seule fois, aucun ajouté, et bien moins
    /// de 16 grilles.
    #[test]
    fn the_reference_session_is_grouped_without_loss() {
        let list = tickets(&[
            "1222122", "1222112", "122212N", "12N2112", "12N2122", "1222121", "122211N", "N222122",
            "1222111", "1N22122", "1N2212N", "N222112", "1N22121", "1N22112", "1N2211N", "N22212N",
        ]);
        let lines = group(Game::Lf7, &list);
        let mut sorted = list.clone();
        sorted.sort();
        assert_eq!(expanded(&lines), sorted);
        assert_eq!(lines.iter().map(Line::combinations).sum::<usize>(), 16);
        assert!(lines.len() < 8, "{} grilles : {lines:?}", lines.len());
        assert!(
            lines
                .iter()
                .all(|l| allowed(Game::Lf7, l.doubles(), l.triples()))
        );
        assert_eq!(group(Game::Lf7, &list), lines);
    }

    #[test]
    fn session_lines_keep_each_grid_apart_and_point_to_the_session_ranks() {
        let a = tickets(&["1111111", "21N12222", "N111111", "21N12122"]);
        let session = [
            ((Game::Lf7, 133), &a[0]),
            ((Game::Lf8, 146), &a[1]),
            ((Game::Lf7, 133), &a[2]),
            ((Game::Lf8, 146), &a[3]),
        ];
        let lines = session_lines(&session);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].grid, (Game::Lf7, 133));
        assert_eq!(lines[0].line.to_string(), "1N 1 1 1 1 1 1");
        assert_eq!(lines[0].tickets, [0, 2]);
        assert_eq!(lines[1].grid, (Game::Lf8, 146));
        assert_eq!(lines[1].line.to_string(), "2 1 N 1 2 12 2 2");
        assert_eq!(lines[1].tickets, [1, 3]);
    }

    #[test]
    fn odd_lists_fall_back_to_single_lines() {
        let twice = tickets(&["1111111", "1111111"]);
        assert_eq!(group(Game::Lf7, &twice).len(), 2);
        let wrong_length = tickets(&["11111111"]);
        assert_eq!(group(Game::Lf7, &wrong_length).len(), 1);
        assert!(group(Game::Lf8, &[]).is_empty());
    }
}
