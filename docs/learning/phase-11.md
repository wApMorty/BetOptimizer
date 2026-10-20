# Phase 11 — Cotes boostées : étude historique sur WePari (`edge boost study`)

Ce qui a été construit : un module `crates/core/src/boost/` qui lit l'historique des cotes boostées publié par WePari (Winamax depuis
2021, Parions Sport depuis fin 2022). Chaque boost portant sur un match du Big 5 ou de l'Écosse est rapproché du match en base, son
intitulé est traduit en conditions sur le score final, puis il est tarifé contre Pinnacle sans marge grâce à une grille de scores
**calée sur le marché**. La commande `edge boost study` affiche l'entonnoir, un contrôle de qualité, le bilan par famille et par
tranche de cote, et des règles de sélection sans fuite du futur. Les chiffres et leur lecture sont dans `docs/sources.md` §29.

Découpage :
- `wepari.rs` lit le HTML ;
- `selection.rs` traduit les intitulés ;
- `pricing.rs` construit la grille et calcule une probabilité ;
- `study.rs` rapproche les boosts des matchs et fait les bilans ;
- `crates/cli/src/boost.rs` regroupe et affiche.

## 1. Concepts Rust rencontrés

**Analyser du HTML avec `scraper`.** `crates/core/src/boost/wepari.rs:71`. `Html::parse_document` construit l'arbre du document ;
`Selector::parse("table[width=\"95%\"] tr")` est un sélecteur CSS, comme dans un navigateur. `parse` renvoie un `Result` (un sélecteur
peut être mal écrit) ; ici il est constant et vérifié par les tests, d'où l'`expect("sélecteur CSS constant valide")`, autorisé par
CLAUDE.md pour un invariant démontré.

**Durées de vie dans une signature : `ElementRef<'_>`.** `wepari.rs:109`. Un `ElementRef` **emprunte** le document : il ne peut pas lui
survivre. `'_` dit « une durée de vie quelconque, que le compilateur déduit ». Les textes rendus par `element.text()` sont des `&str`
empruntés au document : on les copie (`to_string`) dans une `Cell` qui, elle, possède ses données et peut quitter la boucle.

**`collect` vers `Option<Vec<_>>`.** `crates/core/src/boost/selection.rs:128`. Un itérateur d'`Option<Condition>` se collecte en
`Option<Vec<Condition>>` : le résultat vaut `None` dès qu'un seul élément vaut `None`. Une seule clause non reconnue rend donc tout
l'intitulé non évaluable, sans boucle ni drapeau. `Result` fonctionne de la même façon (`study.rs`, `check_grid`, avec `collect::<Result<_>>()`).

**Motif de plusieurs caractères dans `split_once`.** `selection.rs:253`. `split_once(['-', ':'])` coupe au premier `-` **ou** `:` :
un tableau de `char` est un motif (trait `Pattern`). On lit ainsi « 1-0 » (Winamax) et « 1:0 » (Parions Sport) avec le même code.

**Une fermeture `FnMut` qui emprunte un cache.** `crates/core/src/boost/study.rs:165`. La fermeture `fair` capture `grids` (une
`HashMap`) **en emprunt mutable** pour y ranger les grilles déjà calculées, d'où `let mut fair`. Tant qu'elle vit, on ne peut pas toucher
`grids` ailleurs : le compilateur l'interdit, ce qui rend le cache sûr par construction. `entry(…).or_insert_with(|| fit(…))` ne calcule la
grille que si elle manque.

**Renvoyer une donnée possédée plutôt qu'un emprunt.** `study.rs:235`. `locate` renvoie le match (emprunté à `day_games`, qui survit à
l'appel : durée de vie `'g`) **et** les noms d'équipe, copiés par `clone()`. Ces noms appartiennent à `pairs`, un `Vec` local détruit au
retour de la fonction : une référence vers eux serait pendante, et le compilateur la refuserait.

**`while let` avec `find_map`.** `study.rs:269`. On retire les étiquettes « MyMatch », « MT », « Winamaxico »… tant qu'il en reste une en
tête. `find_map` rend le premier `strip_prefix` qui réussit. L'affectation `text = rest….to_string()` est permise alors que `rest`
emprunte `text` : la nouvelle chaîne est construite **avant** que l'ancienne soit remplacée.

**Implémenter un trait externe : `CostFunction` d'`argmin`.** `crates/core/src/boost/pricing.rs:83`. On décrit le coût, et l'optimiseur de
Nelder-Mead fait le reste. C'est l'usage légitime prévu par CLAUDE.md. `Executor::new` **consomme** le problème : pour relire la grille
optimale, on en reconstruit un (`pricing.rs:120`), ce qui ne coûte rien puisque ses deux champs sont `Copy`.

**Une énumération à donnée : `Rho`.** `pricing.rs:38`. `Rho::Fitted` ou `Rho::Fixed(f64)` : deux comportements, aucun trait. Le
`match` sur `Rho` dit au même endroit combien de paramètres on ajuste et quelle tolérance on accepte.

**`Option::is_none_or`.** `crates/cli/src/boost.rs:114`. `best.is_none_or(|(_, b)| loss < b)` : vrai s'il n'y a pas encore de meilleur, ou
si le nouveau est meilleur. C'est la recherche d'un minimum sans valeur sentinelle.

## 2. Concepts statistiques

**Espérance d'un boost.** Pour une cote boostée `c` et une probabilité juste `p` : `E = p·c − 1` par euro misé. La **marge d'origine** est
la même formule avec la cote avant boost : elle mesure ce que l'opérateur prélève sur ce type de pari (−15 à −19 % pour les combinés
Winamax, contre environ −3 % sur le 1X2 de Pinnacle).

**Grille calée sur le marché.** Le marché ne cote pas « Lyon gagne et les deux équipes marquent ». On cherche donc la grille de
Dixon-Coles qui reproduit trois prix de Pinnacle :

`P(x, y) = τ(x, y) · Pois(x; λ) · Pois(y; μ)`, avec `(λ, μ, ρ) = argmin [ (P_H − p_H)² + (P_A − p_A)² + (P_{>2,5} − p_{>2,5})² ]`

C'est trois équations pour trois inconnues, donc un ajustement exact. La probabilité d'un pari est la somme des cases où il est gagné
(`pricing::probability`). Intuition : le 1X2 fixe l'**écart** de force, le plus/moins fixe le **total** de buts, ρ ajuste la part des
petits scores. Tout le reste découle de la forme poissonnienne, et c'est là que se logent les erreurs (§29 : les deux équipes marquent
sous-estimé de 2,4 points).

**Choisir le modèle avant l'étude.** La variante de ρ est choisie sur 2019/20–2020/21, deux saisons qui précèdent tout boost de
WePari. L'étude porte ensuite sur des matchs que ce choix n'a pas vus (invariant 8 ; trois configurations essayées, affichées).

**Log loss d'un pari binaire.** `−[y ln p + (1 − y) ln(1 − p)]`, moyennée sur les matchs puis sur les 14 paris de référence. C'est le
critère de choix, et non le taux de réussite (invariant 3).

**Calibration par z.** `z = (O − Σ p) / √(Σ p(1 − p))` : victoires observées moins attendues, en écarts-types d'une somme de Bernoulli
indépendantes. |z| > 2 signale un biais de tarification, à condition de ne pas avoir regardé vingt groupes (sur vingt groupes, un |z| > 2
arrive par hasard).

**Filtrer sans fuite du futur, mesurer à la clôture.** La règle de sélection n'utilise que le prix d'avant-match, connu au moment de
parier. On la juge ensuite avec la clôture (meilleure estimation de la vraie probabilité : c'est la logique de la CLV) **et** avec le gain
réel, qui ne dépend d'aucun modèle. Quand l'espérance annoncée monte avec le seuil mais que le gain réel stagne (§29 : +15,8 % puis +33,5 %
annoncés, environ +9 à +13 % réalisés), l'écart vient en partie du modèle et non de l'opérateur.

**Antisélection.** L'opérateur choisit ce qu'il booste. Si son propre prix contient une information que notre grille ignore (compositions,
modèle spécialisé du marché « les deux équipes marquent »), les boosts qui nous paraissent les plus généreux sont justement ceux où notre
grille se trompe. Signe mesuré : la famille « les deux équipes marquent » est sous-estimée par la grille sur tous les matchs (z = +4,6),
mais surestimée sur les matchs que Winamax a boostés (z = −2,5).

## 3. Trois questions pour vérifier

1. Un boost « Paris SG gagne et les deux équipes marquent » est lu, mais l'événement s'écrit « Cote Boostée : PSG - Lens » et « PSG »
   n'est pas dans la table d'alias. Que fait `edge boost study`, et pourquoi ce comportement est-il préférable à un rapprochement approximatif ?
2. Pourquoi la grille calée avec ρ fixé à 0 ne peut-elle pas reproduire exactement le 1X2 de Pinnacle, et quelle conséquence cela a-t-il
   eue sur la première comparaison des variantes (78 % de matchs écartés) ?
3. La règle « avantage d'avant-match > 10 % » annonce +33,5 % d'espérance et réalise +12,6 % [−3,4 ; +28,9]. Donne deux explications
   possibles de l'écart, et la mesure qui permettrait de les départager.

## 4. Les trois fichiers à relire en priorité

1. `crates/core/src/boost/pricing.rs` : la grille calée sur le marché, cœur de la tarification et principale source d'erreur.
2. `crates/core/src/boost/selection.rs` : ce qui est reconnu et ce qui est refusé ; les tests listent les tournures réelles.
3. `crates/core/src/boost/study.rs` : le rapprochement strict par alias, les deux moments (avant-match pour filtrer, clôture pour mesurer)
   et le bilan avec intervalles par blocs.
