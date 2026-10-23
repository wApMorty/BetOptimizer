//! Onglet « Grand Loto » (`/loto`) : prochaine édition, séance de l'édition (grilles à jouer), grilles jouées et leur
//! règlement, série EuroMillions au plafond, Grand Loto passés.
//!
//! Lecture seule. Aucun calcul du modèle n'est refait à l'affichage : la séance est lue dans le journal (`loto_session`
//! et ses notes JSON) telle qu'elle a été décidée par `edge loto recommend`. Deux formulaires écrivent, par
//! [`crate::web::actions`] : « J'ai joué la séance » et la déclaration d'une grille saisie à la main.

use chrono::{DateTime, Days, NaiveDate, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;
use serde_json::Value;

use crate::config::GrandLotoEdition;
use crate::error::Result;
use crate::format;
use crate::loto::played::{self, Settled};
use crate::loto::{Game, Grid, alert};
use crate::report::loto_report::{EditionRow, edition_rows};
use crate::store::{self, loto as loto_store};
use crate::web::App;
use crate::web::view::{Flash, Layout, euros, render};

const TEMPLATE: &str = include_str!("../templates/loto.html");

/// La prochaine édition saisie dans `edge.toml`.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct NextView {
    date: String,
    jackpot: String,
    price: String,
    /// `dans 63 jours`, `aujourd'hui`.
    until: String,
    /// Soir de la séance automatique (`mardi 22/12/2026 à 22 h 30`).
    session_on: String,
    /// La séance de cette édition est déjà écrite.
    done: bool,
}

/// Une grille évaluée par la séance.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct GridView {
    /// Numéro de la recommandation (grilles retenues seulement).
    id: Option<i64>,
    balls: Vec<u8>,
    chance: u8,
    ev: String,
    ev_pessimistic: String,
    popularity: String,
    kept: bool,
    played: bool,
    /// Règlement : `à venir`, `rang 8 · 36,20 €`, `perdue`.
    status: String,
    tone: &'static str,
}

/// La séance affichée.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct SessionView {
    date: String,
    created: String,
    central: String,
    pessimistic: String,
    quantile: String,
    editions: usize,
    season: String,
    uniform: String,
    uniform_pessimistic: String,
    threshold: String,
    preferences_validated: bool,
    gain: String,
    grids: Vec<GridView>,
    play: bool,
}

/// Une grille déclarée jouée.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct PlayView {
    date: String,
    grid: String,
    stake: String,
    status: String,
    tone: &'static str,
    off_model: bool,
}

/// L'EuroMillions au plafond.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct EuroView {
    /// Dernier tirage relevé.
    last: Option<String>,
    /// Tirages au plafond sans gagnant qui terminent l'archive.
    streak: usize,
    text: String,
    tone: &'static str,
    /// Alertes déjà envoyées (`20/10/2026 · cinquième`).
    alerts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Page {
    #[serde(flatten)]
    layout: Layout,
    next: Option<NextView>,
    session: Option<SessionView>,
    to_declare: usize,
    plays: Vec<PlayView>,
    /// `4 grilles · 20,00 € misés · 36,20 € gagnés`.
    totals: String,
    off_model_totals: Option<String>,
    euro: EuroView,
    editions: Vec<EditionRow>,
    /// Éditions proposées dans le formulaire : (valeur `AAAA-MM-JJ`, libellé).
    edition_choices: Vec<(String, String)>,
    default_price: String,
}

/// `20 M€`, ou `13,5 M€` si le montant n'est pas un nombre entier de millions.
fn millions_of_euros(amount: f64) -> String {
    let millions = amount / 1e6;
    let digits = if millions.fract() == 0.0 { 0 } else { 1 };
    format!("{} M€", format::decimal(millions, digits))
}

fn day(date: NaiveDate) -> String {
    date.format("%d/%m/%Y").to_string()
}

/// La page.
pub fn page(app: &App, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let today = now.with_timezone(&Paris).date_naive();
    let sessions = loto_store::sessions(&conn)?;
    let next_edition = app
        .config
        .grand_loto_editions
        .iter()
        .filter(|e| e.date >= today)
        .min_by_key(|e| e.date)
        .copied();
    let next = next_edition.map(|edition| NextView {
        date: day(edition.date),
        jackpot: millions_of_euros(edition.jackpot),
        price: euros(edition.price, 2),
        until: match (edition.date - today).num_days() {
            0 => "aujourd'hui".to_string(),
            1 => "demain".to_string(),
            n => format!("dans {n} jours"),
        },
        session_on: session_evening(&edition, app.config.grand_loto_days_before),
        done: sessions.iter().any(|s| s.draw_date == edition.date),
    });

    // La séance de la prochaine édition si elle existe, sinon la plus récente.
    let shown = next_edition
        .and_then(|e| sessions.iter().find(|s| s.draw_date == e.date))
        .or_else(|| sessions.last());
    let ledger = played::ledger(&conn)?;
    let recos = loto_store::recos(&conn)?;
    let session = shown.map(|s| {
        let ids: Vec<(i64, Grid)> = recos
            .iter()
            .filter(|r| r.session_id == s.id)
            .map(|r| (r.id, r.grid))
            .collect();
        session_view(s, &ids, &ledger)
    });
    let to_declare = session
        .as_ref()
        .map(|s| s.grids.iter().filter(|g| g.kept && !g.played).count())
        .unwrap_or(0);

    let plays = ledger
        .rows
        .iter()
        .rev()
        .map(|r| {
            let (status, tone) = settlement(r);
            PlayView {
                date: day(r.play.draw_date),
                grid: r.play.grid.to_string(),
                stake: euros(r.play.stake, 2),
                status,
                tone,
                off_model: r.play.reco_id.is_none(),
            }
        })
        .collect();
    let totals = totals_text(ledger.totals(false));
    let off_model = ledger.totals(true);
    let off_model_totals = (off_model.2 > 0).then(|| totals_text(off_model));

    let mut choices: Vec<NaiveDate> = app
        .config
        .grand_loto_editions
        .iter()
        .map(|e| e.date)
        .chain(sessions.iter().map(|s| s.draw_date))
        .collect();
    choices.sort_unstable_by(|a, b| b.cmp(a));
    choices.dedup();
    // La prochaine édition d'abord (choix par défaut du formulaire), puis les autres, de la plus récente à la plus ancienne.
    if let Some(next) = next_edition {
        choices.retain(|d| *d != next.date);
        choices.insert(0, next.date);
    }

    render(
        "loto.html",
        TEMPLATE,
        &Page {
            layout: Layout::new("loto", now, flash),
            next,
            session,
            to_declare,
            plays,
            totals,
            off_model_totals,
            euro: euro_view(&conn)?,
            editions: edition_rows(&loto_store::draws(&conn, Game::GrandLoto)?),
            edition_choices: choices
                .into_iter()
                .map(|d| (d.format("%Y-%m-%d").to_string(), day(d)))
                .collect(),
            default_price: format::decimal(crate::loto::rules::GRAND_LOTO_PRICE, 0),
        },
    )
}

/// Soir de la séance automatique : `grand_loto_days_before` jours avant l'édition, au passage du timer de 22 h 30.
fn session_evening(edition: &GrandLotoEdition, days_before: u32) -> String {
    let evening = edition
        .date
        .checked_sub_days(Days::new(u64::from(days_before)))
        .unwrap_or(edition.date);
    // `long_date` attend un instant : midi UTC tombe le même jour à Paris.
    let noon = evening.and_hms_opt(12, 0, 0).map(|t| t.and_utc());
    format!(
        "{} à 22 h 30",
        noon.map(format::long_date).unwrap_or_else(|| day(evening))
    )
}

/// La séance, d'après ses notes JSON. Une note absente s'affiche « — » plutôt que de faire échouer la page.
fn session_view(
    session: &loto_store::SessionRow,
    ids: &[(i64, Grid)],
    ledger: &played::Ledger,
) -> SessionView {
    let notes: Value = serde_json::from_str(&session.notes).unwrap_or(Value::Null);
    let params: Value = serde_json::from_str(&session.params).unwrap_or(Value::Null);
    let number = |v: &Value| v.as_f64();
    let millions = |v: &Value| {
        number(v).map_or("—".to_string(), |n| {
            format!("{} millions", format::decimal(n / 1e6, 2))
        })
    };
    let money = |v: &Value| number(v).map_or("—".to_string(), |n| euros(n, 2));
    let forecast = &notes["forecast"];
    let season = match forecast["season"].as_str() {
        Some("Summer") => "été",
        Some("Christmas") => "Noël",
        Some(_) => "autre",
        None => "—",
    };
    let gain = &notes["walk_forward"]["gain"];
    let grids: Vec<GridView> = notes["candidates"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let grid = Grid::parse(c["grid"].as_str()?)?;
                    let kept = c["kept"].as_bool().unwrap_or(false);
                    let id = ids.iter().find(|(_, g)| *g == grid).map(|(id, _)| *id);
                    let settled = ledger
                        .rows
                        .iter()
                        .find(|r| id.is_some() && r.play.reco_id == id);
                    let (status, tone) = match settled {
                        Some(row) => settlement(row),
                        None if kept => ("pas déclarée".to_string(), "t-warn"),
                        None => ("écartée".to_string(), ""),
                    };
                    Some(GridView {
                        id,
                        balls: grid.balls.to_vec(),
                        chance: grid.chance,
                        ev: money(&c["ev"]),
                        ev_pessimistic: money(&c["ev_pessimistic"]),
                        popularity: number(&c["popularity"])
                            .map_or("—".to_string(), |p| format::decimal(p, 2)),
                        kept,
                        played: settled.is_some(),
                        status,
                        tone,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    SessionView {
        date: day(session.draw_date),
        created: session
            .created_at
            .with_timezone(&Paris)
            .format("%d/%m/%Y %H:%M")
            .to_string(),
        central: millions(&forecast["central"]),
        pessimistic: millions(&forecast["pessimistic"]),
        quantile: number(&forecast["quantile"])
            .map_or("—".to_string(), |q| format!("{:.0}e centile", q * 100.0)),
        editions: forecast["editions"].as_array().map_or(0, Vec::len),
        season: season.to_string(),
        uniform: money(&notes["uniform_value"]["central"]),
        uniform_pessimistic: money(&notes["uniform_value"]["pessimistic"]),
        threshold: number(&params["threshold"])
            .map_or("—".to_string(), |t| format::decimal(t, 2)),
        preferences_validated: params["preferences"].as_str() == Some("Validated"),
        gain: match (
            number(&gain["mean"]),
            number(&gain["low"]),
            number(&gain["high"]),
        ) {
            (Some(m), Some(l), Some(h)) => format!(
                "{} [{} ; {}]",
                format::decimal(m, 0),
                format::decimal(l, 0),
                format::decimal(h, 0)
            ),
            _ => "—".to_string(),
        },
        play: grids.iter().any(|g| g.kept),
        grids,
    }
}

/// Règlement d'une grille jouée, et sa tonalité.
fn settlement(row: &Settled) -> (String, &'static str) {
    match (row.gain, row.rank) {
        (None, _) => ("à venir".to_string(), ""),
        (Some(gain), Some(rank)) => (format!("rang {rank} · {}", euros(gain, 2)), "t-pos"),
        (Some(_), None) => ("perdue".to_string(), "t-neg"),
    }
}

fn totals_text((staked, won, count): (f64, f64, usize)) -> String {
    format!(
        "{count} grille(s) réglée(s) · {} misés · {} gagnés",
        euros(staked, 2),
        euros(won, 2)
    )
}

/// Série en cours à l'EuroMillions et alertes déjà envoyées.
fn euro_view(conn: &rusqlite::Connection) -> Result<EuroView> {
    let draws = loto_store::draws(conn, Game::EuroMillions)?;
    let alerts = loto_store::alerts(conn)?
        .into_iter()
        .map(|(_, date, level)| {
            let label = if level == "fifth" {
                "cinquième tirage"
            } else {
                "veille"
            };
            format!("{} · {label}", day(date))
        })
        .collect();
    Ok(match alert::current_streak(&draws) {
        Err(error) => EuroView {
            last: None,
            streak: 0,
            text: error.to_string(),
            tone: "t-neg",
            alerts,
        },
        Ok((streak, last)) => {
            let (text, tone) = match streak.len() {
                0 => ("Aucune série au plafond en cours.".to_string(), "t-pos"),
                n @ 1..=2 => (
                    format!("{n} tirage(s) au plafond sans gagnant : la veille commence au troisième."),
                    "t-warn",
                ),
                3 => (
                    "Trois tirages au plafond sans gagnant : le prochain est le quatrième, ou le cinquième (vérifier sur fdj.fr)."
                        .to_string(),
                    "t-warn",
                ),
                _ => (
                    "Quatre tirages au plafond sans gagnant : le prochain est le cinquième, jackpot versé au rang 2 s'il n'est pas gagné."
                        .to_string(),
                    "t-neg",
                ),
            };
            EuroView {
                last: last.map(day),
                streak: streak.len(),
                text,
                tone,
                alerts,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::web::testing::{empty, grand_loto_edition, sample};

    #[test]
    fn the_session_shows_its_grids_and_the_play_button() {
        let sample = sample();
        let html = sample.html("/loto");
        // Les numéros de la première grille de l'exemple, en boules.
        for ball in ["35", "36", "39", "40", "46"] {
            assert!(
                html.contains(&format!("<span class=\"ball\">{ball}</span>")),
                "{html}"
            );
        }
        assert!(html.contains("J'ai joué la séance"), "{html}");
        assert!(html.contains("pas déclarée"), "{html}");
        assert!(html.contains("action=\"/loto/grille\""), "{html}");
        let date = grand_loto_edition(sample.now)
            .date
            .format("%d/%m/%Y")
            .to_string();
        assert!(
            html.contains(&format!("Séance du Grand Loto du {date}")),
            "{html}"
        );
        assert!(html.contains("écartée"), "{html}");
        assert!(html.contains("20 M€"), "{html}");
    }

    #[test]
    fn reading_the_page_writes_nothing() {
        let sample = sample();
        let count = || {
            let conn = crate::store::open_read_only(&sample.app.paths.db_file).unwrap();
            let plays = crate::store::loto::plays(&conn).unwrap().len();
            let sessions = crate::store::loto::sessions(&conn).unwrap().len();
            (plays, sessions)
        };
        let before = count();
        sample.html("/loto");
        assert_eq!(count(), before);
    }

    #[test]
    fn an_empty_base_shows_empty_states() {
        let html = empty().html("/loto");
        assert!(html.contains("Aucune séance pour l'instant"), "{html}");
        assert!(html.contains("Aucune édition à venir"), "{html}");
        assert!(html.contains("Aucune série au plafond"), "{html}");
    }
}
