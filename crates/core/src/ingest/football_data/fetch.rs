//! Téléchargement des CSV de football-data.co.uk, avec cache sur disque.
//!
//! Le site limite le débit (réponse 429 après une rafale de requêtes) : on ne télécharge que ce qui manque,
//! avec une pause entre deux requêtes. Une saison passée est définitive, donc mise en cache pour toujours ;
//! la saison en cours est rafraîchie au plus une fois par jour.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{League, Season};
use crate::error::{Error, Result};

/// Âge au-delà duquel le fichier de la saison en cours est retéléchargé.
const REFRESH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// Réponse HTTP réduite à son statut et son corps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBytes {
    /// Code de statut HTTP.
    pub status: u16,
    /// Corps de la réponse, octets bruts (l'encodage est géré à la lecture).
    pub body: Vec<u8>,
}

/// Fonction qui effectue un `GET` sur une adresse complète.
pub type GetBytes<'a> = &'a mut dyn FnMut(&str) -> Result<HttpBytes>;

/// Bilan d'un téléchargement.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchReport {
    /// Fichiers téléchargés.
    pub downloaded: usize,
    /// Fichiers déjà en cache et récents, non retéléchargés.
    pub cached: usize,
    /// Fichiers de la saison en cours que le site n'a pas encore publiés (HTTP 404).
    pub not_published: Vec<String>,
}

/// Emplacement d'un fichier dans le cache : `E0_2526.csv`.
pub fn cache_path(dir: &Path, league: &League, season: Season) -> PathBuf {
    dir.join(format!("{}_{}.csv", league.div, season.code()))
}

/// Télécharge les fichiers manquants (ou périmés, ou tous si `force`).
///
/// `pause` est appelée avant chaque requête sauf la première ; en production elle dort 1,5 s.
pub fn fetch_all(
    dir: &Path,
    wanted: &[(League, Season)],
    current: Season,
    force: bool,
    now: SystemTime,
    get: GetBytes,
    pause: &mut dyn FnMut(),
) -> Result<FetchReport> {
    fs::create_dir_all(dir).map_err(Error::io(dir))?;
    let mut report = FetchReport::default();
    let mut requested_before = false;
    for (league, season) in wanted {
        let path = cache_path(dir, league, *season);
        if !force && is_fresh(&path, *season == current, now) {
            report.cached += 1;
            continue;
        }
        if requested_before {
            pause();
        }
        requested_before = true;

        let url = season.url(league);
        let response = get(&url)?;
        match response.status {
            200 => {
                write_atomically(&path, &response.body)?;
                report.downloaded += 1;
            }
            404 if *season == current => report.not_published.push(url),
            429 => {
                return Err(Error::Http(
                    "football-data.co.uk a répondu 429 (trop de requêtes) : réessaie plus tard ; \
                     les fichiers déjà téléchargés restent en cache"
                        .to_string(),
                ));
            }
            status => return Err(Error::Http(format!("{url} : HTTP {status}"))),
        }
    }
    Ok(report)
}

/// Un fichier est « frais » s'il existe et, pour la saison en cours, a moins de 24 h.
fn is_fresh(path: &Path, is_current_season: bool, now: SystemTime) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !is_current_season {
        return true;
    }
    metadata
        .modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age < REFRESH_AFTER)
}

/// Écrit dans un fichier temporaire puis le renomme : un téléchargement interrompu ne laisse jamais
/// un fichier tronqué que l'on prendrait ensuite pour un fichier complet.
fn write_atomically(path: &Path, body: &[u8]) -> Result<()> {
    let temporary = path.with_extension("csv.part");
    fs::write(&temporary, body).map_err(Error::io(&temporary))?;
    fs::rename(&temporary, path).map_err(Error::io(path))
}

/// Client HTTP pour les fichiers de football-data (`ureq`, sans `async`).
pub struct FileClient {
    agent: ureq::Agent,
}

impl FileClient {
    /// Crée le client. Le `User-Agent` indique l'outil et ne contient aucune donnée personnelle.
    pub fn new() -> FileClient {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .https_only(true)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent("edge/0.1 (usage personnel, telechargement occasionnel)")
            .build();
        FileClient {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// `GET` d'une adresse complète.
    pub fn get(&self, url: &str) -> Result<HttpBytes> {
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|e| Error::Http(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| Error::Http(e.to_string()))?;
        Ok(HttpBytes { status, body })
    }
}

impl Default for FileClient {
    fn default() -> FileClient {
        FileClient::new()
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;
    use crate::ingest::football_data::LEAGUES;

    fn wanted() -> Vec<(League, Season)> {
        vec![
            (LEAGUES[0], Season::new(2024)),
            (LEAGUES[0], Season::new(2025)),
            (LEAGUES[0], Season::new(2026)),
        ]
    }

    fn ok(body: &str) -> Result<HttpBytes> {
        Ok(HttpBytes {
            status: 200,
            body: body.as_bytes().to_vec(),
        })
    }

    fn status(code: u16) -> Result<HttpBytes> {
        Ok(HttpBytes {
            status: code,
            body: Vec::new(),
        })
    }

    #[test]
    fn missing_files_are_downloaded_with_a_pause_between_requests() {
        let dir = tempfile::tempdir().unwrap();
        let mut urls = Vec::new();
        let mut pauses = 0;

        let report = fetch_all(
            dir.path(),
            &wanted(),
            Season::new(2026),
            false,
            SystemTime::now(),
            &mut |url| {
                urls.push(url.to_string());
                ok("Div\n")
            },
            &mut || pauses += 1,
        )
        .unwrap();

        assert_eq!(report.downloaded, 3);
        assert_eq!(pauses, 2, "une pause avant chaque requête sauf la première");
        assert_eq!(urls[0], "https://football-data.co.uk/mmz4281/2425/E0.csv");
        assert!(dir.path().join("E0_2526.csv").exists());
        assert!(!dir.path().join("E0_2526.csv.part").exists());
    }

    #[test]
    fn past_seasons_in_cache_are_never_downloaded_again() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("E0_2425.csv"), "ancien").unwrap();
        fs::write(dir.path().join("E0_2526.csv"), "ancien").unwrap();
        let mut calls = 0;

        let report = fetch_all(
            dir.path(),
            &wanted(),
            Season::new(2026),
            false,
            SystemTime::now(),
            &mut |_| {
                calls += 1;
                ok("neuf")
            },
            &mut || {},
        )
        .unwrap();

        assert_eq!(calls, 1, "seule la saison en cours manquait");
        assert_eq!(report.cached, 2);
        assert_eq!(
            fs::read_to_string(dir.path().join("E0_2425.csv")).unwrap(),
            "ancien"
        );
    }

    #[test]
    fn the_current_season_is_refreshed_only_after_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("E0_2627.csv");
        fs::write(&path, "ancien").unwrap();
        let only_current = [(LEAGUES[0], Season::new(2026))];
        let now = SystemTime::now();

        // Fichier récent : on ne retélécharge pas.
        let mut calls = 0;
        fetch_all(
            dir.path(),
            &only_current,
            Season::new(2026),
            false,
            now,
            &mut |_| {
                calls += 1;
                ok("neuf")
            },
            &mut || {},
        )
        .unwrap();
        assert_eq!(calls, 0);

        // Fichier vieux de 25 h : on le rafraîchit.
        let file = File::options().write(true).open(&path).unwrap();
        file.set_modified(now - Duration::from_secs(25 * 3600))
            .unwrap();
        fetch_all(
            dir.path(),
            &only_current,
            Season::new(2026),
            false,
            now,
            &mut |_| {
                calls += 1;
                ok("neuf")
            },
            &mut || {},
        )
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), "neuf");
    }

    #[test]
    fn force_downloads_everything_again() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("E0_2425.csv"), "ancien").unwrap();
        let mut calls = 0;

        fetch_all(
            dir.path(),
            &wanted(),
            Season::new(2026),
            true,
            SystemTime::now(),
            &mut |_| {
                calls += 1;
                ok("neuf")
            },
            &mut || {},
        )
        .unwrap();

        assert_eq!(calls, 3);
        assert_eq!(
            fs::read_to_string(dir.path().join("E0_2425.csv")).unwrap(),
            "neuf"
        );
    }

    #[test]
    fn a_429_stops_the_run_with_a_clear_message_and_keeps_earlier_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut count = 0;

        let err = fetch_all(
            dir.path(),
            &wanted(),
            Season::new(2026),
            false,
            SystemTime::now(),
            &mut |_| {
                count += 1;
                if count == 2 { status(429) } else { ok("ok") }
            },
            &mut || {},
        )
        .unwrap_err();

        assert!(err.to_string().contains("429"));
        assert!(err.to_string().contains("réessaie plus tard"));
        assert!(
            dir.path().join("E0_2425.csv").exists(),
            "le premier fichier reste en cache"
        );
        assert!(!dir.path().join("E0_2526.csv").exists());
    }

    #[test]
    fn a_404_is_tolerated_for_the_current_season_only() {
        let dir = tempfile::tempdir().unwrap();
        let only_current = [(LEAGUES[0], Season::new(2026))];
        let report = fetch_all(
            dir.path(),
            &only_current,
            Season::new(2026),
            false,
            SystemTime::now(),
            &mut |_| status(404),
            &mut || {},
        )
        .unwrap();
        assert_eq!(report.not_published.len(), 1);

        let past = [(LEAGUES[0], Season::new(2020))];
        let err = fetch_all(
            dir.path(),
            &past,
            Season::new(2026),
            false,
            SystemTime::now(),
            &mut |_| status(404),
            &mut || {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("404"));
    }

    #[test]
    fn cache_paths_follow_the_div_and_season_code() {
        assert_eq!(
            cache_path(Path::new("/c"), &LEAGUES[5], Season::new(2018)),
            Path::new("/c/SC0_1819.csv")
        );
    }
}
