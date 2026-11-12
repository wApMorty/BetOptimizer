-- Journal des alertes d'exploitation envoyées sur Discord (phase 21, docs/sources.md §73) : source figée, unité systemd
-- en échec. Il ne sert qu'à ne pas répéter une alerte à chaque passage d'un timer : avant d'envoyer, on regarde quand la
-- même alerte (`kind`, `subject`) est partie pour la dernière fois. Une ligne n'est écrite qu'après un envoi réussi.
CREATE TABLE alert_log (
    id       INTEGER PRIMARY KEY,
    sent_at  TEXT    NOT NULL,                       -- UTC, RFC 3339
    kind     TEXT    NOT NULL CHECK (kind IN ('stale_source', 'unit_failed')),
    subject  TEXT    NOT NULL CHECK (subject <> ''), -- ce que l'alerte désigne : 'lf8-2026-154', 'edge-snapshot.service'
    detail   TEXT    NOT NULL                        -- le texte envoyé
) STRICT;

CREATE INDEX alert_log_by_subject ON alert_log (kind, subject, sent_at);

CREATE TRIGGER alert_log_no_update BEFORE UPDATE ON alert_log
BEGIN SELECT RAISE(ABORT, 'journal immuable : alert_log ne se modifie pas'); END;
CREATE TRIGGER alert_log_no_delete BEFORE DELETE ON alert_log
BEGIN SELECT RAISE(ABORT, 'journal immuable : alert_log ne se supprime pas'); END;
