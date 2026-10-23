# Phase 15 — Application Web (`edge serve`) à la place d'edge-gui

Ce qui a été construit : l'application d'edge devient un site, consulté depuis le PC de la maison. Mêmes écrans que l'ancienne
application egui (Aujourd'hui, Opportunités, Paris, Performance, Bonus, Loto Foot), plus le paper trading, la latence et les
rapports. Les saisies se font par des formulaires classiques, sans script : le serveur calcule chaque page et la renvoie toute faite.

Le principe d'architecture ne change pas : tout est dans le cœur, testable sans réseau. Le serveur HTTP de la CLI ne fait que
traduire une requête en `web::Request` et une `web::Response` en réponse HTTP.

| Fichier | Rôle |
|---|---|
| `core/src/web/mod.rs` | types `Request`, `Response`, routage (`handle`) |
| `core/src/web/auth.rs` | connexion, cookies, contrôle d'origine |
| `core/src/web/form.rs` | décodage des champs envoyés par le navigateur |
| `core/src/web/forms.rs` | formulaires de saisie → `Action` validée |
| `core/src/web/actions.rs` | les écritures, par les fonctions du cœur |
| `core/src/web/pages/*.rs` | une page par onglet, et les pages de saisie (`entry.rs`) |
| `core/src/web/charts.rs` | graphiques SVG écrits à la main |
| `core/src/report/theme.rs` | palette commune (site, rapport, images) |
| `core/src/report/lotofoot_image.rs` | image de la séance Loto Foot, en bulletin |
| `cli/src/serve.rs` | la traduction HTTP, avec `tiny_http` |

## 1. Concepts Rust rencontrés

**Une structure qui emprunte : `Request<'a>`.** `crates/core/src/web/mod.rs:69`.
- Les champs `target`, `cookies`, `origin`… sont des `&'a str` : ils pointent dans la requête de `tiny_http`, sans copie.
- Le paramètre de durée de vie `'a` dit au compilateur que la `Request` ne peut pas vivre plus longtemps que ce qu'elle emprunte.
- C'est sans risque ici : la `Request` est construite, passée à `handle`, puis abandonnée avant que la requête HTTP ne soit
  consommée par `respond`.

**L'ordre des emprunts dans `handle` du serveur.** `crates/cli/src/serve.rs:59` à `:115`.
- Lire le corps demande `&mut request` (un emprunt exclusif).
- Lire les en-têtes demande `&request` (un emprunt partagé).
- On lit donc le corps d'abord. Ensuite seulement on emprunte les en-têtes pour construire la `web::Request`.
- Enfin `request.respond(response)` prend la requête par valeur : après cette ligne, le compilateur interdit d'y toucher.

**Lire au plus N octets : `take`.** `crates/cli/src/serve.rs:63`.
- `reader.take(MAX_BODY + 1)` enveloppe le lecteur : il s'arrête après ce nombre d'octets, même si le client en envoie plus.
- Lire un octet de plus que la limite permet de savoir que la limite est dépassée, et de répondre 413.

**`#[serde(flatten)]` : aplatir une structure dans une autre.** `crates/core/src/web/view.rs:43`, `pages/lotofoot.rs:49`.
- Chaque page contient un `Layout` (onglet actif, message, heure).
- Avec `flatten`, ses champs sont sérialisés au même niveau que ceux de la page. Le gabarit écrit `{{ active }}`, pas
  `{{ layout.active }}`.
- La page Loto Foot aplatit même deux structures : le cadre et la vue déjà construite par `report::lotofoot_page`.

**Comparaison en temps constant.** `crates/core/src/web/auth.rs:30`.
- `a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y))` accumule toutes les différences, au lieu de s'arrêter à la première
  comme `==`.
- La durée de la comparaison ne dit donc pas combien de caractères étaient justes.

**`?` sur une `Option`.** `crates/core/src/web/mod.rs:318`.
- Dans une fonction qui renvoie `Option`, `?` rend `None` dès qu'une étape échoue.
- `path.strip_prefix(prefix)?.strip_suffix(suffix)?.parse().ok()` lit le numéro de `/paris/12/regler` en une ligne. Aucun
  `if let` imbriqué.

**Une énumération dont les variantes portent des données : `Reply`.** `crates/core/src/web/pages/entry.rs:34`.
- Une saisie aboutit à trois issues : écrite (redirection et message), à réafficher (page et statut), ou introuvable.
- Le `match` de `reply` (`mod.rs:308`) est exhaustif. Une quatrième variante ajoutée un jour ne compilerait pas tant qu'elle
  n'y est pas traitée.

**`bool::then` : une valeur seulement si une condition tient.** `crates/core/src/web/pages/performance.rs:133`.
- `(!curve.points.is_empty()).then(|| …)` donne `Some(graphique)` ou `None`.
- La fermeture n'est exécutée que si la condition est vraie : rien n'est calculé pour rien.

**Découper sans paniquer : `slice.get(range)`.** `crates/core/src/web/form.rs:51`.
- `bytes.get(i + 1..i + 3)` renvoie `None` si la plage dépasse la fin, là où `bytes[i + 1..i + 3]` paniquerait.
- Un `%` tronqué en fin de champ est donc gardé tel quel.

## 2. Concepts statistiques et de calcul

**Contraste WCAG.** `crates/core/src/report/theme.rs:140`.
- Luminance relative : L = 0,2126 R + 0,7152 G + 0,0722 B, chaque canal linéarisé :
  c/12,92 si c ≤ 0,03928, sinon ((c + 0,055)/1,055)^2,4.
- Contraste = (L_clair + 0,05) / (L_sombre + 0,05). Le test exige 4,5:1 pour tout texte, sur toute surface des deux palettes.
- Intuition : l'œil est bien plus sensible au vert qu'au bleu, d'où les poids.

**Graduations « rondes ».** `crates/core/src/web/charts.rs:35`.
- Pas brut r = (max − min) / 4, puis m = 10^⌊log10 r⌋, puis le premier pas parmi m, 2m, 5m, 10m qui dépasse r (Heckbert, 1990).
- Les graduations tombent sur 1 000, 1 100, 1 200 plutôt que sur 1 003,7.

**Lire le diagramme de fiabilité** (onglet Performance, vue Calibration).
- En abscisse, la probabilité annoncée ; en ordonnée, la fréquence observée, par tranche.
- Sur la diagonale, la probabilité est fiable ; au-dessus, l'issue arrive plus souvent qu'annoncé.
- Avec quelques dizaines de paris par tranche, l'écart à la diagonale est surtout du bruit : la CLV reste la mesure principale.

**Une grille multiple paie comme ses tickets** (image Loto Foot, §39 et §40 de `docs/sources.md`).
- Le prix d'une colonne est le produit du nombre de signes cochés par rencontre (8 € = 2 × 2 × 2).
- La somme des colonnes redonne toujours le nombre de tickets de la séance.

## 3. Questions pour vérifier la compréhension

1. Pourquoi un formulaire envoyé depuis un autre site est-il refusé deux fois plutôt qu'une, et lequel des deux contrôles a
   failli tout bloquer pendant les essais ?
2. Après « J'ai parié », le serveur répond `303` vers `/paris` au lieu d'afficher directement la page. Qu'arriverait-il si tu
   actualisais la page dans l'autre cas ?
3. Dans `cli/src/serve.rs`, pourquoi faut-il lire le corps de la requête **avant** de construire la `web::Request` ? Que dirait le
   compilateur dans l'ordre inverse ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/web/mod.rs` : tout le trajet d'une requête (connexion, `GET`, `POST`, réponses). C'est la carte du module.
2. `crates/core/src/web/auth.rs` : court, et c'est lui qui protège l'écriture au journal ; ses tests décrivent chaque refus.
3. `crates/core/src/web/pages/entry.rs` : le motif commun des saisies (préremplir, valider, écrire, ou réafficher avec l'erreur),
   repris quatre fois.

Les mentions de `crates/gui/…` dans les fiches des phases 6, 9, 12 et 14 renvoient à du code retiré : elles restent comme historique.
