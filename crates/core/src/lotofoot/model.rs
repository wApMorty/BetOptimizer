//! Espérance exacte d'un ticket simple du Loto Foot, connaissant le marché et la répartition du public.
//!
//! Modèle (étude et calibration : `docs/sources.md` §37) :
//! - les rencontres sont indépendantes ; l'issue de la rencontre i suit les probabilités du marché `p_i` ;
//! - le public a joué `S` grilles d'un euro, signe par signe selon la répartition FDJ `q_i` (indépendance mesurée :
//!   gagnants observés / prédits entre 0,97 et 1,16 sur 1 589 grilles) ;
//! - pour une issue `o`, la part du public qui compte exactement `k` bons pronostics suit une loi de Poisson-binomiale
//!   de paramètres `a_i = q_i(o_i)` ; nos concurrents au même rang sont en moyenne `λ = S · P(k = n − e)`, corrigés en
//!   `μ = κ · λ^β + φ · S / 3^n` (ajustement de 2021 à 2023, gelé ; `φ` : part des mises des joueurs à systèmes,
//!   répartie sur toutes les combinaisons) ;
//! - la masse `M_e` du rang atteint avec `e` erreurs se partage entre nous et `W ~ Poisson(μ)` concurrents :
//!   `E[M_e / (1 + W)] = M_e · (1 − e^{−μ}) / μ`.
//!
//! Espérance : `E[gain] = Σ_o P(o) · M_{e(o)} · (1 − e^{−μ(o)}) / μ(o)`, la somme portant sur les issues où le ticket
//! fait au plus `R − 1` erreurs (`R` rangs payés). Contrôle : pondérée par la répartition du public, l'espérance moyenne
//! d'une grille vaut le taux de retour (70 %), voir les tests.

use crate::domain::{Euros, Outcome, Probs1x2};

use super::{Game, PAYOUT_RATE, TICKET_PRICE, Ticket};

/// Paramètres du nombre attendu de concurrents (κ, β, φ) et, pour le scénario défavorable, un multiplicateur appliqué
/// quand `λ` est petit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Competitors {
    /// Facteur d'échelle κ.
    pub kappa: f64,
    /// Exposant β.
    pub beta: f64,
    /// Part φ des mises répartie uniformément sur les `3^n` combinaisons.
    pub phi: f64,
    /// Multiplicateur des concurrents attendus quand `λ < low_cut` (1 dans le modèle de base).
    pub low_mult: f64,
    /// Seuil de `λ` sous lequel `low_mult` s'applique.
    pub low_cut: f64,
}

/// Paramètres gelés le 2026-10-22, ajustés par maximum de vraisemblance (binomiale négative) sur les 1 660 observations
/// LF7 et LF8 d'entraînement (grilles closes avant 2024) : `docs/sources.md` §37, `scripts/lotofoot/fdj_calib.py`.
pub const FROZEN: Competitors = Competitors {
    kappa: 0.870_476_731_094_13,
    beta: 1.020_977_501_690_405_2,
    phi: 0.104_308_000_946_678_32,
    low_mult: 1.0,
    low_cut: 3.0,
};

/// Scénario défavorable (« S3 » de l'étude) : concurrents × 1,4 quand `λ < 3` (sous-prédiction mesurée aux faibles `λ`)…
pub const PESSIMISTIC_LOW_MULT: f64 = 1.4;

/// … et probabilités du marché rapprochées de 10 % de la répartition du public (les cotes surestiment les signes à
/// contre-courant d'environ 0,7 point avant la clôture).
pub const PESSIMISTIC_SHRINK: f64 = 0.10;

impl Competitors {
    /// Les mêmes paramètres dans le scénario défavorable.
    pub fn pessimistic(self) -> Competitors {
        Competitors {
            low_mult: PESSIMISTIC_LOW_MULT,
            ..self
        }
    }

    /// Nombre attendu de concurrents `μ` pour `λ` grilles du public au même rang, sur une grille de `n` rencontres et
    /// `stakes` grilles jouées.
    pub(crate) fn expected(&self, lambda: f64, stakes: f64, n: usize) -> f64 {
        // `max(1e-300)` : `0^β` vaut 0, mais un λ exactement nul resterait sans effet ; le plancher évite un NaN si β < 0.
        let systems = self.phi * stakes / 3f64.powi(i32::try_from(n).unwrap_or(i32::MAX));
        let mu = self.kappa * lambda.max(1e-300).powf(self.beta) + systems;
        if lambda < self.low_cut {
            mu * self.low_mult
        } else {
            mu
        }
    }
}

/// Valeur d'un ticket : espérance par euro misé et probabilité d'atteindre un rang payé.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TicketValue {
    /// Euros rendus en moyenne pour un euro misé (1,0 : ni gain ni perte en moyenne).
    pub expected_return: f64,
    /// Probabilité que le ticket atteigne un rang payé.
    pub paid_probability: f64,
}

/// Une grille prête à être évaluée : marché, public, mises et masses de gains.
#[derive(Debug, Clone, PartialEq)]
pub struct GridModel {
    /// Probabilités du marché, une ligne par rencontre (domicile, nul, extérieur).
    p: Vec<[f64; 3]>,
    /// Répartition du public, une ligne par rencontre.
    q: Vec<[f64; 3]>,
    /// Nombre de grilles d'un euro jouées par le public (S).
    stakes: f64,
    /// Masse de gains de chaque rang, du sans-faute (indice 0) au dernier rang payé.
    masses: Vec<f64>,
}

impl GridModel {
    /// Crée une grille. Renvoie `None` si les longueurs diffèrent, s'il n'y a aucune rencontre ou aucun rang, ou s'il y a
    /// autant de rangs que de rencontres (un rang à `n` erreurs n'a pas de sens).
    pub fn new(
        p: &[Probs1x2],
        q: &[Probs1x2],
        stakes: Euros,
        masses: &[Euros],
    ) -> Option<GridModel> {
        if p.is_empty() || p.len() != q.len() || masses.is_empty() || masses.len() >= p.len() {
            return None;
        }
        Some(GridModel {
            p: p.iter().map(Probs1x2::to_array).collect(),
            q: q.iter().map(Probs1x2::to_array).collect(),
            stakes: stakes.get(),
            masses: masses.iter().map(|m| m.get()).collect(),
        })
    }

    /// Masses de gains d'une formule : 70 % de `stakes` réparti selon les parts des rangs, le rang 1 relevé au gain
    /// minimum garanti (« pactole ») s'il y en a un et qu'il dépasse la part naturelle (règlement v6, art. 10.1.7).
    pub fn masses_for(game: Game, stakes: Euros, guaranteed: Option<Euros>) -> Vec<Euros> {
        let total = PAYOUT_RATE * stakes.get();
        let mut masses: Vec<f64> = game
            .rank_shares()
            .iter()
            .map(|share| share * total)
            .collect();
        if let (Some(first), Some(floor)) = (masses.first_mut(), guaranteed) {
            *first = first.max(floor.get());
        }
        // Une masse est positive par construction : `Euros::new` ne peut pas échouer ici, `filter_map` évite tout `unwrap`.
        masses.into_iter().filter_map(Euros::new).collect()
    }

    /// Nombre de rencontres.
    pub fn len(&self) -> usize {
        self.p.len()
    }

    /// `true` pour une grille sans rencontre (impossible par construction ; exigé par Clippy avec `len`).
    pub fn is_empty(&self) -> bool {
        self.p.is_empty()
    }

    /// Probabilités du marché de la rencontre `i`.
    pub fn market(&self, i: usize) -> Option<[f64; 3]> {
        self.p.get(i).copied()
    }

    /// Répartition du public de la rencontre `i`.
    pub fn public(&self, i: usize) -> Option<[f64; 3]> {
        self.q.get(i).copied()
    }

    /// Nombre de rangs payés.
    pub fn ranks(&self) -> usize {
        self.masses.len()
    }

    /// La même grille dans le scénario défavorable : `p ← (1 − s) · p + s · q`, puis renormalisée, `s` = 10 %.
    pub fn pessimistic(&self) -> GridModel {
        let s = PESSIMISTIC_SHRINK;
        let p = self
            .p
            .iter()
            .zip(&self.q)
            .map(|(p, q)| {
                let mixed = [0, 1, 2].map(|k| (1.0 - s) * p[k] + s * q[k]);
                let sum: f64 = mixed.iter().sum();
                mixed.map(|x| x / sum)
            })
            .collect();
        GridModel { p, ..self.clone() }
    }

    /// Espérance et probabilité de gain d'un ticket simple. Un ticket de mauvaise longueur ne gagne rien.
    pub fn value(&self, ticket: &Ticket, competitors: &Competitors) -> TicketValue {
        let mut acc = Accumulator::default();
        if ticket.len() == self.len() {
            let mut outcome = Vec::with_capacity(self.len());
            self.visit(
                ticket.signs(),
                0,
                0,
                1.0,
                &mut outcome,
                competitors,
                &mut acc,
            );
        }
        TicketValue {
            expected_return: acc.gain / TICKET_PRICE,
            paid_probability: acc.paid,
        }
    }

    /// Parcourt en profondeur toutes les issues où le ticket fait au plus `R − 1` erreurs : à chaque rencontre, on garde
    /// le signe du ticket, ou on le remplace par l'un des deux autres tant qu'il reste des erreurs permises.
    ///
    /// `outcome` est un seul vecteur réutilisé (empilé puis dépilé) : on évite d'allouer un vecteur par issue.
    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        ticket: &[Outcome],
        i: usize,
        errors: usize,
        prob: f64,
        outcome: &mut Vec<usize>,
        competitors: &Competitors,
        acc: &mut Accumulator,
    ) {
        let Some(&sign) = ticket.get(i) else {
            self.leaf(outcome, errors, prob, competitors, acc);
            return;
        };
        let ours = sign.index();
        for real in 0..3 {
            let wrong = usize::from(real != ours);
            if errors + wrong >= self.ranks() {
                continue;
            }
            let p = self.p.get(i).map_or(0.0, |row| row[real]);
            outcome.push(real);
            self.visit(
                ticket,
                i + 1,
                errors + wrong,
                prob * p,
                outcome,
                competitors,
                acc,
            );
            outcome.pop();
        }
    }

    /// Une issue complète : notre rang, la part attendue de sa masse, et la probabilité de l'issue.
    fn leaf(
        &self,
        outcome: &[usize],
        errors: usize,
        prob: f64,
        competitors: &Competitors,
        acc: &mut Accumulator,
    ) {
        let n = self.len();
        let a: Vec<f64> = outcome
            .iter()
            .zip(&self.q)
            .map(|(&real, q)| q[real])
            .collect();
        let public = poisson_binomial(&a);
        let lambda = self.stakes * public.get(n - errors).copied().unwrap_or(0.0);
        let mu = competitors.expected(lambda, self.stakes, n);
        let mass = self.masses.get(errors).copied().unwrap_or(0.0);
        acc.gain += prob * mass * expected_share(mu);
        acc.paid += prob;
    }
}

impl GridModel {
    /// Mises de la grille (grilles d'un euro jouées par le public).
    pub fn stakes(&self) -> f64 {
        self.stakes
    }

    /// Masse du rang atteint avec `errors` erreurs (0 : sans-faute) ; `None` au-delà du dernier rang payé.
    pub fn mass(&self, errors: usize) -> Option<f64> {
        self.masses.get(errors).copied()
    }

    /// Gain espéré d'un ticket **sachant les signes sortis** : la masse du rang atteint multipliée par la part attendue
    /// face aux concurrents ; 0 hors des rangs payés. Sert à simuler la loi du gain (vérification du modèle).
    pub fn outcome_gain(
        &self,
        ticket: &Ticket,
        results: &[Outcome],
        competitors: &Competitors,
    ) -> f64 {
        let n = self.len();
        if ticket.len() != n || results.len() != n {
            return 0.0;
        }
        let errors = n - ticket.good(results);
        let Some(mass) = self.mass(errors) else {
            return 0.0;
        };
        let a: Vec<f64> = results
            .iter()
            .zip(&self.q)
            .map(|(o, q)| q[o.index()])
            .collect();
        let lambda = self.stakes * poisson_binomial(&a).get(n - errors).copied().unwrap_or(0.0);
        mass * expected_share(competitors.expected(lambda, self.stakes, n))
    }

    /// Nombre attendu de concurrents du public au rang atteint avec `errors` erreurs, sachant les signes sortis (`λ`,
    /// avant correction). Sert à comparer prédiction et gagnants publiés.
    pub fn public_winners(&self, results: &[Outcome], errors: usize) -> f64 {
        let n = self.len();
        let a: Vec<f64> = results
            .iter()
            .zip(&self.q)
            .map(|(o, q)| q[o.index()])
            .collect();
        self.stakes
            * poisson_binomial(&a)
                .get(n.saturating_sub(errors))
                .copied()
                .unwrap_or(0.0)
    }
}

/// Répartition FDJ affichée (pourcents entiers, arrondis : leur somme vaut 99, 100 ou 101) → probabilités qui somment
/// à 1. Une part affichée « 0 » est comptée 0,5 % : le modèle divise par la part du public, et la vraie part n'est pas
/// nulle, seulement arrondie. `None` si une ligne est vide.
pub fn public_split(split: &[[u32; 3]]) -> Option<Vec<Probs1x2>> {
    split
        .iter()
        .map(|s| {
            let shares = s.map(|x| f64::from(x).max(0.5));
            let sum: f64 = shares.iter().sum();
            if s.iter().all(|x| *x == 0) {
                return None;
            }
            let home = shares[0] / sum;
            let draw = shares[1] / sum;
            // Le troisième terme est le complément : la somme vaut 1 aux erreurs d'arrondi près, comme l'exige `Probs1x2`.
            Probs1x2::new(home, draw, 1.0 - home - draw)
        })
        .collect()
}

/// Sommes accumulées pendant le parcours des issues.
#[derive(Default)]
struct Accumulator {
    gain: f64,
    paid: f64,
}

/// Part espérée d'une masse partagée entre nous et `W ~ Poisson(μ)` concurrents : `E[1/(1 + W)] = (1 − e^{−μ}) / μ`.
/// `exp_m1` calcule `e^x − 1` sans perte de précision quand `x` est proche de 0.
fn expected_share(mu: f64) -> f64 {
    if mu < 1e-9 { 1.0 } else { -(-mu).exp_m1() / mu }
}

/// Loi de Poisson-binomiale : `P(K = k)` pour `K` somme de variables de Bernoulli indépendantes de probabilités `a_i`.
///
/// Récurrence classique (convolution) : en ajoutant une variable de probabilité `x`,
/// `P'(k) = P(k) · (1 − x) + P(k − 1) · x`. Le vecteur renvoyé a `a.len() + 1` éléments et somme à 1.
pub fn poisson_binomial(a: &[f64]) -> Vec<f64> {
    let mut dist = vec![0.0; a.len() + 1];
    if let Some(first) = dist.first_mut() {
        *first = 1.0;
    }
    for (count, &x) in a.iter().enumerate() {
        // Parcours à rebours : chaque `dist[k]` est mis à jour avec l'ancien `dist[k − 1]`, pas avec le nouveau.
        for k in (1..=count + 1).rev() {
            let previous = dist.get(k - 1).copied().unwrap_or(0.0);
            if let Some(cell) = dist.get_mut(k) {
                *cell = *cell * (1.0 - x) + previous * x;
            }
        }
        if let Some(first) = dist.first_mut() {
            *first *= 1.0 - x;
        }
    }
    dist
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn probs(rows: &[[f64; 3]]) -> Vec<Probs1x2> {
        rows.iter()
            .map(|r| Probs1x2::new(r[0], r[1], r[2]).unwrap())
            .collect()
    }

    fn euros(x: f64) -> Euros {
        Euros::new(x).unwrap()
    }

    const P7: [[f64; 3]; 7] = [
        [0.47, 0.23, 0.30],
        [0.43, 0.25, 0.32],
        [0.14, 0.18, 0.68],
        [0.35, 0.27, 0.38],
        [0.50, 0.25, 0.25],
        [0.45, 0.24, 0.31],
        [0.45, 0.26, 0.29],
    ];
    const Q7: [[f64; 3]; 7] = [
        [0.31, 0.24, 0.45],
        [0.61, 0.19, 0.20],
        [0.14, 0.17, 0.69],
        [0.45, 0.26, 0.29],
        [0.35, 0.25, 0.40],
        [0.47, 0.25, 0.28],
        [0.61, 0.20, 0.19],
    ];
    const P8: [[f64; 3]; 8] = [
        [0.42, 0.29, 0.29],
        [0.34, 0.26, 0.40],
        [0.54, 0.27, 0.19],
        [0.61, 0.22, 0.17],
        [0.31, 0.29, 0.40],
        [0.41, 0.29, 0.30],
        [0.44, 0.28, 0.28],
        [0.29, 0.28, 0.43],
    ];
    const Q8: [[f64; 3]; 8] = [
        [0.53, 0.25, 0.22],
        [0.25, 0.32, 0.43],
        [0.68, 0.18, 0.14],
        [0.67, 0.18, 0.15],
        [0.33, 0.36, 0.31],
        [0.43, 0.30, 0.27],
        [0.63, 0.20, 0.17],
        [0.43, 0.25, 0.32],
    ];

    fn lf7() -> GridModel {
        let stakes = euros(160_000.0);
        GridModel::new(
            &probs(&P7),
            &probs(&Q7),
            stakes,
            &GridModel::masses_for(Game::Lf7, stakes, None),
        )
        .unwrap()
    }

    fn lf8() -> GridModel {
        let stakes = euros(300_000.0);
        let masses = GridModel::masses_for(Game::Lf8, stakes, Some(euros(150_000.0)));
        GridModel::new(&probs(&P8), &probs(&Q8), stakes, &masses).unwrap()
    }

    #[test]
    fn poisson_binomial_sums_to_one_and_matches_the_binomial() {
        let dist = poisson_binomial(&[0.3, 0.6, 0.9, 0.1]);
        assert_relative_eq!(dist.iter().sum::<f64>(), 1.0, epsilon = 1e-12);
        // Probabilités égales : loi binomiale B(3 ; 0,5).
        let dist = poisson_binomial(&[0.5, 0.5, 0.5]);
        for (k, expected) in [0.125, 0.375, 0.375, 0.125].iter().enumerate() {
            assert_relative_eq!(dist[k], *expected, epsilon = 1e-12);
        }
        // Une variable certaine décale la loi d'un cran.
        assert_eq!(poisson_binomial(&[1.0]), vec![0.0, 1.0]);
    }

    /// Valeurs de référence du script de l'étude (`scripts/lotofoot/lf_fast.py`, `ev_p`) sur les mêmes grilles.
    #[test]
    fn expected_return_matches_the_study_script() {
        let ticket = Ticket::parse("1222122").unwrap();
        let value = lf7().value(&ticket, &FROZEN);
        assert_relative_eq!(
            value.expected_return,
            3.666_968_973_690_510_7,
            epsilon = 1e-9
        );
        assert_relative_eq!(
            value.paid_probability,
            0.021_013_397_872_000_007,
            epsilon = 1e-12
        );

        let ticket = Ticket::parse("N1NN2222").unwrap();
        let value = lf8().value(&ticket, &FROZEN);
        assert_relative_eq!(value.expected_return, 3.345_080_491_017_704, epsilon = 1e-9);
        assert_relative_eq!(
            value.paid_probability,
            0.001_638_993_600_960_000_6,
            epsilon = 1e-12
        );
    }

    /// Ticket très à contre-courant (λ ≈ 2,5 au rang 1) : le scénario défavorable change le résultat.
    #[test]
    fn the_pessimistic_scenario_matches_the_study_script() {
        let ticket = Ticket::parse("21221N21").unwrap();
        let base = lf8().value(&ticket, &FROZEN);
        assert_relative_eq!(
            base.expected_return,
            1.087_171_330_145_364_3,
            epsilon = 1e-9
        );
        let low = lf8().value(&ticket, &FROZEN.pessimistic());
        assert_relative_eq!(low.expected_return, 0.945_163_891_168_867_8, epsilon = 1e-9);
        let s3 = lf8().pessimistic().value(&ticket, &FROZEN.pessimistic());
        assert_relative_eq!(s3.expected_return, 0.876_862_891_115_735_8, epsilon = 1e-9);
        assert_relative_eq!(
            s3.paid_probability,
            0.000_530_902_770_492_701_4,
            epsilon = 1e-12
        );
    }

    /// Contrôle de cohérence de l'étude : pondérée par la répartition du public, l'espérance d'une grille vaut le taux
    /// de retour, quand les concurrents suivent exactement le public (κ = β = 1, φ = 0).
    #[test]
    fn the_average_public_ticket_returns_the_payout_rate() {
        let plain = Competitors {
            kappa: 1.0,
            beta: 1.0,
            phi: 0.0,
            low_mult: 1.0,
            low_cut: 3.0,
        };
        let grid = lf7();
        let mut total = 0.0;
        let mut weight = 0.0;
        // Les 3^7 = 2 187 tickets, chacun pondéré par sa part dans le public.
        for code in 0..3usize.pow(7) {
            let mut rest = code;
            let mut signs = Vec::with_capacity(7);
            let mut share = 1.0;
            for i in 0..7 {
                let real = rest % 3;
                rest /= 3;
                signs.push(Outcome::ALL[real]);
                share *= grid.public(i).unwrap()[real];
            }
            total += share * grid.value(&Ticket::new(signs), &plain).expected_return;
            weight += share;
        }
        assert_relative_eq!(weight, 1.0, epsilon = 1e-12);
        // Pas exactement 0,700 : quand une combinaison n'a aucun gagnant, sa masse n'est versée à personne.
        assert!(
            (total - PAYOUT_RATE).abs() < 0.01,
            "espérance moyenne {total}"
        );
    }

    /// L'espérance d'un ticket est la moyenne, sur les issues, de son gain sachant l'issue.
    #[test]
    fn expected_return_is_the_average_of_outcome_gains() {
        let grid = lf7();
        let ticket = Ticket::parse("1222122").unwrap();
        let mut total = 0.0;
        for code in 0..3usize.pow(7) {
            let mut rest = code;
            let mut results = Vec::with_capacity(7);
            let mut prob = 1.0;
            for i in 0..7 {
                let real = rest % 3;
                rest /= 3;
                results.push(Outcome::ALL[real]);
                prob *= grid.market(i).unwrap()[real];
            }
            total += prob * grid.outcome_gain(&ticket, &results, &FROZEN);
        }
        assert_relative_eq!(
            total,
            grid.value(&ticket, &FROZEN).expected_return,
            epsilon = 1e-9
        );
    }

    #[test]
    fn displayed_splits_are_normalised() {
        let q = public_split(&[[61, 20, 19], [93, 5, 2], [100, 0, 0]]).unwrap();
        for row in &q {
            assert_relative_eq!(row.to_array().iter().sum::<f64>(), 1.0, epsilon = 1e-12);
        }
        assert!(q[2].to_array()[1] > 0.0);
        assert!(public_split(&[[0, 0, 0]]).is_none());
    }

    #[test]
    fn the_guarantee_raises_only_the_first_rank() {
        let masses = GridModel::masses_for(Game::Lf8, euros(300_000.0), Some(euros(150_000.0)));
        assert_relative_eq!(masses[0].get(), 150_000.0);
        assert_relative_eq!(masses[1].get(), 0.55 * 0.7 * 300_000.0, epsilon = 1e-9);
        let masses = GridModel::masses_for(Game::Lf8, euros(1_000_000.0), Some(euros(150_000.0)));
        assert_relative_eq!(masses[0].get(), 0.45 * 0.7 * 1_000_000.0, epsilon = 1e-9);
    }

    #[test]
    fn inconsistent_grids_are_refused() {
        let stakes = euros(1000.0);
        let masses = GridModel::masses_for(Game::Lf7, stakes, None);
        assert!(GridModel::new(&probs(&P7), &probs(&Q8), stakes, &masses).is_none());
        assert!(GridModel::new(&[], &[], stakes, &masses).is_none());
        // Un ticket de la mauvaise longueur ne gagne rien.
        let value = lf7().value(&Ticket::parse("12").unwrap(), &FROZEN);
        assert_eq!(value.expected_return, 0.0);
    }
}
