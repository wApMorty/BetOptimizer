-- Cotes plus/moins de buts (football-data publie la ligne 2,5 depuis 2019/20). Comme `game_odds`, cette table se met
-- à jour à chaque import (upsert). `bookmaker` suit la convention du 1X2 : PS = Pinnacle, Avg, B365, Max.
CREATE TABLE game_totals_odds (
    game_id   INTEGER NOT NULL REFERENCES game (id),
    bookmaker TEXT    NOT NULL,
    moment    TEXT    NOT NULL CHECK (moment IN ('pre', 'close')),
    line      REAL    NOT NULL,
    over      REAL    NOT NULL CHECK (over > 1.0),
    under     REAL    NOT NULL CHECK (under > 1.0),
    PRIMARY KEY (game_id, bookmaker, moment, line)
) STRICT;
