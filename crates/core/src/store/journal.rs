//! Journal des recommandations et des paris réels (tables immuables, migration 7).
//!
//! Uniquement des `INSERT` : des déclencheurs interdisent toute modification ou suppression. Un règlement est
//! une ligne de plus (table `bet_result`), jamais une mise à jour du pari. Un pari sans recommandation est marqué
//! `off_model` et se rapporte à part (invariant 7 de CLAUDE.md).

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::{Euros, Odds, Outcome, Probability};
use crate::error::{Error, Result};

/// Horodatage au format du journal : RFC 3339, UTC, à la seconde.
pub(crate) fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Relit un horodatage du journal.
pub(crate) fn parse_timestamp(text: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| {
            Error::Data(format!(
                "horodatage illisible dans la base « {text} » : {e}"
            ))
        })
}

/// Palier d'une recommandation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Notifiée : l'avantage atteint le seuil.
    Recommended,
    /// Journalisée en silence : avantage positif sous le seuil, pour mesurer le CLV par tranche.
    Shadow,
}

impl Tier {
    /// Texte stocké en base.
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Recommended => "recommended",
            Tier::Shadow => "shadow",
        }
    }

    fn parse(text: &str) -> Result<Tier> {
        match text {
            "recommended" => Ok(Tier::Recommended),
            "shadow" => Ok(Tier::Shadow),
            other => Err(Error::Data(format!(
                "palier inconnu dans la base : {other}"
            ))),
        }
    }
}

/// Ce qu'une recommandation enregistre.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRecommendation {
    /// Nom de la règle (`price_vs_pinnacle`).
    pub rule: String,
    /// Palier.
    pub tier: Tier,
    /// Identifiant du match chez The Odds API.
    pub event_id: String,
    /// Compétition.
    pub sport_key: String,
    /// Équipe à domicile, nom brut.
    pub home_team: String,
    /// Équipe à l'extérieur, nom brut.
    pub away_team: String,
    /// Coup d'envoi (UTC).
    pub kickoff: DateTime<Utc>,
    /// Bookmaker qui propose le prix.
    pub bookmaker: String,
    /// Issue recommandée.
    pub outcome: Outcome,
    /// Cote proposée.
    pub odds: Odds,
    /// Probabilité de référence (Pinnacle sans marge).
    pub probability: Probability,
    /// Espérance `p × cote − 1`.
    pub edge: f64,
    /// Seuil d'avantage appliqué.
    pub min_edge: f64,
    /// Cote en dessous de laquelle le pari n'est plus recommandé.
    pub minimal_odds: f64,
    /// Mise suggérée ; `None` pour le journal fantôme ou si elle tombe sous le minimum.
    pub stake: Option<Euros>,
    /// Fraction de Kelly appliquée.
    pub kelly_fraction: f64,
    /// Bankroll au moment du calcul.
    pub bankroll: Euros,
    /// Méthode de retrait de la marge.
    pub margin_method: String,
    /// Relevé d'où viennent les cotes.
    pub api_call_id: i64,
    /// Version du programme.
    pub code_version: String,
}

/// Une recommandation relue dans le journal.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredRecommendation {
    /// Identifiant.
    pub id: i64,
    /// Instant de création (UTC).
    pub created_at: DateTime<Utc>,
    /// Contenu.
    pub record: NewRecommendation,
}

/// Enregistre une recommandation. Renvoie `None` si le match a déjà une recommandation de cette règle et de ce palier :
/// relancer la commande ne duplique rien, et la première reste telle qu'elle a été écrite.
pub fn insert_recommendation(
    conn: &Connection,
    new: &NewRecommendation,
    created_at: DateTime<Utc>,
) -> Result<Option<i64>> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO recommendation
             (created_at, rule, tier, event_id, sport_key, home_team_raw, away_team_raw, kickoff, bookmaker_key,
              outcome, odds, probability, edge, min_edge, minimal_odds, stake, kelly_fraction, bankroll,
              margin_method, api_call_id, code_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            timestamp(created_at),
            new.rule,
            new.tier.as_str(),
            new.event_id,
            new.sport_key,
            new.home_team,
            new.away_team,
            timestamp(new.kickoff),
            new.bookmaker,
            new.outcome.code(),
            new.odds.get(),
            new.probability.get(),
            new.edge,
            new.min_edge,
            new.minimal_odds,
            new.stake.map(Euros::get),
            new.kelly_fraction,
            new.bankroll.get(),
            new.margin_method,
            new.api_call_id,
            new.code_version,
        ],
    )?;
    Ok((changed == 1).then(|| conn.last_insert_rowid()))
}

/// Le match a-t-il déjà une recommandation de cette règle et de ce palier ?
pub fn recommendation_exists(
    conn: &Connection,
    rule: &str,
    tier: Tier,
    event_id: &str,
) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM recommendation WHERE rule = ?1 AND tier = ?2 AND event_id = ?3",
        params![rule, tier.as_str(), event_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

const RECOMMENDATION_COLUMNS: &str = "id, created_at, rule, tier, event_id, sport_key, home_team_raw, away_team_raw,
     kickoff, bookmaker_key, outcome, odds, probability, edge, min_edge, minimal_odds, stake, kelly_fraction,
     bankroll, margin_method, api_call_id, code_version";

fn recommendation_from_row(row: &Row<'_>) -> rusqlite::Result<Result<StoredRecommendation>> {
    // Les valeurs lues sont déjà validées à l'écriture (contraintes CHECK) ; on les revalide en types du domaine.
    let (id, created_at, kickoff): (i64, String, String) = (row.get(0)?, row.get(1)?, row.get(8)?);
    let (tier, outcome): (String, String) = (row.get(3)?, row.get(10)?);
    let (odds, probability, bankroll): (f64, f64, f64) = (row.get(11)?, row.get(12)?, row.get(18)?);
    let stake: Option<f64> = row.get(16)?;
    let record = (|| -> Result<StoredRecommendation> {
        Ok(StoredRecommendation {
            id,
            created_at: parse_timestamp(&created_at)?,
            record: NewRecommendation {
                rule: row.get(2)?,
                tier: Tier::parse(&tier)?,
                event_id: row.get(4)?,
                sport_key: row.get(5)?,
                home_team: row.get(6)?,
                away_team: row.get(7)?,
                kickoff: parse_timestamp(&kickoff)?,
                bookmaker: row.get(9)?,
                outcome: Outcome::from_code(&outcome).ok_or_else(|| {
                    Error::Data(format!("issue inconnue dans la base : {outcome}"))
                })?,
                odds: Odds::new(odds)
                    .ok_or_else(|| Error::Data(format!("cote invalide : {odds}")))?,
                probability: Probability::new(probability)
                    .ok_or_else(|| Error::Data(format!("probabilité invalide : {probability}")))?,
                edge: row.get(13)?,
                min_edge: row.get(14)?,
                minimal_odds: row.get(15)?,
                stake: stake.and_then(Euros::new),
                kelly_fraction: row.get(17)?,
                bankroll: Euros::new(bankroll)
                    .ok_or_else(|| Error::Data(format!("bankroll invalide : {bankroll}")))?,
                margin_method: row.get(19)?,
                api_call_id: row.get(20)?,
                code_version: row.get(21)?,
            },
        })
    })();
    Ok(record)
}

/// Toutes les recommandations, de la plus ancienne à la plus récente.
pub fn recommendations(conn: &Connection) -> Result<Vec<StoredRecommendation>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {RECOMMENDATION_COLUMNS} FROM recommendation ORDER BY id"
    ))?;
    let rows = statement.query_map([], recommendation_from_row)?;
    let mut all = Vec::new();
    for row in rows {
        all.push(row??);
    }
    Ok(all)
}

/// Une recommandation par son identifiant.
pub fn recommendation(conn: &Connection, id: i64) -> Result<Option<StoredRecommendation>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {RECOMMENDATION_COLUMNS} FROM recommendation WHERE id = ?1"
    ))?;
    let found = statement
        .query_row([id], recommendation_from_row)
        .optional()?;
    found.transpose()
}

/// Un match tel que l'archive des cotes l'a vu (dernier relevé où il figure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedEvent {
    /// Compétition.
    pub sport_key: String,
    /// Équipe à domicile, nom brut.
    pub home_team: String,
    /// Équipe à l'extérieur, nom brut.
    pub away_team: String,
    /// Coup d'envoi (UTC).
    pub kickoff: DateTime<Utc>,
}

/// Retrouve un match de l'archive par son identifiant (pour enregistrer un pari hors modèle).
pub fn archived_event(conn: &Connection, event_id: &str) -> Result<Option<ArchivedEvent>> {
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT sport_key, home_team_raw, away_team_raw, commence_time FROM event_seen
             WHERE event_id = ?1 ORDER BY api_call_id DESC LIMIT 1",
            [event_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    row.map(|(sport_key, home_team, away_team, kickoff)| {
        Ok(ArchivedEvent {
            sport_key,
            home_team,
            away_team,
            kickoff: parse_timestamp(&kickoff)?,
        })
    })
    .transpose()
}

/// Un pari réellement placé.
#[derive(Debug, Clone, PartialEq)]
pub struct NewBet {
    /// Recommandation suivie ; `None` = pari hors modèle (`off_model`).
    pub recommendation_id: Option<i64>,
    /// Identifiant du match chez The Odds API.
    pub event_id: String,
    /// Compétition.
    pub sport_key: String,
    /// Équipe à domicile, nom brut.
    pub home_team: String,
    /// Équipe à l'extérieur, nom brut.
    pub away_team: String,
    /// Coup d'envoi (UTC).
    pub kickoff: DateTime<Utc>,
    /// Bookmaker chez qui le pari est placé.
    pub bookmaker: String,
    /// Issue jouée.
    pub outcome: Outcome,
    /// Cote réellement obtenue.
    pub odds: Odds,
    /// Mise.
    pub stake: Euros,
}

/// Enregistre un pari réel et renvoie son identifiant.
pub fn insert_bet(conn: &Connection, new: &NewBet, created_at: DateTime<Utc>) -> Result<i64> {
    conn.execute(
        "INSERT INTO bet
             (created_at, recommendation_id, off_model, event_id, sport_key, home_team_raw, away_team_raw, kickoff,
              bookmaker_key, outcome, odds, stake)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            timestamp(created_at),
            new.recommendation_id,
            i64::from(new.recommendation_id.is_none()),
            new.event_id,
            new.sport_key,
            new.home_team,
            new.away_team,
            timestamp(new.kickoff),
            new.bookmaker,
            new.outcome.code(),
            new.odds.get(),
            new.stake.get(),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Issue d'un pari.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// Gagné.
    Won,
    /// Perdu.
    Lost,
    /// Remboursé : match reporté ou annulé.
    Void,
}

impl Settlement {
    /// Texte stocké en base.
    pub fn as_str(self) -> &'static str {
        match self {
            Settlement::Won => "won",
            Settlement::Lost => "lost",
            Settlement::Void => "void",
        }
    }

    fn parse(text: &str) -> Result<Settlement> {
        match text {
            "won" => Ok(Settlement::Won),
            "lost" => Ok(Settlement::Lost),
            "void" => Ok(Settlement::Void),
            other => Err(Error::Data(format!(
                "règlement inconnu dans la base : {other}"
            ))),
        }
    }

    /// Gain net d'une mise : `mise × (cote − 1)` si gagné, `−mise` si perdu, `0` si remboursé.
    pub fn profit(self, odds: Odds, stake: Euros) -> f64 {
        match self {
            Settlement::Won => stake.get() * (odds.get() - 1.0),
            Settlement::Lost => -stake.get(),
            Settlement::Void => 0.0,
        }
    }
}

/// D'où vient un règlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementSource {
    /// Résultat lu dans l'historique football-data importé.
    FootballData,
    /// Saisi à la main (`edge bet settle --result …`).
    Manual,
}

impl SettlementSource {
    /// Texte stocké en base.
    pub fn as_str(self) -> &'static str {
        match self {
            SettlementSource::FootballData => "football-data",
            SettlementSource::Manual => "manual",
        }
    }

    fn parse(text: &str) -> Result<SettlementSource> {
        match text {
            "football-data" => Ok(SettlementSource::FootballData),
            "manual" => Ok(SettlementSource::Manual),
            other => Err(Error::Data(format!(
                "source de règlement inconnue : {other}"
            ))),
        }
    }
}

/// Règlement d'un pari.
#[derive(Debug, Clone, PartialEq)]
pub struct BetResult {
    /// Instant du règlement (UTC).
    pub settled_at: DateTime<Utc>,
    /// Issue du pari.
    pub settlement: Settlement,
    /// Origine du résultat.
    pub source: SettlementSource,
    /// Gain net en euros.
    pub profit: f64,
}

/// Un pari relu dans le journal, avec son règlement éventuel.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredBet {
    /// Identifiant.
    pub id: i64,
    /// Instant de l'enregistrement (UTC).
    pub created_at: DateTime<Utc>,
    /// Contenu.
    pub bet: NewBet,
    /// Règlement, une fois connu.
    pub result: Option<BetResult>,
}

/// Règle un pari. Renvoie `false` si le pari est déjà réglé : un règlement ne se réécrit pas.
pub fn insert_result(
    conn: &Connection,
    bet: &StoredBet,
    settlement: Settlement,
    source: SettlementSource,
    settled_at: DateTime<Utc>,
) -> Result<bool> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO bet_result (bet_id, settled_at, result, source, profit)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            bet.id,
            timestamp(settled_at),
            settlement.as_str(),
            source.as_str(),
            settlement.profit(bet.bet.odds, bet.bet.stake),
        ],
    )?;
    Ok(changed == 1)
}

/// Tous les paris réels avec leur règlement, du plus ancien au plus récent.
pub fn bets(conn: &Connection) -> Result<Vec<StoredBet>> {
    let mut statement = conn.prepare(
        "SELECT b.id, b.created_at, b.recommendation_id, b.event_id, b.sport_key, b.home_team_raw, b.away_team_raw,
                b.kickoff, b.bookmaker_key, b.outcome, b.odds, b.stake,
                r.settled_at, r.result, r.source, r.profit
         FROM bet b LEFT JOIN bet_result r ON r.bet_id = b.id
         ORDER BY b.id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, f64>(10)?,
            row.get::<_, f64>(11)?,
            row.get::<_, Option<String>>(12)?,
            row.get::<_, Option<String>>(13)?,
            row.get::<_, Option<String>>(14)?,
            row.get::<_, Option<f64>>(15)?,
        ))
    })?;
    let mut all = Vec::new();
    for row in rows {
        let (
            id,
            created_at,
            recommendation_id,
            event_id,
            sport_key,
            home,
            away,
            kickoff,
            bookmaker,
            outcome,
            odds,
            stake,
            settled_at,
            result,
            source,
            profit,
        ) = row?;
        let result = match (settled_at, result, source, profit) {
            (Some(settled_at), Some(result), Some(source), Some(profit)) => Some(BetResult {
                settled_at: parse_timestamp(&settled_at)?,
                settlement: Settlement::parse(&result)?,
                source: SettlementSource::parse(&source)?,
                profit,
            }),
            _ => None,
        };
        all.push(StoredBet {
            id,
            created_at: parse_timestamp(&created_at)?,
            bet: NewBet {
                recommendation_id,
                event_id,
                sport_key,
                home_team: home,
                away_team: away,
                kickoff: parse_timestamp(&kickoff)?,
                bookmaker,
                outcome: Outcome::from_code(&outcome).ok_or_else(|| {
                    Error::Data(format!("issue inconnue dans la base : {outcome}"))
                })?,
                odds: Odds::new(odds)
                    .ok_or_else(|| Error::Data(format!("cote invalide : {odds}")))?,
                stake: Euros::new(stake)
                    .ok_or_else(|| Error::Data(format!("mise invalide : {stake}")))?,
            },
            result,
        });
    }
    Ok(all)
}

/// Bankroll courante : bankroll initiale + gains nets des paris réglés − mises des paris encore ouverts.
///
/// Une mise ouverte est déjà sortie de la bankroll disponible : on ne la remise pas une seconde fois.
pub fn current_bankroll(conn: &Connection, initial: Euros) -> Result<Euros> {
    let delta: f64 = conn.query_row(
        "SELECT COALESCE(SUM(CASE WHEN r.bet_id IS NULL THEN -b.stake ELSE r.profit END), 0.0)
         FROM bet b LEFT JOIN bet_result r ON r.bet_id = b.id",
        [],
        |row| row.get(0),
    )?;
    // Une bankroll ne descend pas sous zéro : un pari ne coûte jamais plus que la mise (floor_to_step garantit
    // qu'on ne mise pas plus que la bankroll), mais des mises saisies à la main pourraient la dépasser.
    Ok(Euros::new((initial.get() + delta).max(0.0))
        .expect("max(0.0) d'un nombre fini donne un montant valide"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::migrate;

    fn database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate::migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO api_call (id, called_at, endpoint, sport_key, params, http_status)
             VALUES (1, '2026-10-18T12:00:00Z', 'odds', 'soccer_epl', 'markets=h2h', 200)",
        )
        .unwrap();
        conn
    }

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn recommendation_data(tier: Tier) -> NewRecommendation {
        NewRecommendation {
            rule: "price_vs_pinnacle".into(),
            tier,
            event_id: "match-1".into(),
            sport_key: "soccer_epl".into(),
            home_team: "Arsenal".into(),
            away_team: "Chelsea".into(),
            kickoff: at("2026-10-24T14:00:00Z"),
            bookmaker: "betclic_fr".into(),
            outcome: Outcome::Home,
            odds: Odds::new(2.3).unwrap(),
            probability: Probability::new(0.4865).unwrap(),
            edge: 0.1189,
            min_edge: 0.03,
            minimal_odds: 2.117,
            stake: Euros::new(91.4),
            kelly_fraction: 1.0,
            bankroll: Euros::new(1000.0).unwrap(),
            margin_method: "power".into(),
            api_call_id: 1,
            code_version: "0.1.0".into(),
        }
    }

    fn bet_data(recommendation_id: Option<i64>) -> NewBet {
        NewBet {
            recommendation_id,
            event_id: "match-1".into(),
            sport_key: "soccer_epl".into(),
            home_team: "Arsenal".into(),
            away_team: "Chelsea".into(),
            kickoff: at("2026-10-24T14:00:00Z"),
            bookmaker: "betclic_fr".into(),
            outcome: Outcome::Home,
            odds: Odds::new(2.2).unwrap(),
            stake: Euros::new(50.0).unwrap(),
        }
    }

    #[test]
    fn a_recommendation_round_trips_through_the_journal() {
        let conn = database();
        let data = recommendation_data(Tier::Recommended);

        let id = insert_recommendation(&conn, &data, at("2026-10-18T14:05:00Z"))
            .unwrap()
            .unwrap();

        let stored = recommendation(&conn, id).unwrap().unwrap();
        assert_eq!(stored.record, data);
        assert_eq!(stored.created_at, at("2026-10-18T14:05:00Z"));
        assert_eq!(recommendations(&conn).unwrap().len(), 1);
    }

    #[test]
    fn running_again_does_not_duplicate_and_the_first_row_is_kept() {
        let conn = database();
        let first = recommendation_data(Tier::Recommended);
        let mut later = recommendation_data(Tier::Recommended);
        later.bookmaker = "winamax_fr".into();

        assert!(
            insert_recommendation(&conn, &first, at("2026-10-18T14:05:00Z"))
                .unwrap()
                .is_some()
        );
        let second = insert_recommendation(&conn, &later, at("2026-10-19T14:05:00Z")).unwrap();

        assert_eq!(second, None);
        let all = recommendations(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].record.bookmaker, "betclic_fr");
        assert!(
            recommendation_exists(&conn, "price_vs_pinnacle", Tier::Recommended, "match-1")
                .unwrap()
        );
        assert!(
            !recommendation_exists(&conn, "price_vs_pinnacle", Tier::Shadow, "match-1").unwrap()
        );
    }

    #[test]
    fn the_same_match_can_hold_one_recommended_and_one_shadow_row() {
        let conn = database();
        insert_recommendation(
            &conn,
            &recommendation_data(Tier::Shadow),
            at("2026-10-18T14:05:00Z"),
        )
        .unwrap();
        let second = insert_recommendation(
            &conn,
            &recommendation_data(Tier::Recommended),
            at("2026-10-19T14:05:00Z"),
        )
        .unwrap();

        assert!(second.is_some());
    }

    #[test]
    fn journal_tables_refuse_update_and_delete() {
        let conn = database();
        let rec = insert_recommendation(
            &conn,
            &recommendation_data(Tier::Recommended),
            at("2026-10-18T14:05:00Z"),
        )
        .unwrap()
        .unwrap();
        let bet = insert_bet(&conn, &bet_data(Some(rec)), at("2026-10-18T15:00:00Z")).unwrap();
        let stored = bets(&conn).unwrap().remove(0);
        insert_result(
            &conn,
            &stored,
            Settlement::Won,
            SettlementSource::Manual,
            at("2026-10-25T10:00:00Z"),
        )
        .unwrap();

        for statement in [
            "UPDATE recommendation SET odds = 3.0",
            "DELETE FROM recommendation",
            "UPDATE bet SET stake = 1.0",
            "DELETE FROM bet",
            "UPDATE bet_result SET profit = 0.0",
            "DELETE FROM bet_result",
        ] {
            let error = conn.execute(statement, []).unwrap_err().to_string();
            assert!(error.contains("journal immuable"), "{statement} : {error}");
        }
        assert_eq!(bet, stored.id);
    }

    #[test]
    fn a_bet_without_a_recommendation_is_marked_off_model() {
        let conn = database();
        let rec = insert_recommendation(
            &conn,
            &recommendation_data(Tier::Recommended),
            at("2026-10-18T14:05:00Z"),
        )
        .unwrap()
        .unwrap();
        insert_bet(&conn, &bet_data(Some(rec)), at("2026-10-18T15:00:00Z")).unwrap();
        insert_bet(&conn, &bet_data(None), at("2026-10-18T15:01:00Z")).unwrap();

        let all = bets(&conn).unwrap();
        let flags: Vec<i64> = conn
            .prepare("SELECT off_model FROM bet ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(all[0].bet.recommendation_id, Some(rec));
        assert_eq!(all[1].bet.recommendation_id, None);
        assert_eq!(flags, vec![0, 1]);
    }

    #[test]
    fn an_archived_event_is_found_by_its_id_in_its_latest_snapshot() {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO event_seen (api_call_id, event_id, sport_key, commence_time, home_team_raw, away_team_raw)
             VALUES (1, 'm1', 'soccer_epl', '2026-10-24T14:00:00Z', 'Arsenal', 'Chelsea')",
        )
        .unwrap();

        let found = archived_event(&conn, "m1").unwrap().unwrap();

        assert_eq!(found.home_team, "Arsenal");
        assert_eq!(found.kickoff, at("2026-10-24T14:00:00Z"));
        assert!(archived_event(&conn, "absent").unwrap().is_none());
    }

    #[test]
    fn a_bet_cannot_point_to_a_missing_recommendation() {
        let conn = database();

        assert!(insert_bet(&conn, &bet_data(Some(999)), at("2026-10-18T15:00:00Z")).is_err());
    }

    #[test]
    fn the_profit_of_each_settlement_follows_the_stake_and_the_odds() {
        let odds = Odds::new(2.2).unwrap();
        let stake = Euros::new(50.0).unwrap();

        assert!((Settlement::Won.profit(odds, stake) - 60.0).abs() < 1e-9);
        assert_eq!(Settlement::Lost.profit(odds, stake), -50.0);
        assert_eq!(Settlement::Void.profit(odds, stake), 0.0);
    }

    #[test]
    fn a_settlement_is_written_once() {
        let conn = database();
        insert_bet(&conn, &bet_data(None), at("2026-10-18T15:00:00Z")).unwrap();
        let stored = bets(&conn).unwrap().remove(0);

        let first = insert_result(
            &conn,
            &stored,
            Settlement::Won,
            SettlementSource::FootballData,
            at("2026-10-25T10:00:00Z"),
        )
        .unwrap();
        let second = insert_result(
            &conn,
            &stored,
            Settlement::Lost,
            SettlementSource::Manual,
            at("2026-10-26T10:00:00Z"),
        )
        .unwrap();

        assert!(first);
        assert!(!second);
        let settled = bets(&conn).unwrap().remove(0).result.unwrap();
        assert_eq!(settled.settlement, Settlement::Won);
        assert_eq!(settled.source, SettlementSource::FootballData);
        assert!((settled.profit - 60.0).abs() < 1e-9);
    }

    #[test]
    fn the_bankroll_counts_settled_profits_and_open_stakes() {
        let conn = database();
        let initial = Euros::new(1000.0).unwrap();
        assert_eq!(current_bankroll(&conn, initial).unwrap().get(), 1000.0);

        // Pari 1 gagné (+60), pari 2 perdu (−50), pari 3 ouvert (mise 50 déjà sortie).
        for _ in 0..3 {
            insert_bet(&conn, &bet_data(None), at("2026-10-18T15:00:00Z")).unwrap();
        }
        let all = bets(&conn).unwrap();
        insert_result(
            &conn,
            &all[0],
            Settlement::Won,
            SettlementSource::Manual,
            at("2026-10-25T10:00:00Z"),
        )
        .unwrap();
        insert_result(
            &conn,
            &all[1],
            Settlement::Lost,
            SettlementSource::Manual,
            at("2026-10-25T10:00:00Z"),
        )
        .unwrap();

        let bankroll = current_bankroll(&conn, initial).unwrap().get();

        assert!(
            (bankroll - (1000.0 + 60.0 - 50.0 - 50.0)).abs() < 1e-9,
            "{bankroll}"
        );
    }
}
