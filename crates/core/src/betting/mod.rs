//! Détection de valeur et dimensionnement des mises.
//!
//! Le programme ne parie jamais lui-même : il **recommande**. Ce module répond à deux questions :
//! « ce prix a-t-il de la valeur ? » ([`value`]) et « combien miser ? » ([`kelly`], [`simultaneous`]) ; [`live`]
//! les applique aux derniers relevés de l'archive.

pub mod kelly;
pub mod live;
pub mod simultaneous;
pub mod value;
