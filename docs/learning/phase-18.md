# Phase 18 — Suivi en direct des grilles Loto Foot

Ce qui a été construit : pendant les matchs, la page `/lotofoot` relève les scores chez ESPN (API publique, gratuite, sans clé). Chaque
rencontre affiche son score et sa minute ; chaque case cochée devient verte (juste) ou rouge barrée (fausse), en tirets tant que le
match se joue ; chaque grille à remplir et chaque ticket prend une couleur : gris (à venir), vert (sans faute), orange (une erreur),
rouge (perdu). La page se recharge seule toutes les minutes tant qu'un match se joue. Sources et choix : `docs/sources.md` §46.

| Fichier | Rôle |
|---|---|
| `core/src/lotofoot/espn.rs` | client ESPN, codes de ligue, lecture de la page « scoreboard » |
| `core/src/lotofoot/scores.rs` | rencontres à suivre, appels limités, appariement par alias, état des tickets |
| `core/src/store/lotofoot_live.rs`, `store/migrations/0017_lotofoot_scores.sql` | dernier score de chaque rencontre |
| `core/src/web/pages/lotofoot.rs`, `web/templates/lotofoot.html` | couleurs, scores, rechargement |
| `config/aliases.toml` (`[ESPN]`) | noms d'ESPN → noms de The Odds API |

## 1. Concepts Rust rencontrés

**Ne déclarer que les champs utiles d'un JSON : `#[derive(Deserialize)]` partiel.** `crates/core/src/lotofoot/espn.rs:244`.
- La réponse d'ESPN compte des centaines de champs ; les structures `Raw…` n'en déclarent qu'une dizaine. serde ignore le reste.
- `#[serde(rename = "homeAway")]` relie un nom JSON à un nom Rust ; `#[serde(default)]` donne une valeur par défaut à un champ absent
  (`period`, absent avant le coup d'envoi).

**`match` sur une chaîne avec gardes : `"pre" if stopped => …`.** `crates/core/src/lotofoot/espn.rs:337`.
- Le même motif (`"pre"`) apparaît deux fois : la garde `if` choisit la branche. Le dernier bras `_ => None` traite tout état inconnu
  (le match est alors ignoré et compté comme illisible).

**`ON CONFLICT … DO UPDATE` : écrire ou remplacer en une requête.** `crates/core/src/store/lotofoot_live.rs:50`.
- SQLite insère la ligne, ou met à jour celle qui a la même clé primaire. `excluded.x` désigne la valeur qu'on tentait d'insérer.
- C'est la seule table du projet qui se modifie : un score provisoire n'est qu'un affichage (choix du 2026-10-23).

**`Option::transpose` : de `Option<Result<T>>` à `Result<Option<T>>`.** `crates/core/src/store/lotofoot_live.rs:123`.
- `map(parse_timestamp)` sur un `Option<&str>` donne un `Option<Result<…>>` ; `transpose` inverse les deux pour que `?` (ou le
  retour de la fonction) traite l'erreur, tout en gardant « pas de ligne » comme `None`.

**Durée de vie explicite : `fn pair<'a>(…, games: &'a [EspnGame]) -> Result<&'a EspnGame, String>`.** `crates/core/src/lotofoot/scores.rs:249`.
- Trois références entrent, une sort : le compilateur ne peut pas deviner de laquelle elle provient. `'a` dit « elle pointe dans
  `games` » : le match trouvé reste valide tant que la page lue existe, sans copie.

**`Option` comme liste de zéro ou un élément : `report.issues.extend(issue)`.** `crates/core/src/lotofoot/scores.rs:227`.
- `Option` implémente `IntoIterator` : `extend` ajoute le texte s'il existe, rien sinon. Pas besoin de `if let Some(…)`.
- `issue.as_deref()`, juste avant, prête le texte (`Option<&str>`) à la base sans le copier ni le consommer.

**`bool::then` : une valeur seulement si une condition tient.** `crates/core/src/lotofoot/scores.rs:220`.
- `(!problems.is_empty()).then(|| problems.join(" ; "))` vaut `Some(texte)` s'il y a des problèmes, `None` sinon. La fermeture
  n'est appelée que dans le premier cas.

**Fermeture passée comme `&mut dyn FnMut` : le vrai client ou un faux.** `crates/core/src/web/pages/lotofoot.rs:141`.
- `refresh` reçoit l'appel réseau en paramètre (type `Fetch`, déjà utilisé pour The Odds API et prono-match). La page lui passe une
  fermeture qui appelle ESPN ; les tests une fermeture qui rend une page écrite à la main et compte les appels.

**Ne jamais casser la page : `unwrap_or_else` sur un `Result`.** `crates/core/src/web/pages/lotofoot.rs:164`.
- Si le suivi échoue (base, réseau), l'erreur devient un message sur la page au lieu d'une erreur 500.

## 2. Concepts statistiques

**Erreurs d'une grille multiple = celles de son meilleur ticket.** `crates/core/src/lotofoot/scores.rs:390`.
- Une grille multiple regroupe sans perte tous les tickets de son produit de cases (§39). Une rencontre ne la fait échouer que si
  **aucune** case cochée n'est sortie : le nombre de rencontres fausses est alors exactement le nombre d'erreurs de son meilleur
  ticket. Au Loto Foot, 0 erreur paie le 1er rang, 1 erreur le 2e.
- Formule : erreurs = #{ i : signe(i) connu et signe(i) ∉ cases(i) }.

**Provisoire contre définitif.** `crates/core/src/lotofoot/scores.rs:347`.
- Seul compte le score à la fin du temps réglementaire (règlement Loto Sports). Un match en cours donne un signe provisoire ; un match
  prolongé n'en donne aucun, car ESPN ne publie pas le score à 90 minutes. « Perdu » n'est affiché qu'à deux erreurs **définitives** :
  un but tardif peut encore ramener un ticket orange au vert.

## 3. Questions pour vérifier

1. Un dimanche, la page affiche « noms ESPN absents de la section [ESPN] : Stade Brestois 29 ». Que faut-il écrire, où, et que
   faut-il faire ensuite pour que le score apparaisse ?
2. Tu laisses la page ouverte pendant trois matchs de Ligue 2 et un de Serie A, tous en cours. Combien d'appels ESPN par minute, et
   pourquoi pas un par rencontre ?
3. Un match de coupe se termine 1 – 1 après prolongation et 3 – 2 aux tirs au but. Quelle couleur prend un ticket qui avait coché
   `N` sur ce match, et pourquoi ?

## 4. Fichiers à relire en priorité

1. `crates/core/src/lotofoot/scores.rs` : quand on appelle ESPN, comment une rencontre est appariée, comment un ticket est jugé.
2. `crates/core/src/lotofoot/espn.rs` : ce qu'on lit de la réponse d'ESPN et comment un état devient « en cours », « terminé » ou
   « à confirmer ».
3. `crates/core/src/web/pages/lotofoot.rs` (`follow`, `column`) : ce que la page ajoute au bulletin et pourquoi une erreur de suivi
   ne la casse jamais.
