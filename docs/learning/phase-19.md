# Phase 19 — Repli OddsPapi des séances Loto Foot

Ce qui a été construit : quand notre archive The Odds API n'a pas la cote Pinnacle d'une rencontre (coupe nationale, compétition non
couverte), même après le relevé des compétitions manquantes, la séance la cherche chez OddsPapi, qui relaie aussi Pinnacle. Coût : une
requête par séance (`fixtures`), l'historique étant gratuit. Origine : le LF7 n°135 du 2026-10-29, écarté pour Benfica – Gil Vicente
(Taça da Liga). Sources et choix : `docs/sources.md` §62.

| Fichier | Rôle |
|---|---|
| `core/src/lotofoot/oddspapi.rs` | recherche du match, appariement par alias exacts, dernier prix Pinnacle, pistes |
| `core/src/store/oddspapi.rs`, `store/migrations/0021_lotofoot_oddspapi.sql` | archive immuable des relevés |
| `core/src/lotofoot/odds.rs` | provenance d'une cote (`Source`) |
| `core/src/lotofoot/session.rs` | repli après The Odds API, provenance figée au journal |
| `core/src/lotofoot/scores.rs` | suivi en direct : coup d'envoi lu chez OddsPapi, absence de code ESPN signalée |

## 1. Concepts Rust rencontrés

**`enum` à données et `match` exhaustif.** `crates/core/src/lotofoot/odds.rs:66`.
- `Source::OddsApi { api_call_id, sport_key }` et `Source::OddsPapi { quote_id, last_change_at }` : chaque variante porte ses propres
  champs. On ne peut lire ces champs qu'après avoir dit de quelle variante il s'agit (`match`, `if let`, `let … else`).
- Le compilateur refuse un `match` qui oublie une variante : le jour où une troisième source s'ajoute, chaque lecture de la
  provenance (journal, suivi en direct) sera signalée à compléter. Avec deux champs optionnels dans une struct, rien ne l'aurait dit.

**`let … else` pour sortir tôt.** `crates/core/src/lotofoot/oddspapi.rs:286`.
- `let Some((quote_id, quote)) = … else { return Ok(None); };` : si le motif ne correspond pas, le bloc `else` doit quitter la
  fonction. Après la ligne, `quote` est une valeur ordinaire, sans niveau d'imbrication de plus.

**Motif sur une tranche : zéro, un ou plusieurs.** `crates/core/src/lotofoot/oddspapi.rs:118`.
- `match matching.as_slice() { [fixture] => …, [] => …, several => … }` distingue exactement un match (retenu), aucun (refus et
  pistes) et plusieurs (refus) : l'ambiguïté ne peut pas passer pour un succès.

**Durée de vie d'un temporaire.** `crates/core/src/lotofoot/live.rs:87`.
- `Access` garde des références (`&mut`) vers l'appel réseau et la pause. Écrire `pause: &mut std::thread::sleep` dans l'expression
  crée une valeur temporaire, détruite à la fin de l'instruction, alors que `Access` sert ensuite pendant toute la séance : le
  compilateur refuse (« cannot return value referencing temporary value »). D'où `let mut sleep = std::thread::sleep;` avant, qui vit
  jusqu'à la fin du bloc.

**Une fermeture qui emprunte et lit la même variable qu'un autre emprunt partagé.** `crates/core/src/lotofoot/live.rs:81`.
- `papi_get` emprunte `papi` en lecture, et `papi.is_some()` aussi : plusieurs emprunts partagés (`&`) coexistent sans problème ;
  seul un emprunt exclusif (`&mut`) l'interdirait.

**`&mut` réutilisé plusieurs fois : le réemprunt implicite.** `crates/core/src/lotofoot/oddspapi.rs:87`.
- `fetch` et `pause` sont des `&mut dyn FnMut`. On les passe successivement à `read_usage`, `upcoming` et `download` : à chaque appel,
  Rust prête un nouvel emprunt `&mut *fetch` qui finit avec l'appel, si bien que la même référence resservira ensuite.

## 2. Concepts statistiques

**Dernier prix connu, jamais un prix futur.** `crates/core/src/lotofoot/oddspapi.rs:239`.
- L'historique OddsPapi liste chaque changement de prix. La cote retenue pour l'issue k est celle du dernier changement antérieur au
  relevé : p_k = prix(max { t_i : t_i ≤ relevé }). Un changement postérieur (même déjà publié) est ignoré, et la séance ne lit que
  les relevés faits avant elle (invariant 1).

**Même retrait de marge pour les deux sources.** `crates/core/src/market/margin.rs:48`.
- Les cotes OddsPapi passent par la méthode configurée (`margin_method`, puissance par défaut) : la somme des probabilités vaut 1,
  et une cote venue d'OddsPapi se compare à une cote venue de The Odds API.

## 3. Questions pour vérifier

1. Pourquoi les relevés OddsPapi du Loto Foot ne sont-ils pas écrits dans `oddspapi_fixture`, la table du test de latence ?
2. Une rencontre a deux matchs OddsPapi possibles dans la fenêtre (aller et retour d'une coupe). Que fait la séance, et pourquoi ne
   choisit-elle pas le plus proche ?
3. Dans le journal, `odds_calls` vaut `[12, 12, null]` et `oddspapi_quotes` `[null, null, 5]`. D'où vient la cote de chaque
   rencontre, et que montre le suivi en direct pour la troisième ?

## 4. Fichiers à relire en priorité

1. `crates/core/src/lotofoot/oddspapi.rs` : tout le repli tient dans ce fichier (recherche, appariement, dernier prix, pistes).
2. `crates/core/src/lotofoot/session.rs:342` : l'ordre des sources (archive The Odds API, puis OddsPapi) et le moment du repli.
3. `crates/core/src/store/migrations/0021_lotofoot_oddspapi.sql` : ce qui est archivé, et pourquoi c'est immuable.
