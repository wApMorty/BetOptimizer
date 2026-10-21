//! `edge bonus …` : bonus de bienvenue couverts (`docs/bonus-bienvenue.md`).
//!
//! Le calcul et les règles vivent dans `edge_core::bonus` ; ce fichier lit les options, appelle le cœur et affiche.

use anyhow::{Context, Result, bail};
use chrono::{Local, NaiveDate, Utc};
use edge_core::bonus::cover::cover;
use edge_core::bonus::pick::Candidate;
use edge_core::bonus::tracking::{
    self, Board, OfferState, OperationRequest, prepare_operation, record_operation,
};
use edge_core::bonus::{OfferKind, is_operator_key, operator_name};
use edge_core::config::Config;
use edge_core::domain::{Euros, Odds, Outcome};
use edge_core::format;
use edge_core::ingest::aliases::Aliases;
use edge_core::paths::Paths;
use edge_core::report::recommend_report::league_name;
use edge_core::store::bonus::{self, Eligibility, Leg, MatchResult, NewOffer, Role};
use edge_core::store::{self, Connection};

/// Options de `edge bonus offer`.
pub struct OfferOptions {
    /// Clé de l'opérateur.
    pub bookmaker: String,
    /// Forme : `cash_refund`, `freebet_refund` ou `freebet` (absente si `ineligible`).
    pub kind: Option<String>,
    /// Montant maximal.
    pub amount: Option<f64>,
    /// Cote minimale imposée par l'offre.
    pub min_odds: Option<f64>,
    /// Offre inaccessible : le compte ne sert qu'à couvrir.
    pub ineligible: bool,
    /// Où l'éligibilité a été vérifiée.
    pub source: String,
    /// Date de la vérification (par défaut : aujourd'hui).
    pub checked_on: Option<NaiveDate>,
    /// Remarque libre.
    pub note: Option<String>,
}

/// Options de `edge bonus cover`.
pub struct CoverOptions {
    /// Forme de l'offre.
    pub kind: String,
    /// Montant du bonus.
    pub amount: f64,
    /// Cote du pari bonus.
    pub odds: f64,
    /// Cotes de couverture des autres issues.
    pub covers: Vec<f64>,
    /// Valeur d'un euro de freebet (par défaut : celle de la configuration).
    pub freebet_value: Option<f64>,
}

/// Options de `edge bonus log`.
pub struct LogOptions {
    /// Opérateur du bonus.
    pub bookmaker: String,
    /// Forme de l'opération (par défaut : déduite de l'offre).
    pub kind: Option<String>,
    /// Identifiant du match.
    pub event: String,
    /// Pari bonus, `ISSUE:COTE:MISE`.
    pub bonus: String,
    /// Couvertures, `OPÉRATEUR:ISSUE:COTE:MISE`.
    pub covers: Vec<String>,
}

/// `edge bonus status` : états, comptes de couverture, propositions du dernier relevé, opérations.
pub fn status(paths: &Paths) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let conn = open_database(paths)?;
    let board = tracking::board(&conn, config.bonus_freebet_value, Utc::now())?;
    print_operators(&board);
    print_plans(&board, 1);
    print_operations(&board);
    Ok(())
}

/// `edge bonus pick` : les meilleures opérations du dernier relevé pour un opérateur.
pub fn pick(paths: &Paths, bookmaker: &str) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let conn = open_database(paths)?;
    let mut board = tracking::board(&conn, config.bonus_freebet_value, Utc::now())?;
    board
        .plans
        .retain(|plan| plan.target.bookmaker == bookmaker);
    if board.plans.is_empty() {
        bail!(
            "rien à jouer chez {} : `edge bonus status` montre l'état de chaque offre",
            operator_name(bookmaker)
        );
    }
    print_plans(&board, usize::MAX);
    Ok(())
}

/// `edge bonus offer` : enregistre ce que l'on sait de l'offre d'un opérateur (la plus récente fait foi).
pub fn offer(paths: &Paths, options: &OfferOptions) -> Result<()> {
    if !is_operator_key(&options.bookmaker) {
        bail!(
            "--bookmaker : « {} » n'est pas une clé d'opérateur (minuscules, chiffres et _ ; ex. betclic_fr)",
            options.bookmaker
        );
    }
    let eligibility = if options.ineligible {
        if options.kind.is_some() || options.amount.is_some() || options.min_odds.is_some() {
            bail!("--ineligible ne se combine pas avec --kind, --amount ni --min-odds");
        }
        Eligibility::Ineligible
    } else {
        let kind = parse_kind(
            options
                .kind
                .as_deref()
                .context("--kind est requis (ou --ineligible)")?,
        )?;
        let amount = options
            .amount
            .context("--amount est requis (ou --ineligible)")?;
        Eligibility::Eligible {
            kind,
            amount: Euros::new(amount)
                .filter(|a| a.get() > 0.0)
                .context("--amount doit être un montant positif")?,
            min_odds: match options.min_odds {
                Some(value) => {
                    Some(Odds::new(value).context("--min-odds doit être supérieure à 1")?)
                }
                None => None,
            },
        }
    };
    let new = NewOffer {
        bookmaker: options.bookmaker.clone(),
        eligibility,
        checked_on: options
            .checked_on
            .unwrap_or_else(|| Local::now().date_naive()),
        source: options.source.clone(),
        note: options.note.clone(),
    };
    let conn = open_database(paths)?;
    let id = bonus::insert_offer(&conn, &new, Utc::now())?;
    println!(
        "Offre n° {id} enregistrée pour {} : {}.",
        operator_name(&new.bookmaker),
        offer_label(&new.eligibility)
    );
    Ok(())
}

/// `edge bonus cover` : mises de couverture sur les cotes affichées au moment de parier. Ne lit ni n'écrit la base.
pub fn cover_calc(paths: &Paths, options: &CoverOptions) -> Result<()> {
    let kind = parse_kind(&options.kind)?;
    let freebet_value = match options.freebet_value {
        Some(value) => value,
        None => Config::load(&paths.config_file)?.bonus_freebet_value,
    };
    let amount = Euros::new(options.amount).context("--amount doit être un montant positif")?;
    let odds = Odds::new(options.odds).context("--odds doit être supérieure à 1")?;
    let others: Vec<Odds> = options
        .covers
        .iter()
        .map(|o| {
            Odds::new(*o)
                .with_context(|| format!("--cover {o} : une cote doit être supérieure à 1"))
        })
        .collect::<Result<_>>()?;
    let result = cover(kind, amount, odds, freebet_value, &others)?;

    println!(
        "Bonus {} de {} € à la cote {}{}",
        kind.label(),
        format::decimal(amount.get(), 2),
        format::decimal(odds.get(), 2),
        if kind == OfferKind::FreebetRefund {
            format!(
                " (freebet compté {} € par euro)",
                format::decimal(freebet_value, 2)
            )
        } else {
            String::new()
        }
    );
    for (index, (stake, o)) in result.stakes.iter().zip(&others).enumerate() {
        println!(
            "  Couverture {} à la cote {} : {} €",
            index + 1,
            format::decimal(o.get(), 2),
            format::decimal(stake.get(), 2)
        );
    }
    println!(
        "Gain net si le bonus passe : {} € ; si la couverture 1, 2… passe : {} €",
        format::signed(result.if_bonus_wins, 2),
        result
            .if_other_wins
            .iter()
            .map(|g| format::signed(*g, 2))
            .collect::<Vec<_>>()
            .join(" ; ")
    );
    println!(
        "Gain garanti : {} € · argent sorti : {} € (il revient après le match)",
        format::signed(result.guaranteed, 2),
        format::decimal(result.cash_out, 2)
    );
    if result.guaranteed <= 0.0 {
        println!(
            "Attention : ce bonus ne rapporte rien à ces cotes. Cherche un autre match (`edge bonus pick`)."
        );
    }
    Ok(())
}

/// `edge bonus log` : enregistre une opération réellement jouée.
pub fn log(paths: &Paths, options: &LogOptions) -> Result<()> {
    let config = Config::load(&paths.config_file)?;
    let mut legs = vec![parse_bonus_leg(&options.bookmaker, &options.bonus)?];
    for text in &options.covers {
        legs.push(parse_cover_leg(text)?);
    }
    let request = OperationRequest {
        bookmaker: options.bookmaker.clone(),
        kind: options.kind.as_deref().map(parse_kind).transpose()?,
        event_id: options.event.clone(),
        legs,
        freebet_value: config.bonus_freebet_value,
    };
    let mut conn = open_database(paths)?;
    let new = prepare_operation(&conn, &request)?;
    let id = record_operation(&mut conn, &new, Utc::now())?;
    println!(
        "Opération n° {id} enregistrée : {} – {}, {} chez {}, gain garanti {} €.",
        new.event.home_team,
        new.event.away_team,
        new.kind.label(),
        operator_name(&options.bookmaker),
        format::signed(new.guaranteed, 2)
    );
    Ok(())
}

/// `edge bonus settle` : règle les opérations ouvertes depuis l'historique football-data, ou une opération à la main.
pub fn settle(paths: &Paths, manual: Option<(i64, &str)>) -> Result<()> {
    let conn = open_database(paths)?;
    let now = Utc::now();
    if let Some((id, text)) = manual {
        let result = match text {
            "void" => MatchResult::Void,
            code => MatchResult::Played(
                Outcome::from_code(code).context("--result doit valoir H, D, A ou void")?,
            ),
        };
        tracking::settle_operation(&conn, id, result, now)?;
        println!("Opération n° {id} réglée.");
        return Ok(());
    }
    let report = tracking::settle_open_operations(&conn, &Aliases::bundled()?, now)?;
    println!(
        "{} opération(s) réglée(s), {} encore ouverte(s) (résultat pas encore importé : `edge fd fetch` puis `edge fd import`).",
        report.settled, report.still_open
    );
    Ok(())
}

fn print_operators(board: &Board) {
    println!("Offres :");
    for status in &board.operators {
        // L'état dit déjà « inéligible » : on ne détaille que les offres éligibles, et la vérification dans tous les cas.
        let offer = status.offer.as_ref().map_or(String::new(), |o| {
            let detail = match &o.offer.eligibility {
                Eligibility::Eligible { .. } => format!(" · {}", offer_label(&o.offer.eligibility)),
                Eligibility::Ineligible => " · compte de couverture".to_string(),
            };
            format!(
                "{detail} · vérifié le {} ({})",
                o.offer.checked_on.format("%d/%m/%Y"),
                o.offer.source
            )
        });
        let mut extra = String::new();
        if let Some(freebet) = status.pending_freebet {
            extra += &format!(
                " · freebet de {} € à jouer",
                format::decimal(freebet.get(), 2)
            );
        }
        if status.open_operations > 0 {
            extra += &format!(" · {} opération(s) ouverte(s)", status.open_operations);
        }
        if status.realized != 0.0 {
            extra += &format!(" · réalisé {} €", format::signed(status.realized, 2));
        }
        println!(
            "  {:<18} {:<11}{offer}{extra}",
            status.name,
            status.state.label()
        );
    }
    let covers: Vec<&str> = board
        .cover_accounts
        .iter()
        .map(|k| operator_name(k))
        .collect();
    println!(
        "Comptes de couverture : {}",
        if covers.is_empty() {
            "aucun (enregistre au moins un opérateur inéligible, Winamax par exemple)".to_string()
        } else {
            covers.join(", ")
        }
    );
    let to_check = board
        .operators
        .iter()
        .filter(|s| s.state == OfferState::ToCheck)
        .count();
    if to_check > 0 {
        println!(
            "{to_check} opérateur(s) à vérifier : lire le règlement, puis `edge bonus offer --bookmaker … --kind … --amount … --source …`."
        );
    }
}

fn print_plans(board: &Board, per_plan: usize) {
    for plan in &board.plans {
        println!(
            "\n{} — {} de {} €{}",
            plan.name,
            plan.target.kind.label(),
            format::decimal(plan.target.amount.get(), 2),
            plan.target.min_odds.map_or(String::new(), |m| format!(
                ", cote minimale {}",
                format::decimal(m.get(), 2)
            ))
        );
        if !plan.in_archive {
            println!(
                "  Cotes absentes de l'archive : compare les cotes affichées et calcule avec `edge bonus cover`."
            );
            continue;
        }
        if plan.candidates.is_empty() {
            println!(
                "  Aucune opération rentable dans le dernier relevé (comptes de couverture : {}).",
                board.cover_accounts.len()
            );
            continue;
        }
        for (rank, candidate) in plan.candidates.iter().take(per_plan).enumerate() {
            print_candidate(
                rank + 1,
                candidate,
                plan.target.kind,
                &plan.target.bookmaker,
            );
        }
    }
}

fn print_candidate(rank: usize, candidate: &Candidate, kind: OfferKind, bookmaker: &str) {
    let now = Utc::now();
    let event = &candidate.event;
    println!(
        "  [{rank}] {} – {}, {} ({}) · gain garanti {} € · argent sorti {} € · cotes {}",
        event.home_team,
        event.away_team,
        format::kickoff_label(event.kickoff),
        league_name(&event.sport_key),
        format::signed(candidate.cover.guaranteed, 2),
        format::decimal(candidate.cover.cash_out, 2),
        format::age(candidate.priced_at, now)
    );
    for leg in &candidate.legs {
        println!(
            "     {:<6} {:<18} {:<9} à {} : {} €",
            if leg.role == Role::Bonus {
                "Bonus"
            } else {
                "Couv."
            },
            operator_name(&leg.bookmaker),
            format::outcome_label(leg.outcome),
            format::decimal(leg.odds.get(), 2),
            format::decimal(leg.stake.get(), 2)
        );
    }
    let bonus = candidate.bonus_leg();
    let covers = &candidate.legs[1..];
    println!(
        "     Recalcul : edge bonus cover --kind {} --amount {} --odds {}{}",
        kind.as_str(),
        bonus.stake.get(),
        bonus.odds.get(),
        covers
            .iter()
            .map(|l| format!(" --cover {}", l.odds.get()))
            .collect::<String>()
    );
    println!(
        "     Enregistrement : edge bonus log --bookmaker {bookmaker} --event {} --bonus {}:{}:{}{}",
        event.event_id,
        bonus.outcome.code(),
        bonus.odds.get(),
        bonus.stake.get(),
        covers
            .iter()
            .map(|l| format!(
                " --cover {}:{}:{}:{}",
                l.bookmaker,
                l.outcome.code(),
                l.odds.get(),
                l.stake.get()
            ))
            .collect::<String>()
    );
}

fn print_operations(board: &Board) {
    if board.operations.is_empty() {
        return;
    }
    println!("\nOpérations :");
    for stored in &board.operations {
        let op = &stored.operation;
        let state = match (&stored.settled, tracking::realized_gain(stored)) {
            (Some(settled), Some(gain)) => format!(
                "réglée ({}) : {} €",
                match settled.result {
                    MatchResult::Played(o) => format::outcome_label(o),
                    MatchResult::Void => "annulé",
                },
                format::signed(gain, 2)
            ),
            _ => "ouverte".to_string(),
        };
        println!(
            "  n° {:<3} {} – {}, {} · {} chez {} · garanti {} € · {state}",
            stored.id,
            op.event.home_team,
            op.event.away_team,
            format::kickoff_label(op.event.kickoff),
            op.kind.label(),
            operator_name(&stored.bonus_leg().bookmaker),
            format::signed(op.guaranteed, 2)
        );
    }
    println!(
        "Bilan : réalisé {} € · garanti sur les opérations ouvertes {} €",
        format::signed(board.realized, 2),
        format::signed(board.open_guaranteed, 2)
    );
}

fn offer_label(eligibility: &Eligibility) -> String {
    match eligibility {
        Eligibility::Eligible {
            kind,
            amount,
            min_odds,
        } => format!(
            "{} jusqu'à {} €{}",
            kind.label(),
            format::decimal(amount.get(), 2),
            min_odds.map_or(String::new(), |m| format!(
                ", cote minimale {}",
                format::decimal(m.get(), 2)
            ))
        ),
        Eligibility::Ineligible => "inéligible (compte de couverture)".to_string(),
    }
}

fn parse_kind(text: &str) -> Result<OfferKind> {
    OfferKind::ALL
        .into_iter()
        .find(|kind| kind.as_str() == text)
        .with_context(|| {
            format!("forme « {text} » inconnue : cash_refund, freebet_refund ou freebet")
        })
}

/// `ISSUE:COTE:MISE` (ex. `A:3.10:100`) : le pari bonus, chez l'opérateur de l'offre.
fn parse_bonus_leg(bookmaker: &str, text: &str) -> Result<Leg> {
    let parts: Vec<&str> = text.split(':').collect();
    let [outcome, odds, stake] = parts.as_slice() else {
        bail!("--bonus « {text} » : format attendu ISSUE:COTE:MISE, par exemple A:3.10:100");
    };
    leg(Role::Bonus, bookmaker, outcome, odds, stake).with_context(|| format!("--bonus « {text} »"))
}

/// `OPÉRATEUR:ISSUE:COTE:MISE` (ex. `winamax_fr:H:2.10:122`) : une couverture.
fn parse_cover_leg(text: &str) -> Result<Leg> {
    let parts: Vec<&str> = text.split(':').collect();
    let [bookmaker, outcome, odds, stake] = parts.as_slice() else {
        bail!(
            "--cover « {text} » : format attendu OPÉRATEUR:ISSUE:COTE:MISE, par exemple winamax_fr:H:2.10:122"
        );
    };
    leg(Role::Cover, bookmaker, outcome, odds, stake).with_context(|| format!("--cover « {text} »"))
}

fn leg(role: Role, bookmaker: &str, outcome: &str, odds: &str, stake: &str) -> Result<Leg> {
    // La virgule décimale est acceptée : « 3,10 » se tape plus naturellement en français.
    let number = |text: &str| text.replace(',', ".").parse::<f64>();
    Ok(Leg {
        role,
        bookmaker: bookmaker.to_string(),
        outcome: Outcome::from_code(outcome).context("issue H, D ou A")?,
        odds: number(odds)
            .ok()
            .and_then(Odds::new)
            .context("cote supérieure à 1")?,
        stake: number(stake)
            .ok()
            .and_then(Euros::new)
            .filter(|s| s.get() > 0.0)
            .context("mise positive")?,
    })
}

fn open_database(paths: &Paths) -> Result<Connection> {
    store::open_migrated(&paths.db_file).context("base inaccessible : lance d'abord `edge init`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legs_are_read_from_their_compact_form() {
        let bonus = parse_bonus_leg("betclic_fr", "A:3,10:100").unwrap();
        assert_eq!(
            (bonus.outcome, bonus.odds.get(), bonus.stake.get()),
            (Outcome::Away, 3.1, 100.0)
        );
        let cover = parse_cover_leg("winamax_fr:H:2.10:122.5").unwrap();
        assert_eq!(cover.bookmaker, "winamax_fr");
        assert_eq!(cover.role, Role::Cover);
        assert!(parse_cover_leg("H:2.10:122").is_err());
        assert!(parse_bonus_leg("betclic_fr", "X:2:10").is_err());
        assert!(parse_bonus_leg("betclic_fr", "H:1:10").is_err());
        assert!(parse_bonus_leg("betclic_fr", "H:2:0").is_err());
    }
}
