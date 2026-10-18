-- Archive des cotes. Ces trois tables sont en ajout seul : chaque relevé s'ajoute, rien n'est jamais
-- modifié ni supprimé (les triggers en bas du fichier l'imposent). Les noms d'équipes, de bookmakers et
-- d'issues sont conservés tels que l'API les renvoie ; la correspondance entre sources viendra en phase 2.

-- Un match vu dans une réponse /odds. Son heure de début peut changer d'un relevé à l'autre (report),
-- d'où une ligne par relevé plutôt qu'une table de matchs unique.
CREATE TABLE event_seen (
    api_call_id   INTEGER NOT NULL REFERENCES api_call (id),
    event_id      TEXT    NOT NULL,                -- identifiant The Odds API
    sport_key     TEXT    NOT NULL,                -- 'soccer_epl'…
    commence_time TEXT    NOT NULL,                -- UTC, RFC 3339
    home_team_raw TEXT    NOT NULL,
    away_team_raw TEXT    NOT NULL,
    PRIMARY KEY (api_call_id, event_id)
) STRICT;

-- Une cote : relevé × match × bookmaker × marché × issue.
CREATE TABLE quote (
    api_call_id      INTEGER NOT NULL,
    event_id         TEXT    NOT NULL,
    bookmaker_key    TEXT    NOT NULL,             -- 'winamax_fr'
    market_key       TEXT    NOT NULL,             -- 'h2h'
    quote_updated_at TEXT    NOT NULL,             -- last_update du MARCHÉ (celui du bookmaker est déprécié)
    outcome_raw      TEXT    NOT NULL,             -- nom brut de l'issue : une équipe ou 'Draw'
    point            REAL,                         -- NULL en 1X2 ; servira aux totaux et aux écarts
    price            REAL    NOT NULL CHECK (price > 1.0),
    FOREIGN KEY (api_call_id, event_id) REFERENCES event_seen (api_call_id, event_id)
) STRICT;

CREATE INDEX quote_by_event ON quote (event_id, bookmaker_key, api_call_id);

-- Immuabilité : toute modification ou suppression est refusée, y compris celle du journal des appels.
CREATE TRIGGER api_call_no_update BEFORE UPDATE ON api_call
BEGIN SELECT RAISE(ABORT, 'archive immuable : api_call ne se modifie pas'); END;
CREATE TRIGGER api_call_no_delete BEFORE DELETE ON api_call
BEGIN SELECT RAISE(ABORT, 'archive immuable : api_call ne se supprime pas'); END;
CREATE TRIGGER event_seen_no_update BEFORE UPDATE ON event_seen
BEGIN SELECT RAISE(ABORT, 'archive immuable : event_seen ne se modifie pas'); END;
CREATE TRIGGER event_seen_no_delete BEFORE DELETE ON event_seen
BEGIN SELECT RAISE(ABORT, 'archive immuable : event_seen ne se supprime pas'); END;
CREATE TRIGGER quote_no_update BEFORE UPDATE ON quote
BEGIN SELECT RAISE(ABORT, 'archive immuable : quote ne se modifie pas'); END;
CREATE TRIGGER quote_no_delete BEFORE DELETE ON quote
BEGIN SELECT RAISE(ABORT, 'archive immuable : quote ne se supprime pas'); END;
