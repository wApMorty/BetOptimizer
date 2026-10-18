//! Contrôle qualité des données de tennis : signale, ne corrige jamais.
//!
//! Le risque propre à ces fichiers : aucun identifiant de joueur, seulement un nom (« Federer R. »). Deux joueurs
//! homonymes se confondraient dans l'Elo. Le détecteur est exact pour une partie du risque : **un joueur ne joue pas deux
//! tournois le même jour**, donc un nom qui apparaît à la même date dans deux tournois différents désigne au moins deux
//! personnes. Un même nom deux fois à la même date **dans un même tournoi** n'est pas un homonyme (deux joueurs ne
//! portent pas le même nom dans un même tableau) : c'est une date approximative (demi-finale et finale datées du même
//! jour après une interruption), comptée à part.

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::Tour;
use crate::error::Result;

/// Première année de tennis-data utilisée (les `.xlsx` commencent en 2013).
pub const FIRST_YEAR: u16 = 2013;

/// Marge totale plausible d'un marché à deux issues (somme des probabilités implicites).
const IMPLIED_SUM_RANGE: (f64, f64) = (1.0, 1.30);

/// Bookmakers dont on vérifie les cotes (`Max` n'est pas un bookmaker : sa somme peut être sous 1).
const CHECKED: [&str; 3] = ["PS", "B365", "Avg"];

/// Comptes d'une année d'un circuit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeasonCheck {
    /// Circuit.
    pub tour: Tour,
    /// Année du fichier.
    pub season: u16,
    /// Matchs en base.
    pub matches: u32,
    /// Matchs terminés normalement.
    pub completed: u32,
    /// Abandons.
    pub retired: u32,
    /// Forfaits.
    pub walkovers: u32,
    /// Matchs avec une cote Pinnacle valide.
    pub with_pinnacle: u32,
    /// Matchs dont la date n'est pas dans l'année du fichier.
    pub out_of_season: u32,
}

/// Une paire de cotes dont la marge est hors de `[1,00 ; 1,30]`.
#[derive(Debug, Clone, PartialEq)]
pub struct OddsAnomaly {
    /// Date du match.
    pub date: String,
    /// Vainqueur.
    pub winner: String,
    /// Perdant.
    pub loser: String,
    /// Préfixe du bookmaker.
    pub bookmaker: String,
    /// `1/cote_vainqueur + 1/cote_perdant`.
    pub implied_sum: f64,
}

/// Un nom qui apparaît deux fois à la même date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomonymSuspect {
    /// Circuit.
    pub tour: Tour,
    /// Nom publié.
    pub name: String,
    /// Nombre de dates où il apparaît dans plusieurs tournois.
    pub days: u32,
}

/// Le contrôle complet.
#[derive(Debug, Clone, PartialEq)]
pub struct QcReport {
    /// Une ligne par circuit et par année présents.
    pub seasons: Vec<SeasonCheck>,
    /// Années attendues (de [`FIRST_YEAR`] à l'année courante) sans aucun match.
    pub missing: Vec<(Tour, u16)>,
    /// Cotes aberrantes.
    pub odds_anomalies: Vec<OddsAnomaly>,
    /// Noms suspects d'être portés par deux joueurs.
    pub homonyms: Vec<HomonymSuspect>,
    /// Couples (circuit, nom, date) où un joueur apparaît deux fois le même jour dans le même tournoi : dates approximatives.
    pub repeated_dates: u32,
    /// Paires de tours d'un même tournoi où le tour avancé est daté entièrement avant le tour précédent.
    pub round_order_violations: u32,
}

/// Contrôle la base. `current_year` borne les années attendues.
pub fn run(conn: &Connection, current_year: u16) -> Result<QcReport> {
    let mut seasons = Vec::new();
    let mut statement = conn.prepare(
        "SELECT m.tour, m.season, COUNT(*),
                SUM(m.status = 'completed'), SUM(m.status = 'retired'), SUM(m.status = 'walkover'),
                SUM(EXISTS (SELECT 1 FROM tennis_odds o WHERE o.match_id = m.id AND o.bookmaker = 'PS')),
                SUM(CAST(substr(m.match_date, 1, 4) AS INTEGER) != m.season)
         FROM tennis_match m GROUP BY m.tour, m.season ORDER BY m.tour, m.season",
    )?;
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u16>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, u32>(3)?,
            row.get::<_, u32>(4)?,
            row.get::<_, u32>(5)?,
            row.get::<_, u32>(6)?,
            row.get::<_, u32>(7)?,
        ))
    })? {
        let (tour, season, matches, completed, retired, walkovers, with_pinnacle, out_of_season) =
            row?;
        if let Some(tour) = Tour::parse(&tour) {
            seasons.push(SeasonCheck {
                tour,
                season,
                matches,
                completed,
                retired,
                walkovers,
                with_pinnacle,
                out_of_season,
            });
        }
    }

    let missing = Tour::ALL
        .into_iter()
        .flat_map(|tour| (FIRST_YEAR..=current_year).map(move |year| (tour, year)))
        .filter(|(tour, year)| !seasons.iter().any(|s| s.tour == *tour && s.season == *year))
        .collect();

    let mut odds_anomalies = Vec::new();
    let mut statement = conn.prepare(
        "SELECT m.match_date, m.winner, m.loser, o.bookmaker, 1.0 / o.winner_odds + 1.0 / o.loser_odds
         FROM tennis_odds o JOIN tennis_match m ON m.id = o.match_id
         WHERE o.bookmaker IN ('PS', 'B365', 'Avg')
           AND (1.0 / o.winner_odds + 1.0 / o.loser_odds < ?1 OR 1.0 / o.winner_odds + 1.0 / o.loser_odds > ?2)
         ORDER BY m.match_date, m.id",
    )?;
    debug_assert_eq!(
        CHECKED.len(),
        3,
        "la liste SQL ci-dessus doit rester celle de CHECKED"
    );
    for row in statement.query_map([IMPLIED_SUM_RANGE.0, IMPLIED_SUM_RANGE.1], |row| {
        Ok(OddsAnomaly {
            date: row.get(0)?,
            winner: row.get(1)?,
            loser: row.get(2)?,
            bookmaker: row.get(3)?,
            implied_sum: row.get(4)?,
        })
    })? {
        odds_anomalies.push(row?);
    }

    // Un nom à la même date dans deux tournois : au moins deux personnes derrière ce nom.
    let mut days: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut statement = conn.prepare(
        "SELECT tour, name FROM (
             SELECT tour, winner AS name, match_date, tournament FROM tennis_match
             UNION ALL SELECT tour, loser, match_date, tournament FROM tennis_match
         ) GROUP BY tour, name, match_date HAVING COUNT(DISTINCT tournament) > 1",
    )?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        *days.entry(row?).or_default() += 1;
    }
    let repeated_dates: u32 = conn.query_row(
        "SELECT COUNT(*) FROM (
             SELECT 1 FROM (
                 SELECT tour, winner AS name, match_date, tournament FROM tennis_match
                 UNION ALL SELECT tour, loser, match_date, tournament FROM tennis_match
             ) GROUP BY tour, name, match_date HAVING COUNT(*) > 1 AND COUNT(DISTINCT tournament) = 1
         )",
        [],
        |row| row.get(0),
    )?;
    let round_order_violations = round_order_violations(conn)?;
    let homonyms = days
        .into_iter()
        .filter_map(|((tour, name), days)| {
            Some(HomonymSuspect {
                tour: Tour::parse(&tour)?,
                name,
                days,
            })
        })
        .collect();

    Ok(QcReport {
        seasons,
        missing,
        odds_anomalies,
        homonyms,
        repeated_dates,
        round_order_violations,
    })
}

/// Paires de tours d'un même tournoi où TOUS les matchs du tour avancé sont datés avant TOUS ceux du tour précédent.
fn round_order_violations(conn: &Connection) -> Result<u32> {
    // (circuit, année, tournoi) → rang du tour → (première date, dernière date) ; `AAAA-MM-JJ` se compare comme du texte.
    type Rounds = BTreeMap<u8, (String, String)>;
    let mut tournaments: BTreeMap<(String, i64, String), Rounds> = BTreeMap::new();
    let mut statement = conn.prepare(
        "SELECT tour, season, tournament, round, MIN(match_date), MAX(match_date)
         FROM tennis_match GROUP BY tour, season, tournament, round",
    )?;
    for row in statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })? {
        let (tour, season, tournament, round, first, last) = row?;
        let rank = super::round_rank(&round);
        if rank > 0 {
            tournaments
                .entry((tour, season, tournament))
                .or_default()
                .insert(rank, (first, last));
        }
    }
    let mut violations = 0;
    for rounds in tournaments.values() {
        for (rank_a, (first_a, _)) in rounds {
            for (rank_b, (_, last_b)) in rounds {
                if rank_a < rank_b && last_b < first_a {
                    violations += 1;
                }
            }
        }
    }
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::tennis::import::{find_files, import_files};
    use crate::store::migrate;

    fn database_with_the_fixture() -> Connection {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("atp")).unwrap();
        std::fs::write(
            dir.path().join("atp").join("2024.xlsx"),
            include_bytes!("../../../tests/fixtures/tennis/mini_atp_2024.xlsx"),
        )
        .unwrap();
        let (files, _) = find_files(dir.path()).unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        migrate::migrate(&mut conn).unwrap();
        import_files(&mut conn, &files).unwrap();
        conn
    }

    #[test]
    fn the_season_counts_split_completed_retired_walkover_and_pinnacle_coverage() {
        let conn = database_with_the_fixture();

        let report = run(&conn, 2024).unwrap();

        assert_eq!(report.seasons.len(), 1);
        let season = &report.seasons[0];
        assert_eq!(
            (season.tour, season.season, season.matches),
            (Tour::Atp, 2024, 6)
        );
        assert_eq!(
            (season.completed, season.retired, season.walkovers),
            (4, 1, 1)
        );
        // Pinnacle valide sur le match 1, le forfait, l'abandon et le Grand Chelem ; absent sur la terre battue, invalide en finale.
        assert_eq!(season.with_pinnacle, 4);
        assert_eq!(season.out_of_season, 0);
    }

    #[test]
    fn missing_years_are_listed_for_both_tours() {
        let conn = database_with_the_fixture();

        let report = run(&conn, 2024).unwrap();

        assert!(report.missing.contains(&(Tour::Atp, 2013)));
        assert!(!report.missing.contains(&(Tour::Atp, 2024)));
        assert!(report.missing.contains(&(Tour::Wta, 2024)));
    }

    #[test]
    fn an_implausible_margin_is_reported_and_never_corrected() {
        let conn = database_with_the_fixture();
        conn.execute_batch(
            "UPDATE tennis_odds SET winner_odds = 3.0, loser_odds = 3.0
             WHERE bookmaker = 'PS' AND match_id = (SELECT MIN(id) FROM tennis_match)",
        )
        .unwrap();

        let report = run(&conn, 2024).unwrap();

        assert_eq!(report.odds_anomalies.len(), 1);
        assert_eq!(report.odds_anomalies[0].bookmaker, "PS");
        assert!((report.odds_anomalies[0].implied_sum - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_name_twice_on_the_same_day_in_the_same_tournament_is_a_rough_date_not_a_homonym() {
        let conn = database_with_the_fixture();
        // « Alpha A. » a déjà gagné le 7 janvier dans « Fake Open » : une demi-finale du même tournoi, même jour.
        conn.execute_batch(
            "INSERT INTO tennis_match (tour, season, tournament, match_date, surface, round, best_of, winner, loser, status)
             VALUES ('atp', 2024, 'Fake Open', '2024-01-07', 'hard', 'Semifinals', 3, 'Alpha A.', 'Zulu Z.', 'completed')",
        )
        .unwrap();

        let report = run(&conn, 2024).unwrap();

        assert!(report.homonyms.is_empty());
        assert_eq!(report.repeated_dates, 1);
    }

    #[test]
    fn a_later_round_dated_entirely_before_an_earlier_round_is_counted() {
        let conn = database_with_the_fixture();
        assert_eq!(run(&conn, 2024).unwrap().round_order_violations, 0);
        // Une finale datée de 2023 pour un tournoi dont les premiers tours sont en janvier 2024.
        conn.execute_batch(
            "INSERT INTO tennis_match (tour, season, tournament, match_date, surface, round, best_of, winner, loser, status)
             VALUES ('atp', 2024, 'Fake Open', '2023-12-31', 'hard', 'The Final', 3, 'Alpha A.', 'Zulu Z.', 'completed')",
        )
        .unwrap();

        assert!(run(&conn, 2024).unwrap().round_order_violations >= 1);
    }

    #[test]
    fn a_name_playing_twice_on_the_same_day_is_a_homonym_suspect() {
        let conn = database_with_the_fixture();
        assert!(run(&conn, 2024).unwrap().homonyms.is_empty());

        // « Alpha A. » apparaît une seconde fois le 7 janvier 2024, dans un autre tournoi.
        conn.execute_batch(
            "INSERT INTO tennis_match (tour, season, tournament, match_date, surface, round, best_of, winner, loser, status)
             VALUES ('atp', 2024, 'Autre Open', '2024-01-07', 'hard', '1st Round', 3, 'Alpha A.', 'Zulu Z.', 'completed')",
        )
        .unwrap();

        let report = run(&conn, 2024).unwrap();

        assert_eq!(report.homonyms.len(), 1);
        assert_eq!(report.homonyms[0].name, "Alpha A.");
        assert_eq!(report.homonyms[0].days, 1);
    }
}
