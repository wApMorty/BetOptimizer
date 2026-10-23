//! Séance de recommandation (`edge lotofoot recommend`, le samedi midi) : les tickets de la semaine, écrits au journal.
//!
//! Déroulé : grilles LF7 et LF8 qui ferment au moins `lotofoot_min_hours_before_close` heures après la séance ; pour
//! chacune, la dernière répartition FDJ relevée, les mises estimées, les probabilités du marché (avec, au besoin, un
//! relevé des compétitions manquantes) ; puis les meilleurs tickets de chaque grille, le seuil et le budget sur
//! l'ensemble. Une grille incomplète est écartée avec la raison, jamais devinée. Aucune donnée postérieure à la séance
//! n'est lue (CLAUDE.md, invariant 1).

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde_json::json;

use super::Game;
use super::model::{FROZEN, GridModel, PESSIMISTIC_LOW_MULT, PESSIMISTIC_SHRINK, public_split};
use super::odds::{self, LeagueSearch, MatchMarket};
use super::optimize::{self, Candidate};
use crate::config::Config;
use crate::domain::Probs1x2;
use crate::error::Result;
use crate::ingest::aliases::Aliases;
use crate::ingest::snapshot::{Clock, Fetch};
use crate::store::lotofoot::{self as store, GridRow, NewReco, NewSession};

/// Version du modèle, enregistrée avec chaque recommandation.
pub const MODEL_VERSION: &str = "lotofoot-1";

/// Âge maximal de la répartition utilisée : au-delà, le relevé horaire a manqué et la grille est écartée.
pub const MAX_SPLIT_AGE_HOURS: i64 = 3;

/// Grilles passées dont on tire la médiane des mises.
const STAKES_WINDOW: usize = 8;

/// Les rencontres d'une grille se jouent dans les jours qui suivent sa clôture.
const MATCH_WINDOW_DAYS: i64 = 5;

/// Une grille écartée et pourquoi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Grille (« LF7 n°133 »).
    pub grid: String,
    /// Raison lisible.
    pub reason: String,
}

/// Une grille évaluée : ses mises estimées et son meilleur ticket.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluated {
    /// Formule.
    pub game: Game,
    /// Numéro.
    pub number: u32,
    /// Fin de validation.
    pub closes_at: DateTime<Utc>,
    /// Mises estimées.
    pub stakes: f64,
    /// Espérance défavorable du meilleur ticket.
    pub best_pessimistic: f64,
    /// Rencontres, dans l'ordre du bulletin (noms de prono-match).
    pub matches: Vec<(String, String)>,
    /// Modèle de la grille (marché, public, masses) : sert à dire ce que peut rapporter une grille à remplir.
    pub model: GridModel,
}

/// Bilan d'une séance.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionReport {
    /// Grilles évaluées.
    pub evaluated: Vec<Evaluated>,
    /// Grilles écartées.
    pub skipped: Vec<Skipped>,
    /// Noms d'équipes inconnus de la table d'alias (à ajouter dans `config/aliases.toml`).
    pub unknown_names: Vec<String>,
    /// Recherche des compétitions manquantes.
    pub leagues: LeagueSearch,
    /// Tickets retenus, du meilleur au moins bon.
    pub chosen: Vec<Candidate>,
    /// Séance enregistrée (`None` à blanc).
    pub session_id: Option<i64>,
}

/// Une grille candidate après la première étape : répartition, mises et noms canoniques connus, cotes encore à trouver.
struct Pending {
    row: GridRow,
    stakes: f64,
    split_fetch_id: i64,
    split: Vec<[u32; 3]>,
    canonical: Vec<(String, String)>,
}

/// Une grille prête : ses entrées et son modèle.
struct Ready {
    row: GridRow,
    stakes: f64,
    split_fetch_id: i64,
    markets: Vec<MatchMarket>,
    model: GridModel,
}

/// Lance la séance. `max_tickets` : mise plafond en euros (un ticket coûte 1 €), ramenée au plafond de la
/// configuration (`lotofoot_max_tickets`, 16 € gelés au `docs/sources.md` §37) s'il le dépasse. `dry_run` : rien n'est
/// écrit et aucune compétition n'est relevée (aucun crédit).
///
/// Une grille qui a déjà des tickets au journal est écartée : chaque grille n'est recommandée qu'une fois (séance du
/// bouton de l'application, puis timer du samedi). Une séance qui ne retient aucun ticket n'est pas écrite : elle n'a
/// rien à journaliser, et elle masquerait sinon la séance précédente (page `/lotofoot`, « J'ai joué la séance »).
pub fn run(
    conn: &mut Connection,
    config: &Config,
    aliases: &Aliases,
    now: Clock,
    fetch: Fetch,
    max_tickets: u32,
    dry_run: bool,
) -> Result<SessionReport> {
    let max_tickets = max_tickets.min(config.lotofoot_max_tickets);
    let started = now();
    let min_close = started + Duration::hours(i64::from(config.lotofoot_min_hours_before_close));
    let fresh_after = started - Duration::hours(i64::from(config.lotofoot_max_odds_age_hours));
    let mut report = SessionReport {
        evaluated: Vec::new(),
        skipped: Vec::new(),
        unknown_names: Vec::new(),
        leagues: LeagueSearch::default(),
        chosen: Vec::new(),
        session_id: None,
    };

    // 1. Grilles candidates, pas encore recommandées, avec répartition récente, noms connus et mises estimables.
    let recommended = store::recos(conn)?;
    let mut pending: Vec<Pending> = Vec::new();
    for row in store::grids_closing_after(conn, min_close)? {
        let label = format!("{} n°{}", row.game, row.number);
        let skip = |reason: String| Skipped {
            grid: label.clone(),
            reason,
        };
        if let Some(earlier) = recommended
            .iter()
            .find(|r| (r.game, r.number) == (row.game, row.number))
        {
            report.skipped.push(skip(format!(
                "déjà recommandée (séance n°{})",
                earlier.session_id
            )));
            continue;
        }
        let Some((split_fetch_id, split)) =
            store::latest_split(conn, row.game, row.number, started)?
        else {
            report
                .skipped
                .push(skip("répartition FDJ pas encore publiée".to_string()));
            continue;
        };
        if store::fetch_time(conn, split_fetch_id)? < started - Duration::hours(MAX_SPLIT_AGE_HOURS)
        {
            report.skipped.push(skip(format!(
                "répartition relevée il y a plus de {MAX_SPLIT_AGE_HOURS} h"
            )));
            continue;
        }
        let canonical = match odds::canonical_matches(aliases, &row.matches) {
            Ok(canonical) => canonical,
            Err(unknown) => {
                report.skipped.push(skip(format!(
                    "{} nom(s) d'équipe inconnu(s)",
                    unknown.len()
                )));
                report.unknown_names.extend(unknown);
                continue;
            }
        };
        let Some(stakes) = store::stakes_estimate(conn, row.game, row.number, STAKES_WINDOW)?
        else {
            report.skipped.push(skip(
                "aucun rapport passé pour estimer les mises".to_string(),
            ));
            continue;
        };
        pending.push(Pending {
            row,
            stakes: stakes.get(),
            split_fetch_id,
            split,
            canonical,
        });
    }

    // 2. Probabilités du marché ; relevé des compétitions manquantes (sauf à blanc), puis seconde lecture de l'archive.
    let window_of = |row: &GridRow| {
        (
            row.closes_at - Duration::hours(2),
            row.closes_at + Duration::days(MATCH_WINDOW_DAYS),
        )
    };
    let lookup = |conn: &Connection,
                  row: &GridRow,
                  canonical: &[(String, String)]|
     -> Result<Vec<Option<MatchMarket>>> {
        let (from, until) = window_of(row);
        canonical
            .iter()
            .map(|(h, a)| {
                odds::archived_market(
                    conn,
                    (h, a),
                    from.max(started),
                    until,
                    fresh_after,
                    config.margin_method,
                )
            })
            .collect()
    };
    let mut missing: Vec<(String, String)> = Vec::new();
    for Pending { row, canonical, .. } in &pending {
        for (market, teams) in lookup(conn, row, canonical)?.iter().zip(canonical) {
            if market.is_none() && !missing.contains(teams) {
                missing.push(teams.clone());
            }
        }
    }
    if !missing.is_empty() && !dry_run {
        let from = started;
        let until = pending
            .iter()
            .map(|p| window_of(&p.row).1)
            .max()
            .unwrap_or(started);
        report.leagues = odds::refresh_leagues(conn, config, &missing, (from, until), now, fetch)?;
    }

    let mut ready: Vec<Ready> = Vec::new();
    for Pending {
        row,
        stakes,
        split_fetch_id,
        split,
        canonical,
    } in pending
    {
        let label = format!("{} n°{}", row.game, row.number);
        let markets: Option<Vec<MatchMarket>> =
            lookup(conn, &row, &canonical)?.into_iter().collect();
        let Some(markets) = markets else {
            report.skipped.push(Skipped {
                grid: label,
                reason: "cote Pinnacle absente pour au moins une rencontre".to_string(),
            });
            continue;
        };
        let Some(model) = build_model(&row, stakes, &split, &markets) else {
            report.skipped.push(Skipped {
                grid: label,
                reason: "répartition ou cotes incohérentes".to_string(),
            });
            continue;
        };
        ready.push(Ready {
            row,
            stakes,
            split_fetch_id,
            markets,
            model,
        });
    }

    // 3. Tickets de chaque grille, puis seuil et budget sur l'ensemble de la séance.
    let mut all: Vec<Candidate> = Vec::new();
    for grid in &ready {
        let key = (grid.row.game, grid.row.number);
        let candidates =
            optimize::candidates(key, &grid.model, &FROZEN, config.lotofoot_frequency_weight);
        let best = candidates
            .iter()
            .map(|c| c.pessimistic.expected_return)
            .fold(f64::MIN, f64::max);
        report.evaluated.push(Evaluated {
            game: grid.row.game,
            number: grid.row.number,
            closes_at: grid.row.closes_at,
            stakes: grid.stakes,
            best_pessimistic: best,
            // `clone` : le bilan possède ses données, car il survit aux grilles préparées (`ready`).
            matches: grid.row.matches.clone(),
            // Même raison pour le modèle : le message de la séance s'en sert après la fin de `ready`.
            model: grid.model.clone(),
        });
        all.extend(candidates);
    }
    let budget = usize::try_from(max_tickets).unwrap_or(usize::MAX);
    report.chosen = optimize::select(all, config.lotofoot_threshold, budget);

    // 4. Journal immuable : la séance, ses paramètres, et chaque ticket avec tout ce qui a servi à le choisir.
    if !dry_run && !report.chosen.is_empty() {
        let params = json!({
            "kappa": FROZEN.kappa, "beta": FROZEN.beta, "phi": FROZEN.phi,
            "frequency_weight": config.lotofoot_frequency_weight, "threshold": config.lotofoot_threshold,
            "max_tickets": max_tickets, "pessimistic_shrink": PESSIMISTIC_SHRINK,
            "pessimistic_low_mult": PESSIMISTIC_LOW_MULT, "margin_method": config.margin_method.name(),
        })
        .to_string();
        let notes = json!({
            "evaluated": report.evaluated.iter().map(|e| json!({
                "grid": format!("{} n°{}", e.game, e.number), "stakes": e.stakes, "best_pessimistic": e.best_pessimistic,
            })).collect::<Vec<_>>(),
            "skipped": report.skipped.iter().map(|s| json!({"grid": s.grid, "reason": s.reason})).collect::<Vec<_>>(),
            "unknown_names": report.unknown_names,
            "leagues_fetched": report.leagues.fetched, "credits_spent": report.leagues.spent,
        })
        .to_string();
        // Les entrées JSON sont construites d'abord : `NewReco` ne fait qu'emprunter ces textes, qui doivent donc vivre
        // jusqu'à l'écriture.
        let inputs: Vec<String> = report
            .chosen
            .iter()
            .map(|c| {
                let grid = ready.iter().find(|g| (g.row.game, g.row.number) == c.grid);
                grid.map(inputs_json).unwrap_or_default()
            })
            .collect();
        let recos: Vec<NewReco<'_>> = report
            .chosen
            .iter()
            .zip(&inputs)
            .enumerate()
            .filter_map(|(i, (c, inputs))| {
                let grid = ready
                    .iter()
                    .find(|g| (g.row.game, g.row.number) == c.grid)?;
                Some(NewReco {
                    game: c.grid.0,
                    number: c.grid.1,
                    ticket: &c.ticket,
                    position: i + 1,
                    ev_base: c.base.expected_return,
                    ev_pessimistic: c.pessimistic.expected_return,
                    paid_probability: c.base.paid_probability,
                    stakes_estimate: grid.stakes,
                    split_fetch_id: grid.split_fetch_id,
                    inputs,
                })
            })
            .collect();
        let session = NewSession {
            created_at: started,
            model_version: MODEL_VERSION,
            params: &params,
            notes: &notes,
        };
        report.session_id = Some(store::record_session(conn, &session, &recos)?);
    }
    Ok(report)
}

/// Modèle d'une grille : marché, répartition normalisée, mises et masses (pactole compris).
fn build_model(
    row: &GridRow,
    stakes: f64,
    split: &[[u32; 3]],
    markets: &[MatchMarket],
) -> Option<GridModel> {
    let q = public_split(split)?;
    let p: Vec<Probs1x2> = markets.iter().map(|m| m.probs).collect();
    let stakes = crate::domain::Euros::new(stakes)?;
    let masses = GridModel::masses_for(row.game, stakes, row.jackpot);
    GridModel::new(&p, &q, stakes, &masses)
}

/// Entrées du calcul d'un ticket, figées dans le journal.
fn inputs_json(grid: &Ready) -> String {
    let p: Vec<[f64; 3]> = (0..grid.model.len())
        .filter_map(|i| grid.model.market(i))
        .collect();
    let q: Vec<[f64; 3]> = (0..grid.model.len())
        .filter_map(|i| grid.model.public(i))
        .collect();
    let calls: Vec<i64> = grid.markets.iter().map(|m| m.api_call_id).collect();
    json!({
        "p": p, "q": q, "stakes": grid.stakes, "jackpot": grid.row.jackpot.map(|j| j.get()),
        "odds_calls": calls, "matches": grid.row.matches,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Euros, Quota};
    use crate::ingest::client::HttpResponse;
    use crate::lotofoot::pronomatch::{GridPage, OpenGrid, PastRanks, RankLine};
    use crate::store::archive::{self, Endpoint, NewApiCall};
    use crate::store::lotofoot::{Page, record_fetch, record_grid_page};
    use crate::store::migrate::migrate;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// Une grille LF7 synthétique de 7 rencontres « Hi – Ai », avec sa répartition, et les cotes Pinnacle archivées.
    fn setup(conn: &mut Connection, aliases_text: &mut String) {
        let teams: Vec<(String, String)> = (1..=7)
            .map(|i| (format!("H{i}"), format!("A{i}")))
            .collect();
        let fetch_id = record_fetch(
            conn,
            at("2026-10-24T09:30:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        let page = GridPage {
            open: Some(OpenGrid {
                game: Game::Lf7,
                accepting: true,
                number: 133,
                closes_at: at("2026-10-25T16:10:00Z"),
                jackpot: Euros::new(100_000.0),
                matches: teams.clone(),
                split: Some(vec![
                    [61, 20, 19],
                    [31, 24, 45],
                    [70, 18, 12],
                    [45, 26, 29],
                    [35, 25, 40],
                    [47, 25, 28],
                    [55, 25, 20],
                ]),
                split_at: None,
            }),
            history: vec![PastRanks {
                number: 132,
                ranks: vec![
                    RankLine {
                        good: None,
                        winners: 135,
                        unit_gain: Euros::new(578.0),
                    },
                    RankLine {
                        good: None,
                        winners: 2393,
                        unit_gain: Euros::new(39.8),
                    },
                ],
            }],
        };
        record_grid_page(conn, fetch_id, Game::Lf7, &page).unwrap();
        aliases_text.push_str("[LOTOFOOT]\n");
        for (h, a) in &teams {
            aliases_text.push_str(&format!("\"{h}\" = []\n\"{a}\" = []\n"));
        }
        // Cotes Pinnacle archivées pour les 7 rencontres, relevées la veille à midi.
        let body: Vec<String> = teams
            .iter()
            .enumerate()
            .map(|(i, (h, a))| {
                let (oh, od, oa) = [(2.1, 3.6, 3.4), (2.3, 3.5, 3.1), (1.6, 4.0, 5.5), (2.8, 3.5, 2.5), (1.9, 3.8, 3.9), (2.2, 3.7, 3.2), (2.0, 3.6, 3.7)][i];
                format!(
                    r#"{{"id":"e{i}","sport_key":"soccer_x","commence_time":"2026-10-25T18:00:00Z","home_team":"{h}","away_team":"{a}",
                    "bookmakers":[{{"key":"pinnacle","markets":[{{"key":"h2h","last_update":"2026-10-23T11:59:00Z",
                    "outcomes":[{{"name":"{h}","price":{oh}}},{{"name":"Draw","price":{od}}},{{"name":"{a}","price":{oa}}}]}}]}}]}}"#
                )
            })
            .collect();
        let body = format!("[{}]", body.join(","));
        let called_at = at("2026-10-23T12:00:00Z");
        let call_id = archive::record_api_call(
            conn,
            &NewApiCall {
                called_at,
                endpoint: Endpoint::Odds,
                sport_key: Some("soccer_x"),
                params: "",
                http_status: 200,
                quota: Quota::default(),
                body: Some(&body),
            },
        )
        .unwrap();
        let parsed = crate::ingest::odds_api::parse_odds(&body, called_at).unwrap();
        archive::record_snapshot(conn, call_id, &parsed.events).unwrap();
    }

    #[test]
    fn a_session_records_tickets_above_the_threshold_within_the_budget() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let config = Config {
            lotofoot_threshold: 1.0,
            ..Config::default()
        };
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let report = run(
            &mut conn, &config, &aliases, &mut clock, &mut fetch, 16, false,
        )
        .unwrap();
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        assert_eq!(report.evaluated.len(), 1);
        assert!(!report.chosen.is_empty() && report.chosen.len() <= 16);
        assert!(
            report
                .chosen
                .iter()
                .all(|c| c.pessimistic.expected_return >= 1.0)
        );
        let recos = store::recos(&conn).unwrap();
        assert_eq!(recos.len(), report.chosen.len());
        assert_eq!(recos[0].position, 1);
    }

    #[test]
    fn the_stake_caps_the_tickets_and_a_grid_is_recommended_only_once() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let config = Config {
            lotofoot_threshold: 1.0,
            ..Config::default()
        };
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        // Mise de 3 € : 3 tickets au plus ; une mise au-delà du plafond gelé (16 €) y est ramenée.
        let first = run(
            &mut conn, &config, &aliases, &mut clock, &mut fetch, 3, false,
        )
        .unwrap();
        assert_eq!(first.chosen.len(), 3);
        assert!(first.session_id.is_some());
        // Seconde séance sur la même grille : écartée, et rien n'est écrit (aucun ticket retenu).
        let again = run(
            &mut conn, &config, &aliases, &mut clock, &mut fetch, 100, false,
        )
        .unwrap();
        assert!(again.chosen.is_empty());
        assert!(
            again.skipped[0]
                .reason
                .starts_with("déjà recommandée (séance n°"),
            "{:?}",
            again.skipped
        );
        assert!(again.session_id.is_none());
        assert_eq!(store::recos(&conn).unwrap().len(), 3);
        assert!(
            store::last_session(&conn)
                .unwrap()
                .is_some_and(|(id, _, _)| Some(id) == first.session_id)
        );
    }

    #[test]
    fn a_session_without_tickets_is_not_written() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let aliases = Aliases::parse(&aliases_text).unwrap();
        // Seuil inatteignable : la grille est évaluée, aucun ticket ne passe.
        let config = Config {
            lotofoot_threshold: 1e9,
            ..Config::default()
        };
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let report = run(
            &mut conn, &config, &aliases, &mut clock, &mut fetch, 16, false,
        )
        .unwrap();
        assert_eq!(report.evaluated.len(), 1);
        assert!(report.chosen.is_empty() && report.session_id.is_none());
        assert!(store::last_session(&conn).unwrap().is_none());
    }

    #[test]
    fn stale_splits_unknown_names_and_dry_runs_are_handled() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let config = Config::default();
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };

        // Table d'alias vide : la grille est écartée, les 14 noms sont listés, rien n'est relevé ni écrit à blanc.
        let empty = Aliases::parse("").unwrap();
        let mut clock = || at("2026-10-24T10:00:00Z");
        let report = run(&mut conn, &config, &empty, &mut clock, &mut fetch, 16, true).unwrap();
        assert_eq!(report.unknown_names.len(), 14);
        assert!(report.session_id.is_none());

        // Séance à 14 h : la répartition de 9 h 30 a plus de 3 heures.
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let mut late = || at("2026-10-24T14:00:00Z");
        let report = run(
            &mut conn, &config, &aliases, &mut late, &mut fetch, 16, true,
        )
        .unwrap();
        assert!(report.skipped[0].reason.contains("plus de 3 h"));
    }
}
