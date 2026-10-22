//! Évaluation de prévisions probabilistes : log loss, Brier, RPS et intervalles de confiance.
//!
//! Règle du projet : on juge un modèle (ou un marché) sur ces scores et sur la calibration, jamais sur le
//! taux de bons pronostics. Plus un score est bas, meilleure est la prévision.

pub mod bootstrap;
pub mod latency;
pub mod metrics;
