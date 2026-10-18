-- Historique des matchs et des cotes (football-data.co.uk). Contrairement à l'archive des cotes en direct
-- (migration 2), ces tables se mettent à jour : football-data corrige parfois ses fichiers, et la saison
-- en cours s'enrichit chaque semaine. L'import fait un « upsert » sur la clé naturelle du match.

-- Équipes, par nom canonique (voir config/aliases.toml).
CREATE TABLE team (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE game (
    id            INTEGER PRIMARY KEY,
    div           TEXT    NOT NULL,                 -- code football-data de la ligue : E0, SP1, D1, I1, F1, SC0
    season        TEXT    NOT NULL,                 -- '2025/26'
    match_date    TEXT    NOT NULL,                 -- 'YYYY-MM-DD', date locale publiée
    kickoff_utc   TEXT,                             -- RFC 3339 UTC ; NULL si l'heure n'est pas publiée (2016/17 à 2018/19)
    home_id       INTEGER NOT NULL REFERENCES team (id),
    away_id       INTEGER NOT NULL REFERENCES team (id),
    ft_home       INTEGER NOT NULL,                 -- buts à la fin du match
    ft_away       INTEGER NOT NULL,
    ft_result     TEXT    NOT NULL CHECK (ft_result IN ('H', 'D', 'A')),  -- tel que publié ; la QC le confronte aux buts
    ht_home       INTEGER,
    ht_away       INTEGER,
    referee       TEXT,
    shots_home INTEGER, shots_away INTEGER,
    shots_on_target_home INTEGER, shots_on_target_away INTEGER,
    corners_home INTEGER, corners_away INTEGER,
    fouls_home INTEGER, fouls_away INTEGER,
    yellow_home INTEGER, yellow_away INTEGER,
    red_home INTEGER, red_away INTEGER,
    xg_home       REAL,                             -- publié depuis 2026/27 seulement, source non documentée
    xg_away       REAL,
    UNIQUE (div, match_date, home_id, away_id)
) STRICT;

CREATE INDEX game_by_season ON game (div, season, match_date);

-- Cotes 1X2 par bookmaker et par moment. `bookmaker` est le préfixe de colonne football-data
-- (PS = Pinnacle, B365 = Bet365, Avg = moyenne du marché, Max = meilleure cote de chaque issue…).
CREATE TABLE game_odds (
    game_id   INTEGER NOT NULL REFERENCES game (id),
    bookmaker TEXT    NOT NULL,
    moment    TEXT    NOT NULL CHECK (moment IN ('pre', 'close')),
    home      REAL    NOT NULL CHECK (home > 1.0),
    draw      REAL    NOT NULL CHECK (draw > 1.0),
    away      REAL    NOT NULL CHECK (away > 1.0),
    PRIMARY KEY (game_id, bookmaker, moment)
) STRICT;
