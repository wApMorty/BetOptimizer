//! Onglet « Bonus » (`/bonus`) : offres de bienvenue à jouer, opérations couvertes proposées sur le dernier relevé,
//! calculateur, suivi des offres et des opérations.
//!
//! Tout vient de [`crate::bonus::tracking::board`], le même tableau que `edge bonus status`. Le calculateur est un
//! formulaire `GET` : ses champs reviennent dans l'adresse, la page les relit et affiche le résultat, rien n'est écrit.
//! Le bouton « Relever les cotes » lance un relevé du football (comme `edge snapshot`, sans le tennis) : il coûte des crédits.

use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;

use crate::bonus::cover::{Cover, cover};
use crate::bonus::pick::Candidate;
use crate::bonus::tracking::{self, OfferState, OperatorStatus, Plan};
use crate::bonus::{OfferKind, operator_name};
use crate::config::Config;
use crate::domain::{Euros, Odds};
use crate::error::Result;
use crate::format;
use crate::ingest::client::OddsApiClient;
use crate::ingest::snapshot::{self, Fetch, SnapshotReport};
use crate::report::recommend_report::league_name;
use crate::store::archive;
use crate::store::bonus::{Eligibility, Leg, MatchResult, Role, StoredOperation};
use crate::store::{self};
use crate::web::App;
use crate::web::form::{Form, encode, parse_number, required};
use crate::web::pages::entry::Reply;
use crate::web::view::{Flash, Layout, euros, fixture, render, sign_color, signed_euros, when};

const TEMPLATE: &str = include_str!("../templates/bonus.html");

/// Opérations proposées par offre (la CLI `edge bonus pick` en montre davantage).
const SHOWN_PER_PLAN: usize = 3;

/// Écart minimal entre deux relevés du football : un double clic ou un formulaire renvoyé ne dépense pas deux fois les
/// crédits.
const MIN_REFRESH_GAP: TimeDelta = TimeDelta::minutes(15);

/// Préfixe des compétitions relevées par le bouton.
const FOOTBALL: &str = "soccer_";

/// Libellés courts des formes d'offre, dans l'ordre de [`OfferKind::ALL`].
pub const KIND_LABELS: [&str; 3] = ["Remb. argent", "Remb. freebet", "Freebet"];

/// Un pari d'une opération, en pastille (`Betclic · domicile 2,63 · 100,00 €`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LegChip {
    /// Texte.
    pub text: String,
    /// Le pari bonus (mis en avant) ?
    pub bonus: bool,
}

/// Les pastilles des paris d'une opération.
pub fn leg_chips(legs: &[Leg]) -> Vec<LegChip> {
    legs.iter()
        .map(|leg| LegChip {
            text: format!(
                "{} · {} {} · {}",
                operator_name(&leg.bookmaker),
                format::outcome_label(leg.outcome),
                format::decimal(leg.odds.get(), 2),
                euros(leg.stake.get(), 2)
            ),
            bonus: leg.role == Role::Bonus,
        })
        .collect()
}

/// Une opération proposée.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct CandidateView {
    fixture: String,
    meta: String,
    legs: Vec<LegChip>,
    guaranteed: String,
    color: &'static str,
    cash_out: String,
    /// Adresse du formulaire « J'ai joué », prérempli avec cette opération.
    play: String,
}

/// Une offre à jouer.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct PlanView {
    name: String,
    kind: &'static str,
    amount: String,
    min_odds: Option<String>,
    in_archive: bool,
    candidates: Vec<CandidateView>,
}

/// Un opérateur et l'état de son offre.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct OperatorView {
    key: String,
    name: String,
    state: &'static str,
    tone: &'static str,
    offer: Option<String>,
    checked: Option<String>,
    source: Option<String>,
    freebet: Option<String>,
    realized: Option<String>,
    realized_tone: &'static str,
    has_offer: bool,
}

/// Une opération enregistrée.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct OperationView {
    id: i64,
    fixture: String,
    meta: String,
    legs: Vec<LegChip>,
    guaranteed: String,
    /// Réglée : gain réalisé, sa couleur, et le résultat.
    gain: Option<String>,
    color: &'static str,
    result: String,
}

/// Le calculateur : champs saisis, et résultat ou message d'erreur.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct Calculator {
    kinds: Vec<(&'static str, &'static str)>,
    kind: &'static str,
    amount: String,
    odds: String,
    cover1: String,
    cover2: String,
    result: Option<String>,
    detail: Option<String>,
    color: &'static str,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BonusPage {
    #[serde(flatten)]
    layout: Layout,
    realized: String,
    realized_color: &'static str,
    open_guaranteed: String,
    to_play: usize,
    covers: String,
    /// Âge du dernier relevé du football (« il y a 2 h 10 »), ou « aucun relevé ».
    last_snapshot: String,
    /// Coût en crédits d'un relevé du football.
    refresh_cost: u32,
    plans: Vec<PlanView>,
    calc: Calculator,
    operators: Vec<OperatorView>,
    operations: Vec<OperationView>,
}

/// La page ; les champs du calculateur arrivent dans l'adresse.
pub fn page(app: &App, query: &Form, now: DateTime<Utc>, flash: Option<Flash>) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let freebet_value = app.config.bonus_freebet_value;
    let board = tracking::board(&conn, freebet_value, now)?;
    let last_snapshot = match archive::last_odds_call(&conn, FOOTBALL)? {
        Some(at) => format!("relevé {}", format::age(at, now)),
        None => "aucun relevé".to_string(),
    };
    let covers: Vec<&str> = board
        .cover_accounts
        .iter()
        .map(|k| operator_name(k))
        .collect();
    render(
        "bonus.html",
        TEMPLATE,
        &BonusPage {
            layout: Layout::new("bonus", now, flash),
            realized: signed_euros(board.realized),
            realized_color: sign_color(board.realized),
            open_guaranteed: signed_euros(board.open_guaranteed),
            to_play: board.plans.len(),
            covers: if covers.is_empty() {
                "aucun compte de couverture".to_string()
            } else {
                format!("couverture : {}", covers.join(", "))
            },
            last_snapshot,
            refresh_cost: snapshot::plan(&app.config).total_cost,
            plans: board
                .plans
                .iter()
                .map(|plan| plan_view(plan, now))
                .collect(),
            calc: calculator(query, freebet_value),
            operators: board.operators.iter().map(operator_view).collect(),
            // Les plus récentes d'abord.
            operations: board
                .operations
                .iter()
                .rev()
                .map(|op| operation_view(op, now))
                .collect(),
        },
    )
}

/// Bouton « Relever les cotes » : relève le football puis revient sur `/bonus`, avec le bilan en message.
pub fn refresh(app: &App, now: DateTime<Utc>) -> Reply {
    let flash = match &app.odds_api_key {
        None => Flash::error(
            "clé The Odds API absente : définis EDGE_ODDS_API_KEY ou odds_api_key dans la configuration, puis relance edge serve",
        ),
        Some(key) => {
            // `clone()` : la clé appartient à l'application, partagée par toutes les requêtes ; le client en veut sa
            // propre copie (une petite chaîne).
            let client = OddsApiClient::new(key.clone());
            snapshot_football(app, now, &mut |path, params| client.get(path, params))
                .unwrap_or_else(|error| Flash::error(format!("relevé non fait : {error}")))
        }
    };
    Reply::Done {
        to: "/bonus",
        flash,
    }
}

/// Relève les cotes du football, sauf si le dernier relevé a moins de [`MIN_REFRESH_GAP`]. La garde de budget mensuelle
/// de [`snapshot::run`] s'applique ligue par ligue, comme pour `edge snapshot`.
fn snapshot_football(app: &App, now: DateTime<Utc>, fetch: Fetch) -> Result<Flash> {
    let mut conn = store::open_migrated(&app.paths.db_file)?;
    // `if let … && …` (édition 2024) : le motif et la condition se lisent ensemble, sans `if` imbriqué.
    if let Some(last) = archive::last_odds_call(&conn, FOOTBALL)?
        && now < last + MIN_REFRESH_GAP
    {
        return Ok(Flash::error(format!(
            "relevé refusé : le précédent date de moins de {} min ({})",
            MIN_REFRESH_GAP.num_minutes(),
            format::age(last, now)
        )));
    }
    // Le football seul : les offres de bienvenue se jouent sur ces matchs, et le tennis doublerait le coût.
    // `clone()` : la configuration est partagée par toutes les requêtes ; on modifie une copie.
    let football = Config {
        tennis_archive: false,
        ..app.config.clone()
    };
    let report = snapshot::run(&mut conn, &football, &mut Utc::now, fetch)?;
    let text = refresh_summary(&report, football.monthly_credit_cap);
    Ok(if report.leagues.is_empty() {
        Flash::error(text)
    } else {
        Flash::ok(text)
    })
}

/// Bilan d'un relevé en une phrase : ligues archivées, matchs, crédits, échecs.
fn refresh_summary(report: &SnapshotReport, cap: u32) -> String {
    let events: usize = report.leagues.iter().map(|l| l.stored.events).sum();
    let head = if report.leagues.is_empty() {
        "Aucune ligue archivée".to_string()
    } else {
        format!(
            "Relevé fait : {} ligue(s), {events} match(s) archivé(s)",
            report.leagues.len()
        )
    };
    let mut text = format!(
        "{head} ; crédits utilisés {} → {} (plafond {cap}).",
        report.used_before, report.used_after
    );
    for failure in &report.failures {
        text.push_str(&format!(
            " Échec {} : {}.",
            failure.sport_key, failure.reason
        ));
    }
    text
}

fn plan_view(plan: &Plan, now: DateTime<Utc>) -> PlanView {
    PlanView {
        name: plan.name.clone(),
        kind: plan.target.kind.label(),
        amount: euros(plan.target.amount.get(), 2),
        min_odds: plan
            .target
            .min_odds
            .map(|min| format!("cote ≥ {}", format::decimal(min.get(), 2))),
        in_archive: plan.in_archive,
        candidates: plan
            .candidates
            .iter()
            .take(SHOWN_PER_PLAN)
            .map(|c| candidate_view(plan, c, now))
            .collect(),
    }
}

fn candidate_view(plan: &Plan, candidate: &Candidate, now: DateTime<Utc>) -> CandidateView {
    let event = &candidate.event;
    CandidateView {
        fixture: fixture(&event.home_team, &event.away_team),
        meta: format!(
            "{} · {} · cotes {}",
            league_name(&event.sport_key),
            when(event.kickoff, now),
            format::age(candidate.priced_at, now)
        ),
        legs: leg_chips(&candidate.legs),
        guaranteed: signed_euros(candidate.cover.guaranteed),
        color: sign_color(candidate.cover.guaranteed),
        cash_out: euros(candidate.cover.cash_out, 0),
        play: format!(
            "/bonus/operation?operateur={}&forme={}&match={}",
            encode(&plan.target.bookmaker),
            plan.target.kind.as_str(),
            encode(&event.event_id)
        ),
    }
}

fn operator_view(status: &OperatorStatus) -> OperatorView {
    let tone = match status.state {
        OfferState::Available => "t-pos",
        OfferState::Consumed => "t-acc",
        OfferState::Ineligible => "t-warn",
        OfferState::ToCheck => "",
    };
    let stored = status.offer.as_ref();
    OperatorView {
        key: status.key.clone(),
        name: status.name.clone(),
        state: status.state.label(),
        tone,
        offer: stored.and_then(|s| match &s.offer.eligibility {
            Eligibility::Eligible { kind, amount, .. } => {
                Some(format!("{} · {}", kind.label(), euros(amount.get(), 0)))
            }
            Eligibility::Ineligible => None,
        }),
        checked: stored.map(|s| format!("vérifié le {}", s.offer.checked_on.format("%d/%m/%Y"))),
        source: stored.map(|s| s.offer.source.clone()),
        freebet: status
            .pending_freebet
            .map(|f| format!("freebet {} à jouer", euros(f.get(), 2))),
        realized: (status.realized != 0.0)
            .then(|| format!("réalisé {}", signed_euros(status.realized))),
        realized_tone: if status.realized > 0.0 {
            "t-pos"
        } else {
            "t-neg"
        },
        has_offer: stored.is_some(),
    }
}

fn operation_view(stored: &StoredOperation, now: DateTime<Utc>) -> OperationView {
    let op = &stored.operation;
    let gain = tracking::realized_gain(stored);
    let result = match &stored.settled {
        Some(settled) => {
            let result = match settled.result {
                MatchResult::Played(outcome) => format::outcome_label(outcome),
                MatchResult::Void => "annulé",
            };
            format!(
                "réglée : {result} · garanti {}",
                signed_euros(op.guaranteed)
            )
        }
        None => String::new(),
    };
    OperationView {
        id: stored.id,
        fixture: fixture(&op.event.home_team, &op.event.away_team),
        meta: format!(
            "n° {} · {} chez {} · {}",
            stored.id,
            op.kind.label(),
            operator_name(&stored.bonus_leg().bookmaker),
            when(op.event.kickoff, now)
        ),
        legs: leg_chips(&op.legs),
        guaranteed: format!("garanti {}", signed_euros(op.guaranteed)),
        color: gain.map_or("", sign_color),
        gain: gain.filter(|_| stored.settled.is_some()).map(signed_euros),
        result,
    }
}

/// La forme d'offre nommée dans l'adresse (`freebet_refund`…), le remboursement en freebet par défaut.
pub fn kind_from(text: &str) -> OfferKind {
    OfferKind::parse(text).unwrap_or(OfferKind::FreebetRefund)
}

/// Calcule les mises de couverture d'un bonus : montant, cote du pari bonus, une ou deux cotes de couverture.
fn compute(
    kind: OfferKind,
    query: &Form,
    freebet_value: f64,
) -> std::result::Result<Cover, String> {
    let amount = Euros::new(required(query.get("montant"), "montant")?)
        .ok_or("montant : il doit être positif")?;
    let odds = Odds::new(required(query.get("cote"), "cote du bonus")?)
        .ok_or("cote du bonus : elle doit être supérieure à 1")?;
    let mut others = Vec::new();
    for (index, field) in ["c1", "c2"].iter().enumerate() {
        if let Some(value) = parse_number(query.get(field), &format!("couverture {}", index + 1))? {
            others.push(Odds::new(value).ok_or("une cote de couverture doit être supérieure à 1")?);
        }
    }
    cover(kind, amount, odds, freebet_value, &others).map_err(|e| e.to_string())
}

fn calculator(query: &Form, freebet_value: f64) -> Calculator {
    let kind = kind_from(query.get("forme"));
    let mut calc = Calculator {
        kinds: OfferKind::ALL
            .iter()
            .zip(KIND_LABELS)
            .map(|(k, label)| (k.as_str(), label))
            .collect(),
        kind: kind.as_str(),
        amount: if query.has("montant") {
            query.get("montant").to_string()
        } else {
            "100".to_string()
        },
        odds: query.get("cote").to_string(),
        cover1: query.get("c1").to_string(),
        cover2: query.get("c2").to_string(),
        result: None,
        detail: None,
        color: "",
        error: None,
    };
    if calc.odds.trim().is_empty() {
        return calc;
    }
    match compute(kind, query, freebet_value) {
        Ok(result) => {
            let stakes: Vec<String> = result.stakes.iter().map(|s| euros(s.get(), 2)).collect();
            calc.result = Some(format!("Mises de couverture : {}", stakes.join(" · ")));
            let mut detail = format!(
                "Gain garanti {} · argent sorti {}",
                signed_euros(result.guaranteed),
                euros(result.cash_out, 2)
            );
            if kind == OfferKind::FreebetRefund {
                detail.push_str(&format!(
                    " · freebet compté {} par euro",
                    format::decimal(freebet_value, 2)
                ));
            }
            calc.detail = Some(detail);
            calc.color = sign_color(result.guaranteed);
        }
        Err(message) => calc.error = Some(message),
    }
    calc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Quota;
    use crate::ingest::client::HttpResponse;
    use crate::ingest::odds_api::SPORTS_PATH;
    use crate::web::testing::empty;

    const SPORTS: &str = include_str!("../../../tests/fixtures/sports_with_tennis.json");
    const EPL: &str = include_str!("../../../tests/fixtures/odds_epl.json");

    fn response(used: u32, body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            quota: Quota {
                cost: Some(1),
                used: Some(used),
                remaining: Some(500 - used),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn the_button_snapshots_football_only_then_refuses_a_second_click() {
        let mut sample = empty();
        sample.app.config.leagues = vec!["soccer_epl".to_string()];
        sample.app.config.tennis_archive = true;
        let mut paths = Vec::new();
        let mut fetch = |path: &str, _params: &str| {
            paths.push(path.to_string());
            let body = if path == SPORTS_PATH { SPORTS } else { EPL };
            Ok(response(10 + paths.len() as u32, body))
        };

        let flash = snapshot_football(&sample.app, Utc::now(), &mut fetch).unwrap();
        assert_eq!(flash.kind, "t-pos", "{}", flash.text);
        assert!(flash.text.contains("1 ligue(s)"), "{}", flash.text);
        // Le tennis est actif dans la réponse de `/sports`, mais le bouton ne relève que le football.
        assert_eq!(paths, vec![SPORTS_PATH, "/sports/soccer_epl/odds"]);

        let mut no_call = |path: &str, _params: &str| -> Result<HttpResponse> {
            panic!("aucun appel attendu : {path}")
        };
        let flash = snapshot_football(&sample.app, Utc::now(), &mut no_call).unwrap();
        assert_eq!(flash.kind, "t-neg");
        assert!(flash.text.contains("moins de 15 min"), "{}", flash.text);
        assert!(sample.html("/bonus").contains("relevé il y a 0 min"));
    }

    #[test]
    fn without_a_key_the_button_explains_instead_of_calling() {
        let sample = empty();
        let html = sample.html("/bonus");
        assert!(html.contains("action=\"/bonus/relever\""), "{html}");
        assert!(html.contains("aucun relevé"));
        match refresh(&sample.app, sample.now) {
            Reply::Done { to, flash } => {
                assert_eq!(to, "/bonus");
                assert!(flash.text.contains("EDGE_ODDS_API_KEY"), "{}", flash.text);
            }
            _ => panic!("retour attendu sur /bonus"),
        }
    }

    #[test]
    fn the_calculator_reads_its_fields_from_the_address() {
        let sample = empty();
        let page = |target: &str| sample.html(target);

        let html = page("/bonus?forme=freebet&montant=50&cote=2&c1=2&c2=");
        assert!(html.contains("Gain garanti +25,00 €"), "{html}");
        assert!(html.contains("value=\"freebet\" checked"));

        assert!(page("/bonus?forme=freebet&montant=50&cote=x").contains("cote du bonus"));
        assert!(page("/bonus?forme=freebet&montant=50&cote=2").contains("couverture"));
        // Rien de saisi : la consigne, pas d'erreur.
        assert!(page("/bonus").contains("Tape la cote du pari bonus"));
    }

    #[test]
    fn without_any_offer_the_page_says_what_to_do() {
        let html = empty().html("/bonus");
        assert!(html.contains("Aucune offre à jouer"), "{html}");
        assert!(html.contains("href=\"/bonus/offre?operateur=betclic_fr\""));
    }
}
