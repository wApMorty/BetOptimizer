//! Backtest : rejouer le passé avec les règles de mise, sur les seules informations disponibles à l'époque.
//!
//! Principe (invariant 4) : une décision se prend sur les cotes **d'avant-match** ; la cote de clôture ne sert qu'à
//! mesurer, après coup, si l'on a obtenu un meilleur prix que le marché final (le CLV).

pub mod candidates;
pub mod engine;
pub mod metrics;
pub mod study;
