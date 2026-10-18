# Phase 6 — Application egui

Ce qui a été construit : une application de bureau (`edge-gui`) avec cinq onglets — Recommandations, Paris, Bankroll, CLV et
calibration, Rapports. Elle lit et écrit le journal par l'API d'`edge-core`, n'appelle jamais l'API de cotes (aucun crédit
dépensé) et ne parie jamais. Avant elle, la logique de saisie des paris a quitté la CLI pour `core` (`betting/ledger.rs`), et la
courbe de bankroll et le diagramme de fiabilité y sont nés (`betting/overview.rs`).

## 1. Concepts Rust rencontrés

**Une interface en « mode immédiat ».** `crates/gui/src/app.rs:131` — egui n'a pas d'arbre de widgets conservé : à chaque image,
`App::ui` redessine tout depuis l'état (`draw`, `:78`). Un bouton n'a pas de gestionnaire de clic : `ui.button("…").clicked()`
répond `true` pendant l'image où le clic a lieu. L'état (formulaire ouvert, onglet choisi) vit dans de simples structures.

**Décrire plutôt qu'exécuter : une énumération d'actions.** `crates/gui/src/actions.rs:19` et `:56`. Les onglets reçoivent des
données en lecture seule et renvoient des `Action` (`LogFollowed`, `SettleManual`…) ; seule `App::run` (`app.rs:59`) les exécute
sur la base. L'affichage ne touche donc jamais le journal, et `apply` se teste sans fenêtre. Le `match` exhaustif garantit
qu'une action ajoutée sera traitée.

**Emprunts disjoints de champs dans une fermeture.** `app.rs:78` (`draw`). La fermeture du panneau central lit `self.data` et
modifie `self.states` en même temps : Rust, depuis l'édition 2021, emprunte les champs séparément, tant qu'on les nomme
(`self.data`, `self.states`) au lieu de passer `self` entier.

**`Iterator::partition`.** `crates/gui/src/data.rs:97` : une seule passe sépare les prix notifiés (avantage ≥ 3 %) des autres.

**Tri sur deux clés.** `data.rs:138` : `sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)))` trie par
date décroissante, puis par nom pour départager.

**Une crate avec son propre `rust-version`.** `crates/gui/Cargo.toml:7`. Le workspace fixe 1.89 (le cœur) ; `eframe` exige 1.95. Seule
la crate `gui` le déclare : le cœur n'a pas à monter de version pour une interface.

**Fermetures `move` et durées de vie dans un graphique.** `crates/gui/src/tabs/clv.rs:53`. Le formateur d'axe doit posséder la liste
des étiquettes (`move`), car le graphique est dessiné plus tard que la création de la fermeture. `PlotPoints<'_>`
(`tabs/bankroll.rs:47`) porte une durée de vie : il peut emprunter les points plutôt que les copier.

**Garde à la destruction.** `app.rs:162`. Un `FullOutput` d'egui panique s'il est détruit sans que ses textures aient été
appliquées : un test sans fenêtre doit dire explicitement `drop_without_applying_deltas()`. C'est le motif « `Drop` qui refuse
l'oubli ».

## 2. Concepts statistiques

**Bankroll réalisée et bankroll disponible.** La courbe ne bouge qu'au règlement d'un pari (gain net réalisé). La bankroll
*disponible* retire en plus les mises encore ouvertes : c'est elle qui sert à dimensionner les prochaines mises
(`betting/overview.rs:42`). Les deux ne sont pas la même chose, et les confondre fait miser deux fois le même argent.

**Drawdown.** La plus forte baisse relative depuis un sommet de la bankroll réalisée (`max_drawdown`). Ce n'est pas une perte
finale : c'est la pire descente que tu aurais dû supporter. Le départ compte comme premier sommet.

**Diagramme de fiabilité.** On regroupe les recommandations réglées par probabilité annoncée et on compare cette probabilité à la
fréquence observée. Sur la diagonale, la probabilité est fiable. Avec peu de recommandations réglées, chaque point vaut quelques
paris : l'effectif est affiché à côté, et le graphique ne conclut rien tant qu'il est petit (`overview.rs:88`).

**CLV par tranche d'avantage.** Si l'avantage annoncé est réel, le CLV doit *monter* avec lui. S'il baisse, ou reste plat, l'avantage
annoncé n'était qu'une erreur de prix : c'est le diagnostic qui a rejeté le modèle en phase 4.

**Prudence sur les petits effectifs.** Un intervalle de confiance calculé sur 5 paris est mathématiquement valide et pratiquement
inutile. L'onglet CLV marque « indicatif » tout groupe de moins de 30 clôtures (`tabs/clv.rs`, `MIN_FOR_CONFIDENCE`).

## 3. Trois questions pour vérifier que tu as compris

1. Pourquoi les onglets renvoient-ils des `Action` au lieu d'appeler la base eux-mêmes ? Cite deux conséquences concrètes sur les
   tests et sur ce qui peut arriver à ton journal.
2. Ta bankroll de départ est de 1 000 €, tu as un pari gagné (+60 €), puis un pari perdu (−50 €), et un pari ouvert (50 €). Que valent
   la bankroll réalisée, la bankroll disponible et le drawdown ? Laquelle sert à calculer la prochaine mise ?
3. Le diagramme de fiabilité montre un seul point à (0,45 ; 100 %) construit sur deux recommandations. Pourquoi ne faut-il rien en
   conclure, et quel autre onglet te donne une mesure plus tôt ?

## 4. Les trois fichiers à relire en priorité

1. `crates/gui/src/actions.rs` — le seul endroit où l'application écrit dans le journal, et ses tests : saisie suivie, hors modèle,
   règlement manuel, refus.
2. `crates/core/src/betting/ledger.rs` — les règles de saisie partagées par la CLI et l'application : cote minimale, mise positive,
   un seul pari par recommandation. Si une règle change, elle change aux deux endroits à la fois.
3. `crates/gui/src/app.rs` — la boucle : onglets, bandeau de messages, exécution des actions, relecture des données, et les tests de
   rendu sans fenêtre.
