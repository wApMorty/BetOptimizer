-- Prédictions des modèles (walk-forward) et sessions de réglage des hyperparamètres.
-- Journal immuable (invariant 6) : une prédiction n'est jamais modifiée ni supprimée. Le modèle est déterministe :
-- si le code d'un modèle change, on change sa version (`models::walk_forward::CODE_VERSION`) et l'on crée de nouvelles
-- exécutions ; les anciennes restent consultables.

-- Une exécution = un modèle et ses hyperparamètres, sur une période.
CREATE TABLE model_run (
    id                          INTEGER PRIMARY KEY,
    model                       TEXT    NOT NULL,                -- 'poisson' | 'dixon_coles'
    version                     TEXT    NOT NULL,                -- famille, hyperparamètres et version du code
    window_start                TEXT    NOT NULL,                -- première date évaluée (incluse)
    window_end                  TEXT    NOT NULL,                -- dernière date évaluée (incluse)
    created_at                  TEXT    NOT NULL,                -- UTC, RFC 3339
    skipped_for_lack_of_history INTEGER NOT NULL,                -- matchs non prédits : historique insuffisant
    skipped_invalid             INTEGER NOT NULL,                -- matchs non prédits : loi de probabilité invalide
    rho_clamped                 INTEGER NOT NULL,                -- prédictions où ρ a été ramené dans le domaine faisable
    UNIQUE (version, window_start, window_end)
) STRICT;

CREATE TABLE prediction (
    run_id    INTEGER NOT NULL REFERENCES model_run (id),
    game_id   INTEGER NOT NULL REFERENCES game (id),
    as_of     TEXT    NOT NULL,                                  -- lundi d'ajustement : seuls des matchs antérieurs ont servi
    home_mean REAL    NOT NULL,                                  -- buts attendus à domicile
    away_mean REAL    NOT NULL,
    p_home    REAL    NOT NULL,
    p_draw    REAL    NOT NULL,
    p_away    REAL    NOT NULL,
    p_over25  REAL    NOT NULL,
    p_btts    REAL    NOT NULL,
    PRIMARY KEY (run_id, game_id)
) STRICT;

-- Une session de réglage : la meilleure configuration d'un modèle sur la validation, « gelée » avant le test.
-- Garder toutes les sessions permet de rapporter honnêtement combien de fois on a réglé (invariant 8).
CREATE TABLE tuning_session (
    id                  INTEGER PRIMARY KEY,
    created_at          TEXT    NOT NULL,
    window_start        TEXT    NOT NULL,
    window_end          TEXT    NOT NULL,
    configs_tried       INTEGER NOT NULL,                        -- configurations essayées, tous modèles confondus
    model               TEXT    NOT NULL,
    decay               REAL    NOT NULL,
    ridge               REAL    NOT NULL,
    blend               REAL    NOT NULL,
    validation_log_loss REAL    NOT NULL
) STRICT;

CREATE TRIGGER model_run_no_update BEFORE UPDATE ON model_run
BEGIN SELECT RAISE(ABORT, 'journal immuable : model_run ne se modifie pas'); END;
CREATE TRIGGER model_run_no_delete BEFORE DELETE ON model_run
BEGIN SELECT RAISE(ABORT, 'journal immuable : model_run ne se supprime pas'); END;
CREATE TRIGGER prediction_no_update BEFORE UPDATE ON prediction
BEGIN SELECT RAISE(ABORT, 'journal immuable : prediction ne se modifie pas'); END;
CREATE TRIGGER prediction_no_delete BEFORE DELETE ON prediction
BEGIN SELECT RAISE(ABORT, 'journal immuable : prediction ne se supprime pas'); END;
CREATE TRIGGER tuning_session_no_update BEFORE UPDATE ON tuning_session
BEGIN SELECT RAISE(ABORT, 'journal immuable : tuning_session ne se modifie pas'); END;
CREATE TRIGGER tuning_session_no_delete BEFORE DELETE ON tuning_session
BEGIN SELECT RAISE(ABORT, 'journal immuable : tuning_session ne se supprime pas'); END;
