-- Phase 8 : NBA. Les résultats viennent de l'API de cotes (`/scores`) : aucune source gratuite et licite d'historique
-- n'a été trouvée (docs/sources.md §23). La table se met à jour (« upsert » sur l'identifiant du match) : l'API
-- redonne les matchs des trois derniers jours à chaque appel.

CREATE TABLE nba_game (
    event_id      TEXT    PRIMARY KEY,                -- même identifiant que dans l'archive des cotes (`event_seen`, `quote`)
    commence_time TEXT    NOT NULL,                   -- UTC, RFC 3339
    season        INTEGER NOT NULL,                   -- année de début de la saison : octobre 2026 → 2026, mars 2027 → 2026
    home_team     TEXT    NOT NULL,
    away_team     TEXT    NOT NULL,
    home_score    INTEGER NOT NULL CHECK (home_score >= 0),
    away_score    INTEGER NOT NULL CHECK (away_score >= 0),
    fetched_at    TEXT    NOT NULL
) STRICT;

CREATE INDEX nba_game_by_time ON nba_game (commence_time);
