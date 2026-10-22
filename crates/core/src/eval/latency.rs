//! Test prospectif de la latence des opérateurs français contre Pinnacle (phase 13, `docs/sources.md` §34).
//!
//! Question : dans la fenêtre des compositions (60 à 15 minutes avant le coup d'envoi), Winamax ou Unibet affichent-ils
//! encore des cotes que Pinnacle a déjà corrigées ? Mesure : la CLV des paris qu'une règle **figée** aurait pris.
//! La règle a été fixée le 2026-10-22, avant tout match du test ; ses constantes ne se modifient pas.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use rusqlite::Connection;

use super::bootstrap::{MeanCi, block_bootstrap_mean};
use crate::domain::{Odds, Odds1x2, Outcome, Probability};
use crate::error::Result;
use crate::ingest::aliases::Aliases;
use crate::ingest::football_data::LEAGUES as FD_LEAGUES;
use crate::ingest::oddspapi::league_div;
use crate::market::margin::{MarginMethod, remove_margin};
use crate::store::oddspapi::{self as store, ArchiveQuote, StoredFixture, StoredPrice};

/// Bookmaker de référence (prix « vrai » une fois la marge retirée).
pub const REFERENCE: &str = "pinnacle";
/// Opérateurs français testés (Betclic est absent d'OddsPapi).
pub const FRENCH_BOOKS: [&str; 2] = ["winamax.fr", "unibet.fr"];
/// Avantage minimal pour qu'un pari soit pris : seuil des recommandations en production (2026-10-20).
pub const THRESHOLD: f64 = 0.01;
/// Horizons de décision de la fenêtre, en minutes avant le coup d'envoi, du plus lointain au plus proche.
pub const WINDOW_MINUTES: [i64; 4] = [60, 45, 30, 15];
/// Filtre de qualité : changements de prix relevés chez l'opérateur dans les 24 h avant le match. En dessous, OddsPapi ne
/// suivait pas l'opérateur et ses cotes « figées » créeraient une fausse valeur (artefact d'avril à septembre 2026, §34).
pub const MIN_CHANGES: usize = 10;
/// Filtre plus strict, en lecture secondaire seulement (sans verdict).
pub const SECONDARY_MIN_CHANGES: usize = 20;
/// Paris nécessaires au verdict : puissance de 80 % pour une CLV de 2 % avec σ = 4,5 % (test unilatéral à 5 %), soit
/// n = ((z₀,₉₅ + z₀,₈₀) · σ / μ)² = (2,49 × 0,045 / 0,02)² ≈ 32.
pub const MIN_BETS: usize = 32;
/// CLV moyenne minimale d'un succès.
pub const MIN_CLV: f64 = 0.01;
/// Rééchantillonnages du bootstrap et graine (reproductible : invariant 9).
const RESAMPLES: usize = 2000;
const SEED: u64 = 7;

/// Début du test : seuls les matchs dont le coup d'envoi est postérieur ou égal comptent.
pub fn test_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 22, 0, 0, 0)
        .single()
        .expect("le 2026-10-22 à minuit UTC est une date valide et non ambiguë")
}

/// Échéance : moins de [`MIN_BETS`] paris à cette date, le signal est jugé trop rare pour être utile.
pub fn deadline() -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, 1, 18).expect("le 2027-01-18 est une date valide")
}

/// Un changement de prix d'une issue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    /// Instant du changement.
    pub at: DateTime<Utc>,
    /// Cote (absente pour une suspension sans prix).
    pub price: Option<Odds>,
    /// `false` : issue suspendue à partir de cet instant.
    pub active: bool,
}

/// Historique du 1X2 d'un bookmaker sur un match : une série triée par instant pour chaque issue.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Book1x2 {
    /// Issue 101 d'OddsPapi.
    pub home: Vec<Entry>,
    /// Issue 102.
    pub draw: Vec<Entry>,
    /// Issue 103.
    pub away: Vec<Entry>,
}

impl Book1x2 {
    fn series(&self, outcome: Outcome) -> &[Entry] {
        match outcome {
            Outcome::Home => &self.home,
            Outcome::Draw => &self.draw,
            Outcome::Away => &self.away,
        }
    }
}

/// Un match et l'historique de chaque bookmaker.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchHistory {
    /// Identifiant OddsPapi.
    pub fixture_id: String,
    /// Championnat OddsPapi.
    pub tournament_id: u32,
    /// Coup d'envoi prévu.
    pub kickoff: DateTime<Utc>,
    /// Noms bruts d'OddsPapi.
    pub home: String,
    /// Noms bruts d'OddsPapi.
    pub away: String,
    /// Historique par bookmaker. `BTreeMap` : ordre de parcours fixe, donc résultats reproductibles.
    pub books: BTreeMap<String, Book1x2>,
}

/// Regroupe les lignes de la base par match et par bookmaker. Les prix doivent être triés par instant (c'est l'ordre de
/// la requête) ; une issue inconnue (autre que 101, 102, 103) est ignorée.
pub fn build_histories(
    fixtures: Vec<StoredFixture>,
    prices: Vec<StoredPrice>,
) -> Vec<MatchHistory> {
    // `into_iter` : on consomme les lignes lues (elles ne servent plus ensuite), ce qui évite toute copie des noms.
    let mut by_id: BTreeMap<String, MatchHistory> = fixtures
        .into_iter()
        .map(|f| {
            let history = MatchHistory {
                fixture_id: f.fixture_id.clone(),
                tournament_id: f.tournament_id,
                kickoff: f.kickoff,
                home: f.home_raw,
                away: f.away_raw,
                books: BTreeMap::new(),
            };
            (f.fixture_id, history)
        })
        .collect();
    for price in prices {
        let Some(history) = by_id.get_mut(&price.fixture_id) else {
            continue;
        };
        let book = history.books.entry(price.bookmaker).or_default();
        let series = match price.outcome_id {
            101 => &mut book.home,
            102 => &mut book.draw,
            103 => &mut book.away,
            _ => continue,
        };
        series.push(Entry {
            at: price.changed_at,
            price: price.price,
            active: price.active,
        });
    }
    by_id.into_values().collect()
}

/// Cote valable à l'instant `t` : celle de la **dernière entrée antérieure ou égale à `t`**. Aucune entrée postérieure
/// n'est lue : c'est ce qui garantit qu'aucune décision n'utilise le futur (invariant 1). `None` si aucune entrée, ou si
/// la dernière est une suspension.
pub fn price_at(series: &[Entry], t: DateTime<Utc>) -> Option<Odds> {
    // `partition_point` : nombre d'entrées dont l'instant est ≤ t (la série est triée), par recherche dichotomique.
    let count = series.partition_point(|entry| entry.at <= t);
    let last = series.get(count.checked_sub(1)?)?;
    if last.active { last.price } else { None }
}

/// Les trois cotes d'un bookmaker à l'instant `t`, si les trois issues sont disponibles.
pub fn odds_at(book: &Book1x2, t: DateTime<Utc>) -> Option<Odds1x2> {
    Some(Odds1x2 {
        home: price_at(&book.home, t)?,
        draw: price_at(&book.draw, t)?,
        away: price_at(&book.away, t)?,
    })
}

/// Changements de prix relevés dans les 24 h qui précèdent le coup d'envoi, toutes issues confondues.
pub fn changes_last_day(book: &Book1x2, kickoff: DateTime<Utc>) -> usize {
    let from = kickoff - Duration::hours(24);
    Outcome::ALL
        .iter()
        .map(|&o| {
            book.series(o)
                .iter()
                .filter(|e| e.at >= from && e.at < kickoff)
                .count()
        })
        .sum()
}

/// Un pari que la règle aurait pris.
#[derive(Debug, Clone, PartialEq)]
pub struct Bet {
    /// Match.
    pub fixture_id: String,
    /// Championnat OddsPapi.
    pub tournament_id: u32,
    /// Coup d'envoi.
    pub kickoff: DateTime<Utc>,
    /// Noms bruts.
    pub home: String,
    /// Noms bruts.
    pub away: String,
    /// Opérateur.
    pub bookmaker: String,
    /// Horizon de décision, en minutes avant le coup d'envoi.
    pub horizon_minutes: i64,
    /// Issue jouée.
    pub outcome: Outcome,
    /// Cote de l'opérateur à l'instant de décision.
    pub odds: Odds,
    /// Probabilité de Pinnacle sans marge au même instant.
    pub probability: Probability,
    /// Avantage annoncé `o · p − 1`.
    pub edge: f64,
    /// CLV `o · p_clôture − 1`.
    pub clv: f64,
    /// Bloc du bootstrap : championnat × semaine ISO.
    pub block: u32,
}

/// Probabilités sans marge de Pinnacle à l'instant `t` (méthode puissance, celle de la production).
fn fair_probabilities(book: &Book1x2, t: DateTime<Utc>) -> Option<[f64; 3]> {
    remove_margin(&odds_at(book, t)?, MarginMethod::Power).map(|p| p.to_array())
}

/// Bloc d'un match : championnat × semaine ISO (les matchs d'une même journée ne sont pas indépendants).
fn block(tournament_id: u32, kickoff: DateTime<Utc>) -> u32 {
    let week = kickoff.iso_week();
    // 36 championnats au plus × 10⁶ + année × 100 + semaine : tient largement dans un u32 (< 4,3 × 10⁹).
    let year = u32::try_from(week.year()).unwrap_or(0);
    tournament_id * 1_000_000 + year * 100 + week.week()
}

/// Paris de la règle sur un match : pour chaque opérateur suivi (au moins `min_changes` changements dans les 24 h), le
/// **premier** horizon de la fenêtre où l'issue de plus grand avantage dépasse [`THRESHOLD`]. Un pari au plus par match
/// et par opérateur.
///
/// Formules (calcul élémentaire de l'espérance d'un pari de mise 1) :
/// - avantage annoncé `o · p_t − 1`, où `p_t` est la probabilité de Pinnacle sans marge à l'instant de décision ;
/// - CLV `o · p_c − 1`, où `p_c` est celle du dernier prix de Pinnacle avant le coup d'envoi (Buchdahl, « Squares &
///   Sharps », 2016 : la clôture d'un bookmaker sharp est le meilleur estimateur disponible de la probabilité).
pub fn window_bets(history: &MatchHistory, min_changes: usize) -> Vec<Bet> {
    let Some(reference) = history.books.get(REFERENCE) else {
        return Vec::new();
    };
    let Some(close) = fair_probabilities(reference, history.kickoff - Duration::seconds(1)) else {
        return Vec::new();
    };
    let mut bets = Vec::new();
    for bookmaker in FRENCH_BOOKS {
        let Some(book) = history.books.get(bookmaker) else {
            continue;
        };
        if changes_last_day(book, history.kickoff) < min_changes {
            continue;
        }
        for minutes in WINDOW_MINUTES {
            let t = history.kickoff - Duration::minutes(minutes);
            let (Some(fair), Some(odds)) = (fair_probabilities(reference, t), odds_at(book, t))
            else {
                continue;
            };
            // Issue de plus grand avantage ; à égalité, la première dans l'ordre domicile, nul, extérieur.
            let mut best: Option<(Outcome, f64)> = None;
            for outcome in Outcome::ALL {
                let edge = odds.get(outcome).get() * fair[outcome.index()] - 1.0;
                if best.is_none_or(|(_, b)| edge > b) {
                    best = Some((outcome, edge));
                }
            }
            let Some((outcome, edge)) = best else {
                continue;
            };
            if edge < THRESHOLD {
                continue;
            }
            let price = odds.get(outcome);
            let Some(probability) = Probability::new(fair[outcome.index()]) else {
                continue;
            };
            bets.push(Bet {
                fixture_id: history.fixture_id.clone(),
                tournament_id: history.tournament_id,
                kickoff: history.kickoff,
                home: history.home.clone(),
                away: history.away.clone(),
                bookmaker: bookmaker.to_string(),
                horizon_minutes: minutes,
                outcome,
                odds: price,
                probability,
                edge,
                clv: price.get() * close[outcome.index()] - 1.0,
                block: block(history.tournament_id, history.kickoff),
            });
            break;
        }
    }
    bets
}

/// Verdict du test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Moins de [`MIN_BETS`] paris avant l'échéance : on attend.
    Pending,
    /// CLV moyenne ≥ [`MIN_CLV`] et borne basse de l'intervalle à 95 % > 0.
    Success,
    /// Assez de paris, mais la CLV ne remplit pas le critère.
    Failure,
    /// Échéance atteinte avec moins de [`MIN_BETS`] paris : signal trop rare pour être utile.
    TooRare,
}

/// Résultat de la règle sur tous les matchs du test.
#[derive(Debug, Clone, PartialEq)]
pub struct Assessment {
    /// Filtre de suivi appliqué.
    pub min_changes: usize,
    /// Matchs du test (avec historique).
    pub matches: usize,
    /// Couples match × opérateur qui passent le filtre de suivi.
    pub tracked: usize,
    /// Paris, du plus ancien au plus récent.
    pub bets: Vec<Bet>,
    /// CLV moyenne et intervalle à 95 % (bootstrap par blocs), `None` sans pari.
    pub clv: Option<MeanCi>,
    /// Verdict (n'a de sens que pour le filtre principal [`MIN_CHANGES`]).
    pub verdict: Verdict,
}

/// Verdict à la date `today` pour des paris triés du plus ancien au plus récent.
///
/// Seuls les [`MIN_BETS`] premiers paris sont jugés : le verdict est rendu une seule fois, et les paris suivants ne
/// peuvent plus le changer.
pub fn verdict(bets: &[Bet], today: NaiveDate) -> Verdict {
    if bets.len() < MIN_BETS {
        return if today >= deadline() {
            Verdict::TooRare
        } else {
            Verdict::Pending
        };
    }
    let judged: Vec<(u32, f64)> = bets[..MIN_BETS].iter().map(|b| (b.block, b.clv)).collect();
    match block_bootstrap_mean(&judged, RESAMPLES, SEED) {
        Some(ci) if ci.mean >= MIN_CLV && ci.low > 0.0 => Verdict::Success,
        _ => Verdict::Failure,
    }
}

/// Applique la règle aux matchs du test (coup d'envoi ≥ [`test_start`]) et rend le verdict à la date `today`.
pub fn assess(histories: &[MatchHistory], min_changes: usize, today: NaiveDate) -> Assessment {
    let in_test: Vec<&MatchHistory> = histories
        .iter()
        .filter(|h| h.kickoff >= test_start())
        .collect();
    let tracked = in_test
        .iter()
        .map(|h| {
            FRENCH_BOOKS
                .iter()
                .filter(|b| {
                    h.books
                        .get(**b)
                        .is_some_and(|book| changes_last_day(book, h.kickoff) >= min_changes)
                })
                .count()
        })
        .sum();
    let mut bets: Vec<Bet> = in_test
        .iter()
        .flat_map(|h| window_bets(h, min_changes))
        .collect();
    bets.sort_by(|a, b| {
        (a.kickoff, &a.fixture_id, &a.bookmaker).cmp(&(b.kickoff, &b.fixture_id, &b.bookmaker))
    });
    let verdict = verdict(&bets, today);
    // L'intervalle affiché porte sur tous les paris ; tant que le verdict n'est pas rendu, c'est le même échantillon.
    let all: Vec<(u32, f64)> = bets.iter().map(|b| (b.block, b.clv)).collect();
    Assessment {
        min_changes,
        matches: in_test.len(),
        tracked,
        clv: block_bootstrap_mean(&all, RESAMPLES, SEED),
        bets,
        verdict,
    }
}

/// Contrôle de qualité : les cotes d'OddsPapi concordent-elles avec notre archive The Odds API aux mêmes instants ?
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QualityCheck {
    /// Cotes comparées (un relevé de notre archive, une issue, un opérateur).
    pub compared: usize,
    /// Cotes identiques.
    pub identical: usize,
    /// Noms absents de la table d'alias, au format `ligue: nom` : ces matchs sont exclus du contrôle, pas du test.
    pub unknown_names: BTreeSet<String>,
}

/// Compare, pour chaque cote Winamax et Unibet de notre archive, la cote d'OddsPapi valable au même instant. Les matchs
/// sont appariés par ligue, coup d'envoi et noms canoniques (table d'alias) : jamais par ressemblance de noms.
pub fn quality_check(
    histories: &[MatchHistory],
    quotes: &[ArchiveQuote],
    aliases: &Aliases,
) -> QualityCheck {
    let mut check = QualityCheck::default();
    let mut index: BTreeMap<(&str, DateTime<Utc>, &str, &str), &MatchHistory> = BTreeMap::new();
    for history in histories {
        let Some(div) = league_div(history.tournament_id) else {
            continue;
        };
        match (
            aliases.resolve(div, &history.home),
            aliases.resolve(div, &history.away),
        ) {
            (Some(home), Some(away)) => {
                index.insert((div, history.kickoff, home, away), history);
            }
            (home, away) => {
                for (resolved, raw) in [(home, &history.home), (away, &history.away)] {
                    if resolved.is_none() {
                        check.unknown_names.insert(format!("{div}: {raw}"));
                    }
                }
            }
        }
    }
    for quote in quotes {
        let Some(div) = FD_LEAGUES
            .iter()
            .find(|l| l.sport_key == quote.sport_key)
            .map(|l| l.div)
        else {
            continue;
        };
        let (Some(home), Some(away)) = (
            aliases.resolve(div, &quote.home_raw),
            aliases.resolve(div, &quote.away_raw),
        ) else {
            continue;
        };
        let Some(history) = index.get(&(div, quote.kickoff, home, away)) else {
            continue;
        };
        let outcome = if quote.outcome_raw == quote.home_raw {
            Outcome::Home
        } else if quote.outcome_raw == quote.away_raw {
            Outcome::Away
        } else if quote.outcome_raw == "Draw" {
            Outcome::Draw
        } else {
            continue;
        };
        // Clés de The Odds API (`winamax_fr`) et d'OddsPapi (`winamax.fr`).
        let Some(book) = history
            .books
            .get(&quote.bookmaker_key.replace("_fr", ".fr"))
        else {
            continue;
        };
        let Some(price) = price_at(book.series(outcome), quote.called_at) else {
            continue;
        };
        check.compared += 1;
        if (price.get() - quote.price).abs() < 1e-9 {
            check.identical += 1;
        }
    }
    check
}

/// Ce qu'affichent `edge latency report` et la page `/latence` du site : la règle (filtre principal, avec verdict), la
/// lecture secondaire (filtre plus strict, sans verdict) et le contrôle de qualité.
#[derive(Debug, Clone, PartialEq)]
pub struct LatencyReport {
    /// Règle figée, filtre de [`MIN_CHANGES`] changements.
    pub primary: Assessment,
    /// Même règle, filtre de [`SECONDARY_MIN_CHANGES`] changements : descriptif.
    pub secondary: Assessment,
    /// Concordance avec notre archive The Odds API.
    pub quality: QualityCheck,
}

/// Lit l'archive OddsPapi et la nôtre, puis applique la règle à la date `today` (lecture seule).
pub fn evaluate(conn: &Connection, aliases: &Aliases, today: NaiveDate) -> Result<LatencyReport> {
    let since = test_start();
    let histories = build_histories(
        store::fixtures_since(conn, since)?,
        store::prices_1x2_since(conn, since)?,
    );
    let quotes = store::archive_quotes_since(conn, since, &["winamax_fr", "unibet_fr"])?;
    Ok(LatencyReport {
        primary: assess(&histories, MIN_CHANGES, today),
        secondary: assess(&histories, SECONDARY_MIN_CHANGES, today),
        quality: quality_check(&histories, &quotes, aliases),
    })
}

/// Nom lisible d'un championnat OddsPapi (`Ligue 1`), ou son numéro s'il n'est pas suivi.
pub fn league_label(tournament_id: u32) -> String {
    league_div(tournament_id)
        .and_then(|div| FD_LEAGUES.iter().find(|l| l.div == div))
        .map_or_else(|| tournament_id.to_string(), |l| l.name.to_string())
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().to_utc()
    }

    fn entry(t: &str, price: f64) -> Entry {
        Entry {
            at: at(t),
            price: Odds::new(price),
            active: true,
        }
    }

    /// Un livre dont les trois issues ont un seul prix, publié à l'instant `t`.
    fn book(t: &str, home: f64, draw: f64, away: f64) -> Book1x2 {
        Book1x2 {
            home: vec![entry(t, home)],
            draw: vec![entry(t, draw)],
            away: vec![entry(t, away)],
        }
    }

    /// Ajoute `n` changements de prix (identiques) dans les 24 h avant 15 h, pour passer le filtre de suivi.
    fn tracked(mut b: Book1x2, n: usize) -> Book1x2 {
        for i in 0..n {
            let t = at("2026-10-25T03:00:00Z") + Duration::minutes(i64::try_from(i).unwrap());
            let price = b.draw[0].price.unwrap().get();
            b.draw.insert(
                0,
                Entry {
                    at: t,
                    price: Odds::new(price),
                    active: true,
                },
            );
        }
        b.draw.sort_by_key(|e| e.at);
        b
    }

    fn history(books: Vec<(&str, Book1x2)>) -> MatchHistory {
        MatchHistory {
            fixture_id: "id1".into(),
            tournament_id: 34,
            kickoff: at("2026-10-25T15:00:00Z"),
            home: "Angers SCO".into(),
            away: "Olympique Marseille".into(),
            books: books.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }
    }

    #[test]
    fn the_price_at_t_ignores_later_entries_and_suspensions() {
        let mut series = vec![
            entry("2026-10-25T13:00:00Z", 2.0),
            entry("2026-10-25T14:00:00Z", 2.2),
        ];
        assert_eq!(price_at(&series, at("2026-10-25T12:59:59Z")), None);
        assert_eq!(
            price_at(&series, at("2026-10-25T13:30:00Z")),
            Odds::new(2.0)
        );
        // Une entrée exactement à `t` compte.
        assert_eq!(
            price_at(&series, at("2026-10-25T14:00:00Z")),
            Odds::new(2.2)
        );
        series.push(Entry {
            at: at("2026-10-25T14:10:00Z"),
            price: Odds::new(2.2),
            active: false,
        });
        assert_eq!(price_at(&series, at("2026-10-25T14:20:00Z")), None);
    }

    #[test]
    fn fair_probabilities_sum_to_one() {
        let p = fair_probabilities(
            &book("2026-10-24T10:00:00Z", 2.10, 3.40, 3.80),
            at("2026-10-25T11:00:00Z"),
        )
        .unwrap();
        assert_relative_eq!(p.iter().sum::<f64>(), 1.0, epsilon = 1e-9);
    }

    #[test]
    fn a_stale_french_price_is_bet_once_at_the_first_qualifying_horizon() {
        // Pinnacle à 2,00 / 3,60 / 4,00 : le domicile vaut environ 0,49. Winamax garde 2,30 sur le domicile.
        let pinnacle = book("2026-10-24T10:00:00Z", 2.00, 3.60, 4.00);
        let winamax = tracked(book("2026-10-24T10:00:00Z", 2.30, 3.20, 3.40), MIN_CHANGES);
        let bets = window_bets(
            &history(vec![
                ("pinnacle", pinnacle.clone()),
                ("winamax.fr", winamax),
            ]),
            MIN_CHANGES,
        );
        assert_eq!(bets.len(), 1);
        let bet = &bets[0];
        assert_eq!((bet.outcome, bet.horizon_minutes), (Outcome::Home, 60));
        let fair = fair_probabilities(&pinnacle, at("2026-10-25T14:00:00Z")).unwrap();
        assert_relative_eq!(bet.edge, 2.30 * fair[0] - 1.0, epsilon = 1e-12);
        // Pinnacle n'a pas bougé : la CLV vaut l'avantage annoncé.
        assert_relative_eq!(bet.clv, bet.edge, epsilon = 1e-12);
    }

    #[test]
    fn an_operator_that_is_not_tracked_is_skipped() {
        let pinnacle = book("2026-10-24T10:00:00Z", 2.00, 3.60, 4.00);
        let winamax = tracked(
            book("2026-10-24T10:00:00Z", 2.30, 3.20, 3.40),
            MIN_CHANGES - 1,
        );
        assert!(
            window_bets(
                &history(vec![("pinnacle", pinnacle), ("winamax.fr", winamax)]),
                MIN_CHANGES
            )
            .is_empty()
        );
    }

    #[test]
    fn no_bet_without_edge() {
        let pinnacle = book("2026-10-24T10:00:00Z", 2.00, 3.60, 4.00);
        let winamax = tracked(book("2026-10-24T10:00:00Z", 1.85, 3.30, 3.60), MIN_CHANGES);
        assert!(
            window_bets(
                &history(vec![("pinnacle", pinnacle), ("winamax.fr", winamax)]),
                MIN_CHANGES
            )
            .is_empty()
        );
    }

    #[test]
    fn a_later_change_does_not_alter_an_earlier_decision() {
        // Absence de fuite du futur : ajouter des cotes après l'instant de décision ne change ni la cote ni l'avantage.
        let pinnacle = book("2026-10-24T10:00:00Z", 2.00, 3.60, 4.00);
        let winamax = tracked(book("2026-10-24T10:00:00Z", 2.30, 3.20, 3.40), MIN_CHANGES);
        let before = window_bets(
            &history(vec![
                ("pinnacle", pinnacle.clone()),
                ("winamax.fr", winamax.clone()),
            ]),
            MIN_CHANGES,
        );
        let mut later = winamax;
        later.home.push(entry("2026-10-25T14:30:00Z", 1.90));
        let after = window_bets(
            &history(vec![("pinnacle", pinnacle), ("winamax.fr", later)]),
            MIN_CHANGES,
        );
        assert_eq!(
            (before[0].odds, before[0].edge),
            (after[0].odds, after[0].edge)
        );
    }

    #[test]
    fn the_closing_line_measures_the_clv() {
        // Pinnacle raccourcit le domicile à 1,80 dix minutes avant le match : la CLV dépasse l'avantage annoncé.
        let mut pinnacle = book("2026-10-24T10:00:00Z", 2.00, 3.60, 4.00);
        pinnacle.home.push(entry("2026-10-25T14:50:00Z", 1.80));
        pinnacle.draw.push(entry("2026-10-25T14:50:00Z", 3.90));
        pinnacle.away.push(entry("2026-10-25T14:50:00Z", 4.60));
        let winamax = tracked(book("2026-10-24T10:00:00Z", 2.30, 3.20, 3.40), MIN_CHANGES);
        let bets = window_bets(
            &history(vec![
                ("pinnacle", pinnacle.clone()),
                ("winamax.fr", winamax),
            ]),
            MIN_CHANGES,
        );
        let close = fair_probabilities(&pinnacle, at("2026-10-25T14:59:59Z")).unwrap();
        assert_relative_eq!(bets[0].clv, 2.30 * close[0] - 1.0, epsilon = 1e-12);
        assert!(bets[0].clv > bets[0].edge);
    }

    fn bet(i: usize, clv: f64) -> Bet {
        Bet {
            fixture_id: format!("id{i:03}"),
            tournament_id: 34,
            kickoff: at("2026-10-25T15:00:00Z") + Duration::days(i64::try_from(i).unwrap()),
            home: "A".into(),
            away: "B".into(),
            bookmaker: "winamax.fr".into(),
            horizon_minutes: 60,
            outcome: Outcome::Home,
            odds: Odds::new(2.0).unwrap(),
            probability: Probability::new(0.5).unwrap(),
            edge: 0.02,
            clv,
            block: u32::try_from(i).unwrap(),
        }
    }

    fn verdict_of(clvs: &[f64], today: NaiveDate) -> Verdict {
        let bets: Vec<Bet> = clvs.iter().enumerate().map(|(i, c)| bet(i, *c)).collect();
        verdict(&bets, today)
    }

    #[test]
    fn the_verdict_follows_the_frozen_rule() {
        let soon = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let clear: Vec<f64> = (0..MIN_BETS)
            .map(|i| if i % 2 == 0 { 0.02 } else { 0.04 })
            .collect();
        assert_eq!(verdict_of(&clear, soon), Verdict::Success);
        let null: Vec<f64> = (0..MIN_BETS)
            .map(|i| if i % 2 == 0 { -0.03 } else { 0.03 })
            .collect();
        assert_eq!(verdict_of(&null, soon), Verdict::Failure);
        assert_eq!(verdict_of(&clear[..10], soon), Verdict::Pending);
        assert_eq!(verdict_of(&clear[..10], deadline()), Verdict::TooRare);
    }

    #[test]
    fn assess_ignores_matches_before_the_test_and_uses_the_same_rule() {
        let pinnacle = book("2026-10-20T10:00:00Z", 2.00, 3.60, 4.00);
        let winamax = book("2026-10-20T10:00:00Z", 2.30, 3.20, 3.40);
        let mut old = history(vec![("pinnacle", pinnacle), ("winamax.fr", winamax)]);
        old.kickoff = at("2026-10-21T15:00:00Z");
        let today = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let assessment = assess(&[old], MIN_CHANGES, today);
        assert_eq!((assessment.matches, assessment.bets.len()), (0, 0));
        assert_eq!(assessment.verdict, Verdict::Pending);
    }

    #[test]
    fn the_quality_check_pairs_matches_through_the_alias_table_only() {
        let aliases = Aliases::parse(
            r#"[F1]
"Angers" = ["Angers SCO"]
"Marseille" = ["Olympique Marseille"]"#,
        )
        .unwrap();
        let winamax = book("2026-10-24T10:00:00Z", 3.40, 3.80, 1.88);
        let h = history(vec![("winamax.fr", winamax)]);
        let quote = |outcome: &str, price: f64| ArchiveQuote {
            called_at: at("2026-10-25T12:00:00Z"),
            kickoff: at("2026-10-25T15:00:00Z"),
            sport_key: "soccer_france_ligue_one".into(),
            home_raw: "Angers".into(),
            away_raw: "Marseille".into(),
            bookmaker_key: "winamax_fr".into(),
            outcome_raw: outcome.into(),
            price,
        };
        let check = quality_check(
            std::slice::from_ref(&h),
            &[quote("Angers", 3.40), quote("Draw", 3.75)],
            &aliases,
        );
        assert_eq!((check.compared, check.identical), (2, 1));
        assert!(check.unknown_names.is_empty());

        let mut unknown = h;
        unknown.away = "OM".into();
        let check = quality_check(&[unknown], &[quote("Angers", 3.40)], &aliases);
        assert_eq!(check.compared, 0);
        assert!(check.unknown_names.contains("F1: OM"));
    }
}
