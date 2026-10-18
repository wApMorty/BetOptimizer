-- Phase 7 : tennis. Matchs et cotes de tennis-data.co.uk (tableurs téléchargés à la main), puis réglage gelé de l'Elo.
-- Comme pour le football (migration 3), l'import est un « upsert » : le fichier de l'année en cours s'enrichit chaque semaine.

CREATE TABLE tennis_match (
    id             INTEGER PRIMARY KEY,
    tour           TEXT    NOT NULL CHECK (tour IN ('atp', 'wta')),
    season         INTEGER NOT NULL,                  -- année du fichier
    tournament     TEXT    NOT NULL,
    location       TEXT,
    match_date     TEXT    NOT NULL,                  -- 'YYYY-MM-DD'
    series         TEXT,                              -- ATP : Series ; WTA : Tier (texte brut)
    court          TEXT,                              -- 'Indoor' ou 'Outdoor'
    surface        TEXT    NOT NULL CHECK (surface IN ('hard', 'clay', 'grass', 'carpet')),
    round          TEXT    NOT NULL,
    best_of        INTEGER NOT NULL CHECK (best_of IN (3, 5)),
    winner         TEXT    NOT NULL,                  -- nom tel que publié (« Federer R. ») : pas d'identifiant de joueur
    loser          TEXT    NOT NULL,
    winner_rank    INTEGER,
    loser_rank     INTEGER,
    winner_points  INTEGER,
    loser_points   INTEGER,
    winner_sets    INTEGER,
    loser_sets     INTEGER,
    status         TEXT    NOT NULL CHECK (status IN ('completed', 'retired', 'walkover')),
    UNIQUE (tour, match_date, tournament, round, winner, loser)
) STRICT;

CREATE INDEX tennis_match_by_date ON tennis_match (tour, match_date);

-- Cotes du vainqueur et du perdant, par bookmaker (préfixe de colonne : PS = Pinnacle, B365, Avg = moyenne Oddsportal…).
-- « Les plus récentes avant le début du match » : quasiment la clôture (notes.txt de tennis-data).
CREATE TABLE tennis_odds (
    match_id    INTEGER NOT NULL REFERENCES tennis_match (id),
    bookmaker   TEXT    NOT NULL,
    winner_odds REAL    NOT NULL CHECK (winner_odds > 1.0),
    loser_odds  REAL    NOT NULL CHECK (loser_odds > 1.0),
    PRIMARY KEY (match_id, bookmaker)
) STRICT;

-- Réglage de l'Elo : la configuration choisie sur la validation, gelée avant le test. Toutes les sessions sont conservées.
CREATE TABLE tennis_tuning (
    id             INTEGER PRIMARY KEY,
    created_at     TEXT    NOT NULL,
    k_base         REAL    NOT NULL,
    k_shape        REAL    NOT NULL,
    surface_weight REAL    NOT NULL,
    validation_log_loss REAL NOT NULL,
    validation_matches  INTEGER NOT NULL,
    configs_tried  INTEGER NOT NULL
) STRICT;

CREATE TRIGGER tennis_tuning_no_update BEFORE UPDATE ON tennis_tuning
BEGIN SELECT RAISE(ABORT, 'journal immuable : tennis_tuning ne se modifie pas'); END;
CREATE TRIGGER tennis_tuning_no_delete BEFORE DELETE ON tennis_tuning
BEGIN SELECT RAISE(ABORT, 'journal immuable : tennis_tuning ne se supprime pas'); END;
