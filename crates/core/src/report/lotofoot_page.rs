//! Loto Foot (phase 14) : page `/lotofoot` de l'application Web et message Discord de la séance du samedi.
//!
//! Les calculs viennent de [`crate::lotofoot`] (règlement, vérification), comme `edge lotofoot report` : ce module les
//! met en forme pour le gabarit `lotofoot.html` de l'application et pour le texte du message.

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;
use crate::format::{decimal, kickoff_label, long_date};
use crate::lotofoot::session::SessionReport;
use crate::lotofoot::settle::{self, Ledger};
use crate::lotofoot::verify::{self, Verdict, Verification};
use crate::lotofoot::{Game, TICKET_PRICE};
use crate::store::lotofoot as store;

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
    /// Règlement (« en attente », « perdu », « rang 2 : 21,90 € »).
    pub status: String,
    /// Déjà déclaré joué (`edge lotofoot played`).
    pub played: bool,
}

/// Une grille de la séance, avec l'ordre de ses rencontres (celui du bulletin).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GridView {
    /// Grille.
    pub grid: String,
    /// Clôture, heure de Paris.
    pub closes: String,
    /// Rencontres numérotées.
    pub matches: Vec<String>,
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
    /// Dernière séance (date), vide s'il n'y en a pas.
    pub session: String,
    /// Grilles de la dernière séance.
    pub grids: Vec<GridView>,
    /// Tickets de la dernière séance.
    pub tickets: Vec<TicketView>,
    /// Mise totale de la dernière séance.
    pub stake: String,
    /// Grilles écartées (« LF7 n°133 : répartition pas encore publiée »).
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

/// Construit la page à partir du journal.
pub fn page(conn: &Connection, now: DateTime<Utc>) -> Result<LotofootPage> {
    let verification = verify::verify(conn, now)?;
    let recommended = settle::recommended(conn)?;
    let (played_model, played_off) = settle::played_ledgers(conn)?;
    let mut page = LotofootPage {
        session: String::new(),
        grids: Vec::new(),
        tickets: Vec::new(),
        stake: String::new(),
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
    let Some((session_id, created_at, notes)) = store::last_session(conn)? else {
        return Ok(page);
    };
    page.session = long_date(created_at);
    if let Ok(notes) = serde_json::from_str::<serde_json::Value>(&notes) {
        page.skipped = notes
            .get("skipped")
            .and_then(|s| s.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|s| {
                        Some(format!(
                            "{} : {}",
                            s.get("grid")?.as_str()?,
                            s.get("reason")?.as_str()?
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        page.unknown_names = notes
            .get("unknown_names")
            .and_then(|s| s.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|n| n.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
    }
    let recos: Vec<_> = store::recos(conn)?
        .into_iter()
        .filter(|r| r.session_id == session_id)
        .collect();
    let played: Vec<i64> = store::plays(conn)?
        .iter()
        .filter_map(|p| p.reco_id)
        .collect();
    let mut seen: Vec<(Game, u32)> = Vec::new();
    for reco in &recos {
        let grid = store::grid(conn, reco.game, reco.number)?;
        let closes = grid
            .as_ref()
            .map(|g| kickoff_label(g.closes_at))
            .unwrap_or_default();
        if !seen.contains(&(reco.game, reco.number)) {
            seen.push((reco.game, reco.number));
            page.grids.push(GridView {
                grid: format!("{} n°{}", reco.game, reco.number),
                closes: closes.clone(),
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
        page.tickets.push(TicketView {
            id: reco.id,
            grid: format!("{} n°{}", reco.game, reco.number),
            closes,
            ticket: reco.ticket.to_string(),
            ev: decimal(reco.ev_base, 2),
            ev_low: decimal(reco.ev_pessimistic, 2),
            paid: format!("{} %", decimal(reco.paid_probability * 100.0, 1)),
            status,
            played: played.contains(&reco.id),
        });
    }
    page.stake = format!("{} €", decimal(recos.len() as f64 * TICKET_PRICE, 0));
    Ok(page)
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

/// Texte du message Discord d'une séance : pour chaque grille, l'ordre des rencontres puis les tickets à remplir.
pub fn discord_text(report: &SessionReport, site_url: Option<&str>) -> String {
    let mut text = String::from("**edge · Loto Foot** — séance du samedi\n");
    if report.chosen.is_empty() {
        text.push_str("Aucun ticket ne passe le seuil cette semaine : rien à jouer.\n");
    } else {
        text.push_str(&format!(
            "{} ticket(s) à 1 €, soit {} € (budget plafond). À valider avant la clôture de chaque grille.\n",
            report.chosen.len(),
            report.chosen.len()
        ));
    }
    for grid in &report.evaluated {
        let tickets: Vec<_> = report
            .chosen
            .iter()
            .filter(|c| c.grid == (grid.game, grid.number))
            .collect();
        if tickets.is_empty() {
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
        for c in tickets {
            text.push_str(&format!(
                "`{}`  espérance {} (défavorable {})\n",
                c.ticket,
                decimal(c.base.expected_return, 2),
                decimal(c.pessimistic.expected_return, 2)
            ));
        }
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
        };
        let text = discord_text(&report, Some("http://portable.local:8737/"));
        assert!(text.contains("1 ticket(s) à 1 €"));
        assert!(text.contains("**LF8 n°147**"));
        assert!(text.contains("1. H1 – A1"));
        assert!(text.contains("`21N12222`  espérance 2,50 (défavorable 2,00)"));
        assert!(text.contains("Inconnu FC"));
        assert!(text.contains("<http://portable.local:8737/lotofoot>"));
        let empty = SessionReport {
            chosen: vec![],
            ..report
        };
        assert!(discord_text(&empty, None).contains("rien à jouer"));
    }

    #[test]
    fn an_empty_journal_gives_an_empty_page() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let page = page(&conn, Utc::now()).unwrap();
        assert!(page.tickets.is_empty());
        assert_eq!(page.verdict, "Aucune séance enregistrée.");
    }
}
