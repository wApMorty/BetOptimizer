# Phase 12 — Bonus de bienvenue couverts (`edge bonus`, onglet Bonus, page `/bonus`)

Ce qui a été construit : un module `crates/core/src/bonus/` qui calcule les mises de couverture d'un bonus de bienvenue, cherche sur le
dernier relevé les matchs où le gain garanti est le plus haut, et tient le suivi des offres dans un journal immuable (migration 13).
Une offre consommée n'est plus jamais proposée et son opérateur devient un compte de couverture. Le même tableau de bord
(`tracking::board`) alimente trois vues :
- la commande `edge bonus` (status, pick, offer, cover, log, settle) ;
- l'onglet Bonus de l'application, où se font les saisies ;
- la page `/bonus` du site, en lecture seule.

La stratégie et ses mesures sont dans `docs/bonus-bienvenue.md` ; les vérifications sont dans `docs/sources.md` §31.

Découpage :
- `cover.rs` : la formule, sans base ;
- `pick.rs` : le choix du match ;
- `tracking.rs` : les états, les règles et le règlement ;
- `store/bonus.rs` : l'écriture et la lecture ;
- `report/bonus_page.rs` : la page du site.

## 1. Concepts Rust rencontrés

**Un `enum` à données pour un invariant.** `crates/core/src/store/bonus.rs:20`. `Eligibility::Eligible { kind, amount, min_odds }` ou
`Eligibility::Ineligible`. Avec des champs `Option`, on pourrait écrire une offre « inéligible de 100 € » ; avec l'énumération, le
compilateur l'interdit. La base vérifie la même règle (`CHECK` de la migration 13) : deux gardes, une en Rust, une en SQL.

**Motif de tranche avec `let … else`.** `crates/core/src/bonus/tracking.rs:424`. `let [bonus_leg] = bonus_legs.as_slice() else { … }`
réussit seulement si la tranche a **exactement** un élément, et le nomme du même coup. Même idée dans `crates/cli/src/bonus.rs:489`
(`let [outcome, odds, stake] = parts.as_slice()`) pour lire `A:3.10:100` : un nombre de morceaux faux tombe dans le `else`.

**Une transaction, tout ou rien.** `crates/core/src/store/bonus.rs:291`. `conn.transaction()` demande `&mut Connection` : pendant
l'enregistrement, personne d'autre ne peut utiliser la connexion. Si un `INSERT` de pari échoue (deux mises sur une même issue), le `?`
quitte la fonction avant `tx.commit()` (ligne 326). La transaction est alors détruite, ce qui l'annule : aucune opération à moitié écrite.
Le test `the_database_refuses_two_bonus_legs_or_two_stakes_on_one_outcome` le vérifie.

**`pub(crate)` pour partager sans exposer.** `crates/core/src/betting/live.rs:112`. `latest_quotes` et `BookQuotes` (ligne 81) servent
à `scan` et au choix des bonus, mais restent invisibles hors de `edge-core`. Le refactor a extrait la lecture du dernier relevé sans
changer `scan` : ses tests sont restés verts.

**`into_iter()` contre `iter()` : déplacer plutôt que copier.** `crates/core/src/bonus/pick.rs:135`. `covers.into_iter()` consomme le
vecteur : chaque `bookmaker` (un `String`) **passe** dans son `Leg` sans `clone()`. `zip(&result.stakes)` emprunte les mises, qui
restent dans `result`, déplacé ensuite dans le `Candidate`.

**Une petite structure plutôt qu'un tuple.** `pick.rs:163`. Clippy a refusé
`Option<Vec<(String, Outcome, Odds, DateTime<Utc>)>>` (lint `type_complexity`). `CoverQuote` nomme ses champs :
`quote.odds` se lit mieux que `.2`.

**`iter_mut()` puis `zip` : modifier en parcourant.** `crates/gui/src/forms.rs:217`. On réécrit la mise de chaque couverture du
formulaire. `iter_mut().filter(…)` donne des `&mut BonusLegForm` ; le compilateur garantit qu'aucune autre référence au vecteur
n'existe pendant la boucle.

**`Option<Result<…>>` vers `Result<Option<…>>` : `transpose`.** `crates/cli/src/bonus.rs:227`.
`options.kind.as_deref().map(parse_kind)` donne une `Option<Result<OfferKind>>`. `transpose()` la retourne en
`Result<Option<OfferKind>>`, et le `?` propage l'erreur d'une forme mal tapée tout en gardant « absent » comme valeur possible.

**`matches!`.** `tracking.rs:233`. `matches!(s.state, OfferState::Ineligible | OfferState::Consumed)` est un `match` qui rend un booléen :
plus court qu'un `match` à deux bras, et toujours vérifié par le compilateur (un nom de variante faux ne compile pas).

**Une somme vide de flottants vaut −0,0.** `tracking.rs:225`. `Iterator::sum` sur des `f64` part de `−0,0` (l'élément neutre exact de
l'addition flottante). Une somme vide s'affichait donc « −0,00 € ». `fold(0.0, …)` part de `+0,0`. C'est un détail, mais il est visible.

**Index partiel SQLite.** `crates/core/src/store/migrations/0013_bonus.sql:55`.
`CREATE UNIQUE INDEX … ON bonus_bet (operation_id) WHERE role = 'bonus'` impose un seul pari bonus par opération, sans contraindre
les couvertures.

## 2. Concepts statistiques et financiers

**Une seule formule pour trois offres.** On mise `A` à la cote `o` avec le bonus, puis `c_i` en argent sur chaque autre issue `i`. Gain
du bonus s'il passe : `W = A·(o − 1)`. Valeur du bonus s'il perd : `L = 0` (remboursé en argent, ou freebet), ou `L = −A·(1 − g)`
(remboursé en freebet que l'on convertira à `g` par euro). On égalise `W − Σc` et `L + c_i·o_i − Σc` :

`c_i = (W − L) / o_i`, gain garanti `= W − (W − L)·Σ 1/o_i` (`crates/core/src/bonus/cover.rs:38`).

Intuition : `Σ 1/o_i` est la part de la mise de couverture que les opérateurs gardent en marge. Plus elle approche 1, moins il reste.

**Pourquoi pas de grosse cote quand on couvre.** Une cote haute sur le bonus veut dire que l'issue contraire est le favori. Il faut alors
le couvrir lourdement (`c_i` grand), et chaque euro de couverture paie la marge française. Mesure : 22 € garantis pour 243 € sortis à la
cote 2–3, mais −21 € au-delà de 10 (`docs/bonus-bienvenue.md` §6).

**Espérance contre gain garanti.** Sans couverture, un freebet vaut en moyenne `F·(r − p)` (environ 0,70 à la cote 5). Couvert, il rend
avec certitude 0,53 à 0,56. La différence est le prix de la certitude. Pour un premier pari remboursé en freebet, le gain « garanti »
suppose en plus que le freebet reçu sera converti à `g` au moins : `bonus_freebet_value` (0,53) est donc pris au bas de la fourchette.

**L'arrondi des mises.** Les mises de couverture sont arrondies au 0,10 € **le plus proche** (`domain.rs:32`) et non à la baisse : on
cherche l'égalité des gains, pas un plafond. Le gain garanti se recalcule sur les mises arrondies ; l'écart entre issues reste de quelques
centimes.

## 3. Trois questions pour vérifier

1. Tu as enregistré Betclic (remboursé en freebet) et Winamax (inéligible). Tu enregistres une opération Betclic couverte chez PMU, dont
   l'offre est disponible. Que répond `edge bonus log`, et pourquoi ce refus te protège-t-il ?
2. Avec `Σ 1/o_i = 0,80` et un freebet de 100 € à la cote 4, quel est le gain garanti ? Et pour un premier pari de 100 € remboursé en
   freebet, à la même cote, avec `g = 0,53` ?
3. Pourquoi `Board.realized` ne compte-t-il pas le freebet reçu après un premier pari perdu, et où ce freebet apparaît-il ensuite ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/bonus/cover.rs` : la formule, ses trois formes et ses tests (fermeture, marché juste, freebet perdu sans coût).
2. `crates/core/src/bonus/tracking.rs` : les états d'une offre, les comptes de couverture et les refus de `check_operation` : c'est
   là que se trouvent les règles qui évitent de gâcher une offre.
3. `crates/core/src/store/migrations/0013_bonus.sql` : le journal immuable, les contraintes en double avec Rust, et l'index partiel.
