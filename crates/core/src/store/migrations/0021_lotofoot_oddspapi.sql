-- Loto Foot : cotes Pinnacle relevées chez OddsPapi quand notre archive The Odds API n'a pas le match (docs/sources.md §62).
-- Une ligne par relevé : le dernier prix Pinnacle 1X2 connu au moment du relevé, tiré de `historical-odds`. Immuable.
-- Table distincte de `oddspapi_fixture` : un match à venir écrit là passerait pour « déjà téléchargé » et le relevé
-- hebdomadaire du test de latence ne prendrait jamais son historique complet.
CREATE TABLE lotofoot_oddspapi_quote (
    id             INTEGER PRIMARY KEY,
    call_id        INTEGER NOT NULL REFERENCES oddspapi_call (id),   -- appel `historical-odds`
    fixture_id     TEXT    NOT NULL,                                 -- 'id1000032772708164'
    tournament_id  INTEGER NOT NULL,                                 -- compétition OddsPapi (327 : Taça da Liga…)
    kickoff        TEXT    NOT NULL,                                 -- UTC
    home_raw       TEXT    NOT NULL,                                 -- noms bruts d'OddsPapi
    away_raw       TEXT    NOT NULL,
    home_team      TEXT    NOT NULL,                                 -- noms canoniques (section LOTOFOOT) de l'appariement
    away_team      TEXT    NOT NULL,
    price_home     REAL    NOT NULL CHECK (price_home > 1.0),
    price_draw     REAL    NOT NULL CHECK (price_draw > 1.0),
    price_away     REAL    NOT NULL CHECK (price_away > 1.0),
    last_change_at TEXT    NOT NULL,                                 -- dernier changement de prix Pinnacle avant le relevé
    fetched_at     TEXT    NOT NULL
) STRICT;

CREATE INDEX lotofoot_oddspapi_quote_by_teams ON lotofoot_oddspapi_quote (home_team, away_team, fetched_at);

CREATE TRIGGER lotofoot_oddspapi_quote_no_update BEFORE UPDATE ON lotofoot_oddspapi_quote
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_oddspapi_quote ne se modifie pas'); END;
CREATE TRIGGER lotofoot_oddspapi_quote_no_delete BEFORE DELETE ON lotofoot_oddspapi_quote
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_oddspapi_quote ne se supprime pas'); END;
