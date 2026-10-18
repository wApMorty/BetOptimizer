-- Journal des appels à l'API de cotes : un appel = une ligne, jamais modifiée.
-- Sert à la garde de budget (dernier `used` connu) et conserve le corps brut des réponses /odds :
-- le passé ne pourra pas être retéléchargé, ce corps permet de relire un relevé si le parseur évolue.
CREATE TABLE api_call (
    id          INTEGER PRIMARY KEY,
    called_at   TEXT    NOT NULL,                 -- UTC, RFC 3339 (« 2026-10-18T12:00:00Z »)
    endpoint    TEXT    NOT NULL CHECK (endpoint IN ('sports', 'events', 'odds')),
    sport_key   TEXT,                             -- NULL pour /sports
    params      TEXT    NOT NULL,                 -- paramètres envoyés, SANS la clé API
    http_status INTEGER NOT NULL,
    cost        INTEGER,                          -- en-tête x-requests-last
    used        INTEGER,                          -- en-tête x-requests-used
    remaining   INTEGER,                          -- en-tête x-requests-remaining
    body        TEXT                              -- corps brut de la réponse /odds
) STRICT;

-- Les en-têtes de quota sont absents des réponses 401 : cost, used et remaining sont donc facultatifs.
CREATE INDEX api_call_called_at ON api_call (called_at);
