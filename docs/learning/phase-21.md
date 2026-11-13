# Phase 21 — Correctifs de l'audit du 2026-11-12

Ce qui a été construit : les correctifs de la première semaine du plan d'action de `docs/audit-2026-11-12.md`. La règle « prix contre
Pinnacle » n'examine plus un match à plus de 72 h et écarte une référence trop large ; une page de prono-match figée se détecte, s'annonce
et se voit ; une unité systemd en échec envoie une alerte ; la base se sauvegarde. Mesures et choix : `docs/sources.md` §73.

| Fichier | Rôle |
|---|---|
| `core/src/betting/live.rs` | horizon de 72 h, garde de surronde de la référence, compteurs `too_far` et `wide_reference` |
| `core/src/betting/recommend.rs` | version de la règle (`pvp-2`) écrite dans `code_version` |
| `core/src/store/alerts.rs`, migration 23 | journal des alertes : ne pas répéter une alerte à chaque passage d'un timer |
| `core/src/lotofoot/fetch.rs` | page périmée : grille close encore affichée « en cours » |
| `core/src/store/lotofoot.rs` (`page_health`) | dernière lecture réussie et erreur en cours d'une page |
| `core/src/web/pages/mod.rs`, `pages/lotofoot.rs` | pastille « Loto Foot » de l'accueil, bandeau « Source en défaut » |
| `core/src/notify/message.rs` | texte de l'alerte de panne, contrôle du nom d'unité |
| `core/src/store/backup.rs` | copie datée et vérifiée de la base, rotation |
| `cli/src/commands.rs`, `cli/src/lotofoot.rs` | `edge notify-failure`, `edge backup`, envoi de l'alerte « source figée » |
| `deploy/systemd/` | `OnFailure=` sur sept services, unité modèle `edge-notify-failure@.service`, `edge-backup` |

## 1. Concepts Rust rencontrés

**`Option::take_if` : retirer une valeur si elle remplit une condition.** `crates/core/src/lotofoot/fetch.rs:166`.
- `page.open.take_if(|grid| grid.closes_at <= at)` : si la grille affichée est close, elle sort de `page` (qui garde `None`) et nous est
  remise ; sinon rien ne bouge. Il faut `mut page` : on modifie la page reçue de l'analyseur avant de la passer au store, qui n'écrit
  donc ni la grille ni sa répartition. Sans cette méthode, il aurait fallu un `if`, un `take()` et une variable de plus.

**`match` sur un couple.** `crates/core/src/lotofoot/fetch.rs:172`.
- `match (stale, closed)` traite d'un coup les trois cas (périmée, close depuis peu, rien à signaler). Le compilateur vérifie que tous
  les couples possibles sont couverts : un quatrième cas oublié ne compilerait pas.

**`optional()` et `transpose()` : une ligne qui peut manquer, une date qui peut être illisible.** `crates/core/src/store/alerts.rs:60` et `:63`.
- `query_row` échoue quand la requête ne rend aucune ligne ; `.optional()` (trait `OptionalExtension` de `rusqlite`) change cette
  erreur-là en `None`. On a alors un `Option<String>` à lire comme une date, opération qui peut elle-même échouer : `map` donnerait
  un `Option<Result<…>>`, que `?` ne sait pas propager. `transpose()` le retourne en `Result<Option<…>>`.

**Deux `Option` emboîtées et `flatten`.** `crates/core/src/store/lotofoot.rs:98`.
- La lecture la plus récente d'une page peut ne pas exister (`optional()` : premier `Option`), et sa colonne `error` peut être NULL
  (second `Option`). Les deux veulent dire « pas d'erreur en cours » : `flatten()` les réunit en un seul.

**`.ok()` : continuer sans ce qui a échoué.** `crates/cli/src/commands.rs:1323`.
- `open_database(paths).ok()` change un `Result` en `Option` et abandonne l'erreur. C'est voulu ici, et commenté : une alerte de
  panne ne doit pas dépendre de la base, qui est peut-être ce qui est en panne. Partout ailleurs le projet propage l'erreur avec `?`.

**`if let … && …` (rappel).** `crates/cli/src/commands.rs:1327`.
- Depuis l'édition 2024, un `if let` peut enchaîner une seconde condition : « la base est ouverte ET l'alerte n'est pas due ». Clippy
  refuse la version à deux `if` imbriqués.

**Un bloc pour fermer une connexion plus tôt.** `crates/core/src/store/backup.rs:119`.
- `let wanted = { let source = …; …; version };` : `source` appartient au bloc et est détruite à son accolade fermante, ce qui ferme
  la connexion. Le bloc rend seulement le nombre dont on a besoin ensuite. C'est la règle de possession qui fait le ménage, pas un
  appel à `close()`.

**`let _ = …` : ignorer un résultat, exprès.** `crates/core/src/store/backup.rs:86`.
- La suppression du fichier partiel peut échouer, et le compilateur signale tout `Result` laissé sans suite. `let _ =` dit « je l'ai vu,
  je n'en fais rien » : l'erreur à rapporter est celle de la copie, pas celle du ménage.

**`#[cfg(unix)]` et permissions à la création (rappel).** `crates/core/src/store/backup.rs:111`.
- `options.mode(0o600)` n'existe que sous Unix : le bloc est compilé sous Linux et ignoré sous Windows, le cœur compile des deux côtés.
  Le fichier naît illisible pour les autres comptes, au lieu d'être restreint après coup.

**`sort_by_key` avec `Reverse`.** `crates/core/src/store/backup.rs:166`.
- Trier « du plus récent au plus ancien » : la clé est la date, enveloppée dans `std::cmp::Reverse` qui inverse l'ordre. Clippy préfère
  cette forme à une fermeture de comparaison écrite à la main.

**`Copy` et `*shown`.** `crates/core/src/lotofoot/fetch.rs:39` et `crates/cli/src/lotofoot.rs:79`.
- `StaleGrid` tient en deux petites valeurs et dérive `Copy` : `*shown` en fait une copie au lieu d'un emprunt du bilan, et la liste
  `fresh` peut vivre sans lui. Pour un type qui possède une `String`, il faudrait un `clone()` explicite.

**`as_deref()` sur un `Option<PathBuf>`.** `crates/cli/src/commands.rs:1280`.
- `config.backup_dir.as_deref()` prête le chemin (`Option<&Path>`) sans le copier ; `to.or(…)` choisit `--to` s'il est donné.

## 2. Concepts statistiques

**Une référence n'en est une que là où elle est précise.** `crates/core/src/betting/live.rs:43` et `:49`.
- L'avantage annoncé vaut `p̂ · cote − 1`, où `p̂` est la probabilité de Pinnacle sans marge. Une erreur `ε` sur `p̂` devient une erreur
  `cote · ε` sur l'avantage : à une cote de 3, un point de probabilité fait trois points d'avantage. Or `p̂` bouge de 0,94 point par jour
  en médiane à moins de trois jours, et la surronde passe de 1,03 la veille à 1,05 au-delà de sept jours (plus de marge à retirer, donc
  plus d'arbitraire dans le retrait). Un seuil de 1 % est plus petit que ce bruit.

**Le maximum d'écarts bruités sélectionne le bruit.** `crates/core/src/betting/live.rs:227`.
- Écart observé = écart vrai + erreur. La règle garde, par match, le plus grand écart parmi une douzaine de prix : quand les écarts vrais
  sont proches de zéro, le plus grand est surtout celui dont l'erreur est la plus favorable, et `E[max] > max E`. C'est la « malédiction
  de l'optimiseur » (Smith & Winkler 2006, déjà citée au §69). Le +4,2 % de Deportivo – Barcelone en est un exemple : l'erreur était
  dans la référence. Réduire le bruit de la référence (horizon, surronde) réduit le biais ; cela ne crée pas d'avantage.

**Une garde n'est pas un seuil réglé.** `docs/sources.md` §73.
- 72 h et 1,06 ont été lus sur la description de l'archive (marge et stabilité de Pinnacle selon l'horizon), pas choisis pour maximiser
  un résultat de pari. L'invariant 8 demande de compter les configurations essayées : une seule. La mesure qui tranchera reste le CLV du
  journal, désormais relu par version de règle.

## 3. Questions pour vérifier

1. Le match Lens – Toulouse a été recommandé sous `pvp-1` à 15 jours. Quand il entrera dans les 72 h, sera-t-il recommandé de nouveau
   sous `pvp-2` ? Quelle contrainte de la base en décide ?
2. Pourquoi une page périmée n'est-elle pas comptée comme un échec du relevé (code de sortie nul), alors qu'elle écrit une erreur dans
   `lotofoot_fetch` ? Qu'est-ce qui te prévient, et combien de fois ?
3. `edge backup` refuse de créer son dossier de destination. Quel scénario cette règle évite-t-elle, et par quel chemin l'apprendras-tu
   s'il se produit ?

## 4. Fichiers à relire en priorité

1. `crates/core/src/betting/live.rs` : les deux gardes tiennent en dix lignes dans `scan`, et les constantes portent leurs mesures.
2. `crates/core/src/lotofoot/fetch.rs` : la détection de la page périmée, autour de `take_if`, et ses trois tests.
3. `crates/core/src/store/backup.rs` : court, linéaire, et il rassemble quatre des concepts ci-dessus (bloc de portée, `let _`,
   `cfg(unix)`, `Reverse`).
