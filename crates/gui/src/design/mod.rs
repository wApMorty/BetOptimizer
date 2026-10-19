//! Système de design de l'application : jetons, style egui, mouvement, icônes et composants.
//!
//! Principes (voir `docs/learning/phase-9.md`) : une hiérarchie visuelle par la taille, le poids et le contraste ; l'espace
//! plutôt que les traits pour regrouper ; une seule couleur d'accent ; au plus quelques indicateurs par écran ; le détail
//! révélé à la demande ; des mouvements brefs qui disent quelque chose et que l'on peut couper.

pub mod icons;
pub mod motion;
pub mod style;
pub mod tokens;
pub mod widgets;
