//! Séance d'un Grand Loto : prévision du nombre de grilles, validation des préférences du public, choix des grilles,
//! décision, écriture au journal (une séance par édition, jamais refaite).
//!
//! Règle (choix de Paul du 2026-10-23, `docs/sources.md` §44) : budget de 20 €, soit quatre grilles à 5 € ; une grille
//! est jouée si ses euros rendus par euro misé atteignent 1,05 au nombre de grilles pessimiste (80e centile de la
//! prévision). Les grilles suivent les préférences mesurées si elles passent le critère du walk-forward
//! ([`super::popularity::Verdict`]) ; sinon elles sont tirées au hasard, comme un « Flash ».

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::Connection;
use serde_json::json;

use super::estimate::{self, EditionOutcome, Forecast};
use super::expectation::{self, Edition};
use super::popularity::{self, Preferences, Verdict, WalkForward};
use super::{Game, Grid};
use crate::config::{Config, GrandLotoEdition};
use crate::error::{Error, Result};
use crate::store::loto::{self as store, NewReco, NewSession, SessionRow};

/// Version du modèle, écrite avec chaque séance (invariant 9).
pub const MODEL_VERSION: &str = "grandloto-1";

/// Une grille évaluée.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Candidate {
    /// La grille.
    pub grid: Grid,
    /// Euros rendus, nombre de grilles central.
    pub ev: f64,
    /// Euros rendus, nombre de grilles pessimiste.
    pub ev_pessimistic: f64,
    /// Popularité relative de la grille selon les préférences mesurées (1 : grille moyenne).
    pub popularity: f64,
    /// Retenue : son espérance pessimiste atteint le seuil.
    pub kept: bool,
}

/// Bilan d'une séance.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionReport {
    /// Date de l'édition.
    pub date: NaiveDate,
    /// L'édition (jackpot, prix, codes).
    pub edition: Edition,
    /// Prévision du nombre de grilles.
    pub forecast: Forecast,
    /// Validation des préférences.
    pub walk: WalkForward,
    /// Euros rendus par une grille au hasard : (central, pessimiste).
    pub uniform: (f64, f64),
    /// Grilles évaluées, dans l'ordre de recommandation.
    pub candidates: Vec<Candidate>,
    /// Seuil appliqué (euros rendus par euro misé).
    pub threshold: f64,
    /// Séance écrite (`None` à blanc).
    pub session_id: Option<i64>,
}

impl SessionReport {
    /// Grilles à jouer.
    pub fn kept(&self) -> impl Iterator<Item = &Candidate> {
        self.candidates.iter().filter(|c| c.kept)
    }
}

/// Résultat d'un lancement.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Aucune édition dans les jours qui viennent (`grand_loto_days_before`).
    NotDue,
    /// La séance de l'édition existe déjà : le timer ne la refait jamais.
    AlreadyDone(SessionRow),
    /// Séance faite (ou calculée à blanc).
    Done(Box<SessionReport>),
}

/// Prochaine édition à traiter le `today` : la plus proche dont la date est entre aujourd'hui et
/// `grand_loto_days_before` jours plus tard.
pub fn due_edition(config: &Config, today: NaiveDate) -> Option<GrandLotoEdition> {
    config
        .grand_loto_editions
        .iter()
        .filter(|e| {
            let days = (e.date - today).num_days();
            (0..=i64::from(config.grand_loto_days_before)).contains(&days)
        })
        .min_by_key(|e| e.date)
        .copied()
}

/// Lance la séance de l'édition `date` (ou de l'édition due aujourd'hui si `date` est `None`). À blanc, rien n'est écrit.
pub fn run(
    conn: &mut Connection,
    config: &Config,
    now: DateTime<Utc>,
    date: Option<NaiveDate>,
    dry_run: bool,
) -> Result<Outcome> {
    let today = now.date_naive();
    let edition = match date {
        Some(date) => config
            .grand_loto_editions
            .iter()
            .find(|e| e.date == date)
            .copied()
            .ok_or_else(|| {
                Error::Input(format!(
                    "aucune édition du {date} dans grand_loto_editions (edge.toml)"
                ))
            })?,
        None => match due_edition(config, today) {
            Some(edition) => edition,
            None => return Ok(Outcome::NotDue),
        },
    };
    if let Some(done) = store::session_for(conn, edition.date)? {
        return Ok(Outcome::AlreadyDone(done));
    }
    let mut report = evaluate(conn, config, &edition)?;
    if !dry_run {
        report.session_id = Some(write(conn, config, now, &report)?);
    }
    Ok(Outcome::Done(Box::new(report)))
}

/// Calcule la séance sans rien écrire.
pub fn evaluate(
    conn: &Connection,
    config: &Config,
    edition: &GrandLotoEdition,
) -> Result<SessionReport> {
    let outcomes: Vec<EditionOutcome> = store::draws(conn, Game::GrandLoto)?
        .iter()
        .filter_map(estimate::edition_outcome)
        .collect();
    let forecast = estimate::forecast(&outcomes, edition.date, config.grand_loto_quantile)?;
    // Aucune fuite du futur : seuls les Loto d'avant l'édition servent aux préférences.
    let loto: Vec<_> = store::draws(conn, Game::Loto)?
        .into_iter()
        .filter(|d| d.date < edition.date)
        .collect();
    let walk = popularity::walk_forward(&loto)?;
    // Les préférences mesurées sur tous les Loto connus servent à afficher la popularité des grilles ; elles ne guident
    // le choix et l'espérance que si le walk-forward les valide.
    let measured = popularity::fit(&loto)?;
    let prefs = match walk.verdict {
        Verdict::Validated => measured.clone(),
        Verdict::Rejected => Preferences::uniform(),
    };
    let game = Edition {
        jackpot: edition.jackpot,
        price: edition.price,
        codes: edition.codes,
        code_gain: edition.code_gain,
    };
    let count = (config.grand_loto_budget / edition.price).floor() as usize;
    let grids: Vec<Grid> = match walk.verdict {
        Verdict::Validated => expectation::best_grids(&prefs, &game, forecast.central, count)
            .into_iter()
            .map(|(grid, _)| grid)
            .collect(),
        Verdict::Rejected => expectation::random_grids(seed(edition.date), count),
    };
    let threshold = config.grand_loto_threshold;
    let candidates = grids
        .into_iter()
        .map(|grid| {
            let ev = expectation::grid_value(&prefs, &grid, &game, forecast.central);
            let ev_pessimistic =
                expectation::grid_value(&prefs, &grid, &game, forecast.pessimistic);
            Candidate {
                grid,
                ev,
                ev_pessimistic,
                popularity: measured.grid_popularity(&grid),
                kept: ev_pessimistic / edition.price >= threshold,
            }
        })
        .collect();
    let uniform = Preferences::uniform();
    let any = Grid::new([1, 2, 3, 4, 5], 1).expect("grille valide écrite en dur");
    Ok(SessionReport {
        date: edition.date,
        edition: game,
        uniform: (
            expectation::grid_value(&uniform, &any, &game, forecast.central),
            expectation::grid_value(&uniform, &any, &game, forecast.pessimistic),
        ),
        forecast,
        walk,
        candidates,
        threshold,
        session_id: None,
    })
}

/// Graine des grilles au hasard : la date de l'édition (`20261225`), pour des grilles reproductibles.
fn seed(date: NaiveDate) -> u64 {
    date.format("%Y%m%d").to_string().parse().unwrap_or(0)
}

/// Écrit la séance et ses grilles retenues (aucune si la décision est de ne pas jouer).
fn write(
    conn: &mut Connection,
    config: &Config,
    now: DateTime<Utc>,
    report: &SessionReport,
) -> Result<i64> {
    let params = json!({
        "edition": report.edition,
        "budget": config.grand_loto_budget,
        "threshold": report.threshold,
        "quantile": report.forecast.quantile,
        "preferences": format!("{:?}", report.walk.verdict),
    });
    let kept: Vec<NewReco> = report
        .kept()
        .map(|c| NewReco {
            grid: c.grid,
            stake: report.edition.price,
            ev: c.ev,
            ev_pessimistic: c.ev_pessimistic,
        })
        .collect();
    let notes = json!({
        "forecast": report.forecast,
        "walk_forward": {
            "gain": report.walk.gain,
            "slope": report.walk.slope,
            "years": report.walk.years,
        },
        "uniform_value": { "central": report.uniform.0, "pessimistic": report.uniform.1 },
        "candidates": report.candidates,
        "decision": if kept.is_empty() { "ne pas jouer" } else { "jouer" },
    });
    store::record_session(
        conn,
        &NewSession {
            created_at: now,
            draw_date: report.date,
            model_version: MODEL_VERSION,
            params: &params,
            notes: &notes,
            recos: &kept,
        },
    )
}

/// Message Discord de la séance.
pub fn discord_text(report: &SessionReport) -> String {
    let euros = |v: f64| format!("{v:.2}").replace('.', ",");
    let millions = |n: f64| format!("{:.1}", n / 1e6).replace('.', ",");
    let jackpot = report.edition.jackpot / 1e6;
    let mut text = format!(
        "**Grand Loto du {}** · jackpot {jackpot:.0} M€, grille à {} €\n",
        report.date.format("%d/%m/%Y"),
        euros(report.edition.price)
    );
    text.push_str(&format!(
        "Grilles prévues : {} millions (pessimiste : {} millions, {} éditions de la saison « {} »).\n",
        millions(report.forecast.central),
        millions(report.forecast.pessimistic),
        report.forecast.editions.len(),
        report.forecast.season.label()
    ));
    text.push_str(&format!(
        "Une grille au hasard rend {} € ({} € au pessimiste) pour {} € misés.\n",
        euros(report.uniform.0),
        euros(report.uniform.1),
        euros(report.edition.price)
    ));
    let choice = match report.walk.verdict {
        Verdict::Validated => {
            "numéros délaissés par le public (préférences validées hors échantillon)"
        }
        Verdict::Rejected => {
            "grilles au hasard (préférences du public non validées hors échantillon)"
        }
    };
    let kept: Vec<&Candidate> = report.kept().collect();
    if kept.is_empty() {
        text.push_str(&format!(
            "**Ne pas jouer** : aucune grille n'atteint {} € rendus par euro au nombre de grilles pessimiste.",
            euros(report.threshold)
        ));
    } else {
        text.push_str(&format!("**À jouer** ({choice}) :\n"));
        for c in kept {
            text.push_str(&format!(
                "- `{}` : {} € rendus ({} € au pessimiste)\n",
                c.grid,
                euros(c.ev),
                euros(c.ev_pessimistic)
            ));
        }
        text.push_str("Après validation : `edge loto played --last-session`.");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loto::fdj::{self, tests::GRAND_LOTO_CSV};
    use crate::loto::popularity::tests::{planted, simulated};
    use crate::store::migrate::migrate;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn config(dates: &[NaiveDate]) -> Config {
        Config {
            grand_loto_editions: dates
                .iter()
                .map(|&date| GrandLotoEdition {
                    date,
                    jackpot: 20e6,
                    price: 5.0,
                    codes: 100,
                    code_gain: 20_000.0,
                })
                .collect(),
            ..Config::default()
        }
    }

    /// Base avec le Grand Loto de Noël 2025 et des Loto simulés aux préférences plantées.
    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let fetch = store::record_fetch(&conn, Utc::now(), "test", 200, 0, None).unwrap();
        let grand = fdj::parse_csv(GRAND_LOTO_CSV, Game::GrandLoto).unwrap();
        store::record_draws(&mut conn, fetch, &grand).unwrap();
        store::record_draws(&mut conn, fetch, &simulated(&planted(), 600, 31)).unwrap();
        conn
    }

    #[test]
    fn an_edition_is_due_only_in_its_window() {
        let config = config(&[day(2026, 12, 25), day(2027, 7, 3)]);
        assert_eq!(due_edition(&config, day(2026, 12, 21)), None);
        assert_eq!(
            due_edition(&config, day(2026, 12, 22)).map(|e| e.date),
            Some(day(2026, 12, 25))
        );
        assert_eq!(
            due_edition(&config, day(2026, 12, 25)).map(|e| e.date),
            Some(day(2026, 12, 25))
        );
        assert_eq!(due_edition(&config, day(2026, 12, 26)), None);
    }

    #[test]
    fn a_session_is_written_once_with_validated_preferences() {
        let mut conn = database();
        let config = config(&[day(2026, 12, 25)]);
        let now: DateTime<Utc> = "2026-12-22T21:30:00Z".parse().unwrap();

        let Outcome::Done(report) = run(&mut conn, &config, now, None, false).unwrap() else {
            panic!("séance attendue");
        };
        assert_eq!(report.walk.verdict, Verdict::Validated);
        assert_eq!(report.candidates.len(), 4);
        // Une seule édition de Noël connue (6,84 millions) : le central est cette valeur.
        assert!((report.forecast.central / 1e6 - 6.84).abs() < 0.01);
        assert!(report.candidates.iter().all(|c| c.popularity < 1.0));
        assert!(report.candidates.iter().all(|c| c.ev > report.uniform.0));
        assert!(report.session_id.is_some());
        let kept = report.kept().count();
        assert_eq!(store::recos(&conn).unwrap().len(), kept);
        assert!(discord_text(&report).contains("Grand Loto du 25/12/2026"));

        assert!(matches!(
            run(&mut conn, &config, now, None, false).unwrap(),
            Outcome::AlreadyDone(_)
        ));
    }

    #[test]
    fn a_dry_run_writes_nothing_and_an_unknown_date_is_refused() {
        let mut conn = database();
        let config = config(&[day(2026, 12, 25)]);
        let now: DateTime<Utc> = "2026-10-23T12:00:00Z".parse().unwrap();
        assert_eq!(
            run(&mut conn, &config, now, None, false).unwrap(),
            Outcome::NotDue
        );
        let Outcome::Done(report) =
            run(&mut conn, &config, now, Some(day(2026, 12, 25)), true).unwrap()
        else {
            panic!("séance à blanc attendue");
        };
        assert_eq!(report.session_id, None);
        assert!(store::sessions(&conn).unwrap().is_empty());
        assert!(run(&mut conn, &config, now, Some(day(2026, 12, 24)), true).is_err());
    }

    #[test]
    fn the_random_seed_is_the_edition_date() {
        assert_eq!(seed(day(2026, 12, 25)), 20_261_225);
    }
}
