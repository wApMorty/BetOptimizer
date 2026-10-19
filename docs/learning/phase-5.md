# Phase 5 — Recommandations quotidiennes

Ce qui a été construit : un journal immuable des recommandations (deux paliers : notifiées et « fantôme »), la commande
`edge recommend` (avec notification Discord et rapport HTML joint), le relevé de la clôture de Pinnacle juste avant le coup
d'envoi (`edge closing`), l'enregistrement de tes paris réels (`edge bet log|list|settle`, paris hors modèle à part), le
règlement depuis l'historique football-data, le CLV réel (`edge clv`) et les unités systemd correspondantes.

Résultat à retenir : **le programme ne sait pas encore s'il a un avantage** ; il sait maintenant le mesurer en direct, sur
les vraies cotes des opérateurs, sans rien modifier après coup.

## 1. Concepts Rust rencontrés

**Fonctions reçues en paramètre pour isoler le réseau** — `crates/core/src/notify/discord.rs:20`.
`type Post<'a> = &'a mut dyn FnMut(&str, &str, Vec<u8>) -> Result<(u16, String)>` : l'envoi HTTP est une fonction passée à
`send`. En production c'est `http_post` (`ureq`), dans les tests une fermeture qui note ce qu'on lui envoie. Aucun trait
inventé (CLAUDE.md : un trait seulement avec deux implémentations réelles) et aucun test ne touche le réseau.

**Fermeture appelée aussitôt pour utiliser `?`** — `crates/core/src/store/journal.rs:179`.
`(|| -> Result<…> { … })()` définit puis appelle une fermeture dont le type de retour est notre `Result` : c'est ce qui permet
d'écrire `?` à l'intérieur d'une fermeture de `rusqlite` qui, elle, doit renvoyer `rusqlite::Result`. D'où le type imbriqué
`rusqlite::Result<Result<T>>` et le `row??` de l'appelant : la première `?` traite l'erreur SQLite, la seconde la nôtre.

**`Option::transpose`** — `journal.rs:236` et `:270`. Transforme `Option<Result<T>>` en `Result<Option<T>>` : « pas de ligne »
reste `Ok(None)`, une ligne illisible devient une erreur.

**`bool::then` pour un résultat conditionnel** — `journal.rs:151`. `(changed == 1).then(|| conn.last_insert_rowid())` : `Some(id)` si
la ligne a été insérée, `None` si `INSERT OR IGNORE` l'a écartée parce que le match avait déjà sa recommandation.

**Motif de tableau avec `let … else`** — `crates/core/src/betting/clv.rs:68`.
`let [Some(home), Some(draw), Some(away)] = prices else { return … }` n'accepte le tableau que si les trois cotes sont
présentes. Le compilateur impose le `else` : on ne peut pas oublier le cas incomplet.

**Groupes d'arguments et conflits avec `clap`** — `crates/cli/src/main.rs:98`.
`ArgGroup::new("target").required(true).args(["rec", "event"])` oblige à choisir l'un des deux ; `conflicts_with_all` et
`requires_all` encodent les combinaisons permises. Les erreurs de ligne de commande sont ainsi refusées avant tout code métier.

**`Option::zip`** — `main.rs:233`. `id.zip(result.as_deref())` donne `Some((n°, résultat))` seulement si les deux options sont
présentes.

**Savoir si la sortie est un terminal** — `crates/cli/src/commands.rs:588`.
`std::io::stdout().is_terminal()` : `edge closing` se tait sous systemd (la sortie va dans le journal) et parle à la main.
Sans cela, un passage toutes les dix minutes écrirait 144 lignes par jour pour dire « rien à faire ».

**Écrire un test qui a besoin d'`unwrap` hors d'une fonction `#[test]`** — `crates/cli/tests/commands.rs` (module `archive`).
Clippy tolère `unwrap` dans un `#[test]` ou un module `#[cfg(test)]`, pas dans une fonction d'aide libre : on range les aides
dans un module `#[cfg(test)]`.

## 2. Concepts statistiques

**Journal immuable et idempotence.** Chaque recommandation est écrite une fois, avec tout ce qui la justifie (relevé d'origine,
probabilité, seuil, mise, version du code) ; la clé `UNIQUE (règle, palier, match)` (`0007_recommendations.sql:30`) garantit que
relancer la commande ne change rien. Relire une recommandation d'il y a deux mois donne exactement ce qu'on savait ce jour-là,
condition pour juger honnêtement le CLV.

**Échantillonnage : le journal fantôme.** Si l'on ne journalise que ce qui est notifié (avantage ≥ 3 %), on accumule quelques
paris par mois et on ne conclura jamais. Si l'on journalise **tout** ce qui a un avantage ≥ 0 %, sans rien changer à ce qu'on
notifie, on observe le CLV par tranche d'avantage en quelques semaines. Ce n'est pas du « p-hacking » : la règle de sélection est
fixée avant les données, et les deux paliers sont rapportés séparément (`edge clv`).

**Clôture en direct et erreur de mesure.** Le CLV compare la cote obtenue à la probabilité de clôture de Pinnacle sans marge. En
direct, la « clôture » est un relevé pris 20 à 30 minutes avant le coup d'envoi (`ingest/closing.rs:25`). Elle manque donc les
derniers mouvements : le CLV mesuré est un peu différent de celui du backtest, qui utilise la vraie clôture.

**Contrat de la cote minimale.** Chaque recommandation affiche `(1 + seuil) / p`. Un pari pris plus bas n'est plus la
recommandation : il est enregistré hors modèle. Mélanger les deux fausserait la comparaison entre « mes choix » et « ceux du
modèle » (invariant 7).

**Bankroll comptable.** `bankroll = départ + gains réglés − mises ouvertes` (`journal.rs:528`). Une mise ouverte est déjà sortie de
la bankroll : sinon on miserait deux fois le même argent sur deux matchs simultanés.

**Règlement sans fausse certitude.** Un résultat n'est lu que si les deux noms d'équipes passent par la table d'alias ; sinon le
pari reste ouvert (`betting/settle.rs`). Mieux vaut un pari non réglé qu'un pari réglé sur le mauvais match.

## 3. Trois questions pour vérifier que tu as compris

1. Pourquoi le journal fantôme ne crée-t-il pas de biais de sélection, alors qu'on « choisit » les paris à journaliser ? Qu'est-ce
   qui ferait de lui un biais (indice : ce qui se passerait si l'on changeait le seuil après avoir regardé les résultats) ?
2. `edge closing` est lancé toutes les dix minutes mais ne dépense presque aucun crédit. Quelles conditions précises déclenchent
   un appel, et pourquoi le nombre d'essais par match est-il limité à deux (`MAX_ATTEMPTS`) ?
3. Tu places un pari à 2,05 alors que la cote minimale de la recommandation est 2,12. Pourquoi `edge bet log --rec` le refuse-t-il,
   et en quoi le ranger hors modèle protège-t-il la mesure du CLV ?

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/store/journal.rs` — le modèle de données : recommandations, paris, règlements, tous immuables, et le calcul de
   la bankroll. Ses tests décrivent chaque garantie (doublons, déclencheurs, hors modèle).
2. `crates/core/src/betting/recommend.rs` — la chaîne complète d'un relevé à des lignes de journal : paliers, mises en Kelly
   simultané sur le lot, transaction unique.
3. `crates/core/src/ingest/closing.rs` — quand, et à quel prix, on va chercher la clôture ; c'est le seul endroit où le programme
   dépense des crédits sans qu'on le lui demande, d'où la garde de budget et la limite d'essais.

## Mise à jour du 2026-10-19 : un bilan chaque jour, pas seulement des recommandations

Paul trouvait que l'algorithme « ne faisait rien » : sans prix à 3 %, aucun message ne partait. `edge recommend --notify` envoie maintenant un
**bilan quotidien** (`notify::message::daily`) : combien de matchs ont été comparés à Pinnacle, un avertissement `⚠` si Pinnacle manque (panne de
l'API), les recommandations s'il y en a, puis les **cinq prix les plus proches de la valeur**, même négatifs, donnés pour information. Le rapport HTML
joint contient le tableau de tous les matchs (`PriceRow`). Pour cela `recommend::Report` porte `priced` : le scan se fait sans seuil (−1) et le journal
ne retient toujours que les avantages positifs. Concept Rust : `scan` est paramétré par un seuil, donc « tout voir » n'est qu'une valeur d'argument,
pas une nouvelle fonction.
