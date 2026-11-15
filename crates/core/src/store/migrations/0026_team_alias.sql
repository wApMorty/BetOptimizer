-- Phase 23 : alias d'équipes déduits par la rencontre (docs/sources.md §75).
-- Quand une grille Loto Foot (section LOTOFOOT) ou une page ESPN (section ESPN) porte un nom absent de la table d'alias,
-- edge cherche la rencontre parmi les matchs relevés : si l'adversaire est connu et qu'un match lui correspond, il propose
-- l'alias, avec le match qui le prouve. Rien n'entre en vigueur sans décision de Paul (bouton de la page /lotofoot).
-- Les deux tables ne font que croître, comme le journal : une proposition et sa décision ne se modifient jamais.
CREATE TABLE alias_proposal (
    id           INTEGER PRIMARY KEY,
    proposed_at  TEXT    NOT NULL,                       -- UTC, RFC 3339
    section      TEXT    NOT NULL CHECK (section IN ('LOTOFOOT', 'ESPN')),
    name         TEXT    NOT NULL CHECK (name <> ''),    -- nom inconnu, tel que publié (FDJ, ESPN)
    canonical    TEXT    NOT NULL CHECK (canonical <> ''), -- nom canonique proposé (celui de The Odds API)
    context      TEXT    NOT NULL,                       -- où le nom a bloqué : « LF8 n°151, rencontre 3 : Malaga – Fulham »
    source       TEXT    NOT NULL CHECK (source IN ('odds_api', 'oddspapi', 'espn')),
    source_id    TEXT    NOT NULL,                       -- identifiant du match chez la source
    seen_home    TEXT    NOT NULL,                       -- noms du match relevé, tels que publiés par la source
    seen_away    TEXT    NOT NULL,
    kickoff      TEXT    NOT NULL,                       -- coup d'envoi du match relevé, UTC
    UNIQUE (section, name, canonical)                    -- une même proposition n'est faite qu'une fois
) STRICT;

CREATE TRIGGER alias_proposal_no_update BEFORE UPDATE ON alias_proposal
BEGIN SELECT RAISE(ABORT, 'journal immuable : alias_proposal ne se modifie pas'); END;
CREATE TRIGGER alias_proposal_no_delete BEFORE DELETE ON alias_proposal
BEGIN SELECT RAISE(ABORT, 'journal immuable : alias_proposal ne se supprime pas'); END;

-- Une décision au plus par proposition (clé primaire) : acceptée (1) ou refusée (0).
CREATE TABLE alias_decision (
    proposal_id  INTEGER PRIMARY KEY REFERENCES alias_proposal (id),
    decided_at   TEXT    NOT NULL,                       -- UTC, RFC 3339
    accepted     INTEGER NOT NULL CHECK (accepted IN (0, 1))
) STRICT;

CREATE TRIGGER alias_decision_no_update BEFORE UPDATE ON alias_decision
BEGIN SELECT RAISE(ABORT, 'journal immuable : alias_decision ne se modifie pas'); END;
CREATE TRIGGER alias_decision_no_delete BEFORE DELETE ON alias_decision
BEGIN SELECT RAISE(ABORT, 'journal immuable : alias_decision ne se supprime pas'); END;
