//! Lecture des pages d'historique de WePari (wepari.fr), site personnel qui recense les cotes boostées de Winamax et
//! de Parions Sport depuis 2021, avec la cote d'origine, la cote boostée et l'issue.
//!
//! Les pages sont téléchargées une fois à la main (voir `docs/sources.md`) ; ce module ne fait aucune requête.

use chrono::NaiveDateTime;
use scraper::{ElementRef, Html, Selector};

use super::Operator;
use crate::domain::{Euros, Odds};
use crate::error::{Error, Result};

/// Issue d'une cote boostée selon WePari (couleur de la cellule de la cote : vert gagné, orange perdu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// Pari gagné.
    Won,
    /// Pari perdu.
    Lost,
    /// Issue non publiée (WePari ne règle plus les paris depuis début 2025).
    Unknown,
}

/// Une cote boostée telle que publiée, sans interprétation.
#[derive(Debug, Clone, PartialEq)]
pub struct WepariBoost {
    /// Opérateur.
    pub operator: Operator,
    /// Intitulé du pari (« Lyon gagne et les deux équipes marquent »).
    pub description: String,
    /// Intitulé de l'événement (« Cote Boostée : Lyon - Nantes », « Lyon-Nantes - Cotes Boostées Ligue 1 »).
    pub event: String,
    /// Fin de la prise de paris, heure de Paris (le coup d'envoi pour un match).
    pub closes_at: NaiveDateTime,
    /// Cote boostée.
    pub boosted: Odds,
    /// Cote d'origine, avant le boost.
    pub base: Odds,
    /// Mise maximale (publiée pour Parions Sport seulement).
    pub max_stake: Option<Euros>,
    /// Issue selon WePari.
    pub settlement: Settlement,
}

/// Résultat de la lecture d'une page : les boosts lus et le nombre de lignes illisibles (cote absente…).
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// Boosts lus, dans l'ordre de la page (du plus récent au plus ancien).
    pub boosts: Vec<WepariBoost>,
    /// Lignes du tableau qui n'ont pas pu être lues.
    pub malformed: usize,
}

/// Une cellule du tableau : son texte (un morceau par nœud texte) et sa couleur de fond.
struct Cell {
    texts: Vec<String>,
    background: Option<String>,
}

impl Cell {
    /// Texte complet de la cellule, espaces insécables remplacés.
    fn text(&self) -> String {
        clean(&self.texts.join(" "))
    }
}

/// Lit une page d'historique. La mise en page dit l'opérateur : la page de Parions Sport a une colonne « Mise max ».
pub fn parse(html: &str) -> Result<Page> {
    let document = Html::parse_document(html);
    // Sélecteur constant : sa syntaxe est vérifiée par les tests de ce module.
    let rows = Selector::parse(r#"table[width="95%"] tr"#).expect("sélecteur CSS constant valide");
    let cells = Selector::parse("td").expect("sélecteur CSS constant valide");

    let mut operator = None;
    let mut boosts = Vec::new();
    let mut malformed = 0;
    for row in document.select(&rows) {
        let row_cells: Vec<Cell> = row.select(&cells).map(cell_of).collect();
        if row.value().attr("bgcolor") == Some("black") {
            // Ligne d'en-tête.
            let header: String = row_cells.iter().map(Cell::text).collect();
            operator = Some(if header.contains("Mise max") {
                Operator::ParionsSport
            } else {
                Operator::Winamax
            });
            continue;
        }
        let Some(operator) = operator else {
            return Err(Error::Data(
                "page WePari : ligne de données avant l'en-tête du tableau".to_string(),
            ));
        };
        match read_row(operator, &row_cells) {
            Some(boost) => boosts.push(boost),
            None => malformed += 1,
        }
    }
    if operator.is_none() {
        return Err(Error::Data(
            "page WePari : tableau d'historique introuvable".to_string(),
        ));
    }
    Ok(Page { boosts, malformed })
}

/// Textes et couleur d'une cellule. Les textes sont des `&str` empruntés au document : on les copie (`to_string`)
/// parce que la `Cell` doit survivre à l'itération sur le document.
fn cell_of(element: ElementRef<'_>) -> Cell {
    Cell {
        texts: element
            .text()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        background: element.value().attr("bgcolor").map(str::to_string),
    }
}

/// Une ligne de données, selon la mise en page de l'opérateur :
/// - Winamax : pari, cote boostée, cote d'origine, boost %, fin du pari, événement, gain net ;
/// - Parions Sport : pari, cote boostée et cote d'origine (même cellule), boost %, fin du pari, événement, mise max,
///   gain net.
fn read_row(operator: Operator, cells: &[Cell]) -> Option<WepariBoost> {
    let (boosted, base, closes, event, max_stake) = match operator {
        Operator::Winamax => {
            let boosted = cells.get(1)?.text();
            let base = cells.get(2)?.text();
            (boosted, base, cells.get(4)?, cells.get(5)?, None)
        }
        Operator::ParionsSport => {
            let odds = cells.get(1)?;
            let boosted = odds.texts.first()?.clone();
            let base = odds.texts.get(1)?.clone();
            let stake = parse_stake(&cells.get(5)?.text());
            (boosted, base, cells.get(3)?, cells.get(4)?, stake)
        }
    };
    let settlement = match cells.get(1)?.background.as_deref() {
        Some("green") => Settlement::Won,
        Some("orange") => Settlement::Lost,
        _ => Settlement::Unknown,
    };
    Some(WepariBoost {
        operator,
        description: cells.first()?.text(),
        event: event.text(),
        closes_at: NaiveDateTime::parse_from_str(&closes.text(), "%d-%m-%Y %H:%M").ok()?,
        boosted: Odds::new(boosted.parse().ok()?)?,
        base: Odds::new(base.parse().ok()?)?,
        max_stake,
        settlement,
    })
}

/// « 25 € » → 25 €.
fn parse_stake(text: &str) -> Option<Euros> {
    let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
    Euros::new(digits.parse().ok()?)
}

/// Remplace les espaces insécables par des espaces et réduit les suites d'espaces à une seule.
pub(crate) fn clean(text: &str) -> String {
    text.replace('\u{a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINAMAX: &str = r##"<html><body><table width="95%"><tr bgcolor="black">
<td><b>Pari</b></td><td><b>Cote boostée</b></td><td><b>Cote non boostée</b></td><td><b>Boost %</b></td>
<td><b>Fin pari</b></td><td><b>Sport</b></td><td><b>Gain net</b></td></tr>
<tr><td bgcolor="lightgray"> Lyon gagne et les deux équipes marquent</td><td bgcolor="green">3.00</td>
<td bgcolor="#eeeeee">2.55</td><td>+ 18 %</td><td>24-09-2023 20:45</td>
<td>Cote Boostée : Lyon&nbsp;- Nantes</td><td>100.00</td></tr>
<tr><td> Plus de 2,5 buts</td><td bgcolor="orange">2.40</td><td>2.10</td><td>+ 14 %</td><td>24-09-2023 17:00</td>
<td>Cote Boostée : Lens - Metz</td><td>-50.00</td></tr>
<tr><td> Cote absente</td><td bgcolor="lightgray">-</td><td>2.10</td><td></td><td>24-09-2023 17:00</td>
<td>Cote Boostée : Lens - Metz</td><td></td></tr>
</table></body></html>"##;

    const PARIONS_SPORT: &str = r##"<table width="95%"><tr bgcolor="black">
<td><b>Pari</b></td><td><b>Cote boostée<br><small>Sans boost</small></b></td><td><b>Boost %</b></td>
<td><b>Fin pari</b></td><td><b>Sport</b></td><td><b>Mise max</b></td><td><b>Gain net</b></td></tr>
<tr><td bgcolor="lightgray"> Guingamp gagne ? (2,65 => 3,10 / Mise max 25 €)</td>
<td bgcolor="lightgray">3.10<br><small>2.65</small></td><td>+ 17 %</td><td>09-08-2025 14:00</td>
<td>Guingamp-Le Mans - Cotes Boostées Ligue 2 BKT</td><td>25 €</td><td></td></tr></table>"##;

    #[test]
    fn winamax_rows_are_read_with_their_settlement() {
        let page = parse(WINAMAX).unwrap();
        assert_eq!(page.boosts.len(), 2);
        assert_eq!(page.malformed, 1);
        let first = &page.boosts[0];
        assert_eq!(first.operator, Operator::Winamax);
        assert_eq!(first.description, "Lyon gagne et les deux équipes marquent");
        // L'espace insécable de l'événement est normalisé.
        assert_eq!(first.event, "Cote Boostée : Lyon - Nantes");
        assert_eq!(first.boosted.get(), 3.0);
        assert_eq!(first.base.get(), 2.55);
        assert_eq!(first.settlement, Settlement::Won);
        assert_eq!(first.max_stake, None);
        assert_eq!(page.boosts[1].settlement, Settlement::Lost);
    }

    #[test]
    fn parions_sport_rows_carry_the_max_stake() {
        let page = parse(PARIONS_SPORT).unwrap();
        assert_eq!(page.boosts.len(), 1);
        let boost = &page.boosts[0];
        assert_eq!(boost.operator, Operator::ParionsSport);
        assert_eq!(boost.boosted.get(), 3.1);
        assert_eq!(boost.base.get(), 2.65);
        assert_eq!(boost.max_stake.map(Euros::get), Some(25.0));
        assert_eq!(boost.settlement, Settlement::Unknown);
    }

    #[test]
    fn a_page_without_the_history_table_is_refused() {
        assert!(parse("<html><body><p>rien</p></body></html>").is_err());
    }
}
