//! Détection de valeur et dimensionnement des mises.
//!
//! Le programme ne parie jamais lui-même : il **recommande**. Ce module répond à deux questions :
//! « ce prix a-t-il de la valeur ? » ([`value`]) et « combien miser ? » ([`kelly`], [`simultaneous`]) ; [`live`]
//! les applique aux derniers relevés de l'archive et [`recommend`] écrit le résultat dans le journal immuable.

pub mod clv;
pub mod kelly;
pub mod ledger;
pub mod live;
pub mod overview;
pub mod paper;
pub mod recommend;
pub mod settle;
pub mod simultaneous;
pub mod value;

#[cfg(test)]
pub(crate) mod testing;
