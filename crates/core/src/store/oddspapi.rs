//! Archive de l'historique OddsPapi (migration 14) et lecture de notre archive The Odds API pour le contrôle de qualité.
//!
//! Uniquement des `INSERT` dans les tables OddsPapi : des déclencheurs interdisent toute modification ou suppression.

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, params};

use super::journal::parse_timestamp;
use crate::domain::Odds;
use crate::error::Result;
use crate::ingest::oddspapi::format::{Fixture, History};

/// Horodatage à la microseconde : OddsPapi distingue des changements dans la même seconde (suspension puis reprise),
/// et l'ordre de ces entrées décide du prix valable. Format fixe, donc l'ordre des textes est celui des instants.
fn timestamp_micros(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Un appel à journaliser (paramètres sans la clé).
#[derive(Debug, Clone, Copy)]
pub struct NewCall<'a> {
    /// Instant de l'appel.
    pub called_at: DateTime<Utc>,
    /// `account`, `fixtures` ou `historical-odds`.
    pub endpoint: &'a str,
    /// Paramètres envoyés, sans `apiKey`.
    pub params: &'a str,
    /// Statut HTTP de la réponse.
    pub http_status: u16,
    /// Consommation lue sur `account` ; `None` pour les autres appels.
    pub request_count: Option<u32>,
}

/// Journalise un appel et renvoie son identifiant.
pub fn record_call(conn: &Connection, call: &NewCall<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO oddspapi_call (called_at, endpoint, params, http_status, request_count)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            timestamp_micros(call.called_at),
            call.endpoint,
            call.params,
            call.http_status,
            call.request_count,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// `true` si l'historique de ce match a déjà été demandé (trouvé ou non) : on ne le redemande jamais.
pub fn known_fixture(conn: &Connection, fixture_id: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM oddspapi_fixture WHERE fixture_id = ?1",
        [fixture_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Enregistre un match et son historique (`None` : OddsPapi n'en a pas). Une seule transaction : tout ou rien.
/// Renvoie le nombre de changements de prix écrits.
pub fn record_fixture(
    conn: &mut Connection,
    fixture: &Fixture,
    fetched_at: DateTime<Utc>,
    bookmakers: &str,
    history: Option<&History>,
) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut written = 0;
    // Bloc : la requête préparée emprunte `tx`, et cet emprunt doit finir avant `tx.commit()`, qui prend `tx` par valeur.
    {
        tx.execute(
            "INSERT INTO oddspapi_fixture
                 (fixture_id, tournament_id, kickoff, home_raw, away_raw, bookmakers, fetched_at, history, discarded)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                fixture.fixture_id,
                fixture.tournament_id,
                timestamp_micros(fixture.kickoff),
                fixture.home,
                fixture.away,
                bookmakers,
                timestamp_micros(fetched_at),
                if history.is_some() { "ok" } else { "missing" },
                // SQLite stocke des entiers signés : un compte d'entrées tient toujours dans un i64.
                i64::try_from(history.map_or(0, |h| h.discarded)).unwrap_or(i64::MAX),
            ],
        )?;
        let mut insert = tx.prepare_cached(
            "INSERT INTO oddspapi_price (fixture_id, bookmaker, market_id, outcome_id, changed_at, price, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for change in history.map_or(&[][..], |h| &h.prices) {
            insert.execute(params![
                fixture.fixture_id,
                change.bookmaker,
                change.market_id,
                change.outcome_id,
                timestamp_micros(change.changed_at),
                change.price.map(Odds::get),
                change.active,
            ])?;
            written += 1;
        }
    }
    tx.commit()?;
    Ok(written)
}

/// Un match archivé avec un historique.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredFixture {
    /// Identifiant OddsPapi.
    pub fixture_id: String,
    /// Championnat OddsPapi.
    pub tournament_id: u32,
    /// Coup d'envoi, UTC.
    pub kickoff: DateTime<Utc>,
    /// Noms bruts d'OddsPapi.
    pub home_raw: String,
    /// Noms bruts d'OddsPapi.
    pub away_raw: String,
}

/// Un changement de prix du 1X2.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPrice {
    /// Match.
    pub fixture_id: String,
    /// Bookmaker.
    pub bookmaker: String,
    /// 101 domicile, 102 nul, 103 extérieur.
    pub outcome_id: u32,
    /// Instant du changement.
    pub changed_at: DateTime<Utc>,
    /// Cote (absente pour une suspension sans prix).
    pub price: Option<Odds>,
    /// `false` : issue suspendue.
    pub active: bool,
}

/// Matchs archivés avec historique, dont le coup d'envoi est postérieur ou égal à `since`.
pub fn fixtures_since(conn: &Connection, since: DateTime<Utc>) -> Result<Vec<StoredFixture>> {
    let mut stmt = conn.prepare(
        "SELECT fixture_id, tournament_id, kickoff, home_raw, away_raw FROM oddspapi_fixture
         WHERE history = 'ok' AND kickoff >= ?1 ORDER BY kickoff, fixture_id",
    )?;
    let rows = stmt.query_map([timestamp_micros(since)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut fixtures = Vec::new();
    for row in rows {
        let (fixture_id, tournament_id, kickoff, home_raw, away_raw) = row?;
        fixtures.push(StoredFixture {
            fixture_id,
            tournament_id,
            kickoff: parse_timestamp(&kickoff)?,
            home_raw,
            away_raw,
        });
    }
    Ok(fixtures)
}

/// Changements de prix du 1X2 des matchs de [`fixtures_since`], triés par match, bookmaker, issue et instant.
pub fn prices_1x2_since(conn: &Connection, since: DateTime<Utc>) -> Result<Vec<StoredPrice>> {
    let mut stmt = conn.prepare(
        "SELECT p.fixture_id, p.bookmaker, p.outcome_id, p.changed_at, p.price, p.active
         FROM oddspapi_price p JOIN oddspapi_fixture f ON f.fixture_id = p.fixture_id
         WHERE p.market_id = 101 AND f.kickoff >= ?1
         ORDER BY p.fixture_id, p.bookmaker, p.outcome_id, p.changed_at",
    )?;
    let rows = stmt.query_map([timestamp_micros(since)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<f64>>(4)?,
            row.get::<_, bool>(5)?,
        ))
    })?;
    let mut prices = Vec::new();
    for row in rows {
        let (fixture_id, bookmaker, outcome_id, changed_at, price, active) = row?;
        prices.push(StoredPrice {
            fixture_id,
            bookmaker,
            outcome_id,
            changed_at: parse_timestamp(&changed_at)?,
            // La base n'accepte que des prix > 1 (contrainte CHECK) : `Odds::new` ne peut pas échouer ici.
            price: price.and_then(Odds::new),
            active,
        });
    }
    Ok(prices)
}

/// Une cote 1X2 de notre archive The Odds API (contrôle de qualité).
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveQuote {
    /// Instant du relevé.
    pub called_at: DateTime<Utc>,
    /// Coup d'envoi.
    pub kickoff: DateTime<Utc>,
    /// Ligue (`soccer_epl`…).
    pub sport_key: String,
    /// Noms bruts de The Odds API.
    pub home_raw: String,
    /// Noms bruts de The Odds API.
    pub away_raw: String,
    /// `winamax_fr`, `unibet_fr`…
    pub bookmaker_key: String,
    /// Une équipe ou `Draw`.
    pub outcome_raw: String,
    /// Cote.
    pub price: f64,
}

/// Cotes 1X2 de football de notre archive pour les matchs commençant à partir de `since`, chez les `bookmakers` donnés.
pub fn archive_quotes_since(
    conn: &Connection,
    since: DateTime<Utc>,
    bookmakers: &[&str],
) -> Result<Vec<ArchiveQuote>> {
    let mut stmt = conn.prepare(
        "SELECT a.called_at, e.commence_time, e.sport_key, e.home_team_raw, e.away_team_raw,
                q.bookmaker_key, q.outcome_raw, q.price
         FROM quote q
         JOIN event_seen e ON e.api_call_id = q.api_call_id AND e.event_id = q.event_id
         JOIN api_call a ON a.id = q.api_call_id
         WHERE q.market_key = 'h2h' AND e.sport_key LIKE 'soccer%' AND e.commence_time >= ?1
         ORDER BY a.called_at, e.event_id",
    )?;
    // L'archive The Odds API est horodatée à la seconde : on compare donc au même format.
    let since_text = since.to_rfc3339_opts(SecondsFormat::Secs, true);
    let rows = stmt.query_map([since_text], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, f64>(7)?,
        ))
    })?;
    let mut quotes = Vec::new();
    for row in rows {
        let (called_at, kickoff, sport_key, home_raw, away_raw, bookmaker_key, outcome_raw, price) =
            row?;
        if !bookmakers.contains(&bookmaker_key.as_str()) {
            continue;
        }
        quotes.push(ArchiveQuote {
            called_at: parse_timestamp(&called_at)?,
            kickoff: parse_timestamp(&kickoff)?,
            sport_key,
            home_raw,
            away_raw,
            bookmaker_key,
            outcome_raw,
            price,
        });
    }
    Ok(quotes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::oddspapi::format::PriceChange;
    use crate::store::open_migrated;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().to_utc()
    }

    fn fixture() -> Fixture {
        Fixture {
            fixture_id: "id1".into(),
            tournament_id: 34,
            kickoff: at("2026-10-25T15:00:00Z"),
            home: "Angers SCO".into(),
            away: "Olympique Marseille".into(),
            status_id: 2,
        }
    }

    fn change(outcome_id: u32, changed_at: &str, price: f64, active: bool) -> PriceChange {
        PriceChange {
            bookmaker: "winamax.fr".into(),
            market_id: 101,
            outcome_id,
            changed_at: at(changed_at),
            price: Odds::new(price),
            active,
        }
    }

    #[test]
    fn a_fixture_and_its_prices_round_trip_and_cannot_be_changed() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_migrated(&dir.path().join("edge.db")).unwrap();
        let history = History {
            prices: vec![
                change(101, "2026-10-25T14:00:00.250Z", 3.40, true),
                change(101, "2026-10-25T14:00:00.750Z", 3.35, false),
            ],
            discarded: 1,
        };
        let written = record_fixture(
            &mut conn,
            &fixture(),
            at("2026-10-27T08:00:00Z"),
            "winamax.fr",
            Some(&history),
        )
        .unwrap();
        assert_eq!(written, 2);
        assert!(known_fixture(&conn, "id1").unwrap());

        let since = at("2026-10-22T00:00:00Z");
        assert_eq!(
            fixtures_since(&conn, since).unwrap()[0].home_raw,
            "Angers SCO"
        );
        let prices = prices_1x2_since(&conn, since).unwrap();
        // Deux changements dans la même seconde : l'ordre (et donc le prix valable) est conservé grâce aux microsecondes.
        assert_eq!(prices.len(), 2);
        assert!(prices[0].active && !prices[1].active);

        assert!(
            conn.execute("UPDATE oddspapi_price SET price = 9.0", [])
                .is_err()
        );
        assert!(conn.execute("DELETE FROM oddspapi_fixture", []).is_err());
    }

    #[test]
    fn a_missing_history_is_recorded_once_and_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open_migrated(&dir.path().join("edge.db")).unwrap();
        record_fixture(
            &mut conn,
            &fixture(),
            at("2026-10-27T08:00:00Z"),
            "winamax.fr",
            None,
        )
        .unwrap();
        assert!(known_fixture(&conn, "id1").unwrap());
        assert!(
            fixtures_since(&conn, at("2026-10-22T00:00:00Z"))
                .unwrap()
                .is_empty()
        );
        // Le même match ne peut pas être enregistré deux fois (clé primaire).
        assert!(
            record_fixture(
                &mut conn,
                &fixture(),
                at("2026-10-28T08:00:00Z"),
                "winamax.fr",
                None
            )
            .is_err()
        );
    }
}
