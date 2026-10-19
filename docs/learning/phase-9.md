# Phase 9 — Refonte de l'interface (design, animations, capture d'écran)

Ce qui a été construit : `edge-gui` repensée de bout en bout, sans changer ce qu'elle sait faire. Un **système de design**
(`crates/gui/src/design/` : couleurs, espacements, typographie, mouvement, icônes dessinées, composants), cinq **écrans** dans
`pages/` (le verdict du jour, les opportunités, les paris, la performance, les rapports), un **mode démo** (`--demo`, données
fictives, rien lu ni écrit) et une **photographie automatique** des écrans (`--screenshot DOSSIER`) qui m'a permis de *voir* le
résultat au lieu de le deviner. Choix de Paul : sombre élégant, accueil « le verdict du jour », animations fluides et sobres.

## 1. Concepts Rust rencontrés

**Un trait pour ajouter une méthode à un type étranger.** `design/style.rs:18` (`Semibold`). `RichText` appartient à egui : on ne
peut pas lui ajouter de méthode directement, mais on peut définir un trait et l'implémenter pour lui. L'appelant écrit
`.semibold()` comme une méthode ordinaire, à condition d'importer le trait. C'est le seul trait nouveau, et il en a un seul
implémenteur : c'est ici une commodité d'écriture, pas une abstraction.

**Embarquer un fichier dans le binaire.** `design/style.rs:31` (`install_fonts`). `include_bytes!` lit le fichier **à la compilation**
et le range dans le programme sous forme de `&'static [u8]` ; `FontData::from_static` l'emprunte sans le copier. Le binaire ne
dépend d'aucun fichier à l'exécution.

**Fermetures passées par paramètre, appelées une seule fois.** `design/widgets.rs:147` (`split`). `impl FnOnce(&mut Ui)` : la
fonction reçoit deux morceaux de code à exécuter chacun une fois, dans son propre sous-affichage. `FnOnce` (et non `Fn`) laisse
la fermeture consommer ce qu'elle capture, par exemple pousser dans le `Vec` de requêtes de l'appelant.

**Un trait objet évité, un indice choisi à la place.** `design/widgets.rs:132` (`tile_columns`) : pour passer d'une rangée de trois
tuiles à une pile, on ne fabrique pas de tableau de `dyn FnMut` ; on demande `n` colonnes (3 ou 1) et chaque tuile prend la
colonne `i % n`. Le plus simple qui marche, conformément à CLAUDE.md.

**Un test qui mesure.** `app.rs:737` (`no_screen_overflows_…`). Le test dessine chaque écran dans des fenêtres de 640, 855, 1240 et
1900 points et vérifie que la zone réellement occupée (`Context::globally_used_rect`) ne dépasse pas la fenêtre. C'est ce test
qui a trouvé les deux derniers débordements (une rangée de pastilles, une ligne de journal).

**Un encodeur PNG écrit à la main.** `screenshot.rs:53`. egui rend une image en mémoire (`ColorImage`) ; pour la voir, il faut un
fichier. Plutôt qu'une dépendance (`png`, `image`), l'encodeur n'écrit que des blocs « stockés » (non compressés) avec les
sommes de contrôle `crc32` (`:14`) et `adler32` (`:36`) : une trentaine de lignes, testées par aller-retour.

## 2. Concepts de conception (et un concept statistique)

**Le mode immédiat et l'animation.** Dans egui, rien n'est « animé » par le système : à chaque image on redessine à partir d'un
temps. `design/motion.rs:43` (`progress(délai, durée)`) rend une valeur de 0 à 1 ; `ease_out` (`:35`) la fait décélérer ; `reveal`
(`:75`) décale l'apparition de chaque bloc de 50 ms (cascade) et le fait monter de 10 points. Toutes les durées restent entre 150 et
300 ms, sauf les graphiques qui se tracent (0,9 s) et les chiffres qui défilent (0,7 s). Un réglage « réduire les animations »
ramène tout à 1 immédiatement. Ce sont des choix du projet, pas des prescriptions : les chiffres de durée viennent de l'usage
courant, voir `docs/sources.md` §24.

**Hiérarchie par la couleur et la taille, pas par les bordures.** Des gris teintés de bleu (jamais du gris pur) forment trois
niveaux de surface (`fond`, `carte`, `relevé`) ; une seule couleur d'accent (indigo) désigne ce qui se clique ; vert, rouge, ambre
ne servent qu'à dire *gain, perte, prudence*. Échelle d'espacement 4-8-12-16-24-32 et rayons 14 (cartes) / 9 (contrôles).

**Contraste mesuré, pas supposé.** `design/tokens.rs:168` : le rapport de contraste WCAG est
`(L_clair + 0,05) / (L_sombre + 0,05)`, avec la luminance relative `L = 0,2126 R + 0,7152 G + 0,0722 B` (canaux linéarisés). Le test
exige 4,5 :1 (niveau AA du texte courant) pour chaque couleur de texte sur chaque surface, dans les deux thèmes. Il a fait
éclaircir le gris tertiaire du thème sombre (4,49 → 4,85 sur la surface relevée) et assombrir celui du thème clair (4,22 → 4,81).

**Mise en page qui s'adapte (fenêtres tuilées).** Sous Hyprland la fenêtre fait ici 855 points, pas 1240. Trois règles : la barre
latérale se réduit aux icônes sous 1000 points (`app.rs:40`) ; `split` met un bloc à droite *s'il reste 240 points à gauche*, sinon
le range dessous ; les rangées de trois tuiles s'empilent sous 620 points. Piège rencontré : un `horizontal_wrapped` ne passe à la
ligne que devant un composant *simple* ; une pastille faite d'un cadre contenant un `horizontal` déborde (`status_chip`,
`widgets.rs:221`, la dessine d'un seul tenant).

**Concept statistique : ne pas colorier le bruit.** Le CLV réel n'est vert ou rouge que si son intervalle à 95 % **exclut zéro**
(`MeanCi::excludes_zero`, utilisé dans `pages/today.rs:302` et `pages/performance.rs:221`). Sinon le chiffre reste neutre et la
légende dit « pas distinct de zéro ». L'interface ne doit pas suggérer un avantage que la mesure n'établit pas (invariants 2 et 5
de CLAUDE.md).

## 3. Trois questions

1. Dans `split`, pourquoi ne pas simplement écrire `ui.with_layout(Layout::right_to_left(…))` pour le bloc de droite ? Que se passe-t-il
   avec le texte de gauche quand la fenêtre se rétrécit ?
2. Le test de non-débordement a échoué sur « Today occupe 700 points dans une fenêtre de 640 » alors que la mise en page semblait
   correcte à 855. Pourquoi une pastille construite avec `Frame::show` + `horizontal` ne passe-t-elle pas à la ligne ?
3. Pourquoi le CLV de la démo (+0,31 %, 48 clôtures) s'affiche-t-il en gris avec « pas distinct de zéro » alors que la moyenne est
   positive ? Que faudrait-il observer pour qu'il devienne vert ?

## 4. Les trois fichiers à relire en priorité

1. `crates/gui/src/design/widgets.rs` : tous les composants ; `card`, `split` et `segmented` suffisent à comprendre le reste.
2. `crates/gui/src/design/motion.rs` : le mouvement tient en quatre fonctions ; c'est la partie la plus transposable.
3. `crates/gui/src/pages/today.rs` : un écran complet, de la phrase de verdict aux trois indicateurs, avec la règle « couleur
   seulement si l'intervalle exclut zéro ».

## Limites connues

- Les animations ont été vérifiées par des tests de progression et par captures d'écran (état final), pas à l'œil sur plusieurs
  secondes : à juger par Paul.
- Les captures automatiques ont été faites dans une fenêtre tuilée de 855 × 958 points ; la mise en page large (1240 et plus) est
  couverte par le test de non-débordement, pas par une capture.
- Le bureau (Hyprland, Wayland) fait clignoter brièvement la fenêtre pendant `--screenshot`.
