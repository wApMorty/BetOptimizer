//! Cotes boostées : lecture de l'historique publié par WePari, traduction des intitulés en conditions sur le score
//! final, tarification contre le marché (Pinnacle sans marge) et étude de rentabilité.
//!
//! Une cote boostée n'a pas besoin de notre modèle : on la compare directement à la probabilité juste du marché.

pub mod pricing;
pub mod selection;
pub mod study;
pub mod wepari;

/// Opérateur qui a proposé la cote boostée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operator {
    /// Winamax.
    Winamax,
    /// Parions Sport en ligne (FDJ).
    ParionsSport,
}

impl Operator {
    /// Nom affiché.
    pub fn name(self) -> &'static str {
        match self {
            Operator::Winamax => "Winamax",
            Operator::ParionsSport => "Parions Sport",
        }
    }
}
