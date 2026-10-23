//! Scores en direct des rencontres d'une grille : API publique et non documentée d'ESPN (`site.api.espn.com`), gratuite
//! et sans clé (`docs/sources.md` §46).
//!
//! Une page « scoreboard » par compétition et par jour : coup d'envoi, noms, score, état du match. Ce module lit cette
//! page ; le choix des appels et l'appariement des rencontres sont dans [`super::scores`].

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use chrono_tz::America::New_York;
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::ingest::client::HttpResponse;

/// Racine des pages de football.
pub const BASE_URL: &str = "https://site.api.espn.com/apis/site/v2/sports/soccer/";

/// Clé de compétition de The Odds API → code de ligue ESPN. Codes vérifiés le 2026-10-23 sur la page « scoreboard » de
/// chacun (`docs/sources.md` §46) : les compétitions des grilles (`lotofoot_leagues`), le Big 5 et l'Écosse. Seule
/// manque l'Ekstraklasa polonaise : `pol.1` répond « Failed to get events endpoint » (HTTP 400).
const LEAGUES: [(&str, &str); 36] = [
    ("soccer_epl", "eng.1"),
    ("soccer_spain_la_liga", "esp.1"),
    ("soccer_germany_bundesliga", "ger.1"),
    ("soccer_italy_serie_a", "ita.1"),
    ("soccer_france_ligue_one", "fra.1"),
    ("soccer_spl", "sco.1"),
    ("soccer_france_ligue_two", "fra.2"),
    ("soccer_uefa_champs_league", "uefa.champions"),
    ("soccer_uefa_europa_league", "uefa.europa"),
    ("soccer_uefa_europa_conference_league", "uefa.europa.conf"),
    ("soccer_efl_champ", "eng.2"),
    ("soccer_germany_bundesliga2", "ger.2"),
    ("soccer_italy_serie_b", "ita.2"),
    ("soccer_spain_segunda_division", "esp.2"),
    ("soccer_netherlands_eredivisie", "ned.1"),
    ("soccer_portugal_primeira_liga", "por.1"),
    ("soccer_belgium_first_div", "bel.1"),
    ("soccer_turkey_super_league", "tur.1"),
    ("soccer_switzerland_superleague", "sui.1"),
    ("soccer_austria_bundesliga", "aut.1"),
    ("soccer_greece_super_league", "gre.1"),
    ("soccer_denmark_superliga", "den.1"),
    ("soccer_norway_eliteserien", "nor.1"),
    ("soccer_sweden_allsvenskan", "swe.1"),
    ("soccer_league_of_ireland", "irl.1"),
    ("soccer_usa_mls", "usa.1"),
    ("soccer_brazil_campeonato", "bra.1"),
    ("soccer_argentina_primera_division", "arg.1"),
    ("soccer_uefa_nations_league", "uefa.nations"),
    (
        "soccer_fifa_world_cup_qualifiers_europe",
        "fifa.worldq.uefa",
    ),
    ("soccer_france_coupe_de_france", "fra.coupe_de_france"),
    ("soccer_germany_dfb_pokal", "ger.dfb_pokal"),
    ("soccer_england_efl_cup", "eng.league_cup"),
    ("soccer_fa_cup", "eng.fa"),
    ("soccer_spain_copa_del_rey", "esp.copa_del_rey"),
    ("soccer_italy_coppa_italia", "ita.coppa_italia"),
];

/// Code ESPN d'une compétition de The Odds API ; `None` si ESPN n'est pas relié à cette compétition.
pub fn league(sport_key: &str) -> Option<&'static str> {
    LEAGUES
        .iter()
        .find(|(key, _)| *key == sport_key)
        .map(|(_, code)| *code)
}

/// Chemin de la page des matchs d'une compétition.
pub fn scoreboard_path(league: &str) -> String {
    format!("{league}/scoreboard")
}

/// Paramètre du jour d'un coup d'envoi : `dates=AAAAMMJJ`. ESPN range ses journées à l'heure de New York (constaté le
/// 2026-10-23 : un match brésilien à 00:00 UTC le 18 figure sous la date du 17), d'où la conversion.
pub fn date_param(kickoff: DateTime<Utc>) -> String {
    format!(
        "dates={}",
        kickoff.with_timezone(&New_York).format("%Y%m%d")
    )
}

/// Client HTTP bloquant d'ESPN (aucune clé).
pub struct EspnClient {
    agent: ureq::Agent,
}

impl Default for EspnClient {
    fn default() -> Self {
        EspnClient::new()
    }
}

impl EspnClient {
    /// Crée le client. Délai court : la page `/lotofoot` attend la réponse avant de s'afficher.
    pub fn new() -> EspnClient {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(5)))
            .user_agent("edge/0.1 (usage personnel, scores des grilles Loto Foot)")
            .build();
        EspnClient {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// `GET` d'un chemin (`fra.2/scoreboard`) avec ses paramètres (`dates=20261024`).
    pub fn get(&self, path: &str, params: &str) -> Result<HttpResponse> {
        let url = format!("{BASE_URL}{path}?{params}");
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|e| Error::Http(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(HttpResponse {
            status,
            quota: Default::default(),
            body,
        })
    }
}

/// Où en est un match, du point de vue du Loto Foot (seul compte le score à la fin du temps réglementaire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Pas commencé.
    NotStarted,
    /// En cours, dans le temps réglementaire (mi-temps comprise) : score provisoire.
    Playing,
    /// En prolongation ou aux tirs au but : le score du temps réglementaire n'est plus lisible.
    ExtraTime,
    /// Terminé à la fin du temps réglementaire : score final.
    Finished,
    /// Terminé après prolongation ou tirs au but : résultat à confirmer par la FDJ.
    AfterExtraTime,
    /// Reporté, annulé, arrêté : la FDJ décide du résultat.
    Interrupted,
}

impl Phase {
    /// Code en base.
    pub fn code(self) -> &'static str {
        match self {
            Phase::NotStarted => "not_started",
            Phase::Playing => "playing",
            Phase::ExtraTime => "extra_time",
            Phase::Finished => "finished",
            Phase::AfterExtraTime => "after_extra_time",
            Phase::Interrupted => "interrupted",
        }
    }

    /// Phase d'un code en base ; `None` pour toute autre valeur.
    pub fn from_code(code: &str) -> Option<Phase> {
        [
            Phase::NotStarted,
            Phase::Playing,
            Phase::ExtraTime,
            Phase::Finished,
            Phase::AfterExtraTime,
            Phase::Interrupted,
        ]
        .into_iter()
        .find(|phase| phase.code() == code)
    }

    /// `true` quand le match ne bougera plus : inutile de le relever encore.
    pub fn is_over(self) -> bool {
        matches!(
            self,
            Phase::Finished | Phase::AfterExtraTime | Phase::Interrupted
        )
    }

    /// `true` quand le match se joue : la page se recharge seule.
    pub fn is_live(self) -> bool {
        matches!(self, Phase::Playing | Phase::ExtraTime)
    }
}

/// Un match de la page ESPN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EspnGame {
    /// Identifiant ESPN.
    pub id: String,
    /// Coup d'envoi.
    pub kickoff: DateTime<Utc>,
    /// Équipe à domicile, nom d'ESPN.
    pub home: String,
    /// Équipe à l'extérieur, nom d'ESPN.
    pub away: String,
    /// Buts de l'équipe à domicile (0 avant le coup d'envoi).
    pub home_goals: u16,
    /// Buts de l'équipe à l'extérieur.
    pub away_goals: u16,
    /// État.
    pub phase: Phase,
    /// État en clair : « 57' », « MT », « terminé »…
    pub label: String,
}

/// Page lue : les matchs reconnus et le nombre de matchs illisibles (ignorés).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scoreboard {
    /// Matchs lus.
    pub games: Vec<EspnGame>,
    /// Matchs ignorés faute d'un champ attendu.
    pub malformed: usize,
}

// Forme brute de la réponse : seuls les champs utiles sont déclarés, serde ignore les autres.
#[derive(Deserialize)]
struct RawBoard {
    #[serde(default)]
    events: Vec<RawEvent>,
}

#[derive(Deserialize)]
struct RawEvent {
    id: String,
    date: String,
    #[serde(default)]
    competitions: Vec<RawCompetition>,
}

#[derive(Deserialize)]
struct RawCompetition {
    #[serde(default)]
    competitors: Vec<RawCompetitor>,
    status: RawStatus,
}

#[derive(Deserialize)]
struct RawCompetitor {
    #[serde(rename = "homeAway")]
    home_away: String,
    score: Option<String>,
    team: RawTeam,
}

#[derive(Deserialize)]
struct RawTeam {
    #[serde(rename = "displayName")]
    display_name: String,
}

#[derive(Deserialize)]
struct RawStatus {
    #[serde(rename = "displayClock", default)]
    display_clock: String,
    #[serde(default)]
    period: u32,
    #[serde(rename = "type")]
    kind: RawStatusType,
}

#[derive(Deserialize)]
struct RawStatusType {
    name: String,
    state: String,
    #[serde(default)]
    completed: bool,
}

/// Lit une page « scoreboard ». Un match sans les deux équipes, sans date lisible ou d'état inconnu est compté dans
/// `malformed` et ignoré : un match illisible ne doit pas cacher les autres.
pub fn parse_scoreboard(body: &str) -> Result<Scoreboard> {
    let raw: RawBoard = serde_json::from_str(body)?;
    let mut board = Scoreboard {
        games: Vec::new(),
        malformed: 0,
    };
    for event in raw.events {
        match game(event) {
            Some(game) => board.games.push(game),
            None => board.malformed += 1,
        }
    }
    Ok(board)
}

/// Un match brut → un match lu, ou `None` s'il manque un champ.
fn game(event: RawEvent) -> Option<EspnGame> {
    let kickoff = kickoff(&event.date)?;
    let competition = event.competitions.into_iter().next()?;
    let side = |which: &str| {
        competition
            .competitors
            .iter()
            .find(|c| c.home_away == which)
    };
    let (home, away) = (side("home")?, side("away")?);
    let goals = |c: &RawCompetitor| c.score.as_deref().unwrap_or("0").parse::<u16>().ok();
    let phase = phase(&competition.status)?;
    Some(EspnGame {
        id: event.id,
        kickoff,
        home: home.team.display_name.clone(),
        away: away.team.display_name.clone(),
        home_goals: goals(home)?,
        away_goals: goals(away)?,
        phase,
        label: label(phase, &competition.status),
    })
}

/// ESPN écrit ses dates sans secondes (`2026-10-24T18:45Z`) ; on accepte aussi la forme RFC 3339 complète.
fn kickoff(text: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%MZ")
        .map(|naive| naive.and_utc())
        .ok()
        .or_else(|| {
            DateTime::parse_from_rfc3339(text)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        })
}

/// État d'un match, d'après `status.type.state` (`pre`, `in`, `post`), la période (1 et 2 : temps réglementaire ; 3 et 4 :
/// prolongation ; 5 : tirs au but) et le nom de l'état (constatés le 2026-10-23, `docs/sources.md` §46).
fn phase(status: &RawStatus) -> Option<Phase> {
    let name = status.kind.name.as_str();
    let stopped = ["POSTPONED", "CANCELED", "CANCELLED", "ABANDONED"]
        .iter()
        .any(|word| name.contains(word));
    let beyond = status.period >= 3 || name == "STATUS_FINAL_AET" || name == "STATUS_FINAL_PEN";
    match status.kind.state.as_str() {
        "pre" if stopped => Some(Phase::Interrupted),
        "pre" => Some(Phase::NotStarted),
        "in" if beyond => Some(Phase::ExtraTime),
        "in" => Some(Phase::Playing),
        // Un match « post » non terminé a été arrêté ou reporté.
        "post" if !status.kind.completed => Some(Phase::Interrupted),
        "post" if beyond => Some(Phase::AfterExtraTime),
        "post" => Some(Phase::Finished),
        _ => None,
    }
}

/// État en clair, affiché à côté du score.
fn label(phase: Phase, status: &RawStatus) -> String {
    match phase {
        Phase::NotStarted => "à venir".to_string(),
        Phase::Playing if status.kind.name == "STATUS_HALFTIME" => "MT".to_string(),
        Phase::Playing => status.display_clock.clone(),
        Phase::ExtraTime => format!("prol. {}", status.display_clock),
        Phase::Finished => "terminé".to_string(),
        Phase::AfterExtraTime => "après prol.".to_string(),
        Phase::Interrupted => "interrompu ou reporté".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une page réduite aux champs lus, avec un match par état utile (forme constatée le 2026-10-23).
    const BOARD: &str = r#"{"leagues":[{"name":"French Ligue 2"}],"events":[
      {"id":"1","date":"2026-10-24T12:00Z","competitions":[{"competitors":[
         {"homeAway":"home","score":"0","team":{"displayName":"Dijon FCO"}},
         {"homeAway":"away","score":"0","team":{"displayName":"Rodez Aveyron"}}],
       "status":{"displayClock":"0'","period":0,"type":{"name":"STATUS_SCHEDULED","state":"pre","completed":false}}}]},
      {"id":"2","date":"2026-10-24T18:00Z","competitions":[{"competitors":[
         {"homeAway":"away","score":"1","team":{"displayName":"Grenoble"}},
         {"homeAway":"home","score":"2","team":{"displayName":"Pau FC"}}],
       "status":{"displayClock":"57'","period":2,"type":{"name":"STATUS_SECOND_HALF","state":"in","completed":false}}}]},
      {"id":"3","date":"2026-10-24T18:00Z","competitions":[{"competitors":[
         {"homeAway":"home","score":"1","team":{"displayName":"Amiens"}},
         {"homeAway":"away","score":"1","team":{"displayName":"Laval"}}],
       "status":{"displayClock":"45'+2'","period":1,"type":{"name":"STATUS_HALFTIME","state":"in","completed":false}}}]},
      {"id":"4","date":"2026-10-24T15:00Z","competitions":[{"competitors":[
         {"homeAway":"home","score":"3","team":{"displayName":"Troyes"}},
         {"homeAway":"away","score":"0","team":{"displayName":"Annecy"}}],
       "status":{"displayClock":"90'+4'","period":2,"type":{"name":"STATUS_FULL_TIME","state":"post","completed":true}}}]},
      {"id":"5","date":"2026-08-26T18:45Z","competitions":[{"competitors":[
         {"homeAway":"home","score":"0","team":{"displayName":"Bradford City"}},
         {"homeAway":"away","score":"0","team":{"displayName":"Burnley"}}],
       "status":{"displayClock":"96'","period":5,"type":{"name":"STATUS_FINAL_PEN","state":"post","completed":true}}}]},
      {"id":"6","date":"2026-10-24T18:00Z","competitions":[{"competitors":[
         {"homeAway":"home","team":{"displayName":"Bastia"}},
         {"homeAway":"away","team":{"displayName":"Metz"}}],
       "status":{"displayClock":"0'","period":0,"type":{"name":"STATUS_POSTPONED","state":"post","completed":false}}}]},
      {"id":"7","date":"pas une date","competitions":[]}
    ]}"#;

    #[test]
    fn a_scoreboard_reads_scores_and_phases() {
        let board = parse_scoreboard(BOARD).unwrap();
        assert_eq!(board.malformed, 1);
        let phases: Vec<(&str, Phase, &str)> = board
            .games
            .iter()
            .map(|g| (g.id.as_str(), g.phase, g.label.as_str()))
            .collect();
        assert_eq!(
            phases,
            vec![
                ("1", Phase::NotStarted, "à venir"),
                ("2", Phase::Playing, "57'"),
                ("3", Phase::Playing, "MT"),
                ("4", Phase::Finished, "terminé"),
                ("5", Phase::AfterExtraTime, "après prol."),
                ("6", Phase::Interrupted, "interrompu ou reporté"),
            ]
        );
        // Le domicile se lit sur `homeAway`, pas sur l'ordre de la liste.
        let pau = &board.games[1];
        assert_eq!(
            (pau.home.as_str(), pau.away.as_str()),
            ("Pau FC", "Grenoble")
        );
        assert_eq!((pau.home_goals, pau.away_goals), (2, 1));
        assert_eq!(pau.kickoff.to_rfc3339(), "2026-10-24T18:00:00+00:00");
    }

    #[test]
    fn extra_time_in_progress_is_not_a_regulation_score() {
        let body = BOARD.replace(
            r#""period":2,"type":{"name":"STATUS_SECOND_HALF""#,
            r#""period":3,"type":{"name":"STATUS_FIRST_HALF_EXTRA_TIME""#,
        );
        let board = parse_scoreboard(&body).unwrap();
        assert_eq!(board.games[1].phase, Phase::ExtraTime);
        assert!(board.games[1].phase.is_live());
        assert!(!board.games[1].phase.is_over());
    }

    #[test]
    fn the_day_parameter_follows_new_york_time() {
        let late = DateTime::parse_from_rfc3339("2026-10-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(date_param(late), "dates=20261017");
        let afternoon = DateTime::parse_from_rfc3339("2026-10-24T13:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(date_param(afternoon), "dates=20261024");
    }

    #[test]
    fn every_lotofoot_league_but_poland_has_an_espn_code() {
        for key in crate::config::Config::default().lotofoot_leagues {
            let expected = key != "soccer_poland_ekstraklasa";
            assert_eq!(league(&key).is_some(), expected, "{key}");
        }
        assert_eq!(league("soccer_france_ligue_two"), Some("fra.2"));
        assert_eq!(league("basketball_nba"), None);
        for phase in [Phase::NotStarted, Phase::Finished, Phase::Interrupted] {
            assert_eq!(Phase::from_code(phase.code()), Some(phase));
        }
    }
}
