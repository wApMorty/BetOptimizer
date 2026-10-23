# Phase 16 — Grand Loto et alerte EuroMillions (`edge loto`)

Ce qui a été construit : un relevé quotidien de l'historique officiel de la FDJ, une séance par édition du Grand Loto (prévision du
nombre de grilles, choix de grilles délaissées par le public, décision au seuil gelé), la déclaration et le règlement des grilles
jouées, une alerte Discord avant un cinquième tirage EuroMillions au plafond, et un rapport d'étude. Tout est dans le cœur, testé sans
réseau ; la CLI branche le vrai client HTTP et Discord.

| Fichier | Rôle |
|---|---|
| `core/src/loto/mod.rs` | `Game`, `Draw`, `Grid` (cinq numéros et une chance) |
| `core/src/loto/rules.rs` | probabilités et clés de répartition des règlements |
| `core/src/loto/fdj.rs` | archives ZIP de la FDJ : téléchargement, lecture des CSV, passage du timer |
| `core/src/loto/estimate.rs` | nombre de grilles d'un tirage, bilan d'une édition, prévision d'une édition |
| `core/src/loto/popularity.rs` | préférences du public, ajustement par L-BFGS, walk-forward |
| `core/src/loto/expectation.rs` | espérance d'une grille, choix des grilles |
| `core/src/loto/recommend.rs` | séance d'une édition, journal, message Discord |
| `core/src/loto/played.rs` | grilles jouées, règlement |
| `core/src/loto/alert.rs` | détection des tirages EuroMillions au plafond, alerte unique |
| `core/src/store/loto.rs`, `migrations/0016_loto.sql` | tables immuables |
| `core/src/report/loto_report.rs`, `templates/loto.html` | rapport `edge loto study` |
| `cli/src/loto.rs` | les quatre commandes |

## 1. Concepts Rust rencontrés

**Implémenter un trait externe à la main : `serde::Serialize` pour `Grid`.** `crates/core/src/loto/mod.rs:166`.
- `#[derive(Serialize)]` aurait écrit une grille en objet JSON (`{"balls": [...], "chance": 10}`).
- On veut la chaîne `2-3-30-35-44+10`, comme en base. On écrit donc `serialize` soi-même : il appelle `serialize_str`.
- La fonction est générique (`S: Serializer`) parce que le trait l'exige : c'est serde qui choisit le format (JSON ici).

**Une fermeture qui renvoie des octets : `type Download<'a> = &'a mut dyn FnMut(&str) -> Result<(u16, Vec<u8>)>`.**
`crates/core/src/loto/fdj.rs:99`.
- Même principe que le `Fetch` de la phase 1, mais une archive est binaire : le corps est un `Vec<u8>`, pas un `String`.
- En production, la CLI passe `&mut |url| client.get(url)` ; le test passe une fermeture qui renvoie une archive fabriquée.

**`let … else` : sortir tôt quand un motif ne correspond pas.** `crates/core/src/store/loto.rs:146`, `expectation.rs:221`.
- `let Some((balls, extra, codes, code_gain)) = row else { return Ok(None); };` déstructure la ligne lue, ou sort de la fonction.
- Le bloc `else` doit quitter (`return`, `continue`, `break`) : après lui, les variables existent forcément.

**Convertir un `Vec` en tableau : `<[u8; 5]>::try_from(vec)`.** `crates/core/src/loto/expectation.rs:221`.
- Un tableau `[u8; 5]` a sa taille dans son type ; un `Vec` non. La conversion échoue si le vecteur n'a pas cinq éléments.
- On l'utilise avec `let Ok(chosen) = … else { continue; }` : pas d'`unwrap`, et le compilateur sait ensuite qu'il y a cinq numéros.

**Parcourir deux collections ensemble : `iter_mut().zip(…).enumerate()`.** `crates/core/src/loto/popularity.rs:299`.
- `gradient.iter_mut()` donne une référence modifiable vers chaque case ; `zip(&prefs.balls)` y joint le poids du même numéro.
- `enumerate()` ajoute l'indice (le numéro moins un). On écrit `*slot -= d_theta` au lieu de `gradient[i] -= …`.
- Clippy le demande (`needless_range_loop`) : pas d'indice hors limites possible, et l'intention se lit mieux.

**Sortir une valeur d'une table sans la copier : `HashMap::remove`.** `crates/core/src/store/loto.rs:192`.
- Les rangs de tous les tirages sont lus en une requête et rangés par date dans une `HashMap<String, Vec<RankResult>>`.
- Pour chaque tirage, `remove(&date)` rend la liste possédée : on la donne au `Draw` sans `clone()`. Chaque liste ne sert qu'une fois.

**Mettre une grosse variante dans une boîte : `Done(Box<SessionReport>)`.** `crates/core/src/loto/recommend.rs:75`.
- Une `enum` prend la taille de sa plus grosse variante. `SessionReport` est gros, `NotDue` vide.
- `Box` met le rapport sur le tas : l'`enum` ne contient plus qu'un pointeur. Clippy le signale (`large_enum_variant`) quand les variantes diffèrent de plus de 200 octets.

**`Option::is_some_and`.** `crates/core/src/loto/popularity.rs:513`.
- `slope.is_some_and(|s| s.low > MIN_SLOPE_LOW)` vaut `false` si la pente n'existe pas, sinon le résultat du test.
- Plus court et plus sûr qu'un `match` à deux branches : une pente absente ne valide jamais le critère.

**Conditions enchaînées dans un `if let` (édition 2024).** `crates/core/src/loto/expectation.rs:287`.
- `if let Some(grid) = Grid::new(chosen, chance) && !grids.contains(&grid) { … }` : le motif, puis une condition sur la variable
  qu'il vient de lier, sans `if` imbriqué.

## 2. Concepts statistiques

**Estimer le nombre de grilles d'un tirage.** `crates/core/src/loto/estimate.rs:50`.
- Chaque grille gagne au rang `k` avec la probabilité `p_k` du règlement. Le nombre de gagnants `n_k` suit une loi de Poisson de
  moyenne `N · p_k`.
- L'estimateur du maximum de vraisemblance sur plusieurs rangs est `N̂ = Σ n_k / Σ p_k`.
- Intuition : 625 569 gagnants au rang 9, qui arrive à une grille sur 10,8, font environ 6,8 millions de grilles.

**Espérance d'un jeu à jackpot forcé.** `crates/core/src/loto/estimate.rs` (`return_per_grid`) et `expectation.rs:121`.
- Le Grand Loto verse toujours ses `J` euros : au rang 1, ou aux rangs 2 à 9 (art. 8.3.4). Avec `C` euros de codes et une part `r`
  des mises reversée aux autres rangs : `E = (J + C) / N + r`.
- Le jeu est favorable sous le point mort `N* = (J + C) / (prix − r)`, de 8 à 9 millions de grilles pour 20 M€.
- Intuition : le jackpot est un gâteau de taille fixe ; moins il y a de convives, plus la part est grosse.

**Partager un gain avec des inconnus : `E[1/(1+M)] = (1 − e^{−μ})/μ`.** `crates/core/src/loto/expectation.rs:42`.
- Si `M` autres grilles gagnent aussi, et que `M` suit une loi de Poisson de moyenne `μ`, c'est la part moyenne d'une grille gagnante.
- `μ` est petit pour une grille que personne ne joue : la part tend vers 1. C'est tout l'intérêt des numéros délaissés.

**Polynômes symétriques élémentaires.** `crates/core/src/loto/popularity.rs:115`.
- `e_r(a)` est la somme des produits de `r` poids distincts : `e_2(1, 2, 3) = 1·2 + 1·3 + 2·3 = 11`.
- Si un joueur choisit `S` avec une probabilité proportionnelle à `Π_{i∈S} a_i`, la part des joueurs qui ont exactement `m`
  numéros du tirage `D` est `e_m(a_D) · e_{5−m}(a_{D̄}) / e_5(a)`.
- Avec des poids égaux, on retrouve la loi hypergéométrique du règlement : c'est le premier test.

**Vraisemblance conditionnelle : se débarrasser d'un paramètre de nuisance.** `crates/core/src/loto/popularity.rs:214`.
- Chaque tirage a son nombre de grilles `N_d`, inconnu. Maximiser la vraisemblance de Poisson en `N_d` revient à une loi
  multinomiale sur la répartition des gagnants entre les rangs : `N_d` disparaît.
- Seules comptent les **proportions** de gagnants : un tirage de numéros populaires a plus de gagnants à 3 et 4 numéros, par rapport
  aux gagnants du rang 9 (la chance seule).

**Calibration par une pente.** `crates/core/src/loto/popularity.rs:539` et `:559`.
- Pour chaque tirage de test, on compare l'écart au hasard **prédit** de la part du rang 8 à l'écart **observé**.
- Une pente de 1 : les écarts prédits se retrouvent entiers. Une pente de 0,5 : le modèle double les écarts. Sous 0 : il se trompe de
  sens. Mesuré : 0,93, et 0,88 pour quatre numéros trouvés.

**Intervalle de prédiction.** `crates/core/src/loto/estimate.rs:196`.
- Sur `n` valeurs passées de `ln N`, de moyenne `m` et d'écart type `s`, une nouvelle valeur suit une loi de Student à `n − 1`
  degrés, centrée sur `m`, d'échelle `s · √(1 + 1/n)`.
- Le `1/n` vient de l'incertitude sur la moyenne elle-même ; avec deux éditions d'été, l'intervalle est large, et c'est voulu.

## 3. Questions pour vérifier

1. Pourquoi l'espérance réalisée d'un Grand Loto passé (`versé / N`) n'est-elle pas un « coup de chance », alors qu'elle l'est pour un
   Loto ordinaire ?
2. Le gain de log-vraisemblance vaut 5 071 par tirage, un chiffre énorme. Pourquoi n'est-ce pas, à lui seul, une preuve que les
   grilles délaissées rapportent 37 % de plus ? Quelle mesure le complète, et que dit-elle aux rangs 3 et 4 ?
3. Sans les préférences, Noël 2026 donnerait « ne pas jouer » ; avec elles, « jouer ». Qu'est-ce qui, dans la section 2 du rapport,
   te ferait douter de la décision ?

## 4. Fichiers à relire en priorité

1. `crates/core/src/loto/expectation.rs` : la formule qui décide de jouer ; vérifie chaque somme de rang contre le règlement.
2. `crates/core/src/loto/popularity.rs` : le modèle, son critère fixé d'avance et le walk-forward ; c'est lui qui fait basculer Noël.
3. `crates/core/src/loto/alert.rs` : la détection du plafond par l'excédent du rang 2 et ses deux niveaux ; une erreur ici fait manquer
   le seul tirage qui compte.
