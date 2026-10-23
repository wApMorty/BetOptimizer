//! Alerte EuroMillions : le cinquième tirage consécutif au plafond verse le jackpot au rang 2 s'il n'a pas de gagnant
//! (règlement, art. 4.2.5.6). Ce tirage n'est jamais arrivé ; on l'estime à une fois tous les deux à quatre ans, à une
//! espérance de +36 à +79 % par grille (`docs/sources.md` §43, avec les 0,9 € par grille que reçoivent les rangs 2 à 13
//! au plafond, excédent compris). L'alerte retient 0,44 € (la clé normale, sans l'excédent) : elle est plus prudente.
//!
//! **Détection.** L'archive ne donne pas le montant du jackpot quand il n'est pas gagné. Mais à chaque tirage au plafond,
//! l'excédent de la part du rang 1 va au rang inférieur le plus proche ayant un gagnant (art. 4.2.5.5) : ce rang reçoit
//! bien plus que sa clé normale (`2,61 % × 1,10 € × N` pour le rang 2). Depuis février 2020, les tirages au plafond sans
//! gagnant ont un excédent de 6 à 30 M€, les autres tirages moins de 1 M€ : le seuil est [`CAP_EXCESS_MIN`]. Un premier
//! tirage au plafond peut avoir un petit excédent (le jackpot vient d'atteindre le plafond) et passer inaperçu : d'où une
//! veille dès trois tirages, avant l'alerte à quatre.

use chrono::{DateTime, Datelike, Days, NaiveDate, Utc, Weekday};
use rusqlite::Connection;

use super::estimate::{self, rank_probability};
use super::{Draw, Game, rules};
use crate::config::{Config, WEBHOOK_ENV};
use crate::error::{Error, Result};
use crate::notify::discord;
use crate::store::loto as store;

/// Les règles actuelles du plafond s'appliquent depuis le 1er février 2020 (art. 4.2.5.5).
pub fn rules_since() -> NaiveDate {
    NaiveDate::from_ymd_opt(2020, 2, 1).expect("date valide écrite en dur")
}

/// Excédent minimal pour qu'un tirage soit compté au plafond, en euros (voir l'en-tête du module).
pub const CAP_EXCESS_MIN: f64 = 3_000_000.0;

/// Excédent au-delà duquel le jackpot lui-même est descendu au rang 2 : le cycle est fini.
pub const ROLLDOWN_EXCESS_MIN: f64 = 0.5 * rules::EURO_CAP;

/// Écart relatif maximal entre les nombres de grilles estimés par les rangs 12 et 13. Au-delà, les gagnants publiés ne
/// collent plus à la matrice 5/50 et 2/12 : les règles ont sans doute changé (EuroMillions modifiée de 2027).
pub const MATRIX_TOLERANCE: f64 = 0.30;

/// Reversement des rangs 2 à 13 par grille, hors excédent, en euros : 40 % de 1,10 €.
pub fn lower_ranks_per_grid() -> f64 {
    rules::EURO_RANK_SHARES.iter().sum::<f64>() * rules::EURO_WINNERS_PER_GRID
}

/// État d'un tirage vis-à-vis du plafond.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapCheck {
    /// Date du tirage.
    pub date: NaiveDate,
    /// Grilles jouées en Europe (estimation).
    pub grids: f64,
    /// Excédent versé au premier rang inférieur ayant un gagnant, en euros.
    pub excess: f64,
    /// Le jackpot a été gagné au rang 1.
    pub jackpot_won: bool,
}

impl CapCheck {
    /// Tirage au plafond (excédent net).
    pub fn at_cap(&self) -> bool {
        self.excess >= CAP_EXCESS_MIN
    }

    /// Le jackpot est descendu au rang inférieur (cinquième tirage au plafond sans gagnant).
    pub fn rolled_down(&self) -> bool {
        self.excess >= ROLLDOWN_EXCESS_MIN
    }
}

/// Excédent d'un tirage. `None` si l'archive est incomplète.
///
/// `k*` est le premier rang à partir du 2 qui a un gagnant ; les rangs 2 à `k* − 1` sans gagnant lui ont reporté leur
/// somme (art. 4.2.5.8). Excédent = `n_k* · g_k* − Σ_{k=2..k*} clé_k · 1,10 € · N`.
pub fn check(draw: &Draw) -> Option<CapCheck> {
    let grids = estimate::grids(draw)?.n;
    let first = draw.ranks.first()?;
    let (offset, rank) = draw
        .ranks
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, r)| r.winners > 0)?;
    let normal: f64 = rules::EURO_RANK_SHARES[..offset].iter().sum::<f64>()
        * rules::EURO_WINNERS_PER_GRID
        * grids;
    Some(CapCheck {
        date: draw.date,
        grids,
        excess: rank.winners as f64 * rank.unit_gain - normal,
        jackpot_won: first.winners > 0,
    })
}

/// Vérifie que les gagnants publiés collent à la matrice actuelle : les estimations de `N` par les rangs 12 et 13
/// doivent s'accorder à [`MATRIX_TOLERANCE`] près.
pub fn matches_matrix(draw: &Draw) -> bool {
    let single = |k: usize| {
        draw.ranks
            .get(k - 1)
            .map(|r| r.winners as f64 / rank_probability(Game::EuroMillions, k))
    };
    match (single(12), single(13)) {
        (Some(a), Some(b)) if a > 0.0 && b > 0.0 => (a / b - 1.0).abs() <= MATRIX_TOLERANCE,
        _ => false,
    }
}

/// Niveau d'alerte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Trois tirages au plafond sans gagnant : le prochain est le quatrième, ou le cinquième si le premier a échappé à
    /// la détection.
    Watch,
    /// Quatre tirages au plafond sans gagnant : le prochain est le cinquième.
    Fifth,
}

impl Level {
    /// Code en base.
    pub fn code(self) -> &'static str {
        match self {
            Level::Watch => "watch",
            Level::Fifth => "fifth",
        }
    }
}

/// Alerte à envoyer.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    /// Niveau.
    pub level: Level,
    /// Dernier tirage au plafond sans gagnant.
    pub draw_date: NaiveDate,
    /// Tirages au plafond sans gagnant détectés à la suite.
    pub streak: usize,
    /// Date du prochain tirage (mardi ou vendredi).
    pub next_draw: NaiveDate,
    /// Plus grand nombre de grilles de la série.
    pub max_grids: f64,
    /// Message, prêt pour Discord.
    pub message: String,
}

/// Prochain tirage après `date` : les tirages ont lieu le mardi et le vendredi.
pub fn next_draw(date: NaiveDate) -> NaiveDate {
    let days = match date.weekday() {
        Weekday::Mon => 1,
        Weekday::Tue => 3,
        Weekday::Wed => 2,
        Weekday::Thu => 1,
        Weekday::Fri => 4,
        Weekday::Sat => 3,
        Weekday::Sun => 2,
    };
    date + Days::new(days)
}

/// Espérance d'une grille au tirage où le jackpot `J` est forcément versé (au rang 1 ou au rang 2), avec `N` grilles :
/// `J / N + 0,44 €` (tout le jackpot est versé ; les rangs 2 à 13 reçoivent leur part). L'excédent du tirage lui-même,
/// versé en plus, est ignoré : l'estimation est prudente.
pub fn forced_value(grids: f64) -> f64 {
    rules::EURO_CAP / grids + lower_ranks_per_grid()
}

/// Série en cours : tirages au plafond sans gagnant qui terminent l'archive, du plus récent au plus ancien (vide si le
/// dernier tirage n'est pas au plafond), et date du dernier tirage relevé.
///
/// Erreur si le dernier tirage ne colle plus à la matrice : mieux vaut un échec visible (timer en échec) qu'une alerte
/// fausse ou manquée.
pub fn current_streak(draws: &[Draw]) -> Result<(Vec<CapCheck>, Option<NaiveDate>)> {
    let mut recent: Vec<&Draw> = draws
        .iter()
        .filter(|d| d.game == Game::EuroMillions && d.date >= rules_since())
        .collect();
    recent.sort_by_key(|d| d.date);
    let Some(last) = recent.last() else {
        return Ok((Vec::new(), None));
    };
    if !matches_matrix(last) {
        return Err(Error::Data(format!(
            "EuroMillions du {} : les gagnants publiés ne collent plus à la matrice 5/50 et 2/12 ; les règles ont \
             peut-être changé (EuroMillions modifiée de 2027) : revoir `loto::rules` avant de se fier à l'alerte",
            last.date
        )));
    }
    let mut streak = Vec::new();
    for draw in recent.iter().rev() {
        let Some(state) = check(draw) else { break };
        if state.at_cap() && !state.jackpot_won && !state.rolled_down() {
            streak.push(state);
        } else {
            break;
        }
    }
    Ok((streak, Some(last.date)))
}

/// Alerte due d'après les tirages de l'EuroMillions (en base, dans n'importe quel ordre), ou `None`. Erreur si la
/// matrice a changé ([`current_streak`]).
pub fn pending(draws: &[Draw]) -> Result<Option<Alert>> {
    let (streak, _) = current_streak(draws)?;
    let level = match streak.len() {
        0..=2 => return Ok(None),
        3 => Level::Watch,
        _ => Level::Fifth,
    };
    let draw_date = streak[0].date;
    let max_grids = streak.iter().map(|s| s.grids).fold(0.0, f64::max);
    let next = next_draw(draw_date);
    let message = message(level, streak.len(), draw_date, next, max_grids);
    Ok(Some(Alert {
        level,
        draw_date,
        streak: streak.len(),
        next_draw: next,
        max_grids,
        message,
    }))
}

fn message(
    level: Level,
    streak: usize,
    last: NaiveDate,
    next: NaiveDate,
    max_grids: f64,
) -> String {
    let date = |d: NaiveDate| d.format("%d/%m/%Y").to_string();
    let millions = |n: f64| format!("{:.0}", n / 1e6);
    let euros = |v: f64| format!("{:.2}", v).replace('.', ",");
    let percent = |v: f64| format!("{:+.0} %", (v / rules::EURO_PRICE - 1.0) * 100.0);
    let mut text = match level {
        Level::Fifth => format!(
            "**EuroMillions · cinquième tirage au plafond**\n{streak} tirages consécutifs au plafond de 250 M€ sans \
             gagnant (dernier : {}). Le tirage du {} est le cinquième : s'il n'a pas de gagnant au rang 1, les 250 M€ \
             vont aux gagnants du rang 2 (5 numéros et 1 étoile ; règlement, art. 4.2.5.6).\n",
            date(last),
            date(next)
        ),
        Level::Watch => format!(
            "**EuroMillions · veille plafond**\n{streak} tirages consécutifs au plafond sans gagnant (dernier : {}). Le \
             tirage du {} est le quatrième, ou le cinquième si le premier tirage au plafond a échappé à la détection. \
             Vérifie sur fdj.fr le rang du tirage au plafond annoncé.\n",
            date(last),
            date(next)
        ),
    };
    text.push_str("Espérance d'une grille de 2,50 € si c'est le cinquième :");
    for factor in [1.0, 1.5, 2.0] {
        let grids = max_grids * factor;
        let value = forced_value(grids);
        text.push_str(&format!(
            " {} € avec {} millions de grilles ({}) ;",
            euros(value),
            millions(grids),
            percent(value)
        ));
    }
    text.pop();
    text.push_str(
        ".\nUne grille a une chance sur 6 991 908 d'être au rang 2 : loterie à espérance positive, pas un gain probable. \
         Vérifie sur fdj.fr que le jackpot affiché est bien de 250 M€.",
    );
    text
}

/// Ce qu'est devenue l'alerte due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notified {
    /// Aucune alerte due.
    Nothing,
    /// Alerte due, déjà envoyée lors d'un passage précédent.
    AlreadySent,
    /// Alerte due, affichée seulement (envoi non demandé) : elle n'est pas journalisée.
    Shown,
    /// Alerte envoyée sur Discord et journalisée.
    Sent,
    /// Échec de l'envoi : rien n'est journalisé, le passage suivant réessaiera.
    Failed(String),
}

/// Cherche l'alerte due dans la base et, si `notify`, l'envoie sur Discord une seule fois (journal `loto_alert`).
pub fn notify_pending(
    conn: &Connection,
    config: &Config,
    now: DateTime<Utc>,
    notify: bool,
) -> Result<(Option<Alert>, Notified)> {
    let draws = store::draws(conn, Game::EuroMillions)?;
    let Some(alert) = pending(&draws)? else {
        return Ok((None, Notified::Nothing));
    };
    if store::alert_sent(conn, alert.draw_date, alert.level.code())? {
        return Ok((Some(alert), Notified::AlreadySent));
    }
    if !notify {
        return Ok((Some(alert), Notified::Shown));
    }
    let sent = config
        .discord_webhook(std::env::var(WEBHOOK_ENV).ok().as_deref())
        .and_then(|webhook| discord::send(&webhook, &alert.message, &[], &mut discord::http_post));
    match sent {
        Ok(()) => {
            store::record_alert(
                conn,
                now,
                alert.draw_date,
                alert.level.code(),
                &alert.message,
            )?;
            Ok((Some(alert), Notified::Sent))
        }
        Err(error) => Ok((Some(alert), Notified::Failed(error.to_string()))),
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;
    use crate::loto::RankResult;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Tirage EuroMillions de `n` grilles : gagnants à leur espérance, rang 2 normal plus `excess` euros, jackpot gagné
    /// ou non.
    fn draw(date: NaiveDate, n: f64, excess: f64, won: bool) -> Draw {
        let mut ranks: Vec<RankResult> = (1..=13)
            .map(|k| RankResult {
                winners: (n * rank_probability(Game::EuroMillions, k))
                    .round()
                    .max(1.0) as u64,
                unit_gain: 10.0,
            })
            .collect();
        ranks[0].winners = u64::from(won);
        let normal = rules::EURO_RANK_SHARES[0] * rules::EURO_WINNERS_PER_GRID * n;
        ranks[1].unit_gain = (normal + excess) / ranks[1].winners as f64;
        Draw {
            game: Game::EuroMillions,
            date,
            balls: [1, 2, 3, 4, 5],
            extra: vec![1, 2],
            ranks,
            codes: None,
        }
    }

    /// Série de juin 2025 : quatre tirages au plafond (excédents réels de 17 à 30 M€), gagné au quatrième.
    fn june_2025(won_at_fourth: bool) -> Vec<Draw> {
        vec![
            draw(day(2025, 6, 3), 42e6, 0.1e6, false),
            draw(day(2025, 6, 6), 61e6, 17.4e6, false),
            draw(day(2025, 6, 10), 43e6, 22.4e6, false),
            draw(day(2025, 6, 13), 58e6, 29.8e6, false),
            draw(day(2025, 6, 17), 51e6, 22.0e6, won_at_fourth),
        ]
    }

    #[test]
    fn the_excess_of_a_cap_draw_is_measured() {
        let state = check(&draw(day(2025, 6, 13), 58e6, 29.8e6, false)).unwrap();
        assert_relative_eq!(state.excess, 29.8e6, max_relative = 0.01);
        assert!(state.at_cap() && !state.rolled_down());
        assert!(
            !check(&draw(day(2025, 6, 3), 42e6, 0.1e6, false))
                .unwrap()
                .at_cap()
        );
    }

    #[test]
    fn a_won_fourth_draw_raises_no_alert() {
        let draws = june_2025(true);
        assert_eq!(
            pending(&draws).unwrap(),
            None,
            "série terminée par un gagnant"
        );
        // Après le troisième tirage au plafond : veille.
        let watch = pending(&draws[..4]).unwrap().unwrap();
        assert_eq!(watch.level, Level::Watch);
        assert_eq!(watch.next_draw, day(2025, 6, 17));
    }

    #[test]
    fn an_unwon_fourth_draw_raises_the_fifth_alert() {
        let alert = pending(&june_2025(false)).unwrap().unwrap();
        assert_eq!(alert.level, Level::Fifth);
        assert_eq!(alert.streak, 4);
        assert_eq!(alert.draw_date, day(2025, 6, 17));
        assert_eq!(alert.next_draw, day(2025, 6, 20));
        assert!(alert.message.contains("cinquième"), "{}", alert.message);
    }

    #[test]
    fn a_rolldown_ends_the_series() {
        let mut draws = june_2025(false);
        draws.push(draw(day(2025, 6, 20), 90e6, 250e6, false));
        assert_eq!(pending(&draws).unwrap(), None);
    }

    #[test]
    fn a_changed_matrix_is_an_error() {
        let mut draws = june_2025(false);
        let last = draws.last_mut().unwrap();
        last.ranks[11].winners *= 3;
        assert!(pending(&draws).is_err());
    }

    #[test]
    fn the_forced_value_matches_the_study() {
        // §43 : 70 millions de grilles → environ 4,0 € (250 M€ / 70 M + 0,44 €).
        assert_relative_eq!(forced_value(70e6), 250e6 / 70e6 + 0.44, epsilon = 1e-9);
        assert_relative_eq!(lower_ranks_per_grid(), 0.44, epsilon = 1e-9);
    }

    #[test]
    fn the_next_draw_is_a_tuesday_or_a_friday() {
        assert_eq!(next_draw(day(2026, 10, 20)), day(2026, 10, 23));
        assert_eq!(next_draw(day(2026, 10, 23)), day(2026, 10, 27));
    }
}
