//! Test de référence : notre Dixon-Coles contre `penaltyblog` 1.13.1 sur un même jeu de données simulé.
//!
//! La fixture est produite par `scripts/penaltyblog_reference.py` (non exécuté par les tests). Les buts attendus
//! `λ` et `μ`, `ρ` et l'avantage du terrain sont invariants au choix de normalisation des forces d'équipe : on les
//! compare directement. Les deux implémentations maximisent la même vraisemblance pondérée ; ici sans pénalité
//! (`ridge` quasi nul, `penaltyblog` n'en a pas).

use serde::Deserialize;

use super::dixon_coles::DixonColes;
use super::likelihood::{LikelihoodProblem, RHO_MAX};
use super::{MatchModel, TrainingGame};

#[derive(Deserialize)]
struct Reference {
    xi: f64,
    games: Vec<ReferenceGame>,
    home_advantage: f64,
    rho: f64,
    attack: Vec<f64>,
    defence: Vec<f64>,
    pairs: Vec<ReferencePair>,
}

#[derive(Deserialize)]
struct ReferenceGame {
    home: usize,
    away: usize,
    goals_home: u8,
    goals_away: u8,
    day: u32,
}

#[derive(Deserialize)]
struct ReferencePair {
    home: usize,
    away: usize,
    lambda: f64,
    mu: f64,
    p_home: f64,
    p_draw: f64,
    p_away: f64,
}

#[test]
fn dixon_coles_matches_penaltyblog_on_a_simulated_league() {
    let reference: Reference = serde_json::from_str(include_str!(
        "../../tests/fixtures/penaltyblog_reference.json"
    ))
    .unwrap();
    let last_day = reference.games.iter().map(|g| g.day).max().unwrap();
    // Même pondération que `dixon_coles_weights` : exp(−ξ · jours avant le dernier match).
    let games: Vec<TrainingGame> = reference
        .games
        .iter()
        .map(|g| TrainingGame {
            home: g.home,
            away: g.away,
            goals_home: g.goals_home,
            goals_away: g.goals_away,
            target_home: f64::from(g.goals_home),
            target_away: f64::from(g.goals_away),
            weight: (-reference.xi * f64::from(last_day - g.day)).exp(),
        })
        .collect();

    let fitted = DixonColes { ridge: 1e-9 }.fit(8, &games, None).unwrap();

    // 1. Notre optimum est au moins aussi bon (en vraisemblance) que celui de penaltyblog : le problème est
    //    le même, et c'est le critère qui départage deux optimiseurs. Les paramètres de penaltyblog sont
    //    replacés dans notre paramétrage (base 0, ρ via φ = atanh(ρ / RHO_MAX)).
    let problem = LikelihoodProblem::new(8, &games, 1e-9, true);
    let mut theirs = reference.attack.clone();
    theirs.extend(&reference.defence);
    theirs.extend([
        0.0,
        reference.home_advantage,
        (reference.rho / RHO_MAX).atanh(),
    ]);
    let mut ours = fitted.attack.clone();
    ours.extend(&fitted.defence);
    ours.extend([
        fitted.intercept,
        fitted.home_advantage,
        (fitted.rho / RHO_MAX).atanh(),
    ]);
    let (our_cost, _) = problem.evaluate(&ours);
    let (their_cost, _) = problem.evaluate(&theirs);
    eprintln!("coût : nous {our_cost:.6}, penaltyblog {their_cost:.6}");
    assert!(
        our_cost <= their_cost + 1e-6,
        "notre coût {our_cost} contre {their_cost}"
    );
    assert!(
        (our_cost - their_cost).abs() < 0.05,
        "les deux optima doivent être proches : {our_cost} contre {their_cost}"
    );

    // 2. Les quantités invariantes concordent : ρ, avantage du terrain, buts attendus, probabilités du 1X2.
    assert!(
        (fitted.rho - reference.rho).abs() < 5e-3,
        "rho : {} contre {}",
        fitted.rho,
        reference.rho
    );
    assert!(
        (fitted.home_advantage - reference.home_advantage).abs() < 5e-3,
        "avantage : {} contre {}",
        fitted.home_advantage,
        reference.home_advantage
    );
    assert_eq!(reference.pairs.len(), 56);
    let mut worst: f64 = 0.0;
    for pair in &reference.pairs {
        let (lambda, mu) = fitted.expectancies(pair.home, pair.away);
        worst = worst
            .max((lambda / pair.lambda - 1.0).abs())
            .max((mu / pair.mu - 1.0).abs());
        assert!(
            (lambda / pair.lambda - 1.0).abs() < 1e-2,
            "λ {}-{} : {lambda} contre {}",
            pair.home,
            pair.away,
            pair.lambda
        );
        assert!(
            (mu / pair.mu - 1.0).abs() < 1e-2,
            "μ {}-{} : {mu} contre {}",
            pair.home,
            pair.away,
            pair.mu
        );
        let probs = fitted
            .grid(pair.home, pair.away)
            .unwrap()
            .home_draw_away()
            .unwrap()
            .to_array();
        for (ours, theirs) in probs.iter().zip([pair.p_home, pair.p_draw, pair.p_away]) {
            assert!(
                (ours - theirs).abs() < 3e-3,
                "1X2 {}-{} : {ours} contre {theirs}",
                pair.home,
                pair.away
            );
        }
    }
    eprintln!("écart relatif maximal sur λ et μ : {worst:.5}");
}
