//! Cœur d'`edge`.
//!
//! Cette bibliothèque contient toute la logique du projet, application Web comprise ([`web`] : pages et formulaires,
//! sans réseau). Elle ne connaît ni la ligne de commande ni le serveur HTTP : `edge-cli` passe uniquement par son API.

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
pub mod loto;
pub mod lotofoot;
pub mod market;
pub mod models;
pub mod notify;
pub mod paths;
pub mod report;
pub mod store;
pub mod web;
