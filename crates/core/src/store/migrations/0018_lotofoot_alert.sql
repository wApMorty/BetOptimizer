-- Loto Foot : alertes « nouvelle grille » envoyées sur Discord (docs/sources.md §47). Une ligne par grille, écrite après
-- l'envoi réussi : une alerte qui échoue est retentée au relevé suivant, et une grille n'est jamais signalée deux fois,
-- quel que soit le relevé (horaire, séance, bouton) qui l'a vue en premier.
CREATE TABLE lotofoot_alert (
    game     TEXT    NOT NULL,
    number   INTEGER NOT NULL,
    sent_at  TEXT    NOT NULL,                       -- UTC, RFC 3339
    PRIMARY KEY (game, number),
    FOREIGN KEY (game, number) REFERENCES lotofoot_grid (game, number)
) STRICT;

CREATE TRIGGER lotofoot_alert_no_update BEFORE UPDATE ON lotofoot_alert
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_alert ne se modifie pas'); END;
CREATE TRIGGER lotofoot_alert_no_delete BEFORE DELETE ON lotofoot_alert
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_alert ne se supprime pas'); END;
