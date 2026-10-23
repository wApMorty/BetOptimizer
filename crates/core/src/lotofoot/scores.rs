//! Suivi en direct des grilles recommandées : scores relevés chez ESPN à l'affichage de `/lotofoot`, puis état de chaque
//! ticket (sans faute, une erreur, perdu) pour le code couleur de la page (`docs/sources.md` §46).
//!
//! 1. [`tracked`] retrouve, pour chaque rencontre, sa compétition et son coup d'envoi dans notre archive The Odds API
//!    (les appels `/odds` figés avec la recommandation).
//! 2. [`refresh`] appelle ESPN pour les rencontres commencées et pas finies, au plus une fois par minute et par page.
//!    Les noms d'ESPN passent par la section `ESPN` de la table d'alias : un nom inconnu laisse la rencontre sans score,
//!    jamais d'appariement approximatif.
//! 3. [`grid_now`] et [`Standing`] disent où en est chaque ticket. Un match en cours compte avec son score du moment
//!    (choix de Paul du 2026-10-23) ; les résultats officiels, dès qu'ils sont publiés, priment.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::espn::{self, EspnGame, Phase, parse_scoreboard};
use super::odds::canonical_matches;
use super::{Game, Ticket};
use crate::domain::Outcome;
use crate::error::Result;
use crate::ingest::aliases::Aliases;
use crate::ingest::snapshot::Fetch;
use crate::store::journal::parse_timestamp;
use crate::store::lotofoot::{self as store, RecoRow};
use crate::store::lotofoot_live::{self as live, LiveScore};

/// Section de la table d'alias : noms d'ESPN → noms de The Odds API.
pub const ALIAS_SECTION: &str = "ESPN";

/// Délai minimal entre deux appels à la même page ESPN : la page se recharge toutes les minutes, inutile d'aller plus vite.
pub const MIN_INTERVAL: Duration = Duration::seconds(60);

/// Au-delà de ce délai après le coup d'envoi, on cesse de relever un match qu'ESPN ne dit toujours pas terminé.
pub const GIVE_UP: Duration = Duration::hours(6);

/// Écart toléré entre le coup d'envoi d'ESPN et le nôtre pour signaler un nom inconnu (pas pour apparier : l'appariement
/// se fait sur les noms seuls).
const NEARBY: Duration = Duration::hours(3);

/// Une rencontre d'une grille recommandée, telle qu'on la cherche chez ESPN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracked {
    /// Formule.
    pub game: Game,
    /// Numéro de la grille.
    pub number: u32,
    /// Position dans la grille, à partir de 1.
    pub position: u32,
    /// Équipe à domicile, nom de The Odds API.
    pub home: String,
    /// Équipe à l'extérieur, nom de The Odds API.
    pub away: String,
    /// Compétition (clé de The Odds API).
    pub sport_key: String,
    /// Coup d'envoi (The Odds API).
    pub kickoff: DateTime<Utc>,
}

/// Rencontres à suivre des grilles des tickets donnés, et ce qui empêche d'en suivre certaines.
pub fn tracked(
    conn: &Connection,
    aliases: &Aliases,
    recos: &[RecoRow],
) -> Result<(Vec<Tracked>, Vec<String>)> {
    let mut out = Vec::new();
    let mut issues = Vec::new();
    let mut seen: Vec<(Game, u32)> = Vec::new();
    for reco in recos {
        if seen.contains(&(reco.game, reco.number)) {
            continue;
        }
        seen.push((reco.game, reco.number));
        // Résultats officiels publiés : plus rien à suivre, ni à signaler.
        if store::results(conn, reco.game, reco.number)?.is_some() {
            continue;
        }
        let name = format!("{} n°{}", reco.game, reco.number);
        let Some(grid) = store::grid(conn, reco.game, reco.number)? else {
            continue;
        };
        let teams = match canonical_matches(aliases, &grid.matches) {
            Ok(teams) => teams,
            Err(unknown) => {
                issues.push(format!("{name} : noms inconnus {}", unknown.join(", ")));
                continue;
            }
        };
        // Les appels `/odds` d'où viennent les cotes, figés avec la recommandation : un par rencontre, dans l'ordre.
        let calls: Vec<i64> = serde_json::from_str::<serde_json::Value>(&reco.inputs)
            .ok()
            .and_then(|v| {
                v.get("odds_calls")?
                    .as_array()?
                    .iter()
                    .map(serde_json::Value::as_i64)
                    .collect()
            })
            .unwrap_or_default();
        for (index, (home, away)) in teams.into_iter().enumerate() {
            let position = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let event = match calls.get(index) {
                Some(&call) => event(conn, call, &home, &away)?,
                None => None,
            };
            match event {
                Some((sport_key, kickoff)) => out.push(Tracked {
                    game: reco.game,
                    number: reco.number,
                    position,
                    home,
                    away,
                    sport_key,
                    kickoff,
                }),
                None => issues.push(format!(
                    "{name}, rencontre {position} ({home} – {away}) : coup d'envoi introuvable dans l'archive"
                )),
            }
        }
    }
    Ok((out, issues))
}

/// Compétition et coup d'envoi d'un match vu par un appel `/odds` de l'archive.
fn event(
    conn: &Connection,
    api_call_id: i64,
    home: &str,
    away: &str,
) -> Result<Option<(String, DateTime<Utc>)>> {
    let found: Option<(String, String)> = conn
        .query_row(
            "SELECT sport_key, commence_time FROM event_seen
             WHERE api_call_id = ?1 AND home_team_raw = ?2 AND away_team_raw = ?3",
            params![api_call_id, home, away],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match found {
        Some((sport_key, kickoff)) => Ok(Some((sport_key, parse_timestamp(&kickoff)?))),
        None => Ok(None),
    }
}

/// Bilan d'un relevé.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefreshReport {
    /// Rencontres commencées et pas finies : tant qu'il y en a, la page se recharge seule.
    pub watching: usize,
    /// Pages ESPN appelées (`fra.2 20261024`).
    pub fetched: Vec<String>,
    /// Ce qui a empêché d'apparier des rencontres pendant ce relevé.
    pub issues: Vec<String>,
}

/// Relève chez ESPN les rencontres suivies commencées depuis moins de [`GIVE_UP`] et pas encore finies ; une page au plus
/// par compétition et par jour, et pas plus d'une fois par [`MIN_INTERVAL`].
pub fn refresh(
    conn: &Connection,
    aliases: &Aliases,
    tracked: &[Tracked],
    now: DateTime<Utc>,
    fetch: Fetch,
) -> Result<RefreshReport> {
    let mut report = RefreshReport::default();
    // Pages à appeler : (code ESPN, paramètre du jour) → rencontres à y chercher, dans l'ordre de la première vue.
    let mut pages: Vec<((&'static str, String), Vec<&Tracked>)> = Vec::new();
    for t in tracked {
        if !due(conn, t, now)? {
            continue;
        }
        report.watching += 1;
        let Some(league) = espn::league(&t.sport_key) else {
            report
                .issues
                .push(format!("{} : compétition sans code ESPN", t.sport_key));
            continue;
        };
        let key = (league, espn::date_param(t.kickoff));
        match pages.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(t),
            None => pages.push((key, vec![t])),
        }
    }
    for ((league, day), wanted) in pages {
        if let Some(last) = live::last_fetch(conn, league, &day)?
            && now - last < MIN_INTERVAL
        {
            continue;
        }
        report.fetched.push(format!("{league} {day}"));
        let issue = match fetch(&espn::scoreboard_path(league), &day) {
            Err(error) => Some(format!("ESPN {league} : {error}")),
            Ok(response) if response.status != 200 => {
                Some(format!("ESPN {league} : HTTP {}", response.status))
            }
            Ok(response) => match parse_scoreboard(&response.body) {
                Err(error) => Some(format!("ESPN {league} : {error}")),
                Ok(board) => {
                    let mut problems = Vec::new();
                    for t in wanted {
                        match pair(aliases, t, &board.games) {
                            Ok(game) => live::upsert_score(
                                conn,
                                t.game,
                                t.number,
                                t.position,
                                &LiveScore {
                                    league: league.to_string(),
                                    event_id: game.id.clone(),
                                    home_goals: game.home_goals,
                                    away_goals: game.away_goals,
                                    phase: game.phase,
                                    label: game.label.clone(),
                                    fetched_at: now,
                                },
                            )?,
                            Err(problem) => problems.push(problem),
                        }
                    }
                    (!problems.is_empty()).then(|| problems.join(" ; "))
                }
            },
        };
        // `as_deref` prête le texte (`Option<&str>`) sans le copier ; `extend` ajoute ensuite le `Option` lui-même, qui se
        // parcourt comme une liste de zéro ou un élément.
        live::record_fetch(conn, league, &day, now, issue.as_deref())?;
        report.issues.extend(issue);
    }
    Ok(report)
}

/// `true` si la rencontre est commencée et pas finie selon le dernier relevé ([`tracked`] a déjà écarté les grilles
/// dont les résultats officiels sont publiés).
fn due(conn: &Connection, t: &Tracked, now: DateTime<Utc>) -> Result<bool> {
    if now < t.kickoff || now > t.kickoff + GIVE_UP {
        return Ok(false);
    }
    let over = live::scores(conn, t.game, t.number)?
        .iter()
        .any(|(position, score)| *position == t.position && score.phase.is_over());
    Ok(!over)
}

/// Le match d'ESPN dont les deux noms, passés par la table d'alias, sont ceux de la rencontre. Sinon, le problème : noms
/// d'ESPN inconnus parmi les matchs proches du coup d'envoi, ou rencontre absente de la page.
///
/// La durée de vie `'a` dit au compilateur que la référence rendue pointe dans `games` (et non dans `aliases` ou `t`) :
/// le match trouvé reste utilisable tant que la page lue existe, sans copie.
fn pair<'a>(
    aliases: &Aliases,
    t: &Tracked,
    games: &'a [EspnGame],
) -> std::result::Result<&'a EspnGame, String> {
    let resolve = |name: &str| aliases.resolve(ALIAS_SECTION, name);
    if let Some(game) = games.iter().find(|g| {
        resolve(&g.home) == Some(t.home.as_str()) && resolve(&g.away) == Some(t.away.as_str())
    }) {
        return Ok(game);
    }
    let mut unknown: Vec<&str> = games
        .iter()
        .filter(|g| (g.kickoff - t.kickoff).abs() <= NEARBY)
        .flat_map(|g| [g.home.as_str(), g.away.as_str()])
        .filter(|name| resolve(name).is_none())
        .collect();
    unknown.dedup();
    if unknown.is_empty() {
        Err(format!("{} – {} absent de la page ESPN", t.home, t.away))
    } else {
        Err(format!(
            "{} – {} : noms ESPN absents de la section [{ALIAS_SECTION}] de config/aliases.toml : {}",
            t.home,
            t.away,
            unknown.join(", ")
        ))
    }
}

/// Problèmes notés au dernier appel des pages ESPN des rencontres suivies (la page les affiche même sans nouvel appel).
pub fn stored_issues(conn: &Connection, tracked: &[Tracked]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in tracked {
        let Some(league) = espn::league(&t.sport_key) else {
            continue;
        };
        if let Some(issue) = live::fetch_issue(conn, league, &espn::date_param(t.kickoff))?
            && !out.contains(&issue)
        {
            out.push(issue);
        }
    }
    Ok(out)
}

/// Où en est une rencontre, pour l'affichage et le décompte des erreurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchNow {
    /// Signe du moment : définitif si `settled`, provisoire sinon ; `None` tant qu'il n'y en a pas (pas commencé,
    /// prolongation, match interrompu).
    pub outcome: Option<Outcome>,
    /// Le signe ne bougera plus (résultat officiel ou match terminé dans le temps réglementaire).
    pub settled: bool,
    /// Le match se joue.
    pub live: bool,
    /// Score (« 2 – 1 »), vide sans relevé.
    pub score: String,
    /// État en clair (« 57' », « terminé », « résultat officiel »).
    pub label: String,
}

/// Issue d'un score : `1` si l'équipe à domicile mène, `N` à égalité, `2` sinon.
fn outcome(home_goals: u16, away_goals: u16) -> Outcome {
    match home_goals.cmp(&away_goals) {
        std::cmp::Ordering::Greater => Outcome::Home,
        std::cmp::Ordering::Equal => Outcome::Draw,
        std::cmp::Ordering::Less => Outcome::Away,
    }
}

/// État de chaque rencontre d'une grille, dans l'ordre. Les résultats officiels priment sur les scores d'ESPN.
pub fn grid_now(conn: &Connection, game: Game, number: u32) -> Result<Vec<MatchNow>> {
    let official = store::results(conn, game, number)?;
    let scores = live::scores(conn, game, number)?;
    let mut out = Vec::new();
    for index in 0..game.matches() {
        let position = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let score = scores.iter().find(|(p, _)| *p == position).map(|(_, s)| s);
        let text = score.map_or(String::new(), |s| {
            format!("{} – {}", s.home_goals, s.away_goals)
        });
        let now = match (&official, score) {
            (Some(results), _) => MatchNow {
                outcome: results.get(index).copied(),
                settled: true,
                live: false,
                score: text,
                label: "résultat officiel".to_string(),
            },
            (None, None) => MatchNow {
                outcome: None,
                settled: false,
                live: false,
                score: text,
                label: String::new(),
            },
            (None, Some(s)) => {
                let counted = matches!(s.phase, Phase::Playing | Phase::Finished);
                MatchNow {
                    outcome: counted.then(|| outcome(s.home_goals, s.away_goals)),
                    settled: s.phase == Phase::Finished,
                    live: s.phase.is_live(),
                    score: if s.phase == Phase::NotStarted {
                        String::new()
                    } else {
                        text
                    },
                    label: match s.phase {
                        Phase::AfterExtraTime | Phase::ExtraTime => {
                            format!("{} : à confirmer", s.label)
                        }
                        _ => s.label.clone(),
                    },
                }
            }
        };
        out.push(now);
    }
    Ok(out)
}

/// Où en est un ticket ou une grille multiple : erreurs sur les rencontres dont le signe est connu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    /// Erreurs, provisoires comprises.
    pub errors: usize,
    /// Erreurs définitives.
    pub settled_errors: usize,
    /// Rencontres dont le signe est connu (provisoire ou définitif).
    pub known: usize,
    /// Une rencontre en cours compte dans `errors`.
    pub provisional: bool,
    /// Toutes les rencontres ont un signe définitif.
    pub complete: bool,
}

impl Standing {
    /// État de signes cochés contre les rencontres : `covers[i]` liste les signes cochés pour la rencontre `i` (un seul
    /// pour un ticket, deux ou trois pour un double ou un triple). Pour une grille multiple, le compte est celui de son
    /// meilleur ticket : une rencontre n'est fausse que si aucun signe coché n'est sorti.
    pub fn of(covers: &[Vec<Outcome>], matches: &[MatchNow]) -> Standing {
        let mut standing = Standing {
            errors: 0,
            settled_errors: 0,
            known: 0,
            provisional: false,
            complete: !matches.is_empty(),
        };
        for (signs, m) in covers.iter().zip(matches) {
            standing.complete &= m.settled;
            let Some(outcome) = m.outcome else {
                continue;
            };
            standing.known += 1;
            if signs.contains(&outcome) {
                continue;
            }
            standing.errors += 1;
            if m.settled {
                standing.settled_errors += 1;
            } else {
                standing.provisional = true;
            }
        }
        standing
    }

    /// Signes d'un ticket simple, un par rencontre.
    pub fn covers(ticket: &Ticket) -> Vec<Vec<Outcome>> {
        ticket.signs().iter().map(|&sign| vec![sign]).collect()
    }

    /// Classe CSS du code couleur : gris avant le premier signe connu, vert sans faute, orange à une erreur (2e rang
    /// encore possible), rouge au-delà.
    pub fn tone(&self) -> &'static str {
        match (self.known, self.errors) {
            (0, _) => "t-wait",
            (_, 0) => "t-pos",
            (_, 1) => "t-warn",
            _ => "t-neg",
        }
    }

    /// État en clair.
    pub fn label(&self) -> String {
        if self.complete {
            return match self.errors {
                0 => "gagné : sans faute".to_string(),
                1 => "gagné : une erreur".to_string(),
                _ => "perdu".to_string(),
            };
        }
        if self.settled_errors >= 2 {
            return "perdu".to_string();
        }
        let text = match (self.known, self.errors) {
            (0, _) => return "à venir".to_string(),
            (_, 0) => "sans faute".to_string(),
            (_, 1) => "une erreur".to_string(),
            (_, n) => format!("{n} erreurs"),
        };
        if self.provisional {
            format!("{text} (en cours)")
        } else {
            text
        }
    }

    /// Encore en lice pour un rang payé (au plus une erreur).
    pub fn alive(&self) -> bool {
        self.errors <= 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::client::HttpResponse;
    use crate::lotofoot::pronomatch::{GridPage, OpenGrid};
    use crate::store::lotofoot::{Page, record_fetch, record_grid_page};
    use crate::store::migrate::migrate;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const ALIASES: &str = "[LOTOFOOT]\n\"Pau FC\" = [\"Pau\"]\n\"Grenoble\" = []\n\"Dijon\" = []\n\"Rodez\" = []\n\
                           [ESPN]\n\"Pau FC\" = []\n\"Grenoble\" = [\"Grenoble Foot 38\"]\n";

    /// Une grille LF7 n°133 dont seules les deux premières rencontres sont archivées (appel `/odds` n°1).
    fn setup() -> (Connection, Aliases, RecoRow) {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let fetch_id = record_fetch(
            &conn,
            at("2026-10-24T09:00:00Z"),
            Page::Grid(Game::Lf7),
            200,
            None,
        )
        .unwrap();
        let mut matches = vec![
            ("Pau".to_string(), "Grenoble".to_string()),
            ("Dijon".to_string(), "Rodez".to_string()),
        ];
        matches.extend((3..=7).map(|i| (format!("H{i}"), format!("A{i}"))));
        let page = GridPage {
            open: Some(OpenGrid {
                game: Game::Lf7,
                accepting: true,
                number: 133,
                closes_at: at("2026-10-24T11:50:00Z"),
                jackpot: None,
                matches,
                split: None,
                split_at: None,
            }),
            history: Vec::new(),
        };
        record_grid_page(&mut conn, fetch_id, Game::Lf7, &page).unwrap();
        conn.execute_batch(
            "INSERT INTO api_call (id, called_at, endpoint, sport_key, params, http_status)
               VALUES (1, '2026-10-24T08:00:00Z', 'odds', 'soccer_france_ligue_two', '', 200);
             INSERT INTO event_seen VALUES (1, 'e1', 'soccer_france_ligue_two', '2026-10-24T18:00:00Z', 'Pau FC', 'Grenoble');
             INSERT INTO event_seen VALUES (1, 'e2', 'soccer_france_ligue_two', '2026-10-24T16:00:00Z', 'Dijon', 'Rodez');",
        )
        .unwrap();
        // Les rencontres 3 à 7 ont des noms connus mais pas d'appel `/odds` figé : elles ne sont pas suivies.
        let others: String = (3..=7)
            .map(|i| format!("\"H{i}\" = []\n\"A{i}\" = []\n"))
            .collect();
        let aliases = Aliases::parse(&ALIASES.replace("[ESPN]", &(others + "[ESPN]"))).unwrap();
        let reco = RecoRow {
            id: 1,
            session_id: 1,
            created_at: at("2026-10-24T10:00:00Z"),
            game: Game::Lf7,
            number: 133,
            ticket: Ticket::parse("1N11111").unwrap(),
            position: 1,
            ev_base: 1.2,
            ev_pessimistic: 1.0,
            paid_probability: 0.01,
            stakes_estimate: 100_000.0,
            inputs: r#"{"odds_calls":[1,1]}"#.to_string(),
        };
        (conn, aliases, reco)
    }

    fn board(pau: (&str, &str, u32, &str, &str)) -> String {
        let (home_goals, clock, period, name, state) = pau;
        format!(
            r#"{{"events":[{{"id":"401","date":"2026-10-24T18:00Z","competitions":[{{"competitors":[
              {{"homeAway":"home","score":"{home_goals}","team":{{"displayName":"Pau FC"}}}},
              {{"homeAway":"away","score":"1","team":{{"displayName":"Grenoble Foot 38"}}}}],
              "status":{{"displayClock":"{clock}","period":{period},"type":{{"name":"{name}","state":"{state}","completed":{}}}}}}}]}},
             {{"id":"402","date":"2026-10-24T16:00Z","competitions":[{{"competitors":[
              {{"homeAway":"home","score":"0","team":{{"displayName":"Dijon FCO"}}}},
              {{"homeAway":"away","score":"0","team":{{"displayName":"Rodez Aveyron"}}}}],
              "status":{{"displayClock":"90'","period":2,"type":{{"name":"STATUS_FULL_TIME","state":"post","completed":true}}}}}}]}}]}}"#,
            state == "post"
        )
    }

    #[test]
    fn grid_matches_are_found_in_the_odds_archive() {
        let (conn, aliases, reco) = setup();
        let (tracked, issues) = tracked(&conn, &aliases, &[reco]).unwrap();
        assert_eq!(tracked.len(), 2);
        assert_eq!(tracked[0].home, "Pau FC");
        assert_eq!(tracked[0].kickoff, at("2026-10-24T18:00:00Z"));
        assert_eq!(tracked[1].sport_key, "soccer_france_ligue_two");
        // Rencontres 3 à 7 : pas d'appel `/odds` figé, signalées.
        assert_eq!(issues.len(), 5);
    }

    #[test]
    fn espn_is_called_only_during_matches_and_at_most_once_a_minute() {
        let (conn, aliases, reco) = setup();
        let (tracked, _) = tracked(&conn, &aliases, &[reco]).unwrap();
        let mut calls: Vec<String> = Vec::new();
        let body = board(("2", "57'", 2, "STATUS_SECOND_HALF", "in"));
        let mut fetch = |path: &str, params: &str| {
            calls.push(format!("{path}?{params}"));
            Ok(HttpResponse {
                status: 200,
                quota: Default::default(),
                body: body.clone(),
            })
        };
        // Avant tout coup d'envoi : aucun appel.
        let before = refresh(
            &conn,
            &aliases,
            &tracked,
            at("2026-10-24T11:00:00Z"),
            &mut fetch,
        )
        .unwrap();
        assert_eq!(before.watching, 0);
        // Pendant Pau – Grenoble : un appel ; Dijon – Rodez est sur la même page mais ses noms ESPN sont inconnus.
        let during = refresh(
            &conn,
            &aliases,
            &tracked,
            at("2026-10-24T19:00:00Z"),
            &mut fetch,
        )
        .unwrap();
        assert_eq!(during.watching, 2);
        assert_eq!(during.fetched, vec!["fra.2 dates=20261024"]);
        assert_eq!(during.issues.len(), 1);
        assert!(
            during.issues[0].contains("Dijon FCO, Rodez Aveyron"),
            "{:?}",
            during.issues
        );
        // Trente secondes plus tard : pas de nouvel appel, mais le problème reste lisible.
        let soon = refresh(
            &conn,
            &aliases,
            &tracked,
            at("2026-10-24T19:00:30Z"),
            &mut fetch,
        )
        .unwrap();
        assert!(soon.fetched.is_empty());
        assert_eq!(stored_issues(&conn, &tracked).unwrap().len(), 1);
        drop(fetch);
        assert_eq!(calls, vec!["fra.2/scoreboard?dates=20261024"]);
        let now = grid_now(&conn, Game::Lf7, 133).unwrap();
        assert_eq!(now[0].outcome, Some(Outcome::Home));
        assert!(now[0].live && !now[0].settled);
        assert_eq!(
            (now[0].score.as_str(), now[0].label.as_str()),
            ("2 – 1", "57'")
        );
        assert_eq!(now[1].outcome, None);
    }

    #[test]
    fn a_finished_match_is_not_fetched_again() {
        let (conn, aliases, reco) = setup();
        let (tracked, _) = tracked(&conn, &aliases, &[reco]).unwrap();
        let body = board(("1", "90'+4'", 2, "STATUS_FULL_TIME", "post"));
        let mut count = 0;
        let mut fetch = |_: &str, _: &str| {
            count += 1;
            Ok(HttpResponse {
                status: 200,
                quota: Default::default(),
                body: body.clone(),
            })
        };
        // Seule Pau – Grenoble est commencée à 18 h 30 ; terminée dès le premier relevé, elle n'est plus relevée.
        let tracked_pau = &tracked[..1];
        refresh(
            &conn,
            &aliases,
            tracked_pau,
            at("2026-10-24T20:00:00Z"),
            &mut fetch,
        )
        .unwrap();
        let later = refresh(
            &conn,
            &aliases,
            tracked_pau,
            at("2026-10-24T20:10:00Z"),
            &mut fetch,
        )
        .unwrap();
        assert_eq!(later.watching, 0);
        drop(fetch);
        assert_eq!(count, 1);
        let now = grid_now(&conn, Game::Lf7, 133).unwrap();
        assert_eq!(now[0].outcome, Some(Outcome::Draw));
        assert!(now[0].settled);
    }

    #[test]
    fn extra_time_leaves_the_sign_unknown() {
        let (conn, aliases, reco) = setup();
        let (tracked, _) = tracked(&conn, &aliases, &[reco]).unwrap();
        let body = board(("2", "120'", 5, "STATUS_FINAL_PEN", "post"));
        let mut fetch = |_: &str, _: &str| {
            Ok(HttpResponse {
                status: 200,
                quota: Default::default(),
                body: body.clone(),
            })
        };
        refresh(
            &conn,
            &aliases,
            &tracked[..1],
            at("2026-10-24T21:00:00Z"),
            &mut fetch,
        )
        .unwrap();
        let now = grid_now(&conn, Game::Lf7, 133).unwrap();
        assert_eq!(now[0].outcome, None);
        assert_eq!(now[0].label, "après prol. : à confirmer");
    }

    fn known(outcome: Outcome, settled: bool) -> MatchNow {
        MatchNow {
            outcome: Some(outcome),
            settled,
            live: !settled,
            score: String::new(),
            label: String::new(),
        }
    }

    fn pending() -> MatchNow {
        MatchNow {
            outcome: None,
            settled: false,
            live: false,
            score: String::new(),
            label: String::new(),
        }
    }

    #[test]
    fn a_ticket_turns_from_grey_to_green_orange_and_red() {
        use Outcome::{Away, Draw, Home};
        let ticket = Standing::covers(&Ticket::parse("1N2").unwrap());
        let waiting = Standing::of(&ticket, &[pending(), pending(), pending()]);
        assert_eq!(
            (waiting.tone(), waiting.label().as_str()),
            ("t-wait", "à venir")
        );
        let good = Standing::of(&ticket, &[known(Home, true), pending(), pending()]);
        assert_eq!(
            (good.tone(), good.label().as_str()),
            ("t-pos", "sans faute")
        );
        let one = Standing::of(&ticket, &[known(Home, true), known(Home, false), pending()]);
        assert_eq!(
            (one.tone(), one.label().as_str()),
            ("t-warn", "une erreur (en cours)")
        );
        assert!(one.alive());
        let dead = Standing::of(&ticket, &[known(Away, true), known(Home, true), pending()]);
        assert_eq!((dead.tone(), dead.label().as_str()), ("t-neg", "perdu"));
        assert!(!dead.alive());
        let won = Standing::of(
            &ticket,
            &[known(Home, true), known(Draw, true), known(Home, true)],
        );
        assert_eq!(won.label(), "gagné : une erreur");
    }

    #[test]
    fn a_multiple_grid_counts_its_best_ticket() {
        use Outcome::{Away, Draw, Home};
        // Double 1N, puis simple 2 : le 1 et le N sortis sur la première rencontre sont tous deux couverts.
        let covers = vec![vec![Home, Draw], vec![Away]];
        let s = Standing::of(&covers, &[known(Draw, true), known(Home, true)]);
        assert_eq!((s.errors, s.complete), (1, true));
        assert_eq!(s.label(), "gagné : une erreur");
    }
}
