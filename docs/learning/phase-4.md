# Phase 4 — Backtest et mises

Ce qui a été construit : la détection de valeur (espérance, cote minimale), le critère de Kelly (seul, puis simultané),
un moteur de backtest par lots de décision, deux familles de règles de sélection (prix d'un bookmaker contre Pinnacle,
modèle contre prix), le réglage puis le gel des seuils, l'évaluation unique sur le test, le CLV, la simulation de ruine,
`edge backtest tune | run | report` et `edge value` (opportunités du dernier relevé, lecture seule).

Résultat à retenir : **aucune règle n'est validée.** Le modèle est rejeté (CLV −7,6 %), les règles « prix mou contre
Pinnacle » n'ont presque aucun pari au test, donc ni preuve ni réfutation (`docs/sources.md` §18).

## 1. Concepts Rust rencontrés

**`let … else`** — `crates/core/src/betting/live.rs:162` et `:172`.
`let Some(x) = valeur else { continue; };` déballe un `Option` ou sort de la boucle d'un trait. C'est la forme lisible
d'un `match` à deux branches dont l'une ne sert qu'à écarter le cas.

**`#[derive(Default)]` et `entry().or_default()`** — `live.rs:71` et `:152`.
`BookQuotes` dérive `Default` (trois cotes absentes, aucune date) : `books.entry(clé).or_default()` crée l'entrée vide si
elle manque, puis on la remplit au fil des lignes SQL. Pas de `if contains_key` suivi d'un `insert`.

**`BTreeMap` pour un ordre déterministe** — `live.rs:131`.
Un `HashMap` itère dans un ordre qui change d'une exécution à l'autre. Avec un `BTreeMap`, deux bookmakers à
espérance égale sont toujours départagés de la même façon : même entrée, même sortie (invariant « reproductible »).

**`Option::is_none_or`** — `live.rs:184`. « Vrai si absent, ou si la condition tient » : garder le meilleur candidat
sans `match` à trois lignes.

**`bool::then_some`** — `crates/core/src/betting/kelly.rs:28`. `condition.then_some(valeur)` donne `Some(valeur)` ou
`None` : la mise n'existe que si elle atteint la mise minimale.

**Boucle sur un masque binaire** — `crates/core/src/betting/simultaneous.rs:69`.
`for mask in 0u32..(1u32 << n)` énumère les `2ⁿ` combinaisons de paris gagnés/perdus : le bit `i` du masque dit si le pari
`i` gagne. C'est l'« énumération exacte » de CLAUDE.md, tant que `n ≤ 12` (4 096 cas).

**Rendre un problème contraint non contraint** — `simultaneous.rs:99-106`.
L-BFGS cherche sans contrainte, or les mises doivent être positives et sommer à moins de 1. On optimise donc des
nombres libres `x` et on pose `f_i = exp(xᵢ) / (1 + Σ exp(x_k))` (softmax avec une « part de réserve »). Toute valeur de
`x` donne alors une répartition valide.

**`Serialize` sur les résultats, `rename_all`** — `crates/core/src/backtest/study.rs:299-300`, `candidates.rs:27-28`.
Les structures de résultat dérivent `Serialize` : le gabarit HTML les lit directement (`s.verdict == "rejected"`).
`#[serde(rename_all = "snake_case")]` fixe le nom textuel des variantes d'énumération.

**Filtres et macros de gabarit** — `crates/core/src/report/render.rs:75-77`, `templates/backtest.html:36-39`.
Un filtre `spct` (pourcentage signé à la française) et des macros (`clv`, `verdict`) évitent de répéter la même logique
d'affichage dans chaque tableau.

**NaN et comparaisons négatées** — `crates/cli/src/commands.rs` (`value`). `!(x >= 0.0)` est vrai pour NaN, et Clippy
refuse cette écriture ambiguë : on écrit `x.is_nan() || x < 0.0`, qui dit ce qu'on veut.

## 2. Concepts statistiques

**Espérance d'un pari.** `EV = p · cote − 1` pour une mise d'un euro. Positive si le prix dépasse la cote juste `1/p`.
Elle ne vaut que par la qualité de `p` : une probabilité fausse crée de la « valeur » fictive (`betting/value.rs`).

**Cote minimale.** `(1 + seuil) / p` : en dessous, le pari n'a plus l'avantage exigé. C'est la ligne à vérifier sur le site
de l'opérateur avant de parier.

**Kelly.** Pour un pari seul, `f* = (p · cote − 1) / (cote − 1)` maximise la croissance logarithmique de la bankroll
(`betting/kelly.rs`). Kelly complet est agressif : une erreur d'estimation de `p` se paie très cher, d'où les variantes ½
et ¼.

**Kelly simultané.** Plusieurs paris indépendants partagent la bankroll : on maximise
`Σ_S P(S) · ln(1 − Σ fᵢ + Σ_{i∈S} fᵢ·coteᵢ)` sur toutes les issues `S`. Au-delà de 12 paris, une approximation du
second ordre `fᵢ = (μᵢ/σᵢ²) / (1 + Σ μₖ²/σₖ²)` remplace l'énumération. **Elle ne garantit pas `Σ f < 1`** : sur un gros lot à
fort avantage annoncé elle misait plus que la bankroll. Le défaut est apparu dans le rapport (drawdown simulé de 103 %,
impossible) ; la correction est un plafond proportionnel à 95 % (`simultaneous.rs:29`, `:215`).

**Lots de décision.** On ne parie pas match par match mais par lot : le vendredi pour les matchs du vendredi au lundi, le
mardi pour ceux du mardi au jeudi (`backtest/engine.rs:33`). C'est ce que permettent les cotes d'avant-match de
football-data.

**CLV (closing line value).** `CLV = cote obtenue × p_clôture_sans_marge − 1`. La clôture est le meilleur estimateur
disponible de la vraie probabilité ; battre la clôture est le signe d'un avantage réel, avec une variance bien plus
faible que le rendement. Avec ~300 paris, le CLV a un intervalle étroit ; le rendement, lui, en demanderait des dizaines
de milliers (`backtest/metrics.rs:67`, `:78`).

**Sélection sur les erreurs (malédiction du gagnant).** Si l'on retient les paris où le modèle s'écarte le plus du
marché, on retient surtout ses erreurs. Mesuré : le modèle annonce 32,5 % de victoires, on en observe 25,5 %, et la
clôture du marché disait 26,9 %. D'où l'analyse « espérance annoncée contre réalisée » par tranche, et le CLV de plus en plus
mauvais quand l'écart annoncé grandit.

**Gel du seuil et comparaisons multiples.** Chaque configuration essayée (54) est une chance de trouver un « bon »
résultat par hasard. Le seuil est choisi sur entraînement + validation, gelé, puis le test est lu une seule fois
(`backtest/study.rs:149`). Un test vide ne se corrige pas en changeant le protocole après coup.

**Ruine par simulation (Monte Carlo).** On rejoue 2 000 fois la séquence de paris en tirant les résultats au hasard, sous
deux hypothèses : (a) les probabilités annoncées sont vraies ; (b) la probabilité de clôture est la vraie. L'écart entre
(a) et (b) mesure ce que coûte une erreur d'estimation (`metrics.rs:151`). Pour le modèle en Kelly complet : 0 % de ruine en
(a), 98 % en (b).

**Puissance d'un test.** Moins de 30 paris au test : aucun intervalle ne peut exclure zéro ni un gros effet. Le rapport
distingue donc « rejetée », « CLV positif », « indécise » et « test sans puissance » (`study.rs:313`).

## 3. Trois questions pour vérifier que tu as compris

1. Le modèle annonce une espérance de +16,6 % sur ses paris du test et réalise −12,4 %. Pourquoi la tranche « ≥ 10 % »
   est-elle la pire (CLV −8,5 %) et non la meilleure ? Quelle partie de ce résultat vient du modèle, laquelle de la
   méthode de sélection ?
2. Bet365 a un CLV de +0,88 % sur 297 paris au réglage et −0,22 % sur 8 paris au test. Que peux-tu conclure de chacun des
   deux chiffres ? Pourquoi le CLV saison par saison est-il plus instructif que la moyenne ?
3. Avec 40 paris à 55 % de réussite supposée et une cote de 2,20, l'approximation brute misait environ 280 % de la
   bankroll. Pourquoi l'optimum exact ne peut-il jamais dépasser 100 % ? Pourquoi ramener toutes les mises au prorata
   (plutôt que d'en supprimer) conserve-t-il l'esprit de Kelly ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/backtest/study.rs` — le protocole en entier : générer les paris, régler le seuil, le geler, lire le
   test une fois, conclure par un verdict. C'est ici que l'honnêteté méthodologique se joue.
2. `crates/core/src/betting/simultaneous.rs` — l'énumération exacte, le softmax qui transforme un problème contraint en
   problème libre, l'approximation et son plafond ; chaque garantie y a son test.
3. `crates/core/src/betting/live.rs` — le chemin d'une cote archivée à une suggestion de pari : dernier relevé,
   probabilité de Pinnacle sans marge, une sélection par match, prix aberrants écartés, mises en Kelly simultané.
