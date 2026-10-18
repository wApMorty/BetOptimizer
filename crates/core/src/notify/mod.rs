//! Notifications sur téléphone : le texte d'une recommandation ([`message`]) et son envoi à Discord ([`discord`]).
//!
//! Aucun trait « canal de notification » : un seul canal existe (Discord, choix de Paul), et un trait n'est justifié
//! que par deux implémentations réelles.

pub mod discord;
pub mod message;
