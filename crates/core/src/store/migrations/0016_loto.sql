-- Phase 16 : Grand Loto et alerte EuroMillions. Historique officiel de la FDJ (archives ZIP publiques), journal immuable
-- des séances, des grilles recommandées et jouées, et des alertes envoyées (docs/sources.md §43 et §44).
-- Tout est immuable : une ligne ne se modifie ni ne se supprime.

-- Journal des téléchargements d'archives. `error` dit pourquoi une archive n'a pas été lue.
CREATE TABLE loto_fetch (
    id          INTEGER PRIMARY KEY,
    fetched_at  TEXT    NOT NULL,                     -- UTC, RFC 3339
    document    TEXT    NOT NULL,                     -- identifiant de l'archive FDJ
    http_status INTEGER NOT NULL,
    draws       INTEGER NOT NULL CHECK (draws >= 0),  -- tirages lus dans l'archive
    error       TEXT                                  -- NULL si l'archive a été lue
) STRICT;

-- Un tirage, tel que publié la première fois (une archive officielle ne se corrige pas : une différence est signalée).
CREATE TABLE loto_draw (
    game        TEXT    NOT NULL CHECK (game IN ('loto', 'grand_loto', 'euromillions')),
    draw_date   TEXT    NOT NULL,                     -- AAAA-MM-JJ
    balls       TEXT    NOT NULL,                     -- '2-3-30-35-44', ordre croissant
    extra       TEXT    NOT NULL,                     -- numéro chance ('10') ou étoiles ('4-6')
    codes       INTEGER CHECK (codes IS NULL OR codes >= 0),
    code_gain   REAL    CHECK (code_gain IS NULL OR code_gain >= 0),
    fetch_id    INTEGER NOT NULL REFERENCES loto_fetch (id),
    PRIMARY KEY (game, draw_date)
) STRICT;

-- Rangs d'un tirage : gagnants (en Europe pour l'EuroMillions) et gain unitaire en euros.
CREATE TABLE loto_rank (
    game        TEXT    NOT NULL,
    draw_date   TEXT    NOT NULL,
    rank        INTEGER NOT NULL CHECK (rank BETWEEN 1 AND 13),
    winners     INTEGER NOT NULL CHECK (winners >= 0),
    unit_gain   REAL    NOT NULL CHECK (unit_gain >= 0),
    PRIMARY KEY (game, draw_date, rank),
    FOREIGN KEY (game, draw_date) REFERENCES loto_draw (game, draw_date)
) STRICT;

-- Séance de recommandation d'un Grand Loto : version et paramètres du modèle, prévision du nombre de grilles et décision,
-- gelés au moment de la séance. Une seule séance par édition : le timer ne la refait pas.
CREATE TABLE loto_session (
    id            INTEGER PRIMARY KEY,
    created_at    TEXT    NOT NULL,
    draw_date     TEXT    NOT NULL UNIQUE,            -- date de l'édition (AAAA-MM-JJ)
    model_version TEXT    NOT NULL,                   -- 'grandloto-1'
    params        TEXT    NOT NULL,                   -- JSON : jackpot, prix, budget, seuil, quantile, préférences
    notes         TEXT    NOT NULL                    -- JSON : prévision, éditions utilisées, décision et raison
) STRICT;

-- Grille recommandée : tout ce qui a servi à la choisir, figé à sa création (CLAUDE.md, invariant 6).
CREATE TABLE loto_reco (
    id              INTEGER PRIMARY KEY,
    session_id      INTEGER NOT NULL REFERENCES loto_session (id),
    grid            TEXT    NOT NULL,                 -- '2-3-30-35-44+10'
    position        INTEGER NOT NULL CHECK (position >= 1),
    stake           REAL    NOT NULL CHECK (stake > 0),
    ev              REAL    NOT NULL,                 -- euros rendus par grille, nombre de grilles central
    ev_pessimistic  REAL    NOT NULL,                 -- idem, nombre de grilles pessimiste
    UNIQUE (session_id, grid)
) STRICT;

-- Grille réellement jouée (déclarée par Paul). `off_model = 1` : elle ne correspond à aucune recommandation (invariant 7).
CREATE TABLE loto_play (
    id          INTEGER PRIMARY KEY,
    played_at   TEXT    NOT NULL,
    draw_date   TEXT    NOT NULL,
    grid        TEXT    NOT NULL,
    stake       REAL    NOT NULL CHECK (stake > 0),
    reco_id     INTEGER REFERENCES loto_reco (id),
    off_model   INTEGER NOT NULL CHECK (off_model IN (0, 1)),
    CHECK ((reco_id IS NULL) = (off_model = 1))
) STRICT;

-- Alerte EuroMillions envoyée : une seule par niveau et par tirage déclencheur.
CREATE TABLE loto_alert (
    id          INTEGER PRIMARY KEY,
    sent_at     TEXT    NOT NULL,
    draw_date   TEXT    NOT NULL,                     -- dernier tirage au plafond sans gagnant
    level       TEXT    NOT NULL CHECK (level IN ('watch', 'fifth')),
    message     TEXT    NOT NULL,
    UNIQUE (draw_date, level)
) STRICT;

CREATE TRIGGER loto_fetch_no_update BEFORE UPDATE ON loto_fetch
BEGIN SELECT RAISE(ABORT, 'archive immuable : loto_fetch ne se modifie pas'); END;
CREATE TRIGGER loto_fetch_no_delete BEFORE DELETE ON loto_fetch
BEGIN SELECT RAISE(ABORT, 'archive immuable : loto_fetch ne se supprime pas'); END;
CREATE TRIGGER loto_draw_no_update BEFORE UPDATE ON loto_draw
BEGIN SELECT RAISE(ABORT, 'archive immuable : loto_draw ne se modifie pas'); END;
CREATE TRIGGER loto_draw_no_delete BEFORE DELETE ON loto_draw
BEGIN SELECT RAISE(ABORT, 'archive immuable : loto_draw ne se supprime pas'); END;
CREATE TRIGGER loto_rank_no_update BEFORE UPDATE ON loto_rank
BEGIN SELECT RAISE(ABORT, 'archive immuable : loto_rank ne se modifie pas'); END;
CREATE TRIGGER loto_rank_no_delete BEFORE DELETE ON loto_rank
BEGIN SELECT RAISE(ABORT, 'archive immuable : loto_rank ne se supprime pas'); END;
CREATE TRIGGER loto_session_no_update BEFORE UPDATE ON loto_session
BEGIN SELECT RAISE(ABORT, 'journal immuable : loto_session ne se modifie pas'); END;
CREATE TRIGGER loto_session_no_delete BEFORE DELETE ON loto_session
BEGIN SELECT RAISE(ABORT, 'journal immuable : loto_session ne se supprime pas'); END;
CREATE TRIGGER loto_reco_no_update BEFORE UPDATE ON loto_reco
BEGIN SELECT RAISE(ABORT, 'journal immuable : loto_reco ne se modifie pas'); END;
CREATE TRIGGER loto_reco_no_delete BEFORE DELETE ON loto_reco
BEGIN SELECT RAISE(ABORT, 'journal immuable : loto_reco ne se supprime pas'); END;
CREATE TRIGGER loto_play_no_update BEFORE UPDATE ON loto_play
BEGIN SELECT RAISE(ABORT, 'journal immuable : loto_play ne se modifie pas'); END;
CREATE TRIGGER loto_play_no_delete BEFORE DELETE ON loto_play
BEGIN SELECT RAISE(ABORT, 'journal immuable : loto_play ne se supprime pas'); END;
CREATE TRIGGER loto_alert_no_update BEFORE UPDATE ON loto_alert
BEGIN SELECT RAISE(ABORT, 'journal immuable : loto_alert ne se modifie pas'); END;
CREATE TRIGGER loto_alert_no_delete BEFORE DELETE ON loto_alert
BEGIN SELECT RAISE(ABORT, 'journal immuable : loto_alert ne se supprime pas'); END;
