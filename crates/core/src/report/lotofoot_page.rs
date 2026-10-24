//! Loto Foot (phase 14) : page `/lotofoot` de l'application Web et message Discord de la séance du samedi.
//!
//! Les calculs viennent de [`crate::lotofoot`] (règlement, vérification), comme `edge lotofoot report` : ce module les
//! met en forme pour le gabarit `lotofoot.html` de l'application et pour le texte du message.

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;
use crate::format::{chance, decimal, kickoff_label, long_date, rough_euros};
use crate::lotofoot::abaque;
use crate::lotofoot::alert::Listed;
use crate::lotofoot::bulletin::{Line, session_lines};
use crate::lotofoot::model::{FROZEN, GridModel, RankOdds};
use crate::lotofoot::outlets::Outlet;
use crate::lotofoot::session::{self, MonthBudget, SessionReport};
use crate::lotofoot::settle::{self, Ledger};
use crate::lotofoot::verify::{self, Verdict, Verification};
use crate::lotofoot::{Game, TICKET_PRICE, sign_char};
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
    /// Seuil τ tiré de l'abaque (« 1,8 »), vide si le budget sort de l'abaque.
    pub threshold: String,
    /// Mois en cours (« octobre 2026 »).
    pub month: String,
    /// Tickets en vigueur des grilles qui ferment ce mois (depuis l'entrée en vigueur du budget).
    pub spent: u32,
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
        session: String::new(),
        session_rule: String::new(),
        evaluated: Vec::new(),
        skipped: Vec::new(),
        unknown_names: Vec::new(),
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
        && let (Some(tau), Some(budget)) = (
            params.get("threshold").and_then(|t| t.as_f64()),
            params.get("monthly_budget").and_then(|b| b.as_u64()),
        )
    {
        page.session_rule = format!("τ = {}, budget de {budget} € par mois", decimal(tau, 1));
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
        page.unknown_names = list("unknown_names")
            .iter()
            .filter_map(|n| n.as_str().map(str::to_string))
            .collect();
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
            if store::results(conn, reco.game, reco.number)?.is_none() {
                out.in_play.push(reco);
            }
        } else if store::grid(conn, reco.game, reco.number)?.is_some_and(|g| g.closes_at > now) {
            out.to_play.push(reco);
        }
    }
    out.played = played;
    Ok(out)
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
        let closes = store::grid(conn, reco.game, reco.number)?.map(|g| g.closes_at);
        if closes.is_some_and(|at| session::month_of(at) == month) {
            spent += 1;
        }
    }
    Ok(BudgetView {
        monthly,
        threshold: abaque::threshold(monthly).map_or(String::new(), |t| decimal(t, 1)),
        month: month_label(month),
        spent,
        left: monthly.saturating_sub(spent),
    })
}

/// Un groupe de tickets prêt à afficher : grilles par clôture, tickets, grilles à remplir.
fn group(conn: &Connection, recos: &[&RecoRow], played: &[i64]) -> Result<Group> {
    let mut out = Group::default();
    let mut seen: Vec<(Game, u32)> = Vec::new();
    for reco in recos {
        let grid = store::grid(conn, reco.game, reco.number)?;
        let closes = grid
            .as_ref()
            .map(|g| kickoff_label(g.closes_at))
            .unwrap_or_default();
        if !seen.contains(&(reco.game, reco.number)) {
            seen.push((reco.game, reco.number));
            out.grids.push(GridView {
                grid: format!("{} n°{}", reco.game, reco.number),
                key: format!("{}-{}", reco.game.code(), reco.number),
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
            });
        }
        let status = match store::results(conn, reco.game, reco.number)? {
            None => "en attente".to_string(),
            Some(results) => {
                let ranks = store::ranks(conn, reco.game, reco.number)?;
                let jackpot = grid.as_ref().and_then(|g| g.jackpot);
                match settle::paper(
                    reco.game,
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
            grid: format!("{} n°{}", reco.game, reco.number),
            closes,
            ticket: reco.ticket.to_string(),
            ev: decimal(reco.ev_base, 2),
            ev_low: decimal(reco.ev_pessimistic, 2),
            paid: format!("{} %", decimal(reco.paid_probability * 100.0, 1)),
            chance: chance(reco.paid_probability),
            ranks: rank_views(reco),
            status,
            played: played.contains(&reco.id),
        });
    }
    out.lines = line_views(recos, played);
    out.stake = format!("{} €", decimal(recos.len() as f64 * TICKET_PRICE, 0));
    Ok(out)
}

/// Chance et gain moyen de chaque rang d'un ticket, recalculés par le modèle à partir de ses entrées figées au journal.
fn rank_views(reco: &RecoRow) -> Vec<RankView> {
    GridModel::from_journal(reco.game, &reco.inputs).map_or(Vec::new(), |model| {
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
    let keyed: Vec<_> = recos
        .iter()
        .map(|r| ((r.game, r.number), &r.ticket))
        .collect();
    session_lines(&keyed)
        .into_iter()
        .map(|l| {
            // `l.tickets` donne des rangs dans `recos` : on les traduit en numéros du journal.
            let ids: Vec<i64> = l.tickets.iter().map(|&i| recos[i].id).collect();
            // Tous les tickets d'une grille du Loto Foot partagent les mêmes entrées : celles du premier suffisent.
            let model = l
                .tickets
                .first()
                .and_then(|&i| GridModel::from_journal(recos[i].game, &recos[i].inputs));
            let ranks = model.map_or(Vec::new(), |m| line_ranks(&m, &l.line));
            LineView {
                grid: format!("{} n°{}", l.grid.0, l.grid.1),
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
            "**{} n°{}** : clôture **{}**{jackpot} ({state}).\n",
            grid.game,
            grid.number,
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

/// Budget restant de chaque mois après la séance : « octobre 2026 : 44 € restant(s) sur 60 € ».
pub fn months_text(months: &[MonthBudget]) -> String {
    months
        .iter()
        .map(|m| {
            format!(
                "{} : {} € restant(s) sur {} €",
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

/// Seuil de la séance et budget qui le donne : « τ = 1,8 (budget de 60 € par mois, abaque §49) ».
pub fn threshold_text(report: &SessionReport) -> String {
    format!(
        "τ = {} (budget de {} € par mois, abaque §49)",
        decimal(report.threshold, 1),
        report.monthly_budget
    )
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
        let key = (grid.game, grid.number);
        let own: Vec<_> = lines.iter().filter(|l| l.grid == key).collect();
        if own.is_empty() {
            continue;
        }
        text.push_str(&format!(
            "\n**{} n°{}** — clôture {}\n",
            grid.game,
            grid.number,
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
    if let Some(url) = site_url {
        text.push_str(&format!("\nDétail : <{url}lotofoot>\n"));
    }
    text.push_str("Après validation : `edge lotofoot played --last-session`.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate::migrate;

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
                game: Game::Lf8,
                number: 147,
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
            chosen: vec![Candidate {
                grid: (Game::Lf8, 147),
                ticket: crate::lotofoot::Ticket::parse("21N12222").unwrap(),
                base: value(2.5),
                pessimistic: value(2.0),
            }],
            session_id: Some(1),
            months: vec![MonthBudget {
                month: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                budget: 60,
                spent_before: 4,
                chosen: 1,
            }],
            monthly_budget: 60,
            threshold: 1.8,
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
            text.contains("octobre 2026 : 55 € restant(s) sur 60 € ; τ = 1,8"),
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
                game: Game::Lf7,
                number: 133,
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
    fn an_empty_journal_gives_an_empty_page() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let page = page(&conn, Utc::now(), 60).unwrap();
        assert!(
            page.to_play.tickets.is_empty()
                && page.in_play.tickets.is_empty()
                && page.budget.threshold == "1,8"
        );
        assert_eq!(page.verdict, "Aucune séance enregistrée.");
    }
}
