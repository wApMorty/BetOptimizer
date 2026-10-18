//! « Buts attendus » estimés à partir des tirs : la variante *stat-first* des modèles.
//!
//! Les buts sont rares et bruités ; les tirs sont plus nombreux et plus stables. On estime combien de buts
//! rapporte en moyenne un tir cadré et un tir non cadré (régression des buts sur les deux), puis on s'en sert
//! comme cible partielle de l'ajustement des forces d'équipe. C'est un proxy grossier de xG : il ignore la
//! position et le contexte des tirs, que football-data ne publie pas.

/// Coefficients de la régression `buts ≈ c₁ · tirs cadrés + c₂ · tirs non cadrés` (sans constante).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XgProxy {
    /// Buts attendus par tir cadré.
    pub on_target: f64,
    /// Buts attendus par tir non cadré (bloqués ou à côté).
    pub off_target: f64,
}

/// Une observation : une équipe dans un match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotObservation {
    /// Tirs cadrés.
    pub on_target: u16,
    /// Tirs au total (les cadrés compris).
    pub shots: u16,
    /// Buts marqués.
    pub goals: u8,
}

impl XgProxy {
    /// Ajuste les coefficients par moindres carrés ordinaires, par les équations normales 2 × 2 :
    /// `[Σx₁² Σx₁x₂; Σx₁x₂ Σx₂²]·c = [Σx₁g; Σx₂g]` avec `x₁` = cadrés, `x₂` = non cadrés, `g` = buts.
    ///
    /// Un coefficient négatif n'a pas de sens (un tir ne retire pas de but) : il est ramené à 0 et l'autre est
    /// réajusté seul. Renvoie `None` sans observations exploitables.
    pub fn fit(observations: &[ShotObservation]) -> Option<XgProxy> {
        let (mut s11, mut s12, mut s22, mut s1g, mut s2g) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for obs in observations {
            let x1 = f64::from(obs.on_target);
            let x2 = f64::from(obs.shots.saturating_sub(obs.on_target));
            let goals = f64::from(obs.goals);
            s11 += x1 * x1;
            s12 += x1 * x2;
            s22 += x2 * x2;
            s1g += x1 * goals;
            s2g += x2 * goals;
        }
        let determinant = s11 * s22 - s12 * s12;
        if determinant.abs() > 1e-9 {
            let on_target = (s1g * s22 - s2g * s12) / determinant;
            let off_target = (s2g * s11 - s1g * s12) / determinant;
            if on_target >= 0.0 && off_target >= 0.0 {
                return Some(XgProxy {
                    on_target,
                    off_target,
                });
            }
        }
        // Repli : un seul régresseur à la fois ; on garde celui qui explique le mieux (somme des carrés la plus basse).
        let only_on_target = (s11 > 0.0).then(|| XgProxy {
            on_target: (s1g / s11).max(0.0),
            off_target: 0.0,
        });
        let only_off_target = (s22 > 0.0).then(|| XgProxy {
            on_target: 0.0,
            off_target: (s2g / s22).max(0.0),
        });
        let residual = |proxy: &XgProxy| -> f64 {
            observations
                .iter()
                .map(|obs| {
                    (f64::from(obs.goals) - proxy.expected(obs.shots, obs.on_target)).powi(2)
                })
                .sum()
        };
        match (only_on_target, only_off_target) {
            (Some(a), Some(b)) => Some(if residual(&a) <= residual(&b) { a } else { b }),
            (a, b) => a.or(b),
        }
    }

    /// Buts attendus d'une équipe pour `shots` tirs dont `on_target` cadrés.
    pub fn expected(&self, shots: u16, on_target: u16) -> f64 {
        self.on_target * f64::from(on_target)
            + self.off_target * f64::from(shots.saturating_sub(on_target))
    }
}

/// Qualité du proxy contre des xG publiés : corrélation de Pearson et rapport des moyennes (proxy / publié).
///
/// Chaque élément est `(buts attendus par le proxy, xG publié)`. Renvoie `None` sous deux éléments ou si l'une des
/// séries est constante.
pub fn agreement(pairs: &[(f64, f64)]) -> Option<(f64, f64)> {
    if pairs.len() < 2 {
        return None;
    }
    let n = pairs.len() as f64;
    let mean_proxy = pairs.iter().map(|p| p.0).sum::<f64>() / n;
    let mean_published = pairs.iter().map(|p| p.1).sum::<f64>() / n;
    let covariance: f64 = pairs
        .iter()
        .map(|p| (p.0 - mean_proxy) * (p.1 - mean_published))
        .sum();
    let variance_proxy: f64 = pairs.iter().map(|p| (p.0 - mean_proxy).powi(2)).sum();
    let variance_published: f64 = pairs.iter().map(|p| (p.1 - mean_published).powi(2)).sum();
    if variance_proxy <= 0.0 || variance_published <= 0.0 || mean_published == 0.0 {
        return None;
    }
    Some((
        covariance / (variance_proxy * variance_published).sqrt(),
        mean_proxy / mean_published,
    ))
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    fn obs(on_target: u16, shots: u16, goals: u8) -> ShotObservation {
        ShotObservation {
            on_target,
            shots,
            goals,
        }
    }

    #[test]
    fn the_regression_recovers_exact_coefficients() {
        // Données construites avec buts = 0,5 · cadrés + 0,25 · non cadrés (valeurs choisies pour des buts entiers).
        let observations = [
            obs(4, 8, 3),
            obs(2, 6, 2),
            obs(6, 6, 3),
            obs(0, 8, 2),
            obs(2, 2, 1),
        ];

        let proxy = XgProxy::fit(&observations).unwrap();

        assert_abs_diff_eq!(proxy.on_target, 0.5, epsilon = 1e-9);
        assert_abs_diff_eq!(proxy.off_target, 0.25, epsilon = 1e-9);
    }

    #[test]
    fn a_negative_coefficient_is_clamped_and_the_other_refitted() {
        // Plus de tirs non cadrés → moins de buts : la régression libre donnerait un coefficient négatif.
        let observations = [
            obs(5, 6, 2),
            obs(5, 20, 1),
            obs(5, 40, 1),
            obs(2, 3, 1),
            obs(2, 30, 0),
            obs(8, 9, 3),
        ];

        let proxy = XgProxy::fit(&observations).unwrap();

        assert!(
            proxy.on_target >= 0.0 && proxy.off_target >= 0.0,
            "{proxy:?}"
        );
    }

    #[test]
    fn no_observation_gives_no_proxy() {
        assert!(XgProxy::fit(&[]).is_none());
    }

    #[test]
    fn expected_goals_follow_the_coefficients() {
        let proxy = XgProxy {
            on_target: 0.3,
            off_target: 0.05,
        };
        assert_abs_diff_eq!(proxy.expected(20, 10), 3.5, epsilon = 1e-12);
        assert_abs_diff_eq!(proxy.expected(0, 0), 0.0);
    }

    #[test]
    fn agreement_measures_correlation_and_scale() {
        // Proxy = 2 × publié : corrélation parfaite, rapport 2.
        let pairs = [(2.0, 1.0), (4.0, 2.0), (1.0, 0.5), (6.0, 3.0)];
        let (correlation, ratio) = agreement(&pairs).unwrap();
        assert_abs_diff_eq!(correlation, 1.0, epsilon = 1e-12);
        assert_abs_diff_eq!(ratio, 2.0, epsilon = 1e-12);
        // Anticorrélation parfaite.
        let (correlation, _) = agreement(&[(1.0, 3.0), (2.0, 2.0), (3.0, 1.0)]).unwrap();
        assert_abs_diff_eq!(correlation, -1.0, epsilon = 1e-12);
    }

    #[test]
    fn agreement_needs_variation() {
        assert!(agreement(&[(1.0, 1.0)]).is_none());
        assert!(agreement(&[(1.0, 1.0), (1.0, 2.0)]).is_none());
    }
}
