//! Loto Foot (phase 14) : page `/lotofoot` de l'application Web et message Discord de la séance du samedi.
//!
//! Les calculs viennent de [`crate::lotofoot`] (règlement, vérification), comme `edge lotofoot report` : ce module les
//! met en forme pour le gabarit `lotofoot.html` de l'application et pour le texte du message.

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;
use crate::format::{chance, decimal, kickoff_label, long_date, rough_euros};
use crate::lotofoot::alert::Listed;
use crate::lotofoot::bulletin::{Line, session_lines};
use crate::lotofoot::model::{FROZEN, GridModel, RankOdds};
use crate::lotofoot::oddspapi;
use crate::lotofoot::outlets::Outlet;
use crate::lotofoot::price::{self, PriceTable};
use crate::lotofoot::session::{self, MonthBudget, SessionReport};
use crate::lotofoot::settle::{self, Ledger};
use crate::lotofoot::verify::{self, Verdict, Verification};
use crate::lotofoot::{GridId, TICKET_PRICE, sign_char};
use crate::store::lotofoot::{self as store, RecoRow};

/// Un ticket recommandé, prêt à afficher.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TicketView {
    /// Numéro du ticket au journal (pour `edge lotofoot played`).
    pub id: i64,
    /// Grille (« LF8 n°146 »).
    pub grid: String,
    /// Clôture, heure de Paris.
    pub closes: String,
    /// Signes (`21N12222`).
    pub ticket: String,
    /// Espérance du modèle de base.
    pub ev: String,
    /// Espérance du scénario défavorable.
    pub ev_low: String,
    /// Probabilité d'un rang payé.
    pub paid: String,
    /// Chance d'être payé, quel que soit le rang (« 1 sur 230 »).
    pub chance: String,
    /// Chance et gain moyen de chaque rang, du sans-faute au dernier rang payé (vide si les entrées du modèle manquent).
    pub ranks: Vec<RankView>,
    /// Règlement (« en attente », « perdu », « rang 2 : 21,90 € »).
    pub status: String,
    /// Déjà déclaré joué (`edge lotofoot played`).
    pub played: bool,
    /// Marge de l'espérance défavorable au-dessus du seuil de sa séance (« +12 % »), vide si le seuil n'est pas noté.
    pub margin: String,
    /// Ticket limite : marge sous [`limit_margin`] du délai entre sa séance et la clôture ; il peut passer sous le seuil.
    pub limit: bool,
    /// Marge sous laquelle ce ticket serait limite (« 7,9 % »), vide si le seuil n'est pas noté.
    pub limit_margin: String,
}

/// Dérive par racine d'heure : une fois sur dix, un ticket limite perd au moins 4,4 % de son espérance en une heure (deux
/// premières grilles relevées, `docs/sources.md` §54). Valeur provisoire, choisie par Paul le 2026-10-25, à revoir avec l'étude
/// de dérive (`scripts/lotofoot/PROTOCOLE_DERIVE.md`).
pub const DRIFT_PER_SQRT_HOUR: f64 = 0.044;

/// Marge sous laquelle un ticket choisi `hours` heures avant la clôture est signalé « limite » :
/// `marge(h) = exp(0,044 · √h) − 1` (dérive supposée au hasard, heure après heure : son écart grandit comme `√h`).
/// 4,5 % à 1 h, 7,9 % à 3 h, 14 % à 9 h, 24 % à 24 h. `exp_m1` calcule `e^x − 1` sans perte de précision pour `x` petit.
pub fn limit_margin(hours: f64) -> f64 {
    (DRIFT_PER_SQRT_HOUR * hours.max(0.0).sqrt()).exp_m1()
}

/// Marge d'une espérance défavorable au-dessus d'un seuil (« +12 % ») et ticket limite ou non, pour un ticket choisi `hours`
/// heures avant la clôture.
pub fn margin(ev_pessimistic: f64, threshold: f64, hours: f64) -> (String, bool) {
    let m = ev_pessimistic / threshold - 1.0;
    let sign = if m < 0.0 { "−" } else { "+" };
    (
        format!("{sign}{} %", decimal(m.abs() * 100.0, 0)),
        m < limit_margin(hours),
    )
}

/// Seuil τ noté dans les paramètres d'une séance d'avant le prix dynamique (`None` pour une séance qui ne l'a pas noté).
fn session_threshold(conn: &Connection, session_id: i64) -> Result<Option<f64>> {
    Ok(store::session_params(conn, session_id)?
        .and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
        .and_then(|p| p.get("threshold").and_then(|t| t.as_f64())))
}

/// Valeur et seuil d'un ticket : ceux de sa sélection, écrits dans ses entrées depuis le §70 ; sinon son espérance
/// défavorable et le τ de sa séance.
fn ticket_bar(reco: &RecoRow, session: Option<f64>) -> Option<(f64, f64)> {
    let inputs = serde_json::from_str::<serde_json::Value>(&reco.inputs).ok();
    let read = |key: &str| inputs.as_ref()?.get(key)?.as_f64();
    match (read("marginal"), read("threshold")) {
        (Some(value), Some(threshold)) => Some((value, threshold)),
        _ => session.map(|threshold| (reco.ev_pessimistic, threshold)),
    }
}

/// Un rang payé d'un ticket ou d'une grille : ce qu'il faut réussir, la chance d'y arriver, le gain moyen quand on y arrive.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankView {
    /// « Sans faute », « Une erreur ».
    pub label: &'static str,
    /// « 1 sur 2 400 ».
    pub chance: String,
    /// « ≈ 3 150 € ».
    pub gain: String,
}

/// Une grille du Loto Foot à jouer ou en cours, avec l'ordre de ses rencontres (celui du bulletin).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GridView {
    /// Grille.
    pub grid: String,
    /// Formule et numéro, pour le bouton « J'ai joué cette grille » (`lf7-133`).
    pub key: String,
    /// Clôture, heure de Paris.
    pub closes: String,
    /// Séance qui a choisi ses tickets (« séance n°11, samedi 24 octobre »).
    pub session: String,
    /// Rencontres numérotées.
    pub matches: Vec<String>,
    /// Toutes les grilles à remplir ensemble, selon le meilleur résultat de l'ensemble ([`GridTotal`]).
    pub total: GridTotal,
}

/// Cumul d'une grille du Loto Foot sur tous les tickets du groupe ([`GridModel::lines_odds`]) : chaque rang compte les
/// issues où notre meilleur ticket l'atteint, les rangs s'excluent donc et leurs chances s'additionnent.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GridTotal {
    /// Mise (« 16 € »).
    pub price: String,
    /// Chance et gain moyen de chaque rang, gain de tous nos tickets compris (vide si les entrées du modèle manquent).
    pub ranks: Vec<RankView>,
    /// Chance d'au moins un rang payé (« 1 sur 8 »), vide si les entrées du modèle manquent.
    pub paid: String,
}

/// Un groupe de tickets : ceux à jouer (grille ouverte, pas encore déclarés joués) ou ceux en cours (joués, résultats
/// pas tous connus). Un ticket joué dont la grille a tous ses résultats n'est plus suivi : il ne compte que dans le bilan.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Group {
    /// Grilles du groupe, par clôture.
    pub grids: Vec<GridView>,
    /// Tickets.
    pub tickets: Vec<TicketView>,
    /// Les mêmes tickets regroupés en grilles à remplir.
    pub lines: Vec<LineView>,
    /// Mise du groupe (« 16 € »).
    pub stake: String,
}

/// Le budget du mois en cours et le seuil qu'il donne.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetView {
    /// Budget mensuel en vigueur (euros).
    pub monthly: u32,
    /// Seuil du moment selon le temps et le budget restants (« 1,84 »), vide si le budget est épuisé.
    pub threshold: String,
    /// Mois en cours (« octobre 2026 »).
    pub month: String,
    /// Tickets en vigueur des grilles qui ferment ce mois (depuis l'entrée en vigueur du budget).
    pub spent: u32,
    /// Gains recyclés dans ce mois (euros entiers, §67).
    pub recycled: u32,
    /// Ce qui reste.
    pub left: u32,
}

/// Une grille à remplir au guichet : des tickets de la séance regroupés sans perte en une grille simple ou multiple
/// ([`crate::lotofoot::bulletin`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineView {
    /// Grille du Loto Foot (« LF8 n°146 »).
    pub grid: String,
    /// Signes à cocher pour chaque rencontre, dans l'ordre du bulletin (`["1", "N"]` pour un double 1N).
    pub signs: Vec<Vec<String>>,
    /// Forme écrite (`1N 2 2 1 N2 1N2 1`).
    pub code: String,
    /// Prix (« 8 € ») : nombre de tickets contenus.
    pub price: String,
    /// Numéros au journal des tickets contenus.
    pub tickets: Vec<i64>,
    /// Chance et gain moyen de la grille selon son meilleur résultat : sans faute, puis une erreur (vide si les entrées
    /// du modèle manquent).
    pub ranks: Vec<RankView>,
    /// Tous ses tickets sont déclarés joués.
    pub played: bool,
}

/// Un bilan affiché.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LedgerView {
    /// Libellé.
    pub label: &'static str,
    /// Tickets.
    pub tickets: usize,
    /// Tickets réglés.
    pub settled: usize,
    /// Mises réglées.
    pub staked: String,
    /// Gains.
    pub returned: String,
    /// Rendement.
    pub roi: String,
    /// Tickets payés observés / attendus.
    pub paid: String,
}

/// Une tranche de la calibration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BandView {
    /// Tranche de λ.
    pub range: String,
    /// Observations.
    pub observations: usize,
    /// Gagnants publiés / prédits.
    pub ratio: String,
    /// Tranche jugée et hors bornes.
    pub failing: bool,
}

/// La page `/lotofoot`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LotofootPage {
    /// Budget du mois et seuil.
    pub budget: BudgetView,
    /// Tickets à jouer : grille ouverte, pas encore déclarés joués.
    pub to_play: Group,
    /// Tickets en cours : joués, résultats pas tous connus.
    pub in_play: Group,
    /// Grilles ouvertes ou suivies dont la composition a changé (`docs/sources.md` §52), en clair.
    pub recomposed: Vec<String>,
    /// Dernière séance (« n°11, samedi 24 octobre à 17:34 »), vide s'il n'y en a pas.
    pub session: String,
    /// Seuil et budget de la dernière séance (« τ = 1,7, budget de 60 € par mois »), vide s'ils ne sont pas notés.
    pub session_rule: String,
    /// Grilles évaluées par la dernière séance (« LF8 n°147 : meilleur ticket à 2,25 »).
    pub evaluated: Vec<String>,
    /// Grilles écartées par la dernière séance (« LF7 n°133 : répartition pas encore publiée »).
    pub skipped: Vec<String>,
    /// Noms d'équipes inconnus.
    pub unknown_names: Vec<String>,
    /// Rencontres de la dernière séance dont la cote Pinnacle vient du repli OddsPapi.
    pub oddspapi_found: Vec<String>,
    /// Refus et pistes du repli OddsPapi de la dernière séance.
    pub oddspapi_issues: Vec<String>,
    /// Bilans : recommandés (suivi du modèle), joués du modèle, joués hors modèle.
    pub ledgers: Vec<LedgerView>,
    /// Verdict en clair.
    pub verdict: String,
    /// Classe CSS du verdict.
    pub verdict_class: &'static str,
    /// Calibration des concurrents.
    pub bands: Vec<BandView>,
    /// Gain réalisé contre simulé.
    pub gain: String,
    /// Fréquence des tickets payés.
    pub frequency: String,
}

/// Construit la page à partir du journal ; `monthly_budget` est le budget en vigueur ([`crate::lotofoot::session::monthly_budget`]).
pub fn page(conn: &Connection, now: DateTime<Utc>, monthly_budget: u32) -> Result<LotofootPage> {
    let verification = verify::verify(conn, now)?;
    let recommended = settle::recommended(conn)?;
    let (played_model, played_off) = settle::played_ledgers(conn)?;
    let recos = store::current_recos(conn)?;
    let followed = followed(conn, now)?;
    let to_play: Vec<&RecoRow> = followed.to_play.iter().collect();
    let in_play: Vec<&RecoRow> = followed.in_play.iter().collect();
    let played = followed.played;
    let mut page = LotofootPage {
        budget: budget_view(conn, &recos, now, monthly_budget)?,
        to_play: group(conn, &to_play, &played)?,
        in_play: group(conn, &in_play, &played)?,
        recomposed: recomposed_notices(conn, now, &in_play)?,
        session: String::new(),
        session_rule: String::new(),
        evaluated: Vec::new(),
        skipped: Vec::new(),
        unknown_names: Vec::new(),
        oddspapi_found: Vec::new(),
        oddspapi_issues: Vec::new(),
        ledgers: vec![
            ledger_view(
                "Tickets recommandés (suivi du modèle, joués ou non)",
                &recommended,
                true,
            ),
            ledger_view("Tickets joués du modèle", &played_model, false),
            ledger_view("Tickets joués hors modèle", &played_off, false),
        ],
        verdict: verdict_text(&verification),
        verdict_class: verdict_class(&verification.verdict),
        bands: verification
            .bands
            .iter()
            .filter(|b| b.observations > 0)
            .map(|b| BandView {
                range: if b.high.is_finite() {
                    format!("{} – {}", b.low, b.high)
                } else {
                    format!("≥ {}", b.low)
                },
                observations: b.observations,
                ratio: b.ratio().map_or("—".into(), |r| decimal(r, 2)),
                failing: b.failing(),
            })
            .collect(),
        gain: format!(
            "{} € réalisés pour {} € attendus ; centile {} de la loi simulée ({} ticket(s) réglé(s))",
            decimal(verification.gain.realized, 2),
            decimal(verification.gain.simulated_mean, 2),
            decimal(verification.gain.percentile, 3),
            verification.gain.settled
        ),
        frequency: format!(
            "{} payé(s) pour {} attendu(s) ; probabilité d'en avoir aussi peu : {}",
            verification.frequency.observed,
            decimal(verification.frequency.expected, 2),
            decimal(verification.frequency.p_low, 3)
        ),
    };
    let Some((id, created_at, notes)) = store::last_session(conn)? else {
        return Ok(page);
    };
    page.session = format!("n°{id}, {} à {}", long_date(created_at), clock(created_at));
    if let Some(params) = store::session_params(conn, id)?
        && let Ok(params) = serde_json::from_str::<serde_json::Value>(&params)
        && let Some(budget) = params.get("monthly_budget").and_then(|b| b.as_u64())
    {
        page.session_rule = match params.get("threshold").and_then(|t| t.as_f64()) {
            Some(tau) => format!("τ = {}, budget de {budget} € par mois", decimal(tau, 1)),
            None => format!("prix dynamique, budget de {budget} € par mois"),
        };
    }
    if let Ok(notes) = serde_json::from_str::<serde_json::Value>(&notes) {
        let list = |key: &str| -> Vec<serde_json::Value> {
            notes
                .get(key)
                .and_then(|s| s.as_array())
                .cloned()
                .unwrap_or_default()
        };
        page.evaluated = list("evaluated")
            .iter()
            .filter_map(|e| {
                Some(format!(
                    "{} : meilleur ticket à {}",
                    e.get("grid")?.as_str()?,
                    decimal(e.get("best_pessimistic")?.as_f64()?, 2)
                ))
            })
            .collect();
        page.skipped = list("skipped")
            .iter()
            .filter_map(|s| {
                Some(format!(
                    "{} : {}",
                    s.get("grid")?.as_str()?,
                    s.get("reason")?.as_str()?
                ))
            })
            .collect();
        let texts = |key: &str| -> Vec<String> {
            list(key)
                .iter()
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect()
        };
        page.unknown_names = texts("unknown_names");
        page.oddspapi_found = texts("oddspapi_found");
        page.oddspapi_issues = texts("oddspapi_refused");
        page.oddspapi_issues.extend(texts("oddspapi_hints"));
    }
    Ok(page)
}

/// Tickets suivis par la page : à jouer (grille ouverte, pas déclarés joués) et en cours (joués, résultats pas tous
/// connus). Les autres (joués et finis, ou jamais joués d'une grille close) ne comptent plus que dans le bilan.
pub struct Followed {
    /// À jouer, par séance puis position.
    pub to_play: Vec<RecoRow>,
    /// En cours.
    pub in_play: Vec<RecoRow>,
    /// Numéros des recommandations déclarées jouées.
    pub played: Vec<i64>,
}

/// Répartit les tickets en vigueur entre « à jouer » et « en cours » ([`Followed`]).
pub fn followed(conn: &Connection, now: DateTime<Utc>) -> Result<Followed> {
    let played: Vec<i64> = store::plays(conn)?
        .iter()
        .filter_map(|p| p.reco_id)
        .collect();
    let mut out = Followed {
        to_play: Vec::new(),
        in_play: Vec::new(),
        played: Vec::new(),
    };
    for reco in store::current_recos(conn)? {
        if played.contains(&reco.id) {
            if store::results(conn, reco.grid)?.is_none() {
                out.in_play.push(reco);
            }
        } else if store::grid(conn, reco.grid)?.is_some_and(|g| g.closes_at > now) {
            out.to_play.push(reco);
        }
    }
    out.played = played;
    Ok(out)
}

/// Avis des grilles dont la composition a changé : celles qui ferment après `now`, et celles des tickets en cours.
fn recomposed_notices(
    conn: &Connection,
    now: DateTime<Utc>,
    in_play: &[&RecoRow],
) -> Result<Vec<String>> {
    let mut keys: Vec<GridId> = store::grids_closing_after(conn, now)?
        .iter()
        .map(|g| g.id)
        .collect();
    for reco in in_play {
        if !keys.contains(&reco.grid) {
            keys.push(reco.grid);
        }
    }
    let mut notices = Vec::new();
    for id in keys {
        let Some(change) = store::recomposition(conn, id)? else {
            continue;
        };
        let when = format!("{} à {}", long_date(change.at), clock(change.at));
        notices.push(if change.played_before {
            format!(
                "{id} : rencontres changées ({when}) après des tickets déclarés joués. Grille bloquée : \
                 ni recalcul ni règlement automatique, à régler à la main."
            )
        } else {
            format!(
                "{id} : rencontres changées ({when}). Répartitions et tickets calculés avant sont écartés ; \
                 la prochaine séance la recalcule sur sa nouvelle composition."
            )
        });
    }
    Ok(notices)
}

/// « 17:34 », heure de Paris.
fn clock(at: DateTime<Utc>) -> String {
    at.with_timezone(&chrono_tz::Europe::Paris)
        .format("%H:%M")
        .to_string()
}

/// Budget du mois en cours : tickets en vigueur des grilles qui ferment ce mois, recommandés depuis l'entrée en vigueur.
fn budget_view(
    conn: &Connection,
    recos: &[RecoRow],
    now: DateTime<Utc>,
    monthly: u32,
) -> Result<BudgetView> {
    let month = session::month_of(now);
    let since = session::budget_since();
    let mut spent = 0u32;
    for reco in recos.iter().filter(|r| r.created_at >= since) {
        let closes = store::grid(conn, reco.grid)?.map(|g| g.closes_at);
        if closes.is_some_and(|at| session::month_of(at) == month) {
            spent += 1;
        }
    }
    let recycled = settle::recycled_gains(conn, recos, month, now)?;
    let left = (monthly + recycled).saturating_sub(spent);
    let steps = price::MONTH_STEPS * session::month_left(now);
    Ok(BudgetView {
        monthly,
        threshold: if left > 0 {
            decimal(PriceTable::frozen().threshold(steps, left), 2)
        } else {
            String::new()
        },
        month: month_label(month),
        spent,
        recycled,
        left,
    })
}

/// Un groupe de tickets prêt à afficher : grilles par clôture, tickets, grilles à remplir.
fn group(conn: &Connection, recos: &[&RecoRow], played: &[i64]) -> Result<Group> {
    let mut out = Group::default();
    let mut seen: Vec<GridId> = Vec::new();
    // Seuil de chaque séance, lu une fois par séance.
    let mut thresholds: Vec<(i64, Option<f64>)> = Vec::new();
    for reco in recos {
        let threshold = match thresholds.iter().find(|(id, _)| *id == reco.session_id) {
            Some((_, t)) => *t,
            None => {
                let t = session_threshold(conn, reco.session_id)?;
                thresholds.push((reco.session_id, t));
                t
            }
        };
        let grid = store::grid(conn, reco.grid)?;
        // Délai entre la séance et la clôture : c'est la dérive possible pendant ce temps qui menace le ticket.
        let hours = grid.as_ref().map_or(1.0, |g| {
            (g.closes_at - reco.created_at).num_minutes() as f64 / 60.0
        });
        let bar = ticket_bar(reco, threshold);
        let (margin, limit) = bar.map_or((String::new(), false), |(value, threshold)| {
            margin(value, threshold, hours)
        });
        let limit_margin = bar.map_or(String::new(), |_| {
            format!("{} %", decimal(limit_margin(hours) * 100.0, 1))
        });
        let closes = grid
            .as_ref()
            .map(|g| kickoff_label(g.closes_at))
            .unwrap_or_default();
        if !seen.contains(&reco.grid) {
            seen.push(reco.grid);
            out.grids.push(GridView {
                grid: reco.grid.to_string(),
                key: reco.grid.code(),
                closes: closes.clone(),
                session: format!(
                    "séance n°{}, {} à {}",
                    reco.session_id,
                    long_date(reco.created_at),
                    clock(reco.created_at)
                ),
                matches: grid
                    .as_ref()
                    .map(|g| {
                        g.matches
                            .iter()
                            .enumerate()
                            .map(|(i, (h, a))| format!("{}. {h} – {a}", i + 1))
                            .collect()
                    })
                    .unwrap_or_default(),
                total: grid_total(recos, reco.grid),
            });
        }
        let status = match store::results(conn, reco.grid)? {
            None => "en attente".to_string(),
            Some(results) => {
                let ranks = store::ranks(conn, reco.grid)?;
                let jackpot = grid.as_ref().and_then(|g| g.jackpot);
                match settle::paper(
                    reco.grid.game,
                    &reco.ticket,
                    &results,
                    &ranks,
                    reco.stakes_estimate,
                    jackpot,
                ) {
                    None => "rapports en attente".to_string(),
                    Some(s) => match s.rank {
                        None => format!("perdu ({} bons)", s.good),
                        Some(rank) => format!("rang {rank} : {} €", decimal(s.gain, 2)),
                    },
                }
            }
        };
        out.tickets.push(TicketView {
            id: reco.id,
            grid: reco.grid.to_string(),
            closes,
            ticket: reco.ticket.to_string(),
            ev: decimal(reco.ev_base, 2),
            ev_low: decimal(reco.ev_pessimistic, 2),
            paid: format!("{} %", decimal(reco.paid_probability * 100.0, 1)),
            chance: chance(reco.paid_probability),
            ranks: rank_views(reco),
            status,
            played: played.contains(&reco.id),
            margin,
            limit,
            limit_margin,
        });
    }
    out.lines = line_views(recos, played);
    out.stake = format!("{} €", decimal(recos.len() as f64 * TICKET_PRICE, 0));
    Ok(out)
}

/// Cumul d'une grille du Loto Foot sur les tickets du groupe qui la jouent. Les multiples regroupent sans perte : on
/// évalue directement les tickets, chacun comme une grille simple.
fn grid_total(recos: &[&RecoRow], grid: GridId) -> GridTotal {
    let mine: Vec<&RecoRow> = recos.iter().copied().filter(|r| r.grid == grid).collect();
    let price = format!("{} €", decimal(mine.len() as f64 * TICKET_PRICE, 0));
    // Tous les tickets d'une grille du Loto Foot partagent les mêmes entrées : celles du premier suffisent.
    let Some(model) = mine
        .first()
        .and_then(|r| GridModel::from_journal(r.grid.game, &r.inputs))
    else {
        return GridTotal {
            price,
            ..GridTotal::default()
        };
    };
    let lines: Vec<Line> = mine.iter().map(|r| Line::single(&r.ticket)).collect();
    let odds = model.lines_odds(&lines, &FROZEN);
    GridTotal {
        price,
        paid: chance(odds.iter().map(|o| o.probability).sum()),
        ranks: views(&odds),
    }
}

/// Chance et gain moyen de chaque rang d'un ticket, recalculés par le modèle à partir de ses entrées figées au journal.
fn rank_views(reco: &RecoRow) -> Vec<RankView> {
    GridModel::from_journal(reco.grid.game, &reco.inputs).map_or(Vec::new(), |model| {
        views(&model.rank_odds(&reco.ticket, &FROZEN))
    })
}

/// Chance et gain moyen d'une grille à remplir selon son meilleur résultat ([`GridModel::line_odds`]).
pub fn line_ranks(model: &GridModel, line: &Line) -> Vec<RankView> {
    views(&model.line_odds(line, &FROZEN))
}

/// Mise en forme des rangs : « Sans faute », 1 sur 2 400, ≈ 3 150 €.
fn views(odds: &[RankOdds]) -> Vec<RankView> {
    odds.iter()
        .enumerate()
        .map(|(errors, o)| RankView {
            label: match errors {
                0 => "Sans faute",
                1 => "Une erreur",
                _ => "Plusieurs erreurs",
            },
            chance: chance(o.probability),
            gain: rough_euros(o.gain_if_reached * TICKET_PRICE),
        })
        .collect()
}

/// Les rangs en une ligne de texte, pour Discord et la console : « sans faute 1 sur 2 400 (≈ 3 150 €), une erreur 1
/// sur 60 (≈ 45 €) ».
pub fn ranks_text(ranks: &[RankView]) -> String {
    ranks
        .iter()
        .map(|r| format!("{} {} ({})", r.label.to_lowercase(), r.chance, r.gain))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Grilles à remplir pour les tickets de la séance ; `played` : numéros des tickets déclarés joués.
fn line_views(recos: &[&RecoRow], played: &[i64]) -> Vec<LineView> {
    let keyed: Vec<_> = recos.iter().map(|r| ((r.grid), &r.ticket)).collect();
    session_lines(&keyed)
        .into_iter()
        .map(|l| {
            // `l.tickets` donne des rangs dans `recos` : on les traduit en numéros du journal.
            let ids: Vec<i64> = l.tickets.iter().map(|&i| recos[i].id).collect();
            // Tous les tickets d'une grille du Loto Foot partagent les mêmes entrées : celles du premier suffisent.
            let model = l
                .tickets
                .first()
                .and_then(|&i| GridModel::from_journal(recos[i].grid.game, &recos[i].inputs));
            let ranks = model.map_or(Vec::new(), |m| line_ranks(&m, &l.line));
            LineView {
                grid: l.grid.to_string(),
                signs: l
                    .line
                    .signs()
                    .iter()
                    .map(|choices| choices.iter().map(|&s| sign_char(s).to_string()).collect())
                    .collect(),
                code: l.line.to_string(),
                price: format!("{} €", l.line.combinations()),
                played: ids.iter().all(|id| played.contains(id)),
                tickets: ids,
                ranks,
            }
        })
        .collect()
}

fn ledger_view(label: &'static str, ledger: &Ledger, with_expected: bool) -> LedgerView {
    LedgerView {
        label,
        tickets: ledger.tickets,
        settled: ledger.settled,
        staked: format!("{} €", decimal(ledger.staked, 0)),
        returned: format!("{} €", decimal(ledger.returned, 2)),
        roi: ledger.roi().map_or("—".into(), |r| {
            format!("{} %", crate::format::signed(r * 100.0, 1))
        }),
        paid: if with_expected {
            format!("{} / {}", ledger.paid, decimal(ledger.expected_paid, 1))
        } else {
            ledger.paid.to_string()
        },
    }
}

/// Verdict en clair.
pub fn verdict_text(v: &Verification) -> String {
    match &v.verdict {
        Verdict::NoSession => "Aucune séance enregistrée.".to_string(),
        Verdict::Pending { weeks, next } => format!(
            "Pas de verdict avant {next} semaines ({weeks} écoulée(s) depuis la première séance). Le critère est figé : \
             calibration des concurrents, et gain réalisé au-dessus du 5e centile de sa loi simulée."
        ),
        Verdict::Consistent { checkpoint } => {
            format!(
                "À {checkpoint} semaines : le modèle tient (calibration dans les bornes, gain au-dessus du 5e centile)."
            )
        }
        Verdict::Failed { checkpoint, reason } => {
            format!("À {checkpoint} semaines : modèle EN DÉFAUT — {reason}. Arrêter de jouer.")
        }
    }
}

fn verdict_class(v: &Verdict) -> &'static str {
    match v {
        Verdict::Consistent { .. } => "",
        Verdict::Failed { .. } => "bad",
        _ => "warn",
    }
}

/// Message Discord d'une ou plusieurs grilles qui viennent de sortir : leur heure de clôture, sans aucun ticket (il est
/// trop tôt : la répartition des autres joueurs n'est pas encore représentative). Le calcul se fait au guichet.
///
/// Avec des points de vente configurés, chaque grille reçoit aussi, pour chacun, le dernier moment pour la valider
/// (heure de fermeture du guichet ou clôture de la grille, la plus proche des deux ; `now` : instant du message).
pub fn listing_text(
    listed: &[Listed],
    outlets: &[Outlet],
    now: DateTime<Utc>,
    site_url: Option<&str>,
) -> String {
    let mut text = String::from("**edge · Loto Foot** — nouvelle grille\n");
    for grid in listed {
        let jackpot = grid.jackpot.map_or(String::new(), |j| {
            format!(", pactole {} €", crate::format::grouped(u64::from(j)))
        });
        let state = if grid.split_published {
            "répartition FDJ publiée"
        } else {
            "répartition FDJ pas encore publiée"
        };
        text.push_str(&format!(
            "**{}** : clôture **{}**{jackpot} ({state}).\n",
            grid.grid,
            kickoff_label(grid.closes_at)
        ));
        if !outlets.is_empty() {
            text.push_str(&format!(
                "Dernier moment pour valider : {}.\n",
                last_chances(outlets, grid.closes_at, now)
            ));
        }
    }
    match site_url {
        Some(url) => text.push_str(&format!(
            "Au guichet, juste avant de valider : « Optimiser maintenant » sur <{url}lotofoot>."
        )),
        None => text.push_str(
            "Au guichet, juste avant de valider : « Optimiser maintenant » sur la page Loto Foot.",
        ),
    }
    text
}

/// Pour chaque point de vente, le dernier moment pour valider une grille qui ferme à `closes_at` :
/// « Le Longchamp **dim. 25/10 13:00** ; Relay, gare de Rennes jusqu'à la clôture ».
fn last_chances(outlets: &[Outlet], closes_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let mut any = false;
    let parts: Vec<String> = outlets
        .iter()
        .map(|outlet| match outlet.last_chance(closes_at, now) {
            Some(at) if at == closes_at => {
                any = true;
                format!("{} jusqu'à la clôture", outlet.name)
            }
            Some(at) => {
                any = true;
                format!("{} **{}**", outlet.name, kickoff_label(at))
            }
            None => format!("{} fermé d'ici la clôture", outlet.name),
        })
        .collect();
    let mut text = parts.join(" ; ");
    if !any {
        text.push_str(" — aucun de tes points de vente n'ouvre avant la clôture");
    }
    text
}

/// Budget restant de chaque mois après la séance : « octobre 2026 : 44 € restant(s) sur 60 € », suivi au besoin de
/// « + 12 € de gains recyclés ».
pub fn months_text(months: &[MonthBudget]) -> String {
    months
        .iter()
        .map(|m| {
            let recycled = if m.recycled > 0 {
                format!(" + {} € de gains recyclés", m.recycled)
            } else {
                String::new()
            };
            format!(
                "{} : {} € restant(s) sur {} €{recycled}",
                month_label(m.month),
                m.left(),
                m.budget
            )
        })
        .collect::<Vec<_>>()
        .join(" ; ")
}

/// « octobre 2026 ».
pub fn month_label(first_day: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    format!(
        "{} {}",
        crate::format::MONTHS[first_day.month0() as usize],
        first_day.year()
    )
}

/// Seuils de la séance et budget : « seuil de 1,62 à 2,04 (prix dynamique, budget de 100 € par mois) ».
pub fn threshold_text(report: &SessionReport) -> String {
    let mut bars = report.thresholds.clone();
    bars.sort_by(f64::total_cmp);
    let seuil = match (bars.first(), bars.last()) {
        (Some(low), Some(high)) if low != high => {
            format!("seuil de {} à {}", decimal(*low, 2), decimal(*high, 2))
        }
        (Some(bar), _) => format!("seuil de {}", decimal(*bar, 2)),
        _ => "aucun seuil calculé".to_string(),
    };
    format!(
        "{seuil} (prix dynamique, budget de {} € par mois)",
        report.monthly_budget
    )
}

/// Bilan du repli OddsPapi en une phrase (CLI, application, Discord) ; `None` s'il n'a pas servi.
pub fn oddspapi_text(search: &oddspapi::Search) -> Option<String> {
    if search.found.is_empty() && search.refused.is_empty() {
        return None;
    }
    let mut parts = vec![format!("Repli OddsPapi ({} requête(s))", search.spent)];
    if !search.found.is_empty() {
        parts.push(format!(
            "cote Pinnacle relevée : {}",
            search.found.join(" ; ")
        ));
    }
    if !search.refused.is_empty() {
        parts.push(format!("refus : {}", search.refused.join(" ; ")));
    }
    if !search.hints.is_empty() {
        parts.push(format!(
            "pistes pour la section [LOTOFOOT] de config/aliases.toml : {}",
            search.hints.join(" ; ")
        ));
    }
    Some(format!("{}.", parts.join(" — ")))
}

/// Texte du message Discord d'une séance : pour chaque grille, l'ordre des rencontres puis les tickets à remplir.
pub fn discord_text(report: &SessionReport, site_url: Option<&str>) -> String {
    let mut text = String::from("**edge · Loto Foot** — séance\n");
    if report.chosen.is_empty() {
        text.push_str("Aucun nouveau ticket : rien de plus à jouer.\n");
        // Sans ticket, les raisons comptent : grille déjà recommandée, répartition pas encore publiée…
        for skipped in &report.skipped {
            text.push_str(&format!("- {} : {}\n", skipped.grid, skipped.reason));
        }
    }
    let keyed: Vec<_> = report.chosen.iter().map(|c| (c.grid, &c.ticket)).collect();
    let lines = session_lines(&keyed);
    if !report.chosen.is_empty() {
        text.push_str(&format!(
            "{} ticket(s) à 1 €, soit {} € (budget plafond), en {} grille(s) à remplir (multiples sans perte : mêmes \
             tickets, même mise). À valider avant la clôture de chaque grille.\n",
            report.chosen.len(),
            report.chosen.len(),
            lines.len()
        ));
    }
    for grid in &report.evaluated {
        let own: Vec<_> = lines.iter().filter(|l| l.grid == grid.grid).collect();
        if own.is_empty() {
            continue;
        }
        text.push_str(&format!(
            "\n**{}** — clôture {}\n",
            grid.grid,
            kickoff_label(grid.closes_at)
        ));
        for (i, (h, a)) in grid.matches.iter().enumerate() {
            text.push_str(&format!("{}. {h} – {a}\n", i + 1));
        }
        for l in own {
            text.push_str(&format!(
                "`{}`  {} € — {}\n",
                l.line,
                l.line.combinations(),
                ranks_text(&line_ranks(&grid.model, &l.line))
            ));
        }
    }
    if !report.months.is_empty() {
        text.push_str(&format!(
            "\nBudget : {} ; {}.\n",
            months_text(&report.months),
            threshold_text(report)
        ));
    }
    if !report.unknown_names.is_empty() {
        text.push_str(&format!(
            "\nNoms inconnus (grilles écartées, à ajouter dans config/aliases.toml) : {}\n",
            report.unknown_names.join(", ")
        ));
    }
    if let Some(oddspapi) = oddspapi_text(&report.oddspapi) {
        text.push_str(&format!("\n{oddspapi}\n"));
    }
    if let Some(url) = site_url {
        text.push_str(&format!("\nDétail : <{url}lotofoot>\n"));
    }
    text.push_str("Après validation : `edge lotofoot played --last-session`.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lotofoot::{Game, grid_of_2026};
    use crate::store::migrate::migrate;

    #[test]
    fn the_oddspapi_fallback_is_summed_up_only_when_it_ran() {
        assert_eq!(oddspapi_text(&oddspapi::Search::default()), None);
        let search = oddspapi::Search {
            spent: 1,
            found: vec!["Benfica – Gil Vicente (SL Benfica – Gil Vicente Barcelos)".into()],
            refused: vec!["Braga – Famalicão : introuvable chez OddsPapi".into()],
            hints: vec![
                "Braga – Famalicão : piste OddsPapi « SC Braga » – « FC Famalicao »".into(),
            ],
        };
        let text = oddspapi_text(&search).unwrap();
        assert!(
            text.starts_with("Repli OddsPapi (1 requête(s)) — cote Pinnacle relevée : Benfica")
        );
        assert!(
            text.contains("refus : Braga") && text.contains("pistes pour la section [LOTOFOOT]")
        );
    }

    #[test]
    fn the_discord_text_lists_each_grid_then_its_tickets() {
        use crate::lotofoot::model::TicketValue;
        use crate::lotofoot::optimize::Candidate;
        use crate::lotofoot::session::Evaluated;
        let at = DateTime::parse_from_rfc3339("2026-10-24T15:55:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let value = |ev: f64| TicketValue {
            expected_return: ev,
            paid_probability: 0.01,
        };
        let report = SessionReport {
            evaluated: vec![Evaluated {
                grid: grid_of_2026(Game::Lf8, 147),
                closes_at: at,
                stakes: 300_000.0,
                best_pessimistic: 2.0,
                matches: (1..=8)
                    .map(|i| (format!("H{i}"), format!("A{i}")))
                    .collect(),
                // Huit rencontres à un tiers chacune : payé avec 7 ou 8 bons, soit 17 issues sur 6 561.
                model: {
                    use crate::domain::{Euros, Probs1x2};
                    use crate::lotofoot::model::GridModel;
                    let third = vec![Probs1x2::new(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0).unwrap(); 8];
                    let stakes = Euros::new(300_000.0).unwrap();
                    GridModel::new(
                        &third,
                        &third,
                        stakes,
                        &GridModel::masses_for(Game::Lf8, stakes, None),
                    )
                    .unwrap()
                },
            }],
            skipped: vec![],
            unknown_names: vec!["LOTOFOOT: Inconnu FC".into()],
            leagues: Default::default(),
            oddspapi: Default::default(),
            chosen: vec![Candidate {
                grid: grid_of_2026(Game::Lf8, 147),
                ticket: crate::lotofoot::Ticket::parse("21N12222").unwrap(),
                base: value(2.5),
                pessimistic: value(2.0),
                marginal: 2.0,
                threshold: 1.8,
                budget_left: 56,
            }],
            session_id: Some(1),
            months: vec![MonthBudget {
                month: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                budget: 60,
                recycled: 0,
                spent_before: 4,
                chosen: 1,
            }],
            monthly_budget: 60,
            thresholds: vec![1.8],
        };
        let text = discord_text(&report, Some("http://portable.local:8737/"));
        assert!(text.contains("1 ticket(s) à 1 €"));
        assert!(text.contains("**LF8 n°147**"));
        assert!(text.contains("1. H1 – A1"));
        assert!(text.contains("en 1 grille(s) à remplir"));
        // Huit rencontres à un tiers : sans faute 1 issue sur 6 561, une erreur 16 sur 6 561 (1 sur 410).
        assert!(
            text.contains("`2 1 N 1 2 2 2 2`  1 € — sans faute 1 sur 6\u{a0}600 (≈ "),
            "{text}"
        );
        assert!(text.contains("une erreur 1 sur 410 (≈ "), "{text}");
        assert!(text.contains("Inconnu FC"));
        assert!(
            text.contains(
                "octobre 2026 : 55 € restant(s) sur 60 € ; seuil de 1,80 (prix dynamique"
            ),
            "{text}"
        );
        assert!(text.contains("<http://portable.local:8737/lotofoot>"));
        let empty = SessionReport {
            chosen: vec![],
            ..report
        };
        assert!(discord_text(&empty, None).contains("rien de plus à jouer"));
    }

    /// Un tabac ouvert du mardi au samedi de 7 h à 20 h et le dimanche matin.
    fn outlet() -> Outlet {
        let range = |r: &str| vec![r.to_string()];
        Outlet::from_config(&crate::config::LotofootOutlet {
            name: "Le Longchamp".to_string(),
            lun: vec![],
            mar: range("07:00-20:00"),
            mer: range("07:00-20:00"),
            jeu: range("07:00-20:00"),
            ven: range("07:00-20:00"),
            sam: range("07:00-20:00"),
            dim: range("08:30-13:00"),
        })
        .unwrap()
    }

    #[test]
    fn a_new_grid_is_announced_with_its_closing_time_and_no_ticket() {
        // 16 h 10 UTC le 25 octobre, jour du passage à l'heure d'hiver : 17 h 10 à Paris.
        let closes = DateTime::parse_from_rfc3339("2026-10-25T16:10:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let text = listing_text(
            &[Listed {
                grid: grid_of_2026(Game::Lf7, 133),
                closes_at: closes,
                jackpot: Some(100_000),
                split_published: false,
            }],
            &[outlet()],
            DateTime::parse_from_rfc3339("2026-10-24T09:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            Some("http://portable.local:8737/"),
        );
        assert!(
            text.contains("Dernier moment pour valider : Le Longchamp **dim. 25/10 13:00**."),
            "{text}"
        );
        assert!(
            text.contains("**LF7 n°133** : clôture **dim. 25/10 17:10**, pactole 100\u{a0}000 €"),
            "{text}"
        );
        assert!(
            text.contains("répartition FDJ pas encore publiée"),
            "{text}"
        );
        assert!(
            text.contains("<http://portable.local:8737/lotofoot>"),
            "{text}"
        );
    }

    #[test]
    fn the_limit_margin_grows_with_the_square_root_of_the_time_to_close() {
        approx::assert_abs_diff_eq!(limit_margin(1.0), 0.045, epsilon = 1e-3);
        approx::assert_abs_diff_eq!(limit_margin(9.0), 0.1412, epsilon = 1e-3);
        approx::assert_abs_diff_eq!(limit_margin(24.0), 0.2408, epsilon = 1e-3);
        assert_eq!(limit_margin(-2.0), 0.0);
        // +10 % : sûr une heure avant, limite neuf heures avant.
        assert_eq!(margin(1.98, 1.8, 1.0), ("+10 %".to_string(), false));
        assert_eq!(margin(1.98, 1.8, 9.0), ("+10 %".to_string(), true));
        assert_eq!(margin(1.8, 1.8, 0.0), ("+0 %".to_string(), false));
    }

    #[test]
    fn an_empty_journal_gives_an_empty_page() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let page = page(&conn, Utc::now(), 60).unwrap();
        assert!(
            page.to_play.tickets.is_empty()
                && page.in_play.tickets.is_empty()
                && !page.budget.threshold.is_empty()
        );
        assert_eq!(page.verdict, "Aucune séance enregistrée.");
    }
}
