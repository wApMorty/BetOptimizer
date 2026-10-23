-- Phase 18 : scores en direct des rencontres des grilles Loto Foot, relevés chez ESPN à l'affichage de `/lotofoot`
-- (docs/sources.md §46).
--
-- Exception assumée à la règle « archive immuable » : ces tables gardent le DERNIER état seulement (choix de Paul du
-- 2026-10-23). Un score provisoire n'est qu'un affichage ; le règlement des tickets reste celui des résultats officiels
-- (`lotofoot_result`), qui ne se modifient jamais.

-- Dernier score lu d'une rencontre de grille.
CREATE TABLE lotofoot_live (
    game        TEXT    NOT NULL,
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
    PRIMARY KEY (game, number, position),
    FOREIGN KEY (game, number, position) REFERENCES lotofoot_match (game, number, position)
) STRICT;

-- Dernier appel à une page ESPN (compétition, jour) : sert au délai minimal entre deux appels et garde ce qui a empêché
-- d'apparier une rencontre (erreur réseau, noms absents de la table d'alias).
CREATE TABLE lotofoot_live_fetch (
    league      TEXT    NOT NULL,
    day         TEXT    NOT NULL,                     -- jour demandé, heure de New York (`20261024`)
    fetched_at  TEXT    NOT NULL,
    issue       TEXT,                                 -- NULL si tout a été apparié
    PRIMARY KEY (league, day)
) STRICT;
