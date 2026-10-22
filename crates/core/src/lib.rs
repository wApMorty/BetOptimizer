//! Cœur d'`edge`.
//!
//! Cette bibliothèque contient toute la logique du projet. Elle ne connaît ni la ligne de commande
//! ni l'interface graphique : `edge-cli` (et plus tard `edge-gui`) passent uniquement par son API.

pub mod backtest;
pub mod betting;
pub mod bonus;
pub mod boost;
pub mod config;
pub mod domain;
pub mod error;
pub mod eval;
pub mod format;
pub mod ingest;
pub mod init;
pub mod lotofoot;
pub mod market;
pub mod models;
pub mod notify;
pub mod paths;
pub mod report;
pub mod store;
