-- Phase 5 : recommandations, paris réels et règlements. Journal immuable (invariants 6 et 7 de CLAUDE.md) :
-- aucune ligne ne se modifie ni ne se supprime ; un règlement est une ligne de plus, dans une table à part.

-- Une recommandation, enregistrée à sa création avec tout ce qui la justifie.
-- `tier` : 'recommended' = notifiée, avantage ≥ seuil ; 'shadow' = journalisée en silence (avantage ≥ 0 sous le seuil)
-- pour mesurer le CLV réel par tranche d'avantage sans attendre des années.
CREATE TABLE recommendation (
    id             INTEGER PRIMARY KEY,
    created_at     TEXT    NOT NULL,                  -- UTC, RFC 3339
    rule           TEXT    NOT NULL,                  -- 'price_vs_pinnacle'
    tier           TEXT    NOT NULL CHECK (tier IN ('recommended', 'shadow')),
    event_id       TEXT    NOT NULL,                  -- identifiant du match chez The Odds API
    sport_key      TEXT    NOT NULL,
    home_team_raw  TEXT    NOT NULL,                  -- noms bruts de l'API
    away_team_raw  TEXT    NOT NULL,
    kickoff        TEXT    NOT NULL,                  -- UTC
    bookmaker_key  TEXT    NOT NULL,
    outcome        TEXT    NOT NULL CHECK (outcome IN ('H', 'D', 'A')),
    odds           REAL    NOT NULL CHECK (odds > 1.0),
    probability    REAL    NOT NULL,                  -- Pinnacle sans marge, au même relevé
    edge           REAL    NOT NULL,                  -- probability × odds − 1
    min_edge       REAL    NOT NULL,                  -- seuil appliqué ; la cote minimale en découle
    minimal_odds   REAL    NOT NULL,
    stake          REAL,                              -- NULL : journal fantôme, ou mise sous le minimum
    kelly_fraction REAL    NOT NULL,
    bankroll       REAL    NOT NULL,                  -- bankroll au moment du calcul
    margin_method  TEXT    NOT NULL,
    api_call_id    INTEGER NOT NULL REFERENCES api_call (id),   -- le relevé d'où viennent les cotes
    code_version   TEXT    NOT NULL,
    UNIQUE (rule, tier, event_id)                     -- une sélection par match et par palier : relancer ne duplique rien
) STRICT;

CREATE INDEX recommendation_by_kickoff ON recommendation (kickoff);

-- Un pari réellement placé par Paul. `recommendation_id` NULL = pari hors modèle (`off_model`), rapporté à part.
CREATE TABLE bet (
    id                INTEGER PRIMARY KEY,
    created_at        TEXT    NOT NULL,
    recommendation_id INTEGER REFERENCES recommendation (id),
    off_model         INTEGER NOT NULL CHECK (off_model IN (0, 1)),
    event_id          TEXT    NOT NULL,
    sport_key         TEXT    NOT NULL,
    home_team_raw     TEXT    NOT NULL,
    away_team_raw     TEXT    NOT NULL,
    kickoff           TEXT    NOT NULL,
    bookmaker_key     TEXT    NOT NULL,
    outcome           TEXT    NOT NULL CHECK (outcome IN ('H', 'D', 'A')),
    odds              REAL    NOT NULL CHECK (odds > 1.0),   -- la cote réellement obtenue
    stake             REAL    NOT NULL CHECK (stake > 0),
    CHECK ((recommendation_id IS NULL) = (off_model = 1))
) STRICT;

CREATE INDEX bet_by_event ON bet (event_id);

-- Le règlement d'un pari : au plus un par pari.
CREATE TABLE bet_result (
    bet_id     INTEGER PRIMARY KEY REFERENCES bet (id),
    settled_at TEXT    NOT NULL,
    result     TEXT    NOT NULL CHECK (result IN ('won', 'lost', 'void')),   -- 'void' : remboursé (match reporté ou annulé)
    source     TEXT    NOT NULL CHECK (source IN ('football-data', 'manual')),
    profit     REAL    NOT NULL                                              -- gain net : mise × (cote − 1), −mise ou 0
) STRICT;

CREATE TRIGGER recommendation_no_update BEFORE UPDATE ON recommendation
BEGIN SELECT RAISE(ABORT, 'journal immuable : recommendation ne se modifie pas'); END;
CREATE TRIGGER recommendation_no_delete BEFORE DELETE ON recommendation
BEGIN SELECT RAISE(ABORT, 'journal immuable : recommendation ne se supprime pas'); END;
CREATE TRIGGER bet_no_update BEFORE UPDATE ON bet
BEGIN SELECT RAISE(ABORT, 'journal immuable : bet ne se modifie pas'); END;
CREATE TRIGGER bet_no_delete BEFORE DELETE ON bet
BEGIN SELECT RAISE(ABORT, 'journal immuable : bet ne se supprime pas'); END;
CREATE TRIGGER bet_result_no_update BEFORE UPDATE ON bet_result
BEGIN SELECT RAISE(ABORT, 'journal immuable : bet_result ne se modifie pas'); END;
CREATE TRIGGER bet_result_no_delete BEFORE DELETE ON bet_result
BEGIN SELECT RAISE(ABORT, 'journal immuable : bet_result ne se supprime pas'); END;
