# Phase 0 — Fondations

Ce qui a été construit : un workspace Cargo (`edge-core` + `edge-cli`), la configuration TOML avec une clé API
protégée, les chemins standard, une base SQLite avec un moteur de migrations, et la commande `edge init`.
Aucune statistique n'intervient à cette phase.

## 1. Concepts Rust rencontrés

**`Result` et l'opérateur `?`** — `crates/core/src/paths.rs:25`, `crates/core/src/store/migrate.rs:43`.
Une fonction qui peut échouer renvoie `Result<T, E>`. Le `?` placé après un appel signifie : « si c'est une `Err`,
quitte la fonction en renvoyant cette erreur ; sinon, donne-moi la valeur ». Il remplace les `try/catch`. Les
alias `Result<T>` de `crates/core/src/error.rs:93` évitent de réécrire le type d'erreur partout.

**`Option` → `Result` avec `ok_or`** — `crates/core/src/paths.rs:25`, `crates/core/src/config.rs:135`.
Une `Option` dit « il y a une valeur ou non » ; `ok_or(erreur)` la transforme en `Result` pour pouvoir utiliser `?`.

**`impl From` et `#[from]`** — `crates/core/src/error.rs:45`.
Le `?` convertit l'erreur de l'appel dans le type d'erreur de la fonction, grâce au trait `From`. `#[from]`
(crate `thiserror`) écrit cette conversion pour nous : c'est pourquoi on peut écrire `?` sur un résultat
`rusqlite` dans une fonction qui renvoie notre `Error`.

**Newtype** — `crates/core/src/config.rs:30` et `:45`.
`struct ApiKey(String)` enveloppe un `String` dans un type distinct. Pour le compilateur, ce n'est plus un
`String` : on ne peut pas le mélanger avec un autre texte. Ici, on a aussi écrit `Debug` à la main pour qu'il
n'affiche jamais la valeur. Même pattern en phase 1 pour `Odds` (une cote n'est pas une probabilité).

**`match` avec garde, `match` exhaustif** — `crates/core/src/config.rs:113`, `crates/cli/src/main.rs:43`.
La garde (`Err(e) if e.kind() == …`) distingue « fichier absent » (normal) des autres erreurs. Un `match` doit
couvrir tous les cas : ajouter une sous-commande sans la traiter ne compile plus.

**Possession et emprunt** — `crates/core/src/config.rs:135`, `crates/core/src/error.rs:84`.
Une fonction qui prend `&Path` emprunte sans posséder. `clone()` (config.rs:135) crée une copie quand on doit
rendre une valeur indépendante de l'original. `to_path_buf()` (error.rs:86) n'est appelé qu'en cas d'erreur :
on ne paie la copie que si elle sert.

**Fermeture retournée (`impl FnOnce`)** — `crates/core/src/error.rs:84`.
`Error::io(chemin)` renvoie une petite fonction qui sait fabriquer l'erreur pour ce chemin. On l'utilise avec
`map_err(Error::io(chemin))?` : « si ça échoue, ajoute le chemin au message ».

**Accès exclusif (`&mut`) et transaction** — `crates/core/src/store/migrate.rs:24` et `:43-49`.
Ouvrir une transaction demande `&mut Connection` : le compilateur interdit tout autre usage de la connexion
pendant ce temps. Si on sort avant `commit()` (y compris par un `?`), la transaction est détruite et SQLite
annule tout : la destruction automatique d'une valeur (`Drop`) fait le ménage sans code explicite.

**Itérateurs** — `crates/core/src/store/migrate.rs:42`.
`(1..=known).zip(migrations).skip(applied)` enchaîne trois adaptateurs : numéroter, associer chaque numéro à sa
migration, sauter celles déjà appliquées. Rien n'est calculé avant que la boucle `for` ne consomme l'itérateur.

**Compilation conditionnelle `#[cfg(unix)]`** — `crates/core/src/config.rs:141` et `:161`, `crates/core/src/init.rs:47`.
Deux versions d'une fonction, une par famille de systèmes : le cœur doit aussi compiler sous Windows, où les
permissions Unix n'existent pas.

**`include_str!`** — `crates/core/src/config.rs:21`.
Insère le contenu d'un fichier dans le binaire à la compilation. `edge.example.toml` sert ainsi à la fois de
documentation, de modèle écrit par `edge init`, et de référence testée contre les valeurs par défaut du code.

**Lints du workspace** — `Cargo.toml:29` et `:32`.
`unsafe_code = "forbid"` et `unwrap_used = "warn"` (promu en erreur par `-D warnings`) font appliquer nos règles
par le compilateur plutôt que par la relecture.

## 2. Concepts statistiques

Aucun à cette phase. Le premier concept statistique (retrait de la marge des cotes) arrive en phase 2.

## 3. Trois questions pour vérifier que tu as compris

1. Pourquoi `Config::api_key` reçoit-elle la valeur de l'environnement en paramètre (`from_env: Option<&str>`)
   au lieu d'appeler elle-même `std::env::var` ? Qu'est-ce que cela change pour les tests ?
2. Si la migration 2 échoue alors que la 1 a réussi, dans quel état est la base ? Quel mécanisme du langage
   garantit l'annulation de la migration 2 alors que le code n'appelle jamais `rollback()` ?
3. Pourquoi `ApiKey` n'est-elle pas un simple `String`, et qu'est-ce qui empêche `println!("{config:?}")` de
   divulguer la clé ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/store/migrate.rs` — le plus dense : transaction, itérateur, accès exclusif. Toute l'archive
   des cotes reposera sur ce mécanisme.
2. `crates/core/src/config.rs` — newtype, `match` à garde, `clone`, compilation conditionnelle : la plupart des
   idées du projet y apparaissent en petit.
3. `crates/core/src/error.rs` — le modèle d'erreurs qui sera reproduit dans toutes les phases.
