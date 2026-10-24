//! Séance de recommandation (bouton « Optimiser maintenant » au guichet, ou `edge lotofoot recommend`) : les tickets à
//! jouer, écrits au journal.
//!
//! Déroulé : grilles LF7 et LF8 qui ferment au moins `lotofoot_min_minutes_before_close` minutes après la séance ; pour
//! chacune, la dernière répartition FDJ relevée, les mises estimées, les probabilités du marché (avec, au besoin, un
//! relevé des compétitions manquantes) ; puis les meilleurs tickets de chaque grille et leur répartition dans le budget
//! du mois, grille par grille dans l'ordre de clôture, au-dessus du seuil τ que l'abaque tire du budget (§49, §51). Une
//! grille incomplète est écartée avec la raison, jamais devinée ; une grille trop jeune est évaluée mais n'engage pas de
//! budget. Aucune donnée postérieure à la séance n'est lue (CLAUDE.md, invariant 1).

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Europe::Paris;
use rusqlite::Connection;
use serde_json::json;

use super::Game;
use super::abaque;
use super::model::{FROZEN, GridModel, PESSIMISTIC_LOW_MULT, PESSIMISTIC_SHRINK, public_split};
use super::odds::{self, LeagueSearch, MatchMarket};
use super::optimize::{self, Candidate};
use crate::config::Config;
use crate::domain::Probs1x2;
use crate::error::Result;
use crate::ingest::aliases::Aliases;
use crate::ingest::snapshot::{Clock, Fetch};
use crate::store::lotofoot::{self as store, GridRow, NewReco, NewSession, RecoRow};

/// Version du modèle, enregistrée avec chaque recommandation.
pub const MODEL_VERSION: &str = "lotofoot-1";

/// Âge maximal de la répartition utilisée : au-delà, le relevé horaire a manqué et la grille est écartée.
pub const MAX_SPLIT_AGE_HOURS: i64 = 3;

/// Grilles passées dont on tire la médiane des mises.
const STAKES_WINDOW: usize = 8;

/// Les rencontres d'une grille se jouent dans les jours qui suivent sa clôture.
const MATCH_WINDOW_DAYS: i64 = 5;

/// Âge minimal d'une grille (heures depuis sa première apparition dans un relevé) pour engager du budget : en deçà, sa
/// répartition bouge encore beaucoup (de 2 à 14 points en moyenne sur les trois premières grilles relevées). Valeur
/// provisoire, choisie par Paul le 2026-10-24, à revoir avec l'étude de dérive (`scripts/lotofoot/PROTOCOLE_DERIVE.md`).
pub const MIN_GRID_AGE_HOURS: i64 = 8;

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
    /// Budget de chaque mois concerné par les grilles évaluées.
    pub months: Vec<MonthBudget>,
    /// Budget mensuel de la séance (euros) et seuil τ qu'en tire l'abaque.
    pub monthly_budget: u32,
    /// Seuil τ de la séance.
    pub threshold: f64,
}

/// Budget d'un mois calendaire (heure de Paris, selon la clôture des grilles), en tickets d'un euro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthBudget {
    /// Premier jour du mois.
    pub month: NaiveDate,
    /// Budget du mois (budget mensuel en vigueur à la séance).
    pub budget: u32,
    /// Tickets déjà recommandés en vigueur ce mois avant la séance (hors grilles que la séance remplace).
    pub spent_before: u32,
    /// Tickets retenus par la séance pour ce mois.
    pub chosen: u32,
}

impl MonthBudget {
    /// Ce qui reste après la séance.
    pub fn left(&self) -> u32 {
        self.budget.saturating_sub(self.spent_before + self.chosen)
    }
}

/// Entrée en vigueur du budget (hebdomadaire le 2026-10-24, mensuel depuis le §51, appliqué tout de suite) : le
/// 2026-10-24 à minuit, heure de Paris. Les recommandations plus anciennes restent suivies (bilan, vérification) mais ne
/// comptent dans aucun budget : exception voulue par Paul pour les 16 tickets du LF8 n°146 joués le 23 octobre
/// (`docs/sources.md` §47).
pub fn budget_since() -> DateTime<Utc> {
    // Date fixe et valide : `single` ne peut renvoyer `None` (pas de changement d'heure à minuit le 24 octobre).
    Paris
        .with_ymd_and_hms(2026, 10, 24, 0, 0, 0)
        .single()
        .expect("le 24 octobre 2026 à minuit existe une seule fois à Paris")
        .with_timezone(&Utc)
}

/// Premier jour du mois (heure de Paris) où tombe un instant.
pub fn month_of(at: DateTime<Utc>) -> NaiveDate {
    let day = at.with_timezone(&Paris).date_naive();
    day.with_day(1)
        .expect("le premier jour d'un mois existe toujours")
}

/// Budget mensuel en vigueur à `at` : la dernière saisie de l'application, sinon `lotofoot_monthly_budget`.
pub fn monthly_budget(conn: &Connection, config: &Config, at: DateTime<Utc>) -> Result<u32> {
    Ok(store::budget_at(conn, at)?.unwrap_or(config.lotofoot_monthly_budget))
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
    /// Grille vue depuis moins de [`MIN_GRID_AGE_HOURS`] : évaluée, sans engager de budget.
    young: bool,
}

/// Ce que demande une séance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Request {
    /// Budget mensuel en euros (un ticket coûte 1 €).
    pub monthly_budget: u32,
    /// Seuil τ : celui de l'abaque pour ce budget ([`Request::from_budget`]) ; les tests en fixent d'autres.
    pub threshold: f64,
    /// Recalculer aussi les grilles déjà recommandées (bouton de l'application) : la nouvelle séance remplace alors leur
    /// recommandation, sauf pour une grille dont des tickets sont déjà déclarés joués. Le timer ne remplace jamais.
    pub replace: bool,
    /// Rien n'est écrit et aucune compétition n'est relevée (aucun crédit).
    pub dry_run: bool,
}

impl Request {
    /// Demande au seuil de l'abaque pour ce budget mensuel ; refusée si le budget sort de l'abaque.
    pub fn from_budget(monthly_budget: u32, replace: bool, dry_run: bool) -> Result<Request> {
        Ok(Request {
            monthly_budget,
            threshold: abaque::threshold(monthly_budget)?,
            replace,
            dry_run,
        })
    }
}

/// Lance la séance.
///
/// Une grille déjà recommandée est écartée, sauf si `request.replace` : elle est alors recalculée, et la séance la
/// remplace même si elle n'y retient aucun ticket (« remplacée par rien », noté dans les paramètres de la séance).
/// Seule la dernière recommandation de chaque grille compte ([`store::current_recos`], `docs/sources.md` §41). Une
/// séance qui ne retient aucun ticket et ne remplace rien n'est pas écrite : elle n'aurait rien à journaliser.
pub fn run(
    conn: &mut Connection,
    config: &Config,
    aliases: &Aliases,
    now: Clock,
    fetch: Fetch,
    request: Request,
) -> Result<SessionReport> {
    let dry_run = request.dry_run;
    let started = now();
    let min_close =
        started + Duration::minutes(i64::from(config.lotofoot_min_minutes_before_close));
    let fresh_after = started - Duration::hours(i64::from(config.lotofoot_max_odds_age_hours));
    let mut report = SessionReport {
        evaluated: Vec::new(),
        skipped: Vec::new(),
        unknown_names: Vec::new(),
        leagues: LeagueSearch::default(),
        chosen: Vec::new(),
        session_id: None,
        months: Vec::new(),
        monthly_budget: request.monthly_budget,
        threshold: request.threshold,
    };

    // 1. Grilles candidates, pas encore recommandées, avec répartition récente, noms connus et mises estimables.
    let recommended = store::current_recos(conn)?;
    let played: Vec<i64> = store::plays(conn)?
        .iter()
        .filter_map(|p| p.reco_id)
        .collect();
    let mut pending: Vec<Pending> = Vec::new();
    for row in store::grids_closing_after(conn, min_close)? {
        let label = format!("{} n°{}", row.game, row.number);
        let skip = |reason: String| Skipped {
            grid: label.clone(),
            reason,
        };
        // Des tickets joués portent sur l'ancienne composition : on ne recalcule pas une grille qui n'est plus la leur.
        if store::recomposition(conn, row.game, row.number)?.is_some_and(|r| r.played_before) {
            report.skipped.push(skip(
                "rencontres changées après des tickets déclarés joués : grille bloquée, à régler à la main".to_string(),
            ));
            continue;
        }
        let earlier: Vec<&RecoRow> = recommended
            .iter()
            .filter(|r| (r.game, r.number) == (row.game, row.number))
            .collect();
        if let Some(first) = earlier.first() {
            if !request.replace {
                report.skipped.push(skip(format!(
                    "déjà recommandée (séance n°{})",
                    first.session_id
                )));
                continue;
            }
            if earlier.iter().any(|r| played.contains(&r.id)) {
                report.skipped.push(skip(format!(
                    "tickets de la séance n°{} déjà déclarés joués : non recalculée",
                    first.session_id
                )));
                continue;
            }
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
        // Une part affichée à 0 % signe une grille à peine ouverte : la répartition ne repose que sur une poignée de
        // grilles, et le modèle y verrait des signes délaissés de valeur énorme. Aucune des 1 109 répartitions finales
        // LF7/LF8 de l'historique FDJ n'a de part sous 0,5 % (`docs/sources.md` §47).
        if split.iter().any(|shares| shares.contains(&0)) {
            report.skipped.push(skip(
                "répartition pas encore représentative (un signe à 0 %) : grille trop récente"
                    .to_string(),
            ));
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
        // Âge depuis la première apparition (une grille jamais inscrite, impossible ici, compte comme neuve).
        let age = store::first_seen(conn, row.game, row.number)?
            .map_or(Duration::zero(), |seen| started - seen);
        let young = age < Duration::hours(MIN_GRID_AGE_HOURS);
        if young {
            report.skipped.push(Skipped {
                grid: label,
                reason: format!(
                    "grille trop jeune (vue depuis {} h, {MIN_GRID_AGE_HOURS} h au moins) : évaluée, sans ticket",
                    age.num_hours()
                ),
            });
        }
        ready.push(Ready {
            row,
            stakes,
            split_fetch_id,
            markets,
            model,
            young,
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
        if !grid.young {
            all.extend(candidates);
        }
    }
    // Grilles recalculées qui avaient une recommandation : la séance la remplace, tickets retenus ou non (une grille trop
    // jeune garde la sienne : elle n'est pas en état d'être recalculée).
    let replaced: Vec<(Game, u32)> = ready
        .iter()
        .filter(|g| !g.young)
        .map(|g| (g.row.game, g.row.number))
        .filter(|key| recommended.iter().any(|r| (r.game, r.number) == *key))
        .collect();
    report.months = month_budgets(
        conn,
        request.monthly_budget,
        &recommended,
        &replaced,
        &ready,
    )?;
    let ranked = optimize::select(all, request.threshold, usize::MAX);
    report.chosen = within_budget(ranked, &mut report.months, &ready);

    // 4. Journal immuable : la séance, ses paramètres, et chaque ticket avec tout ce qui a servi à le choisir.
    let replaces: Vec<(&str, u32)> = replaced
        .iter()
        .map(|(game, number)| (game.code(), *number))
        .collect();
    if !dry_run && (!report.chosen.is_empty() || !replaces.is_empty()) {
        let params = json!({
            "kappa": FROZEN.kappa, "beta": FROZEN.beta, "phi": FROZEN.phi,
            "frequency_weight": config.lotofoot_frequency_weight, "threshold": request.threshold,
            "monthly_budget": request.monthly_budget, "abaque": abaque::VERSION,
            "allocation": "ordre de clôture", "min_grid_age_hours": MIN_GRID_AGE_HOURS,
            "budget_since": budget_since().to_rfc3339(), "replace": request.replace, "replaces": replaces,
            "pessimistic_shrink": PESSIMISTIC_SHRINK,
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

/// Budget de chaque mois où ferme une grille évaluée : tickets déjà recommandés en vigueur depuis [`budget_since`],
/// sans compter ceux des grilles que la séance remplace.
fn month_budgets(
    conn: &Connection,
    monthly_budget: u32,
    recommended: &[RecoRow],
    replaced: &[(Game, u32)],
    ready: &[Ready],
) -> Result<Vec<MonthBudget>> {
    let since = budget_since();
    let mut months: Vec<MonthBudget> = Vec::new();
    for grid in ready {
        let month = month_of(grid.row.closes_at);
        if months.iter().any(|m| m.month == month) {
            continue;
        }
        let mut spent = 0u32;
        for reco in recommended {
            if reco.created_at < since || replaced.contains(&(reco.game, reco.number)) {
                continue;
            }
            let closes = store::grid(conn, reco.game, reco.number)?.map(|g| g.closes_at);
            if closes.is_some_and(|at| month_of(at) == month) {
                spent += 1;
            }
        }
        months.push(MonthBudget {
            month,
            budget: monthly_budget,
            spent_before: spent,
            chosen: 0,
        });
    }
    Ok(months)
}

/// Tickets au-dessus du seuil, grille par grille dans l'ordre de clôture : chacun est pris tant que le mois de sa grille
/// (clôture lue dans `ready`) a du budget ; `months` compte les tickets retenus.
fn within_budget(
    ranked: Vec<Candidate>,
    months: &mut [MonthBudget],
    ready: &[Ready],
) -> Vec<Candidate> {
    // Ordre de clôture (règle séquentielle de l'étude du §47) : la grille qui ferme la première prend d'abord ses tickets
    // au-dessus du seuil ; le tri est stable, donc chaque grille garde l'ordre de ses tickets (espérance S3 décroissante).
    let closes = |c: &Candidate| {
        ready
            .iter()
            .find(|g| (g.row.game, g.row.number) == c.grid)
            .map(|g| g.row.closes_at)
    };
    let mut ranked = ranked;
    ranked.sort_by_key(|c| closes(c));
    let mut chosen = Vec::new();
    for candidate in ranked {
        let Some(month) = ready
            .iter()
            .find(|g| (g.row.game, g.row.number) == candidate.grid)
            .map(|g| month_of(g.row.closes_at))
        else {
            continue;
        };
        if let Some(budget) = months.iter_mut().find(|m| m.month == month)
            && budget.left() > 0
        {
            budget.chosen += 1;
            chosen.push(candidate);
        }
    }
    chosen
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

    /// Demande de séance : budget du mois, seuil, remplacement, à blanc.
    fn ask(monthly_budget: u32, threshold: f64, replace: bool, dry_run: bool) -> Request {
        Request {
            monthly_budget,
            threshold,
            replace,
            dry_run,
        }
    }

    /// Une grille LF7 synthétique de 7 rencontres « Hi – Ai », avec sa répartition, et les cotes Pinnacle archivées.
    fn setup(conn: &mut Connection, aliases_text: &mut String) {
        let teams: Vec<(String, String)> = (1..=7)
            .map(|i| (format!("H{i}"), format!("A{i}")))
            .collect();
        // Grille vue pour la première fois à 1 h 30 (8 h 30 avant la séance de 10 h), relevée de nouveau à 9 h 30.
        record_grid(
            conn,
            133,
            at("2026-10-25T16:10:00Z"),
            at("2026-10-24T01:30:00Z"),
        );
        record_grid(
            conn,
            133,
            at("2026-10-25T16:10:00Z"),
            at("2026-10-24T09:30:00Z"),
        );
        aliases_text.push_str("[LOTOFOOT]\n");
        for (h, a) in &teams {
            aliases_text.push_str(&format!("\"{h}\" = []\n\"{a}\" = []\n"));
        }
        record_odds(conn, &teams);
    }

    /// Une grille LF7 des rencontres « Hi – Ai », relevée à `fetched`.
    fn record_grid(
        conn: &mut Connection,
        number: u32,
        closes_at: DateTime<Utc>,
        fetched: DateTime<Utc>,
    ) {
        let teams: Vec<(String, String)> = (1..=7)
            .map(|i| (format!("H{i}"), format!("A{i}")))
            .collect();
        record_grid_with(conn, number, closes_at, fetched, teams);
    }

    /// Une grille LF7 de rencontres données, relevée à `fetched`, avec la répartition des tests.
    fn record_grid_with(
        conn: &mut Connection,
        number: u32,
        closes_at: DateTime<Utc>,
        fetched: DateTime<Utc>,
        teams: Vec<(String, String)>,
    ) {
        let fetch_id = record_fetch(conn, fetched, Page::Grid(Game::Lf7), 200, None).unwrap();
        let page = GridPage {
            open: Some(OpenGrid {
                game: Game::Lf7,
                accepting: true,
                number,
                closes_at,
                jackpot: Euros::new(100_000.0),
                matches: teams,
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
    }

    /// Cotes Pinnacle archivées pour les 7 rencontres, relevées la veille à midi.
    fn record_odds(conn: &mut Connection, teams: &[(String, String)]) {
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
        let config = Config::default();
        let tau_config = 1.0;
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let report = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, tau_config, false, false),
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
        let recos = store::current_recos(&conn).unwrap();
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
        let config = Config::default();
        let tau_config = 1.0;
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        // Budget de 3 € pour le mois : 3 tickets au plus.
        let first = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(3, tau_config, false, false),
        )
        .unwrap();
        assert_eq!(first.chosen.len(), 3);
        assert!(first.session_id.is_some());
        // Seconde séance sur la même grille : écartée, et rien n'est écrit (aucun ticket retenu).
        let again = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(100, tau_config, false, false),
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
        assert_eq!(store::current_recos(&conn).unwrap().len(), 3);
        assert!(
            store::last_session(&conn)
                .unwrap()
                .is_some_and(|(id, _, _)| Some(id) == first.session_id)
        );
    }

    #[test]
    fn the_button_replaces_an_unplayed_recommendation_but_never_a_played_one() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let config = Config::default();
        let tau_config = 1.0;
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let first = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, tau_config, false, false),
        )
        .unwrap();
        assert!(first.chosen.len() > 3);
        // Le bouton à 3 € : la grille est recalculée, la nouvelle séance remplace l'ancienne.
        let second = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(3, tau_config, true, false),
        )
        .unwrap();
        assert_eq!(second.chosen.len(), 3);
        let current = store::current_recos(&conn).unwrap();
        assert_eq!(current.len(), 3);
        assert!(
            current
                .iter()
                .all(|r| Some(r.session_id) == second.session_id)
        );
        // Un ticket est joué : la grille n'est plus recalculée, même par le bouton.
        let stake = crate::domain::Euros::new(1.0).unwrap();
        crate::lotofoot::played::declare(
            &conn,
            &[current[0].id],
            at("2026-10-24T11:00:00Z"),
            stake,
        )
        .unwrap();
        let third = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, tau_config, true, false),
        )
        .unwrap();
        assert!(
            third.skipped[0].reason.contains("déjà déclarés joués"),
            "{:?}",
            third.skipped
        );
        assert!(third.session_id.is_none());
        assert_eq!(store::current_recos(&conn).unwrap().len(), 3);
    }

    /// La grille 133 relevée à `fetched` avec les mêmes rencontres dans un autre ordre (la première passe en dernier) :
    /// les positions de la répartition ne désignent plus les mêmes rencontres (`docs/sources.md` §52).
    fn recompose(conn: &mut Connection, fetched: DateTime<Utc>) {
        let teams: Vec<(String, String)> = (2..=7)
            .chain(1..=1)
            .map(|i| (format!("H{i}"), format!("A{i}")))
            .collect();
        record_grid_with(conn, 133, at("2026-10-25T16:10:00Z"), fetched, teams);
    }

    #[test]
    fn a_changed_grid_is_recalculated_from_its_new_composition_or_blocked_if_played() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let config = Config::default();
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let mut clock = || at("2026-10-24T10:00:00Z");
        let first = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, 1.0, false, false),
        )
        .unwrap();
        assert!(!first.chosen.is_empty());
        // Les rencontres changent à 10 h 30 : les tickets de 10 h ne sont plus en vigueur, et la séance suivante
        // recalcule la grille sans le bouton ; son âge repart de la nouvelle composition (une demi-heure).
        recompose(&mut conn, at("2026-10-24T10:30:00Z"));
        assert!(store::current_recos(&conn).unwrap().is_empty());
        let mut later = || at("2026-10-24T11:00:00Z");
        let second = run(
            &mut conn,
            &config,
            &aliases,
            &mut later,
            &mut fetch,
            ask(16, 1.0, false, false),
        )
        .unwrap();
        assert!(
            second.skipped[0].reason.contains("trop jeune"),
            "{:?}",
            second.skipped
        );

        // Autre base : un ticket est déclaré joué avant le changement, la grille est bloquée.
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        setup(&mut conn, &mut String::new());
        run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, 1.0, false, false),
        )
        .unwrap();
        let played = store::current_recos(&conn).unwrap()[0].id;
        let stake = crate::domain::Euros::new(1.0).unwrap();
        crate::lotofoot::played::declare(&conn, &[played], at("2026-10-24T10:15:00Z"), stake)
            .unwrap();
        recompose(&mut conn, at("2026-10-24T10:30:00Z"));
        let blocked = run(
            &mut conn,
            &config,
            &aliases,
            &mut later,
            &mut fetch,
            ask(16, 1.0, true, false),
        )
        .unwrap();
        assert!(
            blocked.skipped[0].reason.contains("grille bloquée"),
            "{:?}",
            blocked.skipped
        );
        assert!(blocked.session_id.is_none());
    }

    #[test]
    fn the_monthly_budget_caps_the_session_and_a_replaced_grid_frees_its_share() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let config = Config::default();
        let tau_config = 1.0;
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        // 5 € de budget pour octobre (clôture le dimanche 25).
        let first = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(5, tau_config, false, false),
        )
        .unwrap();
        assert_eq!(first.chosen.len(), 5);
        let month = first.months[0];
        assert_eq!(month.month, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        assert_eq!((month.spent_before, month.chosen, month.left()), (0, 5, 0));
        // Recalcul par le bouton : les 5 tickets remplacés ne comptent plus, le mois a de nouveau ses 5 €.
        let again = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(5, tau_config, true, false),
        )
        .unwrap();
        assert_eq!(again.chosen.len(), 5);
        assert_eq!(again.months[0].spent_before, 0);
    }

    #[test]
    fn months_follow_paris_time_and_the_budget_starts_on_24_october() {
        // 31 octobre 23 h 30 UTC = 1er novembre 0 h 30 à Paris (heure d'hiver) : novembre.
        assert_eq!(
            month_of(at("2026-10-31T23:30:00Z")),
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );
        assert_eq!(
            month_of(at("2026-10-31T22:30:00Z")),
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        );
        // Minuit le 24 octobre à Paris (heure d'été) = 22 h UTC la veille : la séance n°6 du 23 à 13 h 16 UTC est avant.
        assert_eq!(budget_since(), at("2026-10-23T22:00:00Z"));
    }

    #[test]
    fn the_grid_closing_first_takes_the_budget_first() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        // Même grille sous un autre numéro, qui ferme plus tôt : mêmes tickets, mêmes espérances.
        record_grid(
            &mut conn,
            134,
            at("2026-10-25T12:00:00Z"),
            at("2026-10-24T01:30:00Z"),
        );
        record_grid(
            &mut conn,
            134,
            at("2026-10-25T12:00:00Z"),
            at("2026-10-24T09:30:00Z"),
        );
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let report = run(
            &mut conn,
            &Config::default(),
            &aliases,
            &mut clock,
            &mut fetch,
            ask(5, 1.0, false, false),
        )
        .unwrap();
        assert_eq!(report.chosen.len(), 5);
        assert!(
            report.chosen.iter().all(|c| c.grid == (Game::Lf7, 134)),
            "{:?}",
            report.chosen
        );
    }

    #[test]
    fn a_young_grid_is_evaluated_without_tickets() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        // Grille n°135, vue pour la première fois à 8 h : 2 h d'âge à la séance de 10 h.
        record_grid(
            &mut conn,
            135,
            at("2026-10-25T12:00:00Z"),
            at("2026-10-24T08:00:00Z"),
        );
        record_grid(
            &mut conn,
            135,
            at("2026-10-25T12:00:00Z"),
            at("2026-10-24T09:30:00Z"),
        );
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let report = run(
            &mut conn,
            &Config::default(),
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, 1.0, false, false),
        )
        .unwrap();
        // Les deux grilles sont évaluées ; la jeune, qui ferme pourtant la première, ne prend aucun ticket.
        assert_eq!(report.evaluated.len(), 2);
        assert!(!report.chosen.is_empty());
        assert!(report.chosen.iter().all(|c| c.grid == (Game::Lf7, 133)));
        assert!(
            report
                .skipped
                .iter()
                .any(|s| s.grid == "LF7 n°135" && s.reason.contains("trop jeune")),
            "{:?}",
            report.skipped
        );
    }

    #[test]
    fn a_grid_can_be_replaced_by_nothing() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let easy = Config::default();
        let tau_easy = 1.0;
        run(
            &mut conn,
            &easy,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, tau_easy, false, false),
        )
        .unwrap();
        // Recalcul où plus rien ne passe le seuil : la séance est écrite pour remplacer, sans ticket.
        let strict = Config::default();
        let tau_strict = 1e9;
        let report = run(
            &mut conn,
            &strict,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, tau_strict, true, false),
        )
        .unwrap();
        assert!(report.chosen.is_empty());
        assert!(report.session_id.is_some());
        assert!(store::current_recos(&conn).unwrap().is_empty());
    }

    #[test]
    fn a_split_with_a_sign_at_zero_is_not_representative() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let mut aliases_text = String::new();
        setup(&mut conn, &mut aliases_text);
        let aliases = Aliases::parse(&aliases_text).unwrap();
        // Relevé plus récent de la même grille, juste après son ouverture : 95 / 5 / 0 sur une rencontre.
        let fetch_id = record_fetch(
            &conn,
            at("2026-10-24T09:45:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        let fresh = GridPage {
            open: Some(OpenGrid {
                game: Game::Lf7,
                accepting: true,
                number: 133,
                closes_at: at("2026-10-25T16:10:00Z"),
                jackpot: Euros::new(100_000.0),
                matches: (1..=7)
                    .map(|i| (format!("H{i}"), format!("A{i}")))
                    .collect(),
                split: Some(vec![
                    [95, 5, 0],
                    [31, 24, 45],
                    [70, 18, 12],
                    [45, 26, 29],
                    [35, 25, 40],
                    [47, 25, 28],
                    [55, 25, 20],
                ]),
                split_at: None,
            }),
            history: vec![],
        };
        record_grid_page(&mut conn, fetch_id, Game::Lf7, &fresh).unwrap();
        let config = Config::default();
        let tau_config = 1.0;
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let report = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, tau_config, false, false),
        )
        .unwrap();
        assert!(report.chosen.is_empty());
        assert!(
            report.skipped[0]
                .reason
                .contains("pas encore représentative"),
            "{:?}",
            report.skipped
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
        let config = Config::default();
        let tau_config = 1e9;
        let mut clock = || at("2026-10-24T10:00:00Z");
        let mut fetch = |path: &str, _: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let report = run(
            &mut conn,
            &config,
            &aliases,
            &mut clock,
            &mut fetch,
            ask(16, tau_config, false, false),
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
        let report = run(
            &mut conn,
            &config,
            &empty,
            &mut clock,
            &mut fetch,
            ask(16, 1.7, false, true),
        )
        .unwrap();
        assert_eq!(report.unknown_names.len(), 14);
        assert!(report.session_id.is_none());

        // Séance à 14 h : la répartition de 9 h 30 a plus de 3 heures.
        let aliases = Aliases::parse(&aliases_text).unwrap();
        let mut late = || at("2026-10-24T14:00:00Z");
        let report = run(
            &mut conn,
            &config,
            &aliases,
            &mut late,
            &mut fetch,
            ask(16, 1.7, false, true),
        )
        .unwrap();
        assert!(report.skipped[0].reason.contains("plus de 3 h"));
    }
}
