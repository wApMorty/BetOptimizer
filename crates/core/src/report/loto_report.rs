//! Rapport `edge loto study` : espérance réalisée des Grand Loto passés, préférences du public validées hors échantillon,
//! tirages EuroMillions au plafond, séances et grilles jouées (phase 16). Lecture seule.

use chrono::NaiveDate;
use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;
use crate::eval::bootstrap::MeanCi;
use crate::loto::alert::{self, CapCheck};
use crate::loto::estimate::{self, Season};
use crate::loto::expectation::{self, Edition};
use crate::loto::played;
use crate::loto::popularity::{self, Preferences, Slope, Verdict, YearResult};
use crate::loto::{Game, Grid, rules};
use crate::store::loto as store;

/// Nombre de grilles et jackpot de référence pour traduire les préférences en euros : un Noël à 20 M€ et 7 millions de
/// grilles (la médiane des Noëls 2017 à 2025).
const REFERENCE_GRIDS: f64 = 7.0e6;
const REFERENCE_JACKPOT: f64 = 20.0e6;

/// Une édition passée.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EditionRow {
    /// Date (JJ/MM/AAAA).
    pub date: String,
    /// Saison.
    pub season: &'static str,
    /// Jackpot, en millions d'euros.
    pub jackpot: f64,
    /// Jackpot gagné au rang 1.
    pub won: bool,
    /// Grilles estimées, en millions, et fourchette par un seul rang.
    pub grids: f64,
    /// Borne basse.
    pub grids_low: f64,
    /// Borne haute.
    pub grids_high: f64,
    /// Reversement des rangs inférieurs par grille, en euros.
    pub lower: f64,
    /// Point mort, en millions de grilles.
    pub break_even: f64,
    /// Espérance réalisée d'une grille (euros rendus par euro − 1) et sa fourchette.
    pub expectation: f64,
    /// Borne basse.
    pub expectation_low: f64,
    /// Borne haute.
    pub expectation_high: f64,
}

/// Un numéro et son poids.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Weight {
    /// Numéro.
    pub number: u8,
    /// Poids relatif (1 : moyen) ou part des joueurs (chance).
    pub weight: f64,
}

/// Validation des préférences.
#[derive(Debug, Clone, Serialize)]
pub struct PreferencesView {
    /// Tirages du Loto à neuf rangs en base.
    pub draws: usize,
    /// Une ligne par année de test.
    pub years: Vec<YearResult>,
    /// Gain de log-vraisemblance par tirage.
    pub gain: MeanCi,
    /// Pente de calibration du rang 8.
    pub slope: Option<Slope>,
    /// Pentes de contrôle par nombre de numéros trouvés.
    pub level_slopes: Vec<(usize, Option<Slope>)>,
    /// Critère rempli.
    pub validated: bool,
    /// Numéros les plus joués (poids décroissants).
    pub popular: Vec<Weight>,
    /// Numéros les plus délaissés (poids croissants).
    pub unpopular: Vec<Weight>,
    /// Parts des numéros chance.
    pub chances: Vec<Weight>,
    /// Espérance d'une grille au hasard à la référence, en euros.
    pub uniform_value: f64,
    /// Meilleure grille à la référence et son espérance, si les préférences sont prises au mot.
    pub best_grid: Option<String>,
    /// Son espérance, en euros.
    pub best_value: Option<f64>,
    /// Contrôle sur les Grand Loto : gain de log-vraisemblance de chaque édition, préférences ajustées sur les Loto
    /// antérieurs.
    pub grand_loto_gains: Vec<(String, f64)>,
}

/// Un tirage EuroMillions au plafond.
#[derive(Debug, Clone, Serialize)]
pub struct CapRow {
    /// Date.
    pub date: String,
    /// Grilles en Europe, en millions.
    pub grids: f64,
    /// Excédent versé au rang inférieur, en millions d'euros.
    pub excess: f64,
    /// Jackpot gagné.
    pub won: bool,
}

/// Une séance.
#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    /// Date de l'édition.
    pub date: String,
    /// Grilles recommandées (`n° grille`).
    pub grids: Vec<String>,
}

/// Une grille jouée.
#[derive(Debug, Clone, Serialize)]
pub struct PlayView {
    /// Date de l'édition.
    pub date: String,
    /// Grille.
    pub grid: String,
    /// Mise.
    pub stake: f64,
    /// Gain, ou `None` si le tirage n'est pas encore dans l'archive.
    pub gain: Option<f64>,
    /// Hors modèle.
    pub off_model: bool,
}

/// Le rapport.
#[derive(Debug, Clone, Serialize)]
pub struct LotoReport {
    /// Date de génération.
    pub generated_on: String,
    /// Grand Loto passés.
    pub editions: Vec<EditionRow>,
    /// Reversement des rangs inférieurs selon le règlement, en euros par grille de 5 €.
    pub theory_lower: f64,
    /// Préférences (absent si la base n'a pas de Loto).
    pub preferences: Option<PreferencesView>,
    /// Tirages EuroMillions au plafond depuis février 2020.
    pub caps: Vec<CapRow>,
    /// Alerte due aujourd'hui, en clair.
    pub alert: Option<String>,
    /// Alertes envoyées (date du tirage déclencheur, niveau).
    pub alerts: Vec<(String, String)>,
    /// Séances.
    pub sessions: Vec<SessionView>,
    /// Grilles jouées.
    pub plays: Vec<PlayView>,
    /// Totaux des grilles recommandées jouées : mises, gains, grilles réglées.
    pub totals: (f64, f64, usize),
    /// Totaux des grilles hors modèle.
    pub off_model_totals: (f64, f64, usize),
}

fn date(d: NaiveDate) -> String {
    d.format("%d/%m/%Y").to_string()
}

/// Construit le rapport.
pub fn build(conn: &Connection, today: NaiveDate) -> Result<LotoReport> {
    let price = rules::GRAND_LOTO_PRICE;
    let grand = store::draws(conn, Game::GrandLoto)?;
    let editions = edition_rows(&grand);

    let loto = store::draws(conn, Game::Loto)?;
    let preferences = if loto.is_empty() {
        None
    } else {
        Some(preferences_view(&loto, &grand)?)
    };

    let euro = store::draws(conn, Game::EuroMillions)?;
    let caps = euro
        .iter()
        .filter(|d| d.date >= alert::rules_since())
        .filter_map(alert::check)
        .filter(CapCheck::at_cap)
        .map(|c| CapRow {
            date: date(c.date),
            grids: c.grids / 1e6,
            excess: c.excess / 1e6,
            won: c.jackpot_won,
        })
        .collect();
    let alert = alert::pending(&euro)
        .map(|a| a.map(|a| a.message))
        .unwrap_or_else(|e| Some(e.to_string()));
    let alerts = store::alerts(conn)?
        .into_iter()
        .map(|(_, d, level)| (date(d), level))
        .collect();

    let recos = store::recos(conn)?;
    let sessions = store::sessions(conn)?
        .into_iter()
        .map(|s| SessionView {
            date: date(s.draw_date),
            grids: recos
                .iter()
                .filter(|r| r.session_id == s.id)
                .map(|r| format!("n° {} : {}", r.id, r.grid))
                .collect(),
        })
        .collect();
    let ledger = played::ledger(conn)?;
    let plays = ledger
        .rows
        .iter()
        .map(|r| PlayView {
            date: date(r.play.draw_date),
            grid: r.play.grid.to_string(),
            stake: r.play.stake,
            gain: r.gain,
            off_model: r.play.reco_id.is_none(),
        })
        .collect();

    Ok(LotoReport {
        generated_on: date(today),
        editions,
        theory_lower: price
            * (rules::LOTO_WINNERS_SHARE
                - rules::GRAND_LOTO_JACKPOT_SHARE
                - rules::GRAND_LOTO_SUPER_FUND_SHARE),
        preferences,
        caps,
        alert,
        alerts,
        sessions,
        plays,
        totals: ledger.totals(false),
        off_model_totals: ledger.totals(true),
    })
}

/// Une ligne par Grand Loto passé de l'archive : grilles, reversement, point mort, espérance réalisée. Partagé avec la
/// page `/loto` de l'application.
pub fn edition_rows(grand: &[crate::loto::Draw]) -> Vec<EditionRow> {
    let price = rules::GRAND_LOTO_PRICE;
    grand
        .iter()
        .filter_map(estimate::edition_outcome)
        .map(|o| {
            let (low, high) = o.return_range();
            EditionRow {
                date: date(o.date),
                season: Season::of(o.date).label(),
                jackpot: o.jackpot / 1e6,
                won: o.jackpot_won,
                grids: o.grids.n / 1e6,
                grids_low: o.grids.low / 1e6,
                grids_high: o.grids.high / 1e6,
                lower: o.lower_ranks_per_grid(),
                break_even: o.break_even_grids(price) / 1e6,
                expectation: o.return_per_grid() / price - 1.0,
                expectation_low: low / price - 1.0,
                expectation_high: high / price - 1.0,
            }
        })
        .collect()
}

fn preferences_view(
    loto: &[crate::loto::Draw],
    grand: &[crate::loto::Draw],
) -> Result<PreferencesView> {
    let walk = popularity::walk_forward(loto)?;
    let prefs = popularity::fit(loto)?;
    let mut numbers: Vec<Weight> = prefs
        .balls
        .iter()
        .enumerate()
        .map(|(i, &w)| Weight {
            number: u8::try_from(i + 1).unwrap_or(0),
            weight: w,
        })
        .collect();
    numbers.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    let popular = numbers.iter().take(10).copied().collect();
    let unpopular = numbers.iter().rev().take(10).copied().collect();
    let chances = prefs
        .chances
        .iter()
        .enumerate()
        .map(|(i, &w)| Weight {
            number: u8::try_from(i + 1).unwrap_or(0),
            weight: w,
        })
        .collect();
    let reference = Edition {
        jackpot: REFERENCE_JACKPOT,
        price: rules::GRAND_LOTO_PRICE,
        codes: 100,
        code_gain: 20_000.0,
    };
    let any = Grid::new([1, 2, 3, 4, 5], 1).expect("grille valide écrite en dur");
    let uniform_value =
        expectation::grid_value(&Preferences::uniform(), &any, &reference, REFERENCE_GRIDS);
    let best = expectation::best_grids(&prefs, &reference, REFERENCE_GRIDS, 1);
    let grand_loto_gains = grand
        .iter()
        .filter_map(|g| {
            let before: Vec<crate::loto::Draw> =
                loto.iter().filter(|d| d.date < g.date).cloned().collect();
            if before.is_empty() {
                return None;
            }
            let fitted = popularity::fit(&before).ok()?;
            let gain = popularity::log_likelihood(&fitted, g)?
                - popularity::log_likelihood(&Preferences::uniform(), g)?;
            Some((date(g.date), gain))
        })
        .collect();
    Ok(PreferencesView {
        draws: loto.len(),
        years: walk.years,
        gain: walk.gain,
        slope: walk.slope,
        level_slopes: walk.level_slopes,
        validated: walk.verdict == Verdict::Validated,
        popular,
        unpopular,
        chances,
        uniform_value,
        best_grid: best.first().map(|(g, _)| g.to_string()),
        best_value: best.first().map(|(_, v)| *v),
        grand_loto_gains,
    })
}
