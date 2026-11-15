-- Phase 22 : trois natures d'alerte de plus dans `alert_log` (docs/sources.md §75) :
--   'source_fallback' : la FDJ n'a pas servi une page, resultats-fdj.fr l'a lue à la place ;
--   'source_down'     : ni la FDJ ni resultats-fdj.fr n'ont pu servir une page ;
--   'canceled_grid'   : une grille entièrement annulée porte des tickets joués (mises remboursées au guichet).
-- Une contrainte CHECK ne se modifie pas : la table est reconstruite, sous une table provisoire pour garder le nom
-- d'origine. Rien ne référence `alert_log` (aucune clé étrangère). Reconstruction faite par la migration seule : les
-- déclencheurs d'immuabilité ne protègent que des UPDATE et DELETE, et `DROP TABLE` ne les déclenche pas.
CREATE TABLE alert_log_new (
    id       INTEGER PRIMARY KEY,
    sent_at  TEXT    NOT NULL,                       -- UTC, RFC 3339
    kind     TEXT    NOT NULL CHECK (kind IN ('stale_source', 'unit_failed', 'source_fallback', 'source_down', 'canceled_grid')),
    subject  TEXT    NOT NULL CHECK (subject <> ''), -- ce que l'alerte désigne : 'lf8-2026-154', 'edge-snapshot.service', 'open'
    detail   TEXT    NOT NULL                        -- le texte envoyé
) STRICT;

-- Les identifiants sont repris tels quels.
INSERT INTO alert_log_new (id, sent_at, kind, subject, detail)
    SELECT id, sent_at, kind, subject, detail FROM alert_log;

DROP TABLE alert_log;   -- l'index et les deux déclencheurs de la table partent avec elle
ALTER TABLE alert_log_new RENAME TO alert_log;

CREATE INDEX alert_log_by_subject ON alert_log (kind, subject, sent_at);

CREATE TRIGGER alert_log_no_update BEFORE UPDATE ON alert_log
BEGIN SELECT RAISE(ABORT, 'journal immuable : alert_log ne se modifie pas'); END;
CREATE TRIGGER alert_log_no_delete BEFORE DELETE ON alert_log
BEGIN SELECT RAISE(ABORT, 'journal immuable : alert_log ne se supprime pas'); END;
