//! Grandeurs du domaine, typées pour que le compilateur interdise de les confondre.

/// Cote décimale d'un bookmaker : gain total pour 1 € misé, mise comprise (2,10 → 2,10 € rendus).
///
/// Newtype *validé* : le champ est privé, donc la seule façon d'obtenir une `Odds` est [`Odds::new`],
/// qui refuse les valeurs impossibles. Une fonction qui reçoit une `Odds` n'a jamais à revérifier.
/// `Copy` : le type pèse 8 octets, on le passe par valeur sans emprunt ni `clone()`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Odds(f64);

impl Odds {
    /// Crée une cote. Renvoie `None` si la valeur n'est pas un nombre fini strictement supérieur à 1
    /// (une cote de 1,00 ne rapporte rien ; en dessous, c'est une valeur erronée).
    pub fn new(value: f64) -> Option<Odds> {
        if value.is_finite() && value > 1.0 {
            Some(Odds(value))
        } else {
            None
        }
    }

    /// Valeur décimale de la cote.
    pub fn get(self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_odds_are_accepted() {
        assert_eq!(Odds::new(1.01).unwrap().get(), 1.01);
        assert_eq!(Odds::new(11.44).unwrap().get(), 11.44);
    }

    #[test]
    fn impossible_values_are_refused() {
        for value in [1.0, 0.99, 0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert!(Odds::new(value).is_none(), "{value} doit être refusée");
        }
    }
}
