//! Rapports HTML autonomes : tableaux et graphiques SVG intégrés au fichier. L'application Web ([`crate::web`]) les publie
//! sur le réseau local (`edge serve`), avec l'archive des rapports quotidiens ([`daily`]).

pub mod backtest_report;
pub mod daily;
pub mod latency_page;
pub mod lotofoot_image;
pub mod lotofoot_page;
pub mod market_report;
pub mod models_report;
pub mod nba_report;
pub mod picks_image;
pub mod recommend_report;
pub mod render;
pub mod svg;
pub mod tennis_report;
pub mod theme;
pub mod thresholds_report;
