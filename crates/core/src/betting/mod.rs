//! Détection de valeur et dimensionnement des mises.
//!
//! Le programme ne parie jamais lui-même : il **recommande**. Ce module répond à deux questions :
//! « ce prix a-t-il de la valeur ? » ([`value`]) et « combien miser ? » ([`kelly`], [`simultaneous`]).

pub mod kelly;
pub mod value;
