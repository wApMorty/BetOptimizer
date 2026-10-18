# Phase 1 — Archive des cotes

Ce qui a été construit : un client HTTP écrit à la main pour The Odds API, une garde de budget fondée sur les
crédits réellement consommés, une archive de cotes en ajout seul (SQLite, triggers d'immuabilité), les commandes
`edge quota` et `edge snapshot`, et un timer systemd qui lance le relevé chaque jour à 14h (Paris).

## 1. Concepts Rust rencontrés

**Constructeur qui valide (newtype à invariant)** — `crates/core/src/domain.rs:16`.
Le champ de `Odds` est privé : la seule façon d'obtenir une cote est `Odds::new`, qui renvoie `None` si la valeur
est ≤ 1 ou non finie. Toute fonction qui reçoit une `Odds` sait donc qu'elle est valide, sans la revérifier. C'est
la suite du newtype de la phase 0 (`ApiKey`) : là, le type protégeait un secret ; ici, il protège une règle.

**Références dans une structure (durée de vie `'a`)** — `crates/core/src/store/archive.rs:35`.
`NewApiCall<'a>` contient des `&'a str` au lieu de `String` : on n'a pas à copier les textes. Le `'a` dit au
compilateur « cette structure ne vit pas plus longtemps que les textes qu'elle emprunte ». Elle n'existe que le
temps d'un appel de fonction, donc c'est sûr.

**Emprunt qui doit se terminer avant `commit()`** — `crates/core/src/store/archive.rs:101` et `:138`.
Les requêtes préparées empruntent la transaction ; `tx.commit()` la consomme. Le bloc `{ … }` détruit les
emprunts avant ce moment. Sans lui, le compilateur refuserait de compiler : on ne peut pas consommer ce qui est
encore emprunté.

**Fonctions passées en paramètre (`dyn FnMut`)** — `crates/core/src/ingest/snapshot.rs:20` et `:23`.
Le relevé reçoit « une fonction qui appelle l'API » et « une fonction qui donne l'heure ». En production, ce sont le
vrai client et `Utc::now` ; dans les tests, une fausse API alimentée par des réponses réelles enregistrées. C'est
la façon la plus légère de séparer le réseau de la logique : pas de trait, pas de générique, seulement des
fonctions. `FnMut` (et non `Fn`) car la fausse API modifie son état (compteur de crédits).

**Collecter vers `Option<Vec<_>>`** — `crates/core/src/ingest/odds_api.rs:140` et `:156`.
`collect()` sait produire un `Option<Vec<T>>` à partir d'un itérateur d'`Option<T>` : au premier `None`, le
résultat entier est `None`. C'est ce qui écarte un marché 1X2 dont une seule cote est invalide.

**Déplacer plutôt que copier** — `crates/core/src/ingest/odds_api.rs:101` et `:104`.
`for raw_event in raw` consomme le vecteur : chaque `String` du JSON est *déplacée* dans l'`Event`, sans copie.
Seule la clé du bookmaker, partagée par plusieurs cotes, est copiée (`to_string`).

**Durée de vie dans un retour de fonction** — `crates/core/src/ingest/odds_api.rs:167`.
`bookmakers_without_quotes<'a>` renvoie des `&'a str` empruntées à `requested` : aucune chaîne n'est copiée, et
le compilateur garantit que le résultat ne survit pas à la liste dont il vient.

**Deux niveaux d'erreur : `Result<Outcome>`** — `crates/core/src/ingest/snapshot.rs:88` et `:132`.
`Err` signifie « problème de base de données, on arrête tout » ; `Outcome::Failed` signifie « cette ligue a
échoué (API, réseau), on continue avec la suivante ». Le type exprime la différence, le `?` ne propage que le grave.

**Combinateurs d'`Option`** — `crates/core/src/domain.rs:52`, `crates/core/src/ingest/budget.rs:35`,
`crates/core/src/ingest/client.rs:87`. `and_then` (enchaîner deux étapes qui peuvent échouer), `is_some_and`
(tester une valeur éventuelle), `filter`, `ok_or`. Ils évitent des `match` à rallonge pour les cas simples.

**Arithmétique sans débordement** — `crates/core/src/ingest/budget.rs:26`.
`saturating_add` plafonne au maximum au lieu de « boucler » vers un petit nombre : une garde de budget qui
déborderait laisserait passer un appel hors budget.

**Syntaxe de mise à jour de structure** — `crates/core/src/ingest/snapshot.rs:270`.
`Config { leagues: …, ..Config::default() }` prend les champs restants dans la valeur par défaut : pratique pour
les tests.

## 2. Concepts statistiques (aperçu ; formalisés en phase 2)

**Probabilité implicite.** Une cote décimale `c` correspond à la probabilité `1/c` : une cote de 2,00 « dit » 50 %.

**Marge (surcote) d'un bookmaker.** Pour un 1X2, la somme `1/c_domicile + 1/c_nul + 1/c_extérieur` dépasse 1. Cet
excédent est la marge du bookmaker. Si les trois probabilités sommaient à 1, parier à l'aveugle rapporterait
zéro en moyenne ; avec la marge, on perd la marge en moyenne. Le premier relevé réel donne : Pinnacle 4,7 %,
PMU 8,7 %, Betclic 9,6 %, Unibet 11,9 %, Winamax 14,2 % (voir `docs/sources.md`, section 11). Plus la marge est
haute, plus l'avantage nécessaire pour gagner est grand : c'est pourquoi la référence du projet est Pinnacle.

Intuition à retenir : une marge de 14 % signifie qu'un parieur sans avantage perd environ 14 % de ses mises sur
un pari de ce type ; pour gagner, un modèle doit d'abord « rattraper » cette marge.

## 3. Trois questions pour vérifier que tu as compris

1. Pourquoi le corps brut de la réponse est-il écrit dans `api_call` *avant* d'être interprété, et qu'est-ce que
   cela permet si le parseur contient un bug découvert dans trois mois ?
2. `NewApiCall<'a>` contient des `&'a str`. Que se passerait-il si on essayait de la garder après avoir détruit le
   `String` dont vient `params` ? Qui l'en empêche, et à quel moment (compilation ou exécution) ?
3. La garde de budget se fonde sur `x-requests-used` renvoyé par l'API et non sur un compteur tenu par le
   programme. Cite deux situations où un compteur local donnerait un mauvais résultat.

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/ingest/snapshot.rs` — le chef d'orchestre : garde, appel, journal, archive. Lis `run` puis les
   tests, qui racontent chaque cas (budget, panne d'API, réponse illisible, quota réinitialisé).
2. `crates/core/src/ingest/odds_api.rs` — le contrat avec l'API et ses pièges (issues classées par ordre
   alphabétique, matchs déjà commencés, bookmaker inconnu facturé). Ses tests utilisent de vraies réponses.
3. `crates/core/src/store/migrations/0002_odds_archive.sql` — le schéma de l'archive et ses triggers : tout le
   reste du projet (modèles, backtest, CLV) lira ces tables.
