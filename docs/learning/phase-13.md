# Phase 13 — Test prospectif de la latence française (OddsPapi, `edge latency`, page `/latence`)

Ce qui a été construit : un client bloquant pour OddsPapi, une archive immuable de son historique (migration 14), une règle de pari
**figée** avant tout match du test, et sa mesure par la CLV.

La question posée : dans la fenêtre des compositions (60 à 15 minutes avant le coup d'envoi), Winamax et Unibet affichent-ils encore
des cotes que Pinnacle a déjà corrigées ? L'étude rétrospective de la phase (`docs/sources.md` §34) a montré un piège. Un relevé
incomplet de Winamax par OddsPapi fabriquait une fausse valeur de +5 %. Sur des données propres, il ne restait qu'un signal fragile.
D'où un test **prospectif** : seuls comptent les matchs joués à partir du 2026-10-22, et le verdict tombe une seule fois, à 32 paris.

Chaque lundi à 9 h, le timer `edge-latency` lance deux commandes :
- `edge latency fetch` télécharge l'historique ;
- `edge latency report` applique la règle et affiche le verdict.

La page `/latence` du site montre le même calcul.

Découpage :

| Fichier | Rôle |
|---|---|
| `ingest/oddspapi/mod.rs` | client HTTP, constantes de l'API |
| `ingest/oddspapi/format.rs` | lecture des réponses |
| `ingest/oddspapi/fetch.rs` | un passage hebdomadaire |
| `store/oddspapi.rs` | archive et lecture |
| `eval/latency.rs` | la règle, le verdict, le contrôle de qualité |
| `report/latency_page.rs` | la page du site |
| `cli/src/latency.rs` | la commande |

## 1. Concepts Rust rencontrés

**Recherche dichotomique dans une série triée : `partition_point`.** `crates/core/src/eval/latency.rs:150`.
- `series.partition_point(|e| e.at <= t)` renvoie le nombre d'entrées dont l'instant est antérieur ou égal à `t`.
- La série est triée, donc il suffit de couper en deux à chaque étape : O(log n) au lieu de tout parcourir.
- La dernière entrée valable est à l'indice `count − 1`.
- `count.checked_sub(1)?` renvoie `None` si `count` vaut 0, au lieu de faire « déborder » un `usize` sous zéro.
- C'est aussi la garantie d'absence de fuite du futur : aucune entrée postérieure à `t` n'est lue.

**Déstructurer deux `Option` d'un coup : `let (Some(a), Some(b)) = (…, …) else`.** `crates/core/src/eval/latency.rs:247`.
- On forme un tuple des deux résultats, puis on exige que les deux soient `Some`.
- Sinon, le `else` passe à l'horizon suivant (`continue`).
- C'est la version à deux variables du `let … else` vu en phase 12.

**`Option::is_none_or` pour un maximum sans valeur initiale arbitraire.** `crates/core/src/eval/latency.rs:255`.
- `best.is_none_or(|(_, b)| edge > b)` vaut `true` si l'on n'a encore rien retenu, ou si le nouvel avantage est plus grand.
- On évite un `f64::NEG_INFINITY` de départ, qui serait une valeur sentinelle à expliquer.

**Prendre possession d'une valeur dans une collection : `remove` plutôt que `get`.** `crates/core/src/ingest/oddspapi/format.rs:152` et
ligne 153.
- `outcome.players` est déplacé dans une variable locale.
- `players.remove("0")` en sort la liste d'entrées **par valeur**.
- `get` n'en donnerait qu'une référence, et il faudrait `clone()` pour la garder.
- La réponse JSON ne sert plus ensuite : la déplacer ne coûte rien.

**Des fonctions reçues en paramètre pour tester sans réseau.** `crates/core/src/ingest/oddspapi/fetch.rs:24` et `crates/cli/src/latency.rs:46`.
- `Pause<'a> = &'a mut dyn FnMut(Duration)` complète `Fetch` et `Clock` (phase 1).
- En production, la CLI passe `&mut std::thread::sleep` : une fonction ordinaire est aussi un `FnMut`.
- Dans les tests, une fermeture vide `|_| {}` : les tests ne dorment pas les 5 secondes entre deux appels.

**`dyn` : un trait comme type, sans générique.**
- `&mut dyn FnMut(…)` signifie « une référence vers n'importe quoi qui s'appelle comme une fonction ».
- Le type exact (fermeture du test ou `thread::sleep`) n'est connu qu'à l'exécution.
- On n'écrit donc ni générique ni trait maison, conformément aux règles du projet.

**Une tranche vide comme valeur par défaut : `&[][..]`.** `crates/core/src/store/oddspapi.rs:94`.
- `history.map_or(&[][..], |h| &h.prices)` donne les prix s'il y a un historique, une tranche vide sinon.
- La même boucle sert donc aux deux cas.
- `[][..]` transforme le tableau vide en tranche `&[PriceChange]`, du même type que `&h.prices` après conversion.

**`BTreeMap` pour la reproductibilité.** `crates/core/src/ingest/oddspapi/format.rs:107`.
- Un `HashMap` parcourt ses clés dans un ordre qui change d'une exécution à l'autre.
- Un `BTreeMap` les parcourt triées.
- Même entrée, même ordre, même résultat (invariant 9).

**Ne jamais garder ce qu'on ne doit pas voir.** `crates/core/src/ingest/oddspapi/format.rs:193`.
- La réponse de `account` contient la clé et l'adresse électronique.
- `RawAccount` ne déclare que `subscriptions`, et `serde` ignore les champs non déclarés : ces secrets ne sont jamais copiés dans une
  structure.
- Le corps de cette réponse n'est pas non plus journalisé (`fetch.rs`, `read_usage`).

## 2. Concepts statistiques

**CLV (rappel, phase 4).** Pour un pari à la cote `o` :
- `CLV = o · p_c − 1`, où `p_c` est la probabilité de clôture de Pinnacle sans marge ;
- la clôture d'un bookmaker sharp est le meilleur estimateur disponible de la probabilité réelle (§16 : log loss de clôture le plus bas) ;
- la CLV mesure donc l'espérance du pari bien plus vite que son rendement.

**Avantage annoncé et clôture.** `avantage = o · p_t − 1`, avec `p_t` la probabilité de Pinnacle au moment de la décision.
- Si Pinnacle ne bouge plus, CLV et avantage sont égaux (test `a_stale_french_price_is_bet_once_at_the_first_qualifying_horizon`).
- Si Pinnacle continue dans le même sens, la CLV dépasse l'avantage (test `the_closing_line_measures_the_clv`).

**Taille d'échantillon.** Pour détecter une CLV moyenne `μ` d'écart-type `σ`, avec un test unilatéral à 5 % et une puissance de 80 % :
- formule : `n ≈ ((z₀,₉₅ + z₀,₈₀) · σ / μ)² = (2,49 · σ / μ)²` ;
- avec `σ = 4,5 %` (mesuré sur l'étude rétrospective) : 32 paris pour `μ = 2 %`, 127 pour `μ = 1 %` ;
- la CLV demande des dizaines de paris là où le rendement en demanderait des milliers (§26).

**Pourquoi un test prospectif et un verdict unique.**
- Regarder les résultats chaque semaine, puis s'arrêter dès qu'ils sont favorables, fausse le test : c'est l'« arrêt optionnel ».
- On fixe donc d'avance l'échantillon jugé : les 32 premiers paris.
- Les suivants sont montrés, mais ne changent pas le verdict.
- Le filtre de suivi a été ajouté **après** avoir vu l'artefact de la rétrospective. C'est un filtre de qualité des données, pas un
  réglage sur l'avantage ; il n'est validé que parce qu'il est appliqué tel quel à des matchs futurs.

**Bootstrap par blocs (rappel).**
- Les paris d'une même journée de championnat se ressemblent : mêmes conditions, mêmes décisions des opérateurs.
- On rééchantillonne donc des blocs « championnat × semaine ISO » (`eval/latency.rs:215`), pas des paris isolés.
- Sinon, l'intervalle serait trop étroit.

**Contrôle de qualité par une source indépendante.**
- Une archive tierce peut être fausse sans le dire : c'est l'artefact d'avril à septembre.
- On compare donc ses cotes Winamax et Unibet aux nôtres (The Odds API), aux mêmes instants.
- Les matchs sont appariés par la table d'alias, jamais par ressemblance de noms.

## 3. Questions pour vérifier la compréhension

1. Pourquoi `price_at` renvoie-t-il `None` quand la dernière entrée antérieure à `t` est une suspension, au lieu de la cote d'avant la
   suspension ? Que se passerait-il dans le test si l'on gardait la cote d'avant ?
2. La règle prend le **premier** horizon qualifiant de la fenêtre (60 minutes avant le 15 minutes). Si elle prenait le meilleur des
   quatre, quel biais introduirait-on, et pourquoi la CLV moyenne serait-elle alors trop belle ?
3. Le verdict ne juge que les 32 premiers paris. Imagine que la CLV soit de +3 % après 20 paris et de −1 % après 32. Pourquoi
   serait-ce une erreur de s'arrêter à 20 et de conclure au succès ?

## 4. Les trois fichiers à relire en priorité

1. **`crates/core/src/eval/latency.rs`.**
   - C'est toute la règle figée : constantes, `window_bets`, `verdict`, contrôle de qualité.
   - Les tests en bas du fichier montrent chaque cas : pari unique au premier horizon, opérateur non suivi, absence de fuite du futur,
     verdict.
2. **`crates/core/src/ingest/oddspapi/fetch.rs`.**
   - Le déroulé d'un passage : garde de budget, liste, historique.
   - Le traitement des cas réels rencontrés : 404 « aucun match », 404 « aucun historique », 522 du serveur, refus de débit.
   - L'injection de l'appel réseau, de l'horloge et des pauses.
3. **`crates/core/src/store/migrations/0014_oddspapi.sql`.**
   - Les contraintes d'intégrité : une cote active a toujours un prix, `history` vaut `ok` ou `missing`.
   - Les déclencheurs d'immuabilité.
   - Le choix de ne pas stocker le corps de `account`.
