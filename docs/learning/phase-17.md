# Phase 17 — Page « Grand Loto » (`/loto`)

Ce qui a été construit : un onglet de l'application Web pour le Grand Loto. Il montre la prochaine édition, la séance décidée par le
timer (grilles en boules, espérance, popularité, état), les grilles jouées et leur règlement, la série EuroMillions au plafond et les
Grand Loto passés. Deux formulaires écrivent : « J'ai joué la séance » et la déclaration d'une grille saisie à la main.

| Fichier | Rôle |
|---|---|
| `core/src/web/pages/loto.rs` | la vue : lecture du journal et de l'archive, mise en forme |
| `core/src/web/templates/loto.html` | le gabarit |
| `core/src/web/forms.rs` (`LotoGridForm`) | la grille saisie → action validée |
| `core/src/web/actions.rs` (`LotoPlayedSession`, `LotoGrid`) | les écritures, par `loto::played` |
| `core/src/web/mod.rs` | les routes `/loto`, `/loto/jouee`, `/loto/grille` |
| `core/src/loto/alert.rs` (`current_streak`), `report/loto_report.rs` (`edition_rows`) | calculs partagés avec la CLI |

## 1. Concepts Rust rencontrés

**Lire un JSON sans structure : `serde_json::Value` et l'opérateur `[]`.** `crates/core/src/web/pages/loto.rs:258`.
- `notes["forecast"]["central"]` descend dans le JSON de la séance. Une clé absente donne `Value::Null`, jamais une panique.
- `.as_f64()` rend ensuite un `Option<f64>` : une note manquante s'affiche « — » au lieu de faire échouer la page.
- On évite ainsi de dériver `Deserialize` sur tous les types du modèle pour une page qui ne fait qu'afficher.

**Motif de tranche : `[a, b, c, d, e, chance] => …`.** `crates/core/src/web/forms.rs:421`.
- `match numbers.as_slice()` compare la tranche à un motif de longueur fixe : il ne correspond qu'à exactement six éléments, et
  nomme chacun.
- Toute autre longueur tombe dans `_ => None` : pas d'indice hors limites possible.

**Découper sur une condition : `split(|c: char| !c.is_ascii_digit())`.** `crates/core/src/web/forms.rs:416`.
- `split` accepte une fermeture : tout caractère qui n'est pas un chiffre sépare deux nombres. `2-3-30-35-44+10` et `2 3 30 35 44 10`
  donnent la même grille.

**Lier une valeur dans un `match` : `n @ 1..=2`.** `crates/core/src/web/pages/loto.rs:378`.
- Le motif teste l'intervalle et garde la valeur dans `n` pour l'utiliser dans le message.

**Chaîner deux recherches : `and_then(…).or_else(…)`.** `crates/core/src/web/pages/loto.rs:156`.
- La séance de la prochaine édition si elle existe, sinon (`or_else`) la plus récente. `or_else` prend une fermeture : la seconde
  recherche n'est faite que si la première échoue.

## 2. Concepts statistiques

Aucun nouveau : la page affiche ce que la séance a décidé (phase 16). Un point de méthode, toutefois :

**Afficher la décision, pas la recalculer.** `crates/core/src/web/pages/loto.rs:244`.
- La séance est lue dans le journal, avec la prévision et les espérances du moment où elle a été décidée (invariant 6).
- Recalculer à l'affichage montrerait des chiffres qui ne sont pas ceux sur lesquels on a joué : les Loto des semaines suivantes
  déplacent un peu les préférences. Le walk-forward prend aussi plusieurs secondes.

## 3. Questions pour vérifier

1. Tu ajoutes une édition dans `edge.toml` : pourquoi la page ne la montre-t-elle pas tout de suite, et que faut-il faire ?
2. Tu déclares à la main une grille identique à une grille recommandée : comment est-elle comptée, et si tu la déclares une seconde
   fois ?
3. Pourquoi la page lit-elle la séance dans ses notes JSON plutôt que de relancer `recommend::evaluate` ?

## 4. Fichiers à relire en priorité

1. `crates/core/src/web/pages/loto.rs` : ce que la page affiche et d'où vient chaque chiffre.
2. `crates/core/src/web/forms.rs` (`LotoGridForm`) : la seule saisie libre de la page.
3. `crates/core/src/web/mod.rs` (routes `/loto`) : quelles adresses écrivent, et par quelle action.
