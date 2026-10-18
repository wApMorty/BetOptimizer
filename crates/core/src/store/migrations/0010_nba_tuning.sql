-- Phase 8 : réglage gelé du modèle NBA. Une ligne par session de réglage, jamais modifiée ni effacée : chaque session
-- supplémentaire est une occasion de s'ajuster au hasard, le rapport les compte.

CREATE TABLE nba_tuning (
    id                  INTEGER PRIMARY KEY,
    created_at          TEXT    NOT NULL,
    k                   REAL    NOT NULL,
    home_advantage      REAL    NOT NULL,
    back_to_back        REAL    NOT NULL,
    carry_over          REAL    NOT NULL,
    totals_rate         REAL    NOT NULL,
    margin_per_elo      REAL    NOT NULL,        -- points de marge par point d'écart d'Elo (régression sur la validation)
    margin_sigma        REAL    NOT NULL,        -- écart-type de la marge autour de la prévision
    total_sigma         REAL    NOT NULL,        -- écart-type du total autour de la prévision
    validation_log_loss REAL    NOT NULL,
    validation_games    INTEGER NOT NULL,
    configs_tried       INTEGER NOT NULL
) STRICT;

CREATE TRIGGER nba_tuning_no_update BEFORE UPDATE ON nba_tuning
BEGIN SELECT RAISE(ABORT, 'journal immuable : nba_tuning ne se modifie pas'); END;
CREATE TRIGGER nba_tuning_no_delete BEFORE DELETE ON nba_tuning
BEGIN SELECT RAISE(ABORT, 'journal immuable : nba_tuning ne se supprime pas'); END;
