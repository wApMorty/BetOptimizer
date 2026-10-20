//! Étude historique des cotes boostées : chaque boost de WePari portant sur un match de nos ligues est rapproché du
//! match en base, traduit en conditions sur le score, puis tarifé contre Pinnacle sans marge.
//!
//! Deux prix justes par boost : celui d'**avant-match** (relevé football-data du vendredi ou du mardi, disponible au
//! moment de décider : il sert à **filtrer** sans fuite du futur) et celui de **clôture** (la meilleure estimation de la
//! vraie probabilité : il sert à **mesurer**). Le résultat réel vient du score en base, pas de WePari.

use std::collections::{BTreeSet, HashMap};

use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;

use super::Operator;
use super::pricing::{MarketTargets, Rho, market_grid, probability};
use super::selection::{self, Selection};
use super::wepari::{Page, Settlement, WepariBoost, clean};
use crate::domain::{Euros, Moment, Odds, Odds1x2, Probability};
use crate::error::{Error, Result};
use crate::eval::bootstrap::{MeanCi, block_bootstrap_mean};
use crate::ingest::aliases::Aliases;
use crate::market::dataset::{load, load_totals};
use crate::market::margin::{MarginMethod, remove_margin, remove_margin_two_way};
use crate::models::grid::ScoreGrid;

/// Premier jour couvert par WePari (août 2021) : inutile de charger les matchs antérieurs pour l'étude.
const FIRST_DAY: &str = "2021-08-01";

/// Rééchantillons du bootstrap et graine (résultats reproductibles).
const RESAMPLES: usize = 2000;
const SEED: u64 = 20_261_020;

/// Un boost rapproché d'un match et traduit en conditions sur le score.
#[derive(Debug, Clone, PartialEq)]
pub struct StudyRow {
    /// Opérateur.
    pub operator: Operator,
    /// Date du match.
    pub date: NaiveDate,
    /// Bloc de bootstrap : la semaine du match (les boosts d'une même semaine partagent leurs conditions).
    pub block: u32,
    /// Famille du pari (« victoire + les deux marquent »).
    pub category: String,
    /// Cote boostée.
    pub boosted: Odds,
    /// Cote d'origine.
    pub base: Odds,
    /// Mise maximale, si publiée.
    pub max_stake: Option<Euros>,
    /// Pari gagné, d'après le score final en base.
    pub won: bool,
    /// Accord avec l'issue publiée par WePari (`None` si WePari ne l'a pas publiée) : contrôle de qualité du
    /// rapprochement et de la traduction de l'intitulé.
    pub agrees_with_wepari: Option<bool>,
    /// Probabilité juste d'avant-match (Pinnacle, à défaut moyenne du marché), si les cotes existent.
    pub fair_pre: Option<Probability>,
    /// Probabilité juste de clôture.
    pub fair_close: Option<Probability>,
}

impl StudyRow {
    /// Probabilité juste au moment demandé.
    pub fn fair(&self, moment: Moment) -> Option<Probability> {
        match moment {
            Moment::Pre => self.fair_pre,
            Moment::Close => self.fair_close,
        }
    }

    /// Espérance de gain d'un euro misé à la cote boostée : `p · cote − 1`.
    pub fn edge(&self, moment: Moment) -> Option<f64> {
        Some(self.fair(moment)?.get() * self.boosted.get() - 1.0)
    }

    /// Gain net réalisé d'un euro misé.
    pub fn profit(&self) -> f64 {
        if self.won {
            self.boosted.get() - 1.0
        } else {
            -1.0
        }
    }
}

/// Entonnoir d'un opérateur : combien de boosts survivent à chaque étape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Funnel {
    /// Boosts lus.
    pub read: usize,
    /// Lignes illisibles.
    pub malformed: usize,
    /// Boosts rapprochés d'un match de nos ligues.
    pub matched: usize,
    /// Parmi eux, intitulés traduits en conditions sur le score final.
    pub parsed: usize,
    /// Parmi eux, tarifés à la clôture.
    pub priced_close: usize,
    /// Parmi eux, tarifés à l'avant-match.
    pub priced_pre: usize,
}

/// Résultat de l'étude.
#[derive(Debug, Clone, PartialEq)]
pub struct Study {
    /// Boosts traduits (tarifés ou non).
    pub rows: Vec<StudyRow>,
    /// Entonnoir par opérateur.
    pub funnels: Vec<(Operator, Funnel)>,
    /// Intitulés rapprochés d'un match mais non traduits, sous forme canonique (`@h gagne à la mi-temps…`).
    pub unparsed: Vec<(Operator, String)>,
}

/// Un match en base, avec ses noms canoniques.
struct GameRow {
    id: i64,
    div: String,
    home: String,
    away: String,
    ft_home: u32,
    ft_away: u32,
}

/// Cotes 1X2 et plus/moins 2,5 d'un moment, par match : Pinnacle d'abord, moyenne du marché à défaut.
struct MomentOdds {
    result: HashMap<i64, Odds1x2>,
    totals: HashMap<i64, (Odds, Odds)>,
}

/// Lance l'étude sur les pages lues. Bloque, sans rien produire, si un nom d'équipe d'un pari évaluable est inconnu
/// de la table d'alias (règle du projet : jamais d'appariement approximatif).
pub fn run(
    conn: &Connection,
    aliases: &Aliases,
    pages: &[Page],
    method: MarginMethod,
    rho: Rho,
) -> Result<Study> {
    let games = load_games(conn, FIRST_DAY)?;
    let pre = load_moment(conn, Moment::Pre)?;
    let close = load_moment(conn, Moment::Close)?;
    // Une grille par match et par moment : plusieurs boosts portent souvent sur le même match.
    let mut grids: HashMap<(i64, Moment), Option<ScoreGrid>> = HashMap::new();

    let mut rows = Vec::new();
    let mut funnels: HashMap<Operator, Funnel> = HashMap::new();
    let mut unknown = BTreeSet::new();
    let mut unparsed = Vec::new();
    for page in pages {
        for boost in &page.boosts {
            let funnel = funnels.entry(boost.operator).or_default();
            funnel.read += 1;
            let date = boost.closes_at.date();
            let day_games = games.get(&date).map(Vec::as_slice).unwrap_or(&[]);
            let Some((game, (home, away), selection)) =
                locate(boost, day_games, aliases, &mut unknown)
            else {
                continue;
            };
            funnel.matched += 1;
            let Some(selection) = selection else {
                let wording = selection::normalize(&boost.description, &home, &away);
                unparsed.push((boost.operator, wording));
                continue;
            };
            funnel.parsed += 1;
            let mut fair = |moment: Moment, odds: &MomentOdds| {
                let grid = grids
                    .entry((game.id, moment))
                    .or_insert_with(|| fit(game.id, odds, method, rho));
                grid.as_ref().map(|g| probability(g, &selection))
            };
            let fair_pre = fair(Moment::Pre, &pre);
            let fair_close = fair(Moment::Close, &close);
            funnel.priced_pre += usize::from(fair_pre.is_some());
            funnel.priced_close += usize::from(fair_close.is_some());
            let won = selection.holds(game.ft_home, game.ft_away);
            rows.push(StudyRow {
                operator: boost.operator,
                date,
                block: ((date.num_days_from_ce() - 1) / 7) as u32,
                category: selection.category(),
                boosted: boost.boosted,
                base: boost.base,
                max_stake: boost.max_stake,
                won,
                agrees_with_wepari: match boost.settlement {
                    Settlement::Won => Some(won),
                    Settlement::Lost => Some(!won),
                    Settlement::Unknown => None,
                },
                fair_pre,
                fair_close,
            });
        }
        // Les lignes illisibles sont comptées à part : elles n'ont pas d'opérateur propre, on les attribue à celui de
        // la page.
        if let Some(first) = page.boosts.first() {
            funnels.entry(first.operator).or_default().malformed += page.malformed;
        }
    }
    if !unknown.is_empty() {
        return Err(Error::UnknownTeams {
            names: unknown.into_iter().collect(),
        });
    }
    let mut funnels: Vec<(Operator, Funnel)> = funnels.into_iter().collect();
    funnels.sort_by_key(|(operator, _)| *operator);
    Ok(Study {
        rows,
        funnels,
        unparsed,
    })
}

/// Cherche le match d'un boost parmi ceux du jour. Renvoie le match, les noms d'équipe tels qu'écrits dans
/// l'événement et la traduction de l'intitulé (`None` si l'intitulé n'est pas un pari sur le score final).
///
/// Si une seule des deux équipes est reconnue et joue ce jour-là, et que l'intitulé est un pari évaluable, l'autre nom
/// est noté comme inconnu : il manque probablement un alias.
fn locate<'g>(
    boost: &WepariBoost,
    day_games: &'g [GameRow],
    aliases: &Aliases,
    unknown: &mut BTreeSet<String>,
) -> Option<(&'g GameRow, (String, String), Option<Selection>)> {
    let pairs = team_pairs(&boost.event);
    for game in day_games {
        for (home, away) in &pairs {
            let home_name = aliases.resolve(&game.div, home);
            let away_name = aliases.resolve(&game.div, away);
            let home_ok = home_name == Some(game.home.as_str());
            let away_ok = away_name == Some(game.away.as_str());
            if home_ok && away_ok {
                let selection = selection::parse(&boost.description, home, away);
                // `clone` : les noms appartiennent à `pairs`, une variable locale qui disparaît au retour.
                return Some((game, (home.clone(), away.clone()), selection));
            }
            let evaluable = || selection::parse(&boost.description, home, away).is_some();
            if home_ok && away_name.is_none() && evaluable() {
                unknown.insert(format!("{}: {away}", game.div));
            }
            if away_ok && home_name.is_none() && evaluable() {
                unknown.insert(format!("{}: {home}", game.div));
            }
        }
    }
    None
}

/// Couples (domicile, extérieur) possibles d'un intitulé d'événement.
///
/// Formes rencontrées : « Cote Boostée : Lyon - Nantes », « La Grosse Cote Boostée MT Lyon - Nantes »,
/// « Lyon-Nantes - Cotes Boostées Ligue 1 », « NiceCotes Boostées L1 Uber Eats » (sans séparateur),
/// « Man. City vs Liverpool ». Un nom peut contenir un tiret (« Saint-Etienne ») : on essaie donc chaque tiret comme
/// séparateur, et c'est la table d'alias qui tranche.
fn team_pairs(event: &str) -> Vec<(String, String)> {
    let mut text = clean(event);
    // Parions Sport : la compétition suit les équipes (« … - Cotes Boostées Ligue 1 »).
    if let Some(end) = text.find("Cotes Boostées").filter(|end| *end > 0) {
        text.truncate(end);
    }
    // Winamax : l'étiquette précède les équipes (« Cote Boostée : », « La Grosse Cote Boostée »).
    if let Some(start) = text.rfind("Boostée") {
        let rest = &text[start + "Boostée".len()..];
        // « Boostées » : le « s » du pluriel fait partie de l'étiquette.
        text = rest.strip_prefix('s').unwrap_or(rest).to_string();
    }
    let mut text = text.trim_matches(TRIMMED).to_string();
    // Étiquettes de produits ou de sport placées devant le nom de l'équipe à domicile, parfois plusieurs.
    while let Some(rest) = LABELS.iter().find_map(|label| text.strip_prefix(label)) {
        text = rest.trim_start_matches(TRIMMED).to_string();
    }
    if let Some((home, away)) = text.split_once(" vs ") {
        return vec![(home.trim().to_string(), away.trim().to_string())];
    }
    text.match_indices('-')
        .map(|(at, _)| (text[..at].trim(), text[at + 1..].trim()))
        .filter(|(home, away)| !home.is_empty() && !away.is_empty())
        .map(|(home, away)| (home.to_string(), away.to_string()))
        .collect()
}

/// Séparateurs retirés autour des étiquettes.
const TRIMMED: [char; 3] = [':', '-', ' '];

/// Étiquettes de produits Winamax ou de sport, jamais des noms d'équipe.
const LABELS: [&str; 6] = [
    "Football ",
    "MyMatch ",
    "MT ",
    "WinaTV ",
    "WINAMAXICO ",
    "Winamaxico ",
];

/// Grille calée sur le marché d'un match à un moment, ou `None` si les cotes manquent ou si l'ajustement échoue.
fn fit(game_id: i64, odds: &MomentOdds, method: MarginMethod, rho: Rho) -> Option<ScoreGrid> {
    let result = remove_margin(odds.result.get(&game_id)?, method)?;
    let (over, under) = odds.totals.get(&game_id)?;
    let over_2_5 = remove_margin_two_way(*over, *under, method)?;
    market_grid(MarketTargets { result, over_2_5 }, rho)
}

/// Paris de référence pour contrôler la grille : les familles les plus fréquentes parmi les boosts, écrites pour un
/// match `A`–`B` (A à domicile).
pub const REFERENCE_BETS: [&str; 14] = [
    "les deux équipes marquent",
    "plus de 1,5 buts",
    "plus de 3,5 buts",
    "plus de 4,5 buts",
    "A gagne et les deux équipes marquent",
    "B gagne et les deux équipes marquent",
    "A gagne et plus de 2,5 buts",
    "A gagne par au moins 2 buts d'écart",
    "A gagne sans encaisser de but",
    "A marque au moins 2 buts",
    "B marque au moins 2 buts",
    "A gagne 1-0",
    "A gagne 2-1",
    "A gagne ou fait match nul et les deux équipes marquent",
];

/// Calibration de la grille sur un pari de référence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceCheck {
    /// Intitulé du pari.
    pub wording: &'static str,
    /// Probabilité moyenne annoncée par la grille.
    pub predicted: f64,
    /// Fréquence observée.
    pub observed: f64,
    /// Écart en écarts-types : `(O − Σp) / √(Σ p(1 − p))`.
    pub z: f64,
    /// Log loss moyenne : `−moy[y ln p + (1 − y) ln(1 − p)]`.
    pub log_loss: f64,
}

/// Contrôle de la grille calée sur la clôture de Pinnacle, sur tous les matchs joués entre `from` et `to` inclus.
#[derive(Debug, Clone, PartialEq)]
pub struct GridCheck {
    /// Matchs dont la grille a pu être ajustée.
    pub games: usize,
    /// Matchs écartés : cotes absentes, ou ajustement trop éloigné du marché.
    pub rejected: usize,
    /// Un contrôle par pari de référence, dans l'ordre de [`REFERENCE_BETS`].
    pub checks: Vec<ReferenceCheck>,
}

impl GridCheck {
    /// Log loss moyenne sur l'ensemble des paris de référence : le critère de choix de ρ.
    pub fn mean_log_loss(&self) -> f64 {
        self.checks.iter().map(|c| c.log_loss).sum::<f64>() / self.checks.len().max(1) as f64
    }
}

/// Calibration de la grille : pour chaque match de la période, grille calée sur la clôture de **Pinnacle** (pas de
/// repli sur la moyenne : le contrôle doit porter sur la référence elle-même), puis comparaison des probabilités des
/// paris de référence aux résultats.
pub fn check_grid(
    conn: &Connection,
    method: MarginMethod,
    rho: Rho,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<GridCheck> {
    let selections: Vec<Selection> = REFERENCE_BETS
        .iter()
        .map(|wording| {
            selection::parse(wording, "A", "B")
                .ok_or_else(|| Error::Data(format!("pari de référence illisible : {wording}")))
        })
        .collect::<Result<_>>()?;
    let result: HashMap<i64, Odds1x2> = load(conn, "PS", Moment::Close)?
        .into_iter()
        .map(|g| (g.game_id, g.odds))
        .collect();
    let totals: HashMap<i64, (Odds, Odds)> = load_totals(conn, "PS", Moment::Close)?
        .into_iter()
        .map(|g| (g.game_id, (g.over, g.under)))
        .collect();
    let odds = MomentOdds { result, totals };

    // Par pari de référence : (Σp, Σp(1−p), victoires, Σ pertes logarithmiques).
    let mut sums = vec![(0.0, 0.0, 0usize, 0.0); selections.len()];
    let (mut games, mut rejected) = (0, 0);
    for (date, day) in load_games(conn, &from.format("%Y-%m-%d").to_string())? {
        if date > to {
            continue;
        }
        for game in day {
            let Some(grid) = fit(game.id, &odds, method, rho) else {
                rejected += 1;
                continue;
            };
            games += 1;
            for (selection, sum) in selections.iter().zip(sums.iter_mut()) {
                // Bornée loin de 0 et 1 : un score à probabilité infime ne doit pas rendre la log loss infinie.
                let p = probability(&grid, selection).get().clamp(1e-9, 1.0 - 1e-9);
                let won = selection.holds(game.ft_home, game.ft_away);
                sum.0 += p;
                sum.1 += p * (1.0 - p);
                sum.2 += usize::from(won);
                sum.3 -= if won { p.ln() } else { (1.0 - p).ln() };
            }
        }
    }
    let n = games.max(1) as f64;
    let checks = REFERENCE_BETS
        .iter()
        .zip(&sums)
        .map(|(wording, (p, variance, wins, loss))| ReferenceCheck {
            wording,
            predicted: p / n,
            observed: *wins as f64 / n,
            z: (*wins as f64 - p) / variance.sqrt(),
            log_loss: loss / n,
        })
        .collect();
    Ok(GridCheck {
        games,
        rejected,
        checks,
    })
}

/// Matchs joués à partir de `first_day` (`AAAA-MM-JJ`), regroupés par date.
fn load_games(conn: &Connection, first_day: &str) -> Result<HashMap<NaiveDate, Vec<GameRow>>> {
    let mut statement = conn.prepare(
        "SELECT g.id, g.div, g.match_date, th.name, ta.name, g.ft_home, g.ft_away
           FROM game g
           JOIN team th ON th.id = g.home_id
           JOIN team ta ON ta.id = g.away_id
          WHERE g.match_date >= ?1",
    )?;
    let rows = statement.query_map([first_day], |row| {
        Ok((
            row.get::<_, String>(2)?,
            GameRow {
                id: row.get(0)?,
                div: row.get(1)?,
                home: row.get(3)?,
                away: row.get(4)?,
                ft_home: row.get(5)?,
                ft_away: row.get(6)?,
            },
        ))
    })?;
    let mut by_date: HashMap<NaiveDate, Vec<GameRow>> = HashMap::new();
    for row in rows {
        let (date, game) = row?;
        let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|e| Error::Data(format!("date de match illisible « {date} » : {e}")))?;
        by_date.entry(date).or_default().push(game);
    }
    Ok(by_date)
}

/// Cotes d'un moment : Pinnacle (`PS`) quand elles existent, sinon la moyenne du marché (`Avg`).
fn load_moment(conn: &Connection, moment: Moment) -> Result<MomentOdds> {
    let mut result = HashMap::new();
    let mut totals = HashMap::new();
    // La moyenne d'abord, puis Pinnacle par-dessus : `insert` remplace, donc Pinnacle l'emporte quand il existe.
    for bookmaker in ["Avg", "PS"] {
        for game in load(conn, bookmaker, moment)? {
            result.insert(game.game_id, game.odds);
        }
        for game in load_totals(conn, bookmaker, moment)? {
            totals.insert(game.game_id, (game.over, game.under));
        }
    }
    Ok(MomentOdds { result, totals })
}

/// Bilan d'un groupe de boosts tarifés à un moment donné.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    /// Nombre de boosts tarifés.
    pub count: usize,
    /// Espérance d'un euro misé à la cote boostée, contre le marché, avec intervalle de confiance à 95 %.
    pub edge: MeanCi,
    /// Part des boosts d'espérance positive.
    pub positive_share: f64,
    /// Gain réalisé d'un euro misé sur chaque boost, avec intervalle de confiance à 95 %.
    pub roi: MeanCi,
    /// Espérance moyenne de la cote **d'origine** : la marge de l'opérateur sur ces paris avant le boost.
    pub base_edge: f64,
    /// Victoires attendues selon le marché (somme des probabilités justes).
    pub expected_wins: f64,
    /// Victoires observées.
    pub observed_wins: usize,
}

impl Summary {
    /// Écart entre victoires observées et attendues, en écarts-types : `z = (O − Σp) / √(Σ p(1 − p))`.
    /// Loin de 0 (au-delà de ±2), la tarification se trompe sur cette famille de paris.
    pub fn calibration_z(&self, rows: &[&StudyRow], moment: Moment) -> f64 {
        let variance: f64 = rows
            .iter()
            .filter_map(|r| r.fair(moment))
            .map(|p| p.get() * (1.0 - p.get()))
            .sum();
        (self.observed_wins as f64 - self.expected_wins) / variance.sqrt()
    }
}

/// Gain réel d'un euro misé sur chaque boost **réglé par WePari**, tous sports, sans aucun modèle ni rapprochement :
/// `cote − 1` si gagné, `−1` si perdu. Intervalle par bootstrap par blocs d'un mois. `None` si aucun boost n'est réglé.
pub fn settled_roi<'a>(boosts: impl Iterator<Item = &'a WepariBoost>) -> Option<(usize, MeanCi)> {
    let profits: Vec<(u32, f64)> = boosts
        .filter_map(|b| {
            let profit = match b.settlement {
                Settlement::Won => b.boosted.get() - 1.0,
                Settlement::Lost => -1.0,
                Settlement::Unknown => return None,
            };
            let date = b.closes_at.date();
            // Bloc : le mois (année × 12 + mois), toujours positif pour des dates après l'an 0.
            Some(((date.year() * 12) as u32 + date.month(), profit))
        })
        .collect();
    Some((
        profits.len(),
        block_bootstrap_mean(&profits, RESAMPLES, SEED)?,
    ))
}

/// Bilan des boosts tarifés au `moment` donné parmi `rows`. `None` si aucun ne l'est.
///
/// Intervalles par bootstrap par blocs (une semaine = un bloc) : les boosts d'une même semaine ne sont pas
/// indépendants.
pub fn summarize(rows: &[&StudyRow], moment: Moment) -> Option<Summary> {
    let priced: Vec<(&StudyRow, f64)> = rows
        .iter()
        .filter_map(|r| Some((*r, r.fair(moment)?.get())))
        .collect();
    if priced.is_empty() {
        return None;
    }
    let count = priced.len();
    let edges: Vec<(u32, f64)> = priced
        .iter()
        .map(|(r, p)| (r.block, p * r.boosted.get() - 1.0))
        .collect();
    let profits: Vec<(u32, f64)> = priced.iter().map(|(r, _)| (r.block, r.profit())).collect();
    let positive = edges.iter().filter(|(_, e)| *e > 0.0).count();
    let base_edge = priced
        .iter()
        .map(|(r, p)| p * r.base.get() - 1.0)
        .sum::<f64>()
        / count as f64;
    Some(Summary {
        count,
        edge: block_bootstrap_mean(&edges, RESAMPLES, SEED)?,
        positive_share: positive as f64 / count as f64,
        roi: block_bootstrap_mean(&profits, RESAMPLES, SEED)?,
        base_edge,
        expected_wins: priced.iter().map(|(_, p)| p).sum(),
        observed_wins: priced.iter().filter(|(r, _)| r.won).count(),
    })
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;
    use chrono::NaiveDateTime;

    use super::*;
    use crate::store::migrate;

    #[test]
    fn event_wordings_give_candidate_team_pairs() {
        let pairs = |e: &str| team_pairs(e);
        assert_eq!(
            pairs("Cote Boostée : Lyon - Nantes"),
            vec![("Lyon".to_string(), "Nantes".to_string())]
        );
        assert_eq!(
            pairs("Cote Boostée : MyMatch Arsenal - Chelsea"),
            vec![("Arsenal".to_string(), "Chelsea".to_string())]
        );
        assert_eq!(
            pairs("Man. City vs Liverpool"),
            vec![("Man. City".to_string(), "Liverpool".to_string())]
        );
        // Un nom à tiret : chaque tiret est essayé.
        assert_eq!(
            pairs("Saint-Etienne-Lens - Cotes Boostées Ligue 1"),
            vec![
                ("Saint".to_string(), "Etienne-Lens".to_string()),
                ("Saint-Etienne".to_string(), "Lens".to_string()),
            ]
        );
        assert_eq!(
            pairs("La Grosse Cote Boostée Football : MT Lyon - Nantes"),
            vec![("Lyon".to_string(), "Nantes".to_string())]
        );
        assert_eq!(
            pairs("Nice-Lens Cotes Boostées L1 Uber Eats"),
            vec![("Nice".to_string(), "Lens".to_string())]
        );
        assert!(pairs("Cote Boostée - Serie A").is_empty());
    }

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO team (id, name) VALUES (1, 'Lyon'), (2, 'Nantes');
             INSERT INTO game (id, div, season, match_date, home_id, away_id, ft_home, ft_away, ft_result)
               VALUES (1, 'F1', '2023/24', '2023-09-24', 1, 2, 2, 1, 'H');
             INSERT INTO game_odds VALUES (1, 'PS', 'close', 1.80, 3.80, 4.60), (1, 'PS', 'pre', 1.85, 3.75, 4.40);
             INSERT INTO game_totals_odds VALUES (1, 'PS', 'close', 2.5, 1.90, 1.98);",
        )
        .unwrap();
        conn
    }

    fn boost(description: &str, event: &str) -> WepariBoost {
        WepariBoost {
            operator: Operator::Winamax,
            description: description.to_string(),
            event: event.to_string(),
            closes_at: NaiveDateTime::parse_from_str("24-09-2023 20:45", "%d-%m-%Y %H:%M").unwrap(),
            boosted: Odds::new(3.0).unwrap(),
            base: Odds::new(2.55).unwrap(),
            max_stake: None,
            settlement: Settlement::Won,
        }
    }

    const ALIASES: &str = r#"
[F1]
"Lyon" = ["Olympique Lyonnais"]
"Nantes" = []
"#;

    #[test]
    fn a_boost_is_matched_priced_and_settled_from_the_score() {
        let conn = database();
        let aliases = Aliases::parse(ALIASES).unwrap();
        let page = Page {
            boosts: vec![boost(
                "Olympique Lyonnais gagne et les deux équipes marquent",
                "Cote Boostée : Olympique Lyonnais - Nantes",
            )],
            malformed: 0,
        };
        let study = run(&conn, &aliases, &[page], MarginMethod::Power, Rho::Fitted).unwrap();
        assert_eq!(study.rows.len(), 1);
        let row = &study.rows[0];
        assert!(row.won, "2-1 : Lyon gagne et les deux équipes marquent");
        assert_eq!(row.agrees_with_wepari, Some(true));
        assert_eq!(row.category, "victoire + les deux marquent");
        assert!(row.fair_close.is_some());
        // Pas de cotes plus/moins d'avant-match : pas de prix d'avant-match.
        assert_eq!(row.fair_pre, None);
        let funnel = study.funnels[0].1;
        assert_eq!((funnel.read, funnel.matched, funnel.parsed), (1, 1, 1));
        assert_eq!((funnel.priced_close, funnel.priced_pre), (1, 0));
    }

    #[test]
    fn an_unknown_team_in_an_evaluable_boost_blocks_the_study() {
        let conn = database();
        let aliases = Aliases::parse(ALIASES).unwrap();
        let page = Page {
            boosts: vec![boost("OL gagne", "Cote Boostée : OL - Nantes")],
            malformed: 0,
        };
        let error = run(&conn, &aliases, &[page], MarginMethod::Power, Rho::Fitted).unwrap_err();
        assert!(error.to_string().contains("F1: OL"), "{error}");
    }

    #[test]
    fn boosts_outside_our_leagues_or_out_of_scope_are_ignored_without_blocking() {
        let conn = database();
        let aliases = Aliases::parse(ALIASES).unwrap();
        let page = Page {
            boosts: vec![
                boost("Real Madrid gagne", "Cote Boostée : Real Madrid - Getafe"),
                // Nom inconnu, mais pari de buteur : non évaluable, donc pas bloquant.
                boost("Lacazette buteur", "Cote Boostée : OL - Nantes"),
            ],
            malformed: 0,
        };
        let study = run(&conn, &aliases, &[page], MarginMethod::Power, Rho::Fitted).unwrap();
        assert!(study.rows.is_empty());
    }

    #[test]
    fn the_grid_check_compares_reference_bets_to_results() {
        let conn = database();
        let day = NaiveDate::from_ymd_opt(2023, 9, 24).unwrap();
        let check = check_grid(&conn, MarginMethod::Power, Rho::Fitted, day, day).unwrap();
        assert_eq!((check.games, check.rejected), (1, 0));
        assert_eq!(check.checks.len(), REFERENCE_BETS.len());
        // 2-1 : les deux équipes marquent, donc observé = 100 % sur ce pari.
        let btts = &check.checks[0];
        assert_eq!(btts.wording, "les deux équipes marquent");
        assert_abs_diff_eq!(btts.observed, 1.0);
        assert!(btts.predicted > 0.3 && btts.predicted < 0.7, "{btts:?}");
        assert!(btts.log_loss > 0.0);
        // Hors période : aucun match.
        let later = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        assert_eq!(
            check_grid(&conn, MarginMethod::Power, Rho::Fitted, later, later)
                .unwrap()
                .games,
            0
        );
    }

    #[test]
    fn settled_roi_ignores_unsettled_boosts() {
        let won = boost("A gagne", "Cote Boostée : A - B");
        let mut lost = won.clone();
        lost.settlement = Settlement::Lost;
        let mut open = won.clone();
        open.settlement = Settlement::Unknown;
        let (count, roi) = settled_roi([won, lost, open].iter()).unwrap();
        assert_eq!(count, 2);
        // Cote 3,00 : +2 puis −1, moyenne +0,5.
        assert_abs_diff_eq!(roi.mean, 0.5, epsilon = 1e-12);
        assert!(settled_roi(std::iter::empty()).is_none());
    }

    #[test]
    fn a_fair_boost_has_zero_edge_and_the_summary_counts_wins() {
        let row = |won: bool, block: u32| StudyRow {
            operator: Operator::Winamax,
            date: NaiveDate::from_ymd_opt(2023, 9, 24).unwrap(),
            block,
            category: "victoire".to_string(),
            boosted: Odds::new(2.0).unwrap(),
            base: Odds::new(1.8).unwrap(),
            max_stake: None,
            won,
            agrees_with_wepari: None,
            fair_pre: None,
            fair_close: Probability::new(0.5),
        };
        let rows = [row(true, 0), row(false, 1), row(true, 2), row(false, 3)];
        let refs: Vec<&StudyRow> = rows.iter().collect();
        let summary = summarize(&refs, Moment::Close).unwrap();
        assert_abs_diff_eq!(summary.edge.mean, 0.0, epsilon = 1e-12);
        assert_abs_diff_eq!(summary.base_edge, -0.1, epsilon = 1e-12);
        assert_abs_diff_eq!(summary.roi.mean, 0.0, epsilon = 1e-12);
        assert_eq!(summary.observed_wins, 2);
        assert_abs_diff_eq!(summary.expected_wins, 2.0, epsilon = 1e-12);
        assert_abs_diff_eq!(
            summary.calibration_z(&refs, Moment::Close),
            0.0,
            epsilon = 1e-12
        );
        assert!(summarize(&refs, Moment::Pre).is_none());
    }
}
