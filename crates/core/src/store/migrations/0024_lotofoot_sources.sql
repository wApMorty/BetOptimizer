-- Phase 22 : le Loto Foot quitte prono-match pour la FDJ, avec resultats-fdj.fr en repli (docs/sources.md §74).
-- Deux changements de schéma, dans une seule transaction (celle de `migrate`) :
--
-- 1. `lotofoot_result.outcome` accepte 'C' : rencontre annulée. Une rencontre annulée est bonne pour tous les signes
--    (règlement Loto Sports v6, art. 10.6 ; §39 et §74). LF7 n°140 en a une (match 5), et l'archive ne savait pas l'écrire.
-- 2. `lotofoot_fetch` (journal des lectures de pages) gagne `source` (d'où vient la page) et accepte deux nouvelles pages :
--    'open' (grilles ouvertes, toutes formules : `/grilles/ouvertes/loto-foot` de la FDJ, accueil de resultats-fdj.fr) et
--    'detail' (détail d'une grille, repli). Les lectures déjà journalisées viennent toutes de prono-match.
--
-- Une contrainte CHECK ne se modifie pas : les deux tables sont reconstruites. Reconstruction faite par la migration seule :
-- les déclencheurs d'immuabilité ne protègent que des UPDATE et DELETE, et `DROP TABLE` ne les déclenche pas.
--
-- `lotofoot_fetch` est référencée par six tables (grid, split, result, rank, reco, match_revision) : la renommer ferait
-- suivre à SQLite les clés étrangères vers l'ancienne table (voir 0022). On la supprime donc et on la recrée sous le même
-- nom : les clés des tables filles, qui nomment `lotofoot_fetch`, retrouvent leur parent. Entre les deux, des lignes filles
-- n'ont plus de parent ; `defer_foreign_keys` reporte la vérification à la fin de la transaction, et la remise des mêmes
-- identifiants la rétablit. Si elle échouait, `COMMIT` serait refusé et la migration annulée en entier.
PRAGMA defer_foreign_keys = ON;

-- Résultats : aucune table n'en dépend (pas de clé étrangère vers `lotofoot_result`).
DROP TRIGGER lotofoot_result_no_update;
DROP TRIGGER lotofoot_result_no_delete;
ALTER TABLE lotofoot_result RENAME TO lotofoot_result_old;

CREATE TABLE lotofoot_result (
    game        TEXT    NOT NULL,
    year        INTEGER NOT NULL CHECK (year BETWEEN 2000 AND 2100),
    number      INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('H', 'D', 'A', 'C')),  -- 1, N, 2, ou C : rencontre annulée
    fetch_id    INTEGER NOT NULL REFERENCES lotofoot_fetch (id),
    PRIMARY KEY (game, year, number, position)
) STRICT;

INSERT INTO lotofoot_result (game, year, number, position, outcome, fetch_id)
    SELECT game, year, number, position, outcome, fetch_id FROM lotofoot_result_old;
DROP TABLE lotofoot_result_old;

CREATE TRIGGER lotofoot_result_no_update BEFORE UPDATE ON lotofoot_result
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_result ne se modifie pas'); END;
CREATE TRIGGER lotofoot_result_no_delete BEFORE DELETE ON lotofoot_result
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_result ne se supprime pas'); END;

-- Journal des lectures : les lignes sont mises de côté, la table est recréée, puis les mêmes identifiants sont remis.
CREATE TABLE lotofoot_fetch_old AS
    SELECT id, fetched_at, page, http_status, error FROM lotofoot_fetch;
DROP TABLE lotofoot_fetch;   -- les deux déclencheurs de la table partent avec elle

CREATE TABLE lotofoot_fetch (
    id          INTEGER PRIMARY KEY,
    fetched_at  TEXT    NOT NULL,                     -- UTC, RFC 3339
    source      TEXT    NOT NULL CHECK (source IN ('pronomatch', 'fdj', 'resultats_fdj')),
    page        TEXT    NOT NULL CHECK (page IN ('lf7', 'lf8', 'results', 'open', 'detail')),
    http_status INTEGER NOT NULL,
    error       TEXT                                  -- NULL si la page a été lue
) STRICT;

-- Les identifiants sont repris tels quels : les tables filles s'y réfèrent.
INSERT INTO lotofoot_fetch (id, fetched_at, source, page, http_status, error)
    SELECT id, fetched_at, 'pronomatch', page, http_status, error FROM lotofoot_fetch_old;
DROP TABLE lotofoot_fetch_old;

CREATE TRIGGER lotofoot_fetch_no_update BEFORE UPDATE ON lotofoot_fetch
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_fetch ne se modifie pas'); END;
CREATE TRIGGER lotofoot_fetch_no_delete BEFORE DELETE ON lotofoot_fetch
BEGIN SELECT RAISE(ABORT, 'archive immuable : lotofoot_fetch ne se supprime pas'); END;
