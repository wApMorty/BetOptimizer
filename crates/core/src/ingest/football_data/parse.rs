//! Lecture des fichiers CSV de football-data.co.uk.
//!
//! Les fichiers ont des particularités que ce module absorbe (relevées dans `docs/sources.md`, section 12) :
//! BOM UTF-8 en tête, un fichier en Latin-1, dates à 2 ou 4 chiffres pour l'année, heures en heure du
//! Royaume-Uni, nombre de colonnes variable d'une saison à l'autre (on lit donc **par nom**, jamais par position),
//! lignes vides, cotes parfois absentes.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Europe::London;
use csv::StringRecord;

use crate::domain::{
    BookOdds, MatchRecord, MatchStats, Moment, Odds, Odds1x2, Outcome, TotalsOdds,
};
use crate::error::{Error, Result};

/// Préfixes de colonnes des bookmakers (clés de `notes.txt`). Pour le préfixe `PS`, les cotes d'avant-match
/// sont `PSH`/`PSD`/`PSA` et celles de clôture `PSCH`/`PSCD`/`PSCA`. `Avg` et `Max` sont la moyenne et la
/// meilleure cote du marché. Une liste explicite évite de prendre pour un bookmaker une colonne qui n'en est pas une.
pub const BOOKMAKER_PREFIXES: [&str; 25] = [
    "1XB", "B365", "BF", "BFD", "BFE", "BMGM", "BS", "BV", "BW", "CL", "GB", "IW", "LB", "PP",
    "PS", "SB", "SJ", "SK", "SKB", "SO", "SY", "VC", "WH", "Avg", "Max",
];

/// Bookmakers du marché plus/moins 2,5 buts : `(nom stocké, préfixe de colonne)`. Chez football-data, Pinnacle s'écrit
/// `P` pour ce marché (`P>2.5`, `PC>2.5`) alors qu'il s'écrit `PS` pour le 1X2 : on stocke `PS` dans les deux cas.
const TOTALS_BOOKMAKERS: [(&str, &str); 4] = [
    ("PS", "P"),
    ("Avg", "Avg"),
    ("B365", "B365"),
    ("Max", "Max"),
];

/// Ligne des cotes plus/moins publiées.
const TOTALS_LINE: f64 = 2.5;

/// Un match lu dans un fichier, avec les cotes de tous les bookmakers présents.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMatch {
    /// Résultat, statistiques, noms bruts.
    pub record: MatchRecord,
    /// Cotes 1X2 par bookmaker et par moment.
    pub odds: Vec<BookOdds>,
    /// Cotes plus/moins 2,5 buts par bookmaker et par moment (depuis 2019/20).
    pub totals: Vec<TotalsOdds>,
}

/// Contenu d'un fichier CSV, avec le décompte de ce qui a été écarté.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedFile {
    /// Matchs terminés et complets, dans l'ordre du fichier.
    pub matches: Vec<ParsedMatch>,
    /// Lignes ignorées faute de date, d'équipe, de score ou de résultat.
    pub incomplete_rows: usize,
    /// Triplets de cotes écartés parce qu'une valeur est invalide (≤ 1,0).
    pub dropped_odds: usize,
    /// Lignes ignorées parce que le même match (date, équipes) figure déjà dans le fichier.
    pub duplicate_rows: usize,
}

/// Décode le fichier : UTF-8 si possible, sinon Latin-1 ; retire le BOM éventuel.
///
/// En Latin-1, chaque octet est exactement le code Unicode du même numéro : la conversion est `char::from`.
pub fn decode(bytes: &[u8]) -> String {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|&byte| char::from(byte)).collect(),
    };
    text.trim_start_matches('\u{feff}').to_string()
}

/// Lit un fichier CSV complet.
pub fn parse_csv(bytes: &[u8]) -> Result<ParsedFile> {
    let text = decode(bytes);
    // `flexible` : tolère des lignes plus courtes ou plus longues que l'en-tête (cela arrive en bout de fichier).
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: HashMap<String, usize> = reader
        .headers()?
        .iter()
        .enumerate()
        .map(|(index, name)| (name.trim().to_string(), index))
        .collect();

    let mut parsed = ParsedFile::default();
    let mut seen = HashSet::new();
    for record in reader.records() {
        let record = record?;
        let row = Row {
            headers: &headers,
            record: &record,
        };
        match parse_row(&row, &mut parsed.dropped_odds)? {
            RowOutcome::Blank => {}
            RowOutcome::Incomplete => parsed.incomplete_rows += 1,
            RowOutcome::Match(parsed_match) => {
                let parsed_match = *parsed_match;
                let key = (
                    parsed_match.record.date,
                    parsed_match.record.home_team.clone(),
                    parsed_match.record.away_team.clone(),
                );
                // `insert` renvoie `false` si la clé était déjà présente : c'est un doublon.
                if seen.insert(key) {
                    parsed.matches.push(parsed_match);
                } else {
                    parsed.duplicate_rows += 1;
                }
            }
        }
    }
    Ok(parsed)
}

/// Une ligne du CSV avec accès aux cellules par nom de colonne.
struct Row<'a> {
    headers: &'a HashMap<String, usize>,
    record: &'a StringRecord,
}

impl Row<'_> {
    /// Texte d'une cellule, sans espaces ; `None` si la colonne n'existe pas ou si la cellule est vide.
    fn get(&self, name: &str) -> Option<&str> {
        let index = *self.headers.get(name)?;
        self.record
            .get(index)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// Valeur numérique d'une cellule ; `None` si absente ou illisible.
    fn number<T: FromStr>(&self, name: &str) -> Option<T> {
        self.get(name)?.parse().ok()
    }
}

enum RowOutcome {
    Blank,
    Incomplete,
    // Boîte : `ParsedMatch` est volumineux (cotes de plusieurs bookmakers) et les autres variantes sont vides ;
    // sans `Box`, chaque `RowOutcome` occuperait toujours la taille de la plus grande variante.
    Match(Box<ParsedMatch>),
}

fn parse_row(row: &Row, dropped_odds: &mut usize) -> Result<RowOutcome> {
    let (date, home, away) = (row.get("Date"), row.get("HomeTeam"), row.get("AwayTeam"));
    if date.is_none() && home.is_none() && away.is_none() {
        return Ok(RowOutcome::Blank);
    }
    let result = row.get("FTR").and_then(parse_result);
    let (Some(date), Some(home), Some(away), Some(ft_home), Some(ft_away), Some(ft_result)) = (
        date.and_then(parse_date),
        home,
        away,
        row.number::<u8>("FTHG"),
        row.number::<u8>("FTAG"),
        result,
    ) else {
        return Ok(RowOutcome::Incomplete);
    };

    let kickoff_utc = match row.get("Time").and_then(parse_time) {
        Some(time) => Some(uk_local_to_utc(date, time)?),
        None => None,
    };
    let record = MatchRecord {
        date,
        kickoff_utc,
        home_team: home.to_string(),
        away_team: away.to_string(),
        ft_home,
        ft_away,
        ft_result,
        ht_home: row.number("HTHG"),
        ht_away: row.number("HTAG"),
        referee: row.get("Referee").map(str::to_string),
        stats: MatchStats {
            shots_home: row.number("HS"),
            shots_away: row.number("AS"),
            shots_on_target_home: row.number("HST"),
            shots_on_target_away: row.number("AST"),
            corners_home: row.number("HC"),
            corners_away: row.number("AC"),
            fouls_home: row.number("HF"),
            fouls_away: row.number("AF"),
            yellow_home: row.number("HY"),
            yellow_away: row.number("AY"),
            red_home: row.number("HR"),
            red_away: row.number("AR"),
        },
        xg_home: row.number("HxG"),
        xg_away: row.number("AxG"),
    };
    let odds = extract_odds(row, dropped_odds);
    let totals = extract_totals(row, dropped_odds);
    Ok(RowOutcome::Match(Box::new(ParsedMatch {
        record,
        odds,
        totals,
    })))
}

fn parse_result(text: &str) -> Option<Outcome> {
    match text {
        "H" => Some(Outcome::Home),
        "D" => Some(Outcome::Draw),
        "A" => Some(Outcome::Away),
        _ => None,
    }
}

/// `dd/mm/yy` (2016/17) ou `dd/mm/yyyy` (ensuite).
fn parse_date(text: &str) -> Option<NaiveDate> {
    let mut parts = text.split('/');
    let day = parts.next()?.trim().parse().ok()?;
    let month = parts.next()?.trim().parse().ok()?;
    let year: i32 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let year = if year < 100 { 2000 + year } else { year };
    NaiveDate::from_ymd_opt(year, month, day)
}

fn parse_time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text, "%H:%M").ok()
}

/// Convertit une date et une heure locales du Royaume-Uni (GMT l'hiver, BST l'été) en UTC.
///
/// Les heures de football-data sont britanniques pour toutes les ligues (la Bundesliga à 14:30 est le
/// 15:30 allemand). `earliest()` choisit la première lecture d'une heure ambiguë (changement d'heure d'automne) ;
/// il ne renvoie `None` que pour une heure qui n'existe pas (changement de printemps), donc une donnée fausse.
fn uk_local_to_utc(date: NaiveDate, time: NaiveTime) -> Result<DateTime<Utc>> {
    London
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .map(|local| local.with_timezone(&Utc))
        .ok_or_else(|| {
            Error::Data(format!(
                "heure locale inexistante au Royaume-Uni : {date} {time}"
            ))
        })
}

/// Cherche, pour chaque bookmaker connu, un triplet complet de cotes d'avant-match puis de clôture.
fn extract_odds(row: &Row, dropped_odds: &mut usize) -> Vec<BookOdds> {
    let mut found = Vec::new();
    for prefix in BOOKMAKER_PREFIXES {
        for (moment, suffixes) in [
            (Moment::Pre, ["H", "D", "A"]),
            (Moment::Close, ["CH", "CD", "CA"]),
        ] {
            let values: Vec<Option<f64>> = suffixes
                .iter()
                .map(|suffix| row.number(&format!("{prefix}{suffix}")))
                .collect();
            // Motif sur tranche : n'accepte que les triplets entièrement présents. Un bookmaker absent
            // du fichier (ou de cette saison) n'est pas une anomalie : on passe en silence.
            let [Some(home), Some(draw), Some(away)] = values.as_slice() else {
                continue;
            };
            match (Odds::new(*home), Odds::new(*draw), Odds::new(*away)) {
                (Some(home), Some(draw), Some(away)) => found.push(BookOdds {
                    bookmaker: prefix.to_string(),
                    moment,
                    odds: Odds1x2 { home, draw, away },
                }),
                // Triplet présent mais invalide : on l'écarte et on le compte (jamais corrigé en silence).
                _ => *dropped_odds += 1,
            }
        }
    }
    found
}

/// Cherche, pour chaque bookmaker, les cotes plus/moins 2,5 d'avant-match (`P>2.5`, `P<2.5`) puis de clôture
/// (`PC>2.5`, `PC<2.5`). Un couple invalide (≤ 1,0) est écarté et compté avec les autres cotes écartées.
fn extract_totals(row: &Row, dropped_odds: &mut usize) -> Vec<TotalsOdds> {
    let mut found = Vec::new();
    for (bookmaker, prefix) in TOTALS_BOOKMAKERS {
        for (moment, infix) in [(Moment::Pre, ""), (Moment::Close, "C")] {
            let over: Option<f64> = row.number(&format!("{prefix}{infix}>2.5"));
            let under: Option<f64> = row.number(&format!("{prefix}{infix}<2.5"));
            let (Some(over), Some(under)) = (over, under) else {
                continue;
            };
            match (Odds::new(over), Odds::new(under)) {
                (Some(over), Some(under)) => found.push(TotalsOdds {
                    bookmaker: bookmaker.to_string(),
                    moment,
                    line: TOTALS_LINE,
                    over,
                    under,
                }),
                _ => *dropped_odds += 1,
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use approx::assert_abs_diff_eq;

    use super::*;

    const E0_2526: &[u8] = include_bytes!("../../../tests/fixtures/fd/E0_2526_sample.csv");
    const E0_2627: &[u8] = include_bytes!("../../../tests/fixtures/fd/E0_2627_sample.csv");
    const E0_1617: &[u8] = include_bytes!("../../../tests/fixtures/fd/E0_1617_sample.csv");
    const SC0_1819: &[u8] = include_bytes!("../../../tests/fixtures/fd/SC0_1819_latin1_sample.csv");

    fn odds_of<'a>(m: &'a ParsedMatch, bookmaker: &str, moment: Moment) -> Option<&'a Odds1x2> {
        m.odds
            .iter()
            .find(|o| o.bookmaker == bookmaker && o.moment == moment)
            .map(|o| &o.odds)
    }

    #[test]
    fn season_2025_26_with_bom_stats_and_pinnacle() {
        let parsed = parse_csv(E0_2526).unwrap();
        assert_eq!(parsed.matches.len(), 5);
        assert_eq!(parsed.incomplete_rows, 0);

        let first = &parsed.matches[0];
        assert_eq!(first.record.home_team, "Liverpool");
        assert_eq!(first.record.away_team, "Bournemouth");
        assert_eq!((first.record.ft_home, first.record.ft_away), (4, 2));
        assert_eq!(first.record.ft_result, Outcome::Home);
        assert_eq!(
            (first.record.ht_home, first.record.ht_away),
            (Some(1), Some(0))
        );
        assert_eq!(first.record.referee.as_deref(), Some("A Taylor"));
        assert_eq!(first.record.stats.shots_home, Some(19));
        assert_eq!(first.record.stats.shots_on_target_away, Some(3));
        assert_eq!(first.record.stats.yellow_away, Some(2));
        assert_eq!(first.record.xg_home, None);

        // Pinnacle : relevé d'avant-match et clôture.
        let pre = odds_of(first, "PS", Moment::Pre).unwrap();
        assert_eq!(pre.home.get(), 1.28);
        assert_eq!(pre.draw.get(), 6.56);
        assert_eq!(pre.away.get(), 9.07);
        assert_eq!(
            odds_of(first, "PS", Moment::Close).unwrap().home.get(),
            1.29
        );
        assert!(odds_of(first, "Avg", Moment::Close).is_some());
        assert!(odds_of(first, "Max", Moment::Pre).is_some());
    }

    #[test]
    fn pinnacle_disappears_in_january_2026_but_the_market_average_stays() {
        let parsed = parse_csv(E0_2526).unwrap();
        let after_cut = &parsed.matches[3];
        assert_eq!(after_cut.record.home_team, "Man United");
        assert!(odds_of(after_cut, "PS", Moment::Pre).is_none());
        assert!(odds_of(after_cut, "PS", Moment::Close).is_none());
        assert!(odds_of(after_cut, "Avg", Moment::Close).is_some());
    }

    #[test]
    fn uk_kickoff_is_converted_to_utc_in_summer_and_in_winter() {
        let parsed = parse_csv(E0_2526).unwrap();
        // 15/08/2025 20:00 à Londres (BST, UTC+1) = 19:00 UTC.
        assert_eq!(
            parsed.matches[0].record.kickoff_utc.unwrap().to_rfc3339(),
            "2025-08-15T19:00:00+00:00"
        );
        // 17/01/2026 12:30 (GMT, UTC+0) = 12:30 UTC.
        assert_eq!(
            parsed.matches[3].record.kickoff_utc.unwrap().to_rfc3339(),
            "2026-01-17T12:30:00+00:00"
        );
    }

    #[test]
    fn over_under_odds_are_read_for_pinnacle_average_and_bet365_until_pinnacle_disappears() {
        let parsed = parse_csv(E0_2526).unwrap();
        let totals = |m: &ParsedMatch, bookmaker: &str, moment: Moment| {
            m.totals
                .iter()
                .find(|t| t.bookmaker == bookmaker && t.moment == moment)
                .cloned()
        };

        let before_cut = &parsed.matches[0];
        let pinnacle = totals(before_cut, "PS", Moment::Close).unwrap();
        assert_eq!(pinnacle.line, 2.5);
        assert!(pinnacle.over.get() > 1.0 && pinnacle.under.get() > 1.0);
        assert!(totals(before_cut, "Avg", Moment::Pre).is_some());
        assert!(totals(before_cut, "B365", Moment::Close).is_some());

        // Après janvier 2026 : plus de Pinnacle, mais toujours la moyenne du marché.
        let after_cut = &parsed.matches[3];
        assert!(totals(after_cut, "PS", Moment::Close).is_none());
        assert!(totals(after_cut, "Avg", Moment::Close).is_some());
        // Les saisons d'avant 2019/20 n'ont pas ces colonnes.
        assert!(parse_csv(E0_1617).unwrap().matches[0].totals.is_empty());
    }

    #[test]
    fn season_2026_27_brings_xg_and_new_bookmakers_without_pinnacle() {
        let parsed = parse_csv(E0_2627).unwrap();
        assert_eq!(parsed.matches.len(), 3);
        let first = &parsed.matches[0];
        assert_abs_diff_eq!(first.record.xg_home.unwrap(), 1.88);
        assert_abs_diff_eq!(first.record.xg_away.unwrap(), 0.2);
        assert!(odds_of(first, "PP", Moment::Pre).is_some());
        assert!(odds_of(first, "SKB", Moment::Close).is_some());
        assert!(odds_of(first, "PS", Moment::Pre).is_none());
    }

    #[test]
    fn season_2016_17_has_two_digit_years_and_no_kickoff_time() {
        let parsed = parse_csv(E0_1617).unwrap();
        let first = &parsed.matches[0];
        assert_eq!(
            first.record.date,
            NaiveDate::from_ymd_opt(2016, 8, 13).unwrap()
        );
        assert_eq!(first.record.kickoff_utc, None);
        // Dès 2016/17, la clôture Pinnacle existe, mais pas la moyenne du marché.
        assert!(odds_of(first, "PS", Moment::Close).is_some());
        assert!(odds_of(first, "Avg", Moment::Close).is_none());
    }

    #[test]
    fn a_latin1_file_is_decoded_instead_of_failing() {
        assert!(
            String::from_utf8(SC0_1819.to_vec()).is_err(),
            "la fixture doit bien être non UTF-8"
        );
        let parsed = parse_csv(SC0_1819).unwrap();
        assert_eq!(parsed.matches.len(), 3);
        // Cette ligne n'a pas de cote Pinnacle d'avant-match mais une clôture : les deux moments sont indépendants.
        let second = &parsed.matches[1];
        assert!(odds_of(second, "PS", Moment::Pre).is_none());
        assert!(odds_of(second, "PS", Moment::Close).is_some());
    }

    #[test]
    fn decode_handles_bom_utf8_and_latin1() {
        assert_eq!(decode(b"\xef\xbb\xbfDiv,Date"), "Div,Date");
        assert_eq!(decode("Málaga".as_bytes()), "Málaga");
        assert_eq!(decode(b"M\xe1laga"), "Málaga");
    }

    const EDGE_CASES: &str = "\
Div,Date,Time,HomeTeam,AwayTeam,FTHG,FTAG,FTR,PSH,PSD,PSA,AvgH,AvgD,AvgA
E0,16/08/2025,15:00,Brighton,Fulham,1,1,D,1.94,3.73,3.98,1.92,3.8,4.0
E0,17/08/2025,15:00,Chelsea,Brentford,,,,1.5,4.0,6.0,1.5,4.0,6.0
E0,18/08/2025,15:00,Everton,Leeds,2,0,H,1.0,3.5,4.5,2.0,3.5,4.0
,,,,,,,,,,,,,
E0,16/08/2025,15:00,Brighton,Fulham,1,1,D,1.94,3.73,3.98,1.92,3.8,4.0
E0,10/01/2026,15:00,Spurs,Villa,0,3,A,,,,2.0,3.4,3.9
E0,31/12/2025,xx:yy,Wolves,Burnley,1,0,X,,,,,,
E0,not-a-date,15:00,Wolves,Burnley,1,0,H,,,,,,
";

    #[test]
    fn edge_cases_are_counted_never_silently_fixed() {
        let parsed = parse_csv(EDGE_CASES.as_bytes()).unwrap();

        // Gardés : Brighton–Fulham, Everton–Leeds, Spurs–Villa.
        assert_eq!(parsed.matches.len(), 3);
        // Chelsea (score absent), Wolves (résultat « X »), Wolves (date illisible).
        assert_eq!(parsed.incomplete_rows, 3);
        // La ligne vide n'est ni gardée ni comptée ; le 2e Brighton–Fulham est un doublon.
        assert_eq!(parsed.duplicate_rows, 1);
        // Everton–Leeds : Pinnacle à 1,0 est invalide, écarté et compté ; la moyenne, valide, reste.
        assert_eq!(parsed.dropped_odds, 1);
        let everton = &parsed.matches[1];
        assert!(odds_of(everton, "PS", Moment::Pre).is_none());
        assert!(odds_of(everton, "Avg", Moment::Pre).is_some());
    }

    #[test]
    fn winter_and_summer_kickoffs_on_the_same_clock_time_differ_by_one_hour() {
        let parsed = parse_csv(EDGE_CASES.as_bytes()).unwrap();
        let summer = parsed.matches[0].record.kickoff_utc.unwrap(); // 16/08/2025 15:00 BST
        let winter = parsed.matches[2].record.kickoff_utc.unwrap(); // 10/01/2026 15:00 GMT
        assert_eq!(summer.format("%H:%M").to_string(), "14:00");
        assert_eq!(winter.format("%H:%M").to_string(), "15:00");
    }

    #[test]
    fn a_missing_odds_column_is_not_an_anomaly() {
        let csv = "Div,Date,HomeTeam,AwayTeam,FTHG,FTAG,FTR\nE0,16/08/2025,A,B,1,0,H\n";
        let parsed = parse_csv(csv.as_bytes()).unwrap();
        assert_eq!(parsed.matches.len(), 1);
        assert!(parsed.matches[0].odds.is_empty());
        assert_eq!(parsed.dropped_odds, 0);
    }

    #[test]
    fn columns_are_read_by_name_not_by_position() {
        let reordered = "FTR,FTAG,FTHG,AwayTeam,HomeTeam,Date\nA,3,1,B,A,02/02/2024\n";
        let parsed = parse_csv(reordered.as_bytes()).unwrap();
        let record = &parsed.matches[0].record;
        assert_eq!(
            (record.home_team.as_str(), record.away_team.as_str()),
            ("A", "B")
        );
        assert_eq!((record.ft_home, record.ft_away), (1, 3));
        assert_eq!(record.ft_result, Outcome::Away);
    }
}
