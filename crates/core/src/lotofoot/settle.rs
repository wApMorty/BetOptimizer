//! Règlement des tickets à partir des résultats et des rapports officiels relevés.
//!
//! - Ticket **joué** : il touche le gain unitaire publié du rang atteint (la FDJ compte sa grille parmi les gagnants).
//! - Ticket **recommandé** (suivi du modèle, qu'il ait été joué ou non) : il aurait touché la masse du rang partagée avec
//!   les gagnants publiés, soit `gain unitaire × W / (W + 1)` ; s'il n'y a aucun gagnant publié, la masse entière,
//!   estimée à partir des mises estimées le jour de la séance et du pactole.
//!
//! Rien n'est stocké : le règlement se recalcule à la demande, à partir du journal immuable.

use rusqlite::Connection;

use super::model::GridModel;
use super::{Game, Ticket};
use crate::domain::{Euros, Outcome};
use crate::error::Result;
use crate::store::lotofoot::{self as store, PublishedRank};

/// Règlement d'un ticket.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settled {
    /// Bons pronostics.
    pub good: usize,
    /// Rang atteint (1 : sans-faute), `None` hors des rangs payés.
    pub rank: Option<u32>,
    /// Gain en euros (mise non comprise).
    pub gain: f64,
}

/// Rang atteint par un ticket : 1 pour le sans-faute, 2 pour une erreur, `None` au-delà des rangs payés.
fn rank_of(game: Game, good: usize) -> Option<u32> {
    let errors = game.matches().checked_sub(good)?;
    (errors < game.rank_shares().len()).then(|| u32::try_from(errors + 1).unwrap_or(u32::MAX))
}

/// Règlement d'un ticket **joué** ; `None` tant que les résultats ou les rapports ne sont pas connus.
pub fn played(
    game: Game,
    ticket: &Ticket,
    results: &[Outcome],
    ranks: &[PublishedRank],
) -> Option<Settled> {
    let good = ticket.good(results);
    let rank = rank_of(game, good);
    let gain = match rank {
        None => 0.0,
        Some(r) => ranks
            .iter()
            .find(|p| p.rank == r)?
            .unit_gain
            .map_or(0.0, Euros::get),
    };
    Some(Settled { good, rank, gain })
}

/// Règlement d'un ticket **recommandé**, comme s'il avait été joué seul. `None` tant que les rapports manquent.
pub fn paper(
    game: Game,
    ticket: &Ticket,
    results: &[Outcome],
    ranks: &[PublishedRank],
    stakes_estimate: f64,
    jackpot: Option<Euros>,
) -> Option<Settled> {
    let good = ticket.good(results);
    let rank = rank_of(game, good);
    let gain = match rank {
        None => 0.0,
        Some(r) => {
            let published = ranks.iter().find(|p| p.rank == r)?;
            match published.unit_gain {
                Some(unit) if published.winners > 0 => {
                    let w = f64::from(published.winners);
                    unit.get() * w / (w + 1.0)
                }
                _ => {
                    let masses = GridModel::masses_for(game, Euros::new(stakes_estimate)?, jackpot);
                    masses.get(usize::try_from(r - 1).ok()?)?.get()
                }
            }
        }
    };
    Some(Settled { good, rank, gain })
}

/// Bilan d'un ensemble de tickets.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ledger {
    /// Tickets de l'ensemble.
    pub tickets: usize,
    /// Tickets réglés (résultats et rapports connus).
    pub settled: usize,
    /// Mises des tickets réglés.
    pub staked: f64,
    /// Gains des tickets réglés.
    pub returned: f64,
    /// Tickets réglés qui ont atteint un rang payé.
    pub paid: usize,
    /// Somme des probabilités de gain annoncées des tickets réglés (nombre de tickets payés attendu).
    pub expected_paid: f64,
    /// Somme des espérances annoncées des tickets réglés (gains attendus, mises comprises).
    pub expected_return: f64,
}

impl Ledger {
    /// Rendement : gains / mises − 1 ; `None` sans ticket réglé.
    pub fn roi(&self) -> Option<f64> {
        (self.staked > 0.0).then(|| self.returned / self.staked - 1.0)
    }
}

/// Bilan des tickets recommandés (suivi du modèle).
pub fn recommended(conn: &Connection) -> Result<Ledger> {
    let mut ledger = Ledger::default();
    for reco in store::current_recos(conn)? {
        ledger.tickets += 1;
        let Some(results) = store::results(conn, reco.game, reco.number)? else {
            continue;
        };
        let ranks = store::ranks(conn, reco.game, reco.number)?;
        let jackpot = store::grid(conn, reco.game, reco.number)?.and_then(|g| g.jackpot);
        let Some(settled) = paper(
            reco.game,
            &reco.ticket,
            &results,
            &ranks,
            reco.stakes_estimate,
            jackpot,
        ) else {
            continue;
        };
        ledger.settled += 1;
        ledger.staked += super::TICKET_PRICE;
        ledger.returned += settled.gain;
        ledger.paid += usize::from(settled.rank.is_some());
        ledger.expected_paid += reco.paid_probability;
        ledger.expected_return += reco.ev_base * super::TICKET_PRICE;
    }
    Ok(ledger)
}

/// Bilan des tickets réellement joués, séparé en tickets du modèle et tickets hors modèle (CLAUDE.md, invariant 7).
pub fn played_ledgers(conn: &Connection) -> Result<(Ledger, Ledger)> {
    let mut model = Ledger::default();
    let mut off = Ledger::default();
    for play in store::plays(conn)? {
        let ledger = if play.reco_id.is_some() {
            &mut model
        } else {
            &mut off
        };
        ledger.tickets += 1;
        let Some(results) = store::results(conn, play.game, play.number)? else {
            continue;
        };
        let ranks = store::ranks(conn, play.game, play.number)?;
        let Some(settled) = played(play.game, &play.ticket, &results, &ranks) else {
            continue;
        };
        ledger.settled += 1;
        ledger.staked += play.stake;
        // Une grille jouée avec une mise unitaire de 2 € ou 5 € touche le gain unitaire autant de fois.
        ledger.returned += settled.gain * play.stake / super::TICKET_PRICE;
        ledger.paid += usize::from(settled.rank.is_some());
    }
    Ok((model, off))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranks(w1: u32, g1: Option<f64>, w2: u32, g2: Option<f64>) -> Vec<PublishedRank> {
        vec![
            PublishedRank {
                rank: 1,
                winners: w1,
                unit_gain: g1.and_then(Euros::new),
            },
            PublishedRank {
                rank: 2,
                winners: w2,
                unit_gain: g2.and_then(Euros::new),
            },
        ]
    }

    const RESULTS: [Outcome; 7] = [
        Outcome::Home,
        Outcome::Draw,
        Outcome::Away,
        Outcome::Home,
        Outcome::Home,
        Outcome::Away,
        Outcome::Draw,
    ];

    #[test]
    fn a_played_ticket_receives_the_published_unit_gain() {
        let ranks = ranks(135, Some(578.0), 2393, Some(39.8));
        let perfect = Ticket::parse("1N2112N").unwrap();
        assert_eq!(
            played(Game::Lf7, &perfect, &RESULTS, &ranks).unwrap(),
            Settled {
                good: 7,
                rank: Some(1),
                gain: 578.0
            }
        );
        let one_error = Ticket::parse("1N2112N".replace('N', "1").as_str()).unwrap();
        let s = played(Game::Lf7, &one_error, &RESULTS, &ranks).unwrap();
        assert_eq!((s.good, s.rank), (5, None));
        let second = Ticket::parse("1N21121").unwrap();
        assert_eq!(
            played(Game::Lf7, &second, &RESULTS, &ranks).unwrap().gain,
            39.8
        );
    }

    #[test]
    fn a_paper_ticket_shares_the_mass_with_the_published_winners() {
        let ranks = ranks(1, Some(578.0), 2393, Some(39.8));
        let perfect = Ticket::parse("1N2112N").unwrap();
        // Un seul gagnant publié : avec nous, la masse se partage en deux.
        let s = paper(Game::Lf7, &perfect, &RESULTS, &ranks, 200_000.0, None).unwrap();
        assert!((s.gain - 289.0).abs() < 1e-9);
        // Aucun gagnant publié : nous aurions touché la masse entière (45 % de 70 % des mises, ou le pactole).
        let ranks = ranks_without_winner();
        let s = paper(
            Game::Lf7,
            &perfect,
            &RESULTS,
            &ranks,
            200_000.0,
            Euros::new(100_000.0),
        )
        .unwrap();
        assert!((s.gain - 100_000.0).abs() < 1e-9);
        let s = paper(
            Game::Lf7,
            &perfect,
            &RESULTS,
            &ranks,
            400_000.0,
            Euros::new(100_000.0),
        )
        .unwrap();
        assert!((s.gain - 0.45 * 0.7 * 400_000.0).abs() < 1e-6);
    }

    fn ranks_without_winner() -> Vec<PublishedRank> {
        ranks(0, None, 2393, Some(39.8))
    }

    #[test]
    fn unknown_ranks_leave_the_ticket_unsettled() {
        let perfect = Ticket::parse("1N2112N").unwrap();
        assert!(played(Game::Lf7, &perfect, &RESULTS, &[]).is_none());
        // Un ticket hors des rangs payés se règle sans rapport (gain nul).
        let lost = Ticket::parse("2121221").unwrap();
        assert_eq!(played(Game::Lf7, &lost, &RESULTS, &[]).unwrap().gain, 0.0);
    }
}
