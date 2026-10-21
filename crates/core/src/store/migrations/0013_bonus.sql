-- Phase 12 : bonus de bienvenue couverts (docs/bonus-bienvenue.md). Journal immuable, comme celui des paris : une
-- correction est une ligne de plus, jamais une mise à jour. Ces paris sont tenus à part de la table `bet` : un freebet ne
-- rend pas sa mise, et les bonus ne doivent pas se mêler à la bankroll ni aux métriques du modèle (invariant 7).

-- Ce que l'on sait de l'offre d'un opérateur. Plusieurs lignes par opérateur sont permises : la plus récente fait foi
-- (offre revue, erreur de saisie corrigée).
CREATE TABLE bonus_offer (
    id            INTEGER PRIMARY KEY,
    created_at    TEXT    NOT NULL,                  -- UTC, RFC 3339
    bookmaker_key TEXT    NOT NULL,                  -- 'betclic_fr', ou une clé choisie pour un opérateur absent de l'API
    status        TEXT    NOT NULL CHECK (status IN ('eligible', 'ineligible')),
    kind          TEXT    CHECK (kind IN ('cash_refund', 'freebet_refund', 'freebet')),
    amount        REAL    CHECK (amount > 0),        -- montant maximal de l'offre, en euros
    min_odds      REAL    CHECK (min_odds >= 1.0),   -- cote minimale imposée par l'offre ; NULL : aucune
    checked_on    TEXT    NOT NULL,                  -- date de la vérification d'éligibilité, 'YYYY-MM-DD'
    source        TEXT    NOT NULL,                  -- règlement lu, réponse du service client…
    note          TEXT,
    -- Une offre éligible a une forme et un montant ; une offre inéligible n'en a pas.
    CHECK ((status = 'eligible') = (kind IS NOT NULL AND amount IS NOT NULL)),
    CHECK (status = 'eligible' OR (kind IS NULL AND amount IS NULL AND min_odds IS NULL))
) STRICT;

CREATE INDEX bonus_offer_by_bookmaker ON bonus_offer (bookmaker_key, id);

-- Une opération : un pari bonus et ses couvertures, sur un même match, enregistrés ensemble.
-- `kind` peut différer de celui de l'offre : le freebet reçu après un premier pari perdu se joue en opération 'freebet'.
CREATE TABLE bonus_operation (
    id             INTEGER PRIMARY KEY,
    created_at     TEXT    NOT NULL,
    offer_id       INTEGER NOT NULL REFERENCES bonus_offer (id),
    kind           TEXT    NOT NULL CHECK (kind IN ('cash_refund', 'freebet_refund', 'freebet')),
    freebet_value  REAL    CHECK (freebet_value > 0 AND freebet_value < 1),   -- 'freebet_refund' seulement
    guaranteed     REAL    NOT NULL,                 -- gain garanti calculé à l'enregistrement, mises arrondies comprises
    event_id       TEXT    NOT NULL,                 -- identifiant du match chez The Odds API
    sport_key      TEXT    NOT NULL,
    home_team_raw  TEXT    NOT NULL,
    away_team_raw  TEXT    NOT NULL,
    kickoff        TEXT    NOT NULL,                 -- UTC
    CHECK ((kind = 'freebet_refund') = (freebet_value IS NOT NULL))
) STRICT;

-- Les paris d'une opération : exactement un pari 'bonus' (argent, ou freebet si l'opération est de forme 'freebet'),
-- et une couverture en argent par autre issue.
CREATE TABLE bonus_bet (
    id            INTEGER PRIMARY KEY,
    operation_id  INTEGER NOT NULL REFERENCES bonus_operation (id),
    role          TEXT    NOT NULL CHECK (role IN ('bonus', 'cover')),
    bookmaker_key TEXT    NOT NULL,
    outcome       TEXT    NOT NULL CHECK (outcome IN ('H', 'D', 'A')),
    odds          REAL    NOT NULL CHECK (odds > 1.0),   -- la cote réellement obtenue
    stake         REAL    NOT NULL CHECK (stake > 0),
    UNIQUE (operation_id, outcome)                       -- une seule mise par issue dans une opération
) STRICT;

CREATE UNIQUE INDEX bonus_bet_one_bonus ON bonus_bet (operation_id) WHERE role = 'bonus';

-- Le résultat du match d'une opération : il règle tous ses paris d'un coup. Au plus un par opération.
CREATE TABLE bonus_settlement (
    operation_id INTEGER PRIMARY KEY REFERENCES bonus_operation (id),
    settled_at   TEXT    NOT NULL,
    result       TEXT    NOT NULL CHECK (result IN ('H', 'D', 'A', 'void')),   -- 'void' : match reporté ou annulé
    source       TEXT    NOT NULL CHECK (source IN ('football-data', 'manual'))
) STRICT;

CREATE TRIGGER bonus_offer_no_update BEFORE UPDATE ON bonus_offer
BEGIN SELECT RAISE(ABORT, 'journal immuable : bonus_offer ne se modifie pas'); END;
CREATE TRIGGER bonus_offer_no_delete BEFORE DELETE ON bonus_offer
BEGIN SELECT RAISE(ABORT, 'journal immuable : bonus_offer ne se supprime pas'); END;
CREATE TRIGGER bonus_operation_no_update BEFORE UPDATE ON bonus_operation
BEGIN SELECT RAISE(ABORT, 'journal immuable : bonus_operation ne se modifie pas'); END;
CREATE TRIGGER bonus_operation_no_delete BEFORE DELETE ON bonus_operation
BEGIN SELECT RAISE(ABORT, 'journal immuable : bonus_operation ne se supprime pas'); END;
CREATE TRIGGER bonus_bet_no_update BEFORE UPDATE ON bonus_bet
BEGIN SELECT RAISE(ABORT, 'journal immuable : bonus_bet ne se modifie pas'); END;
CREATE TRIGGER bonus_bet_no_delete BEFORE DELETE ON bonus_bet
BEGIN SELECT RAISE(ABORT, 'journal immuable : bonus_bet ne se supprime pas'); END;
CREATE TRIGGER bonus_settlement_no_update BEFORE UPDATE ON bonus_settlement
BEGIN SELECT RAISE(ABORT, 'journal immuable : bonus_settlement ne se modifie pas'); END;
CREATE TRIGGER bonus_settlement_no_delete BEFORE DELETE ON bonus_settlement
BEGIN SELECT RAISE(ABORT, 'journal immuable : bonus_settlement ne se supprime pas'); END;
