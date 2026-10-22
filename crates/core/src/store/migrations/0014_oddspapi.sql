-- Phase 13 : archive de l'historique OddsPapi (docs/sources.md §34). Immuable, comme l'archive de The Odds API : une
-- ligne ne se modifie ni ne se supprime. Seul l'avant-match est conservé.

-- Journal des appels. Les paramètres sont enregistrés SANS la clé ; le corps de `account` n'est jamais enregistré (il
-- contient la clé et l'adresse électronique du compte).
CREATE TABLE oddspapi_call (
    id            INTEGER PRIMARY KEY,
    called_at     TEXT    NOT NULL,                  -- UTC, RFC 3339
    endpoint      TEXT    NOT NULL CHECK (endpoint IN ('account', 'fixtures', 'historical-odds')),
    params        TEXT    NOT NULL,
    http_status   INTEGER NOT NULL,
    request_count INTEGER                            -- consommation lue sur `account` ; NULL pour les autres appels
) STRICT;

-- Un match dont l'historique a été demandé. `history = 'missing'` : OddsPapi n'en a pas (HTTP 400 ou 404) ; on ne le
-- redemande pas.
CREATE TABLE oddspapi_fixture (
    fixture_id    TEXT    PRIMARY KEY,               -- 'id1000001761301153'
    tournament_id INTEGER NOT NULL,                  -- 17 : Premier League…
    kickoff       TEXT    NOT NULL,                  -- UTC
    home_raw      TEXT    NOT NULL,                  -- noms bruts d'OddsPapi
    away_raw      TEXT    NOT NULL,
    bookmakers    TEXT    NOT NULL,                  -- bookmakers demandés : 'pinnacle,winamax.fr,unibet.fr'
    fetched_at    TEXT    NOT NULL,
    history       TEXT    NOT NULL CHECK (history IN ('ok', 'missing')),
    discarded     INTEGER NOT NULL CHECK (discarded >= 0)   -- entrées actives sans cote valide, écartées
) STRICT;

-- Un changement de prix : la cote vaut jusqu'à l'entrée suivante de la même issue chez le même bookmaker.
CREATE TABLE oddspapi_price (
    fixture_id TEXT    NOT NULL REFERENCES oddspapi_fixture (fixture_id),
    bookmaker  TEXT    NOT NULL,                     -- 'pinnacle', 'winamax.fr', 'unibet.fr'
    market_id  INTEGER NOT NULL,                     -- 101 : 1X2 ; 1010 : plus/moins 2,5 ; 104 : BTTS
    outcome_id INTEGER NOT NULL,                     -- 1X2 : 101 domicile, 102 nul, 103 extérieur
    changed_at TEXT    NOT NULL,                     -- UTC, instant où OddsPapi a vu le changement
    price      REAL    CHECK (price IS NULL OR price > 1.0),
    active     INTEGER NOT NULL CHECK (active IN (0, 1)),
    CHECK (active = 0 OR price IS NOT NULL)          -- une cote active a toujours un prix
) STRICT;

CREATE INDEX oddspapi_price_by_fixture ON oddspapi_price (fixture_id, bookmaker, market_id, outcome_id, changed_at);

CREATE TRIGGER oddspapi_call_no_update BEFORE UPDATE ON oddspapi_call
BEGIN SELECT RAISE(ABORT, 'archive immuable : oddspapi_call ne se modifie pas'); END;
CREATE TRIGGER oddspapi_call_no_delete BEFORE DELETE ON oddspapi_call
BEGIN SELECT RAISE(ABORT, 'archive immuable : oddspapi_call ne se supprime pas'); END;
CREATE TRIGGER oddspapi_fixture_no_update BEFORE UPDATE ON oddspapi_fixture
BEGIN SELECT RAISE(ABORT, 'archive immuable : oddspapi_fixture ne se modifie pas'); END;
CREATE TRIGGER oddspapi_fixture_no_delete BEFORE DELETE ON oddspapi_fixture
BEGIN SELECT RAISE(ABORT, 'archive immuable : oddspapi_fixture ne se supprime pas'); END;
CREATE TRIGGER oddspapi_price_no_update BEFORE UPDATE ON oddspapi_price
BEGIN SELECT RAISE(ABORT, 'archive immuable : oddspapi_price ne se modifie pas'); END;
CREATE TRIGGER oddspapi_price_no_delete BEFORE DELETE ON oddspapi_price
BEGIN SELECT RAISE(ABORT, 'archive immuable : oddspapi_price ne se supprime pas'); END;
