//! Rapports HTML autonomes (lisibles sur téléphone) : tableaux et graphiques SVG intégrés au fichier.
//! [`site`] les publie sur le réseau local (`edge serve`), avec l'archive des rapports quotidiens ([`daily`]).

pub mod backtest_report;
pub mod daily;
pub mod market_report;
pub mod models_report;
pub mod nba_report;
pub mod picks_image;
pub mod recommend_report;
pub mod render;
pub mod site;
pub mod svg;
pub mod tennis_report;
pub mod thresholds_report;
