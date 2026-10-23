//! Historique officiel des tirages de la FDJ : téléchargement des archives ZIP et lecture de leurs fichiers CSV.
//!
//! La FDJ publie, sur les pages « Historique » de chaque jeu, une archive ZIP par période, qui contient un CSV (séparateur
//! `;`, virgule décimale) : boules, numéro chance ou étoiles, nombre de gagnants et gain unitaire par rang. Les archives
//! « en cours » sont mises à jour le soir de chaque tirage ; les autres ne changent plus. Choix de Paul du 2026-10-23 :
//! relevé automatique, fichiers publics, sans compte (`docs/sources.md` §44).

use std::io::{Cursor, Read};
use std::time::Duration;

// `Datelike` : trait qui donne `year_ce` à `NaiveDate` (un trait doit être importé pour que ses méthodes soient visibles).
use chrono::{Datelike, NaiveDate};

use rusqlite::Connection;

use super::{Codes, Draw, Game, RankResult, rules};
use crate::error::{Error, Result};
use crate::ingest::oddspapi::fetch::Pause;
use crate::ingest::snapshot::Clock;
use crate::store::loto as store;

/// Adresse commune des archives (identifiants lus sur les pages « Historique » de fdj.fr le 2026-10-23).
pub const BASE_URL: &str =
    "https://www.sto.api.fdj.fr/anonymous/service-draw-info/v3/documentations/";

/// Une archive de l'historique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Document {
    /// Identifiant de l'archive dans l'adresse.
    pub id: &'static str,
    /// Jeu.
    pub game: Game,
    /// Période couverte, telle que l'annonce la FDJ.
    pub label: &'static str,
    /// Archive en cours (mise à jour à chaque tirage) : relevée à chaque passage. Les autres ne le sont qu'une fois.
    pub current: bool,
}

/// Archives suivies. Le Loto d'avant mars 2017 (six rangs, autre grille de gains) et l'EuroMillions d'avant septembre 2016
/// (onze étoiles) ne sont pas lus : leurs rangs ne correspondent pas aux règles actuelles.
pub const DOCUMENTS: [Document; 8] = [
    Document {
        id: "1a2b3c4d-9876-4562-b3fc-2c963f66afp6",
        game: Game::Loto,
        label: "Loto de novembre 2019 à aujourd'hui",
        current: true,
    },
    Document {
        id: "1a2b3c4d-9876-4562-b3fc-2c963f66afo6",
        game: Game::Loto,
        label: "Loto de février 2019 à novembre 2019",
        current: false,
    },
    Document {
        id: "1a2b3c4d-9876-4562-b3fc-2c963f66afn6",
        game: Game::Loto,
        label: "Loto de mars 2017 à février 2019",
        current: false,
    },
    Document {
        id: "1a2b3c4d-9876-4562-b3fc-2c963f66afg6",
        game: Game::GrandLoto,
        label: "Grand Loto de décembre 2019 à aujourd'hui",
        current: true,
    },
    Document {
        id: "1a2b3c4d-9876-4562-b3fc-2c963f66aff6",
        game: Game::GrandLoto,
        label: "Grand Loto de décembre 2017 à décembre 2018",
        current: false,
    },
    Document {
        id: "1a2b3c4d-9876-4562-b3fc-2c963f66afe6",
        game: Game::EuroMillions,
        label: "EuroMillions de février 2020 à aujourd'hui",
        current: true,
    },
    Document {
        id: "1a2b3c4d-9876-4562-b3fc-2c963f66afd6",
        game: Game::EuroMillions,
        label: "EuroMillions de mars 2019 à février 2020",
        current: false,
    },
    Document {
        id: "1a2b3c4d-9876-4562-b3fc-2c963f66afc6",
        game: Game::EuroMillions,
        label: "EuroMillions de septembre 2016 à février 2019",
        current: false,
    },
];

/// Délai entre deux téléchargements : quelques fichiers par jour, on ménage le serveur.
pub const DOWNLOAD_COOLDOWN: Duration = Duration::from_secs(2);

/// Taille maximale d'une archive acceptée (les plus grosses font 190 Ko).
const MAX_ARCHIVE_BYTES: u64 = 20 * 1024 * 1024;

/// Fonction qui télécharge une adresse et renvoie le code HTTP et le corps, en octets (une archive est binaire).
pub type Download<'a> = &'a mut dyn FnMut(&str) -> Result<(u16, Vec<u8>)>;

/// Client HTTP bloquant des archives (aucune clé, aucun compte).
pub struct FdjClient {
    agent: ureq::Agent,
}

impl Default for FdjClient {
    fn default() -> Self {
        FdjClient::new()
    }
}

impl FdjClient {
    /// Crée le client. Le `User-Agent` nomme l'outil, sans donnée personnelle.
    pub fn new() -> FdjClient {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent("edge/0.1 (usage personnel, historique des tirages une fois par jour)")
            .build();
        FdjClient {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// `GET` d'une adresse ; le corps est lu en entier (dans la limite de 20 Mo).
    pub fn get(&self, url: &str) -> Result<(u16, Vec<u8>)> {
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|e| Error::Http(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_ARCHIVE_BYTES)
            .read_to_vec()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok((status, body))
    }
}

/// Extrait le premier fichier CSV d'une archive ZIP et le renvoie en texte.
///
/// Les fichiers anciens sont en Latin-1, les récents en UTF-8 : seuls les noms de colonnes contiennent des accents, et
/// aucun de ceux qu'on lit. La conversion avec remplacement (`from_utf8_lossy`) suffit donc.
pub fn csv_from_zip(bytes: &[u8]) -> Result<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| Error::Data(format!("archive FDJ illisible : {e}")))?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| Error::Data(format!("archive FDJ illisible : {e}")))?;
        if !file.name().to_lowercase().ends_with(".csv") {
            continue;
        }
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)
            .map_err(|e| Error::Data(format!("archive FDJ illisible : {e}")))?;
        return Ok(String::from_utf8_lossy(&raw).into_owned());
    }
    Err(Error::Data("archive FDJ sans fichier CSV".to_string()))
}

/// Lit le CSV d'une archive de `game`. Une ligne invalide arrête la lecture : on ne devine jamais une valeur.
pub fn parse_csv(text: &str, game: Game) -> Result<Vec<Draw>> {
    // `flexible` : les lignes finissent par un `;` de plus que l'en-tête dans certains fichiers.
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = reader
        .headers()?
        .iter()
        .map(|h| h.trim().to_string())
        .collect();
    let columns = Columns::find(&headers, game)?;
    let mut draws = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let record = record?;
        // Ligne 1 : l'en-tête ; la première ligne de données est la ligne 2 du fichier.
        let line = i + 2;
        let draw = columns
            .read(&record, game)
            .map_err(|e| Error::Data(format!("{game}, ligne {line} : {e}")))?;
        draws.push(draw);
    }
    Ok(draws)
}

/// Positions des colonnes utiles, trouvées par leur nom (l'ordre change d'une archive à l'autre).
struct Columns {
    date: usize,
    balls: [usize; 5],
    extra: Vec<usize>,
    winners: Vec<usize>,
    gains: Vec<usize>,
    codes: Option<(usize, usize)>,
}

impl Columns {
    fn find(headers: &[String], game: Game) -> Result<Columns> {
        let position = |name: &str| {
            headers
                .iter()
                .position(|h| h == name)
                .ok_or_else(|| Error::Data(format!("{game} : colonne « {name} » absente")))
        };
        let mut balls = [0; 5];
        for (i, slot) in balls.iter_mut().enumerate() {
            *slot = position(&format!("boule_{}", i + 1))?;
        }
        let ranks = 1..=game.rank_count();
        let (extra, winners, gains, codes) = match game {
            Game::Loto | Game::GrandLoto => (
                vec![position("numero_chance")?],
                ranks
                    .clone()
                    .map(|k| position(&format!("nombre_de_gagnant_au_rang{k}")))
                    .collect::<Result<Vec<_>>>()?,
                ranks
                    .map(|k| position(&format!("rapport_du_rang{k}")))
                    .collect::<Result<Vec<_>>>()?,
                Some((
                    position("nombre_de_codes_gagnants")?,
                    position("rapport_codes_gagnants")?,
                )),
            ),
            Game::EuroMillions => (
                vec![position("etoile_1")?, position("etoile_2")?],
                ranks
                    .clone()
                    .map(|k| {
                        position(&format!(
                            "nombre_de_gagnant_au_rang{k}_Euro_Millions_en_europe"
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?,
                ranks
                    .map(|k| position(&format!("rapport_du_rang{k}_Euro_Millions")))
                    .collect::<Result<Vec<_>>>()?,
                None,
            ),
        };
        Ok(Columns {
            date: position("date_de_tirage")?,
            balls,
            extra,
            winners,
            gains,
            codes,
        })
    }

    fn read(&self, record: &csv::StringRecord, game: Game) -> Result<Draw> {
        let field = |i: usize| record.get(i).map(str::trim).unwrap_or("");
        let date = parse_date(field(self.date))?;
        let (max_ball, max_extra) = match game {
            Game::Loto | Game::GrandLoto => (rules::LOTO_BALLS, rules::LOTO_CHANCES),
            Game::EuroMillions => (rules::EURO_BALLS, rules::EURO_STARS),
        };
        let mut balls = [0u8; 5];
        for (slot, &column) in balls.iter_mut().zip(&self.balls) {
            *slot = parse_number(field(column), max_ball)?;
        }
        balls.sort_unstable();
        if balls.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::Data(format!("boules répétées {balls:?}")));
        }
        let mut extra = self
            .extra
            .iter()
            .map(|&column| parse_number(field(column), max_extra))
            .collect::<Result<Vec<u8>>>()?;
        extra.sort_unstable();
        let ranks = self
            .winners
            .iter()
            .zip(&self.gains)
            .map(|(&w, &g)| {
                Ok(RankResult {
                    winners: parse_count(field(w))?,
                    unit_gain: parse_amount(field(g))?,
                })
            })
            .collect::<Result<Vec<RankResult>>>()?;
        let codes = match self.codes {
            Some((count, gain)) if !field(count).is_empty() => Some(Codes {
                count: u32::try_from(parse_count(field(count))?)
                    .map_err(|_| Error::Data("nombre de codes démesuré".to_string()))?,
                gain: parse_amount(field(gain))?,
            }),
            _ => None,
        };
        Ok(Draw {
            game,
            date,
            balls,
            extra,
            ranks,
            codes,
        })
    }
}

/// Date d'un tirage : `25/12/2025`, `23/09/16` ou `20110506` selon l'âge de l'archive.
fn parse_date(text: &str) -> Result<NaiveDate> {
    let parsed = if text.contains('/') {
        NaiveDate::parse_from_str(text, "%d/%m/%Y")
            .ok()
            .filter(|d| d.year_ce().1 >= 2000)
            .or_else(|| NaiveDate::parse_from_str(text, "%d/%m/%y").ok())
    } else {
        NaiveDate::parse_from_str(text, "%Y%m%d").ok()
    };
    parsed.ok_or_else(|| Error::Data(format!("date illisible « {text} »")))
}

/// Numéro de boule, de chance ou d'étoile, entre 1 et `max`.
fn parse_number(text: &str, max: u8) -> Result<u8> {
    text.parse::<u8>()
        .ok()
        .filter(|n| (1..=max).contains(n))
        .ok_or_else(|| Error::Data(format!("numéro « {text} » hors de 1 à {max}")))
}

/// Nombre de gagnants (entier positif ; un champ vide vaut 0).
fn parse_count(text: &str) -> Result<u64> {
    if text.is_empty() {
        return Ok(0);
    }
    text.replace(' ', "")
        .parse()
        .map_err(|_| Error::Data(format!("nombre de gagnants « {text} » illisible")))
}

/// Montant en euros à virgule décimale (`329291,5`) ; un champ vide vaut 0.
fn parse_amount(text: &str) -> Result<f64> {
    if text.is_empty() {
        return Ok(0.0);
    }
    text.replace(' ', "")
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| Error::Data(format!("montant « {text} » illisible")))
}

/// Bilan d'un passage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchReport {
    /// Archives lues, avec le nombre de tirages qu'elles contiennent.
    pub read: Vec<String>,
    /// Tirages nouveaux écrits en base.
    pub new_draws: usize,
    /// Tirages déjà connus publiés autrement (non réécrits).
    pub changed: usize,
    /// Archives non lues, avec la raison.
    pub failures: Vec<String>,
}

/// Lance un passage : les archives en cours, et les anciennes qui n'ont encore jamais été lues. Une archive illisible
/// n'arrête pas le passage : le téléchargement est journalisé avec son erreur, et les autres archives sont lues.
pub fn run(
    conn: &mut Connection,
    now: Clock,
    download: Download,
    pause: Pause,
) -> Result<FetchReport> {
    let mut report = FetchReport::default();
    let mut first = true;
    for document in DOCUMENTS {
        if !document.current && store::fetched_once(conn, document.id)? {
            continue;
        }
        if !first {
            pause(DOWNLOAD_COOLDOWN);
        }
        first = false;
        let at = now();
        let outcome = download(&format!("{BASE_URL}{}", document.id)).map(|(status, bytes)| {
            if status != 200 {
                return (status, Err(format!("réponse HTTP {status}")));
            }
            // Une archive illisible est une réponse reçue (HTTP 200) mais inutilisable : elle est journalisée comme telle.
            let draws = csv_from_zip(&bytes).and_then(|text| parse_csv(&text, document.game));
            (status, draws.map_err(|e| e.to_string()))
        });
        match outcome {
            Err(error) => {
                store::record_fetch(conn, at, document.id, 0, 0, Some(&error.to_string()))?;
                report
                    .failures
                    .push(format!("{} : {error}", document.label));
            }
            Ok((status, Err(error))) => {
                store::record_fetch(conn, at, document.id, status, 0, Some(&error))?;
                report
                    .failures
                    .push(format!("{} : {error}", document.label));
            }
            Ok((status, Ok(draws))) => {
                let fetch_id =
                    store::record_fetch(conn, at, document.id, status, draws.len(), None)?;
                let recorded = store::record_draws(conn, fetch_id, &draws)?;
                report.new_draws += recorded.new;
                report.changed += recorded.changed;
                report
                    .read
                    .push(format!("{} ({} tirages)", document.label, draws.len()));
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    /// Deux lignes réelles de l'archive Grand Loto (en-tête et Noël 2025, codes abrégés).
    pub(crate) const GRAND_LOTO_CSV: &str = "annee_numero_de_tirage;jour_de_tirage;date_de_tirage;date_de_forclusion;boule_1;boule_2;boule_3;boule_4;boule_5;numero_chance;combinaison_gagnante_en_ordre_croissant;nombre_de_gagnant_au_rang1;rapport_du_rang1;nombre_de_gagnant_au_rang2;rapport_du_rang2;nombre_de_gagnant_au_rang3;rapport_du_rang3;nombre_de_gagnant_au_rang4;rapport_du_rang4;nombre_de_gagnant_au_rang5;rapport_du_rang5;nombre_de_gagnant_au_rang6;rapport_du_rang6;nombre_de_gagnant_au_rang7;rapport_du_rang7;nombre_de_gagnant_au_rang8;rapport_du_rang8;nombre_de_gagnant_au_rang9;rapport_du_rang9;nombre_de_codes_gagnants;rapport_codes_gagnants;codes_gagnants;numero_jokerplus;devise\r\n25002;JEUDI;25/12/2025;26/03/2026;44;35;30;2;3;10;2-3-30-35-44+10;0;20000000;3;329291,5;48;12519,4;575;4012,6;2838;363,4;27376;170;44065;87,5;441494;36,2;625569;12,9;100;20000;P 3861 7396;;eur\r\n";

    /// Zippe `text` sous le nom `name`, comme les archives de la FDJ.
    pub(crate) fn zipped(name: &str, text: &str) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(text.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[test]
    fn a_real_grand_loto_line_is_read() {
        let draws = parse_csv(GRAND_LOTO_CSV, Game::GrandLoto).unwrap();
        assert_eq!(draws.len(), 1);
        let draw = &draws[0];
        assert_eq!(draw.date, NaiveDate::from_ymd_opt(2025, 12, 25).unwrap());
        assert_eq!(draw.balls, [2, 3, 30, 35, 44]);
        assert_eq!(draw.extra, vec![10]);
        assert_eq!(draw.ranks.len(), 9);
        assert_eq!(draw.ranks[0].winners, 0);
        assert_eq!(draw.ranks[1].winners, 3);
        assert_eq!(draw.ranks[1].unit_gain, 329_291.5);
        assert_eq!(draw.ranks[8].winners, 625_569);
        assert_eq!(
            draw.codes,
            Some(Codes {
                count: 100,
                gain: 20_000.0
            })
        );
    }

    #[test]
    fn an_archive_is_unzipped() {
        let bytes = zipped("grandloto_201912.csv", GRAND_LOTO_CSV);
        let text = csv_from_zip(&bytes).unwrap();
        assert_eq!(parse_csv(&text, Game::GrandLoto).unwrap().len(), 1);
        assert!(csv_from_zip(b"pas une archive").is_err());
    }

    #[test]
    fn dates_of_every_archive_age_are_read() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(parse_date("25/12/2025").unwrap(), day(2025, 12, 25));
        assert_eq!(parse_date("23/09/16").unwrap(), day(2016, 9, 23));
        assert_eq!(parse_date("20110506").unwrap(), day(2011, 5, 6));
        assert!(parse_date("2025-12-25").is_err());
    }

    #[test]
    fn an_impossible_value_stops_the_reading_with_its_line() {
        let bad = GRAND_LOTO_CSV.replace(";44;35;30;2;3;10;", ";44;35;30;2;3;12;");
        let err = parse_csv(&bad, Game::GrandLoto).unwrap_err().to_string();
        assert!(err.contains("ligne 2"), "{err}");
        let missing = GRAND_LOTO_CSV.replace("numero_chance", "chance");
        assert!(parse_csv(&missing, Game::GrandLoto).is_err());
    }

    #[test]
    fn amounts_use_the_decimal_comma_and_thousand_spaces() {
        assert_eq!(parse_amount("329291,5").unwrap(), 329_291.5);
        assert_eq!(parse_amount("6 036 389").unwrap(), 6_036_389.0);
        assert_eq!(parse_amount("").unwrap(), 0.0);
        assert!(parse_amount("-1").is_err());
        assert_eq!(parse_count("").unwrap(), 0);
    }

    #[test]
    fn a_pass_reads_current_archives_and_old_ones_only_once() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::store::migrate::migrate(&mut conn).unwrap();
        let archive = zipped("grandloto.csv", GRAND_LOTO_CSV);
        let mut calls = Vec::new();
        let mut download = |url: &str| {
            calls.push(url.to_string());
            if url.ends_with("afg6") {
                Ok((200, archive.clone()))
            } else if url.ends_with("aff6") {
                Ok((200, b"pas une archive".to_vec()))
            } else {
                Ok((404, Vec::new()))
            }
        };
        let mut clock = || "2026-10-23T20:30:00Z".parse().unwrap();
        let mut pauses = 0;
        let report = run(&mut conn, &mut clock, &mut download, &mut |_| pauses += 1).unwrap();
        assert_eq!(report.new_draws, 1);
        assert_eq!(report.read.len(), 1);
        assert_eq!(report.failures.len(), DOCUMENTS.len() - 1);
        assert_eq!(pauses, DOCUMENTS.len() - 1);

        // Second passage : rien de nouveau ; l'ancienne archive illisible est retentée, les autres anciennes aussi.
        let report = run(&mut conn, &mut clock, &mut download, &mut |_| ()).unwrap();
        assert_eq!(report.new_draws, 0);
        assert_eq!(calls.len(), 2 * DOCUMENTS.len());
    }
}
