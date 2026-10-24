-- Loto Foot : budget mensuel saisi dans l'application (docs/sources.md §51). Historique immuable : le budget en vigueur à
-- un instant est la dernière saisie antérieure ; le seuil τ en découle par l'abaque du §49 (`lotofoot::abaque`).
CREATE TABLE lotofoot_budget (
    id             INTEGER PRIMARY KEY,
    set_at         TEXT    NOT NULL,                      -- UTC, RFC 3339
    monthly_euros  INTEGER NOT NULL CHECK (monthly_euros BETWEEN 20 AND 400)
) STRICT;

CREATE TRIGGER lotofoot_budget_no_update BEFORE UPDATE ON lotofoot_budget
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_budget ne se modifie pas'); END;
CREATE TRIGGER lotofoot_budget_no_delete BEFORE DELETE ON lotofoot_budget
BEGIN SELECT RAISE(ABORT, 'journal immuable : lotofoot_budget ne se supprime pas'); END;
