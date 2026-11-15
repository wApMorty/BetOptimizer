# Phase 23 — Alias d'équipes déduits par la rencontre, validés d'un clic

Ce qui a été construit : quand une grille Loto Foot (section `LOTOFOOT`) ou une page ESPN (section `ESPN`) porte un nom inconnu, edge
cherche le match relevé qui correspond à la rencontre et **propose** l'alias, avec ce match pour preuve. Paul valide ou refuse d'un
clic sur `/lotofoot` ; un alias validé sert aussitôt, sans recompiler, et `edge aliases export` le reporte ensuite dans
`config/aliases.toml`. Une migration (26). Mesure rétrospective, choix et écarts au plan : `docs/sources.md` §75. Rien n'est déployé.

| Fichier | Rôle |
|---|---|
| `core/src/lotofoot/alias_deduction.rs` | la déduction, sans base ni réseau : `from_grid` (grilles), `from_espn` (suivi en direct) |
| `core/src/store/alias_proposals.rs`, migration 26 | propositions et décisions, en insertion seule ; `pending`, `accepted`, `decide` |
| `core/src/ingest/aliases.rs` | `with_learned` (table versionnée + alias acceptés), `export_learned` (report dans le TOML) |
| `core/src/lotofoot/odds.rs` | candidats : `archived_matches` (archive), `listed_matches` (`/events` gratuits) |
| `core/src/lotofoot/session.rs`, `scores.rs` | branchement dans la séance et dans le relevé ESPN |
| `core/src/web/` (`pages/lotofoot.rs`, `actions.rs`, `mod.rs`, gabarit) | bloc « Alias à valider », route `/lotofoot/alias` |
| `cli/src/aliases.rs` | `edge aliases list`, `edge aliases export --to … [--dry-run]` |

## 1. Concepts Rust rencontrés

**`map` sur un tableau de taille fixe.** `crates/core/src/lotofoot/alias_deduction.rs:106`.
- `let known = ours.map(|name| aliases.resolve(LOTOFOOT, name));` : `ours` est un `[&str; 2]` (domicile, extérieur) ; `map` rend un
  `[Option<&str>; 2]`, **toujours de deux éléments**, sans passer par un `Vec`. La taille fait partie du type : le compilateur sait que
  `known[side]` existe pour `side` dans `0..2`.
- Contrairement à `iter().map(…)`, qui est paresseux, `map` sur un tableau calcule tout de suite et rend un tableau.
- Même idée à `:203` : `known == [None, None]` compare deux tableaux élément par élément, aussi lisible qu'en Python.

**`|=` et `&=` sur des booléens.** `crates/core/src/lotofoot/alias_deduction.rs:132` et `:198`.
- `anchored |= theirs[side] == ours[side]` équivaut à `anchored = anchored || (…)`, et `fits &= …` à `fits = fits && …`. La boucle
  sur les deux places accumule ainsi « au moins une place ancrée » et « toutes les places respectées », sans `if` imbriqués.
- Différence avec `||` : `|=` évalue toujours le membre de droite (pas de court-circuit). Ici c'est sans conséquence : une comparaison de
  textes, sans effet de bord.

**`Option` comparée à une valeur : `contains(&Some(…))`.** `crates/core/src/lotofoot/alias_deduction.rs:141`.
- `known.contains(&Some(canonical.as_str()))` demande si l'une des places connues désigne déjà l'équipe proposée. `Option<&str>`
  implémente l'égalité : `Some("Fulham") == Some("Fulham")`, et `None` n'égale aucun `Some`. Pas besoin de `match` ni de `if let`.
- `as_str()` est nécessaire : `canonical` est un `String`, `known` contient des `&str` ; on compare des `Option<&str>` entre eux.

**Une transaction `rusqlite` s'utilise comme une connexion.** `crates/core/src/store/alias_proposals.rs:175` et `:190`.
- `conn.transaction()` demande `&mut Connection` : pendant la transaction, personne d'autre ne peut utiliser la connexion (le
  compilateur l'interdit, comme pour les migrations, phase 0).
- `Transaction` implémente `Deref<Target = Connection>` : `get(&tx, id)` et `bundled.with_learned(&tx)` acceptent `&tx` là où ils
  attendent `&Connection`. Le compilateur insère la conversion (« déréférencement automatique »).
- Sans `tx.commit()`, la transaction est annulée quand `tx` sort de portée (`Drop`) : un `return Err(…)` anticipé n'écrit rien.

**Une structure intermédiaire pour traverser la fermeture de `rusqlite`.** `crates/core/src/store/alias_proposals.rs:75` et `:93`.
- La fermeture passée à `query_map` doit rendre un `rusqlite::Result` : elle ne peut pas porter une erreur d'edge, comme une date
  illisible ou une source inconnue. `raw_row` ne fait donc que **lire** (des `String`), et `stored` **convertit** ensuite, avec `?` et
  les erreurs d'edge.
- `stored(raw: RawRow)` prend sa ligne **par valeur** : `section: raw.section` déplace le texte dans la proposition, sans `clone()`.
  On peut déplacer un champ après l'autre parce que `raw` n'est plus utilisée ensuite.

**`HashMap::entry` : insérer seulement si absent.** `crates/core/src/ingest/aliases.rs:76` et `:81`.
- `self.by_div.entry(section.to_string()).or_default()` rend la table de la section, créée vide si elle n'existait pas : une seule
  recherche dans la `HashMap`, et un `&mut` utilisable aussitôt.
- `names.entry(team.clone()).or_insert_with(|| team.clone())` : la fermeture n'est appelée que si la clé manque. `or_insert(…)`
  calculerait la valeur même quand elle ne sert pas. Les deux `clone()` sont nécessaires : la clé et la valeur sont deux `String`
  distinctes, et `team` sert encore après.

**`&self` qui rend un `Self` neuf.** `crates/core/src/ingest/aliases.rs:63`.
- `with_learned(&self, conn)` emprunte la table versionnée et en rend une copie complétée. L'application garde ainsi la table d'origine
  (`App::aliases`) et en tire une table complète **à chaque page**, car un alias peut être accepté entre deux pages. Avec `self` par
  valeur, l'application aurait perdu sa table au premier appel.
- Le `clone()` de la `HashMap` (quelques centaines de noms) coûte bien moins qu'une page Web ; c'est le prix de cette simplicité.

**Deux modules qui se citent l'un l'autre.** `crates/core/src/ingest/aliases.rs:13` et `crates/core/src/store/alias_proposals.rs:13`.
- `ingest::aliases` appelle `store::alias_proposals::accepted`, et `store::alias_proposals` reçoit un `Aliases`. En Rust, les modules
  d'une même crate peuvent se référencer mutuellement : contrairement aux imports Python, il n'y a pas d'ordre de chargement, le
  compilateur voit la crate entière.
- Entre deux **crates**, ce serait interdit (Cargo refuse les dépendances circulaires). C'est l'une des raisons de garder `core` en une
  seule crate.

**L'emprunt d'une fermeture dure jusqu'à sa dernière utilisation.** `crates/core/src/lotofoot/session.rs:1632` et `:1709`.
- La fermeture `fetch` modifie `calls` (`calls.push(…)`) : elle l'emprunte en écriture. Tant que `fetch` sert encore, lire `calls`
  est refusé (« cannot borrow `calls` as immutable because it is also borrowed as mutable »).
- L'emprunt ne dure pas jusqu'à la fin du bloc, mais jusqu'à la **dernière utilisation** de `fetch` (durées de vie non lexicales).
  Déplacer la lecture de `calls` après le dernier `run(…)` suffit, sans `drop(fetch)` explicite.

Rappels de phases précédentes : `INSERT OR IGNORE` et le nombre de lignes écrites (`alias_proposals.rs:50`, phase 1) ; `let … else`
(`aliases.rs`, phase 14) ; une `enum` avec `code` et `from_code` pour la base (`SeenSource`, `alias_deduction.rs:23`, phases 14 et 21).

## 2. Concepts statistiques et de méthode

**Identifier par les contraintes, pas par la ressemblance.** `crates/core/src/lotofoot/alias_deduction.rs:98` à `:160`.
- Une rencontre se reconnaît à ses deux places. Avec une place **ancrée** (équipe connue, ou nom identique chez les deux sources), la
  rencontre est une contrainte : parmi les matchs relevés dans la fenêtre, seuls ceux qui la respectent conviennent. Le nom inconnu
  est alors **lu** chez la source, pas deviné.
- Une ressemblance de noms (« Malaga » et « Málaga ») se trompe en silence : Inter et Inter Miami, Rangers et Queens Park Rangers.
  La contrainte se trompe seulement si deux matchs la respectent, et alors elle le **voit** : plusieurs propositions, que Paul départage.
- Mesure rétrospective (§75) : 17 noms proposés sur 22, **17 justes**. Sur un si petit échantillon, cela ne prouve pas que les erreurs
  sont impossibles ; c'est pourquoi rien n'entre en vigueur sans clic.

**Pourquoi l'ambiguïté vient de la fenêtre.** `crates/core/src/lotofoot/session.rs:222`.
- Les rencontres d'une grille se jouent entre la clôture moins 2 h et la clôture plus 5 jours. En cinq jours, une équipe joue souvent
  deux fois (championnat et coupe d'Europe) : Lille a reçu deux candidats pour le LF7 n°137. Rétrécir la fenêtre réduirait l'ambiguïté
  mais perdrait des rencontres réelles.
- Côté ESPN, le coup d'envoi est connu à la minute : d'où la règle du match unique à l'heure exacte (`alias_deduction.rs:203`), qui ne
  vaut rien en multiplex (plusieurs matchs à la même heure) et le reconnaît.

**Aucune fuite du futur, même pour un alias.** `crates/core/src/lotofoot/odds.rs:407` ; `crates/core/src/lotofoot/session.rs:413`.
- `archived_matches(conn, window, observed_by)` ne lit que les relevés faits **avant la séance** (`called_at <= observed_by`). Le
  coup d'envoi, lui, est dans le futur : c'est le match à venir que l'on cherche. Ce qui compte pour l'invariant 1, c'est **quand
  l'information a été connue**, pas quand l'événement a lieu.

## 3. Questions pour vérifier

1. La grille porte `Malaga – Fulham`, « Malaga » inconnu. L'archive contient `Málaga – Fulham` (The Odds API, samedi) et
   `Fulham – Everton` (mardi), et OddsPapi `Malaga CF – Fulham` (samedi). Quelles propositions `from_grid` rend-elle, et pourquoi pas
   celle d'OddsPapi ? Que rend-elle si l'archive The Odds API est vide ?
2. Paul valide « Bodo Glimt » → « Bodø/Glimt ». Pourquoi la proposition « Bodo Glimt » → « Rennes » disparaît-elle de la page sans être
   refusée, et que répond `decide` si on essaie quand même de l'accepter ? Dans quelle table, et par quelle requête, la décision est-elle
   écrite ; que se passe-t-il au second clic sur le même bouton ?
3. `with_learned` prend `&self` et clone la table. Que deviendrait l'application si la fonction prenait `self` ? Et pourquoi un alias appris
   qui contredit la table versionnée est-il ignoré (avec un avertissement) plutôt que de faire échouer le chargement ?

## 4. Fichiers à relire en priorité

1. `crates/core/src/lotofoot/alias_deduction.rs` : toute la règle tient dans `from_grid` et `from_espn`, et leurs tests (Bodø/Glimt,
   Montpellier – Red Star, multiplex) disent ce qui est garanti. C'est le seul endroit où une erreur ferait proposer un faux alias.
2. `crates/core/src/store/alias_proposals.rs` avec `store/migrations/0026_team_alias.sql` : comment une décision définitive s'écrit en
   insertion seule, et pourquoi `decide` vérifie dans une transaction.
3. `crates/core/src/ingest/aliases.rs`, `with_learned` puis `export_learned` : la frontière entre l'appris (en base) et le versionné (le
   TOML embarqué), et le contrôle du fichier par relecture avant écriture.
