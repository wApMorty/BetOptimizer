//! Modèles de prévision des scores : Poisson et Dixon-Coles.
//!
//! Un modèle estime, pour chaque match, les buts attendus de chaque équipe, puis en déduit une grille de scores
//! d'où l'on lit les probabilités du 1X2, du plus/moins et de « les deux équipes marquent ».

pub mod grid;
