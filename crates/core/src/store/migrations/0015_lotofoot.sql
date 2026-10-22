-- Phase 14 : Loto Foot (LF7, LF8). Faits relevés sur prono-match.fr (jamais les pages : leurs mentions légales
-- interdisent la reproduction), journal immuable des recommandations et des tickets joués (docs/sources.md §37 et §38).
-- Tout est immuable : une ligne ne se modifie ni ne se supprime.

-- Journal des lectures de pages. Aucune page n'est conservée ; `error` dit pourquoi une page n'a pas été lue.
CREATE TABLE lotofoot_fetch (
    id          INTEGER PRIMARY KEY,
    fetched_at  TEXT    NOT NULL,                     -- UTC, RFC 3339
    page        TEXT    NOT NULL CHECK (page IN ('lf7', 'lf8', 'results')),
    http_status INTEGER NOT NULL,
    error       TEXT                                  -- NULL si la page a été lue
) STRICT;

-- Une grille, telle qu'elle est vue la première fois (clôture et pactole ne changent pas ensuite).
CREATE TABLE lotofoot_grid (
    game        TEXT    NOT NULL CHECK (game IN ('lf7', 'lf8')),
    number      INTEGER NOT NULL CHECK (number > 0),
    closes_at   TEXT    NOT NULL,                     -- fin de validation, UTC
    jackpot     REAL    CHECK (jackpot IS NULL OR jackpot > 0),  -- gain minimum garanti du rang 1, en euros
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    PRIMARY KEY (game, number)
) STRICT;

-- Rencontres d'une grille, noms bruts du site.
CREATE TABLE lotofoot_match (
    game        TEXT    NOT NULL,
    number      INTEGER NOT NULL,
    position    INTEGER NOT NULL CHECK (position >= 1),
    home_raw    TEXT    NOT NULL,
    away_raw    TEXT    NOT NULL,
    PRIMARY KEY (game, number, position),
    FOREIGN KEY (game, number) REFERENCES lotofoot_grid (game, number)
) STRICT;

-- Répartition FDJ d'une rencontre à un relevé donné (pourcents entiers, tels qu'affichés). Un relevé par heure : on
-- mesure ainsi la dérive de la répartition jusqu'à la clôture.
CREATE TABLE lotofoot_split (
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    game        TEXT    NOT NULL,
    number      INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    q_home      INTEGER NOT NULL CHECK (q_home BETWEEN 0 AND 100),
    q_draw      INTEGER NOT NULL CHECK (q_draw BETWEEN 0 AND 100),
    q_away      INTEGER NOT NULL CHECK (q_away BETWEEN 0 AND 100),
    split_at    TEXT,                                 -- heure du relevé annoncée par le site, UTC
    PRIMARY KEY (fetch_id, game, number, position),
    FOREIGN KEY (game, number, position) REFERENCES lotofoot_match (game, number, position)
) STRICT;

-- Signe sorti (page des résultats). Premier relevé seulement : un résultat ne change pas.
CREATE TABLE lotofoot_result (
    game        TEXT    NOT NULL,
    number      INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('H', 'D', 'A')),
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    PRIMARY KEY (game, number, position)
) STRICT;

-- Rapport officiel d'un rang (1 : sans-faute, 2 : une erreur). Le gain unitaire publié inclut tous les gagnants.
CREATE TABLE lotofoot_rank (
    game        TEXT    NOT NULL CHECK (game IN ('lf7', 'lf8')),
    number      INTEGER NOT NULL,
    rank        INTEGER NOT NULL CHECK (rank IN (1, 2)),
    winners     INTEGER NOT NULL CHECK (winners >= 0),
    unit_gain   REAL    CHECK (unit_gain IS NULL OR unit_gain > 0),
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    PRIMARY KEY (game, number, rank)
) STRICT;

-- Séance de recommandation (le samedi) : version et paramètres du modèle, gelés au moment de la séance.
CREATE TABLE lotofoot_session (
    id            INTEGER PRIMARY KEY,
    created_at    TEXT    NOT NULL,
    model_version TEXT    NOT NULL,                   -- 'lotofoot-1'
    params        TEXT    NOT NULL,                   -- JSON : κ, β, φ, μ, τ, budget, scénario défavorable
    notes         TEXT    NOT NULL                    -- JSON : grilles évaluées, grilles écartées et pourquoi
) STRICT;

-- Ticket recommandé : tout ce qui a servi à le choisir, figé à sa création (CLAUDE.md, invariant 6).
CREATE TABLE lotofoot_reco (
    id               INTEGER PRIMARY KEY,
    session_id       INTEGER NOT NULL REFERENCES lotofoot_session (id),
    game             TEXT    NOT NULL,
    number           INTEGER NOT NULL,
    ticket           TEXT    NOT NULL,                -- '1N22121'
    position         INTEGER NOT NULL CHECK (position >= 1),   -- ordre dans la séance (meilleure espérance d'abord)
    ev_base          REAL    NOT NULL,                -- euros rendus par euro misé, modèle de base
    ev_pessimistic   REAL    NOT NULL,                -- idem, scénario défavorable
    paid_probability REAL    NOT NULL CHECK (paid_probability BETWEEN 0 AND 1),
    stakes_estimate  REAL    NOT NULL CHECK (stakes_estimate > 0),
    split_fetch_id   INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    inputs           TEXT    NOT NULL,                -- JSON : p, q, masses, appels d'odds utilisés
    FOREIGN KEY (game, number) REFERENCES lotofoot_grid (game, number)
) STRICT;

-- Ticket réellement joué (déclaré par Paul). `off_model = 1` : il ne correspond à aucune recommandation (invariant 7).
CREATE TABLE lotofoot_play (
    id          INTEGER PRIMARY KEY,
    played_at   TEXT    NOT NULL,
    game        TEXT    NOT NULL,
    number      INTEGER NOT NULL,
    ticket      TEXT    NOT NULL,
    stake       REAL    NOT NULL CHECK (stake > 0),
    reco_id     INTEGER REFERENCES lotofoot_reco (id),
    off_model   INTEGER NOT NULL CHECK (off_model IN (0, 1)),
    CHECK ((reco_id IS NULL) = (off_model = 1)),
    FOREIGN KEY (game, number) REFERENCES lotofoot_grid (game, number)
) STRICT;

CREATE INDEX lotofoot_split_by_grid ON lotofoot_split (game, number, fetch_id);
CREATE INDEX lotofoot_reco_by_grid ON lotofoot_reco (game, number);

CREATE TRIGGER lotofoot_fetch_no_update BEFORE UPDATE ON lotofoot_fetch
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_fetch ne se modifie pas'); END;
CREATE TRIGGER lotofoot_fetch_no_delete BEFORE DELETE ON lotofoot_fetch
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_fetch ne se supprime pas'); END;
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
CREATE TRIGGER lotofoot_session_no_update BEFORE UPDATE ON lotofoot_session
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_session ne se modifie pas'); END;
CREATE TRIGGER lotofoot_session_no_delete BEFORE DELETE ON lotofoot_session
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_session ne se supprime pas'); END;
CREATE TRIGGER lotofoot_reco_no_update BEFORE UPDATE ON lotofoot_reco
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_reco ne se modifie pas'); END;
CREATE TRIGGER lotofoot_reco_no_delete BEFORE DELETE ON lotofoot_reco
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_reco ne se supprime pas'); END;
CREATE TRIGGER lotofoot_play_no_update BEFORE UPDATE ON lotofoot_play
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_play ne se modifie pas'); END;
CREATE TRIGGER lotofoot_play_no_delete BEFORE DELETE ON lotofoot_play
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_play ne se supprime pas'); END;
