# Phase 20 — Règle Loto Foot à prix dynamique

Ce qui a été construit : le seuil τ de l'abaque, le plafond de 16 et le vivier de 64 candidats laissent la place à une seule règle. Dans
l'ordre de clôture, on prend le meilleur des `3^n` tickets tant que sa valeur marginale atteint le seuil du moment, lequel dépend du temps
et du budget qui restent dans le mois, 32 tickets au plus par grille. Sources et choix : `docs/sources.md` §69 et §70.

| Fichier | Rôle |
|---|---|
| `core/src/lotofoot/price.rs`, `core/data/lotofoot_price.csv` | table gelée des seuils, tirée de l'entraînement |
| `core/src/lotofoot/exact.rs` | valeur marginale de chaque ticket, partage entre nos tickets compris |
| `core/src/lotofoot/optimize.rs` | règle d'arrêt d'une grille |
| `core/src/lotofoot/session.rs` | passage des grilles, temps restant du mois, journal |
| `core/src/lotofoot/bulletin.rs` | regroupement glouton au-delà de 16 tickets |
| `scripts/lotofoot/net_*.py` | étude, table, contrôles |

## 1. Concepts Rust rencontrés

**`include_str!` : un fichier dans le binaire.** `crates/core/src/lotofoot/price.rs:21`.
- La macro lit le fichier à la compilation et en fait une `&'static str`. La table voyage avec le programme : pas de chemin à trouver,
  aucune version à désaccorder. Le `expect` de `frozen` est un invariant démontré : un test lit la table réelle.

**Fermeture passée en `&dyn Fn`.** `crates/core/src/lotofoot/optimize.rs:42`.
- `threshold: &dyn Fn(u32) -> f64` : « n'importe quelle fonction de ce type, empruntée ». La séance donne une fermeture qui capture
  la table et le temps restant ; les tests donnent `&|_| 1.5`. On évite ainsi un paramètre générique, avec un coût d'appel négligeable ici.

**Fermeture qui capture par emprunt.** `crates/core/src/lotofoot/session.rs:502`.
- `bar` lit `request`, `prices` et `steps` sans les copier. `unwrap_or_else` ne calcule la table que si aucun seuil n'est imposé.

**Deux emprunts en parallèle : `zip(&mut values)`.** `crates/core/src/lotofoot/session.rs:492`.
- `ready.iter()` prête les grilles en lecture, `&mut values` prête chaque table en écriture : `pick` y note les tickets pris. Le
  compilateur accepte, car ce sont deux vecteurs distincts ; `report.months` est emprunté à part, champ par champ.

**`Option::filter`.** `crates/core/src/lotofoot/session.rs:495`.
- `budget.filter(|_| !grid.young)` transforme `Some` en `None` si la condition échoue ; le `let … else` traite d'un coup « mois
  introuvable » et « grille trop jeune ».

**`HashSet` et `insert` qui répond.** `crates/core/src/lotofoot/bulletin.rs:216` et `:226`.
- `insert` renvoie `false` si l'élément existait : une seule opération pour tester et ajouter. `clone` : `lines` et `seen` possèdent
  chacun leur exemplaire. La recherche passe d'un parcours de liste à un accès direct, indispensable à 32 tickets.

## 2. Concepts statistiques

**Valeur marginale et concavité.** `crates/core/src/lotofoot/exact.rs:81`.
- `V(T) = Σ_t P(t) M_0 E[1/(1 + W_0)] + Σ_o P(o) M_1 F(c(o), μ)`, `F(c, μ) = c · E[1/(c + W)]`. `F` est concave : chaque ticket de
  plus au même rang rapporte un peu moins. D'où la validité du glouton, à 0,07 % au plus de la borne de relaxation linéaire.

**Intégrale de `F`.** `crates/core/src/lotofoot/exact.rs:126`.
- `E[1/(c + W)] = ∫₀¹ x^{c−1} e^{−μ(1−x)} dx`, évaluée par la méthode de Simpson. Pour `c = 1`, on retrouve `(1 − e^{−μ})/μ`.

**Sac à dos en ligne et prix dual.** `crates/core/src/lotofoot/price.rs:90`.
- Les grilles arrivent une à une et le budget est limité. Le seuil vaut `1 + W_t(b) − W_t(b − 1)` : le ticket doit rendre son euro et
  ce que cet euro rapporterait plus tard. Avec beaucoup de budget ou peu de temps, l'euro vaut peu et le seuil baisse. τ était le cas
  d'un prix constant.

**Malédiction de l'optimiseur.** (Smith & Winkler 2006) En choisissant le maximum d'estimations bruitées, on surestime ce qu'on choisit.
La correction ρ essayée sur l'étude est restée à 1 : aucune utile ici, sur des entrées bruitées comme en production.

**Ce que l'optimisation ne fait pas.** Elle multiplie une espérance et n'en crée pas : le test figé du §37 reste échoué, et les années en
perte passent de 55 à 57 % à 100 € par mois.

## 3. Questions pour vérifier

1. Pourquoi deux tickets distincts ne peuvent-ils jamais se partager un sans-faute, et que devient alors `c` au rang une erreur ?
2. Le 28 du mois, il reste 80 € : le seuil est-il plus haut ou plus bas que le 2 du mois avec 80 € ? Pourquoi ?
3. Pourquoi le même plafond donne-t-il presque le même nombre de grilles à remplir, alors qu'il y a plus de tickets par grille ?

## 4. Fichiers à relire en priorité

1. `crates/core/src/lotofoot/exact.rs` : le cœur mathématique, la formule en tête du fichier se retrouve ligne par ligne.
2. `crates/core/src/lotofoot/optimize.rs` : la règle tient en une boucle de quelques lignes.
3. `crates/core/src/lotofoot/session.rs:489` : le branchement du temps, du budget et de la grille, et ce qui part au journal.
