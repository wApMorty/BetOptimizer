# Phase 14 — Loto Foot (LF7, LF8) : `edge lotofoot`, page `/lotofoot`, séance du samedi

Ce qui a été construit : la stratégie retenue au terme de l'étude (`docs/sources.md` §37), mise en production.
- **Chaque heure**, `edge lotofoot fetch` relève sur prono-match la grille en cours du LF7 et du LF8 (avec la répartition FDJ du
  public), puis les résultats et les rapports officiels.
- **Le samedi à midi**, `edge lotofoot recommend --notify` :
  - lit la dernière répartition ;
  - trouve la cote Pinnacle de chaque rencontre (en relevant au besoin la compétition manquante, 1 crédit) ;
  - calcule l'espérance exacte des meilleurs tickets ;
  - garde ceux dont l'espérance, dans le scénario défavorable, atteint 1,2 (16 au plus) ;
  - les écrit au journal immuable et les envoie sur Discord.
- **Après validation chez le buraliste**, `edge lotofoot played --last-session` déclare les tickets joués.
- **Le règlement et la vérification du modèle** se calculent à la demande : `edge lotofoot report`, page `/lotofoot`.

Pourquoi le Loto Foot : c'est un jeu à **répartition**. Les gagnants se partagent 70 % des mises des autres joueurs. L'opérateur ne
perd rien quand un joueur gagne, donc il n'a aucune raison de le limiter. Et le prix d'une issue dépend de ce que joue le public, qui
surjoue les favoris et les clubs populaires.

Découpage :

| Fichier | Rôle |
|---|---|
| `lotofoot/mod.rs` | formules (LF7, LF8), ticket, constantes du règlement |
| `lotofoot/model.rs` | espérance exacte d'un ticket, scénario défavorable |
| `lotofoot/optimize.rs` | tickets gloutons d'une grille, seuil et budget de la séance |
| `lotofoot/pronomatch.rs`, `lotofoot/fetch.rs` | lecture et relevé des pages |
| `lotofoot/odds.rs` | cote Pinnacle de chaque rencontre, compétitions manquantes |
| `lotofoot/session.rs` | la séance du samedi |
| `lotofoot/settle.rs`, `lotofoot/verify.rs` | règlement, vérification figée |
| `store/lotofoot.rs`, `store/migrations/0015_lotofoot.sql` | archive et journal |
| `report/lotofoot_page.rs` | page du site, message Discord |
| `cli/src/lotofoot.rs` | les commandes |

## 1. Concepts Rust rencontrés

**Parcours récursif avec un seul vecteur réutilisé (empiler, dépiler).** `crates/core/src/lotofoot/model.rs:198` et ligne 211.
- Pour énumérer les issues où un ticket fait au plus une erreur, `visit` fixe une rencontre, s'appelle sur la suivante, puis défait
  son choix.
- `walk.outcome.push(real)` avant l'appel, `walk.outcome.pop()` après : le même vecteur sert à toutes les issues, sans allocation.
- La récursion s'arrête par `let Some(&sign) = walk.ticket.get(i) else { … }` : au-delà de la dernière rencontre, `get` renvoie
  `None`, et on traite l'issue complète.

**Une structure qui emprunte : `Walk<'a>`.** `crates/core/src/lotofoot/model.rs:238`.
- Sept paramètres circulaient d'appel en appel : on les regroupe en une structure.
- Le ticket et les paramètres y sont **empruntés** (`&'a [Outcome]`, `&'a Competitors`) ; le `'a` dit que `Walk` ne peut pas leur
  survivre.
- C'est le cas, puisqu'il ne vit que le temps d'un appel à `value`. Ce qui change (l'issue en construction, les sommes) lui appartient.

**Mise à jour en place à rebours.** `crates/core/src/lotofoot/model.rs:337`.
- La loi de Poisson-binomiale se calcule par convolution : `P'(k) = P(k)·(1 − x) + P(k − 1)·x`.
- En parcourant `k` de haut en bas (`(1..=count + 1).rev()`), chaque case est mise à jour avec l'**ancienne** valeur de la case
  précédente, pas avec la nouvelle.
- On évite ainsi un second vecteur.

**`exp_m1` pour la précision.** `crates/core/src/lotofoot/model.rs:323`.
- La part espérée d'une masse vaut `(1 − e^{−μ})/μ`. Quand `μ` est petit, `1 − e^{−μ}` soustrait deux nombres presque égaux et
  perd ses chiffres significatifs.
- `(-μ).exp_m1()` calcule `e^{−μ} − 1` directement, sans cette perte.

**`collect` vers `Option<Vec<_>>`.** `crates/core/src/lotofoot/mod.rs:104`.
- `Ticket::parse` transforme chaque caractère en `Option<Outcome>`.
- `collect::<Option<Vec<_>>>()` s'arrête au premier `None` (un caractère invalide) et renvoie `None` ; sinon il renvoie
  `Some(le vecteur complet)`.
- Même mécanisme qu'en phase 1 pour les cotes d'un marché.

**Un newtype avec sa forme écrite : `impl Display`.** `crates/core/src/lotofoot/mod.rs:87` et ligne 129.
- `Ticket` enveloppe `Vec<Outcome>` : un ticket a exactement un signe par rencontre, et il s'écrit `1N2…`.
- `Display` donne `to_string()` et `{ticket}` dans un `format!` pour le même prix.

**Trier des flottants sans risque : `total_cmp`.** `crates/core/src/lotofoot/optimize.rs:154`.
- `f64` n'est pas totalement ordonné (`NaN`), donc `sort` refuse de le trier directement.
- `total_cmp` définit un ordre total. Combiné à `then_with` sur le ticket, il rend la sélection reproductible même à égalité.

**`is_none_or` pour un « meilleur » sans valeur de départ.** `crates/core/src/lotofoot/optimize.rs:91`.
- `best.as_ref().is_none_or(|(value, _)| current > *value)` vaut vrai tant que rien n'est retenu, ou si le candidat est meilleur.
- Déjà vu en phase 13.

**Lire du HTML par sa structure, pas par ses classes.** `crates/core/src/lotofoot/pronomatch.rs:191` et lignes 262-263.
- `name_cell.text().next()` : seulement le **premier** nœud de texte d'une cellule. Une note ajoutée après le nom (« le 2 est
  surjoué… ») est ignorée.
- `descendants().filter_map(ElementRef::wrap)` parcourt tous les éléments dans l'ordre du document. Un en-tête « Tirage N° … » ouvre
  un bloc, et les rencontres et les rangs qui suivent lui appartiennent.
- Les classes CSS du site sont des identifiants générés (`qe0839a`) qui changent : on s'appuie sur les textes et sur l'attribut
  `title` des signes.

**`INSERT OR IGNORE` : la première observation fait foi.** `crates/core/src/store/lotofoot.rs:80`.
- Une grille est relevée toutes les heures. `OR IGNORE` n'écrit sa clôture et son pactole que la première fois.
- La répartition, elle, est écrite à chaque relevé, avec l'identifiant de la lecture : c'est ce qui permettra de mesurer sa dérive
  jusqu'à la clôture.

**Convertir `usize` pour SQLite.** `crates/core/src/store/lotofoot.rs:21`.
- La taille de `usize` dépend de la machine, donc `rusqlite` ne le convertit pas.
- `i64::try_from(n)` est explicite ; il ne peut pas échouer pour une position de 1 à 8.

**Déstructurer deux `Option` d'un coup.** `crates/core/src/lotofoot/verify.rs:88`.
- `let (Some(results), Some(split)) = (…, …) else { continue }` : une grille sans résultats ou sans répartition finale est sautée.

**Arithmétique « qui boucle » volontairement : `wrapping_add`, `wrapping_mul`.** `crates/core/src/lotofoot/verify.rs:378` et ligne 382.
- Le générateur SplitMix64 repose sur des débordements d'entiers voulus.
- En Rust, un débordement fait paniquer en mode debug. `wrapping_*` dit au compilateur que c'est voulu : le résultat est pris modulo 2^64.

**Une structure nommée plutôt qu'un tuple à cinq éléments.** `crates/core/src/lotofoot/session.rs:80`.
- Clippy refusait `Vec<(GridRow, f64, i64, Vec<[u32; 3]>, Vec<(String, String)>)>` (type trop complexe).
- `Pending { row, stakes, split_fetch_id, split, canonical }` nomme chaque champ.
- On le déstructure dans les boucles (`for Pending { row, canonical, .. } in &pending`) ; le `..` ignore les autres champs.

**Construire d'abord ce qu'on emprunte ensuite.** `crates/core/src/lotofoot/session.rs:291` et ligne 304.
- `NewReco` emprunte le texte JSON de ses entrées (`inputs: &'a str`).
- Ces textes doivent donc exister **avant** les `NewReco` et leur survivre. On les construit dans un vecteur à part (`inputs`), puis
  on les parcourt avec `zip`.

## 2. Concepts statistiques

**Jeu à répartition.** Une grille coûte 1 €, et 70 % des mises reviennent aux gagnants. Pour le LF7 et le LF8, 45 % de cette masse vont
au sans-faute et 55 % à une erreur. Un rang se partage à parts égales entre ses gagnants. Certaines listes garantissent un minimum au
rang 1 (le « pactole » : 100 000 € pour le LF7, 150 000 € pour le LF8).

**Espérance d'un ticket.**
- Formule : `E[gain] = Σ_o P(o) · M_{e(o)} · (1 − e^{−μ(o)})/μ(o)`, la somme portant sur les issues `o` où le ticket fait au plus une
  erreur.
- `P(o)` : produit des probabilités du marché.
- `M_e` : masse du rang atteint.
- `μ(o)` : nombre attendu de concurrents du public au même rang. Il vient de `λ = S · P(k = n − e)`, avec `k` le nombre de bons
  pronostics d'une grille du public (loi de Poisson-binomiale des parts `q_i(o_i)`).
- `(1 − e^{−μ})/μ` est l'espérance de `1/(1 + W)` pour `W ~ Poisson(μ)` : notre part de la masse.
- **Intuition** : quand les concurrents sont nombreux, l'espérance vaut à peu près `0,7 · p(c)/q(c)`. On gagne à jouer les
  combinaisons que le marché juge plus probables que le public.

**Calibration des concurrents.**
- Avec la vraie répartition FDJ, le modèle d'indépendance prédit bien les gagnants publiés (rapport 0,97 à 1,16 sur 1 589 grilles).
- La correction `μ = κ λ^β + φ S/3^n` a été ajustée par maximum de vraisemblance d'une loi binomiale négative. Cette loi tient
  compte de la dispersion d'une grille à l'autre : une loi de Poisson aurait laissé les rangs à des milliers de gagnants dicter
  l'ajustement.
- `φ` représente les joueurs à systèmes, qui couvrent aussi les combinaisons improbables.

**Scénario défavorable et seuil.**
- Le scénario défavorable retient deux hypothèses : concurrents × 1,4 quand `λ < 3`, et marché rapproché de 10 % du public (les cotes
  du vendredi surestiment de 0,7 point les signes à contre-courant).
- Un ticket n'est joué que si son espérance **dans ce scénario** atteint 1,2. Le seuil protège ainsi contre l'erreur du modèle, pas
  seulement contre le hasard.

**Loterie à espérance positive.**
- Les gains sont rares et parfois gros : une grille de 16 tickets LF8 n'est payée qu'environ 5 fois sur 100.
- Le ROI ne dira rien avant des années.
- La vérification porte donc sur ce qui se mesure vite : la calibration des concurrents, la fréquence des tickets payés, et le gain
  réalisé comparé à sa loi simulée. Le verdict tombe aux points de contrôle de 26 et 52 semaines, avec un critère figé avant le
  premier ticket.

## 3. Trois questions pour vérifier la compréhension

1. Pourquoi l'espérance d'un ticket change-t-elle à peine quand on se trompe de 20 % sur les mises `S` de la grille, alors que la masse
   du rang leur est proportionnelle ?
2. Un ticket qui ne fait aucune erreur sur une issue où **personne** d'autre ne gagne touche toute la masse. Pourquoi le modèle
   est-il le moins sûr précisément sur ces tickets, et quelle partie du scénario défavorable le prend en compte ?
3. Dans `GridModel::visit`, que se passerait-il si l'on appelait `walk.outcome.pop()` **avant** l'appel récursif plutôt
   qu'après ?

## 4. Les trois fichiers à relire en priorité

1. **`crates/core/src/lotofoot/model.rs`** : toute la stratégie repose sur cette espérance ; le test de référence la compare au
   script de l'étude à 10⁻⁹ près.
2. **`crates/core/src/lotofoot/session.rs`** : l'enchaînement du samedi, et chaque raison d'écarter une grille. C'est là que se joue
   l'absence de fuite du futur (aucune donnée postérieure à la séance n'est lue).
3. **`crates/core/src/lotofoot/verify.rs`** : le critère figé qui dira, à 26 puis 52 semaines, s'il faut continuer.
