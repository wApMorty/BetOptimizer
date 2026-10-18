# Phase 3 — Modèles

Ce qui a été construit : un modèle de Poisson et un modèle de Dixon-Coles (correction ρ des petits scores,
décroissance temporelle ξ, régularisation L2), ajustés par maximum de vraisemblance avec L-BFGS (`argmin`) et un
gradient analytique ; une variante « informée par les tirs » ; un walk-forward hebdomadaire sans fuite du futur ;
un journal immuable des prédictions ; le réglage des hyperparamètres sur la validation 2024/25 ; l'évaluation figée
sur le test ; et `edge model report`, qui compare les modèles au marché avec des intervalles de confiance.

## 1. Concepts Rust rencontrés

**Implémenter les traits d'une bibliothèque externe** — `crates/core/src/models/likelihood.rs:149` et `:158`.
`argmin` ne connaît pas notre problème : il attend un objet qui sait calculer un coût (`CostFunction`) et un gradient
(`Gradient`). On implémente ces deux traits pour `LikelihoodProblem`, et `argmin` pilote le reste (`:186`). C'est
l'usage « légitime » de l'implémentation de traits externes prévu par CLAUDE.md : aucun trait inventé, on se branche
sur un contrat existant.

**Structure qui emprunte des données (`'a`)** — `crates/core/src/models/likelihood.rs:37` et `:44`.
`LikelihoodProblem<'a>` garde une référence `&'a [TrainingGame]` plutôt qu'une copie de milliers de matchs. Le `'a`
dit que le problème ne peut pas survivre aux données. `impl<'a> … ` (`:44`) introduit ce même `'a` pour les méthodes.

**`match` sur un tuple** — `crates/core/src/models/likelihood.rs:127`.
`match (buts_dom, buts_ext) { (0, 0) => …, (0, 1) => …, _ => None }` énumère les quatre scores corrigés par τ et
laisse tous les autres au cas par défaut : le compilateur vérifie qu'aucun cas n'est oublié.

**Trait avec supertrait, objet de trait et fils d'exécution** — `crates/core/src/models/mod.rs:96`,
`crates/core/src/models/walk_forward.rs:71` et `:140-143`.
`trait MatchModel: Sync` exige que tout modèle puisse être lu depuis plusieurs fils. `Box<dyn MatchModel>` permet
de choisir Poisson ou Dixon-Coles à l'exécution. `thread::scope` lance un fil par ligue en leur prêtant des
références : le compilateur garantit que les fils se terminent avant que les données ne disparaissent. Pas de
`tokio`, pas de verrou : chaque fil calcule sa ligue seul, puis on réassemble dans l'ordre.

**Fermeture qui modifie son environnement** — `crates/core/src/models/data.rs:98`.
`index_of` est une fermeture `mut` qui complète la table des équipes au fur et à mesure : elle emprunte la table de
façon exclusive pendant qu'elle existe.

**`Option::zip`** — `crates/core/src/models/data.rs:130`. `xg.0.zip(xg.1)` donne `Some((a, b))` seulement si les
deux valeurs sont présentes : un xG publié à moitié est ignoré.

**`clamp`** — `crates/core/src/models/mod.rs:81` et `:85`.
`rho.clamp(bas, haut)` ramène une valeur dans un intervalle. Ici il garde ρ dans le domaine où τ reste positif.

**`Box` pour alléger une énumération** — `crates/core/src/ingest/football_data/parse.rs:146` et `:200`.
Clippy a signalé qu'une variante contenait un gros `ParsedMatch` alors que les autres étaient vides : chaque
valeur de l'énumération aurait occupé la taille de la plus grande. `Box<ParsedMatch>` déplace le contenu sur le tas.

**Fonction générique avec contrainte** — `crates/core/src/report/render.rs:47`.
`render_template<T: Serialize>` rend n'importe quel rapport qui sait se sérialiser : un seul code pour les deux gabarits.

**`total_cmp` pour trier des `f64`** — `crates/core/src/report/models_report.rs:364`.
Un `f64` n'est pas `Ord` (à cause de NaN) ; `a.total_cmp(&b)` donne un ordre total, ce qui permet `sort_by` et `min_by`.

**Chaînes `if let … && …`** — `crates/core/src/report/models_report.rs:352`.
Possibles depuis Rust 1.88 en édition 2024 : on teste deux conditions de forme dans un même `if`, sans imbrication.

**Utilitaires de test partagés** — `crates/core/src/models/mod.rs:22`.
`#[cfg(test)] pub(crate) mod testing` n'existe que pour les tests : ligues simulées déterministes à paramètres connus,
réutilisées par les tests du walk-forward, du réglage et du rapport.

## 2. Concepts statistiques

**Modèle de Poisson des buts.** Les buts d'une équipe suivent une loi de Poisson de moyenne `λ` (domicile) et `μ`
(extérieur) : `log λ = base + avantage + attaque_dom + défense_ext`. Une équipe a une force d'attaque et une
faiblesse de défense ; l'avantage du terrain est commun à la ligue.

**Dixon-Coles (1997).** Les buts ne sont pas tout à fait indépendants : les petits scores (0-0, 1-1) sont plus
fréquents et 1-0 / 0-1 moins fréquents qu'avec deux Poisson indépendantes. Le facteur `τ(x, y)` corrige cela avec un
seul paramètre `ρ` (`crates/core/src/models/grid.rs:30`). Valeurs publiées : de −0,03 à −0,2.

**Pondération temporelle.** Un match vieux de `t` jours pèse `exp(−ξ t)` : la forme récente compte davantage. La
demi-vie vaut `ln 2 / ξ` (ξ = 0,002 → 347 jours).

**Régularisation L2 = croyance a priori.** Ajouter `(κ/2)·Σ(attaque² + défense²)` à ce que l'on minimise revient à
supposer que les équipes sont, sans preuve du contraire, « moyennes ». Une équipe promue, sans match dans la ligue,
est ramenée à 0 ; plus elle joue, plus les données l'emportent sur cette croyance.

**Vraisemblance, gradient et L-BFGS.** On cherche les paramètres qui rendent les scores observés les plus probables.
Le gradient (dérivée par rapport à chaque paramètre) est écrit à la main (`likelihood.rs:62`) et **vérifié par
différences finies** dans les tests : c'est la protection contre une erreur de signe qui ferait converger vers n'importe quoi.

**Walk-forward et fuite du futur.** Chaque semaine, le modèle n'a vu que les matchs strictement antérieurs au lundi
(`crates/core/src/models/data.rs:141` et `:151`). Un test réécrit tous les résultats futurs et vérifie que les
prédictions antérieures sont identiques au bit près : si la moindre information du futur s'y glissait, il échouerait.

**Réglage, validation, test.** On choisit les hyperparamètres (ξ, κ, mélange de tirs : 64 configurations) sur la
validation 2024/25, on les **gèle**, puis on évalue une seule fois sur le test (`crates/core/src/models/tuning.rs:41`,
`:147`, `:230`). Chaque configuration essayée est une occasion de s'ajuster au hasard : on compte et on rapporte.

**Test d'information (combinaison géométrique).** Comparer un modèle seul au marché est sévère. La bonne question :
en mélangeant `p ∝ p_marché^(1−w) · p_modèle^w`, fait-on mieux que le marché seul ? Si le meilleur `w` (réglé sur la
validation) est 0, le modèle ne sait rien que le marché ignore.

**Résultats mesurés (2026-10-18).** Validation 2024/25 (1 980 matchs) : meilleur Dixon-Coles 0,9814 ; Poisson
0,9825 ; Dixon-Coles à buts seuls 0,9826. **Test** (2 325 matchs, juillet 2025 à octobre 2026) : moyenne du marché à
la clôture 0,9744 ; Dixon-Coles 0,9931 ; Poisson 0,9926 ; buts seuls 0,9923 ; fréquences passées 1,0713. Le modèle est
moins bon que le marché de 0,0188 [0,0130 ; 0,0251] ; les trois modèles sont indiscernables entre eux (les tirs
n'aident pas) ; le poids optimal du modèle dans la combinaison est **0,00** à la clôture comme à l'avant-match.
Seule la Premier League n'est pas significativement distincte du marché (440 matchs, intervalle large).

## 3. Trois questions pour vérifier que tu as compris

1. « Le modèle est moins bon que le marché » et « le poids optimal du modèle dans la combinaison est 0 » : pourquoi
   le second résultat est-il plus fort que le premier ? Que signifierait un poids de 0,3 avec un écart de log loss
   négatif ?
2. Le test de fuite réécrit les matchs à partir d'une date de coupure, puis vérifie deux choses : les prédictions
   d'avant la coupure ne changent pas, **et** celles d'après changent. Pourquoi la seconde vérification est-elle
   indispensable ?
3. Pourquoi ρ est-il borné à ±0,25 pendant l'ajustement, puis encore ramené dans le domaine faisable au moment de
   prédire ? Qu'est-ce qu'une « probabilité négative » aurait cassé dans la grille de scores ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/models/likelihood.rs` — la vraisemblance, son gradient, et le branchement sur `argmin` : le cœur
   mathématique et le meilleur exemple de test numérique (différences finies, récupération de paramètres simulés).
2. `crates/core/src/models/walk_forward.rs` — comment on prédit sans tricher : une semaine, un ajustement, aucune
   fuite ; ses tests décrivent chaque garantie.
3. `crates/core/src/report/models_report.rs` — comment on juge un modèle contre le marché : mêmes matchs, intervalles
   par blocs, et le test d'information en fin de fichier.
