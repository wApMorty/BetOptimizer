# Phase 10 — Site de rapports sur le réseau local (`edge serve`)

Ce qui a été construit : un site en lecture seule, servi par le portable et consultable depuis un autre appareil du réseau. Il a quatre
pages : le rapport du jour, les rapports précédents, les opportunités actives et le paper trading. Chaque exécution de
`edge recommend` archive désormais son rapport (`reports/daily/`, un fichier par exécution, jamais écrasé). Avec `site_url` configuré,
le message Discord rappelle l'adresse du site au lieu de joindre le HTML.

Le message Discord joint aussi une **image PNG** des recommandations du jour (cartes aux couleurs du site), rendue par `resvg` ;
`edge notify-test` en envoie un exemple fictif.

Découpage : `core` sait tout calculer et tout rendre (`report/site.rs`, `report/daily.rs`, `betting/paper.rs`) ; la CLI
(`crates/cli/src/serve.rs`) ne fait que traduire entre HTTP et une `Page { status, body }`. On teste donc les pages sans ouvrir de port,
et le serveur avec un seul test de bout en bout.

## 1. Concepts Rust rencontrés

**`thread::scope` : emprunter depuis un autre thread.** `crates/cli/src/serve.rs:109`. Un `thread::spawn` ordinaire exige des données
`'static` : le thread pourrait survivre à la fonction, donc il ne peut rien emprunter de la pile. `thread::scope` garantit que tous
les threads lancés dans le bloc sont terminés à sa sortie. Le compilateur accepte alors que le thread du serveur emprunte `server` et
`site` sans `Arc` ni `clone()`.

**Une méthode qui consomme `self`.** `crates/cli/src/serve.rs:60`. `Request::respond(self, …)` prend la requête par valeur : après
l'appel, `request` n'existe plus, et une seconde réponse à la même requête est une erreur de compilation, pas un bogue à l'exécution.
C'est l'ownership utilisé comme règle métier (« une requête, une réponse »).

**`expect` sur un invariant démontré.** `crates/cli/src/serve.rs:70`. `Header::from_bytes` échoue sur un octet non ASCII ou un
retour à la ligne. Les seuls appels passent des constantes écrites dans le fichier : l'échec est impossible, et le message de
`expect` dit pourquoi (règle de CLAUDE.md).

**`let … else`.** `crates/core/src/report/daily.rs:64`. `let Ok(name) = … else { continue; };` déstructure ou quitte le tour de boucle.
C'est plus lisible qu'un `match` à deux bras dont l'un ne fait que `continue`.

**`OpenOptions::create_new`.** `crates/core/src/report/daily.rs:46`. L'ouverture échoue si le fichier existe, en une seule opération
du système. Écrire « si le fichier n'existe pas, alors l'écrire » en deux étapes laisserait une fenêtre où un autre processus
écrirait entre les deux. C'est l'immuabilité de l'archive garantie par le système de fichiers.

**`std::cmp::Reverse` comme clé de tri.** `crates/core/src/report/daily.rs:72`. `sort_by_key(|r| Reverse(r.at))` trie du plus récent
au plus ancien sans écrire de comparateur à la main. Clippy l'a proposé à la place de `sort_by(|a, b| b.at.cmp(&a.at))`.

**`Iterator::scan` et `fold`.** `crates/core/src/betting/paper.rs:181` et `:154`. `scan` transporte un état (le total) d'un élément au
suivant et rend chaque état intermédiaire : c'est la courbe du gain cumulé. `fold(0.0, …)` remplace `sum()` pour une raison
inattendue : depuis Rust 1.81, la somme d'un itérateur **vide** de `f64` vaut −0,0, que le format affiche « -0,0 u ». On l'a vu sur
la vraie base, qui ne contient encore aucune recommandation misée.

**Un type du domaine pour la configuration.** `crates/core/src/config.rs:140`. `serve_bind: SocketAddr` plutôt que `String` : `serde`
valide l'adresse à la lecture du fichier (`"portable:8737"` est refusé tout de suite), et le serveur reçoit une adresse déjà valide.

**Héritage de gabarits `minijinja`.** `crates/core/src/report/templates/recommend.html:1` et `crates/core/src/report/render.rs:129`.
`{% extends "site_base.html" %}` réutilise les styles et la barre de navigation. La base est ajoutée à l'environnement à chaque rendu,
sinon `extends` ne la trouverait pas.

**Rendre une image sans police du système.** `crates/core/src/report/picks_image.rs:15` et `render_png`. `include_bytes!` range la
police dans le binaire ; `fontdb::Database::load_font_data` la reçoit **par valeur** (`Vec<u8>`), d'où le `to_vec()` : la base de polices
doit posséder ses octets, alors que la constante n'est qu'empruntée. `Arc::new(fonts)` partage ensuite la base sans la copier, comme
l'exige `usvg::Options`. Résultat : le même PNG sous Linux et Windows, et un test qui le vérifie (le texte est bien dessiné).

**Mise à jour fonctionnelle d'une structure : `..Default::default()`.** `picks_image.rs`, dans `render_png` : `usvg::Options { font_family,
fontdb, ..usvg::Options::default() }` fixe deux champs et prend les autres dans la valeur par défaut. Ce n'est pas un builder : une seule
expression, lisible, sans couche d'indirection.

**Une liste plutôt qu'une `Option`.** `crates/core/src/notify/discord.rs`, `send(…, attachments: &[Attachment], …)` : passer de « zéro ou
un fichier » à « zéro ou plusieurs » sans changer la forme des appels (`&[]` pour aucun). La tranche `&[T]` emprunte les pièces jointes
de l'appelant, qui restent possédées par ses variables locales.

## 2. Concepts statistiques

**Paper trading.** Chaque recommandation notifiée **et misée** est suivie comme si elle avait été jouée à la cote et à la mise
annoncées. Gain d'un pari de mise `m` à la cote décimale `c` : `m × (c − 1)` s'il gagne, `−m` s'il perd. Les mises sont en unités
(1 u = 1 % de la bankroll du jour de la recommandation), donc indépendantes de tes paris réels. Le journal fantôme et les
recommandations sous la mise minimale sont exclus de la liste comme du rendement (tu ne les joues pas).

**Rendement des mises contre rendement à mise fixe.** `ROI = Σ gains / Σ mises`, pondéré par Kelly : les gros paris pèsent davantage. Le
rendement à mise fixe donne le même poids à chaque pari ; il porte un intervalle de confiance par bootstrap **par blocs de
semaines** (les paris d'une même semaine ne sont pas indépendants). Avec trois paris dans une seule semaine, l'intervalle s'effondre
sur un point : la page affiche le nombre de paris réglés pour qu'on ne le prenne pas pour une certitude.

**Baisse maximale en unités.** `max_t (max_{s ≤ t} G_s − G_t)`, avec `G_0 = 0` : la plus grande chute du gain cumulé depuis un sommet. Elle
est en unités et non en pourcentage, parce que la courbe additionne des mises calculées sur des bankrolls différentes : ce n'est pas
une bankroll.

**La CLV reste la mesure principale.** Le rendement de quelques dizaines de paris est presque du bruit (écart type d'environ `√n`
mises). La CLV, bien moins variable, se prononce bien avant ; la page le rappelle tant qu'il y a moins de 30 clôtures.

## 3. Trois questions pour vérifier

1. Dans `serve.rs`, que se passerait-il à la compilation si l'on remplaçait `std::thread::scope` par `std::thread::spawn` sans rien
   changer d'autre, et pourquoi ?
2. Une requête `GET /rapports/../edge.db` arrive. Quel chemin du code la refuse, et pourquoi aucune normalisation du chemin n'est
   nécessaire ?
3. Une recommandation notifiée sous la mise minimale (mise `NULL`) gagne à 3,00. De combien change le rendement du paper trading, et où
   le code en décide-t-il ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/report/site.rs` : le routage complet et la règle de sécurité (liste blanche des fichiers archivés) ; chaque page est
   une fonction courte qui lit, met en forme et rend.
2. `crates/core/src/betting/paper.rs` : la définition de ce qui compte dans le paper trading, le règlement et les sommes cumulées.
3. `crates/cli/src/serve.rs` : la seule partie qui parle HTTP ; courte, avec les en-têtes de sécurité et le test de bout en bout.
