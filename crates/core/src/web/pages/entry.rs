//! Pages de saisie : « J'ai parié » (`/paris/suivre/<n°>`), pari hors modèle (`/paris/hors-modele`), opération bonus
//! jouée (`/bonus/operation`) et offre d'un opérateur (`/bonus/offre`).
//!
//! En `GET`, chaque page affiche son formulaire prérempli. En `POST`, la saisie est validée puis écrite par
//! [`crate::web::actions::apply`] : réussite, retour à l'onglet avec un message ; refus, le formulaire revient avec ce
//! qui a été tapé et la raison du refus.

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Paris;
use serde::Serialize;

use super::bonus::KIND_LABELS;
use crate::betting::ledger;
use crate::bonus::tracking::{self, Board};
use crate::bonus::{OPERATORS, OfferKind, operator_name};
use crate::error::Result;
use crate::format;
use crate::report::recommend_report::league_name;
use crate::store::bonus::Eligibility;
use crate::store::journal::{self, StoredRecommendation};
use crate::store::{self};
use crate::web::App;
use crate::web::actions::{Action, apply};
use crate::web::form::Form;
use crate::web::forms::{BonusForm, FollowForm, OffModelForm, OfferForm};
use crate::web::view::{Flash, Layout, fixture, render, sign_color, signed_euros, when};

const FOLLOW_TEMPLATE: &str = include_str!("../templates/follow.html");
const OFF_MODEL_TEMPLATE: &str = include_str!("../templates/off_model.html");
const BONUS_TEMPLATE: &str = include_str!("../templates/bonus_operation.html");
const OFFER_TEMPLATE: &str = include_str!("../templates/offer.html");

/// Ce que devient une saisie.
pub enum Reply {
    /// Écrite : retour à l'onglet `to`, avec le message.
    Done {
        /// Adresse de l'onglet.
        to: &'static str,
        /// Message à afficher.
        flash: Flash,
    },
    /// Une page à afficher (formulaire, éventuellement avec l'erreur), avec son statut HTTP.
    Page {
        /// Statut.
        status: u16,
        /// HTML.
        html: String,
    },
    /// Rien à saisir à cette adresse (recommandation ou opération introuvable).
    NotFound,
}

/// Exécute une action validée : réussite, retour à `to` ; refus du cœur, le message d'erreur.
fn write(app: &App, action: &Action, now: DateTime<Utc>) -> std::result::Result<String, String> {
    apply(&app.paths.db_file, &app.aliases, action, now).map_err(|e| e.to_string())
}

/// Un bouton de règlement, une case à cocher… : une action sans formulaire à réafficher. Le résultat, bon ou mauvais,
/// revient en message sur l'onglet `to`.
pub fn act(app: &App, action: &Action, to: &'static str, now: DateTime<Utc>) -> Reply {
    let flash = match write(app, action, now) {
        Ok(message) => Flash::ok(message),
        Err(message) => Flash::error(message),
    };
    Reply::Done { to, flash }
}

// ---------------------------------------------------------------------------------------------------------------------
// « J'ai parié »

#[derive(Debug, Clone, PartialEq, Serialize)]
struct FollowPage {
    #[serde(flatten)]
    layout: Layout,
    id: i64,
    fixture: String,
    meta: String,
    line: String,
    minimal_odds: String,
    form: FollowForm,
    bookmakers: Vec<String>,
    error: Option<String>,
}

fn follow_page(
    app: &App,
    found: &StoredRecommendation,
    form: FollowForm,
    error: Option<String>,
    now: DateTime<Utc>,
) -> Result<String> {
    let r = &found.record;
    render(
        "follow.html",
        FOLLOW_TEMPLATE,
        &FollowPage {
            layout: Layout::new("opportunities", now, None),
            id: found.id,
            fixture: fixture(&r.home_team, &r.away_team),
            meta: format!("{} · {}", league_name(&r.sport_key), when(r.kickoff, now)),
            line: format!(
                "{} · {} à {} · avantage {} %",
                r.bookmaker,
                format::outcome_label(r.outcome),
                format::decimal(r.odds.get(), 2),
                format::signed(r.edge * 100.0, 1)
            ),
            minimal_odds: format::decimal(r.minimal_odds, 2),
            form,
            bookmakers: app.config.bookmakers.clone(),
            error,
        },
    )
}

/// `/paris/suivre/<n°>`, en `GET` (formulaire prérempli) ou en `POST` (`form` : la saisie).
pub fn follow(app: &App, id: i64, form: Option<&Form>, now: DateTime<Utc>) -> Result<Reply> {
    let found = {
        let conn = store::open_read_only(&app.paths.db_file)?;
        journal::recommendation(&conn, id)?
    };
    let Some(found) = found else {
        return Ok(Reply::NotFound);
    };
    let Some(form) = form else {
        let html = follow_page(
            app,
            &found,
            FollowForm::for_recommendation(&found),
            None,
            now,
        )?;
        return Ok(Reply::Page { status: 200, html });
    };
    let typed = FollowForm::from_form(form);
    let result = typed
        .to_action(id)
        .and_then(|action| write(app, &action, now));
    match result {
        Ok(message) => Ok(Reply::Done {
            to: "/paris",
            flash: Flash::ok(message),
        }),
        Err(error) => Ok(Reply::Page {
            status: 422,
            html: follow_page(app, &found, typed, Some(error), now)?,
        }),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Pari hors modèle

/// Un match de la liste déroulante.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct EventChoice {
    id: String,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct OffModelPage {
    #[serde(flatten)]
    layout: Layout,
    events: Vec<EventChoice>,
    bookmakers: Vec<String>,
    form: OffModelForm,
    error: Option<String>,
}

fn off_model_page(
    app: &App,
    form: OffModelForm,
    error: Option<String>,
    now: DateTime<Utc>,
) -> Result<String> {
    let conn = store::open_read_only(&app.paths.db_file)?;
    let events = ledger::upcoming_events(&conn, now)?
        .into_iter()
        .map(|e| EventChoice {
            label: format!(
                "{} · {} – {}",
                format::kickoff_label(e.kickoff),
                e.home_team,
                e.away_team
            ),
            id: e.event_id,
        })
        .collect();
    render(
        "off_model.html",
        OFF_MODEL_TEMPLATE,
        &OffModelPage {
            layout: Layout::new("bets", now, None),
            events,
            bookmakers: app.config.bookmakers.clone(),
            form,
            error,
        },
    )
}

/// `/paris/hors-modele`, en `GET` ou en `POST`.
pub fn off_model(app: &App, form: Option<&Form>, now: DateTime<Utc>) -> Result<Reply> {
    let Some(form) = form else {
        let blank = OffModelForm {
            outcome: "1".to_string(),
            ..OffModelForm::default()
        };
        let html = off_model_page(app, blank, None, now)?;
        return Ok(Reply::Page { status: 200, html });
    };
    let typed = OffModelForm::from_form(form);
    match typed
        .to_action()
        .and_then(|action| write(app, &action, now))
    {
        Ok(message) => Ok(Reply::Done {
            to: "/paris",
            flash: Flash::ok(message),
        }),
        Err(error) => Ok(Reply::Page {
            status: 422,
            html: off_model_page(app, typed, Some(error), now)?,
        }),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Opération bonus

#[derive(Debug, Clone, PartialEq, Serialize)]
struct BonusOperationPage {
    #[serde(flatten)]
    layout: Layout,
    title: String,
    form: BonusForm,
    /// Comptes de couverture proposés pour les couvertures : (clé, nom).
    covers: Vec<(String, String)>,
    bonus_name: String,
    guaranteed: Option<String>,
    color: &'static str,
    error: Option<String>,
}

fn bonus_page(
    app: &App,
    board: &Board,
    form: BonusForm,
    title: String,
    error: Option<String>,
    now: DateTime<Utc>,
) -> Result<String> {
    let gain = form.guaranteed(app.config.bonus_freebet_value);
    render(
        "bonus_operation.html",
        BONUS_TEMPLATE,
        &BonusOperationPage {
            layout: Layout::new("bonus", now, None),
            covers: board
                .cover_accounts
                .iter()
                .map(|k| (k.clone(), operator_name(k).to_string()))
                .collect(),
            bonus_name: operator_name(&form.bookmaker).to_string(),
            title,
            form,
            guaranteed: gain.map(signed_euros),
            color: gain.map_or("", sign_color),
            error,
        },
    )
}

/// `/bonus/operation`. En `GET`, l'adresse désigne l'opération proposée (`operateur`, `forme`, `match`) ; en `POST`,
/// le bouton « Recalculer » réaffiche le formulaire avec les mises de couverture recalculées, sans rien écrire.
pub fn bonus_operation(
    app: &App,
    query: &Form,
    form: Option<&Form>,
    now: DateTime<Utc>,
) -> Result<Reply> {
    let freebet_value = app.config.bonus_freebet_value;
    let board = {
        let conn = store::open_read_only(&app.paths.db_file)?;
        tracking::board(&conn, freebet_value, now)?
    };
    let Some(form) = form else {
        let wanted_kind = OfferKind::parse(query.get("forme")).ok();
        let found = board
            .plans
            .iter()
            .filter(|plan| {
                plan.target.bookmaker == query.get("operateur")
                    && Some(plan.target.kind) == wanted_kind
            })
            .find_map(|plan| {
                plan.candidates
                    .iter()
                    .find(|c| c.event.event_id == query.get("match"))
                    .map(|c| (plan, c))
            });
        let Some((plan, candidate)) = found else {
            return Ok(Reply::NotFound);
        };
        let title = fixture(&candidate.event.home_team, &candidate.event.away_team);
        let prefilled = BonusForm::from_candidate(&plan.target, candidate);
        let html = bonus_page(app, &board, prefilled, title, None, now)?;
        return Ok(Reply::Page { status: 200, html });
    };
    let mut typed = BonusForm::from_form(form);
    let title = form.get("titre").to_string();
    if form.has("recalculer") {
        let error = typed.recompute(freebet_value).err();
        let html = bonus_page(app, &board, typed, title, error, now)?;
        return Ok(Reply::Page { status: 200, html });
    }
    match typed
        .to_action(freebet_value)
        .and_then(|action| write(app, &action, now))
    {
        Ok(message) => Ok(Reply::Done {
            to: "/bonus",
            flash: Flash::ok(message),
        }),
        Err(error) => Ok(Reply::Page {
            status: 422,
            html: bonus_page(app, &board, typed, title, Some(error), now)?,
        }),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Offre d'un opérateur

#[derive(Debug, Clone, PartialEq, Serialize)]
struct OfferPage {
    #[serde(flatten)]
    layout: Layout,
    operators: Vec<(&'static str, &'static str)>,
    kinds: Vec<(&'static str, &'static str)>,
    form: OfferForm,
    error: Option<String>,
}

fn offer_page(form: OfferForm, error: Option<String>, now: DateTime<Utc>) -> Result<String> {
    render(
        "offer.html",
        OFFER_TEMPLATE,
        &OfferPage {
            layout: Layout::new("bonus", now, None),
            operators: OPERATORS.iter().map(|o| (o.key, o.name)).collect(),
            kinds: OfferKind::ALL
                .iter()
                .zip(KIND_LABELS)
                .map(|(k, label)| (k.as_str(), label))
                .collect(),
            form,
            error,
        },
    )
}

/// `/bonus/offre?operateur=<clé>` (`GET`, prérempli avec l'offre enregistrée s'il y en a une) ou `POST`.
pub fn offer(app: &App, query: &Form, form: Option<&Form>, now: DateTime<Utc>) -> Result<Reply> {
    let Some(form) = form else {
        let key = query.get("operateur");
        let mut prefilled = OfferForm::for_operator(key);
        let conn = store::open_read_only(&app.paths.db_file)?;
        let board = tracking::board(&conn, app.config.bonus_freebet_value, now)?;
        // « Corriger l'offre » : on repart de l'offre la plus récente, que la nouvelle remplacera.
        if let Some(stored) = board
            .operators
            .iter()
            .find(|o| o.key == key)
            .and_then(|o| o.offer.as_ref())
        {
            prefilled.source = stored.offer.source.clone();
            prefilled.note = stored.offer.note.clone().unwrap_or_default();
            match &stored.offer.eligibility {
                Eligibility::Eligible {
                    kind,
                    amount,
                    min_odds,
                } => {
                    prefilled.kind = kind.as_str().to_string();
                    prefilled.amount = format::decimal(amount.get(), 2);
                    prefilled.min_odds =
                        min_odds.map_or_else(String::new, |m| format::decimal(m.get(), 2));
                }
                Eligibility::Ineligible => prefilled.eligible = false,
            }
        }
        return Ok(Reply::Page {
            status: 200,
            html: offer_page(prefilled, None, now)?,
        });
    };
    let typed = OfferForm::from_form(form);
    let today = now.with_timezone(&Paris).date_naive();
    match typed
        .to_action(today)
        .and_then(|action| write(app, &action, now))
    {
        Ok(message) => Ok(Reply::Done {
            to: "/bonus",
            flash: Flash::ok(message),
        }),
        Err(error) => Ok(Reply::Page {
            status: 422,
            html: offer_page(typed, Some(error), now)?,
        }),
    }
}
