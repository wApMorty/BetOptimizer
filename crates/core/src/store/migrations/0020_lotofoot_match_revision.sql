-- Loto Foot : compositions successives d'une grille (docs/sources.md §52). `lotofoot_match` garde la première composition
-- vue ; si la page d'une formule montre ensuite d'autres rencontres pour le même numéro, la nouvelle composition entière est
-- écrite ici, rattachée au relevé qui l'a vue. La composition en vigueur est celle du dernier relevé ; une répartition ou une
-- recommandation antérieure à ce relevé ne vaut plus pour la grille.
CREATE TABLE lotofoot_match_revision (
    game        TEXT    NOT NULL,
    number      INTEGER NOT NULL,
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    position    INTEGER NOT NULL CHECK (position >= 1),
    home_raw    TEXT    NOT NULL,
    away_raw    TEXT    NOT NULL,
    PRIMARY KEY (game, number, fetch_id, position),
    FOREIGN KEY (game, number) REFERENCES lotofoot_grid (game, number)
) STRICT;

CREATE TRIGGER lotofoot_match_revision_no_update BEFORE UPDATE ON lotofoot_match_revision
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_match_revision ne se modifie pas'); END;
CREATE TRIGGER lotofoot_match_revision_no_delete BEFORE DELETE ON lotofoot_match_revision
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_match_revision ne se supprime pas'); END;
