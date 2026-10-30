-- Loto Foot : la FDJ renumérote LF7 et LF8 chaque 1er janvier (docs/sources.md §66). Une grille s'identifie désormais par
-- (game, year, number), `year` étant l'année civile de sa clôture, heure de Paris.
--
-- Une clé primaire ne se modifie pas : les dix tables qui portent le numéro sont reconstruites à l'identique, colonne `year`
-- en plus. Reconstruction faite par la migration seule : les déclencheurs d'immuabilité ne protègent que des UPDATE et DELETE,
-- et `DROP TABLE` ne les déclenche pas. Toutes les lignes existantes valent 2026 : l'archive commence en octobre 2026 (n°125
-- et n°138) et la migration part avant la première grille de 2027. Tout tient dans la transaction de `migrate`.
--
-- Ordre : on renomme les anciennes tables (SQLite suit le renommage dans les clés étrangères), puis on crée et remplit les
-- nouvelles, parents d'abord ; les anciennes partent en dernier, enfants d'abord. Les index et déclencheurs d'abord, car leurs
-- noms sont repris à l'identique.
DROP INDEX lotofoot_split_by_grid;
DROP INDEX lotofoot_reco_by_grid;

DROP TRIGGER lotofoot_grid_no_update;
DROP TRIGGER lotofoot_grid_no_delete;
DROP TRIGGER lotofoot_match_no_update;
DROP TRIGGER lotofoot_match_no_delete;
DROP TRIGGER lotofoot_split_no_update;
DROP TRIGGER lotofoot_split_no_delete;
DROP TRIGGER lotofoot_result_no_update;
DROP TRIGGER lotofoot_result_no_delete;
DROP TRIGGER lotofoot_rank_no_update;
DROP TRIGGER lotofoot_rank_no_delete;
DROP TRIGGER lotofoot_reco_no_update;
DROP TRIGGER lotofoot_reco_no_delete;
DROP TRIGGER lotofoot_play_no_update;
DROP TRIGGER lotofoot_play_no_delete;
DROP TRIGGER lotofoot_alert_no_update;
DROP TRIGGER lotofoot_alert_no_delete;
DROP TRIGGER lotofoot_match_revision_no_update;
DROP TRIGGER lotofoot_match_revision_no_delete;

ALTER TABLE lotofoot_grid RENAME TO lotofoot_grid_old;
ALTER TABLE lotofoot_match RENAME TO lotofoot_match_old;
ALTER TABLE lotofoot_split RENAME TO lotofoot_split_old;
ALTER TABLE lotofoot_result RENAME TO lotofoot_result_old;
ALTER TABLE lotofoot_rank RENAME TO lotofoot_rank_old;
ALTER TABLE lotofoot_reco RENAME TO lotofoot_reco_old;
ALTER TABLE lotofoot_play RENAME TO lotofoot_play_old;
ALTER TABLE lotofoot_alert RENAME TO lotofoot_alert_old;
ALTER TABLE lotofoot_live RENAME TO lotofoot_live_old;
ALTER TABLE lotofoot_match_revision RENAME TO lotofoot_match_revision_old;

CREATE TABLE lotofoot_grid (
    game        TEXT    NOT NULL CHECK (game IN ('lf7', 'lf8')),
    year        INTEGER NOT NULL CHECK (year BETWEEN 2000 AND 2100),  -- année civile de la clôture, heure de Paris
    number      INTEGER NOT NULL CHECK (number > 0),
    closes_at   TEXT    NOT NULL,                     -- fin de validation, UTC
    jackpot     REAL    CHECK (jackpot IS NULL OR jackpot > 0),  -- gain minimum garanti du rang 1, en euros
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    PRIMARY KEY (game, year, number)
) STRICT;

CREATE TABLE lotofoot_match (
    game        TEXT    NOT NULL,
    year        INTEGER NOT NULL,
    number      INTEGER NOT NULL,
    position    INTEGER NOT NULL CHECK (position >= 1),
    home_raw    TEXT    NOT NULL,
    away_raw    TEXT    NOT NULL,
    PRIMARY KEY (game, year, number, position),
    FOREIGN KEY (game, year, number) REFERENCES lotofoot_grid (game, year, number)
) STRICT;

CREATE TABLE lotofoot_split (
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    game        TEXT    NOT NULL,
    year        INTEGER NOT NULL,
    number      INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    q_home      INTEGER NOT NULL CHECK (q_home BETWEEN 0 AND 100),
    q_draw      INTEGER NOT NULL CHECK (q_draw BETWEEN 0 AND 100),
    q_away      INTEGER NOT NULL CHECK (q_away BETWEEN 0 AND 100),
    split_at    TEXT,                                 -- heure du relevé annoncée par le site, UTC
    PRIMARY KEY (fetch_id, game, year, number, position),
    FOREIGN KEY (game, year, number, position) REFERENCES lotofoot_match (game, year, number, position)
) STRICT;

-- Pas de clé étrangère vers `lotofoot_grid` : un résultat peut précéder la grille, et des rapports n'ont aucune grille
-- (n°125 à 132, n°138 à 145 : antérieurs au premier relevé).
CREATE TABLE lotofoot_result (
    game        TEXT    NOT NULL,
    year        INTEGER NOT NULL CHECK (year BETWEEN 2000 AND 2100),
    number      INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('H', 'D', 'A')),
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    PRIMARY KEY (game, year, number, position)
) STRICT;

CREATE TABLE lotofoot_rank (
    game        TEXT    NOT NULL CHECK (game IN ('lf7', 'lf8')),
    year        INTEGER NOT NULL CHECK (year BETWEEN 2000 AND 2100),
    number      INTEGER NOT NULL,
    rank        INTEGER NOT NULL CHECK (rank IN (1, 2)),
    winners     INTEGER NOT NULL CHECK (winners >= 0),
    unit_gain   REAL    CHECK (unit_gain IS NULL OR unit_gain > 0),
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    PRIMARY KEY (game, year, number, rank)
) STRICT;

CREATE TABLE lotofoot_reco (
    id               INTEGER PRIMARY KEY,
    session_id       INTEGER NOT NULL REFERENCES lotofoot_session (id),
    game             TEXT    NOT NULL,
    year             INTEGER NOT NULL,
    number           INTEGER NOT NULL,
    ticket           TEXT    NOT NULL,                -- '1N22121'
    position         INTEGER NOT NULL CHECK (position >= 1),   -- ordre dans la séance (meilleure espérance d'abord)
    ev_base          REAL    NOT NULL,                -- euros rendus par euro misé, modèle de base
    ev_pessimistic   REAL    NOT NULL,                -- idem, scénario défavorable
    paid_probability REAL    NOT NULL CHECK (paid_probability BETWEEN 0 AND 1),
    stakes_estimate  REAL    NOT NULL CHECK (stakes_estimate > 0),
    split_fetch_id   INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    inputs           TEXT    NOT NULL,                -- JSON : p, q, masses, appels d'odds utilisés
    FOREIGN KEY (game, year, number) REFERENCES lotofoot_grid (game, year, number)
) STRICT;

CREATE TABLE lotofoot_play (
    id          INTEGER PRIMARY KEY,
    played_at   TEXT    NOT NULL,
    game        TEXT    NOT NULL,
    year        INTEGER NOT NULL,
    number      INTEGER NOT NULL,
    ticket      TEXT    NOT NULL,
    stake       REAL    NOT NULL CHECK (stake > 0),
    reco_id     INTEGER REFERENCES lotofoot_reco (id),
    off_model   INTEGER NOT NULL CHECK (off_model IN (0, 1)),
    CHECK ((reco_id IS NULL) = (off_model = 1)),
    FOREIGN KEY (game, year, number) REFERENCES lotofoot_grid (game, year, number)
) STRICT;

CREATE TABLE lotofoot_alert (
    game     TEXT    NOT NULL,
    year     INTEGER NOT NULL,
    number   INTEGER NOT NULL,
    sent_at  TEXT    NOT NULL,                       -- UTC, RFC 3339
    PRIMARY KEY (game, year, number),
    FOREIGN KEY (game, year, number) REFERENCES lotofoot_grid (game, year, number)
) STRICT;

CREATE TABLE lotofoot_live (
    game        TEXT    NOT NULL,
    year        INTEGER NOT NULL,
    number      INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    league      TEXT    NOT NULL,                     -- code ESPN (`fra.2`)
    event_id    TEXT    NOT NULL,                     -- identifiant ESPN du match
    home_goals  INTEGER NOT NULL CHECK (home_goals >= 0),
    away_goals  INTEGER NOT NULL CHECK (away_goals >= 0),
    phase       TEXT    NOT NULL CHECK (phase IN ('not_started', 'playing', 'extra_time', 'finished',
                                                  'after_extra_time', 'interrupted')),
    label       TEXT    NOT NULL,                     -- état en clair : « 57' », « MT », « terminé »
    fetched_at  TEXT    NOT NULL,                     -- UTC, RFC 3339
    PRIMARY KEY (game, year, number, position),
    FOREIGN KEY (game, year, number, position) REFERENCES lotofoot_match (game, year, number, position)
) STRICT;

CREATE TABLE lotofoot_match_revision (
    game        TEXT    NOT NULL,
    year        INTEGER NOT NULL,
    number      INTEGER NOT NULL,
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    position    INTEGER NOT NULL CHECK (position >= 1),
    home_raw    TEXT    NOT NULL,
    away_raw    TEXT    NOT NULL,
    PRIMARY KEY (game, year, number, fetch_id, position),
    FOREIGN KEY (game, year, number) REFERENCES lotofoot_grid (game, year, number)
) STRICT;

INSERT INTO lotofoot_grid (game, year, number, closes_at, jackpot, fetch_id)
    SELECT game, 2026, number, closes_at, jackpot, fetch_id FROM lotofoot_grid_old;
INSERT INTO lotofoot_match (game, year, number, position, home_raw, away_raw)
    SELECT game, 2026, number, position, home_raw, away_raw FROM lotofoot_match_old;
INSERT INTO lotofoot_split (fetch_id, game, year, number, position, q_home, q_draw, q_away, split_at)
    SELECT fetch_id, game, 2026, number, position, q_home, q_draw, q_away, split_at FROM lotofoot_split_old;
INSERT INTO lotofoot_result (game, year, number, position, outcome, fetch_id)
    SELECT game, 2026, number, position, outcome, fetch_id FROM lotofoot_result_old;
INSERT INTO lotofoot_rank (game, year, number, rank, winners, unit_gain, fetch_id)
    SELECT game, 2026, number, rank, winners, unit_gain, fetch_id FROM lotofoot_rank_old;
-- Les identifiants sont repris tels quels : `lotofoot_play.reco_id` et les séances s'y réfèrent.
INSERT INTO lotofoot_reco (id, session_id, game, year, number, ticket, position, ev_base, ev_pessimistic, paid_probability,
                           stakes_estimate, split_fetch_id, inputs)
    SELECT id, session_id, game, 2026, number, ticket, position, ev_base, ev_pessimistic, paid_probability,
           stakes_estimate, split_fetch_id, inputs FROM lotofoot_reco_old;
INSERT INTO lotofoot_play (id, played_at, game, year, number, ticket, stake, reco_id, off_model)
    SELECT id, played_at, game, 2026, number, ticket, stake, reco_id, off_model FROM lotofoot_play_old;
INSERT INTO lotofoot_alert (game, year, number, sent_at)
    SELECT game, 2026, number, sent_at FROM lotofoot_alert_old;
INSERT INTO lotofoot_live (game, year, number, position, league, event_id, home_goals, away_goals, phase, label, fetched_at)
    SELECT game, 2026, number, position, league, event_id, home_goals, away_goals, phase, label, fetched_at
    FROM lotofoot_live_old;
INSERT INTO lotofoot_match_revision (game, year, number, fetch_id, position, home_raw, away_raw)
    SELECT game, 2026, number, fetch_id, position, home_raw, away_raw FROM lotofoot_match_revision_old;

DROP TABLE lotofoot_live_old;
DROP TABLE lotofoot_split_old;
DROP TABLE lotofoot_match_revision_old;
DROP TABLE lotofoot_alert_old;
DROP TABLE lotofoot_play_old;
DROP TABLE lotofoot_reco_old;
DROP TABLE lotofoot_rank_old;
DROP TABLE lotofoot_result_old;
DROP TABLE lotofoot_match_old;
DROP TABLE lotofoot_grid_old;

CREATE INDEX lotofoot_split_by_grid ON lotofoot_split (game, year, number, fetch_id);
CREATE INDEX lotofoot_reco_by_grid ON lotofoot_reco (game, year, number);

CREATE TRIGGER lotofoot_grid_no_update BEFORE UPDATE ON lotofoot_grid
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_grid ne se modifie pas'); END;
CREATE TRIGGER lotofoot_grid_no_delete BEFORE DELETE ON lotofoot_grid
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_grid ne se supprime pas'); END;
CREATE TRIGGER lotofoot_match_no_update BEFORE UPDATE ON lotofoot_match
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_match ne se modifie pas'); END;
CREATE TRIGGER lotofoot_match_no_delete BEFORE DELETE ON lotofoot_match
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_match ne se supprime pas'); END;
CREATE TRIGGER lotofoot_split_no_update BEFORE UPDATE ON lotofoot_split
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_split ne se modifie pas'); END;
CREATE TRIGGER lotofoot_split_no_delete BEFORE DELETE ON lotofoot_split
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_split ne se supprime pas'); END;
CREATE TRIGGER lotofoot_result_no_update BEFORE UPDATE ON lotofoot_result
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_result ne se modifie pas'); END;
CREATE TRIGGER lotofoot_result_no_delete BEFORE DELETE ON lotofoot_result
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_result ne se supprime pas'); END;
CREATE TRIGGER lotofoot_rank_no_update BEFORE UPDATE ON lotofoot_rank
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_rank ne se modifie pas'); END;
CREATE TRIGGER lotofoot_rank_no_delete BEFORE DELETE ON lotofoot_rank
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_rank ne se supprime pas'); END;
CREATE TRIGGER lotofoot_reco_no_update BEFORE UPDATE ON lotofoot_reco
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_reco ne se modifie pas'); END;
CREATE TRIGGER lotofoot_reco_no_delete BEFORE DELETE ON lotofoot_reco
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_reco ne se supprime pas'); END;
CREATE TRIGGER lotofoot_play_no_update BEFORE UPDATE ON lotofoot_play
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_play ne se modifie pas'); END;
CREATE TRIGGER lotofoot_play_no_delete BEFORE DELETE ON lotofoot_play
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_play ne se supprime pas'); END;
CREATE TRIGGER lotofoot_alert_no_update BEFORE UPDATE ON lotofoot_alert
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_alert ne se modifie pas'); END;
CREATE TRIGGER lotofoot_alert_no_delete BEFORE DELETE ON lotofoot_alert
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_alert ne se supprime pas'); END;
CREATE TRIGGER lotofoot_match_revision_no_update BEFORE UPDATE ON lotofoot_match_revision
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_match_revision ne se modifie pas'); END;
CREATE TRIGGER lotofoot_match_revision_no_delete BEFORE DELETE ON lotofoot_match_revision
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_match_revision ne se supprime pas'); END;
