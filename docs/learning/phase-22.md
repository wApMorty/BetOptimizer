# Phase 22 — Le Loto Foot lit la FDJ, avec resultats-fdj.fr en repli

Ce qui a été construit : prono-match.fr, figé depuis le 5 novembre, laisse la place au site des points de vente de la FDJ (source
principale : l'état JSON embarqué dans la page) et à resultats-fdj.fr (repli, en HTML), lu seulement quand la FDJ échoue, page par page.
Une rencontre annulée s'écrit et se règle (bonne pour tous les signes) ; les noms abrégés de la FDJ se comparent par nom canonique ; trois
natures d'alerte de source s'ajoutent ; l'application Web dit quelle source sert chaque page. Deux migrations (24 et 25). Mesures, essai
réel et choix ouvert : `docs/sources.md` §74. Rien n'est déployé.

| Fichier | Rôle |
|---|---|
| `core/src/lotofoot/fdj.rs` | lecteur de la FDJ : état `serverApp-state`, grilles ouvertes, résultats, rapports |
| `core/src/lotofoot/resultats_fdj.rs` | lecteur du repli : accueil, liste des résultats, page de détail (HTML) |
| `core/src/lotofoot/grid_page.rs` | types communs aux deux sources (`OpenGrid`, `GridResult`, `MatchResult`), `published_split` |
| `core/src/lotofoot/fetch.rs` | relevé à deux sources, repli page par page, garde de page périmée, structure `Io` |
| `core/src/lotofoot/alert.rs`, `store/alerts.rs`, migration 25 | alertes de source, limitées par le journal `alert_log` |
| `core/src/lotofoot/odds.rs`, `config/aliases.toml` | comparaison des noms par nom canonique, alias des abréviations FDJ |
| `core/src/lotofoot/{mod,settle,scores,verify}.rs` | rencontre annulée : règlement, suivi en direct, vérification figée |
| `core/src/store/lotofoot.rs`, migration 24 | `Source`, `SourcesHealth`, refus d'une grille au mauvais nombre de rencontres |
| `core/src/web/pages/` | pastille « Loto Foot » et encadré de `/lotofoot` : santé de chaque source |
| `deploy/systemd/edge-lotofoot-fetch.*` | attente du DNS de la FDJ, délai de démarrage de 600 s |

## 1. Concepts Rust rencontrés

**`Iterator::partition` : couper une liste en deux d'un seul parcours.** `crates/core/src/lotofoot/fetch.rs:374` et `:375`.
- `grids.into_iter().partition(|grid| grid.closes_at <= at)` rend deux collections : les éléments pour lesquels la fermeture répond
  `true` (les grilles closes), puis les autres (les ouvertes).
- Le type du résultat s'écrit (`(Vec<OpenGrid>, Vec<OpenGrid>)`) : `partition` sait remplir n'importe quelle collection, il ne devine
  pas laquelle.
- `into_iter()` consomme `grids` : chaque grille passe dans l'un des deux vecteurs sans `clone()`. La fermeture, elle, ne reçoit qu'une
  référence (`&OpenGrid`) : elle lit la clôture sans prendre la grille.

**`return` dans un bras de `match`.** `crates/core/src/lotofoot/fetch.rs:332` (et `:455`).
- `Ok(grids) => return record_open(…)` : une expression `return` a le type `!` (« ne revient jamais »), que le compilateur accepte à la
  place de n'importe quel type. Le `match` prend donc la valeur de l'autre bras, `failure`.
- Le cas heureux quitte la fonction sur place ; tout ce qui suit (journaliser l'échec de la FDJ, essayer le repli) ne concerne que
  l'échec, sans `else` ni niveau d'indentation de plus. `fdj_failure` reste à portée pour le journal et pour l'alerte.

**`Option` est ordonnée : `max` et `>`.** `crates/core/src/store/lotofoot.rs:198` et `:218`.
- `Option<T>` est ordonnée dès que `T` l'est, et `None` est plus petit que tout `Some`. `self.fdj.last_ok.max(self.fallback.last_ok)`
  rend la lecture réussie la plus récente des deux sources, ou celle qui existe, sans `match`.
- Même règle pour `self.fallback.last_ok > self.fdj.last_ok` : « le repli a réussi après la FDJ », vrai aussi quand la FDJ n'a jamais
  réussi (`None`), faux quand le repli n'a jamais réussi.

**`std::slice::from_ref` : une tranche d'un élément, empruntée.** `crates/core/src/lotofoot/fetch.rs:581`.
- `record_results` demande une tranche `&[GridResult]` ; la page de détail du repli n'en donne qu'un. `std::slice::from_ref(&result)`
  fabrique la tranche de longueur 1 qui **emprunte** `result` : ni déplacement dans un tableau temporaire, ni `clone()` (un `GridResult`
  possède des `Vec` : le copier coûterait).
- Dans un test (`crates/core/src/store/lotofoot.rs:1784`), `reco` sert encore après l'appel : `&[reco]` l'aurait déplacée.

**Lire un JSON en deux temps : `serde_json::from_value`, `rename_all`, `default`.** `crates/core/src/lotofoot/fdj.rs:91`, `:102`, `:220` à `:226`.
- Les structures `Raw…` copient la forme du JSON de la FDJ, dont les clés sont en `camelCase` (`endValidationDate`). `#[serde(rename_all =
  "camelCase")]` relie ces clés aux champs Rust en `snake_case`, sans écrire un `rename` par champ. `#[serde(default)]` donne un
  `Vec` vide quand la clé manque (`winranks` n'existe qu'une fois la grille close) ; `Option<String>` accepte `null` (`result`).
- Les grilles arrivent d'abord comme `serde_json::Value`, car la formule se lit dans `/product/deeplink` (`value.pointer`) : le LF12 et
  le LF15 sont écartés **sans être analysés** (leur format peut différer, et une grille inconnue ne doit pas faire refuser la page).
  Ensuite `serde_json::from_value(value)` lit le `Value` comme une `RawGrid` et le **consomme**.
- L'ordre compte pour l'emprunt : `deeplink` emprunte `value` ; il ne sert plus quand `from_value` la déplace, et `game` (un `Game`,
  type `Copy`) n'a aucun lien avec `value`. Le compilateur accepte ; déplacer `value` avant de lire `deeplink` serait refusé.

**Sortir les éléments d'un tableau : `let [home, away] = <[String; 2]>::try_from(opponents)`.** `crates/core/src/lotofoot/fdj.rs:268`.
- `<[String; 2]>::try_from(vec)` (déjà vu avec `<[u8; 5]>`, phase 16) réussit si le `Vec` a exactement deux éléments. En cas d'échec,
  l'erreur **est le `Vec`** rendu intact : d'où `other.len()` dans le message, sans avoir rien perdu.
- Un motif de tableau de taille fixe ne peut pas échouer : `let [home, away] = …` n'a pas de `else`, contrairement au motif de tranche
  de la phase 12. Les deux `String` sortent du tableau par déplacement, pas par copie.

**Une fermeture rangée dans un champ : `(self.pause)(…)`.** `crates/core/src/lotofoot/fetch.rs:233` à `:256`.
- `Io<'a>` regroupe l'horloge, l'appel réseau et la pause (les trois `&mut dyn FnMut` des phases 1, 13 et 19), plus un drapeau `asked`.
  Toute requête du relevé passe par `Io::get` : l'écart de cinq secondes entre deux requêtes, quelle que soit la source, est garanti en
  **un seul** endroit.
- `self.pause(x)` serait un appel de *méthode* nommée `pause` : une fermeture dans un champ s'appelle avec des parenthèses autour du
  champ, `(self.pause)(x)`. Il faut `&mut self`, car appeler un `FnMut` demande l'accès en écriture au champ.
- `impl Io<'_>` : la durée de vie existe (`Io` ne survit pas aux fonctions que l'appelant possède) mais n'a pas besoin de nom ici.

**Un chemin de fonction à la place d'une fermeture.** `crates/core/src/lotofoot/fetch.rs:331` et `:389` ; `crates/core/src/lotofoot/scores.rs:349` à `:351`.
- `body.and_then(|html| parse(&html).map_err(Failure::unreadable))` : `Failure::unreadable(error: Error) -> Failure` a exactement la forme
  de la fermeture `|e| Failure::unreadable(e)`, donc on passe son nom. Déjà vu avec `ElementRef::wrap` (phase 14).
- Le type doit coller. `result.and_then(MatchResult::sign)` : `sign(self)` prend la valeur, et `results.get(index).copied()` donne un
  `Option<MatchResult>` (`copied` : `Option<&T>` → `Option<T>`, permis car `MatchResult` est `Copy`). `.map(StaleGrid::reason)` :
  `reason(&self)` prend une référence, et `iter()` en fournit.

**`self` par valeur sur un type `Copy`, et `**pick`.** `crates/core/src/lotofoot/grid_page.rs:48` à `:90` ; `crates/core/src/lotofoot/mod.rs:200` à `:208`.
- `MatchResult` (un signe, ou l'annulation) est une petite énumération `Copy` : ses méthodes prennent `self`, pas `&self`
  (`sign(self)`, `is_good_for(self, pick: Outcome)`). Les appeler ne déplace rien et se lit mieux.
- Dans `Ticket::good`, `iter().zip(results)` produit des paires `(&Outcome, &MatchResult)`, et `filter` prête une référence à chaque
  paire : dans la fermeture, `pick` est un `&&Outcome`. `**pick` sort l'`Outcome` (copié). `result.is_good_for(…)` marche sur un
  `&&MatchResult` : le compilateur déréférence automatiquement, puis copie.

**`matches!` avec une garde : ne pas confondre deux inconnus.** `crates/core/src/lotofoot/odds.rs:68` à `:74`.
- `matches!` est vu en phase 12 ; la nouveauté est la garde : `(Some(a), Some(b)) if a == b`. La comparer directement,
  `resolve(left) == resolve(right)`, rendrait vrai pour deux noms **inconnus** de la table, puisque `None == None`. La garde exige que
  les deux noms soient connus *et* ramenés au même nom canonique. Le `||` qui précède ajoute le cas simple : deux noms écrits à l'identique.
- C'est la règle du projet traduite en code : jamais d'appariement approximatif. `Atl. Madrid` et `Atlético Madrid` sont la même équipe
  parce que la table le dit, pas parce que leurs lettres se ressemblent.

**`std::result::Result` à côté de l'alias.** `crates/core/src/lotofoot/fetch.rs:243` ; `crates/core/src/lotofoot/resultats_fdj.rs:332`.
- L'alias du projet, `type Result<T> = std::result::Result<T, Error>` (`crates/core/src/error.rs:182`), ne prend qu'un paramètre. Quand
  l'erreur est une `Failure` ou une `String`, on écrit le chemin complet : `std::result::Result<String, Failure>`. Un `Result<_, Failure>`
  serait refusé : l'alias n'accepte qu'un seul argument de type.

**`transpose` dans l'autre sens, avec `filter_map`.** `crates/core/src/lotofoot/resultats_fdj.rs:230` à `:233`.
- `open_grid` rend un `Result<Option<OpenGrid>>` : `Err` pour une page illisible, `Ok(None)` pour une formule non suivie (LF12, LF15).
  `transpose()` le retourne en `Option<Result<OpenGrid>>` : `Ok(None)` devient `None`, que `filter_map` **jette** ; une erreur reste
  `Some(Err(…))`, que `filter_map` garde.
- `collect()` rassemble en `Result<Vec<_>>` (le type vient du retour de la fonction) et s'arrête à la première erreur. Sens contraire
  de la phase 5 (`Option<Result>` vers `Result<Option>`).

**`rem_euclid` et `i64::from`.** `crates/core/src/lotofoot/fdj.rs:253` à `:256`.
- `i64::from(year.rem_euclid(10)) * 1000 + i64::from(raw.grid_number)` : le dernier chiffre de l'année. `rem_euclid` rend toujours un
  reste positif (le `%` de Rust rend un reste négatif pour un dividende négatif). `i64::from` n'existe que pour les conversions sans
  perte : elle ne peut pas échouer, donc ni `try_from` ni `unwrap`.

**Un faux réseau : une `HashMap` d'adresses.** `crates/core/src/lotofoot/fetch.rs:630` à `:698`.
- `Fake` associe une adresse à une réponse (`Reply::Page(statut, corps)` ou `Reply::Down`) et note dans `calls` chaque adresse
  demandée, dans l'ordre. Une adresse inconnue répond comme un réseau en panne (`Some(Reply::Down) | None`) : un test qui oublie une
  page échoue bruyamment.
- `pass` donne à `run` trois fermetures : `&mut || at(when)`, `&mut |url, _| fake.get(url)` et `&mut |_| pauses += 1`. La deuxième emprunte
  `fake` en écriture, la troisième emprunte le compteur `pauses` : deux emprunts mutables de variables **différentes**, acceptés. Les
  emprunts finissent avec l'instruction : on lit `pauses` et `fake.calls` ensuite.
- Les tests vérifient le **nombre et l'ordre des requêtes**, pas seulement ce qui est écrit en base : le passage nominal
  (`fetch.rs:936` et `:937`) attend `[FDJ_OPEN, FDJ_RESULTS]` et une seule pause entre les deux.

**Côté SQL : reconstruire une table sans casser ses clés étrangères.** `crates/core/src/store/migrations/0024_lotofoot_sources.sql:10` à `:66`, `0025_alert_kinds.sql:5` à `:28`.
- Une contrainte `CHECK` ne se modifie pas : pour que `outcome` accepte `C` et que `kind` accepte trois natures de plus, on **reconstruit**
  la table, dans la transaction unique de `migrate` (une erreur annule tout).
- `ALTER TABLE x RENAME TO y` réécrit aussi les clés étrangères **des autres tables** qui désignent `x` : elles suivent la table
  renommée. Renommer `lotofoot_fetch` (parent de six tables) aurait donc pointé ces six tables vers `lotofoot_fetch_old`, supprimée ensuite.
  Pour `lotofoot_result` et `alert_log`, que rien ne référence, le renommage est sans danger (`:23` ; `0025:21`).
- Pour `lotofoot_fetch` : copie des lignes (`CREATE TABLE … AS SELECT`, `:45`), `DROP TABLE` (`:47`), recréation sous le même nom
  (`:49`), remise **des mêmes identifiants** (`:59`). Mais `DROP TABLE` avec clés actives fait d'abord un `DELETE` implicite, refusé
  si des lignes filles restent. `PRAGMA defer_foreign_keys = ON` (`:18`) reporte la vérification au `COMMIT`, et la remise des
  identifiants la rétablit. Si elle échouait, le `COMMIT` serait refusé et toute la migration annulée. Le réglage retombe tout seul à la
  fin de la transaction.
- Vérifié sur une paire parent/enfant avec le SQLite 3.54.0 du poste (`docs/sources.md` §74) : le renommage entraîne la clé ; le
  `DROP` sans report échoue ; avec report, le `COMMIT` passe ; sans remise des lignes, il est refusé.
- `DROP TABLE` ne déclenche pas les triggers d'immuabilité (ils ne gardent que `UPDATE` et `DELETE`) ; ils partent avec la table et se
  **recréent** à la fin (`:39`, `:63`). Sans cela, l'archive serait modifiable sans que rien ne le dise.

**Rappels (déjà expliqués ; nouveaux emplacements).**
- `let … else` : `fdj.rs:222`, `:417`, `:420` ; `store/lotofoot.rs:457` (phases 10, 12, 13, 14). Le `if let … &&` : `store/lotofoot.rs:475` (phase 21).
- `sort_by_key` avec `Reverse` : `fdj.rs:431`, `resultats_fdj.rs:441` : rangs par bons pronostics décroissants (phases 10, 21).
- `collect` vers `Option<Vec<_>>` : `grid_page.rs:97` (`signs_of`, le type vient du retour de la fonction) ; vers `Result<Vec<_>>` :
  `resultats_fdj.rs:437` (phases 1, 11, 14).
- `bool::then` et `then_some` : `fdj.rs:395` à `:397` (`then(|| Euros::new(…)).flatten()` : `Euros::new` rend déjà un `Option`, d'où l'`Option<Option>`
  aplati de la phase 21) ; `resultats_fdj.rs:500` (`Ok(complete.then_some(matches))` : un `Result<Option<_>>` à trois issues, erreur, rien
  à écrire encore, résultat) ; `fetch.rs:78`, `:386` (phases 4, 5, 18).
- Durée de vie explicite : `resultats_fdj.rs:89` (`first<'a>`, phase 18, `scores.rs:261`). Une seule des deux entrées donne sa durée au
  résultat : l'élément rendu emprunte la **page** (`parent`), non le sélecteur `css`, qui n'est qu'un texte transitoire. Sans `'a`, la
  règle d'élision ne saurait pas laquelle choisir.
- `optional()`, `flatten()`, `transpose()` : `store/lotofoot.rs:154` à `:157` (phase 21). `impl fmt::Display` d'une énumération à
  données : `store/lotofoot.rs:92` (phase 14). `Copy` d'une petite énumération passée par valeur : `Source` et `Page`, `:36` et `:67` (phase 21).
- `match` avec garde : `fetch.rs:251` (`Ok(response) if response.status != 200`, phases 0 et 18).
- Petit ajout : `unwrap_or_default()` (`resultats_fdj.rs:246`) rend `""` pour une `Option<&str>` vide : ni `unwrap` ni `expect`, interdits hors tests.

## 2. Concepts statistiques et de règlement

**Une rencontre annulée est bonne pour tous les signes.** `crates/core/src/lotofoot/grid_page.rs:49` à `:90` ; `crates/core/src/lotofoot/mod.rs:197`.
- Règlement Loto Sports v6, art. 10.6 (`docs/sources.md` §39 et §74). Intuition : la rencontre n'a pas eu lieu, personne ne peut s'y
  tromper. Un ticket de sept signes dont la rencontre 5 est annulée compte donc sept bons si ses six autres signes sont justes.
- Les rangs gardent leurs libellés (N bons et N − 1 bons) : `rank_of` (`settle.rs:36`) n'a pas changé. La nouveauté est de **représenter**
  l'annulation : une `enum MatchResult { Sign(Outcome), Canceled }` plutôt qu'un `Option<Outcome>`, car « annulée » est un résultat qui
  change le calcul, pas une valeur manquante. Elle s'écrit `C` en base (migration 24).
- Mesure (1 735 copies Wayback, 110 rencontres annulées sur 89 grilles) : gagnants observés sur attendus 0,92 pour 58 grilles à un
  annulé, 0,71 pour 8 grilles à deux ou plus, 1,17 sans annulé (1 298 grilles ; calcul avec indépendance des rencontres, rencontre
  annulée retirée du produit, λ ≥ 3). Ces chiffres s'accordent avec la règle ; ils ne valident pas pour autant un modèle de prédiction
  des gagnants de ces grilles.

**Pourquoi la calibration exclut ces grilles.** `crates/core/src/lotofoot/verify.rs:95` à `:104` ; `crates/core/src/lotofoot/grid_page.rs:95`.
- La calibration compare gagnants publiés et gagnants prédits (`λ`) par tranche, et `λ` se calcule à partir d'**une issue par rencontre**.
  Une rencontre annulée n'en a pas : `signs_of` rend `None`, et la grille est sautée.
- Lui inventer un signe, ou retirer la rencontre du produit, reviendrait à utiliser un modèle de substitution **non validé**. Ses rapports
  (0,92 et 0,71) pourraient alors faire déclarer le modèle gelé « en défaut » pour une raison qui n'est pas la sienne. Mieux vaut perdre
  5 % des grilles que fausser le critère.

**Le biais d'indulgence de la vérification figée.** `crates/core/src/lotofoot/verify.rs:225` à `:228` ; `docs/sources.md` §74.
- Pour la fréquence et le gain réalisé, une grille à annulé compte comme le règlement la compte (bonne pour tous). Mais la loi simulée du
  gain tire une issue par rencontre et ne connaît pas l'annulation : le gain réalisé peut dépasser un peu sa loi simulée.
- Le critère d'échec est « gain réalisé sous le 5e centile de la loi simulée ». Un gain réalisé trop haut éloigne de ce seuil : le biais
  **ne peut que retarder** un verdict « en défaut », jamais en créer un. Un biais qui va dans le sens de l'indulgence est moins grave qu'un
  biais qui accuse à tort, mais c'est tout de même un biais. Le choix (garder ces grilles, ou les exclure aussi de la fréquence et du
  gain) est laissé à Paul.

**`externalDrawNumber`, un contrôle de l'année d'une grille.** `crates/core/src/lotofoot/fdj.rs:246` à `:264`.
- `externalDrawNumber = (année de clôture mod 10) × 1 000 + numéro`. Exemple : 6158 pour le LF8 n°158, clos en 2026. Vrai sans
  exception sur 1 735 grilles de 2021 à 2026 et sur les 51 lues le 2026-11-14.
- C'est l'année de la **clôture** qui compte : huit grilles n°1, ouvertes fin décembre et closes début janvier, portent le chiffre de
  l'année suivante. Le numéro encode donc la renumérotation du 1er janvier (§66) : on le compare à l'année que **nous** déduisons de la
  clôture, heure de Paris. Un désaccord fait **refuser la page** : l'archive ne se corrige pas, une grille écrite sous la mauvaise année y
  resterait.
- Limite honnête : un seul chiffre d'année est ambigu à dix ans (2026 et 2036 donnent 6).

**Un zéro n'est pas une observation.** `crates/core/src/lotofoot/grid_page.rs:117` à `:136`.
- Une grille annoncée, ou ouverte depuis peu, porte des pourcentages à 0 (le LF7 n°143 n'en avait toujours aucun une heure après son
  ouverture). `published_split` rend `None` si tous les pourcentages sont nuls ; `published_ranks` fait de même pour des rapports tous
  sans gagnant (un rang 2 réel a toujours des milliers de gagnants). L'archive n'écrit qu'une fois (`INSERT OR IGNORE`) : écrire des
  zéros provisoires y figerait un faux relevé. Une grille sans répartition est écartée de la séance, pas calculée sur du vide.
- La FDJ ne donne la répartition qu'en **pourcentages entiers**, jamais le nombre de grilles jouées : l'estimation des mises (§65)
  reste nécessaire, avec une précision d'un demi-point sur chaque part.

## 3. Questions pour vérifier

1. Un ticket de LF7 `1N21N21` est joué sur une grille dont la rencontre 5 est annulée ; le résultat officiel contredit son signe de la
   rencontre 2, et ses cinq autres rencontres jouées sont justes. Combien de bons pronostics compte-t-il, à quel rang paie-t-il, et par
   quelles fonctions le calcul passe-t-il ? Pourquoi cette grille n'entre-t-elle pas dans la calibration, alors qu'elle est réglée et
   compte dans la vérification du gain ?
2. La FDJ répond 403 à la page des grilles ouvertes, puis 200 à la page des résultats. Quelles requêtes le passage fait-il, vers quels
   sites et dans quel ordre, et quelles lignes écrit-il dans `lotofoot_fetch` ? Avec `--result-pages 3`, si c'est la page 1 des résultats
   que la FDJ n'a pas servie, combien de pages de la FDJ sont encore demandées dans ce passage, et pourquoi ?
3. La migration 24 supprime et recrée `lotofoot_fetch`, alors que la migration 25 renomme `alert_log_new`. Pourquoi cette différence ?
   Que se passerait-il, à `DROP TABLE lotofoot_fetch`, sans `PRAGMA defer_foreign_keys = ON`, et que se passerait-il au `COMMIT` si les
   lignes n'étaient pas remises avec leurs identifiants ?

## 4. Fichiers à relire en priorité

1. `crates/core/src/lotofoot/fetch.rs` : le cœur de la phase. `run`, `Io`, `read_open_grids` et `record_open` suffisent pour comprendre
   le passage (deux sources, repli, page périmée) ; les tests, avec leur faux réseau, disent ensuite ce qui est garanti.
2. `crates/core/src/lotofoot/grid_page.rs` : 136 lignes, et c'est là que le règlement entre dans le code (`MatchResult`, `is_good_for`,
   `signs_of`) avec la règle des zéros (`published_split`). Les deux lecteurs ne produisent que ces types.
3. `crates/core/src/store/migrations/0024_lotofoot_sources.sql` : son en-tête explique en quinze lignes pourquoi on reconstruit, pourquoi
   on ne renomme pas, et ce que fait `defer_foreign_keys`. À lire avec `0025_alert_kinds.sql`, le cas simple, pour sentir la différence.
