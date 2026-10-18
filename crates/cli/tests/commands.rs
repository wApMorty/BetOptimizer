//! Tests d'intégration de `edge quota` et `edge snapshot`, sans réseau : on n'y fournit jamais de clé,
//! sauf pour `--dry-run` qui n'en a pas besoin.

mod common;

use common::{run_edge, text};

#[test]
fn dry_run_shows_the_cost_and_needs_no_key() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["snapshot", "--dry-run"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.contains("Aucun appel n'est effectué"));
    assert!(stdout.contains("6 ligue(s) × 1 crédit(s) = 6 crédit(s)"));
    assert!(stdout.contains("soccer_spl"));
    assert!(stdout.contains("Quota encore inconnu"));
}

#[test]
fn snapshot_without_a_key_fails_before_touching_the_database() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["snapshot"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("EDGE_ODDS_API_KEY"));
    assert!(!home.path().join("edge.db").exists());
}

#[test]
fn quota_without_a_key_fails_clearly() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["quota"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("clé API introuvable"));
}

#[test]
fn fd_import_without_downloaded_files_points_to_fd_fetch() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["fd", "import"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("edge fd fetch"));
}

#[test]
fn market_report_on_an_empty_database_explains_what_to_do() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["market", "report"], None);

    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("edge fd fetch") && stderr.contains("edge fd import"));
}

#[test]
fn fd_check_on_an_empty_database_has_nothing_to_report() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["fd", "check"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("0 ligue-saisons"));
}

/// Parcours complet hors réseau : un CSV réel déposé dans le cache, importé, contrôlé, puis analysé.
#[test]
fn a_cached_csv_goes_through_import_check_and_report() {
    let home = tempfile::tempdir().unwrap();
    let cache = home.path().join("football-data");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("E0_2526.csv"),
        include_bytes!("../../core/tests/fixtures/fd/E0_2526_sample.csv"),
    )
    .unwrap();

    let import = run_edge(home.path(), &["fd", "import"], None);
    assert!(import.status.success(), "{}", text(&import.stderr));
    assert!(text(&import.stdout).contains("1 fichiers importés : 5 matchs (5 nouveaux)"));

    // Relancer ne duplique rien.
    let again = run_edge(home.path(), &["fd", "import"], None);
    assert!(text(&again.stdout).contains("5 matchs (0 nouveaux)"));

    let check = run_edge(home.path(), &["fd", "check"], None);
    assert!(check.status.success(), "{}", text(&check.stderr));
    assert!(text(&check.stdout).contains("Résultats publiés contredisant les buts : 0"));

    let report_path = home.path().join("rapport.html");
    let report = run_edge(
        home.path(),
        &["market", "report", "--out", report_path.to_str().unwrap()],
        None,
    );
    assert!(report.status.success(), "{}", text(&report.stderr));
    let html = std::fs::read_to_string(&report_path).unwrap();
    assert!(html.contains("Le marché comme pronostiqueur"));
    assert!(html.contains("<svg"));
}

#[test]
fn model_evaluate_without_a_frozen_tuning_points_to_model_tune() {
    let home = tempfile::tempdir().unwrap();
    let cache = home.path().join("football-data");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("E0_2526.csv"),
        include_bytes!("../../core/tests/fixtures/fd/E0_2526_sample.csv"),
    )
    .unwrap();
    assert!(
        run_edge(home.path(), &["fd", "import"], None)
            .status
            .success()
    );

    let output = run_edge(home.path(), &["model", "evaluate"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("edge model tune"));
}

#[test]
fn model_report_on_an_empty_database_explains_what_to_do() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["model", "report"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("edge fd fetch"));
}

#[test]
fn backtest_report_without_a_frozen_threshold_points_to_backtest_tune() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["backtest", "report"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("edge backtest tune"));
}

#[test]
fn value_on_an_empty_archive_asks_for_a_snapshot() {
    let home = tempfile::tempdir().unwrap();
    assert!(run_edge(home.path(), &["init"], None).status.success());

    let output = run_edge(home.path(), &["value"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("edge snapshot"));
}

#[test]
fn value_refuses_a_kelly_fraction_above_one() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["value", "--kelly", "2"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("--kelly"));
}

/// Données de test écrites directement dans la base (`unwrap` toléré dans un module de test).
#[cfg(test)]
mod archive {
    use super::run_edge;

    /// Écrit dans la base du dossier un relevé récent où Pinnacle donne 2,00 à la victoire à domicile et Betclic 2,30 :
    /// un avantage d'environ 12 % sur cette issue, pour un match qui commence dans `kickoff_in_minutes` minutes.
    pub fn one_value_match(home: &std::path::Path, kickoff_in_minutes: i64) {
        use chrono::{Duration, Utc};
        use edge_core::domain::{Event, Odds, Quota, Quote};
        use edge_core::store::archive::{Endpoint, NewApiCall, record_api_call, record_snapshot};
        use edge_core::store::{migrate, open};

        assert!(run_edge(home, &["init"], None).status.success());
        let mut conn = open(&home.join("edge.db")).unwrap();
        migrate::migrate(&mut conn).unwrap();

        let called_at = Utc::now() - Duration::hours(1);
        let mut quotes = Vec::new();
        for (bookmaker, home_odds, draw_odds, away_odds) in
            [("pinnacle", 2.0, 3.6, 4.0), ("betclic_fr", 2.3, 3.2, 3.4)]
        {
            for (name, price) in [
                ("Falkirk", home_odds),
                ("Draw", draw_odds),
                ("Kilmarnock", away_odds),
            ] {
                quotes.push(Quote {
                    bookmaker_key: bookmaker.to_string(),
                    market_key: "h2h".to_string(),
                    updated_at: called_at,
                    outcome: name.to_string(),
                    point: None,
                    price: Odds::new(price).unwrap(),
                });
            }
        }
        let event = Event {
            id: "match-cli".to_string(),
            sport_key: "soccer_spl".to_string(),
            commence_time: Utc::now() + Duration::minutes(kickoff_in_minutes),
            home_team: "Falkirk".to_string(),
            away_team: "Kilmarnock".to_string(),
            quotes,
        };
        let call = NewApiCall {
            called_at,
            endpoint: Endpoint::Odds,
            sport_key: Some("soccer_spl"),
            params: "markets=h2h",
            http_status: 200,
            quota: Quota::default(),
            body: None,
        };
        let id = record_api_call(&conn, &call).unwrap();
        record_snapshot(&mut conn, id, &[event]).unwrap();
    }
}

#[test]
fn recommend_writes_the_journal_once_and_a_second_run_adds_nothing() {
    let home = tempfile::tempdir().unwrap();
    archive::one_value_match(home.path(), 2 * 24 * 60);

    let first = run_edge(home.path(), &["recommend"], None);
    let second = run_edge(home.path(), &["recommend"], None);

    assert!(first.status.success(), "{}", text(&first.stderr));
    let first_out = text(&first.stdout);
    assert!(
        first_out.contains("1 recommandation(s) nouvelle(s)"),
        "{first_out}"
    );
    assert!(
        first_out.contains("betclic_fr · domicile à 2,30"),
        "{first_out}"
    );
    assert!(first_out.contains("recommandation n° 1"));
    let second_out = text(&second.stdout);
    assert!(
        second_out.contains("0 recommandation(s) nouvelle(s)"),
        "{second_out}"
    );
    assert!(second_out.contains("1 déjà connue(s)"), "{second_out}");
}

#[test]
fn recommend_with_notify_but_no_webhook_fails_before_writing_anything() {
    let home = tempfile::tempdir().unwrap();
    archive::one_value_match(home.path(), 2 * 24 * 60);

    let refused = run_edge(home.path(), &["recommend", "--notify"], None);
    let plain = run_edge(home.path(), &["recommend"], None);

    assert!(!refused.status.success());
    assert!(text(&refused.stderr).contains("EDGE_DISCORD_WEBHOOK"));
    // La tentative refusée n'a rien écrit : la commande suivante crée bien la première recommandation.
    assert!(text(&plain.stdout).contains("1 recommandation(s) nouvelle(s)"));
}

#[test]
fn value_shows_the_pick_without_writing_to_the_journal() {
    let home = tempfile::tempdir().unwrap();
    archive::one_value_match(home.path(), 2 * 24 * 60);

    let shown = run_edge(home.path(), &["value"], None);
    let recommended = run_edge(home.path(), &["recommend"], None);

    assert!(shown.status.success(), "{}", text(&shown.stderr));
    assert!(text(&shown.stdout).contains("betclic_fr · domicile à 2,30"));
    assert!(text(&shown.stdout).contains("match match-cli"));
    assert!(text(&recommended.stdout).contains("1 recommandation(s) nouvelle(s)"));
}

#[test]
fn closing_with_nothing_to_capture_needs_no_key_and_says_nothing_to_a_journal() {
    let home = tempfile::tempdir().unwrap();
    assert!(run_edge(home.path(), &["init"], None).status.success());

    let output = run_edge(home.path(), &["closing"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    // La sortie n'est pas un terminal (elle est capturée) : la commande reste silencieuse, comme sous systemd.
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn closing_dry_run_lists_the_followed_match_starting_soon_without_calling_the_api() {
    let home = tempfile::tempdir().unwrap();
    archive::one_value_match(home.path(), 20);
    assert!(run_edge(home.path(), &["recommend"], None).status.success());

    let dry = run_edge(home.path(), &["closing", "--dry-run"], None);
    let real = run_edge(home.path(), &["closing"], None);

    assert!(dry.status.success(), "{}", text(&dry.stderr));
    assert!(text(&dry.stdout).contains("match-cli"));
    assert!(text(&dry.stdout).contains("Aucun appel n'est effectué"));
    // Sans clé, l'appel réel échoue clairement, avant tout réseau.
    assert!(!real.status.success());
    assert!(text(&real.stderr).contains("EDGE_ODDS_API_KEY"));
}

#[test]
fn a_followed_bet_updates_the_bankroll_and_cannot_be_logged_twice() {
    let home = tempfile::tempdir().unwrap();
    archive::one_value_match(home.path(), 2 * 24 * 60);
    assert!(run_edge(home.path(), &["recommend"], None).status.success());

    let logged = run_edge(home.path(), &["bet", "log", "--rec", "1"], None);
    let again = run_edge(home.path(), &["bet", "log", "--rec", "1"], None);
    let list = run_edge(home.path(), &["bet", "list"], None);

    assert!(logged.status.success(), "{}", text(&logged.stderr));
    assert!(text(&logged.stdout).contains("Pari n° 1 enregistré"));
    assert!(!again.status.success());
    assert!(text(&again.stderr).contains("déjà enregistré"));
    let listing = text(&list.stdout);
    assert!(listing.contains("ouvert"), "{listing}");
    assert!(listing.contains("recommandation n° 1"), "{listing}");
    // Kelly d'un pari seul (marge retirée par la méthode « power » de la configuration par défaut) : 99,0 € ;
    // la mise ouverte est déjà sortie de la bankroll de 1 000 €.
    assert!(listing.contains("mise 99,0 €"), "{listing}");
    assert!(
        listing.contains("Bankroll courante : 901,00 €"),
        "{listing}"
    );
}

#[test]
fn a_bet_below_the_minimal_odds_is_refused_as_a_followed_bet_and_goes_off_model() {
    let home = tempfile::tempdir().unwrap();
    archive::one_value_match(home.path(), 2 * 24 * 60);
    assert!(run_edge(home.path(), &["recommend"], None).status.success());

    let refused = run_edge(
        home.path(),
        &["bet", "log", "--rec", "1", "--odds", "1.9"],
        None,
    );
    let off_model = run_edge(
        home.path(),
        &[
            "bet",
            "log",
            "--event",
            "match-cli",
            "--bookmaker",
            "winamax_fr",
            "--outcome",
            "D",
            "--odds",
            "3.4",
            "--stake",
            "5",
        ],
        None,
    );

    assert!(!refused.status.success());
    assert!(text(&refused.stderr).contains("cote minimale"));
    assert!(off_model.status.success(), "{}", text(&off_model.stderr));
    assert!(text(&off_model.stdout).contains("(hors modèle)"));
}

#[test]
fn a_manual_settlement_is_written_once_and_the_automatic_pass_leaves_unknown_results_open() {
    let home = tempfile::tempdir().unwrap();
    archive::one_value_match(home.path(), 2 * 24 * 60);
    assert!(run_edge(home.path(), &["recommend"], None).status.success());
    assert!(
        run_edge(home.path(), &["bet", "log", "--rec", "1"], None)
            .status
            .success()
    );

    let automatic = run_edge(home.path(), &["bet", "settle"], None);
    let manual = run_edge(
        home.path(),
        &["bet", "settle", "--id", "1", "--result", "won"],
        None,
    );
    let twice = run_edge(
        home.path(),
        &["bet", "settle", "--id", "1", "--result", "lost"],
        None,
    );
    let list = run_edge(home.path(), &["bet", "list"], None);

    assert!(
        text(&automatic.stdout)
            .contains("0 pari(s) réglé(s) depuis l'historique ; 1 encore ouvert(s)")
    );
    assert!(manual.status.success(), "{}", text(&manual.stderr));
    assert!(!twice.status.success());
    assert!(text(&twice.stderr).contains("déjà réglé"));
    assert!(text(&list.stdout).contains("gagné +"));
}

#[test]
fn bet_log_needs_a_target() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["bet", "log"], None);

    assert!(!output.status.success());
}

#[test]
fn clv_on_a_fresh_journal_prints_the_empty_groups() {
    let home = tempfile::tempdir().unwrap();
    archive::one_value_match(home.path(), 2 * 24 * 60);
    assert!(run_edge(home.path(), &["recommend"], None).status.success());

    let output = run_edge(home.path(), &["clv"], None);

    assert!(output.status.success(), "{}", text(&output.stderr));
    let report = text(&output.stdout);
    assert!(
        report.contains("Recommandations notifiées : 1 (clôture connue : 0, réglés : 0)"),
        "{report}"
    );
    assert!(report.contains("Mes paris hors modèle : 0"), "{report}");
    assert!(report.contains("≥ 5 %"));
}

#[test]
fn notify_test_without_a_webhook_says_where_to_put_it() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["notify-test"], None);

    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("EDGE_DISCORD_WEBHOOK"));
}

#[test]
fn tennis_import_without_files_says_where_to_put_them() {
    let home = tempfile::tempdir().unwrap();

    let output = run_edge(home.path(), &["tennis", "import"], None);

    assert!(!output.status.success());
    let message = text(&output.stderr);
    assert!(message.contains("tennis-data.co.uk/data.php"), "{message}");
    assert!(message.contains("atp/"), "{message}");
}

#[test]
fn tennis_files_are_imported_once_checked_and_reimported_without_duplicates() {
    let home = tempfile::tempdir().unwrap();
    let atp = home.path().join("tennis-data").join("atp");
    std::fs::create_dir_all(&atp).unwrap();
    std::fs::write(
        atp.join("2024.xlsx"),
        include_bytes!("../../core/tests/fixtures/tennis/mini_atp_2024.xlsx"),
    )
    .unwrap();

    let first = run_edge(home.path(), &["tennis", "import"], None);
    let second = run_edge(home.path(), &["tennis", "import"], None);
    let check = run_edge(home.path(), &["tennis", "check"], None);

    assert!(first.status.success(), "{}", text(&first.stderr));
    let first_out = text(&first.stdout);
    assert!(
        first_out.contains("ATP 2024 : 6 match(s), 2 ligne(s) rejetée(s)"),
        "{first_out}"
    );
    assert!(first_out.contains("surface inconnue"), "{first_out}");
    assert!(text(&second.stdout).contains("6 match(s) écrits ou mis à jour"));
    let report = text(&check.stdout);
    assert!(check.status.success(), "{}", text(&check.stderr));
    assert!(report.contains("ATP"), "{report}");
    assert!(report.contains("Années sans aucun match"), "{report}");
}

#[test]
fn tennis_tune_and_report_say_what_is_missing() {
    let home = tempfile::tempdir().unwrap();

    let no_data = run_edge(home.path(), &["tennis", "tune"], None);
    assert!(!no_data.status.success());
    assert!(text(&no_data.stderr).contains("edge tennis import"));

    let atp = home.path().join("tennis-data").join("atp");
    std::fs::create_dir_all(&atp).unwrap();
    std::fs::write(
        atp.join("2024.xlsx"),
        include_bytes!("../../core/tests/fixtures/tennis/mini_atp_2024.xlsx"),
    )
    .unwrap();
    assert!(
        run_edge(home.path(), &["tennis", "import"], None)
            .status
            .success()
    );

    // Les six matchs de la fixture sont de 2024 : aucun ne tombe dans la validation 2022-2023.
    let no_validation = run_edge(home.path(), &["tennis", "tune"], None);
    assert!(!no_validation.status.success());
    assert!(text(&no_validation.stderr).contains("validation"));

    let no_freeze = run_edge(home.path(), &["tennis", "report"], None);
    assert!(!no_freeze.status.success());
    assert!(text(&no_freeze.stderr).contains("edge tennis tune"));
}
