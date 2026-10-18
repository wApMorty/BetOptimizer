-- Backtests : seuils gelés, exécutions et paris. Journal immuable, comme les prédictions des modèles.

-- Le seuil d'avantage d'une source, choisi sur la validation (et l'entraînement quand il existe), puis gelé avant le test.
-- Toutes les sessions sont conservées : chaque réglage supplémentaire est une occasion de s'adapter au hasard.
CREATE TABLE backtest_selection (
    id             INTEGER PRIMARY KEY,
    created_at     TEXT    NOT NULL,                  -- UTC, RFC 3339
    source         TEXT    NOT NULL,                  -- 'soft:B365', 'model:dixon_coles'…
    threshold      REAL    NOT NULL,                  -- avantage minimal gelé
    selection_clv  REAL,                              -- CLV moyen sur la période de réglage (NULL sans clôture)
    selection_bets INTEGER NOT NULL,
    configs_tried  INTEGER NOT NULL                   -- sources × seuils essayés pendant la session
) STRICT;

-- Une exécution : une source, une période, un seuil, une fraction de Kelly, une décote.
CREATE TABLE backtest_run (
    id              INTEGER PRIMARY KEY,
    created_at      TEXT    NOT NULL,
    source          TEXT    NOT NULL,
    period          TEXT    NOT NULL CHECK (period IN ('train', 'validation', 'test')),
    threshold       REAL    NOT NULL,
    kelly_fraction  REAL    NOT NULL,
    price_factor    REAL    NOT NULL,                 -- 1 = cotes telles quelles ; <1 = décote
    bets            INTEGER NOT NULL,
    final_bankroll  REAL    NOT NULL,
    UNIQUE (source, period, threshold, kelly_fraction, price_factor)
) STRICT;

CREATE TABLE backtest_bet (
    run_id      INTEGER NOT NULL REFERENCES backtest_run (id),
    game_id     INTEGER NOT NULL REFERENCES game (id),
    batch_date  TEXT    NOT NULL,                     -- date de décision du lot
    outcome     TEXT    NOT NULL CHECK (outcome IN ('H', 'D', 'A')),
    odds        REAL    NOT NULL CHECK (odds > 1.0),
    probability REAL    NOT NULL,                     -- probabilité estimée à la décision
    close_fair  REAL,                                 -- probabilité de clôture sans marge (CLV uniquement)
    stake       REAL    NOT NULL,
    profit      REAL    NOT NULL,
    PRIMARY KEY (run_id, game_id)
) STRICT;

CREATE TRIGGER backtest_selection_no_update BEFORE UPDATE ON backtest_selection
BEGIN SELECT RAISE(ABORT, 'journal immuable : backtest_selection ne se modifie pas'); END;
CREATE TRIGGER backtest_selection_no_delete BEFORE DELETE ON backtest_selection
BEGIN SELECT RAISE(ABORT, 'journal immuable : backtest_selection ne se supprime pas'); END;
CREATE TRIGGER backtest_run_no_update BEFORE UPDATE ON backtest_run
BEGIN SELECT RAISE(ABORT, 'journal immuable : backtest_run ne se modifie pas'); END;
CREATE TRIGGER backtest_run_no_delete BEFORE DELETE ON backtest_run
BEGIN SELECT RAISE(ABORT, 'journal immuable : backtest_run ne se supprime pas'); END;
CREATE TRIGGER backtest_bet_no_update BEFORE UPDATE ON backtest_bet
BEGIN SELECT RAISE(ABORT, 'journal immuable : backtest_bet ne se modifie pas'); END;
CREATE TRIGGER backtest_bet_no_delete BEFORE DELETE ON backtest_bet
BEGIN SELECT RAISE(ABORT, 'journal immuable : backtest_bet ne se supprime pas'); END;
