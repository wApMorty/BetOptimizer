//! Séance avec les vrais clients : relevé frais de prono-match, cotes manquantes chez The Odds API, séance écrite au
//! journal, et au besoin message Discord avec l'image des grilles à remplir.
//!
//! Partagée par `edge lotofoot recommend` (timer du samedi) et le bouton « Optimiser maintenant » de l'application Web.
//! Seule la demande change : le bouton choisit sa mise et recalcule les grilles déjà recommandées, le timer non.

use chrono::Utc;
use rusqlite::Connection;

use super::fetch::{self, FetchReport, PronoMatchClient};
use super::session::{self, Request, SessionReport};
use crate::config::{API_KEY_ENV, Config, WEBHOOK_ENV};
use crate::error::{Error, Result};
use crate::format::long_date;
use crate::ingest::aliases::Aliases;
use crate::ingest::client::{HttpResponse, OddsApiClient};
use crate::notify::discord;
use crate::report::{lotofoot_image, lotofoot_page};

/// Ce qu'est devenu le message Discord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notified {
    /// Pas demandé (ou séance à blanc).
    NotAsked,
    /// Envoyé.
    Sent,
    /// Échec : la séance est écrite, seul le message manque.
    Failed(String),
}

/// Bilan d'une séance avec les vrais clients.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    /// Relevé de prono-match fait juste avant la séance.
    pub fetched: FetchReport,
    /// La séance.
    pub report: SessionReport,
    /// Le message Discord.
    pub notified: Notified,
}

/// Relève prono-match, lance la séance demandée (mise plafond, remplacement, à blanc), puis envoie le message Discord
/// si `notify`. À blanc, rien n'est écrit, aucune compétition n'est relevée et aucun message n'est envoyé.
pub fn recommend(
    conn: &mut Connection,
    config: &Config,
    aliases: &Aliases,
    request: Request,
    notify: bool,
) -> Result<Live> {
    // Un relevé juste avant la séance : la répartition utilisée a quelques minutes, pas une heure.
    let pages = PronoMatchClient::new();
    let fetched = fetch::run(
        conn,
        &mut Utc::now,
        &mut |path, params| pages.get(path, params),
        &mut std::thread::sleep,
    )?;
    let report = if request.dry_run {
        session::run(
            conn,
            config,
            aliases,
            &mut Utc::now,
            &mut |path, _| refused(path),
            request,
        )?
    } else {
        // `.ok()` : variable d'environnement absente = `None`, la clé est alors lue dans la configuration.
        let key = config.api_key(std::env::var(API_KEY_ENV).ok().as_deref())?;
        let odds = OddsApiClient::new(key);
        session::run(
            conn,
            config,
            aliases,
            &mut Utc::now,
            &mut |path, params| odds.get(path, params),
            request,
        )?
    };
    let notified = if notify && !request.dry_run {
        match send(config, &report) {
            Ok(()) => Notified::Sent,
            Err(error) => Notified::Failed(error.to_string()),
        }
    } else {
        Notified::NotAsked
    };
    Ok(Live {
        fetched,
        report,
        notified,
    })
}

/// Appel refusé à blanc : aucune compétition n'est relevée, aucun crédit n'est dépensé.
fn refused(path: &str) -> Result<HttpResponse> {
    Err(Error::Input(format!(
        "séance à blanc : appel {path} non effectué"
    )))
}

/// Message Discord de la séance, avec l'image des grilles à remplir quand il y en a.
fn send(config: &Config, report: &SessionReport) -> Result<()> {
    let webhook = config.discord_webhook(std::env::var(WEBHOOK_ENV).ok().as_deref())?;
    let text = lotofoot_page::discord_text(report, config.site_url.as_deref());
    let now = Utc::now();
    let title = format!("Loto Foot · séance du {}", long_date(now));
    // L'image ne doit pas empêcher le message : son échec est journalisé et le message part sans elle.
    let image = lotofoot_image::session_png(report, &title).unwrap_or_else(|error| {
        tracing::warn!("image de la séance non jointe : {error}");
        None
    });
    let name = format!("lotofoot-{}.png", now.format("%Y-%m-%d"));
    // `as_deref` emprunte les octets de l'image : la pièce jointe ne les copie pas.
    let attachments: Vec<discord::Attachment<'_>> = image
        .as_deref()
        .map(|png| discord::Attachment {
            filename: &name,
            content_type: "image/png",
            bytes: png,
        })
        .into_iter()
        .collect();
    discord::send(&webhook, &text, &attachments, &mut discord::http_post)
}
