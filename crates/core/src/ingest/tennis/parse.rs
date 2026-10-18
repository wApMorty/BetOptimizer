//! Lecture d'un tableur de tennis-data.co.uk (`.xlsx`, et `.xls` pour les années anciennes) avec `calamine`.
//!
//! Les colonnes sont repérées par leur **nom** (première ligne), pas par leur position : les fichiers anciens n'ont pas
//! toutes les colonnes de cotes. Une ligne inutilisable est **rejetée avec sa raison**, jamais corrigée en silence.
//! Colonnes : `docs/sources.md` §21 (`notes.txt` de tennis-data).

use std::collections::HashMap;
use std::io::Cursor;

use calamine::{Data, Reader, open_workbook_auto_from_rs};
use chrono::{Days, NaiveDate};

use super::{MatchOdds, ParsedMatch, Status, Surface, Tour};
use crate::domain::Odds;
use crate::error::{Error, Result};

/// Bookmakers dont on lit les cotes : préfixe de colonne (`PSW` et `PSL` pour Pinnacle…).
const BOOKMAKERS: [&str; 13] = [
    "B365", "B&W", "CB", "EX", "LB", "GB", "IW", "PS", "SB", "SJ", "UB", "Max", "Avg",
];

/// Une ligne que l'on n'a pas pu lire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// Numéro de ligne dans le tableur (1 = en-tête).
    pub row: usize,
    /// Raison, en français.
    pub reason: String,
}

/// Le contenu utilisable d'un tableur.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFile {
    /// Matchs lus.
    pub matches: Vec<ParsedMatch>,
    /// Lignes rejetées.
    pub rejected: Vec<Rejected>,
    /// Paires de cotes écartées parce qu'une cote est invalide (≤ 1 ou illisible) ; le match est gardé.
    pub invalid_odds: usize,
}

/// Lit la première feuille d'un tableur.
pub fn parse_workbook(bytes: &[u8], tour: Tour, season: u16) -> Result<ParsedFile> {
    let mut workbook = open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|e| Error::Data(format!("tableur illisible : {e}")))?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or_else(|| Error::Data("le tableur ne contient aucune feuille".to_string()))?
        .map_err(|e| Error::Data(format!("feuille illisible : {e}")))?;

    let mut rows = range.rows();
    let header = rows
        .next()
        .ok_or_else(|| Error::Data("le tableur est vide".to_string()))?;
    let columns: HashMap<String, usize> = header
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| text(cell).map(|name| (name.to_lowercase(), index)))
        .collect();
    for required in ["date", "surface", "round", "winner", "loser", "best of"] {
        if !columns.contains_key(required) {
            return Err(Error::Data(format!(
                "colonne « {required} » absente : est-ce bien un fichier de tennis-data ?"
            )));
        }
    }

    let mut file = ParsedFile {
        matches: Vec::new(),
        rejected: Vec::new(),
        invalid_odds: 0,
    };
    for (index, row) in rows.enumerate() {
        // `index` part de 0 pour la première ligne de données, qui est la ligne 2 du tableur.
        let number = index + 2;
        if row.iter().all(|cell| matches!(cell, Data::Empty)) {
            continue;
        }
        match parse_row(row, &columns, tour, season) {
            Ok((parsed, invalid)) => {
                file.invalid_odds += invalid;
                file.matches.push(parsed);
            }
            Err(reason) => file.rejected.push(Rejected {
                row: number,
                reason,
            }),
        }
    }
    Ok(file)
}

/// Une ligne devient un match, avec le nombre de paires de cotes écartées, ou une raison de rejet.
fn parse_row(
    row: &[Data],
    columns: &HashMap<String, usize>,
    tour: Tour,
    season: u16,
) -> std::result::Result<(ParsedMatch, usize), String> {
    let cell = |name: &str| columns.get(name).and_then(|i| row.get(*i));
    let string = |name: &str| cell(name).and_then(text);
    let number = |name: &str| cell(name).and_then(number);

    let winner = string("winner").ok_or("vainqueur manquant")?;
    let loser = string("loser").ok_or("perdant manquant")?;
    if winner == loser {
        return Err(format!(
            "le vainqueur et le perdant sont le même nom (« {winner} »)"
        ));
    }
    let date = cell("date")
        .and_then(date)
        .ok_or("date manquante ou illisible")?;
    let surface_text = string("surface").ok_or("surface manquante")?;
    let surface = Surface::parse(&surface_text)
        .ok_or_else(|| format!("surface inconnue : « {surface_text} »"))?;
    let round = string("round").ok_or("tour du tournoi manquant")?;
    let best_of = match number("best of") {
        Some(n) if (n - 3.0).abs() < f64::EPSILON => 3,
        Some(n) if (n - 5.0).abs() < f64::EPSILON => 5,
        other => return Err(format!("« Best of » inattendu : {other:?}")),
    };

    let comment = string("comment").unwrap_or_default().to_lowercase();
    let status = if comment.contains("walkover") || comment.contains("w/o") {
        Status::Walkover
    } else if comment.contains("retire") {
        Status::Retired
    } else {
        Status::Completed
    };

    let mut odds = Vec::new();
    let mut invalid = 0;
    for bookmaker in BOOKMAKERS {
        let won = number(&format!("{}w", bookmaker.to_lowercase()));
        let lost = number(&format!("{}l", bookmaker.to_lowercase()));
        match (won, lost) {
            (None, None) => {}
            (Some(w), Some(l)) => match (Odds::new(w), Odds::new(l)) {
                (Some(winner), Some(loser)) => odds.push(MatchOdds {
                    bookmaker,
                    winner,
                    loser,
                }),
                _ => invalid += 1,
            },
            // Une seule des deux cotes : inutilisable.
            _ => invalid += 1,
        }
    }

    Ok((
        ParsedMatch {
            tour,
            season,
            tournament: string("tournament").ok_or("tournoi manquant")?,
            location: string("location"),
            date,
            // L'ATP écrit « Series », la WTA « Tier ».
            series: string("series").or_else(|| string("tier")),
            court: string("court"),
            surface,
            round,
            best_of,
            winner,
            loser,
            winner_rank: number("wrank").and_then(whole),
            loser_rank: number("lrank").and_then(whole),
            winner_points: number("wpts").and_then(whole),
            loser_points: number("lpts").and_then(whole),
            winner_sets: number("wsets")
                .and_then(whole)
                .and_then(|n| u8::try_from(n).ok()),
            loser_sets: number("lsets")
                .and_then(whole)
                .and_then(|n| u8::try_from(n).ok()),
            status,
            odds,
        },
        invalid,
    ))
}

/// Texte d'une cellule, sans espaces autour ; `None` si vide.
fn text(cell: &Data) -> Option<String> {
    let value = match cell {
        Data::String(s) => s.trim().to_string(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        _ => return None,
    };
    (!value.is_empty()).then_some(value)
}

/// Nombre d'une cellule : numérique, ou texte qui s'écrit comme un nombre (certaines années écrivent les rangs en texte).
fn number(cell: &Data) -> Option<f64> {
    let value = match cell {
        Data::Float(f) => *f,
        Data::Int(i) => *i as f64,
        Data::String(s) => s.trim().replace(',', ".").parse().ok()?,
        _ => return None,
    };
    value.is_finite().then_some(value)
}

/// Entier positif ou nul si le nombre est entier.
fn whole(value: f64) -> Option<u32> {
    (value >= 0.0 && value.fract() == 0.0 && value <= f64::from(u32::MAX)).then_some(value as u32)
}

/// Date d'une cellule : date Excel (nombre de jours depuis le 30/12/1899), ou texte `AAAA-MM-JJ` / `JJ/MM/AAAA`.
fn date(cell: &Data) -> Option<NaiveDate> {
    match cell {
        Data::DateTime(excel) => excel_date(excel.as_f64()),
        Data::Float(serial) => excel_date(*serial),
        Data::Int(serial) => excel_date(*serial as f64),
        Data::DateTimeIso(iso) | Data::String(iso) => {
            let trimmed = iso.trim();
            let day = trimmed.split(['T', ' ']).next().unwrap_or(trimmed);
            NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .or_else(|_| NaiveDate::parse_from_str(day, "%d/%m/%Y"))
                .ok()
        }
        _ => None,
    }
}

/// Numéro de série Excel → date. Excel compte les jours depuis le 30/12/1899 (le « 29/02/1900 » inexistant de Lotus
/// 1-2-3 est déjà absorbé par cette origine pour toute date postérieure à mars 1900).
fn excel_date(serial: f64) -> Option<NaiveDate> {
    if !(1.0..=2_958_465.0).contains(&serial) {
        return None;
    }
    NaiveDate::from_ymd_opt(1899, 12, 30)?.checked_add_days(Days::new(serial.floor() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI: &[u8] = include_bytes!("../../../tests/fixtures/tennis/mini_atp_2024.xlsx");

    fn parsed() -> ParsedFile {
        parse_workbook(MINI, Tour::Atp, 2024).unwrap()
    }

    #[test]
    fn the_fixture_gives_six_matches_and_two_rejections_with_their_reasons() {
        let file = parsed();

        assert_eq!(file.matches.len(), 6);
        assert_eq!(file.rejected.len(), 2);
        // Ligne 6 du tableur : surface « Wood » ; ligne 8 : vainqueur manquant.
        assert_eq!(file.rejected[0].row, 6);
        assert!(file.rejected[0].reason.contains("surface inconnue"));
        assert_eq!(file.rejected[1].row, 8);
        assert!(file.rejected[1].reason.contains("vainqueur"));
    }

    #[test]
    fn a_normal_match_carries_its_surface_its_date_and_all_its_odds() {
        let file = parsed();
        let first = &file.matches[0];

        assert_eq!(
            (first.winner.as_str(), first.loser.as_str()),
            ("Alpha A.", "Bravo B.")
        );
        assert_eq!(first.date, NaiveDate::from_ymd_opt(2024, 1, 7).unwrap());
        assert_eq!(first.surface, Surface::Hard);
        assert_eq!((first.best_of, first.status), (3, Status::Completed));
        assert_eq!((first.winner_rank, first.loser_rank), (Some(10), Some(50)));
        assert_eq!((first.winner_sets, first.loser_sets), (Some(2), Some(0)));
        let books: Vec<&str> = first.odds.iter().map(|o| o.bookmaker).collect();
        assert_eq!(books, vec!["B365", "PS", "Max", "Avg"]);
        let pinnacle = first.odds.iter().find(|o| o.bookmaker == "PS").unwrap();
        assert_eq!((pinnacle.winner.get(), pinnacle.loser.get()), (1.55, 2.60));
    }

    #[test]
    fn empty_odds_cells_give_no_odds_and_walkovers_and_retirements_are_recognised() {
        let file = parsed();

        let clay = &file.matches[1];
        assert_eq!(clay.surface, Surface::Clay);
        assert_eq!(
            clay.odds.iter().map(|o| o.bookmaker).collect::<Vec<_>>(),
            vec!["B365"]
        );
        assert_eq!(file.matches[2].status, Status::Walkover);
        assert_eq!(file.matches[2].winner_sets, None);
        assert_eq!(file.matches[3].status, Status::Retired);
    }

    #[test]
    fn an_invalid_odd_drops_that_bookmaker_only_and_is_counted() {
        let file = parsed();

        // Ligne 7 : Pinnacle à 1,00 ; le match est gardé avec Bet365 seul.
        let final_match = &file.matches[4];
        assert_eq!(final_match.winner, "Delta D.");
        assert_eq!(
            final_match
                .odds
                .iter()
                .map(|o| o.bookmaker)
                .collect::<Vec<_>>(),
            vec!["B365"]
        );
        assert_eq!(file.invalid_odds, 1);
    }

    #[test]
    fn a_date_written_as_text_is_read_and_best_of_five_is_kept() {
        let file = parsed();

        let slam = &file.matches[5];
        assert_eq!(slam.date, NaiveDate::from_ymd_opt(2024, 1, 14).unwrap());
        assert_eq!((slam.best_of, slam.surface), (5, Surface::Grass));
        assert_eq!(slam.series.as_deref(), Some("Grand Slam"));
    }

    #[test]
    fn excel_dates_and_text_dates_follow_their_formats() {
        assert_eq!(excel_date(45292.0), NaiveDate::from_ymd_opt(2024, 1, 1));
        assert_eq!(excel_date(45298.0), NaiveDate::from_ymd_opt(2024, 1, 7));
        assert_eq!(
            excel_date(45298.75),
            NaiveDate::from_ymd_opt(2024, 1, 7),
            "l'heure est ignorée"
        );
        assert_eq!(excel_date(0.0), None);
        assert_eq!(
            date(&Data::String("14/01/2024".into())),
            NaiveDate::from_ymd_opt(2024, 1, 14)
        );
        assert_eq!(
            date(&Data::String("2024-01-14 00:00:00".into())),
            NaiveDate::from_ymd_opt(2024, 1, 14)
        );
        assert_eq!(date(&Data::String("bientôt".into())), None);
    }

    #[test]
    fn a_file_that_is_not_a_spreadsheet_or_lacks_columns_is_refused_clearly() {
        let not_a_workbook =
            parse_workbook(b"ceci n'est pas un tableur", Tour::Atp, 2024).unwrap_err();
        assert!(not_a_workbook.to_string().contains("illisible"));
    }
}
